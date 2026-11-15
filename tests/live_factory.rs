//! The factory pages (`/held`, `/costs`, `/receipts`) and the provenance
//! sweeper through the real binary, on the work-ledger fixture with its
//! profile and the golden run feed, and with no profile at all.

mod common;

use common::{Server, get, repo_path, start, wait_for, work_store};
use std::time::{Duration, Instant};

fn work_args(s: &common::Store) -> Vec<String> {
    vec![
        "--beads".into(),
        s.beads.to_str().unwrap().into(),
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
    ]
}

/// Reads `path` until it satisfies `check`, for up to 45 s.
fn page_until(server: &Server, path: &str, check: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let (status, page) = get(server, path);
        if status == 200 && check(&page) {
            return page;
        }
        assert!(Instant::now() < deadline, "{path}: {status} {page}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn held_costs_and_receipts_on_the_work_fixture() {
    let s = work_store("factory");
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let args = work_args(&s);
    let server = start(&args.iter().map(String::as_str).collect::<Vec<_>>());
    // Startup reads the bead in progress; the sweeper then reads the held
    // one without any page asking.
    wait_for(&server, "the sweeper did not read ahead", |status, h| {
        let h: serde_json::Value = serde_json::from_str(h).unwrap_or_default();
        status == 200 && h["provenance"]["read"].as_u64().unwrap_or(0) >= 2
    });
    let held = page_until(&server, "/held", |p| p.contains("<code>att-0003-1</code>"));
    assert!(held.contains("Move reports to the new table"), "{held}");
    assert!(
        held.contains("id=\"held-held-scope%2Dexceeded\""),
        "grouped by stop code"
    );
    assert!(held.contains("since <time datetime=\"2026-11-15T09:15:00Z\""));
    assert!(held.contains("href=\"/run/run%2D0003\""));
    assert!(
        held.contains("The change needs a file outside its write scope")
            || (held.contains("stop scope-exceeded") && held.contains("not read yet")),
        "run-0003's stop, or its code until the run doc is read: {held}"
    );
    let by_cause = get(&server, "/held?by=work.suspended.cause").1;
    assert!(
        by_cause.contains("id=\"held-held-scope\""),
        "grouped by cause"
    );
    assert!(by_cause.contains("Move reports to the new table"));
    // Visiting wk-2 reads its provenance; its two attempts are both in
    // run-0002, so the feed's figure counts and provenance's is replaced.
    get(&server, "/bead/wk-2");
    let costs = page_until(
        &server,
        "/costs?q=rounding&from=2026-11-01&to=2026-11-30",
        |p| p.contains("2 attempts, not counted again"),
    );
    assert!(
        costs.contains(
            "Cost known for <strong>1</strong> of <strong>1</strong> items closed in range (3 attempts)"
        ),
        "{costs}"
    );
    assert!(costs.contains("<strong>$5.37</strong>"), "{costs}");
    assert!(
        !costs.contains("$7.47"),
        "an attempt is never counted twice"
    );
    assert!(costs.contains("href=\"/run/run%2D0002\""));
    let (status, receipts) = get(&server, "/receipts?kind=gate");
    assert_eq!(status, 200);
    assert!(
        receipts.contains("href=\"/bead/wk%2Drc%2Dgate\""),
        "{receipts}"
    );
    assert!(receipts.contains("Fix rounding in invoice totals"));
    assert!(receipts.contains("<code>att-0002-2</code><small><a href=\"/run/run%2D0002\">"));
    assert!(!receipts.contains("href=\"/bead/wk%2Drc%2Dship\""));
    for page in [&held, &costs, &receipts] {
        assert!(!page.contains(" style="));
    }
}

#[test]
fn the_factory_pages_explain_themselves_without_a_profile() {
    let s = work_store("factory-bare");
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let server = start(&[
        "--beads",
        s.beads.to_str().unwrap(),
        "--bd",
        s.tools.join("bd").to_str().unwrap(),
    ]);
    for (path, says) in [
        ("/held", "No state in the profile holds work back"),
        ("/receipts", "The profile defines no receipts"),
        ("/costs", "Cost known for"),
    ] {
        let (status, page) = get(&server, path);
        assert_eq!(status, 200, "{path}");
        assert!(page.contains(says), "{path}: {page}");
    }
    let costs = get(&server, "/costs").1;
    assert!(costs.contains("Run feed</strong>: not configured"));
    assert!(!costs.contains("$0.00"), "nothing recorded is not $0");
}
