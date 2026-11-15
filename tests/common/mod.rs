//! The harness the live tests share: fake `bd` and `scrolls` programs, a
//! fake Beads API backed by files a test rewrites, and helpers that start the
//! real binary and read its pages over HTTP.

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
  provenance)
    # bd provenance log --json -- <id>
    if [ -f "provenance-$5.json" ]; then cat "provenance-$5.json"; else echo '[]'; fi ;;
  *) echo "unexpected bd $*" >&2; exit 2 ;;
esac
"#;

const FAKE_SCROLLS: &str = include_str!("../fixtures/scrolls.py");

pub struct Server {
    pub child: Child,
    pub address: String,
    pub stderr: Arc<Mutex<String>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fabriscope-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn script(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// A Beads store on disk plus the fake tools.
pub struct Store {
    pub beads: PathBuf,
    pub tools: PathBuf,
    pub scrolls: PathBuf,
}

pub fn store(name: &str) -> Store {
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
    std::fs::write(
        beads.join("provenance-demo-1.json"),
        fixture("provenance.json"),
    )
    .unwrap();
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
pub fn fake_api(beads: PathBuf) -> (String, Arc<Mutex<Vec<String>>>) {
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

pub fn start(args: &[&str]) -> Server {
    // Never let a test update the build output in target/.
    start_program(
        Path::new(env!("CARGO_BIN_EXE_fabriscope")),
        args,
        &[("FABRISCOPE_NO_UPDATE", "1")],
    )
}

pub fn start_program(program: &Path, args: &[&str], env: &[(&str, &str)]) -> Server {
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

pub fn get(server: &Server, path: &str) -> (u16, String) {
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

pub fn wait_for(server: &Server, what: &str, check: impl Fn(u16, &str) -> bool) {
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

pub fn run(program: &Path, args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    retry_busy(program, || {
        Command::new(program)
            .args(args)
            .env_remove("FABRISCOPE_NO_UPDATE")
            .envs(env.iter().copied())
            .output()
    })
}
