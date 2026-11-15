// Owned by WP6 — Held, costs, receipts, sweeper. Frozen signature:
// `receipts(&View, &Facets) -> String`.
//! `/receipts`: receipt rows by kind, joined to their subjects and attempts
//! (`Derived::receipts_by_subject`, `receipts_by_attempt`).
//!
//! On this page `kind` is a receipt kind (a `[[receipts]]` rule's `kind`),
//! and any query key a receipt rule names under `group` filters on that
//! value. The date range applies to the receipt's own time.

use super::costs::{range_controls, tiles};
use super::fmt::when;
use super::layout::page;
use super::values::{Ctx, value};
use super::{
    PAGE_SIZE, bead_link, escape, heading, pagination, person_link, run_link, select, tone_badge,
};
use crate::model::View;
use crate::profile::{ReceiptRule, Tone, derive::ReceiptFacts};
use crate::query::{Facets, href};
use chrono::Utc;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// A receipt row: its bead index, its facts and the rule that matched it.
struct Row<'a> {
    index: usize,
    facts: &'a ReceiptFacts,
    rule: &'a ReceiptRule,
    /// The receipt's time: the rule's `at`, else when the row was created.
    at: Option<String>,
}

/// `/receipts`.
pub(crate) fn receipts(view: &View, facets: &Facets) -> String {
    let rules = &view.profile.receipts;
    let mut body = heading(
        "Factory",
        "Receipts",
        "Immutable records the work left behind (gates passed, changes shipped, operations run), each joined to the item and attempt it is about.",
    );
    if rules.is_empty() {
        body.push_str("<p class=\"empty\">The profile defines no receipts, so no row is read as one. A profile names them under <code>[[rows]]</code> with <code>class = \"receipt\"</code> and describes each kind under <code>[[receipts]]</code>; start Fabriscope with <code>--profile</code> to use one.</p>");
        return page(view, "Receipts", "/receipts", &body);
    }
    let Some(s) = view.snapshot() else {
        return page(view, "Receipts", "/receipts", &body);
    };
    let now = Utc::now();
    let range = facets.range();
    // Group keys any receipt rule defines, so other query keys are left
    // alone.
    let group_keys: BTreeSet<&str> = rules
        .iter()
        .flat_map(|r| r.group.keys().map(String::as_str))
        .collect();
    let mut all: Vec<Row> = Vec::new();
    for (index, row) in s.derived.rows.iter().enumerate() {
        let Some(facts) = &row.receipt else { continue };
        let Some(rule) = rules.get(facts.rule) else {
            continue;
        };
        all.push(Row {
            index,
            facts,
            rule,
            at: facts
                .at
                .clone()
                .or_else(|| s.beads[index].created_at.clone()),
        });
    }
    let needle = facets.q.to_lowercase();
    let group_value = |row: &Row, key: &str| {
        row.rule
            .group
            .get(key)
            .and_then(|src| src.text(&s.beads[row.index]))
    };
    // Every filter but the kind, so the kind tiles count what a click shows.
    let passes = |row: &Row| {
        let bead = &s.beads[row.index];
        let subject = row.facts.subject.map(|i| &s.beads[i]);
        let text_ok = needle.is_empty()
            || [
                Some(bead.id.as_str()),
                Some(bead.title.as_str()),
                subject.map(|b| b.id.as_str()),
                subject.map(|b| b.title.as_str()),
                subject.map(|b| b.external_ref.as_str()),
                row.facts.attempt.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|t| t.to_lowercase().contains(&needle));
        text_ok
            && row
                .at
                .as_deref()
                .map_or(range == Default::default(), |at| range.contains(at, now))
            && facets
                .groups
                .iter()
                .filter(|(k, _)| group_keys.contains(k.as_str()))
                .all(|(k, v)| group_value(row, k).as_deref() == Some(v.as_str()))
    };
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    let mut shown: Vec<&Row> = Vec::new();
    for row in &all {
        if !passes(row) {
            continue;
        }
        *by_kind.entry(row.facts.kind.as_str()).or_default() += 1;
        if facets.kind.is_empty() || row.facts.kind == facets.kind {
            shown.push(row);
        }
    }
    let kinds: Vec<&str> = {
        let mut seen = Vec::new();
        for rule in rules {
            if !seen.contains(&rule.kind.as_str()) {
                seen.push(rule.kind.as_str());
            }
        }
        seen
    };
    body.push_str(&filter_form(
        facets,
        &kinds,
        &all,
        &group_keys,
        &group_value,
    ));
    let cards: Vec<(&str, String, String)> = kinds
        .iter()
        .map(|k| {
            (
                *k,
                format!(
                    "<a href=\"{}\">{}</a>",
                    escape(&href("/receipts", facets, &[("kind", k)])),
                    by_kind.get(k).copied().unwrap_or(0)
                ),
                "receipts".into(),
            )
        })
        .collect();
    body.push_str(&tiles(&cards));
    let mut missing: Vec<&str> = kinds
        .iter()
        .copied()
        .filter(|k| !all.iter().any(|r| r.facts.kind == *k))
        .collect();
    missing.dedup();
    for kind in missing {
        write!(
            body,
            "<p class=\"muted\">The profile defines <strong>{}</strong> receipts, but the store has none.</p>",
            escape(kind)
        )
        .unwrap();
    }
    let unread = s
        .derived
        .rows
        .iter()
        .filter(|r| r.class == crate::profile::RowClass::Receipt && r.receipt.is_none())
        .count();
    if unread > 0 {
        write!(body, "<p class=\"muted\">{unread} receipt rows match no <code>[[receipts]]</code> rule and are not shown.</p>").unwrap();
    }
    shown.sort_by(|a, b| b.at.cmp(&a.at).then(b.index.cmp(&a.index)));
    let (current, pages) = super::page_of(shown.len(), PAGE_SIZE, facets.page());
    write!(
        body,
        "<section id=\"receipt-list\"><h2>{} <span class=\"muted\">{}</span></h2>",
        if facets.kind.is_empty() {
            "All receipts".to_string()
        } else {
            format!("{} receipts", escape(&facets.kind))
        },
        shown.len()
    )
    .unwrap();
    if shown.is_empty() {
        body.push_str("<p class=\"empty\">No receipt matches these filters.</p></section>");
        return page(view, "Receipts", "/receipts", &body);
    }
    body.push_str("<div class=\"table-wrap\"><table class=\"data receipts\"><thead><tr><th>Kind</th><th>At</th><th>Actor</th><th>Subject</th><th>Attempt</th><th>Details</th><th>Receipt</th></tr></thead><tbody>");
    for row in shown.iter().skip(current * PAGE_SIZE).take(PAGE_SIZE) {
        body.push_str(&receipt_row(view, row));
    }
    body.push_str("</tbody></table></div>");
    if pages > 1 {
        body.push_str(&pagination("/receipts", &facets.pairs(), current, pages));
    }
    body.push_str("</section>");
    page(view, "Receipts", "/receipts", &body)
}

