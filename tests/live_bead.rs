//! The bead page through the real binary, on the work-ledger fixture with
//! its profile and the golden run feed: the held banner, attempts joined to
//! runs with their usage, receipts, typed panels whose values stay inert,
//! the merged timeline, and — without a profile — the page exactly as before.
//!
//! Run docs and history threads are filled by background readers other
//! packages own; until a run doc is read the page says so, and these tests
//! accept either state, asserting the full stop once the doc arrives.

mod common;

use common::{Server, fixture, get, repo_path, start, store, work_store};
use std::time::{Duration, Instant};

fn work_server(s: &common::Store) -> Server {
    std::fs::write(s.beads.join("no-serve"), "").unwrap();
    let args = [
        "--beads".to_string(),
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
    ];
    start(&args.iter().map(String::as_str).collect::<Vec<_>>())
}

/// The page once `ready` holds, re-read until `secs` pass; `None` if it
/// never did.
fn page_when(
    server: &Server,
    path: &str,
    secs: u64,
    ready: impl Fn(&str) -> bool,
) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let (status, page) = get(server, path);
        assert_eq!(status, 200, "{path}");
        if ready(&page) {
            return Some(page);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn page(server: &Server, path: &str, ready: impl Fn(&str) -> bool) -> String {
    page_when(server, path, 30, &ready)
        .unwrap_or_else(|| panic!("{path} never got ready: {}", get(server, path).1))
}

fn position(page: &str, needle: &str) -> usize {
    page.find(needle)
        .unwrap_or_else(|| panic!("{needle} missing from {page}"))
}

#[test]
fn a_held_bead_shows_its_banner_and_the_feed_stop() {
    let s = work_store("bead-held");
    let server = work_server(&s);
    let held = page(&server, "/bead/wk-3", |p| p.contains("Attempts · 1"));
    assert!(
        held.contains("<section class=\"banner bad\" id=\"banner-held\""),
        "{held}"
    );
    assert!(held.contains("Held since 2026-11-15T09:15:00Z: scope-exceeded (scope)"));
    for (label, value) in [
        ("stop code", "scope-exceeded"),
        ("cause", "scope"),
        ("attempt", "att-0003-1"),
        ("policy", "p1"),
    ] {
        assert!(
            held.contains(&format!("<dt>{label}</dt><dd>{value}</dd>")),
            "marker fact {label}"
        );
    }
    assert!(
        !held.contains("Ready, as bd ready decides"),
        "bd ready lists it, but a held bead is never ready"
    );
    assert!(held.contains("href=\"/run/run%2D0003\""));
    // The stop's details live in the run doc, which the page's own visit
    // fetches in the background; until then the page says so.
    let stop = page(&server, "/bead/wk-3", |p| p.contains("id=\"stop\""));
    assert!(
        stop.contains("<strong>Next:</strong> Widen the write scope"),
        "{stop}"
    );
    assert!(stop.contains(
        "<li><code>workctl edit WORK-0003 --add-scope &quot;migrations/**&quot;</code></li>"
    ));
    assert!(stop.contains("<li><code>workctl release WORK-0003</code></li>"));
    assert!(stop.contains("<dt>Question</dt><dd>Widen WORK-0003&#39;s write scope"));
    assert!(stop.contains("<dt>Recommended</dt><dd>yes</dd>"));
    assert!(stop.contains("<dt>If yes</dt><dd>The run resumes"));
    assert!(stop.contains("<dt>If not</dt><dd>A new item is cut"));
}

#[test]
fn a_landed_bead_joins_attempts_runs_receipts_and_costs() {
    let s = work_store("bead-landed");
    let server = work_server(&s);
    let landed = page(&server, "/bead/wk-2", |p| p.contains("Attempts · 2"));
    for (n, id, usd) in [(1, "att-0002-1", "$2.10"), (2, "att-0002-2", "$3.27")] {
        let card = &landed[position(&landed, &format!("<li id=\"attempt-{n}\">"))..];
        let card = &card[..card.find("</li>").unwrap()];
        assert!(card.contains(&format!("<code>{id}</code>")), "{card}");
        assert!(
            card.contains(&format!("<dt>Cost</dt><dd>{usd}</dd>")),
            "{card}"
        );
    }
    assert!(landed.contains("<dt>Gates reused</dt><dd>1 · saved 5m 0s</dd>"));
    assert!(
        landed.contains("href=\"/run/run%2D0002"),
        "attempt 2 joins run-0002"
    );
    assert!(landed.contains("<h2>Runs · 2</h2>"));
    assert!(landed.contains("superseded"));
    assert!(landed.contains("<h2>Receipts · 3</h2>"));
    for receipt in ["wk%2Drc%2Dgate", "wk%2Drc%2Dship", "wk%2Dop%2D1"] {
        assert!(
            landed.contains(&format!("href=\"/bead/{receipt}\"")),
            "{receipt}"
        );
    }
    assert!(landed.contains("<dt>Ledger usage</dt><dd><strong>$5.37</strong>"));
    assert!(landed.contains("<dt>Run feed</dt><dd><strong>$6.01</strong>"));
    assert!(
        !landed.contains("$11.38"),
        "the two sources are never added"
    );
    // Once run-0002's doc is read, attempt 1 links to its place on the run page.
    let joined = page(&server, "/bead/wk-2", |p| p.contains("#attempt-1\""));
    assert!(
        joined.contains("href=\"/run/run%2D0002#attempt-1\""),
        "{joined}"
    );
    assert!(joined.contains("<dt>Retried because</dt><dd>gate-failed</dd>"));
    // A milestone lists the group members it names.
    let milestone = page(&server, "/bead/wk-m1", |p| p.contains("Milestone"));
    assert!(
        milestone.contains("id=\"members-milestone\""),
        "{milestone}"
    );
    assert!(milestone.contains("<h2>Milestone v1 · 3</h2>"));
}

#[test]
fn the_timeline_merges_dates_history_comments_and_provenance() {
    let s = work_store("bead-timeline");
    let server = work_server(&s);
    let p = page(&server, "/bead/wk-1", |p| p.contains("id=\"t-prov-p1-1\""));
    // Newest first. Bob's comment (10:30); then at 10:00 the started date,
    // the provenance claim and the history claim; then authored and created.
    let order = [
        "id=\"t-comment-c-1c\"",
        "id=\"t-started\"",
        "id=\"t-prov-p1-1\"",
        "id=\"t-event-2-claim\"",
        "id=\"t-event-1-authored\"",
        "id=\"t-created\"",
    ];
    for pair in order.windows(2) {
        assert!(
            position(&p, pair[0]) < position(&p, pair[1]),
            "{} before {}",
            pair[0],
            pair[1]
        );
    }
    // The cut (10:01) sits between the comment and the started date.
    let cut = position(&p, "id=\"t-prov-p1-2\"");
    assert!(position(&p, order[0]) < cut && cut < position(&p, order[1]));
    assert!(p.contains("status: open → working"));
    assert!(
        p.contains("<strong>header row</strong>"),
        "comments are Markdown"
    );
    assert!(p.contains("href=\"/person/factory%2FhostA%2FrigA\""));
    // Steps join once run-0001's doc is read.
    let steps = page(&server, "/bead/wk-1", |p| {
        p.contains("id=\"t-step-run-0001-")
    });
    assert!(
        position(&steps, "id=\"t-step-run-0001-s-0001-gate-1\"")
            < position(&steps, "id=\"t-comment-c-1c\"")
    );
    // Panels: every kind, and a field with no value is left out.
    assert!(p.contains("<dt>Timebox</dt><dd>2–3 h</dd>"));
    assert!(p.contains("<dt>Spend cap</dt><dd>$4.50</dd>"));
    assert!(p.contains("href=\"https://example.com/adr/ADR-0007\""));
    assert!(!p.contains("<dt>Evidence</dt>"));
    assert!(!p.contains(" style="));
}

#[test]
fn hostile_field_values_stay_inert() {
    let s = work_store("bead-hostile");
    let mut issues = fixture("work/issues.jsonl");
    issues.push_str(&serde_json::json!({
        "id": "wk-x",
        "title": "<script>alert('t')</script>",
        "status": "open",
        "priority": 2,
        "issue_type": "task",
        "external_ref": "WORK-9999",
        "created_at": "2026-11-15T09:00:00Z",
        "updated_at": "2026-11-15T09:00:00Z",
        "metadata": {"work": {"fields": {
            "write_scope": ["<script>alert(1)</script>", "ok/path"],
            "timebox": "<b>2h</b>",
            "spend_cap": "<i>lots</i>",
            "validation": "<img src=x onerror=alert(1)>",
            "related": ["<x>", "WORK-0001"],
            "evidence": ["javascript:alert(1)", "http://plain.example/x", "https://ok.example/a\"b"],
            "review": "<u>r</u>",
            "gate": "<g>",
            "claimed_by": "<svg onload=alert(1)>",
            "claimed_at": "<t>",
            "notes_md": "<img src=x onerror=alert(1)> [x](javascript:alert(1)) **ok**"
        }}}
    }).to_string());
    issues.push('\n');
    std::fs::write(s.beads.join("issues.jsonl"), issues).unwrap();
    let server = work_server(&s);
    let p = page(&server, "/bead/wk-x", |p| p.contains("id=\"panel-packet\""));
    let main = &p[position(&p, "<main")..];
    for bad in [
        "<script",
        "<img",
        "<svg",
        "<b>2h",
        "<i>lots",
        "<u>r",
        "<x>",
        "<g>",
        "<t>",
        "alert(1)>",
    ] {
        assert!(!main.contains(bad), "{bad} survived: {main}");
    }
    for bad in ["href=\"javascript:", "href=\"http://plain"] {
        assert!(!main.contains(bad), "{bad} became a link");
    }
    assert!(
        main.contains("javascript:alert(1)"),
        "the value is still shown"
    );
    assert!(main.contains("http://plain.example/x"));
    assert!(main.contains("href=\"https://ok.example/a&quot;b\""));
    assert!(main.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(main.contains("<strong>ok</strong>"));
    assert!(!main.contains(" style="));
}

/// The foundation's `/bead/demo-1` without a profile or run feed, with the
/// capture's normalisation (revision, read times and the age left blank).
const GOLDEN: &str = r##"<main id="content" data-revision="N"><div class="breadcrumbs"><a href="/">Overview</a> / <a href="/beads">Beads</a> / <a href="/bead/demo%2De1">Ship the first release</a></div><div class="page-heading"><p class="eyebrow">demo-1</p><h1>Write the README</h1><p class="lede"></p></div><div class="record-meta"><span class="badge open">open</span><span>P1</span><span>task</span><span>Ready, as bd ready decides</span><span class="label">docs</span></div><section class="progress" id="progress"><h2>Progress</h2><p class="muted">Claimed 2 times · from <code>bd provenance log</code>, read <time>TS</time></p><ol class="stages"><li class="stage done"><span class="dot" aria-hidden="true"></span><strong>Created</strong><span class="sr-only"> (done)</span><time datetime="2026-10-02T09:00:00Z" title="2026-10-02T09:00:00Z">2 Oct 09:00</time></li><li class="stage done"><span class="dot" aria-hidden="true"></span><strong>Claimed</strong><span class="sr-only"> (done)</span><time datetime="2026-10-04T12:00:00Z" title="2026-10-04T12:00:00Z">4 Oct 12:00</time><span class="fact">by factory/demo</span></li><li class="stage done"><span class="dot" aria-hidden="true"></span><strong>Started</strong><span class="sr-only"> (done)</span><time datetime="2026-10-04T12:01:00Z" title="2026-10-04T12:01:00Z">4 Oct 12:01</time><code class="fact" title="demo:run-2@2026-10-04T12:00:00Z">demo:run-2@2026-10-04T12:00:00Z</code></li><li class="stage done"><span class="dot" aria-hidden="true"></span><strong>Committed</strong><span class="sr-only"> (done)</span><time datetime="2026-10-04T12:40:00Z" title="2026-10-04T12:40:00Z">4 Oct 12:40</time><code class="fact" title="0123456789abcdef0123456789abcdef01234567">0123456</code></li><li class="stage done"><span class="dot" aria-hidden="true"></span><strong>Landed</strong><span class="sr-only"> (done)</span><time datetime="2026-10-04T12:50:00Z" title="2026-10-04T12:50:00Z">4 Oct 12:50</time><a class="fact" href="https://github.com/example/demo/pull/7" rel="noreferrer">PR #7</a></li><li class="stage current"><span class="dot" aria-hidden="true"></span><strong>Closed</strong><span class="sr-only"> (next)</span></li><li class="stage pending"><span class="dot" aria-hidden="true"></span><strong>Used</strong><span class="sr-only"> (not yet)</span></li></ol><p class="now">Closed is next · AGE</p><details id="earlier-attempts"><summary>Earlier attempts · 1</summary><ul class="attempts"><li><strong>Attempt 1</strong><ol class="stages compact"><li class="stage done" title="Created · done · 2026-10-02T09:00:00Z"><span class="dot" aria-hidden="true"></span><span class="sr-only">Created: done</span></li><li class="stage done" title="Claimed · done · 2026-10-04T09:00:00Z"><span class="dot" aria-hidden="true"></span><span class="sr-only">Claimed: done</span></li><li class="stage done" title="Started · done · 2026-10-04T09:02:00Z"><span class="dot" aria-hidden="true"></span><span class="sr-only">Started: done</span></li><li class="stage stopped" title="Committed · stopped here"><span class="dot" aria-hidden="true"></span><span class="sr-only">Committed: stopped here</span></li><li class="stage pending" title="Landed · not yet"><span class="dot" aria-hidden="true"></span><span class="sr-only">Landed: not yet</span></li><li class="stage pending" title="Closed · not yet"><span class="dot" aria-hidden="true"></span><span class="sr-only">Closed: not yet</span></li><li class="stage pending" title="Used · not yet"><span class="dot" aria-hidden="true"></span><span class="sr-only">Used: not yet</span></li></ol><small>Suspended <time datetime="2026-10-04T10:00:00Z" title="2026-10-04T10:00:00Z">4 Oct 10:00</time> by factory/demo · cause <strong>implementation</strong> · stop code <strong>tests_failed</strong><br>Resumed <time datetime="2026-10-04T11:00:00Z" title="2026-10-04T11:00:00Z">4 Oct 11:00</time> by owner · why <strong>fixed the fixture</strong></small></li></ul></details><details id="provenance-log"><summary>Provenance log · 8 events</summary><div class="table-wrap"><table class="log"><thead><tr><th>When</th><th>Kind</th><th>Reference</th><th>Detail</th><th>By</th></tr></thead><tbody><tr><td><time datetime="2026-10-04T09:00:00Z" title="2026-10-04T09:00:00Z">4 Oct 09:00</time></td><td>claim</td><td></td><td>generation 1</td><td>factory/demo<small>test</small></td></tr><tr><td><time datetime="2026-10-04T09:02:00Z" title="2026-10-04T09:02:00Z">4 Oct 09:02</time></td><td>cut</td><td><code class="fact" title="demo:run-1@2026-10-04T09:00:00Z">demo:run-1@2026-10-04T09:00:00Z</code></td><td>formula build</td><td>factory/demo<small>test</small></td></tr><tr><td><time datetime="2026-10-04T10:00:00Z" title="2026-10-04T10:00:00Z">4 Oct 10:00</time></td><td>suspend</td><td></td><td>cause implementation · stop code tests_failed</td><td>factory/demo<small>test</small></td></tr><tr><td><time datetime="2026-10-04T11:00:00Z" title="2026-10-04T11:00:00Z">4 Oct 11:00</time></td><td>resume</td><td></td><td>why fixed the fixture</td><td>owner<small>test</small></td></tr><tr><td><time datetime="2026-10-04T12:00:00Z" title="2026-10-04T12:00:00Z">4 Oct 12:00</time></td><td>claim</td><td></td><td>generation 1</td><td>factory/demo<small>test</small></td></tr><tr><td><time datetime="2026-10-04T12:01:00Z" title="2026-10-04T12:01:00Z">4 Oct 12:01</time></td><td>cut</td><td><code class="fact" title="demo:run-2@2026-10-04T12:00:00Z">demo:run-2@2026-10-04T12:00:00Z</code></td><td>formula build</td><td>factory/demo<small>test</small></td></tr><tr><td><time datetime="2026-10-04T12:40:00Z" title="2026-10-04T12:40:00Z">4 Oct 12:40</time></td><td>commit</td><td><code class="fact" title="0123456789abcdef0123456789abcdef01234567">0123456</code></td><td></td><td>factory/demo<small>test</small></td></tr><tr><td><time datetime="2026-10-04T12:50:00Z" title="2026-10-04T12:50:00Z">4 Oct 12:50</time></td><td>land</td><td><a class="fact" href="https://github.com/example/demo/pull/7" rel="noreferrer">PR #7</a></td><td></td><td>factory/demo<small>test</small></td></tr></tbody></table></div></details></section><div class="detail-grid"><article class="document"><p>See <a href="/scroll/sc%2Dd1">the decision</a> and &lt;script&gt;alert(1)&lt;/script&gt;.</p>
</article><aside><h3>Depends on</h3><p class="muted">None.</p><h3>Blocks</h3><ul class="relations"><li><a href="/bead/demo%2D2">Publish the crate</a><small>demo-2</small><span class="badge open">open</span></li></ul><h3>Scrolls about this bead</h3><ul class="relations"><li><a href="/scroll/sc%2Dd1">Use Dolt for history</a><small>decision · related work</small></li></ul><details id="metadata"><summary>Metadata</summary><pre>null</pre></details><p class="source-path">Created 2026-10-02T09:00:00Z<br>Updated 2026-10-02T09:00:00Z<br></p></aside></div><footer>Beads via bd export · commit c1 · read TS<br>Scrolls · revision fixture-r · read TS<br>Refreshes automatically · read-only</footer></main>"##;

/// Blanks what changes from run to run, as the golden capture did.
fn normalise(page: &str) -> String {
    let mut out = page.to_string();
    // Replaces the run of `keep` characters after each `marker` with `with`.
    let blank = |out: &mut String, marker: &str, keep: &dyn Fn(char) -> bool, with: &str| {
        let mut from = 0;
        while let Some(i) = out[from..].find(marker).map(|i| i + from + marker.len()) {
            let j = out[i..]
                .find(|c: char| !keep(c))
                .map_or(out.len(), |j| i + j);
            if j > i {
                out.replace_range(i..j, with);
            }
            from = i + 1;
        }
    };
    blank(&mut out, "data-revision=\"", &|c| c.is_ascii_digit(), "N");
    blank(
        &mut out,
        " · read ",
        &|c| c.is_ascii_digit() || "T:-.+Z".contains(c),
        "TS",
    );
    blank(
        &mut out,
        "Closed is next · ",
        &|c| c.is_ascii_digit() || "dhms ".contains(c),
        "AGE",
    );
    // `read <time …>…</time>` in the progress section.
    if let Some(i) = out.find("read <time datetime=") {
        let j = out[i..].find("</time>").unwrap() + i + "</time>".len();
        out.replace_range(i..j, "read <time>TS</time>");
    }
    out
}

#[test]
fn without_a_profile_the_bead_page_is_unchanged() {
    let s = store("bead-golden");
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
    let p = page(&server, "/bead/demo-1", |p| {
        p.contains("Provenance log · 8 events") && p.contains("Scrolls about this bead")
    });
    let main = &p[position(&p, "<main")..];
    let main = &main[..position(main, "</main>") + "</main>".len()];
    let main = normalise(main);
    if main != GOLDEN {
        let at = main
            .bytes()
            .zip(GOLDEN.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(main.len().min(GOLDEN.len()));
        panic!(
            "the no-profile page changed at byte {at}:\n  now: {}\n  was: {}",
            &main[at.saturating_sub(80)..(at + 120).min(main.len())],
            &GOLDEN[at.saturating_sub(80)..(at + 120).min(GOLDEN.len())]
        );
    }
}
