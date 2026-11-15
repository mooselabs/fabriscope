// Owned by WP4 — Flow. Frozen signature:
// `flow(&View, FlowPage, &Facets) -> String`.
//! The flow pages: created and closed per day, claims, cycle time (p50/p90,
//! nearest rank), ageing of open work, and open decisions.
//!
//! Every number comes from `crate::flow`; this module only draws. An unknown
//! is written as "unknown" or "no data", never as 0, and every percentile has
//! the count it left out beside it.

use super::fmt::{self, words};
use super::layout::page;
use super::svg::{self, Bar, Spread};
use super::{
    badge, bead_link, escape, heading, pagination, person_link, select, state_badges, tone_badge,
};
use crate::flow::{self, By, ClaimState, Item, PAGE_SIZE, STALE_DAYS, Stats, Timing, Window};
use crate::model::View;
use crate::profile::Tone;
use crate::query::{Facets, href};
use chrono::{DateTime, Duration, Utc};
use std::fmt::Write;

/// Which flow page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowPage {
    Overview,
    Created,
    Claimed,
    Cycle,
    Ageing,
    Decisions,
}

impl FlowPage {
    /// The page's path and title.
    pub(crate) fn path_and_title(self) -> (&'static str, &'static str) {
        match self {
            FlowPage::Overview => ("/flow", "Flow"),
            FlowPage::Created => ("/flow/created", "Created"),
            FlowPage::Claimed => ("/flow/claimed", "Claimed"),
            FlowPage::Cycle => ("/flow/cycle", "Cycle time"),
            FlowPage::Ageing => ("/flow/ageing", "Ageing"),
            FlowPage::Decisions => ("/flow/decisions", "Decisions"),
        }
    }

    const ALL: [FlowPage; 6] = [
        FlowPage::Overview,
        FlowPage::Created,
        FlowPage::Claimed,
        FlowPage::Cycle,
        FlowPage::Ageing,
        FlowPage::Decisions,
    ];

    fn tab(self) -> &'static str {
        match self {
            FlowPage::Overview => "Overview",
            FlowPage::Created => "Created",
            FlowPage::Claimed => "Claimed",
            FlowPage::Cycle => "Cycle and lead",
            FlowPage::Ageing => "Ageing",
            FlowPage::Decisions => "Decisions",
        }
    }
}

/// One flow page.
pub(crate) fn flow(view: &View, page: FlowPage, facets: &Facets) -> String {
    flow_at(view, page, facets, Utc::now())
}

/// What every part of one flow page shares.
struct Ctx<'v, 'f> {
    view: &'v View,
    facets: &'f Facets,
    page: FlowPage,
    path: &'static str,
    window: Window,
    now: DateTime<Utc>,
    items: Vec<Item<'v>>,
}

/// One flow page as it is at `now`.
pub(crate) fn flow_at(view: &View, which: FlowPage, facets: &Facets, now: DateTime<Utc>) -> String {
    let (path, title) = which.path_and_title();
    let cx = Ctx {
        view,
        facets,
        page: which,
        path,
        window: Window::resolve(facets, now),
        now,
        items: match which {
            FlowPage::Decisions => flow::decision_items(view, facets, now),
            _ => flow::items(view, facets, now),
        },
    };
    let (lede, body) = match which {
        FlowPage::Overview => overview(&cx),
        FlowPage::Created => created(&cx),
        FlowPage::Claimed => claimed(&cx),
        FlowPage::Cycle => cycle(&cx),
        FlowPage::Ageing => ageing(&cx),
        FlowPage::Decisions => decisions(&cx),
    };
    let mut out = String::from("<div class=\"flow\">");
    out.push_str(&heading("Flow", title, &lede));
    out.push_str(&tabs(&cx));
    out.push_str(&controls(&cx));
    out.push_str(&body);
    out.push_str("</div>");
    page(view, title, path, &out)
}

// ---------------------------------------------------------------------------
// Shared parts
// ---------------------------------------------------------------------------

const UNKNOWN: &str = "<span class=\"unknown\">unknown</span>";
const NO_DATA: &str = "<span class=\"unknown\">no data</span>";

fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn time_cell(at: Option<DateTime<Utc>>) -> String {
    at.map_or_else(|| UNKNOWN.to_string(), |at| fmt::when(&rfc3339(at)))
}

fn person_cell(name: Option<&str>) -> String {
    name.map_or_else(|| UNKNOWN.to_string(), person_link)
}

fn duration(value: Option<f64>) -> String {
    value.map_or_else(|| NO_DATA.to_string(), |s| escape(&fmt::seconds(s)))
}

fn days(n: i64) -> String {
    format!("{n} d")
}

fn item_cell(item: &Item) -> String {
    format!(
        "{}<small>{}</small>",
        bead_link(&item.bead.id, &item.bead.title),
        escape(&item.bead.id)
    )
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The line that stands beside every cycle-time percentile.
fn left_out(n: usize) -> String {
    format!(
        "<p class=\"left-out\">{} left out: claim time unknown</p>",
        plural(n, "item", "items")
    )
}

/// A summary card; `value` is markup.
fn card(label: &str, value: &str, detail: &str, link: Option<String>) -> String {
    let inner = format!(
        "<span>{}</span><strong>{value}</strong><small>{}</small>",
        escape(label),
        escape(detail)
    );
    match link {
        Some(href) => format!("<a class=\"metric\" href=\"{}\">{inner}</a>", escape(&href)),
        None => format!("<div class=\"metric\">{inner}</div>"),
    }
}

fn cards(cards: &[String]) -> String {
    format!("<div class=\"metrics flow-cards\">{}</div>", cards.concat())
}

/// A page of a list: the slice to show and the pager under it.
fn paged<'a, T>(cx: &Ctx, rows: &'a [T]) -> (&'a [T], String) {
    let pages = rows.len().div_ceil(PAGE_SIZE).max(1);
    let current = cx.facets.page().min(pages - 1);
    let start = current * PAGE_SIZE;
    let shown = &rows[start..(start + PAGE_SIZE).min(rows.len())];
    let pairs = cx.facets.pairs();
    let query: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut pager = String::new();
    if pages > 1 {
        write!(
            pager,
            "<p class=\"muted\">Rows {}–{} of {}.</p>{}",
            start + 1,
            start + shown.len(),
            rows.len(),
            pagination(cx.path, &query, current, pages)
        )
        .unwrap();
    }
    (shown, pager)
}

