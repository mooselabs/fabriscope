//! Server-rendered HTML. Every value from a store is escaped; Markdown is
//! rendered without raw HTML or images, and links only ever point at external
//! web pages or at other records on this server.

use crate::beads::{self, Bead};
use crate::model::{BeadFilter, ScrollFilter, View};
use crate::scrolls::{self, Scroll};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

const PAGE_SIZE: usize = 100;

pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn url(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

fn bead_link(id: &str, title: &str) -> String {
    format!("<a href=\"/bead/{}\">{}</a>", url(id), escape(title))
}

fn scroll_link(id: &str, title: &str) -> String {
    format!("<a href=\"/scroll/{}\">{}</a>", url(id), escape(title))
}

fn words(value: &str) -> String {
    value.replace('_', " ")
}

/// A bead's status exactly as Beads stores it. The colour follows the
/// category `bd statuses` gives each built-in status; a custom status is
/// shown in the neutral colour.
fn badge(status: &str) -> String {
    let class = match status {
        "open" => "open",
        "in_progress" | "hooked" => "working",
        "blocked" => "waiting",
        "closed" => "done",
        _ => "neutral",
    };
    format!(
        "<span class=\"badge {class}\">{}</span>",
        escape(&words(status))
    )
}

/// Built-in Beads statuses in the order `bd statuses` lists them.
const STATUS_ORDER: [&str; 7] = [
    "open",
    "in_progress",
    "blocked",
    "deferred",
    "closed",
    "pinned",
    "hooked",
];

/// Each distinct value of `field` among `beads`, with how many beads hold
/// it, so a filter offers exactly what the store contains. Statuses keep the
/// Beads order; anything else, and any custom status, sorts by name.
fn tally<'a>(
    beads: impl Iterator<Item = &'a Bead>,
    field: impl Fn(&'a Bead) -> &'a str,
) -> Vec<(&'a str, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for bead in beads {
        let value = field(bead);
        if !value.is_empty() {
            *counts.entry(value).or_default() += 1;
        }
    }
    let mut out: Vec<_> = counts.into_iter().collect();
    out.sort_by_key(|(value, _)| {
        (
            STATUS_ORDER
                .iter()
                .position(|s| s == value)
                .unwrap_or(STATUS_ORDER.len()),
            *value,
        )
    });
    out
}

/// Filter options for every stored status: beads not closed (the default),
/// all beads, then each status the store holds.
fn status_options<'a>(beads: impl Iterator<Item = &'a Bead>) -> Vec<(String, String)> {
    let mut options = vec![
        (String::new(), "Not closed".to_string()),
        ("all".to_string(), "All".to_string()),
    ];
    options.extend(
        tally(beads, |b| b.status.as_str())
            .into_iter()
            .map(|(status, n)| (status.to_string(), format!("{} ({n})", words(status)))),
    );
    options
}

fn selected(on: bool) -> &'static str {
    if on { " selected" } else { "" }
}

fn short(value: &str) -> &str {
    value.get(..9).unwrap_or(value)
}

fn page(view: &View, title: &str, tab: &str, body: &str) -> String {
    let mut nav = String::new();
    let mut tabs = vec![("/", "Overview")];
    if view.beads.is_some() {
        tabs.extend([("/beads", "Beads"), ("/epics", "Epics")]);
    }
    if view.scrolls.is_some() {
        tabs.push(("/scrolls", "Scrolls"));
    }
    for (path, text) in tabs {
        let current = if tab == path {
            " aria-current=\"page\""
        } else {
            ""
        };
        write!(nav, "<a href=\"{path}\"{current}>{text}</a>").unwrap();
    }
    let mut alerts = String::new();
    for error in view.errors() {
        write!(
            alerts,
            "<p class=\"alert\" role=\"alert\">{}</p>",
            escape(error)
        )
        .unwrap();
    }
    let mut provenance = Vec::new();
    if let Some(feed) = &view.beads {
        let commit = feed
            .data
            .commit
            .as_deref()
            .map(|c| format!(" · commit {}", short(c)))
            .unwrap_or_default();
        let via = if feed.data.source.is_empty() {
            String::new()
        } else {
            format!(" via {}", feed.data.source)
        };
        provenance.push(format!(
            "Beads{}{} · read {}",
            escape(&via),
            escape(&commit),
            escape(&feed.data.generated_at)
        ));
    }
    if let Some(feed) = &view.scrolls {
        let revision = feed
            .data
            .revision
            .as_deref()
            .map(|r| format!(" · revision {}", short(r)))
            .unwrap_or_default();
        provenance.push(format!(
            "Scrolls{} · read {}",
            escape(&revision),
            escape(&feed.data.generated_at)
        ));
    }
    let store = if view.title.is_empty() {
        String::new()
    } else {
        format!(" <span class=\"muted\">/ {}</span>", escape(&view.title))
    };
    let suffix = if view.title.is_empty() {
        "Fabriscope".to_string()
    } else {
        format!("{} · Fabriscope", view.title)
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} · {}</title><script src=\"/theme.js\"></script><link rel=\"stylesheet\" href=\"/app.css\"><script defer src=\"/app.js\"></script></head><body><a class=\"skip\" href=\"#content\">Skip to content</a><header><a class=\"brand\" href=\"/\"><span class=\"brand-mark\">F</span>Fabriscope{store}</a><nav aria-label=\"Main navigation\">{nav}</nav><div class=\"theme-control\" hidden><label for=\"color-theme\">Theme</label><select id=\"color-theme\"><option value=\"system\">System</option><option value=\"light\">Light</option><option value=\"dark\">Dark</option></select></div><span id=\"connection\" role=\"status\">Connecting…</span></header><main id=\"content\" data-revision=\"{}\">{alerts}{body}<footer>{}<br>Refreshes automatically · read-only</footer></main></body></html>",
        escape(title),
        escape(&suffix),
        view.revision,
        provenance.join("<br>"),
    )
}

