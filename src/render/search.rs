// Owned by WP1 — Lists, groups, search, overview. Frozen signature:
// `search(&View, &Facets) -> String`.
//! `/search?q=…`: beads, scrolls, people, group values and runs matching
//! the text, each section bounded and linking to its page. An exact bead id
//! or external ref never reaches this page: the handler redirects to the
//! bead (see `filters::exact`).

use super::layout::page;
use super::table::bead_table;
use super::{escape, group_link, heading, person_link, run_link, scroll_link};
use crate::filters::search_beads;
use crate::model::View;
use crate::query::Facets;
use std::fmt::Write;

/// How many hits each section shows.
const HITS: usize = 50;

fn form(q: &str) -> String {
    format!(
        "<form class=\"filters search-form\" action=\"/search\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Id, title, label, description, person, run…\"></label><button type=\"submit\">Search</button></form>",
        escape(q)
    )
}

/// A titled section listing `items` (already-built markup), or nothing.
fn section(id: &str, title: &str, total: usize, items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let shown = if total > items.len() {
        format!("first {} of {total}", items.len())
    } else {
        total.to_string()
    };
    let mut out = format!(
        "<section id=\"{id}\"><h2>{} <span class=\"muted\">· {shown}</span></h2><ul class=\"hits\">",
        escape(title)
    );
    for item in items {
        write!(out, "<li>{item}</li>").unwrap();
    }
    out.push_str("</ul></section>");
    out
}

/// Up to `HITS` of `found`, and how many there were.
fn bounded(found: Vec<String>) -> (usize, Vec<String>) {
    let total = found.len();
    (total, found.into_iter().take(HITS).collect())
}

/// `/search`.
pub(crate) fn search(view: &View, facets: &Facets) -> String {
    let q = facets.q.trim();
    let mut body = heading(
        "Search",
        "Search",
        "Beads, scrolls, people, groups and runs, by any words they carry.",
    );
    body.push_str(&form(q));
    if q.is_empty() {
        body.push_str("<p class=\"empty\">Type an id, a title, a label, a person or a run to search for. An exact bead id or reference opens that bead.</p>");
        return page(view, "Search", "", &body);
    }
    let needle = q.to_lowercase();
    let has = |hay: &str| hay.to_lowercase().contains(&needle);
    let mut sections = String::new();
    if let Some(s) = view.snapshot() {
        let (found, total) = search_beads(view, q, HITS);
        if !found.is_empty() {
            let rows: Vec<_> = found
                .iter()
                .enumerate()
                .map(|(n, &i)| (n + 1, &s.beads[i]))
                .collect();
            let shown = if total > rows.len() {
                format!("first {} of {total}", rows.len())
            } else {
                total.to_string()
            };
            write!(
                sections,
                "<section id=\"beads\"><h2>Beads <span class=\"muted\">· {shown}</span></h2>{}</section>",
                bead_table(s, view.provenance.as_ref(), &rows)
            )
            .unwrap();
        }
        let (total, people) = bounded(
            s.derived
                .people
                .iter()
                .filter(|(name, _)| has(name))
                .map(|(name, beads)| {
                    format!(
                        "{} <span class=\"muted\">· {} items</span>",
                        person_link(name),
                        beads.len()
                    )
                })
                .collect(),
        );
        sections.push_str(&section("people", "People", total, &people));
        let mut values = Vec::new();
        for rule in &view.profile.groups {
            for (value, members) in s.derived.groups.get(&rule.key).into_iter().flatten() {
                if has(value) {
                    values.push(format!(
                        "{} <span class=\"muted\">· {} items</span>",
                        group_link(&rule.key, value, &format!("{} {value}", rule.label)),
                        members.len()
                    ));
                }
            }
        }
        let (total, values) = bounded(values);
        sections.push_str(&section("groups", "Groups", total, &values));
    }
    if let Some(feed) = &view.scrolls {
        let (total, scrolls) = bounded(
            feed.data
                .scrolls
                .iter()
                .filter(|sc| has(&sc.id) || has(&sc.title) || sc.aliases.iter().any(|a| has(a)))
                .map(|sc| {
                    format!(
                        "{} <span class=\"muted\">· {}</span>",
                        scroll_link(&sc.id, &sc.title),
                        escape(&sc.id)
                    )
                })
                .collect(),
        );
        sections.push_str(&section("scrolls", "Scrolls", total, &scrolls));
    }
    if let Some(index) = view.run_index() {
        let (total, runs) = bounded(
            index
                .runs
                .iter()
                .filter(|r| {
                    has(&r.run)
                        || [&r.subject.reference, &r.subject.title, &r.subject.id]
                            .iter()
                            .any(|v| v.as_deref().is_some_and(has))
                })
                .map(|r| {
                    let subject = r
                        .subject
                        .reference
                        .as_deref()
                        .or(r.subject.id.as_deref())
                        .unwrap_or("");
                    let title = r.subject.title.as_deref().unwrap_or("");
                    format!(
                        "{} <span class=\"muted\">· {} {} · {}</span>",
                        run_link(&r.run, &r.run),
                        escape(subject),
                        escape(title),
                        escape(&r.state)
                    )
                })
                .collect(),
        );
        sections.push_str(&section("runs", "Runs", total, &runs));
    }
    if sections.is_empty() {
        write!(
            body,
            "<p class=\"empty\">Nothing matches “{}”.</p>",
            escape(q)
        )
        .unwrap();
    } else {
        body.push_str(&sections);
    }
    page(view, "Search", "", &body)
}

#[cfg(test)]
mod tests {
    use super::search;
    use crate::query::Facets;
    use crate::testing::work_view;

    fn q(text: &str) -> Facets {
        Facets {
            q: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn search_finds_beads_people_groups_and_runs_and_prompts_when_empty() {
        let view = work_view();
        let empty = search(&view, &Facets::default());
        assert!(empty.contains("Type an id"), "{empty}");
        let page = search(&view, &q("rig"));
        assert!(page.contains("href=\"/by/rig/rigA\""), "{page}");
        assert!(page.contains("href=\"/person/factory%2FhostA%2FrigA\""));
        let runs = search(&view, &q("run-0003"));
        assert!(runs.contains("href=\"/run/run%2D0003\""), "{runs}");
        let beads = search(&view, &q("table"));
        assert!(beads.contains("Move reports to the new table"), "{beads}");
        assert!(!beads.contains("wk-rc-"), "receipts never appear");
        let none = search(&view, &q("<zzz>"));
        assert!(none.contains("Nothing matches “&lt;zzz&gt;”"));
        assert!(!page.contains(" style="));
    }
}