fn table(head: &[&str], numeric: &[usize], rows: &str) -> String {
    let mut out =
        "<div class=\"table-wrap\"><table class=\"data flow-table\"><thead><tr>".to_string();
    for (i, h) in head.iter().enumerate() {
        let class = if numeric.contains(&i) {
            " class=\"num\""
        } else {
            ""
        };
        write!(out, "<th{class}>{}</th>", escape(h)).unwrap();
    }
    write!(out, "</tr></thead><tbody>{rows}</tbody></table></div>").unwrap();
    out
}

/// The six flow pages, each link carrying the query.
fn tabs(cx: &Ctx) -> String {
    let mut out = "<nav class=\"flow-tabs\" aria-label=\"Flow views\">".to_string();
    for p in FlowPage::ALL {
        let (path, _) = p.path_and_title();
        let here = if p == cx.page {
            " aria-current=\"page\""
        } else {
            ""
        };
        write!(
            out,
            "<a href=\"{}\"{here}>{}</a>",
            escape(&href(path, cx.facets, &[])),
            p.tab()
        )
        .unwrap();
    }
    out.push_str("</nav>");
    out
}

/// Quick ranges, then the filter form.
fn controls(cx: &Ctx) -> String {
    let today = cx.now.date_naive();
    let yesterday = (today - Duration::days(1)).to_string();
    let f = cx.facets;
    let presets: [(&str, [(&str, &str); 3]); 6] = [
        ("Today", [("days", "1"), ("from", ""), ("to", "")]),
        (
            "Yesterday",
            [("days", ""), ("from", &yesterday), ("to", &yesterday)],
        ),
        ("7 days", [("days", "7"), ("from", ""), ("to", "")]),
        ("14 days", [("days", "14"), ("from", ""), ("to", "")]),
        ("30 days", [("days", "30"), ("from", ""), ("to", "")]),
        ("90 days", [("days", "90"), ("from", ""), ("to", "")]),
    ];
    let mut out =
        "<nav class=\"flow-presets\" aria-label=\"Date range\"><span class=\"muted\">Range</span>"
            .to_string();
    let explicit = !f.days.is_empty() || !f.from.is_empty() || !f.to.is_empty();
    for (label, overrides) in &presets {
        let current = overrides.iter().all(|(k, v)| match *k {
            "days" => f.days == *v || (!explicit && *v == "30"),
            "from" => f.from == *v || f.days.is_empty() && v.is_empty() && !explicit,
            _ => f.to == *v || f.days.is_empty() && v.is_empty() && !explicit,
        }) && (*label != "Yesterday" || f.days.is_empty());
        let here = if current {
            " aria-current=\"true\""
        } else {
            ""
        };
        write!(
            out,
            "<a href=\"{}\"{here}>{label}</a>",
            escape(&href(cx.path, f, overrides))
        )
        .unwrap();
    }
    out.push_str("</nav>");
    out.push_str(&filters(cx));
    out
}

/// The filter form. It submits the range as `from`/`to` (dropping `days`)
/// and carries every other key as a hidden field, so nothing is lost.
fn filters(cx: &Ctx) -> String {
    let view = cx.view;
    let f = cx.facets;
    let mut out = format!(
        "<form class=\"filters flow-filters\" action=\"{}\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Title, id or label…\"></label><label>From<input type=\"date\" name=\"from\" value=\"{}\"></label><label>To<input type=\"date\" name=\"to\" value=\"{}\"></label>",
        escape(cx.path),
        escape(&f.q),
        cx.window.from,
        cx.window.to
    );
    let Some(s) = view.snapshot() else {
        out.push_str("<button type=\"submit\">Apply</button></form>");
        return out;
    };
    let items = || {
        s.beads
            .iter()
            .enumerate()
            .filter(|(i, _)| s.derived.is_item(*i))
            .map(|(_, b)| b)
    };
    let any = |label: &str| vec![(String::new(), label.to_string())];
    let mut states = any("Any status");
    states.extend(
        super::tally(items(), |b| b.status.as_str())
            .into_iter()
            .map(|(v, n)| (v.to_string(), format!("{} ({n})", words(v)))),
    );
    out.push_str(&select("state", "Status", &f.state, &states));
    let mut types = any("All types");
    types.extend(
        super::tally(items(), |b| b.issue_type.as_str())
            .into_iter()
            .map(|(v, n)| (v.to_string(), format!("{} ({n})", words(v)))),
    );
    out.push_str(&select("type", "Type", &f.issue_type, &types));
    let mut priorities: Vec<i64> = items().map(|b| b.priority).collect();
    priorities.sort_unstable();
    priorities.dedup();
    let mut options = any("Any priority");
    options.extend(priorities.iter().map(|p| (p.to_string(), format!("P{p}"))));
    out.push_str(&select("priority", "Priority", &f.priority, &options));
    if !view.profile.kinds.is_empty() {
        let mut kinds = any("All kinds");
        kinds.extend(
            view.profile
                .kinds
                .iter()
                .map(|k| (k.key.clone(), k.label.clone())),
        );
        out.push_str(&select("kind", "Kind", &f.kind, &kinds));
    }
    let mut people: Vec<String> = s.derived.people.keys().cloned().collect();
    for item in &cx.items {
        people.extend(item.creator.iter().cloned());
        people.extend(item.claim.by.iter().cloned());
    }
    if !f.person.is_empty() {
        people.push(f.person.clone());
    }
    people.sort();
    people.dedup();
    if !people.is_empty() {
        let mut options = any("Anyone");
        options.extend(people.into_iter().map(|p| (p.clone(), p)));
        out.push_str(&select("person", "Person", &f.person, &options));
    }
    for group in &view.profile.groups {
        let mut values = any(&format!("Any {}", group.label.to_lowercase()));
        if let Some(index) = s.derived.groups.get(&group.key) {
            values.extend(
                index
                    .iter()
                    .map(|(v, m)| (v.clone(), format!("{v} ({})", m.len()))),
            );
        }
        let current = f.groups.get(&group.key).map(String::as_str).unwrap_or("");
        out.push_str(&select(&group.key, &group.label, current, &values));
    }
    let hidden: [(&str, &str); 5] = [
        ("dstate", &f.dstate),
        ("ready", &f.ready),
        ("epic", &f.epic),
        ("sort", &f.sort),
        ("by", &f.by),
    ];
    for (name, value) in hidden.into_iter().filter(|(_, v)| !v.is_empty()) {
        write!(
            out,
            "<input type=\"hidden\" name=\"{name}\" value=\"{}\">",
            escape(value)
        )
        .unwrap();
    }
    write!(
        out,
        "<button type=\"submit\">Apply</button><a class=\"flow-clear\" href=\"{}\">Clear filters</a></form>",
        cx.path
    )
    .unwrap();
    out
}

