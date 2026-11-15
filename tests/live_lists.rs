//! WP1 through the real binary: the bead list's facets, receipts kept out
//! of lists and counts, held work under way, the ready link, a milestone's
//! declared and stray lanes, search's redirect, and the no-profile bead list
//! unchanged from before profiles.

mod common;

use common::{Server, get, repo_path, start, store, work_store};
use std::io::{Read, Write};
use std::net::TcpStream;

/// The `/beads` body of the demo store (no profile) as the foundation
/// rendered it, from the page heading to the footer.
const GOLDEN_BEADS: &str = r#"<div class="page-heading"><p class="eyebrow">Work</p><h1>Beads</h1><p class="lede">Search every bead in priority order, including closed history.</p></div><form class="filters" action="/beads" method="get"><label>Search<input type="search" name="q" value="" placeholder="Title, id or label…"></label><label>State<select name="state"><option value="" selected>Not closed</option><option value="all">All</option><option value="open">open (3)</option><option value="closed">closed (1)</option></select></label><label>Ready<select name="ready"><option value="" selected>Any</option><option value="yes">Ready</option><option value="no">Not ready</option></select></label><label>Type<select name="type"><option value="" selected>All types</option><option value="epic">epic (1)</option><option value="task">task (3)</option></select></label><label>Priority<select name="priority"><option value="" selected>All priorities</option><option value="0">P0</option><option value="1">P1</option><option value="2">P2</option></select></label><button type="submit">Apply</button></form><p class="muted">3 matching beads · page 1 of 1</p><div class="table-wrap"><table class="items"><thead><tr><th>#</th><th>Bead</th><th>State</th><th>Priority</th></tr></thead><tbody><tr><td class="muted">1</td><td><a href="/bead/demo%2De1">Ship the first release</a><small>demo-e1 · epic</small></td><td><span class="badge open">open</span></td><td>P0</td></tr><tr><td class="muted">2</td><td><a href="/bead/demo%2D1">Write the README</a><small>demo-1 · task · Ship the first release</small></td><td><span class="badge open">open</span></td><td>P1</td></tr><tr><td class="muted">3</td><td><a href="/bead/demo%2D2">Publish the crate</a><small>demo-2 · task · Ship the first release</small></td><td><span class="badge open">open</span></td><td>P1</td></tr></tbody></table></div><nav class="pagination" aria-label="Results pages"></nav>"#;

fn work_server(name: &str, extra: &str) -> (common::Store, Server) {
    let s = work_store(name);
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    if !extra.is_empty() {
        let mut issues = std::fs::read_to_string(s.beads.join("issues.jsonl")).unwrap();
        issues.push_str(extra);
        issues.push('\n');
        std::fs::write(s.beads.join("issues.jsonl"), issues).unwrap();
    }
    let profile = repo_path("tests/fixtures/work/profile.toml");
    let runs = repo_path("tests/fixtures/runfeed/v1");
    let server = start(&[
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
    ]);
    (s, server)
}

/// The status line and headers of a response, without following it.
fn head(server: &Server, path: &str) -> String {
    let mut stream = TcpStream::connect(&server.address).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
        .split_once("\r\n\r\n")
        .map_or(response.clone(), |(h, _)| h.to_string())
}

fn rows(page: &str) -> usize {
    page.matches("<tr><td class=\"muted\">").count()
}

#[test]
fn lists_filter_by_profile_facets_and_leave_receipts_out() {
    let (_s, server) = work_server("lists-facets", "");
    let (status, held) = get(&server, "/beads?dstate=held&area=reports");
    assert_eq!(status, 200, "{held}");
    assert_eq!(rows(&held), 1, "{held}");
    assert!(held.contains("WORK-0003"));
    assert_eq!(rows(&get(&server, "/beads?dstate=held&area=export").1), 0);
    let (_, all) = get(&server, "/beads?state=all");
    assert!(all.contains("8 matching beads"), "{all}");
    for id in [
        "wk-rc-gate",
        "wk-rc-ship",
        "wk-op-1",
        "wk-op-2",
        "wk-coordination",
        "wk-slot",
    ] {
        assert!(!all.contains(&format!(">{id} ")), "{id} is listed");
        assert!(
            !all.contains(&format!("/bead/{}\"", id.replace('-', "%2D"))),
            "{id}"
        );
    }
    // The old keys still work.
    let (_, old) = get(
        &server,
        "/beads?q=table&state=open&ready=no&type=task&priority=2&page=0",
    );
    assert_eq!(rows(&old), 1, "{old}");
    let (_, epics) = get(&server, "/epics?milestone=v1");
    assert!(
        epics.contains("<p class=\"muted\">0 epics"),
        "the epic has no milestone"
    );
    for page in [&held, &all, &epics] {
        assert!(!page.contains(" style="));
    }
}

