//! History threads, /activity and the people pages through the real binary:
//! threads from `bd export` (directly and as a bulk read beside `bd serve`),
//! per-issue reads when only `bd serve` is there, and the filters, paging
//! and person names that must survive a URL.

mod common;

use common::{Store, fake_api, get, repo_path, start, wait_for, work_store};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

fn args(s: &Store, beads: Vec<String>) -> Vec<String> {
    let mut args = beads;
    args.extend([
        "--bd".into(),
        s.tools.join("bd").to_str().unwrap().into(),
        "--profile".into(),
        repo_path("tests/fixtures/work/profile.toml")
            .to_str()
            .unwrap()
            .into(),
        "--runs".into(),
        repo_path("tests/fixtures/runfeed/v1")
            .to_str()
            .unwrap()
            .into(),
    ]);
    args
}

fn threads_read(health: &str) -> u64 {
    let value: serde_json::Value = serde_json::from_str(health).unwrap_or_default();
    value["history"]["read"].as_u64().unwrap_or(0)
}

/// The work fixture plus one more item carrying 130 plain comments by
/// alice and one by bob, so alice's activity spans two pages.
fn busy_store(name: &str) -> Store {
    let s = work_store(name);
    let comments: Vec<serde_json::Value> = (0..130)
        .map(|n| {
            serde_json::json!({
                "id": format!("c{n}"),
                "issue_id": "wk-busy",
                "author": "alice",
                "text": format!("Note number {n} from alice"),
                "created_at": format!("2026-11-02T{:02}:{:02}:00Z", n / 60, n % 60),
            })
        })
        .chain([serde_json::json!({
            "id": "cb",
            "issue_id": "wk-busy",
            "author": "bob",
            "text": "A word from bob",
            "created_at": "2026-11-03T00:00:00Z",
        })])
        .collect();
    let busy = serde_json::json!({
        "_type": "issue", "id": "wk-busy", "title": "A busy item", "status": "open",
        "priority": 3, "issue_type": "task", "created_at": "2026-11-02T00:00:00Z",
        "updated_at": "2026-11-03T00:00:00Z", "comment_count": comments.len(),
        "comments": comments,
    });
    let path = s.beads.join("issues.jsonl");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(&busy.to_string());
    text.push('\n');
    std::fs::write(path, text).unwrap();
    s
}

#[test]
fn threads_come_from_the_export_and_activity_filters_and_pages() {
    let s = busy_store("history-export");
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let args = args(&s, vec!["--beads".into(), s.beads.to_str().unwrap().into()]);
    let server = start(&args.iter().map(String::as_str).collect::<Vec<_>>());
    wait_for(&server, "no history threads", |status, h| {
        status == 200 && threads_read(h) > 0
    });

    // Alice only, over two pages of 100.
    let (status, first) = get(&server, "/activity?person=alice&from=2026-10-01");
    assert_eq!(status, 200);
    assert!(first.contains("page 1 of 2"), "{first}");
    assert!(first.contains("Note number 129 from alice"), "newest first");
    assert!(!first.contains("A word from bob"), "filtered to alice");
    assert!(!first.contains("Note number 0 from alice"), "on page 2");
    assert!(first.contains("page=1"), "a next link");
    let (_, second) = get(&server, "/activity?person=alice&from=2026-10-01&page=1");
    assert!(second.contains("page 2 of 2"));
    assert!(second.contains("Note number 0 from alice"));
    assert!(
        second.contains("Leaning to RFC 4180."),
        "alice on another item"
    );

    // A comment without the marker is a plain comment; an event is not.
    let (_, plain) = get(&server, "/activity?event=comment&from=2026-10-01");
    assert!(plain.contains("Looks right to me {mostly}; check the **header row**."));
    assert!(!plain.contains("work.event.v1"));
    let (_, held) = get(&server, "/activity?event=suspended&from=2026-10-01");
    assert!(held.contains(">Held</span>"), "the profile's label");
    assert!(!held.contains("Looks right to me"));

    // A seat's name survives the URL and is classed apart from people.
    let (status, seat) = get(&server, "/person/factory%2FhostA%2FrigA?from=2026-10-01");
    assert_eq!(status, 200, "{seat}");
    assert!(seat.contains("Factory · seat"));
    assert!(seat.contains("href=\"/by/rig/rigA\""));
    let (status, _) = get(&server, "/person/nobody-at-all");
    assert_eq!(status, 404);
    let (_, people) = get(&server, "/people?from=2026-10-01");
    let humans = people.find("id=\"class-person\"").unwrap();
    let seats = people.find("id=\"class-seat\"").unwrap();
    for name in ["alice", "bob"] {
        let at = people.find(&format!("href=\"/person/{name}\"")).unwrap();
        assert!(humans < at && at < seats, "{name} among people");
    }
    let at = people
        .find("href=\"/person/factory%2FhostA%2FrigA\"")
        .unwrap();
    assert!(at > seats, "the seat among seats");
}

