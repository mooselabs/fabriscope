//! Every internal link resolves. The work fixture is served with its
//! profile and the golden run feed, every same-origin link reachable from
//! `/` is followed (GET only, bounded), and no page may answer 404 or 500,
//! show an unfilled template, or carry an inline style. Then the chains a
//! person follows are walked link by link: a held bead to its run, attempt,
//! gate step and document; a bead to a person and their activity; a bead to
//! its group, lane table and a member; a receipt to its subject; a run to
//! its bead; and search to each kind of result. The same crawl runs with no
//! profile and no run feed, where pages that need one explain themselves.

mod common;

use common::{Server, get, repo_path, start, wait_for, work_store};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

/// Pages fetched at most, and query variants of one path.
const MAX_PAGES: usize = 2500;
const VARIANTS_PER_PATH: usize = 15;

fn work_server(s: &common::Store, profile: bool) -> Server {
    let mut args = vec![
        "--beads".to_string(),
        s.beads.to_str().unwrap().into(),
        "--bd".into(),
        s.tools.join("bd").to_str().unwrap().into(),
        "--title".into(),
        "Work".into(),
    ];
    if profile {
        args.extend([
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
    }
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let server = start(&args.iter().map(String::as_str).collect::<Vec<_>>());
    wait_for(&server, "provenance was not read", |status, h| {
        let h: serde_json::Value = serde_json::from_str(h).unwrap_or_default();
        status == 200 && h["provenance"]["reading"].as_u64() == Some(0)
    });
    server
}

/// The page's internal links, unescaped, without fragments.
fn links(page: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = page;
    while let Some(at) = rest.find("href=\"") {
        rest = &rest[at + 6..];
        let Some(end) = rest.find('"') else { break };
        let raw = rest[..end]
            .replace("&amp;", "&")
            .replace("&#39;", "'")
            .replace("&quot;", "\"");
        rest = &rest[end..];
        if !raw.starts_with('/') || raw.starts_with("//") {
            continue;
        }
        let link = raw.split('#').next().unwrap_or("").to_string();
        if !link.is_empty() {
            out.push(link);
        }
    }
    out
}

/// What a finished page never contains: an unfilled `{source}` hole, a
/// stub, an inline style.
fn defects(page: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    if page.contains(" style=") {
        out.push("an inline style");
    }
    if page.contains("This page is being built") || page.contains("Coming soon") {
        out.push("a stub page");
    }
    // A banner template hole looks like `{work.suspended.at}`.
    let hole = page.match_indices('{').any(|(at, _)| {
        let tail = &page[at + 1..];
        let name: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':'))
            .collect();
        name.contains('.') && tail[name.len()..].starts_with('}')
    });
    if hole {
        out.push("an unfilled {source} hole");
    }
    out
}

/// Follows every internal link from `/`; returns each page fetched.
fn crawl(server: &Server) -> BTreeMap<String, String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut variants: BTreeMap<String, usize> = BTreeMap::new();
    let mut from: BTreeMap<String, String> = BTreeMap::new();
    let mut queue = VecDeque::from(["/".to_string()]);
    let mut pages = BTreeMap::new();
    let mut failures = Vec::new();
    seen.insert("/".into());
    while let Some(path) = queue.pop_front() {
        if pages.len() >= MAX_PAGES {
            break;
        }
        let (status, body) = get(server, &path);
        let origin = from.get(&path).cloned().unwrap_or_default();
        if status != 200 {
            failures.push(format!("{status} {path} (linked from {origin})"));
        }
        if path.ends_with(".css") || path.ends_with(".js") {
            pages.insert(path, body);
            continue;
        }
        for defect in defects(&body) {
            failures.push(format!("{path} has {defect}"));
        }
        for link in links(&body) {
            if link == "/events" || link.starts_with("/events?") || seen.contains(&link) {
                continue;
            }
            let base = link.split('?').next().unwrap_or("").to_string();
            let n = variants.entry(base).or_default();
            if link.contains('?') {
                if *n >= VARIANTS_PER_PATH {
                    continue;
                }
                *n += 1;
            }
            seen.insert(link.clone());
            from.insert(link.clone(), path.clone());
            queue.push_back(link);
        }
        pages.insert(path, body);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("crawled {} pages", pages.len());
    pages
}

/// Fetches `path` until `ready` holds (background reads land), up to 30 s.
fn settled(server: &Server, path: &str, ready: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (status, page) = get(server, path);
        assert_eq!(status, 200, "{path}");
        if ready(&page) {
            return page;
        }
        assert!(Instant::now() < deadline, "{path} never settled: {page}");
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// The first internal link on `page` that starts with `prefix`.
fn link_to(page: &str, prefix: &str) -> String {
    links(page)
        .into_iter()
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("no link to {prefix}… in {page}"))
}

/// Follows `link` and checks it answers 200.
fn follow(server: &Server, link: &str) -> String {
    let (status, page) = get(server, link);
    assert_eq!(status, 200, "{link}: {page}");
    page
}