fn heading(eyebrow: &str, title: &str, description: &str) -> String {
    format!(
        "<div class=\"page-heading\"><p class=\"eyebrow\">{}</p><h1>{}</h1><p class=\"lede\">{}</p></div>",
        escape(eyebrow),
        escape(title),
        escape(description)
    )
}

fn metrics(cards: &[(&str, usize, &str, String)]) -> String {
    let mut out = "<div class=\"metrics\">".to_string();
    for (label, value, detail, href) in cards {
        write!(out, "<a class=\"metric\" href=\"{}\"><span>{}</span><strong>{value}</strong><small>{}</small></a>", escape(href), escape(label), escape(detail)).unwrap();
    }
    out.push_str("</div>");
    out
}

// ---------------------------------------------------------------- history

fn chart(history: &beads::History) -> String {
    let days = &history.days;
    if days.is_empty() {
        return String::new();
    }
    let max = days
        .iter()
        .map(|d| d.end.max(d.start))
        .max()
        .unwrap_or(1)
        .max(1) as f64
        * 1.15;
    let y = |value: usize| 190.0 - (value as f64 / max * 156.0);
    let x = |i: usize| 58.0 + i as f64 / days.len().max(1) as f64 * 802.0;
    let mut out = "<figure><figcaption>Open beads over time <span class=\"legend\"><span class=\"total-key\">Open at the end of each UTC day</span></span></figcaption><svg class=\"chart\" viewBox=\"0 0 900 235\" role=\"img\" aria-label=\"Open bead count at UTC day boundaries\">".to_string();
    for fraction in [0.0, 0.5, 1.0] {
        let value = (max * fraction) as usize;
        write!(out, "<line class=\"gridline\" x1=\"58\" x2=\"860\" y1=\"{}\" y2=\"{}\"/><text x=\"48\" y=\"{}\" text-anchor=\"end\">{value}</text>", y(value), y(value), y(value) + 4.0).unwrap();
    }
    let mut points = format!("{},{}", x(0), y(days[0].start));
    for (i, day) in days.iter().enumerate() {
        write!(points, " {},{}", x(i + 1), y(day.end)).unwrap();
    }
    write!(out, "<polyline class=\"total-line\" points=\"{points}\"/>").unwrap();
    for (i, day) in days.iter().enumerate() {
        write!(out, "<circle class=\"point\" cx=\"{}\" cy=\"{}\" r=\"4\"><title>{}: {} open · {} added · {} closed</title></circle>", x(i + 1), y(day.end), day.date, day.end, day.added, day.closed).unwrap();
    }
    let last = &days[days.len() - 1];
    write!(out, "<text x=\"58\" y=\"220\">{}</text><text x=\"860\" y=\"220\" text-anchor=\"end\">{} · end of day</text></svg></figure>", days[0].date, last.date).unwrap();
    out
}

fn forecast_values(current: usize, change: f64) -> ([usize; 3], String) {
    let remaining =
        [7.0, 14.0, 28.0].map(|days| (current as f64 + change * days).max(0.0).round() as usize);
    let date = if current == 0 {
        "Already empty".into()
    } else if change >= 0.0 {
        "No clearance at this rate".into()
    } else {
        let days = (current as f64 / -change).ceil();
        if days > 3650.0 {
            "Beyond 10 years".into()
        } else {
            (chrono::Utc::now().date_naive() + chrono::Duration::days(days as i64)).to_string()
        }
    };
    (remaining, date)
}

fn velocity(s: &beads::Snapshot) -> String {
    let history = &s.history;
    let mut out = "<section id=\"velocity\"><h2>Delivery &amp; forecast</h2>".to_string();
    out.push_str(&chart(history));
    if !history.windows.is_empty() {
        out.push_str("<div class=\"table-wrap\"><table><thead><tr><th>Complete period</th><th>Closed</th><th>Added</th><th>Net open</th><th>Closed / day</th></tr></thead><tbody>");
        for w in &history.windows {
            write!(out, "<tr><td><strong>{} days</strong><small>{} – {}</small></td><td>{}</td><td>{}</td><td class=\"{}\">{:+}</td><td>{:.1}</td></tr>", w.days, w.start, w.end, w.closed, w.added, if w.net_change < 0 { "decreasing" } else { "increasing" }, w.net_change, w.closed as f64 / w.days as f64).unwrap();
        }
        out.push_str("</tbody></table></div><h3>What happens if these rates continue?</h3><p class=\"muted\">Scenarios for every open bead, including new intake. Conditional arithmetic, not delivery dates or promises.</p><div class=\"table-wrap\"><table><thead><tr><th>Scenario</th><th>Net / day</th><th>In 7 days</th><th>In 14 days</th><th>In 28 days</th><th>Would reach zero</th></tr></thead><tbody>");
        let mut scenarios = Vec::new();
        if let Some(w) = history.windows.iter().find(|w| w.days == 3) {
            let rate = w.net_change as f64 / w.days as f64;
            scenarios.push(("Last 3 days continue", rate));
            scenarios.push(("Half the 3-day net change", rate / 2.0));
        }
        if let Some(w) = history.windows.iter().find(|w| w.days == 7) {
            scenarios.push(("Last 7 days continue", w.net_change as f64 / w.days as f64));
        }
        let open = s.count(|b| !b.is_closed());
        for (label, change) in scenarios {
            let (values, date) = forecast_values(open, change);
            write!(out, "<tr><td>{label}</td><td>{change:+.1}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>", values[0], values[1], values[2], escape(&date)).unwrap();
        }
        out.push_str("</tbody></table></div>");
    }
    out.push_str("<p class=\"method\">Complete UTC days, from each bead’s created and closed timestamps. A bead that is open now counts as open since it was created, even if it was closed and reopened in between; deleted beads are not counted. Counts measure beads, not equal-sized units of effort.</p></section>");
    out
}