fn receipt_row(view: &View, row: &Row) -> String {
    let s = view.snapshot().expect("checked by the caller");
    let bead = &s.beads[row.index];
    let dash = || "<span class=\"muted\">—</span>".to_string();
    let at = row.at.as_deref().map(when).unwrap_or_else(dash);
    let actor = row
        .facts
        .actor
        .as_deref()
        .map(|a| person_link(&view.profile.person(a).name))
        .unwrap_or_else(dash);
    let subject = match row.facts.subject.map(|i| &s.beads[i]) {
        Some(subject) => {
            let reference = if subject.external_ref.is_empty() {
                &subject.id
            } else {
                &subject.external_ref
            };
            format!(
                "{}<small>{}</small>",
                bead_link(&subject.id, &subject.title),
                escape(reference)
            )
        }
        None => "<span class=\"muted\">not in the ledger</span>".into(),
    };
    let attempt = match row.facts.attempt.as_deref() {
        Some(a) => match view.run_for_attempt(a) {
            Some(run) => format!(
                "<code>{}</code><small>{}</small>",
                escape(a),
                run_link(&run.run, &run.run)
            ),
            None => format!("<code>{}</code>", escape(a)),
        },
        None => dash(),
    };
    let mut details = "<dl class=\"facts\">".to_string();
    for (key, source) in &row.rule.group {
        if let Some(text) = source.text(bead) {
            write!(
                details,
                "<dt>{}</dt><dd>{}</dd>",
                escape(key),
                escape(&text)
            )
            .unwrap();
        }
    }
    for field in &row.rule.fields {
        let Some(v) = field.path.value(bead) else {
            continue;
        };
        let ctx = Ctx {
            view,
            unit: field.unit.as_deref(),
            repo: field.repo.as_deref(),
            base: None,
        };
        write!(
            details,
            "<dt>{}</dt><dd>{}</dd>",
            escape(&field.label),
            value(field.kind, &v, &ctx)
        )
        .unwrap();
    }
    details.push_str("</dl>");
    format!(
        "<tr><td>{}</td><td>{at}</td><td>{actor}</td><td>{subject}</td><td>{attempt}</td><td>{details}</td><td>{}</td></tr>",
        tone_badge(&row.facts.kind, Tone::Neutral),
        bead_link(&bead.id, &bead.id)
    )
}

