//! The work-ledger fixture through the real binary, with its profile and the
//! golden run feed: the CLI flags, `profile check`, every navigation tab and
//! stub route, and the list read that asks for gates.
//!
//! A package adds its own `tests/live_<area>.rs` the same way: `mod common;`,
//! `common::work_store`, `common::start` with the arguments below.

mod common;

use common::{fake_api, get, repo_path, start, wait_for, work_store};
use std::process::Command;

fn work_args(s: &common::Store, beads: Vec<String>) -> Vec<String> {
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
        "--title".into(),
        "Work".into(),
    ]);
    args
}

#[test]
fn profile_check_validates_and_names_the_bad_key() {
    let good = Command::new(env!("CARGO_BIN_EXE_fabriscope"))
        .args(["profile", "check"])
        .arg(repo_path("tests/fixtures/work/profile.toml"))
        .output()
        .unwrap();
    assert!(
        good.status.success(),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    let stdout = String::from_utf8_lossy(&good.stdout);
    assert!(
        stdout.contains("a valid fabriscope.profile.v1 profile"),
        "{stdout}"
    );
    assert!(stdout.contains("\"overrides_ready\": true"));
    let bad = common::scratch("bad-profile").join("p.toml");
    std::fs::write(&bad, "schema = \"fabriscope.profile.v1\"\n[[groups]]\nkey = \"q\"\nlabel = \"Q\"\nfrom = \"x\"\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_fabriscope"))
        .args(["profile", "check"])
        .arg(&bad)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("groups[0].key"));
}

#[test]
fn every_tab_and_stub_route_answers_through_bd_export() {
    let s = work_store("work-export");
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let args = work_args(&s, vec!["--beads".into(), s.beads.to_str().unwrap().into()]);
    let server = start(&args.iter().map(String::as_str).collect::<Vec<_>>());
    let (status, health) = get(&server, "/healthz");
    assert_eq!(status, 200, "{health}");
    assert!(
        health.contains("\"name\":\"Work ledger (fixture)\""),
        "{health}"
    );
    assert!(
        health.contains("\"revision\":42"),
        "the run feed is read: {health}"
    );
    for path in [
        "/",
        "/beads?state=all",
        "/epics",
        "/by/milestone",
        "/by/milestone/v1",
        "/search?q=csv",
        "/bead/wk-1",
        "/bead/wk-4",
        "/people",
        "/person/factory%2FhostA%2FrigA",
        "/activity",
        "/flow",
        "/flow/created",
        "/flow/claimed",
        "/flow/cycle",
        "/flow/ageing",
        "/flow/decisions",
        "/runs",
        "/run/run-0002",
        "/run/run-0002/doc/transcript?page=1",
        "/boards",
        "/boards/stop-ratio",
        "/held",
        "/costs",
        "/receipts",
    ] {
        let (status, page) = get(&server, path);
        assert_eq!(status, 200, "{path}");
        assert!(!page.contains(" style="), "{path}");
    }
    let (_, list) = get(&server, "/beads?state=all");
    assert!(
        list.contains("Owner approves the export format"),
        "the gate is listed"
    );
    let (_, flow) = get(&server, "/flow/cycle");
    for tab in ["/flow/created", "/flow/decisions"] {
        assert!(
            flow.contains(&format!("href=\"{tab}\"")),
            "{tab} in the Flow tabs"
        );
    }
}

#[test]
fn the_list_api_is_asked_for_gates() {
    let s = work_store("work-api");
    let (url, _) = fake_api(s.beads.clone());
    let args = work_args(&s, vec!["--beads-url".into(), url]);
    let server = start(&args.iter().map(String::as_str).collect::<Vec<_>>());
    wait_for(&server, "the store was not read", |status, h| {
        status == 200 && h.contains("\"beads\":14")
    });
    assert!(
        get(&server, "/bead/wk-4")
            .1
            .contains("Owner approves the export format"),
        "a gate-type issue arrives through the list API"
    );
}