// ---------------------------------------------------------------- overview

pub fn overview(view: &View) -> String {
    let mut body = heading(
        "Fabriscope",
        "What’s being made, and what’s known.",
        "Live work from Beads and knowledge from Scrolls, in one read-only view.",
    );
    if let Some(feed) = &view.beads {
        let s = &feed.data;
        body.push_str(&metrics(&[
            (
                "Open beads",
                s.count(|b| !b.is_closed()),
                "everything not closed",
                "/beads".into(),
            ),
            (
                "Ready",
                s.ready.len(),
                "as bd ready decides",
                "/beads?ready=yes".into(),
            ),
            (
                "In progress",
                s.count(|b| b.status == "in_progress"),
                "being worked on",
                "/beads?state=in_progress".into(),
            ),
            (
                "Waiting",
                s.count(|b| b.status == "open" && !b.ready),
                "open, but not ready",
                "/beads?state=open&ready=no".into(),
            ),
        ]));
        body.push_str(&velocity(s));
        body.push_str(&ready_next(s));
    }
    if let Some(feed) = &view.scrolls {
        body.push_str(&knowledge(&feed.data));
    }
    page(view, "Overview", "/", &body)
}

fn ready_next(s: &beads::Snapshot) -> String {
    let selected: Vec<&Bead> = s.ready.iter().filter_map(|id| s.get(id)).take(50).collect();
    let mut out = format!(
        "<section id=\"next\"><div class=\"section-heading\"><div><h2>Ready next</h2><p class=\"muted\">The first {} of {} ready beads, in the order bd ready returns them, grouped by parent.</p></div><a href=\"/beads?state=ready\">Browse every ready bead →</a></div>",
        selected.len(),
        s.ready.len()
    );
    if selected.is_empty() {
        out.push_str("<p class=\"empty\">Nothing is ready.</p></section>");
        return out;
    }
    let mut groups: Vec<(&str, Vec<(usize, &Bead)>)> = Vec::new();
    for (i, bead) in selected.iter().enumerate() {
        let parent = bead.parent().unwrap_or("");
        match groups.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, rows)) => rows.push((i + 1, bead)),
            None => groups.push((parent, vec![(i + 1, bead)])),
        }
    }
    for (parent, rows) in groups {
        let title = s
            .get(parent)
            .map(|p| p.title.as_str())
            .unwrap_or("No parent");
        let open = if parent.is_empty() {
            String::new()
        } else {
            format!("<p>{}</p>", bead_link(parent, "Open parent →"))
        };
        write!(out, "<details class=\"theme\" id=\"group-{}\" open><summary><span>{}</span><span class=\"muted\">{} ready</span></summary><div class=\"theme-body\">{open}{}</div></details>", escape(parent), escape(title), rows.len(), bead_table(s, &rows)).unwrap();
    }
    out.push_str("</section>");
    out
}

fn knowledge(s: &scrolls::Snapshot) -> String {
    let mut out = format!(
        "<section id=\"knowledge\"><div class=\"section-heading\"><div><h2>Knowledge</h2><p class=\"muted\">{} scrolls and {} links between them.</p></div><a href=\"/scrolls\">Browse every scroll →</a></div><div class=\"metrics\">",
        s.scrolls.len(),
        s.edges.len()
    );
    for (kind, count) in s.kinds().into_iter().take(8) {
        write!(out, "<a class=\"metric\" href=\"/scrolls?kind={}\"><span>{}</span><strong>{count}</strong><small>kind</small></a>", url(kind), escape(&words(kind))).unwrap();
    }
    out.push_str("</div><h3>Newest scrolls</h3>");
    let newest: Vec<&Scroll> = s.scrolls.iter().take(12).collect();
    out.push_str(&scroll_table(&newest));
    out.push_str("</section>");
    out
}

// ---------------------------------------------------------------- tables

fn bead_table(s: &beads::Snapshot, rows: &[(usize, &Bead)]) -> String {
    if rows.is_empty() {
        return "<p class=\"empty\">No beads match this view.</p>".into();
    }
    let mut out = "<div class=\"table-wrap\"><table class=\"items\"><thead><tr><th>#</th><th>Bead</th><th>State</th><th>Priority</th></tr></thead><tbody>".to_string();
    for (position, bead) in rows {
        let parent = bead
            .parent()
            .and_then(|p| s.get(p))
            .map(|p| format!(" · {}", escape(&p.title)))
            .unwrap_or_default();
        let reference = if bead.external_ref.is_empty() {
            String::new()
        } else {
            format!(" · {}", escape(&bead.external_ref))
        };
        write!(out, "<tr><td class=\"muted\">{position}</td><td>{}<small>{}{reference} · {}{parent}</small></td><td>{}</td><td>P{}</td></tr>", bead_link(&bead.id, &bead.title), escape(&bead.id), escape(&bead.issue_type), badge(&bead.status), bead.priority).unwrap();
    }
    out.push_str("</tbody></table></div>");
    out
}

fn scroll_table(rows: &[&Scroll]) -> String {
    if rows.is_empty() {
        return "<p class=\"empty\">No scrolls match this view.</p>".into();
    }
    let mut out = "<div class=\"table-wrap\"><table class=\"items scrolls\"><thead><tr><th>Kind</th><th>Scroll</th><th>Status</th><th>Created</th></tr></thead><tbody>".to_string();
    for scroll in rows {
        let path = scroll
            .path
            .as_deref()
            .map(|p| format!(" · {}", escape(p)))
            .unwrap_or_default();
        write!(out, "<tr><td class=\"muted\">{}</td><td>{}<small>{}{path}</small></td><td>{}</td><td>{}</td></tr>", escape(&words(&scroll.kind)), scroll_link(&scroll.id, &scroll.title), escape(&scroll.id), badge(&scroll.status), escape(scroll.created.as_deref().unwrap_or(""))).unwrap();
    }
    out.push_str("</tbody></table></div>");
    out
}