fn filter_form(
    facets: &Facets,
    kinds: &[&str],
    all: &[Row],
    group_keys: &BTreeSet<&str>,
    group_value: &dyn Fn(&Row, &str) -> Option<String>,
) -> String {
    let mut out = format!(
        "<form class=\"filters\" action=\"/receipts\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Item, attempt or receipt…\"></label>",
        escape(&facets.q)
    );
    let mut options = vec![(String::new(), "All kinds".to_string())];
    options.extend(kinds.iter().map(|k| (k.to_string(), k.to_string())));
    out.push_str(&select("kind", "Kind", &facets.kind, &options));
    for key in group_keys {
        let values: BTreeSet<String> = all.iter().filter_map(|r| group_value(r, key)).collect();
        let mut options = vec![(String::new(), "Any".to_string())];
        options.extend(values.into_iter().map(|v| (v.clone(), v)));
        let current = facets.groups.get(*key).map(String::as_str).unwrap_or("");
        out.push_str(&select(key, key, current, &options));
    }
    out.push_str(&range_controls(facets));
    out.push_str("<button type=\"submit\">Apply</button></form>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facets(query: &str) -> Facets {
        let uri: axum::http::Uri = format!("/receipts?{query}").parse().unwrap();
        axum::extract::Query::<Facets>::try_from_uri(&uri)
            .unwrap()
            .0
    }

    #[test]
    fn gate_receipts_join_their_subject_and_run() {
        let view = crate::testing::work_view();
        let page = receipts(&view, &facets("kind=gate"));
        assert!(page.contains("gate receipts"), "{page}");
        assert!(page.contains("/bead/wk%2Drc%2Dgate"));
        assert!(
            page.contains("Fix rounding in invoice totals"),
            "subject wk-2"
        );
        assert!(page.contains("<code>att-0002-2</code><small><a href=\"/run/run%2D0002\">"));
        assert!(page.contains("Gate time</dt><dd>6m 52s"));
        assert!(!page.contains("/bead/wk%2Drc%2Dship\""), "kind filter");
        let all = receipts(&view, &Facets::default());
        // Newest first: the operation on wk-1 (15 Nov) before the ship (14 Nov).
        let op2 = all.find("/bead/wk%2Dop%2D2").unwrap();
        let ship = all.find("/bead/wk%2Drc%2Dship").unwrap();
        assert!(op2 < ship);
        let rig = receipts(&view, &facets("rig=rigA&result=landed"));
        assert!(rig.contains("/bead/wk%2Drc%2Dship"));
        assert!(!rig.contains("/bead/wk%2Drc%2Dgate\""));
        assert!(!page.contains(" style="));
    }

    #[test]
    fn without_receipt_rules_the_page_explains() {
        let mut view = crate::testing::work_view();
        view.profile = std::sync::Arc::new(crate::profile::Profile::default());
        assert!(receipts(&view, &Facets::default()).contains("The profile defines no receipts"));
    }
}