#[test]
fn every_link_from_the_overview_resolves_with_a_profile_and_a_feed() {
    let s = work_store("crawl-profile");
    let server = work_server(&s, true);
    // Let the bead pages fetch their run docs, so the crawl sees the
    // finished forms with their attempt and step links.
    for id in ["wk-1", "wk-2", "wk-3"] {
        get(&server, &format!("/bead/{id}"));
    }
    settled(&server, "/bead/wk-2", |p| p.contains("#attempt-1\""));
    let pages = crawl(&server);
    assert!(pages.len() > 150, "only {} pages reached", pages.len());
    for route in [
        "/beads",
        "/epics",
        "/by/milestone",
        "/people",
        "/activity",
        "/flow",
        "/flow/created",
        "/flow/claimed",
        "/flow/cycle",
        "/flow/ageing",
        "/flow/decisions",
        "/held",
        "/runs",
        "/costs",
        "/receipts",
        "/boards",
        "/run/run%2D0002",
        "/person/factory%2FhostA%2FrigA",
    ] {
        assert!(pages.contains_key(route), "{route} was not reached");
    }
    assert!(
        pages.keys().any(|p| p.starts_with("/run/run%2D0002/doc/")),
        "a document page was reached"
    );
    assert!(
        pages.keys().any(|p| p.starts_with("/boards/")),
        "a board was reached"
    );
}

#[test]
fn the_chains_a_person_follows_resolve_link_by_link() {
    let s = work_store("crawl-chains");
    let server = work_server(&s, true);
    // Held bead → banner → run → attempt anchor → gate step → document.
    let held = settled(&server, "/bead/wk-3", |p| p.contains("id=\"stop\""));
    assert!(held.contains("id=\"banner-held\""), "{held}");
    let run_link = link_to(&held, "/run/run%2D0003");
    let run = settled(&server, &run_link, |p| p.contains("id=\"attempt-1\""));
    assert!(
        run.contains("id=\"step-s%2D0003%2Dimplement%2D1\""),
        "{run}"
    );
    let doc = link_to(&run, "/run/run%2D0003/doc/");
    follow(&server, &doc);
    // The landed bead's gate step and its log.
    let landed = settled(&server, "/bead/wk-2", |p| p.contains("#attempt-1\""));
    let attempt = links(&landed)
        .into_iter()
        .find(|l| l.starts_with("/run/run%2D0002"))
        .unwrap();
    let run = settled(&server, &attempt, |p| p.contains("id=\"attempt-1\""));
    assert!(
        run.contains("id=\"step-s%2D0002%2Dgate%2D1\""),
        "the gate step"
    );
    let log = link_to(&run, "/run/run%2D0002/doc/gate%2Dlog");
    settled(&server, &log, |p| p.contains("id=\"L1\""));
    // Run → bead.
    assert!(links(&run).iter().any(|l| l == "/bead/wk%2D2"), "{run}");
    // Bead → person → activity.
    let working = follow(&server, "/bead/wk-1");
    let person = link_to(&working, "/person/");
    let person_page = follow(&server, &person);
    let activity = link_to(&person_page, "/activity?");
    let activity_page = follow(&server, &activity);
    assert!(activity_page.contains("/bead/wk%2D1"), "{activity_page}");
    // Bead → group → lane table → member.
    let group = link_to(&working, "/by/milestone/");
    let lane = follow(&server, &group);
    assert!(lane.contains("lane"), "a lane table: {lane}");
    let member = link_to(&lane, "/bead/");
    follow(&server, &member);
    // Receipt → subject bead.
    let receipt = follow(&server, "/bead/wk-rc-gate");
    assert!(
        links(&receipt).iter().any(|l| l == "/bead/wk%2D2"),
        "{receipt}"
    );
    // Search → each kind of result.
    for (q, prefix) in [
        ("table", "/bead/"),
        ("rigA", "/person/"),
        ("rig", "/by/rig/"),
        ("run-0003", "/run/"),
    ] {
        let found = follow(&server, &format!("/search?q={q}"));
        follow(&server, &link_to(&found, prefix));
    }
    // An exact reference opens the bead.
    let (status, exact) = get(&server, "/search?q=WORK-0003");
    assert!(
        status == 200 || status == 303,
        "{status}: an exact reference finds its bead: {exact}"
    );
}

#[test]
fn without_a_profile_or_feed_every_link_still_resolves() {
    let s = work_store("crawl-bare");
    let server = work_server(&s, false);
    let pages = crawl(&server);
    assert!(pages.len() > 100, "only {} pages reached", pages.len());
    let nav = &pages["/"];
    for absent in [
        "href=\"/held\"",
        "href=\"/receipts\"",
        "href=\"/runs\"",
        "/by/",
    ] {
        assert!(!nav.contains(absent), "{absent} needs a profile or a feed");
    }
    for (path, says) in [
        ("/held", "No state in the profile holds work back"),
        ("/receipts", "The profile defines no receipts"),
    ] {
        let (status, page) = get(&server, path);
        assert_eq!(status, 200, "{path}");
        assert!(page.contains(says), "{path}: {page}");
    }
}