fn select(name: &str, label: &str, current: &str, options: &[(String, String)]) -> String {
    let mut out = format!("<label>{}<select name=\"{name}\">", escape(label));
    for (value, text) in options {
        write!(
            out,
            "<option value=\"{}\"{}>{}</option>",
            escape(value),
            selected(current == value),
            escape(text)
        )
        .unwrap();
    }
    out.push_str("</select></label>");
    out
}

fn pagination(path: &str, query: &[(&str, &str)], current: usize, pages: usize) -> String {
    let mut out = "<nav class=\"pagination\" aria-label=\"Results pages\">".to_string();
    let base: String = query
        .iter()
        .map(|(k, v)| format!("{k}={}&amp;", url(v)))
        .collect();
    for (label, target) in [
        ("← Previous", current.checked_sub(1)),
        ("Next →", (current + 1 < pages).then_some(current + 1)),
    ] {
        if let Some(target) = target {
            write!(out, "<a href=\"{path}?{base}page={target}\">{label}</a>").unwrap();
        }
    }
    out.push_str("</nav>");
    out
}

// ---------------------------------------------------------------- beads

pub fn beads(view: &View, filter: &BeadFilter) -> String {
    let Some(feed) = &view.beads else {
        return not_found(view);
    };
    let s = &feed.data;
    let mut body = heading(
        "Work",
        "Beads",
        "Search every bead in priority order, including closed history.",
    );
    let priorities: BTreeSet<i64> = s.beads.iter().map(|b| b.priority).collect();
    let mut types_options = vec![(String::new(), "All types".to_string())];
    types_options.extend(
        tally(s.beads.iter(), |b| b.issue_type.as_str())
            .into_iter()
            .map(|(t, n)| (t.to_string(), format!("{} ({n})", words(t)))),
    );
    let ready_options = [("", "Any"), ("yes", "Ready"), ("no", "Not ready")]
        .map(|(v, t)| (v.to_string(), t.to_string()));
    let mut priority_options = vec![(String::new(), "All priorities".to_string())];
    priority_options.extend(priorities.iter().map(|p| (p.to_string(), format!("P{p}"))));
    write!(body, "<form class=\"filters\" action=\"/beads\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Title, id or label…\"></label>", escape(&filter.q)).unwrap();
    body.push_str(&select(
        "state",
        "State",
        &filter.state,
        &status_options(s.beads.iter()),
    ));
    body.push_str(&select("ready", "Ready", &filter.ready, &ready_options));
    body.push_str(&select("type", "Type", &filter.issue_type, &types_options));
    body.push_str(&select(
        "priority",
        "Priority",
        &filter.priority,
        &priority_options,
    ));
    body.push_str("<button type=\"submit\">Apply</button></form>");
    let items: Vec<&Bead> = s.ordered().filter(|b| filter.matches(b)).collect();
    let pages = items.len().div_ceil(PAGE_SIZE).max(1);
    let current = filter.page.min(pages - 1);
    write!(
        body,
        "<p class=\"muted\">{} matching beads · page {} of {pages}</p>",
        items.len(),
        current + 1
    )
    .unwrap();
    let rows: Vec<_> = items
        .iter()
        .enumerate()
        .skip(current * PAGE_SIZE)
        .take(PAGE_SIZE)
        .map(|(i, b)| (i + 1, *b))
        .collect();
    body.push_str(&bead_table(s, &rows));
    body.push_str(&pagination(
        "/beads",
        &[
            ("q", &filter.q),
            ("state", &filter.state),
            ("ready", &filter.ready),
            ("type", &filter.issue_type),
            ("priority", &filter.priority),
        ],
        current,
        pages,
    ));
    page(view, "Beads", "/beads", &body)
}

fn summary(markdown: &str) -> String {
    let text: String = markdown
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("---"))
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() > 220 {
        format!("{}…", text.chars().take(220).collect::<String>())
    } else {
        text
    }
}

pub fn epics(view: &View, filter: &BeadFilter) -> String {
    let Some(feed) = &view.beads else {
        return not_found(view);
    };
    let s = &feed.data;
    let mut body = heading(
        "Work",
        "Epics",
        "Follow each epic from its remaining children to the ones already closed.",
    );
    write!(body, "<form class=\"filters\" action=\"/epics\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Title or id…\"></label>", escape(&filter.q)).unwrap();
    body.push_str(&select(
        "state",
        "State",
        &filter.state,
        &status_options(s.beads.iter().filter(|b| b.is_epic())),
    ));
    body.push_str("<button type=\"submit\">Apply</button></form>");
    let epics: Vec<&Bead> = s
        .ordered()
        .filter(|b| b.is_epic() && filter.matches(b))
        .collect();
    let membership = match &s.parent_key {
        Some(key) => format!(
            "children are beads with a parent-child dependency on the epic, or whose metadata <code>{}</code> names it",
            escape(key)
        ),
        None => "children are beads with a parent-child dependency on the epic".to_string(),
    };
    write!(
        body,
        "<p class=\"muted\">{} epics · {membership}</p><div class=\"epic-grid\">",
        epics.len()
    )
    .unwrap();
    for epic in epics {
        let children = s.children(&epic.id);
        let closed = children.iter().filter(|c| c.is_closed()).count();
        let ready = children.iter().filter(|c| c.ready).count();
        write!(body, "<article class=\"epic-card\"><div class=\"card-top\"><small>P{} · {}</small>{}</div><h2>{}</h2><p>{}</p><progress max=\"{}\" value=\"{closed}\" aria-label=\"Closed children\"></progress><div class=\"card-bottom\"><span>{closed} / {} closed</span><span>{} open · {ready} ready</span></div></article>", epic.priority, escape(&epic.id), badge(&epic.status), bead_link(&epic.id, &epic.title), escape(&summary(&epic.description)), children.len().max(1), children.len(), children.len() - closed).unwrap();
    }
    body.push_str("</div>");
    page(view, "Epics", "/epics", &body)
}