#[test]
fn the_overview_counts_items_and_shows_held_work_under_way() {
    let (_s, server) = work_server("lists-overview", "");
    let (status, page) = get(&server, "/");
    assert_eq!(status, 200);
    assert!(
        page.contains("<span>Open beads</span><strong>6</strong>"),
        "{page}"
    );
    assert!(page.contains("<span>In progress</span><strong>1</strong>"));
    assert!(page.contains("href=\"/beads?dstate=held\"><span>Held</span><strong>1</strong>"));
    assert!(page.contains("<h2>Under way</h2>"));
    let held = page
        .split("<tr class=\"stopped\">")
        .nth(1)
        .unwrap_or_else(|| panic!("no held row: {page}"));
    assert!(
        held.split("</tr>").next().unwrap().contains("/bead/wk%2D3"),
        "{held}"
    );
    assert!(page.contains("href=\"/beads?ready=yes\""));
    assert!(!page.contains("state=ready"), "the old ready link");
    assert!(page.contains("<h2>By milestone</h2>"));
}

#[test]
fn a_milestone_shows_declared_and_stray_lanes_and_its_members() {
    let stray = r#"{"_type":"issue","priority":3,"issue_type":"task","id":"wk-9","title":"Triage the backlog","status":"open","external_ref":"WORK-0009","created_at":"2026-11-13T09:00:00Z","updated_at":"2026-11-13T09:00:00Z","metadata":{"work":{"fields":{"milestone":"v1","lane":"triage"}}}}"#;
    let (_s, server) = work_server("lists-lanes", stray);
    let (status, page) = get(&server, "/by/milestone/v1");
    assert_eq!(status, 200, "{page}");
    assert!(
        page.contains("href=\"/bead/wk%2Dm1\""),
        "the milestone bead"
    );
    let lanes = page.split("id=\"lanes\"").nth(1).unwrap();
    let build = lanes.find("<td>build</td>").unwrap();
    let polish = lanes.find("<td>polish</td>").unwrap();
    let triage = lanes
        .find("triage <span class=\"badge warn\">stray</span>")
        .unwrap();
    assert!(
        build < polish && polish < triage,
        "declared first, in order"
    );
    assert!(page.contains("<th class=\"num\">Open gates</th>"));
    assert!(page.contains("3 matching items"), "wk-1, wk-4, wk-9");
    let (_, group) = get(&server, "/by/milestone");
    assert!(group.contains("href=\"/by/milestone/v2\""));
    assert!(
        group.contains("href=\"/by/rig\""),
        "tabs to the other groups"
    );
    assert_eq!(get(&server, "/by/milestone/nope").0, 404);
    assert_eq!(get(&server, "/by/nope").0, 404);
}

#[test]
fn search_redirects_an_exact_reference_and_lists_the_rest() {
    let (_s, server) = work_server("lists-search", "");
    let head = head(&server, "/search?q=WORK-0003");
    assert!(head.starts_with("HTTP/1.1 303"), "{head}");
    assert!(
        head.to_lowercase().contains("location: /bead/wk%2d3"),
        "{head}"
    );
    assert!(head_ok(&server, "/search?q=wk-1"));
    let (status, page) = get(&server, "/search?q=reports");
    assert_eq!(status, 200);
    assert!(page.contains("href=\"/by/area/reports\""), "{page}");
    assert!(page.contains("Move reports to the new table"));
    let (_, runs) = get(&server, "/search?q=run-000");
    assert!(runs.contains("href=\"/run/run%2D0003\""), "{runs}");
    let (_, empty) = get(&server, "/search");
    assert!(empty.contains("Type an id"));
}

fn head_ok(server: &Server, path: &str) -> bool {
    head(server, path).starts_with("HTTP/1.1 303")
}

#[test]
fn without_a_profile_the_bead_list_is_unchanged() {
    let s = store("lists-golden");
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
    let (status, page) = get(&server, "/beads");
    assert_eq!(status, 200);
    let start = page.find("<div class=\"page-heading\">").unwrap();
    let end = page.find("<footer>").unwrap();
    assert_eq!(&page[start..end], GOLDEN_BEADS);
}
