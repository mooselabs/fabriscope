//! The run feed through the real binary (WP5): a directory feed and the same
//! feed over HTTP from a small server in this file, lazy run docs and
//! document pages, a failing poll that keeps the last good read, a digest
//! change that fetches a run again, and hrefs that try to leave the feed
//! root.
//!
//! tests/fixtures/runs-extra/ is a feed that is wrong on purpose: four runs
//! name hrefs that try to leave the root (`../`, an absolute path,
//! percent-encoded `../`, and a symlink this test creates), and one run,
//! its subject and its document carry names that need percent-encoding.

mod common;

use common::{Server, get, repo_path, start_program, work_store};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Starts Fabriscope on the work fixture with `--runs runs` (and any
/// extra arguments), polling the feed every 200 ms.
fn start_with(name: &str, runs: &str, extra: &[&str]) -> Server {
    let s = work_store(name);
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let beads = s.beads.to_str().unwrap().to_string();
    let bd = s.tools.join("bd").to_str().unwrap().to_string();
    let profile = repo_path("tests/fixtures/work/profile.toml");
    let mut args = vec![
        "--beads",
        &beads,
        "--bd",
        &bd,
        "--profile",
        profile.to_str().unwrap(),
        "--runs",
        runs,
    ];
    args.extend(extra);
    start_program(
        Path::new(env!("CARGO_BIN_EXE_fabriscope")),
        &args,
        &[
            ("FABRISCOPE_NO_UPDATE", "1"),
            ("FABRISCOPE_RUNS_POLL_MS", "200"),
        ],
    )
}

/// Reads `path` until `check` holds, for up to 45 s; the last body.
fn until(server: &Server, path: &str, what: &str, check: impl Fn(u16, &str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let (status, body) = get(server, path);
        if check(status, &body) {
            return body;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: {path} answered {status}: {body}\n{}",
            server.stderr.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A copy of a fixture feed in a scratch directory a test may change.
fn copy_feed(fixture: &str, name: &str) -> PathBuf {
    let root = common::scratch(name);
    let feed = root.join("feed");
    copy_dir(&repo_path(fixture), &feed);
    feed
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A feed served over HTTP the way a static file server would, under
/// `/feed/`: each path segment percent-decoded once and joined to the root
/// as it is (so a reader that did not encode a `%2F` or `%2E%2E` would reach
/// outside), with ETags on `feed.json`.
struct FeedServer {
    url: String,
    paths: Arc<Mutex<Vec<String>>>,
    auth: Arc<Mutex<Vec<String>>>,
    fail: Arc<AtomicBool>,
}

fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&segment[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn serve_feed(root: PathBuf) -> FeedServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/feed", listener.local_addr().unwrap());
    let paths: Arc<Mutex<Vec<String>>> = Arc::default();
    let auth: Arc<Mutex<Vec<String>>> = Arc::default();
    let fail = Arc::new(AtomicBool::new(false));
    let (seen, tokens, failing) = (paths.clone(), auth.clone(), fail.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (root, seen, tokens, failing) =
                (root.clone(), seen.clone(), tokens.clone(), failing.clone());
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = Vec::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    head.push(line.trim_end().to_string());
                }
                let target = head
                    .first()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("")
                    .to_string();
                let header = |name: &str| {
                    head.iter().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
                    })
                };
                if let Some(value) = header("authorization") {
                    tokens.lock().unwrap().push(value);
                }
                seen.lock().unwrap().push(target.clone());
                let reply = |stream: &mut std::net::TcpStream,
                             status: &str,
                             extra: &str,
                             body: &[u8]| {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(body);
                };
                if failing.load(Ordering::SeqCst) {
                    return reply(
                        &mut stream,
                        "500 Internal Server Error",
                        "",
                        b"feed host is down",
                    );
                }
                let Some(rest) = target.strip_prefix("/feed/") else {
                    return reply(&mut stream, "404 Not Found", "", b"no");
                };
                let decoded: Vec<String> = rest.split('/').map(decode).collect();
                let path = root.join(decoded.join("/"));
                match std::fs::read(&path) {
                    Ok(body) => {
                        let tag = format!(
                            "\"{}-{}\"",
                            body.len(),
                            body.iter().map(|b| *b as u64).sum::<u64>()
                        );
                        if rest == "feed.json"
                            && header("if-none-match").as_deref() == Some(tag.as_str())
                        {
                            return reply(
                                &mut stream,
                                "304 Not Modified",
                                &format!("ETag: {tag}\r\n"),
                                b"",
                            );
                        }
                        reply(
                            &mut stream,
                            "200 OK",
                            &format!("ETag: {tag}\r\nContent-Type: application/json\r\n"),
                            &body,
                        )
                    }
                    Err(_) => reply(&mut stream, "404 Not Found", "", b"not found"),
                }
            });
        }
    });
    FeedServer {
        url,
        paths,
        auth,
        fail,
    }
}

fn run_count(health: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(health).ok()?;
    value["runs"]["runs"].as_u64()
}

