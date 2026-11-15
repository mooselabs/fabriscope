// Owned by WP6 — Held, costs, receipts, sweeper. Frozen signature:
// `costs(&View, &Facets) -> String`.
//! `/costs`: spend by attempt, run, group and model, from provenance usage
//! and the run feed (see `crate::costs`). Also the small helpers the WP6
//! pages (`held`, `receipts`) share: tiles and the date-range controls.

use super::fmt::{count, money, seconds, tokens};
use super::layout::page;
use super::svg::{Bar, hbars};
use super::{PAGE_SIZE, bead_link, escape, heading, maybe, pagination, run_link, select, unknown};
use crate::costs::{self, Figure, ItemRow, Report, Share};
use crate::model::View;
use crate::profile::Tone;
use crate::query::{Facets, href};
use chrono::Utc;
use std::fmt::Write;

/// Days a factory page measures when the query names no range.
pub(super) const DEFAULT_DAYS: u32 = 30;

/// Days, from and to controls for a filter form (labelled inputs).
pub(super) fn range_controls(facets: &Facets) -> String {
    let days: Vec<(String, String)> = [
        ("", "Choose dates"),
        ("7", "Last 7 days"),
        ("30", "Last 30 days"),
        ("90", "Last 90 days"),
        ("365", "Last year"),
    ]
    .iter()
    .map(|(v, t)| (v.to_string(), t.to_string()))
    .collect();
    format!(
        "{}<label>From<input type=\"date\" name=\"from\" value=\"{}\"></label><label>To<input type=\"date\" name=\"to\" value=\"{}\"></label>",
        select("days", "Range", &facets.days, &days),
        escape(&facets.from),
        escape(&facets.to)
    )
}

/// Hidden inputs carrying every facet except `skip`, so a second form on a
/// page keeps the filters the first one set.
pub(super) fn hidden(facets: &Facets, skip: &[&str]) -> String {
    facets
        .pairs()
        .into_iter()
        .filter(|(k, _)| !skip.contains(&k.as_str()))
        .map(|(k, v)| {
            format!(
                "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
                escape(&k),
                escape(&v)
            )
        })
        .collect()
}

/// A row of tiles: (label, value HTML, detail).
pub(super) fn tiles(cards: &[(&str, String, String)]) -> String {
    let mut out = "<div class=\"metrics tiles\">".to_string();
    for (label, value, detail) in cards {
        out.push_str(&super::card(label, value, detail, None));
    }
    out.push_str("</div>");
    out
}

/// Whether any facet that picks items (not dates, sort or page) is set.
pub(super) fn picks_items(facets: &Facets) -> bool {
    !facets.person.is_empty() || facets.selects_items()
}

fn dollars(usd: Option<f64>, partial: bool) -> String {
    match usd {
        Some(usd) if partial => format!("≥ {}", escape(&money(usd))),
        Some(usd) => escape(&money(usd)),
        None => unknown().into(),
    }
}

