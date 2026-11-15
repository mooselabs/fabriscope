// Owned by WP1 — Lists, groups, search, overview. Frozen signature:
// `overview(&View) -> String`.
//! The overview page: headline counts, work under way (in progress and
//! held), a strip for the profile's first group, delivery and forecast,
//! what is ready next, and knowledge. Every count is of items: receipt and
//! hidden rows are never counted.

use super::layout::page;
use super::progress::compact;
use super::scrolls::scroll_table;
use super::svg::chart;
use super::table::bead_table;
use super::{bead_link, escape, fmt::words, heading, metrics, tone_badge, url};
use crate::beads::{self, Bead};
use crate::filters::{Counts, group_bead, holding};
use crate::model::View;
use crate::profile::StateRule;
use crate::scrolls::{self, Scroll};
use std::fmt::Write;

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

fn velocity(s: &beads::Snapshot, open: usize) -> String {
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
        for (label, change) in scenarios {
            let (values, date) = forecast_values(open, change);
            write!(out, "<tr><td>{label}</td><td>{change:+.1}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>", values[0], values[1], values[2], escape(&date)).unwrap();
        }
        out.push_str("</tbody></table></div>");
    }
    out.push_str("<p class=\"method\">Complete UTC days, from each bead’s created and closed timestamps. A bead that is open now counts as open since it was created, even if it was closed and reopened in between; deleted beads are not counted. Counts measure beads, not equal-sized units of effort.</p></section>");
    out
}

pub fn overview(view: &View) -> String {
    let mut body = heading(
        "Fabriscope",
        "What’s being made, and what’s known.",
        "Live work from Beads and knowledge from Scrolls, in one read-only view.",
    );
    if let Some(feed) = &view.beads {
        let s = &feed.data;
        let hold = holding(&view.profile);
        let counts = Counts::of(s, 0..s.beads.len(), &hold, |_| true);
        let mut cards = vec![
            (
                "Open beads",
                counts.open,
                "everything not closed",
                "/beads".to_string(),
            ),
            (
                "Ready",
                counts.ready,
                "as bd ready decides",
                "/beads?ready=yes".into(),
            ),
            (
                "In progress",
                counts.working,
                "being worked on",
                "/beads?state=in_progress".into(),
            ),
            (
                "Waiting",
                counts.waiting,
                "open, but not ready",
                "/beads?state=open&ready=no".into(),
            ),
        ];
        for (state, n) in hold.iter().zip(&counts.held) {
            cards.push((
                state.label.as_str(),
                *n,
                "held back from ready",
                format!("/beads?dstate={}", url(&state.key)),
            ));
        }
        body.push_str(&metrics(&cards));
        body.push_str(&under_way(view, s, &hold));
        body.push_str(&group_strip(view, s, &hold));
        body.push_str(&velocity(s, counts.open));
        body.push_str(&ready_next(s));
    }
    if let Some(feed) = &view.scrolls {
        body.push_str(&knowledge(&feed.data));
    }
    page(view, "Overview", "/", &body)
}