fn bead_relations(title: &str, rows: &[(&Bead, String)]) -> String {
    let mut out = format!("<h3>{}</h3>", escape(title));
    if rows.is_empty() {
        out.push_str("<p class=\"muted\">None.</p>");
        return out;
    }
    out.push_str("<ul class=\"relations\">");
    for (bead, note) in rows {
        write!(
            out,
            "<li>{}<small>{}{}</small>{}</li>",
            bead_link(&bead.id, &bead.title),
            escape(&bead.id),
            escape(note),
            badge(&bead.status)
        )
        .unwrap();
    }
    out.push_str("</ul>");
    out
}

pub fn bead(view: &View, id: &str) -> Option<String> {
    let s = &view.beads.as_ref()?.data;
    let bead = s.get(id)?;
    let links = Links::new(view);
    let parent = bead.parent().and_then(|p| s.get(p));
    let mut body = format!(
        "<div class=\"breadcrumbs\"><a href=\"/\">Overview</a> / <a href=\"{}\">{}</a>{}</div>",
        if bead.is_epic() { "/epics" } else { "/beads" },
        if bead.is_epic() { "Epics" } else { "Beads" },
        parent
            .map(|p| format!(" / {}", bead_link(&p.id, &p.title)))
            .unwrap_or_default()
    );
    let eyebrow = if bead.external_ref.is_empty() {
        bead.id.clone()
    } else {
        format!("{} · {}", bead.id, bead.external_ref)
    };
    body.push_str(&heading(&eyebrow, &bead.title, ""));
    write!(
        body,
        "<div class=\"record-meta\">{}<span>P{}</span><span>{}</span>",
        badge(&bead.status),
        bead.priority,
        escape(&bead.issue_type)
    )
    .unwrap();
    if bead.ready {
        body.push_str("<span>Ready, as bd ready decides</span>");
    }
    if !bead.assignee.is_empty() {
        write!(body, "<span>Assigned to {}</span>", escape(&bead.assignee)).unwrap();
    }
    for label in &bead.labels {
        write!(body, "<span class=\"label\">{}</span>", escape(label)).unwrap();
    }
    body.push_str("</div>");
    let children = s.children(&bead.id);
    if !children.is_empty() {
        let open: Vec<_> = children
            .iter()
            .copied()
            .filter(|c| !c.is_closed())
            .enumerate()
            .map(|(i, c)| (i + 1, c))
            .collect();
        let closed: Vec<_> = children
            .iter()
            .copied()
            .filter(|c| c.is_closed())
            .enumerate()
            .map(|(i, c)| (i + 1, c))
            .collect();
        write!(body, "<section><h2>Open children · {}</h2>{}<details id=\"closed-children\"><summary>Closed children · {}</summary>{}</details></section>", open.len(), bead_table(s, &open), closed.len(), bead_table(s, &closed)).unwrap();
    }
    body.push_str("<div class=\"detail-grid\"><article class=\"document\">");
    let mut any = false;
    for (title, text) in [
        ("Description", &bead.description),
        ("Design", &bead.design),
        ("Acceptance criteria", &bead.acceptance_criteria),
        ("Notes", &bead.notes),
    ] {
        if text.trim().is_empty() {
            continue;
        }
        if title != "Description" || any {
            write!(body, "<h2>{title}</h2>").unwrap();
        }
        body.push_str(&markdown(text, None, &links));
        any = true;
    }
    if !any {
        body.push_str("<p class=\"muted\">No description.</p>");
    }
    if bead.is_closed() && !bead.close_reason.is_empty() {
        write!(
            body,
            "<h2>Close reason</h2><p>{}</p>",
            escape(&bead.close_reason)
        )
        .unwrap();
    }
    body.push_str("</article><aside>");
    let mut blocking = Vec::new();
    let mut related = Vec::new();
    for dep in &bead.dependencies {
        if dep.kind == "parent-child" {
            continue;
        }
        if let Some(target) = s.get(&dep.depends_on_id) {
            let row = (target, format!(" · {}", words(&dep.kind)));
            if dep.kind == "blocks" {
                blocking.push(row)
            } else {
                related.push(row)
            }
        }
    }
    let mut unblocks = Vec::new();
    for (other, kind) in s.dependents(&bead.id) {
        match kind {
            "parent-child" => {}
            "blocks" => unblocks.push((other, String::new())),
            kind => related.push((other, format!(" · {} (from)", words(kind)))),
        }
    }
    body.push_str(&bead_relations("Depends on", &blocking));
    body.push_str(&bead_relations("Blocks", &unblocks));
    if !related.is_empty() {
        body.push_str(&bead_relations("Related", &related));
    }
    if let Some(feed) = &view.scrolls {
        let mentions = feed.data.mentions_of_bead(&bead.id);
        if !mentions.is_empty() {
            body.push_str("<h3>Scrolls about this bead</h3><ul class=\"relations\">");
            for edge in mentions {
                if let Some(scroll) = feed.data.get(&edge.from) {
                    write!(
                        body,
                        "<li>{}<small>{} · {}</small></li>",
                        scroll_link(&scroll.id, &scroll.title),
                        escape(&words(&scroll.kind)),
                        escape(&words(&edge.rel))
                    )
                    .unwrap();
                }
            }
            body.push_str("</ul>");
        }
    }
    let mut dates = String::new();
    for (label, value) in [
        ("Created", &bead.created_at),
        ("Updated", &bead.updated_at),
        ("Closed", &bead.closed_at),
    ] {
        if let Some(value) = value {
            write!(dates, "{label} {}<br>", escape(value)).unwrap();
        }
    }
    let metadata = serde_json::to_string_pretty(&bead.metadata).unwrap_or_default();
    write!(body, "<details id=\"metadata\"><summary>Metadata</summary><pre>{}</pre></details><p class=\"source-path\">{dates}</p></aside></div>", escape(&metadata)).unwrap();
    Some(page(
        view,
        &bead.title,
        if bead.is_epic() { "/epics" } else { "/beads" },
        &body,
    ))
}