/// `/costs`.
pub(crate) fn costs(view: &View, facets: &Facets) -> String {
    let now = Utc::now();
    let (range, default_range) = facets.range_or(DEFAULT_DAYS);
    // Costs are mostly of closed work: no state filter means every state.
    let items = facets.any_status();
    let picking = picks_items(facets);
    let keep = |item: Option<usize>| match item {
        Some(i) => items.matches(view, i),
        None => !picking,
    };
    let prov = costs::provenance_attempts(view);
    let feed = costs::feed_runs(view, &prov, range, now);
    let feed_count = feed.len();
    let combined = costs::combine(prov, feed);
    let closed: Vec<usize> = view
        .snapshot()
        .map(|s| {
            (0..s.beads.len())
                .filter(|&i| {
                    let bead = &s.beads[i];
                    bead.is_closed()
                        && bead
                            .closed_at
                            .as_deref()
                            .is_some_and(|at| range.contains(at, now))
                        && items.matches(view, i)
                })
                .collect()
        })
        .unwrap_or_default();
    let report = costs::report(&combined, keep, range, now, &closed);
    let mut body = heading(
        "Factory",
        "Costs",
        "What the work cost, from the run feed and from provenance usage. When both record an attempt, the run feed's figure counts and the attempt is counted once.",
    );
    body.push_str(&filter_form(view, facets));
    let when = super::range_words(&range, now);
    write!(
        body,
        "<p class=\"muted\">Spend recorded in {}{}.</p>",
        escape(&when),
        if default_range {
            " (the default; choose a range above)"
        } else {
            ""
        }
    )
    .unwrap();
    body.push_str(&sources(view, feed_count));
    write!(
        body,
        "<p class=\"coverage\" id=\"coverage\">Cost known for <strong>{}</strong> of <strong>{}</strong> items closed in range ({} attempts).</p>",
        report.coverage.known,
        report.coverage.closed,
        report.coverage.attempts
    )
    .unwrap();
    body.push_str(&totals(&report));
    if report.figures.is_empty() {
        body.push_str("<p class=\"empty\">No cost is recorded for this range and these filters. That means nothing was recorded, not that the work was free.</p>");
        return page(view, "Costs", "/costs", &body);
    }
    body.push_str(&breakdowns(view, &report));
    body.push_str(&item_table(view, facets, &report));
    page(view, "Costs", "/costs", &body)
}

fn filter_form(view: &View, facets: &Facets) -> String {
    let mut out = format!(
        "<form class=\"filters\" action=\"/costs\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Title, id or label…\"></label>",
        escape(&facets.q)
    );
    let any = |label: &str| vec![(String::new(), label.to_string())];
    if let Some(s) = view.snapshot() {
        if !view.profile.kinds.is_empty() {
            let mut kinds = any("All kinds");
            kinds.extend(
                view.profile
                    .kinds
                    .iter()
                    .map(|k| (k.key.clone(), k.label.clone())),
            );
            out.push_str(&select("kind", "Kind", &facets.kind, &kinds));
        }
        let mut epics = any("Any epic");
        epics.extend(
            s.ordered()
                .filter(|b| b.is_epic())
                .map(|b| (b.id.clone(), b.title.clone())),
        );
        if epics.len() > 1 {
            out.push_str(&select("epic", "Epic", &facets.epic, &epics));
        }
        if !s.derived.people.is_empty() {
            let mut people = any("Anyone");
            people.extend(s.derived.people.keys().map(|p| (p.clone(), p.clone())));
            out.push_str(&select("person", "Person", &facets.person, &people));
        }
        for group in &view.profile.groups {
            let mut values = any(&format!("Any {}", group.label.to_lowercase()));
            if let Some(index) = s.derived.groups.get(&group.key) {
                values.extend(index.keys().map(|v| (v.clone(), v.clone())));
            }
            let current = facets
                .groups
                .get(&group.key)
                .map(String::as_str)
                .unwrap_or("");
            out.push_str(&select(&group.key, &group.label, current, &values));
        }
    }
    out.push_str(&range_controls(facets));
    let mut skip = vec!["q", "kind", "epic", "person", "days", "from", "to"];
    skip.extend(view.profile.groups.iter().map(|g| g.key.as_str()));
    out.push_str(&hidden(facets, &skip));
    out.push_str("<button type=\"submit\">Apply</button></form>");
    out
}

/// Which sources were available, in one paragraph each.
fn sources(view: &View, runs: usize) -> String {
    let prov = match (&view.provenance, view.snapshot()) {
        (Some(store), Some(_)) => format!(
            "<li><strong>Provenance</strong> (<code>used</code> events per attempt): logs read for {} beads{}.</li>",
            count(store.trails.len() as u64),
            if store.reading.is_empty() {
                String::new()
            } else {
                format!(", {} being read", store.reading.len())
            }
        ),
        _ => "<li><strong>Provenance</strong>: not available (no local <code>bd</code> reads provenance logs here).</li>".into(),
    };
    let feed = match view.run_index() {
        Some(_) => format!(
            "<li><strong>Run feed</strong> (cost per run; per model and role once a run's details are read): {} runs listed.</li>",
            count(runs as u64)
        ),
        None => {
            "<li><strong>Run feed</strong>: not configured (start with <code>--runs</code>).</li>"
                .into()
        }
    };
    format!("<ul class=\"cost-sources\">{prov}{feed}</ul>")
}

