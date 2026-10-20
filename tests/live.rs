//! Runs the real binary against fake `bd` and `scrolls` programs and reads
//! its pages over HTTP. The fake `bd serve` points Fabriscope at a small
//! fake Beads API that this test runs, backed by files the test rewrites.
//! The update tests run a copy of the binary against a fake GitHub release.

use sha2::Digest;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FAKE_BD: &str = r#"#!/bin/sh
# Answers Fabriscope's bd calls from files in its working directory.
case "$1" in
  serve)
    if [ -f no-serve ]; then
      echo "Error: cannot resolve workspace context: cannot determine repository root: not a git repository" >&2
      exit 1
    fi
    echo "bd serve: listening on $(cat serve-url)"
    # Stay up, like the real server, until Fabriscope goes away.
    while kill -0 "$PPID" 2>/dev/null; do sleep 1; done ;;
  vc) cat commit.json ;;
  export) cat issues.jsonl ;;
  ready) cat ready.json ;;
  *) echo "unexpected bd $*" >&2; exit 2 ;;
esac
"#;

const FAKE_SCROLLS: &str = include_str!("fixtures/scrolls.py");

struct Server {
    child: Child,
    address: String,
    stderr: Arc<Mutex<String>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fabriscope-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn script(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// A Beads store on disk plus the fake tools.
struct Store {
    beads: PathBuf,
    tools: PathBuf,
    scrolls: PathBuf,
}

fn store(name: &str) -> Store {
    let beads = scratch(&format!("{name}-beads"));
    let tools = scratch(&format!("{name}-tools"));
    let scrolls = scratch(&format!("{name}-store")).join(".scrolls");
    std::fs::create_dir_all(&scrolls).unwrap();
    std::fs::write(beads.join("issues.jsonl"), fixture("issues.jsonl")).unwrap();
    std::fs::write(
        beads.join("commit.json"),
        r#"{"branch":"main","commit":"c1"}"#,
    )
    .unwrap();
    std::fs::write(beads.join("ready.json"), r#"[{"id":"demo-1"}]"#).unwrap();
    std::fs::write(tools.join("graph.json"), fixture("graph.json")).unwrap();
    script(&tools.join("bd"), FAKE_BD);
    script(&tools.join("scrolls"), FAKE_SCROLLS);
    Store {
        beads,
        tools,
        scrolls,
    }
}

/// The fake Beads API: answers from the store's files and records every
/// Authorization header it sees.
fn fake_api(beads: PathBuf) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let auth = Arc::new(Mutex::new(Vec::new()));
    let seen = auth.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            if let Some(value) = request.lines().find_map(|l| {
                l.strip_prefix("authorization: ")
                    .or(l.strip_prefix("Authorization: "))
            }) {
                seen.lock().unwrap().push(value.to_string());
            }
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            let route = path.split('?').next().unwrap_or("");
            let (status, body) = match route {
                "/v0/beads/issues" => match std::fs::read_to_string(beads.join("issues.jsonl")) {
                    Ok(text) => {
                        let items: Vec<serde_json::Value> = text
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .map(|l| serde_json::from_str(l).unwrap())
                            .collect();
                        (
                            200,
                            serde_json::json!({"items": items, "has_more": false}).to_string(),
                        )
                    }
                    Err(_) => (
                        500,
                        r#"{"code":"internal","detail":"store unreadable"}"#.to_string(),
                    ),
                },
                "/v0/beads/ready" => {
                    let items: serde_json::Value = serde_json::from_str(
                        &std::fs::read_to_string(beads.join("ready.json")).unwrap(),
                    )
                    .unwrap();
                    (
                        200,
                        serde_json::json!({"items": items, "has_more": false}).to_string(),
                    )
                }
                "/v0/beads/stats" => {
                    let size = std::fs::metadata(beads.join("issues.jsonl"))
                        .map(|m| m.len())
                        .unwrap_or(0);
                    (
                        200,
                        serde_json::json!({"summary": {"size": size}}).to_string(),
                    )
                }
                "/v0/beads/events" => (
                    409,
                    r#"{"code":"events_journal_disabled","detail":"off"}"#.to_string(),
                ),
                _ => (
                    404,
                    r#"{"code":"not_found","detail":"no such route"}"#.to_string(),
                ),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, auth)
}

fn start(args: &[&str]) -> Server {
    // Never let a test update the build output in target/.
    start_program(
        Path::new(env!("CARGO_BIN_EXE_fabriscope")),
        args,
        &[("FABRISCOPE_NO_UPDATE", "1")],
    )
}

fn start_program(program: &Path, args: &[&str], env: &[(&str, &str)]) -> Server {
    let mut child = retry_busy(program, || {
        Command::new(program)
            .args(args)
            .args(["--bind", "127.0.0.1:0"])
            .envs(env.iter().copied())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    });
    let stderr = Arc::new(Mutex::new(String::new()));
    let sink = stderr.clone();
    let pipe = child.stderr.take().unwrap();
    std::thread::spawn(move || {
        // Line by line: the pipe only closes when Fabriscope exits.
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            let mut text = sink.lock().unwrap();
            text.push_str(&line);
            text.push('\n');
        }
    });
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let address = line
        .split("http://")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no address in {line:?}: {}", stderr.lock().unwrap()))
        .to_string();
    Server {
        child,
        address,
        stderr,
    }
}

fn get(server: &Server, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(&server.address).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

fn wait_for(server: &Server, what: &str, check: impl Fn(u16, &str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let (status, health) = get(server, "/healthz");
        if check(status, &health) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: {health}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn check_pages(server: &Server) {
    let (status, page) = get(server, "/");
    assert_eq!(status, 200);
    assert!(page.contains("Fabriscope <span class=\"muted\">/ Demo</span>"));
    assert!(
        page.contains("Write the README"),
        "ready bead on the overview"
    );
    assert!(
        page.contains("Use Dolt for history"),
        "newest scrolls on the overview"
    );
    let (_, bead) = get(server, "/bead/demo-1");
    assert!(bead.contains("&lt;script&gt;"), "document HTML is escaped");
    assert!(
        bead.contains("href=\"/scroll/sc%2Dd1\""),
        "a document link reaches the scroll"
    );
    assert!(bead.contains("Scrolls about this bead"));
    assert!(bead.contains("Ready, as bd ready decides"));
    let (_, blocked) = get(server, "/bead/demo-2");
    assert!(
        blocked.contains("<span class=\"badge open\">open</span>"),
        "the badge is the stored status, not a computed one"
    );
    assert!(!blocked.contains("Ready, as bd ready decides"));
    // Filters offer exactly the statuses and types the store holds.
    let (_, list) = get(server, "/beads?state=all");
    for option in [
        "<option value=\"open\">open (3)</option>",
        "<option value=\"closed\">closed (1)</option>",
        "<option value=\"epic\">epic (1)</option>",
        "<option value=\"task\">task (3)</option>",
    ] {
        assert!(list.contains(option), "{option}");
    }
    assert!(
        !list.contains("value=\"hooked\""),
        "no status the store lacks"
    );
    assert!(
        get(server, "/beads?state=open&ready=no")
            .1
            .contains("2 matching beads"),
        "open beads bd ready does not list"
    );
    assert!(
        blocked.contains("href=\"/bead/demo%2D1\""),
        "its blocker is linked"
    );
    assert!(get(server, "/epics").1.contains("1 / 3 closed"));
    let (_, scroll) = get(server, "/scroll/sc-d1");
    assert!(scroll.contains("bead demo-1"));
    assert!(
        scroll.contains("Rollback readback"),
        "incoming support edge"
    );
    assert!(
        get(server, "/scrolls?kind=evidence")
            .1
            .contains("1 matching scrolls")
    );
    assert_eq!(get(server, "/bead/nope").0, 404);
}

#[test]
fn reads_through_a_private_bd_serve_and_follows_a_live_change() {
    let s = store("serve");
    let (url, _) = fake_api(s.beads.clone());
    std::fs::write(s.beads.join("serve-url"), &url).unwrap();
    let server = start(&[
        "--beads",
        s.beads.to_str().unwrap(),
        "--scrolls",
        s.scrolls.to_str().unwrap(),
        "--bd",
        s.tools.join("bd").to_str().unwrap(),
        "--scrolls-bin",
        s.tools.join("scrolls").to_str().unwrap(),
        "--title",
        "Demo",
    ]);
    let (_, health) = get(&server, "/healthz");
    assert!(
        health.contains(&format!("\"source\":\"bd serve {url}\"")),
        "{health}"
    );
    check_pages(&server);
    assert!(
        get(&server, "/")
            .1
            .contains(&format!("Beads via bd serve {url}"))
    );

    // A new bead and a new commit appear without a restart.
    let mut issues = fixture("issues.jsonl");
    issues.push_str(r#"{"id":"demo-4","title":"Announce it","status":"open","priority":3,"issue_type":"task","created_at":"2026-10-05T09:00:00Z"}"#);
    issues.push('\n');
    std::fs::write(s.beads.join("issues.jsonl"), issues).unwrap();
    std::fs::write(
        s.beads.join("commit.json"),
        r#"{"branch":"main","commit":"c2"}"#,
    )
    .unwrap();
    wait_for(&server, "no live refresh", |_, h| {
        h.contains("\"commit\":\"c2\"") && h.contains("\"beads\":5")
    });
    assert!(get(&server, "/beads").1.contains("Announce it"));

    // A failing read keeps the last good data and reports the error.
    std::fs::remove_file(s.beads.join("issues.jsonl")).unwrap();
    std::fs::write(
        s.beads.join("commit.json"),
        r#"{"branch":"main","commit":"c3"}"#,
    )
    .unwrap();
    wait_for(&server, "the failed read was not reported", |status, h| {
        status == 503 && h.contains("\"beads\":5")
    });
    assert!(get(&server, "/").1.contains("Beads read failed"));
}

#[test]
fn falls_back_to_bd_export_when_bd_serve_refuses() {
    let s = store("fallback");
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let server = start(&[
        "--beads",
        s.beads.to_str().unwrap(),
        "--scrolls",
        s.scrolls.to_str().unwrap(),
        "--bd",
        s.tools.join("bd").to_str().unwrap(),
        "--scrolls-bin",
        s.tools.join("scrolls").to_str().unwrap(),
        "--title",
        "Demo",
    ]);
    assert!(
        get(&server, "/healthz")
            .1
            .contains("\"source\":\"bd export\"")
    );
    check_pages(&server);
    // The message comes before the ready line, but the reader thread may
    // not have collected it yet.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let stderr = server.stderr.lock().unwrap().clone();
        if stderr.contains("bd serve did not start") && stderr.contains("not a git repository") {
            break;
        }
        assert!(Instant::now() < deadline, "no fallback message: {stderr}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn attaches_to_a_running_bd_serve_with_a_token() {
    let s = store("url");
    let (url, auth) = fake_api(s.beads.clone());
    let token = s.tools.join("token");
    std::fs::write(&token, "\nsecret-one\nsecret-two\n").unwrap();
    let server = start(&[
        "--beads-url",
        &url,
        "--beads-token-file",
        token.to_str().unwrap(),
    ]);
    let (_, page) = get(&server, "/");
    assert!(page.contains("Write the README"));
    assert!(
        !page.contains("href=\"/scrolls\""),
        "no Scrolls store, no Scrolls tab"
    );
    assert!(
        auth.lock()
            .unwrap()
            .iter()
            .all(|a| a == "Bearer secret-one")
    );
    assert!(!auth.lock().unwrap().is_empty());
}

#[test]
fn refuses_to_start_on_bad_arguments_or_an_unreadable_store() {
    let empty = scratch("unreadable");
    let output = Command::new(env!("CARGO_BIN_EXE_fabriscope"))
        .args([
            "--beads",
            empty.to_str().unwrap(),
            "--bd",
            "/nonexistent/bd",
            "--bind",
            "127.0.0.1:0",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not run /nonexistent/bd"));
    let output = Command::new(env!("CARGO_BIN_EXE_fabriscope"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "give --beads DIR (or --beads-url URL), --scrolls STORE (or --scrolls-url URL), or both"
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_fabriscope"))
        .args(["--beads-url", "http://127.0.0.1:1", "--bind", "127.0.0.1:0"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "an unreachable bd serve is an error at startup"
    );
}

/// A fake GitHub release: `latest` redirects to the tag, and the tag serves
/// one archive and its `SHA256SUMS`.
struct Release {
    tag: &'static str,
    archive: Vec<u8>,
    sums: String,
}

const TARGET: &str = "test-target";

fn archive_with(program: &str) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    for (path, body) in [("fabriscope", program), ("README.md", "readme")] {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, path, body.as_bytes())
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn release(tag: &'static str, reports: &str) -> Release {
    let archive = archive_with(&format!(
        "#!/bin/sh\necho 'fabriscope {reports} (test-target)'\n"
    ));
    let sum: String = sha2::Sha256::digest(&archive)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Release {
        tag,
        sums: format!("{sum}  fabriscope-{TARGET}.tar.gz\n0000  fabriscope-other.tar.gz\n"),
        archive,
    }
}

/// Serves the release and returns its base URL (`…/releases`).
fn serve_release(release: Release) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/releases", listener.local_addr().unwrap());
    let location = format!("{base}/tag/{}", release.tag);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            while reader.read_line(&mut request).unwrap_or(0) > 2 {}
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            let download = format!("/releases/download/{}/", release.tag);
            let (status, extra, body): (u16, String, Vec<u8>) = if path == "/releases/latest" {
                (302, format!("Location: {location}\r\n"), vec![])
            } else if path == format!("{download}fabriscope-{TARGET}.tar.gz") {
                (200, String::new(), release.archive.clone())
            } else if path == format!("{download}SHA256SUMS") {
                (200, String::new(), release.sums.clone().into_bytes())
            } else {
                (404, String::new(), b"not found".to_vec())
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    base
}

/// A copy of the binary in its own folder, so an update replaces the copy.
/// The folder goes when the test ends: a debug build is large.
struct Installed(PathBuf);

impl std::ops::Deref for Installed {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Installed {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Installed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

fn installed_copy(name: &str) -> Installed {
    let dir = scratch(&format!("{name}-bin"));
    let program = dir.join("fabriscope");
    std::fs::copy(env!("CARGO_BIN_EXE_fabriscope"), &program).unwrap();
    Installed(program)
}

/// A copy written moments ago can be briefly "text file busy" if another
/// test thread forked while it was open, so starting it is retried.
fn retry_busy<T>(program: &Path, mut launch: impl FnMut() -> std::io::Result<T>) -> T {
    for _ in 0..40 {
        match launch() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(50))
            }
            result => return result.unwrap(),
        }
    }
    panic!("{} stayed busy", program.display());
}

fn run(program: &Path, args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    retry_busy(program, || {
        Command::new(program)
            .args(args)
            .env_remove("FABRISCOPE_NO_UPDATE")
            .envs(env.iter().copied())
            .output()
    })
}

fn update_env(base: &str) -> [(&'static str, &str); 2] {
    [
        ("FABRISCOPE_TARGET", TARGET),
        ("FABRISCOPE_RELEASES_URL", base),
    ]
}

fn leftovers(program: &Path) -> Vec<String> {
    std::fs::read_dir(program.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "fabriscope")
        .collect()
}

#[test]
fn update_installs_a_newer_release_over_the_copy_that_runs() {
    let base = serve_release(release("v9.9.9", "9.9.9"));
    let program = installed_copy("update-newer");
    let output = run(&program, &["update"], &update_env(&base));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("updated to 9.9.9"), "{stdout}");
    let after = run(&program, &["--version"], &[]);
    assert_eq!(
        String::from_utf8_lossy(&after.stdout).trim(),
        "fabriscope 9.9.9 (test-target)"
    );
    assert!(leftovers(&program).is_empty(), "{:?}", leftovers(&program));
}

#[test]
fn update_refuses_a_bad_checksum_or_a_program_that_lies_about_its_version() {
    let mut bad_sum = release("v9.9.9", "9.9.9");
    bad_sum.sums = bad_sum
        .sums
        .replacen(|c: char| c.is_ascii_hexdigit(), "x", 1);
    let wrong_version = release("v9.9.9", "1.0.0");
    for (case, release, says) in [
        ("bad-sum", bad_sum, "does not match its SHA256SUMS entry"),
        ("wrong-version", wrong_version, "not version 9.9.9"),
    ] {
        let base = serve_release(release);
        let program = installed_copy(&format!("update-{case}"));
        let before = std::fs::read(&program).unwrap();
        let output = run(&program, &["update"], &update_env(&base));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{case}");
        assert!(stderr.contains(says), "{case}: {stderr}");
        assert!(
            std::fs::read(&program).unwrap() == before,
            "{case}: replaced"
        );
        assert!(leftovers(&program).is_empty(), "{case}");
    }
}

#[test]
fn update_leaves_the_latest_release_alone_and_source_builds_refuse() {
    let base = serve_release(release("v0.0.1", "0.0.1"));
    let program = installed_copy("update-current");
    let before = std::fs::read(&program).unwrap();
    let output = run(&program, &["update"], &update_env(&base));
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("is the latest release"));
    assert!(std::fs::read(&program).unwrap() == before);
    let version = run(&program, &["--version"], &[("FABRISCOPE_TARGET", TARGET)]);
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        format!("fabriscope {} (test-target)", env!("CARGO_PKG_VERSION"))
    );
    if option_env!("FABRISCOPE_TARGET").is_none() {
        let output = run(
            &program,
            &["update"],
            &[("FABRISCOPE_RELEASES_URL", base.as_str())],
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("built from source"));
        assert!(std::fs::read(&program).unwrap() == before);
    }
}

#[test]
fn a_running_server_updates_in_the_background_unless_told_not_to() {
    let s = store("background");
    let scrolls_bin = s.tools.join("scrolls");
    let args = [
        "--scrolls",
        s.scrolls.to_str().unwrap(),
        "--scrolls-bin",
        scrolls_bin.to_str().unwrap(),
    ];
    let base = serve_release(release("v9.9.9", "9.9.9"));

    let program = installed_copy("background");
    let server = start_program(&program, &args, &update_env(&base));
    wait_for(&server, "no background update", |_, h| {
        h.contains(r#""update":{"installed":"9.9.9"}"#)
    });
    assert!(
        server
            .stderr
            .lock()
            .unwrap()
            .contains("updated to v9.9.9; restart to use it")
    );
    let (_, health) = get(&server, "/healthz");
    assert!(
        health.contains(&format!(r#""version":"{}""#, env!("CARGO_PKG_VERSION"))),
        "the running server keeps its own version: {health}"
    );
    drop(server);
    assert!(String::from_utf8_lossy(&run(&program, &["--version"], &[]).stdout).contains("9.9.9"));

    let program = installed_copy("background-off");
    let before = std::fs::read(&program).unwrap();
    let mut env = update_env(&base).to_vec();
    env.push(("FABRISCOPE_NO_UPDATE", "1"));
    let server = start_program(&program, &args, &env);
    std::thread::sleep(Duration::from_secs(2));
    let (_, health) = get(&server, "/healthz");
    assert!(health.contains(r#""update":null"#), "{health}");
    drop(server);
    assert!(std::fs::read(&program).unwrap() == before);
}

#[test]
fn scrolls_managed_refresh_keeps_last_good_data_and_reaps_child() {
    let s = store("scrolls-refresh");
    let mut server = start(&[
        "--scrolls",
        s.scrolls.to_str().unwrap(),
        "--scrolls-bin",
        s.tools.join("scrolls").to_str().unwrap(),
        "--scrolls-dolt",
        "/test/dolt",
    ]);
    let initial: serde_json::Value = serde_json::from_str(&get(&server, "/healthz").1).unwrap();
    assert_eq!(
        initial["scrolls"]["store_id"],
        "00000000-0000-4000-8000-000000000001"
    );
    assert_eq!(initial["scrolls"]["branch"], "main");
    assert_eq!(initial["scrolls"]["revision"], "fixture-rev");
    assert!(
        initial["scrolls"]["source"]
            .as_str()
            .unwrap()
            .starts_with("scrolls serve http://127.0.0.1:")
    );
    let argv: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(s.tools.join("argv.json")).unwrap()).unwrap();
    assert_eq!(
        &argv[argv.len() - 4..],
        ["--json", "serve", "--bind", "127.0.0.1:0"]
    );
    assert!(argv.windows(2).any(|a| a == ["--dolt", "/test/dolt"]));
    assert!(!argv.iter().any(|a| a == "render"));
    wait_for(&server, "unchanged graph was not checked", |_, body| {
        let h: serde_json::Value = serde_json::from_str(body).unwrap();
        h["scrolls"]["last_successful_check"] != initial["scrolls"]["last_successful_check"]
    });
    let unchanged: serde_json::Value = serde_json::from_str(&get(&server, "/healthz").1).unwrap();
    assert_eq!(
        unchanged["revision"], initial["revision"],
        "304 must not reload pages"
    );
    let mut graph: serde_json::Value = serde_json::from_str(&fixture("graph.json")).unwrap();
    graph["records"]["sc-d1"]["title"] = serde_json::json!("Fresh graph title");
    std::fs::write(s.tools.join("graph.json"), graph.to_string()).unwrap();
    wait_for(&server, "changed graph was not refreshed", |_, body| {
        serde_json::from_str::<serde_json::Value>(body).unwrap()["revision"] != initial["revision"]
    });
    assert!(
        get(&server, "/scroll/sc-d1")
            .1
            .contains("Fresh graph title")
    );
    std::fs::write(s.tools.join("mode"), "invalid").unwrap();
    wait_for(&server, "malformed response was accepted", |status, _| {
        status == 503
    });
    let warning = get(&server, "/scroll/sc-d1").1;
    assert!(warning.contains("Fresh graph title"));
    assert!(warning.contains("Scrolls read failed"));
    std::fs::write(s.tools.join("mode"), "").unwrap();
    wait_for(&server, "recovery did not clear warning", |status, _| {
        status == 200
    });
    assert!(!get(&server, "/").1.contains("Scrolls read failed"));
    let requests = std::fs::read_to_string(s.tools.join("requests.jsonl")).unwrap();
    assert!(
        requests
            .lines()
            .skip(2)
            .any(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["etag"].is_string())
    );
    let pid = std::fs::read_to_string(s.tools.join("pid")).unwrap();
    // Existing tests kill Fabriscope outright; exercise orderly termination too.
    let status = Command::new("kill")
        .args(["-TERM", &server.child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    #[cfg(target_os = "linux")]
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "owned server survived shutdown"
    );
}

#[test]
fn scrolls_attachment_is_not_owned_and_incompatible_servers_fail_clearly() {
    let s = store("scrolls-attached");
    let mut child = Command::new(s.tools.join("scrolls"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    std::thread::spawn(move || {
        let mut sink = std::io::sink();
        let _ = std::io::copy(&mut stdout, &mut sink);
    });
    let event: serde_json::Value = serde_json::from_str(&line).unwrap();
    let url = format!("http://{}", event["address"].as_str().unwrap());
    let server = start(&["--scrolls-url", &url]);
    assert!(
        get(&server, "/scroll/sc-d1")
            .1
            .contains("Use Dolt for history")
    );
    drop(server);
    assert!(
        child.try_wait().unwrap().is_none(),
        "attached server was terminated"
    );
    std::fs::write(s.tools.join("mode"), "legacy").unwrap();
    let out = run(
        Path::new(env!("CARGO_BIN_EXE_fabriscope")),
        &["--scrolls-url", &url],
        &[("FABRISCOPE_NO_UPDATE", "1")],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Upgrade Scrolls"));
    assert!(child.try_wait().unwrap().is_none());
    child.kill().unwrap();
    child.wait().unwrap();
    for mode in ["legacy", "bad-start", "exit", "invalid"] {
        std::fs::write(s.tools.join("mode"), mode).unwrap();
        let out = run(
            Path::new(env!("CARGO_BIN_EXE_fabriscope")),
            &[
                "--scrolls",
                s.scrolls.to_str().unwrap(),
                "--scrolls-bin",
                s.tools.join("scrolls").to_str().unwrap(),
            ],
            &[("FABRISCOPE_NO_UPDATE", "1")],
        );
        assert!(!out.status.success(), "{mode}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(if mode == "invalid" {
                "Invalid Scrolls graph"
            } else {
                "Upgrade Scrolls"
            }),
            "{mode}: {:?}",
            out
        );
        #[cfg(target_os = "linux")]
        {
            let pid = std::fs::read_to_string(s.tools.join("pid")).unwrap();
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "failed startup leaked {pid}"
            );
        }
    }
}