// ---------------------------------------------------------------- scrolls

pub fn scrolls(view: &View, filter: &ScrollFilter) -> String {
    let Some(feed) = &view.scrolls else {
        return not_found(view);
    };
    let s = &feed.data;
    let mut body = heading(
        "Knowledge",
        "Scrolls",
        "Search every record in the knowledge graph, newest first.",
    );
    let mut kinds = vec![(String::new(), "All kinds".to_string())];
    kinds.extend(
        s.kinds()
            .into_iter()
            .map(|(k, n)| (k.to_string(), format!("{} ({n})", words(k)))),
    );
    let statuses: BTreeSet<&str> = s
        .scrolls
        .iter()
        .filter(|r| filter.kind.is_empty() || r.kind == filter.kind)
        .map(|r| r.status.as_str())
        .collect();
    let mut status_options = vec![(String::new(), "All statuses".to_string())];
    status_options.extend(statuses.iter().map(|v| (v.to_string(), words(v))));
    write!(body, "<form class=\"filters\" action=\"/scrolls\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Title, id, path or alias…\"></label>", escape(&filter.q)).unwrap();
    body.push_str(&select("kind", "Kind", &filter.kind, &kinds));
    body.push_str(&select("status", "Status", &filter.status, &status_options));
    body.push_str("<button type=\"submit\">Apply</button></form>");
    let items: Vec<&Scroll> = s.scrolls.iter().filter(|r| filter.matches(r)).collect();
    let pages = items.len().div_ceil(PAGE_SIZE).max(1);
    let current = filter.page.min(pages - 1);
    write!(
        body,
        "<p class=\"muted\">{} matching scrolls · page {} of {pages}</p>",
        items.len(),
        current + 1
    )
    .unwrap();
    let rows: Vec<&Scroll> = items
        .into_iter()
        .skip(current * PAGE_SIZE)
        .take(PAGE_SIZE)
        .collect();
    body.push_str(&scroll_table(&rows));
    body.push_str(&pagination(
        "/scrolls",
        &[
            ("q", &filter.q),
            ("kind", &filter.kind),
            ("status", &filter.status),
        ],
        current,
        pages,
    ));
    page(view, "Scrolls", "/scrolls", &body)
}

fn edge_list(
    view: &View,
    s: &scrolls::Snapshot,
    title: &str,
    edges: &[&scrolls::Edge],
    outgoing: bool,
) -> String {
    let mut out = format!("<h3>{}</h3>", escape(title));
    if edges.is_empty() {
        out.push_str("<p class=\"muted\">None.</p>");
        return out;
    }
    let mut groups: Vec<(&str, Vec<&scrolls::Edge>)> = Vec::new();
    for edge in edges {
        match groups.iter_mut().find(|(rel, _)| *rel == edge.rel) {
            Some((_, list)) => list.push(edge),
            None => groups.push((&edge.rel, vec![edge])),
        }
    }
    for (rel, list) in groups {
        write!(out, "<details class=\"edges\" id=\"{}-{}\"{}><summary>{} · {}</summary><ul class=\"relations\">", if outgoing { "out" } else { "in" }, escape(rel), if list.len() <= 12 { " open" } else { "" }, escape(&words(rel)), list.len()).unwrap();
        for edge in list {
            let other = if outgoing { &edge.to } else { &edge.from };
            let why = edge
                .why
                .as_deref()
                .map(|w| format!(" · {}", escape(w)))
                .unwrap_or_default();
            if let Some(scroll) = s.get(other) {
                write!(
                    out,
                    "<li>{}<small>{}{why}</small></li>",
                    scroll_link(&scroll.id, &scroll.title),
                    escape(&words(&scroll.kind))
                )
                .unwrap();
            } else if let Some(bead) = scrolls::bead_reference(other)
                .and_then(|id| view.beads.as_ref()?.data.get(id).cloned())
            {
                write!(
                    out,
                    "<li>{}<small>bead {}{why}</small>{}</li>",
                    bead_link(&bead.id, &bead.title),
                    escape(&bead.id),
                    badge(&bead.status)
                )
                .unwrap();
            } else {
                write!(
                    out,
                    "<li>{}<small>external{why}</small></li>",
                    escape(other)
                )
                .unwrap();
            }
        }
        out.push_str("</ul></details>");
    }
    out
}