fn totals(r: &Report) -> String {
    let attempts = |n: usize| format!("{n} attempt{}", if n == 1 { "" } else { "s" });
    let mut cards = vec![
        (
            "Total, counted once",
            dollars(r.combined.spend.dollars, r.combined.partial),
            format!(
                "{} across {} items",
                attempts(r.combined.attempts),
                r.items.len()
            ),
        ),
        (
            "From the run feed",
            dollars(r.feed.spend.dollars, r.feed.partial),
            format!("{} runs · {}", r.feed.figures, attempts(r.feed.attempts)),
        ),
        (
            "From provenance only",
            dollars(r.provenance.spend.dollars, false),
            format!(
                "{} the feed does not price",
                attempts(r.provenance.attempts)
            ),
        ),
        (
            "Provenance the feed replaced",
            dollars(r.replaced.spend.dollars, false),
            format!("{}, not counted again", attempts(r.replaced.attempts)),
        ),
    ];
    let mut out = tiles(&cards);
    cards.clear();
    let s = &r.combined.spend;
    cards.push(("Tokens", maybe(s.tokens, tokens), "where recorded".into()));
    cards.push((
        "Wall time",
        maybe(s.wall_seconds, seconds),
        "where recorded".into(),
    ));
    cards.push((
        "Gate time",
        maybe(s.gate_seconds, seconds),
        match s.gate_runs {
            Some(n) => format!("{} gate runs", count(n)),
            None => "gate runs unknown".into(),
        },
    ));
    cards.push((
        "Gate time saved",
        maybe(s.gate_saved_seconds, seconds),
        match s.gate_reused {
            Some(n) => format!("{} results reused", count(n)),
            None => "reuse unknown".into(),
        },
    ));
    out.push_str(&tiles(&cards));
    if r.combined.partial {
        out.push_str("<p class=\"muted\">≥ marks a lower bound: the run feed says some usage is missing from a run it priced.</p>");
    }
    if r.unknown_runs > 0 {
        write!(
            out,
            "<p class=\"muted\">{} runs in range report no cost; they are left out, not counted as $0.</p>",
            r.unknown_runs
        )
        .unwrap();
    }
    out
}

fn share_section(id: &str, title: &str, shares: &[Share], note: &str) -> String {
    if shares.is_empty() {
        return String::new();
    }
    let bars: Vec<Bar> = shares
        .iter()
        .filter_map(|s| {
            Some(Bar {
                label: s.key.clone(),
                value: s.dollars?,
                title: format!("{}: {}", s.key, money(s.dollars?)),
                tone: Tone::Neutral,
            })
        })
        .take(12)
        .collect();
    let mut out = format!(
        "<details class=\"cost-share\" id=\"{}\" open><summary>{}</summary>",
        escape(id),
        escape(title)
    );
    if !note.is_empty() {
        write!(out, "<p class=\"muted\">{}</p>", escape(note)).unwrap();
    }
    out.push_str(&hbars(title, &bars, money));
    out.push_str("<div class=\"table-wrap\"><table class=\"data\"><thead><tr><th></th><th class=\"num\">Dollars</th><th class=\"num\">Items</th><th class=\"num\">Attempts</th></tr></thead><tbody>");
    for s in shares {
        write!(
            out,
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            escape(&s.key),
            dollars(s.dollars, false),
            s.items,
            s.attempts
        )
        .unwrap();
    }
    out.push_str("</tbody></table></div></details>");
    out
}

const NOT_IN_LEDGER: &str = "(not in the ledger)";