/// Values of the profile's first group, most open first, with how each
/// stands; at most `STRIP` values.
fn group_strip(view: &View, s: &beads::Snapshot, hold: &[&StateRule]) -> String {
    const STRIP: usize = 12;
    let Some(group) = view.profile.groups.first() else {
        return String::new();
    };
    let Some(values) = s.derived.groups.get(&group.key) else {
        return String::new();
    };
    let mut rows: Vec<(&String, Counts)> = values
        .iter()
        .map(|(value, members)| {
            (
                value,
                Counts::of(s, members.iter().copied(), hold, |_| true),
            )
        })
        .collect();
    rows.sort_by(|a, b| b.1.open.cmp(&a.1.open).then(a.0.cmp(b.0)));
    let path = format!("/by/{}", url(&group.key));
    let mut out = format!(
        "<section id=\"by-group\"><div class=\"section-heading\"><div><h2>By {}</h2><p class=\"muted\">{} of {} values, most open first.</p></div><a href=\"{}\">Every {} →</a></div><div class=\"table-wrap\"><table class=\"data group-strip\"><thead><tr><th>{}</th><th class=\"num\">Open</th><th class=\"num\">Ready</th><th class=\"num\">Working</th>",
        escape(&group.label.to_lowercase()),
        rows.len().min(STRIP),
        rows.len(),
        escape(&path),
        escape(&group.label.to_lowercase()),
        escape(&group.label)
    );
    for state in hold {
        write!(out, "<th class=\"num\">{}</th>", escape(&state.label)).unwrap();
    }
    out.push_str("<th class=\"num\">Closed</th></tr></thead><tbody>");
    for (value, c) in rows.iter().take(STRIP) {
        let title = group_bead(s, group, value)
            .map(|b| format!("<small>{}</small>", escape(&b.title)))
            .unwrap_or_default();
        write!(
            out,
            "<tr><td>{}{title}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>",
            super::group_link(&group.key, value, value),
            c.open,
            c.ready,
            c.working
        )
        .unwrap();
        for n in &c.held {
            write!(out, "<td class=\"num\">{n}</td>").unwrap();
        }
        write!(out, "<td class=\"num\">{}</td></tr>", c.closed).unwrap();
    }
    out.push_str("</tbody></table></div></section>");
    out
}