pub fn scroll(view: &View, id: &str) -> Option<String> {
    let s = &view.scrolls.as_ref()?.data;
    let scroll = s.get(id)?;
    let links = Links::new(view);
    let mut body = format!(
        "<div class=\"breadcrumbs\"><a href=\"/\">Overview</a> / <a href=\"/scrolls\">Scrolls</a> / <a href=\"/scrolls?kind={}\">{}</a></div>",
        url(&scroll.kind),
        escape(&words(&scroll.kind))
    );
    body.push_str(&heading(
        &format!("{} · {}", words(&scroll.kind), scroll.id),
        &scroll.title,
        "",
    ));
    write!(body, "<div class=\"record-meta\">{}", badge(&scroll.status)).unwrap();
    for (label, value) in [
        ("Owner", &scroll.owner),
        ("Created", &scroll.created),
        ("Reviewed", &scroll.last_reviewed),
        ("Confidence", &scroll.confidence),
    ] {
        if let Some(value) = value {
            write!(body, "<span>{label} {}</span>", escape(value)).unwrap();
        }
    }
    write!(body, "<span>Revision {}</span></div>", scroll.revision).unwrap();
    body.push_str("<div class=\"detail-grid\"><article class=\"document\">");
    if scroll.body.trim().is_empty() {
        body.push_str("<p class=\"muted\">No body.</p>");
    } else {
        body.push_str(&markdown(&scroll.body, scroll.path.as_deref(), &links));
    }
    body.push_str("</article><aside>");
    body.push_str(&edge_list(
        view,
        s,
        "Links from here",
        &s.outgoing(&scroll.id),
        true,
    ));
    body.push_str(&edge_list(
        view,
        s,
        "Links to here",
        &s.incoming(&scroll.id),
        false,
    ));
    if !scroll.aliases.is_empty() {
        body.push_str("<h3>Aliases</h3><ul class=\"relations\">");
        for alias in &scroll.aliases {
            write!(body, "<li><code>{}</code></li>", escape(alias)).unwrap();
        }
        body.push_str("</ul>");
    }
    let fields = serde_json::to_string_pretty(&scroll.fields).unwrap_or_default();
    write!(body, "<details id=\"metadata\"><summary>Fields</summary><pre>{}</pre></details><p class=\"source-path\">{}</p></aside></div>", escape(&fields), escape(scroll.path.as_deref().unwrap_or(""))).unwrap();
    Some(page(view, &scroll.title, "/scrolls", &body))
}

pub fn not_found(view: &View) -> String {
    page(
        view,
        "Not found",
        "",
        &heading(
            "404",
            "This isn’t in the current read.",
            "It may have been removed or renamed, or its store isn’t being shown.",
        ),
    )
}

// ---------------------------------------------------------------- markdown

/// Turns a link in a document into a safe destination: external web links
/// pass through, references to a bead or scroll on show become local links,
/// and everything else goes nowhere.
pub struct Links<'a> {
    beads: Option<&'a beads::Snapshot>,
    scrolls: Option<&'a scrolls::Snapshot>,
}

impl<'a> Links<'a> {
    pub fn new(view: &'a View) -> Self {
        Self {
            beads: view.beads.as_ref().map(|f| &*f.data),
            scrolls: view.scrolls.as_ref().map(|f| &*f.data),
        }
    }

    fn bead(&self, key: &str) -> Option<String> {
        let beads = self.beads?;
        let bead = beads.find(key).or_else(|| {
            // A file named after an external reference, e.g. TICKET-12-fix-login.md.
            beads.beads.iter().find(|b| {
                !b.external_ref.is_empty() && key.starts_with(&format!("{}-", b.external_ref))
            })
        })?;
        Some(format!("/bead/{}", url(&bead.id)))
    }

    pub fn resolve(&self, base: Option<&str>, raw: &str) -> String {
        let lower = raw.trim().to_ascii_lowercase();
        if lower.starts_with("https://")
            || lower.starts_with("http://")
            || lower.starts_with("mailto:")
            || raw.starts_with('#')
        {
            return raw.to_string();
        }
        if raw.starts_with("beads://") || (raw.contains(':') && !raw.contains('/')) {
            return scrolls::bead_reference(raw)
                .and_then(|id| self.bead(id))
                .unwrap_or_else(|| "#".into());
        }
        if raw.contains(':') || raw.starts_with('/') || raw.contains('\\') {
            return "#".into();
        }
        let (path, anchor) = raw.split_once('#').unwrap_or((raw, ""));
        let anchor = if anchor.is_empty() {
            String::new()
        } else {
            format!("#{}", url(anchor))
        };
        let decoded = percent_encoding::percent_decode_str(path)
            .decode_utf8_lossy()
            .into_owned();
        if let Some(scrolls) = self.scrolls {
            if let Some(joined) = base.and_then(|b| join(b, &decoded)) {
                if let Some(target) = scrolls.by_path(&joined) {
                    return format!("/scroll/{}{anchor}", url(&target.id));
                }
            }
            let stripped = decoded.trim_start_matches("./").trim_start_matches("../");
            let stripped = stripped
                .split('/')
                .skip_while(|p| *p == ".." || *p == ".")
                .collect::<Vec<_>>()
                .join("/");
            if let Some(target) = scrolls.by_path_suffix(&stripped) {
                return format!("/scroll/{}{anchor}", url(&target.id));
            }
        }
        let stem = decoded
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_end_matches(".md");
        if !stem.is_empty() {
            if let Some(local) = self.bead(stem) {
                return local;
            }
        }
        "#".into()
    }
}