/// What every source of the golden feed must show.
fn golden_pages(server: &Server) {
    until(server, "/healthz", "the feed is read", |s, h| {
        s == 200 && run_count(h) == Some(5)
    });
    let (status, list) = get(server, "/runs");
    assert_eq!(status, 200);
    for run in [
        "run%2D0001",
        "run%2D0002",
        "run%2D0003",
        "run%2D0004",
        "run%2D0005",
    ] {
        assert!(
            list.contains(&format!("href=\"/run/{run}\"")),
            "{run}: {list}"
        );
    }
    let (_, first) = get(server, "/run/run-0002");
    assert!(
        first.contains("Fetching run…"),
        "the first visit does not wait: {first}"
    );
    let full = until(server, "/run/run-0002", "the run doc arrives", |_, b| {
        b.contains("id=\"attempt-2\"")
    });
    assert!(full.contains("href=\"/run/run%2D0002/doc/transcript\""));
    let log = until(
        server,
        "/run/run-0002/doc/gate-log",
        "the log page arrives",
        |_, b| b.contains("id=\"L10\""),
    );
    assert!(log.contains("href=\"#L1\">1</a>"));
    let turns = until(
        server,
        "/run/run-0002/doc/transcript?page=1",
        "the transcript page arrives",
        |_, b| b.contains("class=\"transcript\""),
    );
    assert!(turns.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!turns.contains("<script>alert"), "never a tag");
    let stopped = until(
        server,
        "/run/run-0003",
        "the stopped run arrives",
        |_, b| b.contains("id=\"attempt-1\""),
    );
    assert!(stopped.contains("withheld"));
    assert!(
        !stopped.contains("/run/run%2D0003/doc/transcript"),
        "a withheld document has no link"
    );
    let board = until(server, "/boards/stop-ratio", "the board arrives", |_, b| {
        b.contains("Stops per merge")
    });
    assert!(board.contains("stops_per_merge"));
    assert_eq!(get(server, "/run/nope").0, 404);
    assert_eq!(get(server, "/boards/nope").0, 404);
    assert_eq!(get(server, "/run/run-0002/doc/nope").0, 404);
}

#[test]
fn a_directory_feed_lists_the_runs_and_fills_pages_lazily() {
    let golden = repo_path("tests/fixtures/runfeed/v1");
    let server = start_with("runs-dir", golden.to_str().unwrap(), &[]);
    golden_pages(&server);
}

#[test]
fn an_http_feed_reads_the_same_with_a_rotating_token() {
    let feed = copy_feed("tests/fixtures/runfeed/v1", "runs-http");
    let http = serve_feed(feed.clone());
    let token = feed.parent().unwrap().join("token");
    std::fs::write(&token, "tok-1\n").unwrap();
    let server = start_with(
        "runs-http-beads",
        &http.url,
        &["--runs-token-file", token.to_str().unwrap()],
    );
    golden_pages(&server);
    assert!(
        http.auth
            .lock()
            .unwrap()
            .iter()
            .all(|a| a == "Bearer tok-1")
    );
    std::fs::write(&token, "tok-2\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !http
        .auth
        .lock()
        .unwrap()
        .iter()
        .any(|a| a == "Bearer tok-2")
    {
        assert!(Instant::now() < deadline, "the token file is read again");
        std::thread::sleep(Duration::from_millis(100));
    }
    let paths = http.paths.lock().unwrap().clone();
    assert!(
        paths
            .iter()
            .any(|p| p == "/feed/docs/run-0002/transcript/0002.ndjson")
    );
    assert!(
        paths.iter().filter(|p| *p == "/feed/index.json").count() <= 2,
        "an unchanged revision does not reread the index: {paths:?}"
    );
}

#[test]
fn a_failing_poll_keeps_the_runs_and_a_new_digest_refetches() {
    let feed = copy_feed("tests/fixtures/runfeed/v1", "runs-fail");
    let http = serve_feed(feed.clone());
    let server = start_with("runs-fail-beads", &http.url, &[]);
    until(&server, "/healthz", "the feed is read", |s, h| {
        s == 200 && run_count(h) == Some(5)
    });
    until(&server, "/run/run-0002", "the run doc arrives", |_, b| {
        b.contains("Files changed")
    });
    http.fail.store(true, Ordering::SeqCst);
    let health = until(&server, "/healthz", "a failed poll is unhealthy", |s, _| {
        s == 503
    });
    assert!(health.contains("Run feed read failed"), "{health}");
    assert_eq!(run_count(&health), Some(5), "the last good index stays");
    let (_, list) = get(&server, "/runs");
    assert!(list.contains("href=\"/run/run%2D0005\""));
    assert!(list.contains("class=\"alert\""), "the page warns");
    assert!(list.contains("feed host is down"));
    http.fail.store(false, Ordering::SeqCst);
    until(
        &server,
        "/healthz",
        "the next good poll recovers",
        |s, _| s == 200,
    );
    // Publish revision 43 with a changed run-0002 doc and digest.
    let edit = |file: &str, from: &str, to: &str| {
        let path = feed.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "{file}: {from}");
        std::fs::write(&path, text.replace(from, to)).unwrap();
    };
    let old = "91e4e0fdc43a654d0e52c6c900a55ad7926a01a6043069ef7366ff01a9ddc3eb";
    let new = "1111111111111111111111111111111111111111111111111111111111111111";
    edit("runs/run-0002.json", "Files changed", "Files touched");
    edit("runs/run-0002.json", old, new);
    edit("index.json", old, new);
    edit("feed.json", "\"revision\": 42", "\"revision\": 43");
    until(&server, "/healthz", "the new revision is read", |_, h| {
        serde_json::from_str::<serde_json::Value>(h).is_ok_and(|v| v["runs"]["revision"] == 43)
    });
    until(
        &server,
        "/run/run-0002",
        "the changed run is fetched again",
        |_, b| b.contains("Files touched"),
    );
}