#[test]
fn beside_bd_serve_the_history_is_read_in_bulk() {
    let s = work_store("history-bulk");
    let (url, _) = fake_api(s.beads.clone());
    std::fs::write(s.beads.join("serve-url"), &url).unwrap();
    let args = args(&s, vec!["--beads".into(), s.beads.to_str().unwrap().into()]);
    let server = start(&args.iter().map(String::as_str).collect::<Vec<_>>());
    wait_for(&server, "the bulk read did not finish", |status, h| {
        status == 200 && threads_read(h) == 14 && h.contains("\"reading\":0")
    });
    let (_, activity) = get(&server, "/activity?event=comment&from=2026-10-01");
    assert!(activity.contains("Leaning to RFC 4180."), "{activity}");
}

/// A proxy in front of the fake API that records each request's path.
fn recording_proxy(upstream: String) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = upstream.trim_start_matches("http://").to_string();
    let paths = Arc::new(Mutex::new(Vec::new()));
    let seen = paths.clone();
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let Ok(mut client) = client else { continue };
            let upstream = upstream.clone();
            let seen = seen.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(client.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                if let Some(path) = head.split_whitespace().nth(1) {
                    seen.lock().unwrap().push(path.to_string());
                }
                let mut server = TcpStream::connect(&upstream).unwrap();
                server.write_all(head.as_bytes()).unwrap();
                let mut response = Vec::new();
                let _ = server.read_to_end(&mut response);
                let _ = client.write_all(&response);
            });
        }
    });
    (url, paths)
}

#[test]
fn without_a_local_bd_a_visit_reads_that_issue_only() {
    let s = work_store("history-api");
    let (api, _) = fake_api(s.beads.clone());
    let (url, paths) = recording_proxy(api);
    let mut args = args(&s, vec!["--beads-url".into(), url]);
    // No local bd at all: drop `--bd`.
    let at = args.iter().position(|a| a == "--bd").unwrap();
    args.drain(at..at + 2);
    let server = start(&args.iter().map(String::as_str).collect::<Vec<_>>());
    let issue_reads = || -> Vec<String> {
        paths
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.starts_with("/v0/beads/issues/"))
            .cloned()
            .collect()
    };
    // Read ahead: the item in progress and the held one.
    wait_for(&server, "read ahead", |status, h| {
        status == 200 && threads_read(h) == 2
    });
    let ahead = issue_reads();
    assert_eq!(ahead.len(), 2, "{ahead:?}");
    assert!(ahead.iter().all(|p| p.ends_with("?include_comments=true")));
    assert!(
        ahead
            .iter()
            .any(|p| p.starts_with("/v0/beads/issues/wk-1?"))
    );
    assert!(
        ahead
            .iter()
            .any(|p| p.starts_with("/v0/beads/issues/wk-3?"))
    );

    let (status, _) = get(&server, "/bead/wk-q1");
    assert_eq!(status, 200);
    wait_for(&server, "the visit's read", |_, h| threads_read(h) == 3);
    assert_eq!(
        issue_reads()[2..],
        ["/v0/beads/issues/wk-q1?include_comments=true".to_string()]
    );
    let (_, activity) = get(&server, "/activity?from=2026-10-01");
    assert!(activity.contains("Leaning to RFC 4180."));
    assert!(
        !activity.contains("status: working → done"),
        "wk-2 was neither visited nor read ahead"
    );
    // A second visit inside the freshness window reads nothing.
    get(&server, "/bead/wk-q1");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(issue_reads().len(), 3, "{:?}", issue_reads());
}