fn ready_next(s: &beads::Snapshot) -> String {
    let selected: Vec<&Bead> = s.ready.iter().filter_map(|id| s.get(id)).take(50).collect();
    let mut out = format!(
        "<section id=\"next\"><div class=\"section-heading\"><div><h2>Ready next</h2><p class=\"muted\">The first {} of {} ready beads, in the order bd ready returns them, grouped by parent.</p></div><a href=\"/beads?ready=yes\">Browse every ready bead →</a></div>",
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
        write!(out, "<details class=\"theme\" id=\"group-{}\" open><summary><span>{}</span><span class=\"muted\">{} ready</span></summary><div class=\"theme-body\">{open}{}</div></details>", escape(parent), escape(title), rows.len(), bead_table(s, None, &rows)).unwrap();
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

/// Work under way: beads in progress with where each is up to, then held
/// beads (in a state that overrides ready) marked as stopped, with the
/// state's banner line. A bead in progress whose log is read and empty (an
/// epic, say) has nothing to show, so it is left out; without a local `bd`
/// only held beads show.
fn under_way(view: &View, s: &beads::Snapshot, hold: &[&StateRule]) -> String {
    const ROWS: usize = 50;
    let trails = view.provenance.as_ref();
    let now = chrono::Utc::now();
    let mut working: Vec<(&Bead, String)> = Vec::new();
    let mut held: Vec<(&Bead, String)> = Vec::new();
    for &i in &s.order {
        if !s.derived.is_item(i) {
            continue;
        }
        let bead = &s.beads[i];
        if bead.status == "in_progress" {
            let Some(trails) = trails else { continue };
            if working.len() >= ROWS {
                continue;
            }
            let stage = match compact(trails, bead, now) {
                Some((bar, line)) => format!("{bar}<small>{line}</small>"),
                None if trails.get(&bead.id).is_some() => continue,
                None => "<small>Reading the provenance log…</small>".to_string(),
            };
            working.push((bead, stage));
            continue;
        }
        let Some(state) = s.derived.rows[i]
            .states
            .iter()
            .find_map(|k| hold.iter().find(|h| &h.key == k))
        else {
            continue;
        };
        if held.len() >= ROWS {
            continue;
        }
        let mut stage = tone_badge(&state.label, state.tone);
        if let Some((bar, line)) = trails.and_then(|t| compact(t, bead, now)) {
            write!(stage, "{bar}<small>{line}</small>").unwrap();
        }
        if let Some(line) = view.profile.banner(&state.key, bead) {
            write!(
                stage,
                "<small class=\"stop-line\">{}</small>",
                escape(&line)
            )
            .unwrap();
        }
        held.push((bead, stage));
    }
    if working.is_empty() && held.is_empty() {
        return String::new();
    }
    let held_words = if held.is_empty() {
        String::new()
    } else {
        format!(" and {} held", held.len())
    };
    let mut out = format!(
        "<section id=\"in-progress\"><div class=\"section-heading\"><div><h2>Under way</h2><p class=\"muted\">Where {} beads in progress{held_words} are up to, from their provenance logs.</p></div><a href=\"/beads?state=in_progress\">Browse every bead in progress →</a></div><div class=\"table-wrap\"><table class=\"items progress-list\"><thead><tr><th>Bead</th><th>Stage</th></tr></thead><tbody>",
        working.len()
    );
    for (rows, class) in [(working, ""), (held, " class=\"stopped\"")] {
        for (bead, stage) in rows {
            write!(
                out,
                "<tr{class}><td>{}<small>{}</small></td><td>{stage}</td></tr>",
                bead_link(&bead.id, &bead.title),
                escape(&bead.id)
            )
            .unwrap();
        }
    }
    out.push_str("</tbody></table></div></section>");
    out
}

#[cfg(test)]
mod tests {
    use super::{forecast_values, overview};
    use crate::model::Feed;
    use crate::query::Facets;
    use crate::render::beads;
    use crate::render::tests::{view, with_trail};
    use std::sync::Arc;

    #[test]
    fn the_overview_lists_beads_in_progress_with_a_compact_bar() {
        let mut v = view();
        let mut data = (*v.beads.as_ref().unwrap().data).clone();
        data.beads[0].status = "in_progress".into();
        v.beads = Some(Feed {
            data: Arc::new(data),
            error: None,
        });
        assert!(
            !overview(&v).contains("id=\"in-progress\""),
            "needs a local bd"
        );
        let v = with_trail(
            v,
            r#"[{"id":"1","kind":"claim","occurred_at":"2026-11-15T03:53:05Z"}]"#,
        );
        let page = overview(&v);
        assert!(page.contains("id=\"in-progress\""), "{page}");
        assert!(page.contains("<ol class=\"stages compact\">"));
        assert!(page.contains("Started is next"));
        let mut empty = v.clone();
        empty
            .provenance
            .as_mut()
            .unwrap()
            .record("fx-1", Ok(vec![]), None, chrono::Utc::now());
        assert!(
            !overview(&empty).contains("id=\"in-progress\""),
            "a bead with an empty log is left out"
        );
        assert!(beads(&v, &Facets::default()).contains("class=\"stage-mini\""));
    }

    #[test]
    fn the_profiled_overview_counts_items_and_shows_held_work_under_way() {
        let view = crate::testing::work_view();
        let page = overview(&view);
        // 6 open items; receipts (wk-op-2 is open-ish data) and hidden rows
        // (wk-slot is open) are not counted.
        assert!(
            page.contains("<span>Open beads</span><strong>6</strong>"),
            "{page}"
        );
        assert!(page.contains(
            "<a class=\"metric\" href=\"/beads?dstate=held\"><span>Held</span><strong>1</strong>"
        ));
        assert!(page.contains("<h2>Under way</h2>"));
        let held = page
            .split("<tr class=\"stopped\">")
            .nth(1)
            .expect("a held row");
        assert!(
            held.contains("WORK-0003") || held.contains("wk-3"),
            "{held}"
        );
        assert!(held.contains("Held since 2026-11-15T09:15:00Z: scope-exceeded (scope)"));
        assert!(page.contains("href=\"/beads?ready=yes\">Browse every ready bead"));
        assert!(!page.contains("state=ready"));
        assert!(page.contains("<h2>By milestone</h2>"));
        assert!(page.contains("href=\"/by/milestone/v1\""));
        assert!(!page.contains(" style="));
    }

    #[test]
    fn forecasts_distinguish_growth_zero_and_burn_down() {
        assert_eq!(forecast_values(100, -10.0).0, [30, 0, 0]);
        assert_eq!(forecast_values(100, 10.0).0, [170, 240, 380]);
        assert_eq!(forecast_values(100, 0.0).1, "No clearance at this rate");
        assert_eq!(forecast_values(0, -10.0).1, "Already empty");
    }
}
