//! Runs the real binary against fake `bd` and `scrolls` programs and reads
//! its pages over HTTP. The fake `bd serve` points Fabriscope at a small
//! fake Beads API that this test runs, backed by files the test rewrites.

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

const FAKE_SCROLLS: &str = r#"#!/bin/sh
# Writes graph.json where `render --out DIR` asks for it.
graph="$(dirname "$0")/graph.json"
while [ $# -gt 0 ]; do
  if [ "$1" = "--out" ]; then out="$2"; fi
  shift
done
[ -n "$out" ] || { echo "no --out" >&2; exit 2; }
mkdir -p "$out/scrolls"
cp "$graph" "$out/scrolls/graph.json"
printf 'Scrolls test\nDolt revision: fixture-rev\n' > "$out/RENDERED"
"#;

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
    let mut child = Command::new(env!("CARGO_BIN_EXE_fabriscope"))
        .args(args)
        .args(["--bind", "127.0.0.1:0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
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
    let deadline = Instant::now() + Duration::from_secs(20);
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
    let (_, blocked) = get(server, "/bead/demo-2");
    assert!(blocked.contains("badge waiting"));
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
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("give --beads DIR (or --beads-url URL), --scrolls STORE, or both")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_fabriscope"))
        .args(["--beads-url", "http://127.0.0.1:1", "--bind", "127.0.0.1:0"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "an unreachable bd serve is an error at startup"
    );
}