/// A chart in a box that scrolls sideways on a narrow screen, so the page
/// itself never does.
fn scroll(chart: String) -> String {
    if chart.is_empty() {
        return chart;
    }
    format!("<div class=\"chart-scroll\">{chart}</div>")
}

/// A status as a label: `in_progress` → `In progress`.
fn sentence(value: &str) -> String {
    let text = words(value);
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn method(text: &str) -> String {
    format!("<p class=\"method\">{}</p>", escape(text))
}

fn range_lede(cx: &Ctx, what: &str) -> String {
    format!("{} · whole UTC days · {what}", cx.window.describe())
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

fn overview(cx: &Ctx) -> (String, String) {
    // A tile names its status, so the list it opens means the same items.
    let outside = cx.facets.without(&["by"]);
    let mut body = "<section id=\"flow-status\"><h2>Status now</h2><p class=\"muted\">Every counted item, whatever the range.</p>".to_string();
    let tiles: Vec<String> = flow::statuses(&cx.items)
        .into_iter()
        .map(|(status, n)| {
            card(
                &sentence(&status),
                &n.to_string(),
                "items now",
                Some(href("/beads", &outside, &[("state", &status)])),
            )
        })
        .collect();
    if tiles.is_empty() {
        body.push_str("<p class=\"empty\">No items match these filters.</p>");
    } else {
        body.push_str(&cards(&tiles));
    }
    let bars: Vec<Bar> = flow::open_by_priority(&cx.items)
        .into_iter()
        .map(|(p, n)| Bar {
            label: format!("P{p}"),
            value: n as f64,
            title: format!("P{p}: {}", plural(n, "item", "items")),
            tone: if p <= 1 { Tone::Warn } else { Tone::Neutral },
        })
        .collect();
    body.push_str(&scroll(svg::hbars(
        "Items not closed, by priority",
        &bars,
        |v| format!("{v:.0}"),
    )));
    body.push_str("</section>");
    body.push_str(&throughput(cx));
    body.push_str(&headlines(cx));
    body.push_str(&method("Counted items are the work items the profile's [flow] items rule selects, narrowed by the filters; receipts and hidden rows are never counted. Status counts ignore the range. An empty status filter means every status here, so closed work is counted."));
    (
        range_lede(cx, &plural(cx.items.len(), "item counted", "items counted")),
        body,
    )
}

fn throughput(cx: &Ctx) -> String {
    let t = flow::throughput(cx.view, &cx.window, &cx.items);
    let unit = if cx.window.weekly() {
        "ISO week"
    } else {
        "UTC day"
    };
    let total = |v: &[usize]| v.iter().sum::<usize>();
    let series = [
        format!("Created ({})", total(&t.created)),
        format!("Claimed ({})", total(&t.claimed)),
        format!("Closed ({})", total(&t.closed)),
    ];
    let names: Vec<&str> = series.iter().map(String::as_str).collect();
    let groups: Vec<(String, Vec<f64>)> = t
        .buckets
        .iter()
        .enumerate()
        .map(|(i, b)| {
            (
                b.label.clone(),
                vec![t.created[i] as f64, t.claimed[i] as f64, t.closed[i] as f64],
            )
        })
        .collect();
    let mut out = format!(
        "<section id=\"flow-throughput\"><h2>Throughput</h2><p class=\"muted\">Items created, claimed and closed per {unit} in the range.</p><div class=\"chart-scroll\">{}</div>",
        svg::grouped_bars(
            &format!("Created, claimed and closed per {unit}"),
            &names,
            &groups
        )
    );
    if t.claim_unknown > 0 {
        write!(
            out,
            "<p class=\"left-out\">{} in the range not in the chart: claim time unknown</p>",
            plural(t.claim_unknown, "claim", "claims")
        )
        .unwrap();
    }
    let mut rows = String::new();
    for (i, b) in t.buckets.iter().enumerate() {
        write!(
            rows,
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            escape(&b.name),
            t.created[i],
            t.claimed[i],
            t.closed[i]
        )
        .unwrap();
    }
    write!(
        out,
        "<details id=\"flow-throughput-table\"><summary><span>The same numbers as a table</span></summary>{}</details></section>",
        table(
            &[if cx.window.weekly() { "Week" } else { "Day" }, "Created", "Claimed", "Closed"],
            &[1, 2, 3],
            &rows
        )
    )
    .unwrap();
    out
}

/// One card per sub-view with its headline number.
fn headlines(cx: &Ctx) -> String {
    let view = cx.view;
    let link = |p: FlowPage| Some(href(p.path_and_title().0, cx.facets, &[]));
    let created: Vec<&Item> = cx
        .items
        .iter()
        .filter(|i| i.created.is_some_and(|c| cx.window.contains(c)))
        .collect();
    let creators = flow::tally(created.iter().copied(), |i| {
        i.creator.clone().unwrap_or_default()
    })
    .len();
    let claims: Vec<&Item> = cx
        .items
        .iter()
        .filter(|i| i.claimed_in(&cx.window, view))
        .collect();
    let unknown = claims
        .iter()
        .filter(|i| i.claim.state == ClaimState::TimeUnknown)
        .count();
    let timings = flow::timings(&cx.window, &cx.items);
    let m = flow::measures(&timings);
    let wip = flow::wip(view, &cx.items);
    let oldest = wip
        .first()
        .and_then(|i| i.claim.at)
        .map(|at| flow::age_days(at, cx.now));
    let decision_items = flow::decision_items(view, cx.facets, cx.now);
    let decisions = flow::decisions(view, &decision_items, cx.now);
    let stale = decisions.iter().filter(|d| d.stale(cx.now)).count();
    let past = decisions.iter().filter(|d| d.past_due).count();
    let cards_html = [
        card(
            "Created",
            &created.len().to_string(),
            &format!(
                "in the range, by {}",
                plural(creators, "creator", "creators")
            ),
            link(FlowPage::Created),
        ),
        card(
            "Claimed",
            &(claims.len() - unknown).to_string(),
            &format!("in the range · {unknown} with claim time unknown"),
            link(FlowPage::Claimed),
        ),
        card(
            "Cycle time p50",
            &duration(m.cycle.p50),
            &format!(
                "p90 {} · {} left out: claim time unknown",
                m.cycle.p90.map_or("no data".into(), fmt::seconds),
                plural(m.claim_unknown, "item", "items")
            ),
            link(FlowPage::Cycle),
        ),
        card(
            "Work in progress",
            &wip.len().to_string(),
            &match oldest {
                Some(d) => format!("oldest claimed {}", days(d)),
                None if wip.is_empty() => "nothing in progress".into(),
                None => "claim times unknown".into(),
            },
            link(FlowPage::Ageing),
        ),
        card(
            "Open decisions",
            &decisions.len().to_string(),
            &format!("{stale} waiting over {STALE_DAYS} d · {past} past due"),
            link(FlowPage::Decisions),
        ),
    ];
    format!(
        "<section id=\"flow-views\"><h2>Views</h2>{}</section>",
        cards(&cards_html)
    )
}

// ---------------------------------------------------------------------------
// Created
// ---------------------------------------------------------------------------

fn created(cx: &Ctx) -> (String, String) {
    let mut rows: Vec<&Item> = cx
        .items
        .iter()
        .filter(|i| i.created.is_some_and(|c| cx.window.contains(c)))
        .collect();
    rows.sort_by(|a, b| {
        b.created
            .cmp(&a.created)
            .then_with(|| a.bead.id.cmp(&b.bead.id))
    });
    let unknown = cx.items.iter().filter(|i| i.created.is_none()).count();
    let per = flow::tally(rows.iter().copied(), |i| {
        i.creator.clone().unwrap_or_else(|| flow::UNKNOWN.into())
    });
    let bars: Vec<Bar> = per
        .iter()
        .map(|(name, n)| Bar {
            label: name.clone(),
            value: *n as f64,
            title: format!("{name}: {}", plural(*n, "item", "items")),
            tone: Tone::Good,
        })
        .collect();
    let mut body = "<section id=\"flow-creators\"><h2>By creator</h2>".to_string();
    if per.is_empty() {
        body.push_str("<p class=\"empty\">Nothing was created in the range.</p>");
    } else {
        body.push_str(&scroll(svg::hbars(
            "Items created per person",
            &bars,
            |v| format!("{v:.0}"),
        )));
        body.push_str("<ul class=\"flow-people\" aria-label=\"Show one creator's items\">");
        for (name, n) in &per {
            if name == flow::UNKNOWN {
                write!(body, "<li>{UNKNOWN} <span class=\"muted\">{n}</span></li>").unwrap();
                continue;
            }
            let here = if cx.facets.person == *name {
                " aria-current=\"true\""
            } else {
                ""
            };
            write!(
                body,
                "<li><a href=\"{}\"{here}>{}</a> <span class=\"muted\">{n}</span></li>",
                escape(&href(cx.path, cx.facets, &[("person", name)])),
                escape(name)
            )
            .unwrap();
        }
        if !cx.facets.person.is_empty() {
            write!(
                body,
                "<li><a href=\"{}\">Everyone</a></li>",
                escape(&href(cx.path, cx.facets, &[("person", "")]))
            )
            .unwrap();
        }
        body.push_str("</ul>");
    }
    body.push_str("</section><section id=\"flow-created-items\"><h2>Items, newest first</h2>");
    let (shown, pager) = paged(cx, &rows);
    let mut trs = String::new();
    for item in shown {
        write!(
            trs,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            item_cell(item),
            escape(&words(&item.bead.issue_type)),
            person_cell(item.creator.as_deref()),
            time_cell(item.created),
            badge(&item.bead.status)
        )
        .unwrap();
    }
    if rows.is_empty() {
        body.push_str("<p class=\"empty\">No items were created in the range.</p>");
    } else {
        body.push_str(&table(
            &["Item", "Type", "Created by", "Created", "Status"],
            &[],
            &trs,
        ));
        body.push_str(&pager);
    }
    body.push_str("</section>");
    let creator_note = if cx.view.profile.role("creator").is_some() {
        "The creator is the profile's creator role, which may read the item's history."
    } else {
        "The creator is the profile's creator role when it has one, else the stored created_by."
    };
    body.push_str(&method(&format!("{creator_note} A creator nobody records is shown as unknown. Choosing a person narrows every flow view to the items they created, claimed or hold.")));
    let mut what = plural(rows.len(), "item created", "items created");
    if unknown > 0 {
        write!(what, " · {unknown} with no created time").unwrap();
    }
    (range_lede(cx, &what), body)
}

// ---------------------------------------------------------------------------
// Claimed
// ---------------------------------------------------------------------------

fn claimed(cx: &Ctx) -> (String, String) {
    let view = cx.view;
    let mut rows: Vec<&Item> = cx
        .items
        .iter()
        .filter(|i| i.claimed_in(&cx.window, view))
        .collect();
    rows.sort_by(|a, b| match (a.claim.at, b.claim.at) {
        (Some(x), Some(y)) => y.cmp(&x).then_with(|| a.bead.id.cmp(&b.bead.id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.bead.id.cmp(&b.bead.id),
    });
    let known = rows
        .iter()
        .filter(|i| i.claim.state == ClaimState::Claimed)
        .count();
    let unknown = rows.len() - known;
    let unread = rows.iter().filter(|i| i.claim.unread).count();
    let other = rows.iter().filter(|i| i.held_by_other()).count();
    let mut body = cards(&[
        card(
            "Claimed in the range",
            &known.to_string(),
            "claim time known",
            None,
        ),
        card(
            "Claim time unknown",
            &unknown.to_string(),
            &if unread > 0 {
                format!("claimed or in progress, no claim time · {unread} not read yet")
            } else {
                "claimed or in progress, no claim time".into()
            },
            None,
        ),
        card(
            "Held by someone else now",
            &other.to_string(),
            "the holder now is not the claimant",
            None,
        ),
    ]);
    body.push_str("<section id=\"flow-claims\"><h2>Every claim, newest first</h2>");
    let (shown, pager) = paged(cx, &rows);
    let mut trs = String::new();
    for item in shown {
        let holder = match (&item.holder, item.held_by_other()) {
            (Some(h), true) => format!(
                "{} {}",
                person_link(h),
                tone_badge("someone else", Tone::Warn)
            ),
            (Some(h), false) => person_link(h),
            (None, _) => "<span class=\"muted\">—</span>".into(),
        };
        let at = if item.claim.unread && item.claim.at.is_none() {
            "<span class=\"unknown\">unknown (not read yet)</span>".to_string()
        } else {
            time_cell(item.claim.at)
        };
        write!(
            trs,
            "<tr><td>{}</td><td>{}</td><td>{at}</td><td>{}</td><td>{holder}</td></tr>",
            item_cell(item),
            person_cell(item.claim.by.as_deref()),
            badge(&item.bead.status)
        )
        .unwrap();
    }
    if rows.is_empty() {
        body.push_str("<p class=\"empty\">Nothing was claimed in the range.</p>");
    } else {
        body.push_str(&table(
            &[
                "Item",
                "Claimed by",
                "Claimed at",
                "Status now",
                "Held now by",
            ],
            &[],
            &trs,
        ));
        body.push_str(&pager);
    }
    body.push_str("</section>");
    body.push_str(&method(&format!(
        "The claim time is the first of the profile's [flow] claim_at sources that has a value ({}); the claimant the first of its claimant sources, else the assignee. An item with a claimant or in a work-in-progress status but no claim time is listed with its claim time unknown when it was created in the range or is still in progress; it is never counted as claimed at 0.",
        sources(&view.profile.flow.claim_at)
    )));
    (
        range_lede(cx, &plural(rows.len(), "claim listed", "claims listed")),
        body,
    )
}

fn sources(list: &[crate::profile::Source]) -> String {
    if list.is_empty() {
        return "none".into();
    }
    list.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// Cycle and lead time
// ---------------------------------------------------------------------------

fn stat_panel(title: &str, stats: &Stats, extra: &str) -> String {
    format!(
        "<div class=\"flow-stat\"><h3>{}</h3><dl><div><dt>p50</dt><dd>{}</dd></div><div><dt>p90</dt><dd>{}</dd></div><div><dt>counted</dt><dd>{}</dd></div></dl>{extra}</div>",
        escape(title),
        duration(stats.p50),
        duration(stats.p90),
        stats.n
    )
}

fn cycle(cx: &Ctx) -> (String, String) {
    let view = cx.view;
    let rows = flow::timings(&cx.window, &cx.items);
    let total = flow::measures(&rows);
    let mut extra = left_out(total.claim_unknown);
    if total.claim_after_close > 0 {
        write!(
            extra,
            "<p class=\"left-out\">{} left out: claimed after it closed</p>",
            plural(total.claim_after_close, "item", "items")
        )
        .unwrap();
    }
    let lead_extra = if total.lead_unknown > 0 {
        format!(
            "<p class=\"left-out\">{} left out: created time unknown</p>",
            plural(total.lead_unknown, "item", "items")
        )
    } else {
        "<p class=\"left-out\">none left out</p>".to_string()
    };
    let mut body = format!(
        "<div class=\"flow-stats\">{}{}</div>",
        stat_panel("Cycle time · claim to close", &total.cycle, &extra),
        stat_panel("Lead time · created to close", &total.lead, &lead_extra)
    );
    // Breakdown.
    let by = By::chosen(view, cx.facets);
    let label = by.label(view);
    body.push_str("<section id=\"flow-breakdown\"><h2>Breakdown</h2><nav class=\"flow-by\" aria-label=\"Break down by\"><span class=\"muted\">By</span>");
    for option in By::options(view) {
        let here = if option == by {
            " aria-current=\"true\""
        } else {
            ""
        };
        write!(
            body,
            "<a href=\"{}\"{here}>{}</a>",
            escape(&href(cx.path, cx.facets, &[("by", option.key())])),
            escape(&option.label(view))
        )
        .unwrap();
    }
    body.push_str("</nav>");
    let parts = flow::breakdown(view, &by, &rows);
    let spreads: Vec<Spread> = parts
        .iter()
        .filter_map(|(value, m)| {
            Some(Spread {
                label: value.clone(),
                p50: m.cycle.p50?,
                p90: m.cycle.p90?,
                n: m.cycle.n,
            })
        })
        .collect();
    body.push_str(&scroll(svg::range_bars(
        &format!(
            "Cycle time by {}: median (dot) to 90th percentile",
            label.to_lowercase()
        ),
        &spreads,
        fmt::seconds,
    )));
    let mut trs = String::new();
    for (value, m) in &parts {
        write!(
            trs,
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            breakdown_value(value),
            m.closed,
            duration(m.cycle.p50),
            duration(m.cycle.p90),
            m.claim_unknown,
            duration(m.lead.p50),
            duration(m.lead.p90),
        )
        .unwrap();
    }
    if parts.is_empty() {
        body.push_str("<p class=\"empty\">Nothing closed in the range.</p>");
    } else {
        body.push_str(&table(
            &[
                &label,
                "Closed",
                "Cycle p50",
                "Cycle p90",
                "Left out: claim time unknown",
                "Lead p50",
                "Lead p90",
            ],
            &[1, 2, 3, 4, 5, 6],
            &trs,
        ));
        if matches!(by, By::Group(_)) {
            body.push_str("<p class=\"muted\">An item with several values in this group counts once under each.</p>");
        }
    }
    body.push_str("</section>");
    body.push_str(&per_item(cx, &rows, &by));
    body.push_str(&method("Items closed in the range. Cycle time runs from the claim to the close, lead time from creation to the close, both in elapsed time. p50 and p90 are by nearest rank: the value at rank ⌈p/100 × n⌉ of the sorted times, never interpolated. An item without a known claim time has no cycle time; it is left out of the cycle percentiles and counted beside them."));
    (
        range_lede(cx, &plural(total.closed, "item closed", "items closed")),
        body,
    )
}

fn breakdown_value(value: &str) -> String {
    if value == flow::UNKNOWN || value == flow::NONE {
        format!("<span class=\"unknown\">{}</span>", escape(value))
    } else {
        escape(value)
    }
}

fn per_item(cx: &Ctx, rows: &[Timing], by: &By) -> String {
    let by_lead = cx.facets.sort == "lead";
    let mut sorted: Vec<&Timing> = rows.iter().collect();
    let key = |t: &Timing| -> Option<f64> { if by_lead { t.lead } else { t.cycle.ok() } };
    sorted.sort_by(|a, b| match (key(a), key(b)) {
        (Some(x), Some(y)) => y
            .total_cmp(&x)
            .then_with(|| a.item.bead.id.cmp(&b.item.bead.id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.item.bead.id.cmp(&b.item.bead.id),
    });
    let mut out = "<section id=\"flow-cycle-items\"><h2>Per item</h2><nav class=\"flow-by\" aria-label=\"Sort items\"><span class=\"muted\">Longest first by</span>".to_string();
    for (key, label, on) in [
        ("cycle", "cycle time", !by_lead),
        ("lead", "lead time", by_lead),
    ] {
        let here = if on { " aria-current=\"true\"" } else { "" };
        write!(
            out,
            "<a href=\"{}\"{here}>{label}</a>",
            escape(&href(cx.path, cx.facets, &[("sort", key)]))
        )
        .unwrap();
    }
    out.push_str("</nav>");
    let (shown, pager) = paged(cx, &sorted);
    let mut trs = String::new();
    for t in shown {
        let cycle = match t.cycle {
            Ok(s) => escape(&fmt::seconds(s)),
            Err(flow::Gap::ClaimUnknown) => UNKNOWN.to_string(),
            Err(flow::Gap::ClaimAfterClose) => {
                "<span class=\"unknown\">claimed after close</span>".to_string()
            }
        };
        let values = by.values(cx.view, t.item).join(", ");
        write!(
            trs,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{cycle}</td><td class=\"num\">{}</td><td>{}</td></tr>",
            item_cell(t.item),
            breakdown_value(&values),
            person_cell(t.item.claim.by.as_deref()),
            t.lead.map_or_else(|| UNKNOWN.to_string(), |s| escape(&fmt::seconds(s))),
            time_cell(t.item.closed)
        )
        .unwrap();
    }
    if rows.is_empty() {
        out.push_str("<p class=\"empty\">Nothing closed in the range.</p>");
    } else {
        out.push_str(&table(
            &[
                "Item",
                &by.label(cx.view),
                "Claimant",
                "Cycle",
                "Lead",
                "Closed",
            ],
            &[3, 4],
            &trs,
        ));
        out.push_str(&pager);
    }
    out.push_str("</section>");
    out
}

// ---------------------------------------------------------------------------
// Ageing
// ---------------------------------------------------------------------------

fn ageing(cx: &Ctx) -> (String, String) {
    let view = cx.view;
    let wip = flow::wip(view, &cx.items);
    let ages: Vec<i64> = wip
        .iter()
        .filter_map(|i| i.claim.at.map(|at| flow::age_days(at, cx.now)))
        .collect();
    let unknown = wip.len() - ages.len();
    let oldest = ages.iter().copied().max();
    let held = |item: &Item| {
        view.row(item.index).is_some_and(|r| {
            r.states.iter().any(|k| {
                view.profile
                    .state(k)
                    .is_some_and(|s| s.overrides_ready || s.tone == Tone::Bad)
            })
        })
    };
    let held_count = wip.iter().filter(|i| held(i)).count();
    let statuses = view.profile.flow.wip_status.join(", ");
    let mut body = cards(&[
        card(
            "Work in progress",
            &wip.len().to_string(),
            &format!("status {}", words(&statuses)),
            None,
        ),
        card(
            "Oldest",
            &oldest.map_or_else(|| NO_DATA.to_string(), days),
            "since its claim",
            None,
        ),
        card(
            "Claim time unknown",
            &unknown.to_string(),
            "listed last, not aged",
            None,
        ),
        card(
            "Held",
            &held_count.to_string(),
            "in a state that holds work back",
            None,
        ),
    ]);
    body.push_str("<section id=\"flow-ageing\">");
    if !ages.is_empty() {
        body.push_str(&scroll(svg::age_bars(
            "Work in progress by days since its claim",
            &flow::age_buckets(&ages),
        )));
    }
    body.push_str(&left_out(unknown));
    let (shown, pager) = paged(cx, &wip);
    let scale = oldest.unwrap_or(0).max(1);
    let mut trs = String::new();
    for item in shown {
        let age = item.claim.at.map(|at| flow::age_days(at, cx.now));
        let age_cell = match age {
            Some(d) => format!(
                "{}<meter min=\"0\" max=\"{scale}\" value=\"{d}\" title=\"{} of the oldest\">{}</meter>",
                days(d),
                days(d),
                days(d)
            ),
            None => UNKNOWN.to_string(),
        };
        let row_class = if held(item) { " class=\"held\"" } else { "" };
        let row = view
            .row(item.index)
            .map(|r| state_badges(view, r))
            .unwrap_or_default();
        write!(
            trs,
            "<tr{row_class}><td>{}</td><td>{} {row}</td><td>{}</td><td>{}</td><td class=\"num age\">{age_cell}</td></tr>",
            item_cell(item),
            badge(&item.bead.status),
            person_cell(item.claim.by.as_deref()),
            time_cell(item.claim.at),
        )
        .unwrap();
    }
    if wip.is_empty() {
        body.push_str("<p class=\"empty\">Nothing is in progress.</p>");
    } else {
        body.push_str(&table(
            &["Item", "Status", "Claimed by", "Claimed", "Age"],
            &[4],
            &trs,
        ));
        body.push_str(&pager);
    }
    body.push_str("</section>");
    body.push_str(&method(&format!(
        "Items whose status is one of the profile's [flow] wip_status ({}), whatever the range, oldest claim first. Age is whole days since the claim; an item with no known claim time is listed last and not aged. Rows marked held are in a state that holds work back.",
        statuses
    )));
    (
        format!(
            "Now · {}",
            plural(wip.len(), "item in progress", "items in progress")
        ),
        body,
    )
}

// ---------------------------------------------------------------------------
// Decisions
// ---------------------------------------------------------------------------

fn decisions(cx: &Ctx) -> (String, String) {
    let view = cx.view;
    let list = flow::decisions(view, &cx.items, cx.now);
    let stale = list.iter().filter(|d| d.stale(cx.now)).count();
    let past = list.iter().filter(|d| d.past_due).count();
    let nobody = list.iter().filter(|d| d.assignee.is_none()).count();
    let oldest = list.first().and_then(|d| d.age(cx.now));
    let mut body = cards(&[
        card("Waiting", &list.len().to_string(), "open decisions", None),
        card(
            "Oldest",
            &oldest.map_or_else(|| NO_DATA.to_string(), days),
            &format!("{stale} waiting over {STALE_DAYS} d"),
            None,
        ),
        card("No one assigned", &nobody.to_string(), "nobody named", None),
        card("Past due", &past.to_string(), "due date passed", None),
    ]);
    body.push_str("<section id=\"flow-decisions\"><h2>Open decisions, oldest first</h2>");
    let (shown, pager) = paged(cx, &list);
    let mut trs = String::new();
    for d in shown {
        let classes = [
            d.stale(cx.now).then_some("stale"),
            d.past_due.then_some("past-due"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        let class = if classes.is_empty() {
            String::new()
        } else {
            format!(" class=\"{classes}\"")
        };
        let assignee = d
            .assignee
            .as_deref()
            .map_or_else(|| tone_badge("no one", Tone::Warn), person_link);
        let due = match (&d.due, d.past_due) {
            (Some(due), true) => format!("{} {}", escape(due), tone_badge("past due", Tone::Bad)),
            (Some(due), false) => escape(due),
            (None, _) => "<span class=\"muted\">—</span>".into(),
        };
        let age = match d.age(cx.now) {
            Some(n) if n > STALE_DAYS => {
                format!("{} {}", days(n), tone_badge("over a week", Tone::Warn))
            }
            Some(n) => days(n),
            None => UNKNOWN.to_string(),
        };
        write!(
            trs,
            "<tr{class}><td>{}</td><td>{assignee}</td><td>{}</td><td class=\"num\">{age}</td><td>{due}</td><td>{}</td></tr>",
            item_cell(d.item),
            time_cell(d.since),
            badge(&d.item.bead.status)
        )
        .unwrap();
    }
    if list.is_empty() {
        body.push_str("<p class=\"empty\">No decisions are waiting.</p>");
    } else {
        body.push_str(&table(
            &[
                "Decision",
                "Waiting on",
                "Waiting since",
                "Age",
                "Due",
                "Status",
            ],
            &[3],
            &trs,
        ));
        body.push_str(&pager);
    }
    body.push_str("</section>");
    let rule = &view.profile.flow.decisions;
    body.push_str(&method(&format!(
        "Items the profile's [flow.decisions] rule selects, whatever the range, oldest first. Age counts whole days from {}; over {STALE_DAYS} days is highlighted, and a due date before today (UTC) is marked past due.",
        rule.since
            .as_ref()
            .map_or_else(|| "when the item was created".to_string(), |s| s.to_string())
    )));
    (
        format!(
            "Now · {}",
            plural(list.len(), "decision waiting", "decisions waiting")
        ),
        body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::tests::{at, bead, facets, generated, view_of};

    fn now() -> DateTime<Utc> {
        at("2026-11-15T12:00:00Z")
    }

    fn render(view: &View, page: FlowPage, query: &str) -> String {
        flow_at(view, page, &facets(query), now())
    }

    #[test]
    fn every_page_renders_the_work_fixture_without_inline_styles() {
        let view = crate::testing::work_view();
        for p in FlowPage::ALL {
            let html = render(&view, p, "milestone=v1&days=30");
            assert!(!html.contains("style="), "{p:?}");
            assert!(html.contains("class=\"flow-tabs\""), "{p:?}");
            // Every tab carries the filters.
            for q in FlowPage::ALL {
                let path = q.path_and_title().0;
                assert!(
                    html.contains(&format!("href=\"{path}?days=30&amp;milestone=v1\"")),
                    "{p:?} → {q:?}: {html}"
                );
            }
        }
        let overview = render(&view, FlowPage::Overview, "");
        assert!(overview.contains("id=\"flow-throughput-table\""));
        assert!(overview.contains("series-2"), "three series");
        assert!(overview.contains("href=\"/flow/cycle\""), "a card per view");
        let cycle = render(&view, FlowPage::Cycle, "");
        assert!(cycle.contains("left out: claim time unknown"), "{cycle}");
        assert!(cycle.contains("WORK-0002") || cycle.contains("wk-2"));
        let decisions = render(&view, FlowPage::Decisions, "");
        assert!(decisions.contains("past due"));
        assert!(decisions.contains("class=\"stale past-due\""));
        assert!(decisions.contains("/person/bob"));
        let claimed = render(&view, FlowPage::Claimed, "");
        assert!(claimed.contains("/bead/wk%2D1"), "{claimed}");
        let ageing = render(&view, FlowPage::Ageing, "");
        assert!(ageing.contains("<meter"), "{ageing}");
        let created = render(&view, FlowPage::Created, "");
        assert!(
            created.contains("href=\"/flow/created?person=alice\""),
            "{created}"
        );
    }

    #[test]
    fn unknown_is_never_drawn_as_zero_and_no_data_says_so() {
        let view = view_of(Vec::new(), None, now());
        let cycle = render(&view, FlowPage::Cycle, "");
        assert!(cycle.contains("no data"));
        assert!(cycle.contains("0 items left out: claim time unknown"));
        let mut unknown = bead("u-1", "closed", "2026-11-01T00:00:00Z");
        unknown.closed_at = Some("2026-11-10T00:00:00Z".into());
        let view = view_of(vec![unknown], None, now());
        let cycle = render(&view, FlowPage::Cycle, "");
        assert!(
            cycle.contains("1 item left out: claim time unknown"),
            "{cycle}"
        );
        assert!(!cycle.contains(">0s<"), "an unknown cycle is not 0 s");
        let overview = render(&view, FlowPage::Overview, "");
        assert!(overview.contains("Cycle time p50</span><strong><span class=\"unknown\">no data"));
    }

    #[test]
    fn the_breakdown_and_sort_links_keep_the_filters() {
        let view = crate::testing::work_view();
        let html = render(
            &view,
            FlowPage::Cycle,
            "by=rig&state=closed&from=2026-10-01&to=2026-11-15",
        );
        assert!(html.contains("href=\"/flow/cycle?state=closed&amp;from=2026%2D10%2D01&amp;to=2026%2D11%2D15&amp;by=type\""), "{html}");
        assert!(html.contains("aria-current=\"true\">Rig</a>"));
        assert!(html.contains("Cycle time by rig"));
        assert!(html.contains("sort=lead"));
        // Links that leave the flow pages drop the breakdown key.
        let overview = render(&view, FlowPage::Overview, "by=rig&state=closed");
        assert!(
            overview.contains("href=\"/beads?state=closed\""),
            "{overview}"
        );
        assert!(!overview.contains("/beads?state=closed&amp;by"));
    }

    #[test]
    fn no_profile_pages_use_generic_defaults() {
        let mut d = bead("d-1", "open", "2026-11-01T00:00:00Z");
        d.issue_type = "decision".into();
        d.assignee = "carol".into();
        let mut w = bead("d-2", "in_progress", "2026-11-01T00:00:00Z");
        w.started_at = Some("2026-11-05T00:00:00Z".into());
        w.assignee = "dan".into();
        w.created_by = "erin".into();
        let view = view_of(vec![d, w], None, now());
        for p in FlowPage::ALL {
            let html = render(&view, p, "");
            assert!(html.contains("<div class=\"flow\">"), "{p:?}");
        }
        assert!(render(&view, FlowPage::Decisions, "").contains("/person/carol"));
        let ageing = render(&view, FlowPage::Ageing, "");
        assert!(ageing.contains("10 d"), "{ageing}");
        assert!(render(&view, FlowPage::Created, "").contains("/person/erin"));
        assert!(render(&view, FlowPage::Cycle, "").contains("aria-current=\"true\">Type</a>"));
    }

    #[test]
    fn weekly_buckets_past_31_days() {
        let view = crate::testing::work_view();
        let html = render(&view, FlowPage::Overview, "days=90");
        assert!(html.contains("per ISO week"));
        assert!(html.contains("<th>Week</th>"));
        let html = render(&view, FlowPage::Overview, "days=31");
        assert!(html.contains("per UTC day"));
    }

    /// Each page over a generated 12,000-bead store. The bound is generous
    /// (debug builds, a shared host) but still catches quadratic work: one
    /// O(n²) pass over 12,000 items is ~144M steps, far past it.
    #[test]
    fn every_page_renders_twelve_thousand_beads_quickly() {
        let view = generated(12_000, now());
        let mut report = Vec::new();
        for query in ["", "days=90", "by=claimant&sort=lead&page=3"] {
            for p in FlowPage::ALL {
                let start = std::time::Instant::now();
                let html = render(&view, p, query);
                let took = start.elapsed();
                assert!(html.len() > 1000);
                report.push(format!("{p:?} {query:?}: {took:?}"));
                assert!(took.as_millis() < 500, "{p:?} {query:?} took {took:?}");
            }
        }
        eprintln!("flow page timings on 12,000 beads:\n{}", report.join("\n"));
    }
}