fn breakdowns(view: &View, r: &Report) -> String {
    let s = view.snapshot();
    let bead = |f: &Figure| s.zip(f.item).map(|(s, i)| &s.beads[i]);
    let mut out = "<section id=\"cost-breakdowns\"><h2>Where it went</h2><p class=\"muted\">An item in several groups counts in each, so a breakdown can add up to more than the total.</p>".to_string();
    out.push_str(&share_section(
        "cost-by-model",
        "By model",
        &costs::split(&r.figures, false),
        "Run feed figures split by model once the run's details are read; provenance by the model each usage event names. What neither names is not attributed.",
    ));
    if r.figures.iter().any(|f| !f.by_role.is_empty()) {
        out.push_str(&share_section(
            "cost-by-role",
            "By role",
            &costs::split(&r.figures, true),
            "From run details only.",
        ));
    }
    for group in &view.profile.groups {
        let shares = costs::breakdown(&r.figures, |f| match (s, f.item) {
            (Some(s), Some(i)) => s.derived.rows[i]
                .groups
                .get(&group.key)
                .cloned()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| vec![format!("(no {})", group.label.to_lowercase())]),
            _ => vec![NOT_IN_LEDGER.into()],
        });
        out.push_str(&share_section(
            &format!("cost-by-{}", super::url(&group.key)),
            &format!("By {}", group.label.to_lowercase()),
            &shares,
            "",
        ));
    }
    if view.profile.people.roles.is_empty() {
        let shares = costs::breakdown(&r.figures, |f| {
            vec![match bead(f) {
                Some(b) if !b.assignee.is_empty() => view.profile.person(&b.assignee).name,
                Some(_) => "(unassigned)".into(),
                None => NOT_IN_LEDGER.into(),
            }]
        });
        out.push_str(&share_section(
            "cost-by-assignee",
            "By assignee",
            &shares,
            "",
        ));
    }
    for role in &view.profile.people.roles {
        let shares = costs::breakdown(&r.figures, |f| {
            vec![match bead(f) {
                Some(b) => view
                    .person_in_role(&b.id, &role.key)
                    .unwrap_or_else(|| "(nobody)".into()),
                None => NOT_IN_LEDGER.into(),
            }]
        });
        out.push_str(&share_section(
            &format!("cost-by-{}", super::url(&role.key)),
            &format!("By person: {}", role.label.to_lowercase()),
            &shares,
            "",
        ));
    }
    let shares = costs::breakdown(&r.figures, |f| {
        vec![match bead(f) {
            Some(b) => match b.parent().and_then(|p| s.and_then(|s| s.find(p))) {
                Some(epic) => epic.title.clone(),
                None => "(no epic)".into(),
            },
            None => NOT_IN_LEDGER.into(),
        }]
    });
    out.push_str(&share_section("cost-by-epic", "By epic", &shares, ""));
    out.push_str("</section>");
    out
}

/// The sort keys the item table offers, with their headings.
const SORTS: [(&str, &str); 6] = [
    ("item", "Item"),
    ("attempts", "Attempts"),
    ("dollars", "Dollars"),
    ("tokens", "Tokens"),
    ("wall", "Wall"),
    ("gate", "Gate time"),
];

fn sort_value(row: &ItemRow, key: &str) -> Option<f64> {
    let s = &row.total.spend;
    match key {
        "attempts" => Some(row.total.attempts as f64),
        "tokens" => s.tokens.map(|t| t as f64),
        "wall" => s.wall_seconds,
        "gate" => s.gate_seconds,
        _ => s.dollars,
    }
}

