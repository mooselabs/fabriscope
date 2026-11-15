//! The flow pages through the real binary: the work fixture with its
//! profile and the golden run feed, and the demo store with no profile.

mod common;

use common::{get, repo_path, start, store, work_store};

const PAGES: [&str; 6] = [
    "/flow",
    "/flow/created",
    "/flow/claimed",
    "/flow/cycle",
    "/flow/ageing",
    "/flow/decisions",
];

/// A range that holds every fixture date, whatever day the test runs.
const RANGE: &str = "from=2026-09-01&to=2026-11-30";

fn work_server(name: &str) -> common::Server {
    let s = work_store(name);
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let profile = repo_path("tests/fixtures/work/profile.toml");
    let runs = repo_path("tests/fixtures/runfeed/v1");
    start(&[
        "--beads",
        s.beads.to_str().unwrap(),
        "--bd",
        s.tools.join("bd").to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
        "--runs",
        runs.to_str().unwrap(),
        "--title",
        "Work",
    ])
}

#[test]
fn every_flow_page_answers_and_its_tabs_carry_the_filters() {
    let server = work_server("flow-work");
    for path in PAGES {
        let (status, page) = get(&server, &format!("{path}?milestone=v1&{RANGE}"));
        assert_eq!(status, 200, "{path}");
        assert!(!page.contains(" style="), "{path}");
        assert!(page.contains("<div class=\"flow\">"), "{path}");
        // Each of the six tabs links with the same filters.
        for tab in PAGES {
            let wanted = format!(
                "href=\"{tab}?from=2026%2D09%2D01&amp;to=2026%2D11%2D30&amp;milestone=v1\""
            );
            assert!(page.contains(&wanted), "{path} → {tab}");
        }
    }
    // The breakdown key travels between flow tabs but never leaves them.
    let (_, overview) = get(&server, &format!("/flow?by=rig&{RANGE}"));
    assert!(
        overview
            .contains("href=\"/flow/cycle?from=2026%2D09%2D01&amp;to=2026%2D11%2D30&amp;by=rig\"")
    );
    assert!(
        overview
            .contains("href=\"/beads?from=2026%2D09%2D01&amp;to=2026%2D11%2D30&amp;state=closed\""),
        "a status tile links to the list without the breakdown key"
    );
    // The cycle page says what it left out, beside the percentiles.
    let (_, cycle) = get(&server, &format!("/flow/cycle?{RANGE}"));
    assert!(cycle.contains("left out: claim time unknown"), "{cycle}");
    assert!(cycle.contains("2 items closed"), "wk-2 and wk-5");
    let (_, cycle) = get(&server, &format!("/flow/cycle?by=rig&{RANGE}"));
    assert!(cycle.contains("Cycle time by rig"));
    // Decisions are not flow items here, but have their own page.
    let (_, decisions) = get(&server, "/flow/decisions");
    assert!(decisions.contains("Q-0001") || decisions.contains("wk-q1"));
    assert!(decisions.contains("past due"));
    // A person filter narrows to the items they created, claimed or hold,
    // and the form shows it chosen. (Creators named only by history events
    // appear once the history reader has read the threads.)
    let (status, created) = get(&server, &format!("/flow/created?person=alice&{RANGE}"));
    assert_eq!(status, 200);
    assert!(
        created.contains("<option value=\"alice\" selected>alice</option>"),
        "{created}"
    );
}

#[test]
fn without_a_profile_every_flow_page_uses_the_generic_defaults() {
    let s = store("flow-plain");
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let server = start(&[
        "--beads",
        s.beads.to_str().unwrap(),
        "--bd",
        s.tools.join("bd").to_str().unwrap(),
    ]);
    for path in PAGES {
        let (status, page) = get(&server, &format!("{path}?{RANGE}"));
        assert_eq!(status, 200, "{path}");
        assert!(page.contains("<div class=\"flow\">"), "{path}");
        assert!(!page.contains(" style="), "{path}");
    }
    let (_, cycle) = get(&server, &format!("/flow/cycle?{RANGE}"));
    assert!(
        cycle.contains("left out: claim time unknown"),
        "the demo's closed item has no started_at: {cycle}"
    );
    assert!(cycle.contains("aria-current=\"true\">Type</a>"));
}