/// The extra feed's oddly named run and its escapes, through `runs`.
fn odd_names_and_escapes(server: &Server) {
    until(server, "/healthz", "the extra feed is read", |s, h| {
        s == 200 && run_count(h) == Some(5)
    });
    let (_, list) = get(server, "/runs");
    assert!(list.contains("href=\"/run/odd%20run%2F1\""), "{list}");
    assert!(list.contains("Odd &lt;names&gt; &amp; co"));
    let odd = until(
        server,
        "/run/odd%20run%2F1",
        "the odd run arrives",
        |_, b| b.contains("id=\"attempt-1\""),
    );
    assert!(
        odd.contains("href=\"/run/odd%20run%2F1/doc/notes\""),
        "{odd}"
    );
    assert!(odd.contains("WEIRD a/b?#ü"));
    let notes = until(
        server,
        "/run/odd%20run%2F1/doc/notes",
        "the odd notes arrive",
        |_, b| b.contains("id=\"L2\""),
    );
    assert!(notes.contains("&lt;b&gt;not bold&lt;/b&gt;"));
    for (run, why) in [
        ("escape-dotdot", "leaves the feed root"),
        ("escape-abs", "leaves the feed root"),
        ("escape-pct", "could not be read"),
    ] {
        let page = until(
            server,
            &format!("/run/{run}"),
            "the escape is refused",
            |_, b| b.contains("could not be read"),
        );
        assert!(page.contains(why), "{run}: {page}");
        assert!(!page.contains("SECRET-OUTSIDE"), "{run}");
    }
}

/// The extra feed in a scratch directory, with a secret beside it that each
/// escaping href aims at, and a symlink inside that points to it.
fn extra_feed(name: &str) -> PathBuf {
    let feed = copy_feed("tests/fixtures/runs-extra", name);
    let secret = r#"{"schema":"runfeed.run.v1","run":"escape-link","subject":{"id":"x","title":"SECRET-OUTSIDE"},"attempts":[{"id":"a","number":1,"state":"passed","steps":[]}]}"#;
    std::fs::write(feed.parent().unwrap().join("secret.json"), secret).unwrap();
    std::os::unix::fs::symlink(
        feed.parent().unwrap().join("secret.json"),
        feed.join("runs/link.json"),
    )
    .unwrap();
    feed
}

#[test]
fn a_directory_feed_round_trips_odd_names_and_refuses_escapes() {
    let feed = extra_feed("runs-extra-dir");
    let server = start_with("runs-extra-dir-beads", feed.to_str().unwrap(), &[]);
    odd_names_and_escapes(&server);
    let link = until(
        &server,
        "/run/escape-link",
        "the symlink is refused",
        |_, b| b.contains("could not be read"),
    );
    assert!(link.contains("leaves the feed root"), "{link}");
    assert!(!link.contains("SECRET-OUTSIDE"));
}

#[test]
fn an_http_feed_encodes_every_segment() {
    let feed = extra_feed("runs-extra-http");
    let http = serve_feed(feed);
    let server = start_with("runs-extra-http-beads", &http.url, &[]);
    odd_names_and_escapes(&server);
    let paths = http.paths.lock().unwrap().clone();
    for wanted in [
        "/feed/subjects/WEIRD%2520a%252Fb%253F%2523%25C3%25BC.json",
        "/feed/runs/odd%2520run%252F1.json",
        "/feed/docs/odd%2520run%252F1/notes/0001.txt",
        "/feed/runs/%252E%252E%252F%252E%252E%252Fsecret.json",
    ] {
        assert!(paths.iter().any(|p| p == wanted), "{wanted}: {paths:?}");
    }
    assert!(
        paths
            .iter()
            .all(|p| !p.contains("..") && !p.contains("passwd")),
        "refused hrefs are never requested: {paths:?}"
    );
}

#[test]
fn without_a_feed_the_run_pages_are_not_found() {
    let s = work_store("runs-none");
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let server = common::start(&[
        "--beads",
        s.beads.to_str().unwrap(),
        "--bd",
        s.tools.join("bd").to_str().unwrap(),
    ]);
    for path in [
        "/runs",
        "/run/run-0002",
        "/run/run-0002/doc/transcript",
        "/boards",
        "/boards/stop-ratio",
    ] {
        assert_eq!(get(&server, path).0, 404, "{path}");
    }
}