fn item_table(view: &View, facets: &Facets, r: &Report) -> String {
    let s = view.snapshot();
    let sort = if SORTS.iter().any(|(k, _)| *k == facets.sort) {
        facets.sort.as_str()
    } else {
        "dollars"
    };
    let name = |row: &ItemRow| {
        s.zip(row.item)
            .map(|(s, i)| s.beads[i].title.to_lowercase())
            .unwrap_or_default()
    };
    let mut rows: Vec<&ItemRow> = r.items.iter().collect();
    if sort == "item" {
        rows.sort_by_key(|row| name(row));
    } else {
        // Largest first; unknown values last.
        rows.sort_by(|a, b| {
            sort_value(b, sort)
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&sort_value(a, sort).unwrap_or(f64::NEG_INFINITY))
        });
    }
    let (current, pages) = super::page_of(rows.len(), PAGE_SIZE, facets.page());
    let mut out = format!(
        "<section id=\"cost-items\"><h2>By item</h2><p class=\"muted\">{} items. Sorted by {}.</p><div class=\"table-wrap\"><table class=\"data cost-items\"><thead><tr>",
        rows.len(),
        SORTS
            .iter()
            .find(|(k, _)| *k == sort)
            .map(|(_, l)| l.to_lowercase())
            .unwrap_or_default()
    );
    for (key, label) in SORTS {
        let class = if key == "item" { "" } else { " class=\"num\"" };
        let here = if key == sort {
            " aria-sort=\"descending\""
        } else {
            ""
        };
        write!(
            out,
            "<th{class}{here}><a href=\"{}#cost-items\">{label}</a></th>",
            escape(&href("/costs", facets, &[("sort", key)]))
        )
        .unwrap();
    }
    out.push_str("<th>Source</th><th>Runs</th></tr></thead><tbody>");
    for row in rows.iter().skip(current * PAGE_SIZE).take(PAGE_SIZE) {
        let item = match s.zip(row.item) {
            Some((s, i)) => {
                let bead = &s.beads[i];
                let reference = if bead.external_ref.is_empty() {
                    &bead.id
                } else {
                    &bead.external_ref
                };
                format!(
                    "{}<small>{}</small>",
                    bead_link(&bead.id, &bead.title),
                    escape(reference)
                )
            }
            None => format!("<span class=\"muted\">{NOT_IN_LEDGER}</span>"),
        };
        let spend = &row.total.spend;
        let sources: Vec<&str> = row.sources.iter().map(|s| s.label()).collect();
        let runs: Vec<String> = row.runs.iter().map(|run| run_link(run, run)).collect();
        write!(
            out,
            "<tr><td>{item}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td><td>{}</td></tr>",
            row.total.attempts,
            dollars(spend.dollars, row.total.partial),
            maybe(spend.tokens, tokens),
            maybe(spend.wall_seconds, seconds),
            maybe(spend.gate_seconds, seconds),
            escape(&sources.join(" + ")),
            runs.join(", ")
        )
        .unwrap();
    }
    out.push_str("</tbody></table></div>");
    if pages > 1 {
        out.push_str(&pagination("/costs", &facets.pairs(), current, pages));
    }
    out.push_str("</section>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::costs::Source;

    fn facets(query: &str) -> Facets {
        let uri: axum::http::Uri = format!("/costs?{query}").parse().unwrap();
        axum::extract::Query::<Facets>::try_from_uri(&uri)
            .unwrap()
            .0
    }

    #[test]
    fn the_costs_page_counts_wk2_once_and_says_what_it_covers() {
        let view = crate::testing::work_view();
        let page = costs(&view, &facets("from=2026-11-01&to=2026-11-30&q=rounding"));
        assert!(page.contains("Cost known for <strong>1</strong> of <strong>1</strong> items closed in range (3 attempts)"), "{page}");
        assert!(page.contains("$5.37"), "run-0002 counts once");
        assert!(!page.contains("$7.47"), "never feed plus provenance");
        assert!(page.contains(Source::Feed.label()));
        assert!(page.contains("By model"));
        assert!(page.contains("model-large"));
        assert!(page.contains("By person: claimed by"));
        assert!(page.contains("/run/run%2D0002"));
        assert!(!page.contains(" style="));
        // Everything in the fixture, any time, sorted by tokens.
        let all = costs(&view, &facets("from=2026-01-01&to=2026-12-31&sort=tokens"));
        assert!(all.contains("aria-sort=\"descending\"><a href=\"/costs?from=2026%2D01%2D01&amp;to=2026%2D12%2D31&amp;sort=tokens#cost-items\">Tokens"), "{all}");
        assert!(all.contains("By epic"));
    }

    #[test]
    fn without_any_figure_the_page_says_nothing_was_recorded_not_zero() {
        let mut view = crate::testing::work_view();
        view.provenance = None;
        view.runs = None;
        let page = costs(&view, &Facets::default());
        assert!(page.contains("not available"));
        assert!(page.contains("not configured"));
        assert!(page.contains("not that the work was free"));
        assert!(page.contains(unknown()));
        assert!(!page.contains("$0.00"));
    }
}