/// `relative` resolved against the directory of `base`, or `None` if it
/// climbs above the root.
fn join(base: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for part in relative.split('/') {
        match part {
            ".." => {
                parts.pop()?;
            }
            "." | "" => {}
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

pub fn markdown(text: &str, base: Option<&str>, links: &Links) -> String {
    let mut parsed: Vec<_> = Parser::new_ext(
        text,
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
    )
    .collect();
    let mut heading_counts: HashMap<String, usize> = HashMap::new();
    for i in 0..parsed.len() {
        if matches!(parsed[i], Event::Start(Tag::Heading { .. })) {
            let text: String = parsed[i + 1..]
                .iter()
                .take_while(|event| !matches!(event, Event::End(TagEnd::Heading(_))))
                .filter_map(|event| match event {
                    Event::Text(t) | Event::Code(t) => Some(t.as_ref()),
                    _ => None,
                })
                .collect();
            let slug: String = text
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || c.is_whitespace())
                .map(|c| if c.is_whitespace() { '-' } else { c })
                .collect();
            let count = heading_counts.entry(slug.clone()).or_insert(0);
            let slug = if *count == 0 {
                slug
            } else {
                format!("{slug}-{count}")
            };
            *count += 1;
            if let Event::Start(Tag::Heading { id, .. }) = &mut parsed[i] {
                *id = Some(slug.into());
            }
        }
    }
    let events = parsed.into_iter().map(|event| match event {
        // Documents are content, never executable HTML. Images become links so
        // a document cannot start background requests to third parties.
        Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        })
        | Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: links.resolve(base, &dest_url).into(),
            title,
            id,
        }),
        Event::End(TagEnd::Image) => Event::End(TagEnd::Link),
        other => other,
    });
    let mut output = String::new();
    html::push_html(&mut output, events);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Feed;
    use std::sync::Arc;

    fn view() -> View {
        let mut bead = Bead {
            id: "fx-1".into(),
            title: "Fix login".into(),
            status: "open".into(),
            external_ref: "TICKET-12".into(),
            ..Default::default()
        };
        bead.created_at = Some("2026-10-01T00:00:00Z".into());
        let beads = beads::Snapshot::new(
            vec![bead],
            vec!["fx-1".into()],
            Some("abcdef0123456789".into()),
            chrono::Utc::now(),
        );
        let records = vec![
            Scroll {
                id: "sc-a".into(),
                kind: "decision".into(),
                title: "Use Dolt".into(),
                path: Some("decisions/a.md".into()),
                ..Default::default()
            },
            Scroll {
                id: "sc-b".into(),
                kind: "note".into(),
                title: "Notes".into(),
                path: Some("notes/b.md".into()),
                ..Default::default()
            },
        ];
        let edges = vec![scrolls::Edge {
            from: "sc-a".into(),
            rel: "related_work".into(),
            to: "beads://store/fx-1".into(),
            why: None,
        }];
        View {
            title: "demo".into(),
            beads: Some(Feed {
                data: Arc::new(beads),
                error: None,
            }),
            scrolls: Some(Feed {
                data: Arc::new(scrolls::Snapshot::new(records, edges, Some("rev".into()))),
                error: None,
            }),
            revision: 3,
            scrolls_checked_at: None,
        }
    }

    #[test]
    fn markdown_never_executes_html_or_follows_unsafe_links() {
        let view = view();
        let links = Links::new(&view);
        let html = markdown(
            "<script>alert(1)</script>\n\n[x](javascript:alert) ![remote](https://example.org/i.png) [secret](../../../.secret) [abs](/etc/passwd)",
            None,
            &links,
        );
        assert!(!html.contains("<script>"));
        assert!(!html.contains("href=\"javascript:"));
        assert!(!html.contains("<img"));
        assert!(!html.contains(".secret"));
        assert!(!html.contains("/etc/passwd"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("href=\"https://example.org/i.png\""));
    }

    #[test]
    fn links_reach_scrolls_and_beads_across_stores() {
        let view = view();
        let links = Links::new(&view);
        assert_eq!(
            links.resolve(Some("decisions/a.md"), "../notes/b.md#top"),
            "/scroll/sc%2Db#top"
        );
        assert_eq!(links.resolve(None, "../../notes/b.md"), "/scroll/sc%2Db");
        assert_eq!(
            links.resolve(None, "../items/TICKET-12-fix-login.md"),
            "/bead/fx%2D1"
        );
        assert_eq!(links.resolve(None, "fx-1.md"), "/bead/fx%2D1");
        assert_eq!(links.resolve(None, "beads://store/fx-1"), "/bead/fx%2D1");
        assert_eq!(links.resolve(None, "work:fx-1"), "/bead/fx%2D1");
        assert_eq!(links.resolve(None, "missing.md"), "#");
        assert_eq!(links.resolve(None, "ftp://example.org/x"), "#");
    }

    #[test]
    fn headings_get_distinct_anchors() {
        let view = View::default();
        let html = markdown(
            "## Acceptance\n\nOne\n\n## Acceptance\n\nTwo",
            None,
            &Links::new(&view),
        );
        assert!(html.contains("id=\"acceptance\""));
        assert!(html.contains("id=\"acceptance-1\""));
    }

    #[test]
    fn pages_show_both_stores_and_cross_links() {
        let view = view();
        let page = overview(&view);
        assert!(page.contains("Fabriscope <span class=\"muted\">/ demo</span>"));
        assert!(page.contains("Beads · commit abcdef012"));
        let mut served = view.clone();
        let mut data = (*served.beads.as_ref().unwrap().data).clone();
        data.source = "bd serve http://127.0.0.1:9".into();
        served.beads = Some(Feed {
            data: Arc::new(data),
            error: None,
        });
        assert!(
            overview(&served).contains("Beads via bd serve http://127.0.0.1:9 · commit abcdef012")
        );
        assert!(page.contains("Scrolls · revision rev"));
        assert!(page.contains("/bead/fx%2D1"));
        let bead = bead(&view, "fx-1").unwrap();
        assert!(bead.contains("Scrolls about this bead"));
        assert!(bead.contains("/scroll/sc%2Da"));
        let scroll = scroll(&view, "sc-a").unwrap();
        assert!(scroll.contains("bead fx-1"));
        assert!(bead_page_absent(&view));
    }

    fn bead_page_absent(view: &View) -> bool {
        bead(view, "nope").is_none() && scroll(view, "nope").is_none()
    }

    #[test]
    fn a_beads_only_view_hides_scroll_navigation() {
        let mut view = view();
        view.scrolls = None;
        let page = overview(&view);
        assert!(!page.contains("href=\"/scrolls\""));
        assert!(page.contains("href=\"/epics\""));
        assert!(scrolls(&view, &ScrollFilter::default()).contains("404"));
    }

    #[test]
    fn forecasts_distinguish_growth_zero_and_burn_down() {
        assert_eq!(forecast_values(100, -10.0).0, [30, 0, 0]);
        assert_eq!(forecast_values(100, 10.0).0, [170, 240, 380]);
        assert_eq!(forecast_values(100, 0.0).1, "No clearance at this rate");
        assert_eq!(forecast_values(0, -10.0).1, "Already empty");
    }
}
