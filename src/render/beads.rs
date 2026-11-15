// Owned by WP1 — Lists, groups, search, overview. Frozen signatures:
// `beads(&View, &Facets) -> String`, `epics(&View, &Facets) -> String`
// (moved from `&BeadFilter` by WP1; web/lists.rs is WP1's too).
//! The bead list and the epics page.
//!
//! Both filter with `query::Facets`, so the query keys they always read
//! (`q`, `state`, `ready`, `type`, `priority`, `page`) keep working, and a
//! profile adds kind, derived state, person, epic and group facets. Receipt
//! and hidden rows never appear. Without a profile both pages render as they
//! did before profiles.

use super::layout::{not_found, page};
use super::table::bead_table;
use super::{
    PAGE_SIZE, badge, bead_link, escape, facets_form, fmt::words, heading, pagination, select,
    status_options, tally,
};
use crate::beads::{Bead, Snapshot};
use crate::filters::profiled;
use crate::model::View;
use crate::query::Facets;
use std::collections::BTreeSet;
use std::fmt::Write;

const APPLY: &str = "<button type=\"submit\">Apply</button></form>";

/// Item beads in display order.
fn items<'a>(s: &'a Snapshot) -> impl Iterator<Item = &'a Bead> + 'a {
    s.order
        .iter()
        .filter(|&&i| s.derived.is_item(i))
        .map(|&i| &s.beads[i])
}

/// The ready select every bead list has offered.
fn ready_select(facets: &Facets) -> String {
    let options = [("", "Any"), ("yes", "Ready"), ("no", "Not ready")]
        .map(|(v, t)| (v.to_string(), t.to_string()));
    select("ready", "Ready", &facets.ready, &options)
}

/// The priority select: every priority an item holds.
fn priority_select(s: &Snapshot, facets: &Facets) -> String {
    let priorities: BTreeSet<i64> = items(s).map(|b| b.priority).collect();
    let mut options = vec![(String::new(), "All priorities".to_string())];
    options.extend(priorities.iter().map(|p| (p.to_string(), format!("P{p}"))));
    select("priority", "Priority", &facets.priority, &options)
}

/// The filter form of `/beads`: without a profile, the form it always had;
/// with one, the shared facets form plus ready and priority.
fn beads_form(view: &View, s: &Snapshot, facets: &Facets) -> String {
    if profiled(&view.profile) {
        let form = facets_form(view, facets, "/beads");
        return match form.strip_suffix(APPLY) {
            Some(head) => format!(
                "{head}{}{}{APPLY}",
                ready_select(facets),
                priority_select(s, facets)
            ),
            None => form,
        };
    }
    let mut types_options = vec![(String::new(), "All types".to_string())];
    types_options.extend(
        tally(s.beads.iter(), |b| b.issue_type.as_str())
            .into_iter()
            .map(|(t, n)| (t.to_string(), format!("{} ({n})", words(t)))),
    );
    let mut body = format!(
        "<form class=\"filters\" action=\"/beads\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Title, id or label…\"></label>",
        escape(&facets.q)
    );
    body.push_str(&select(
        "state",
        "State",
        &facets.state,
        &status_options(s.beads.iter()),
    ));
    body.push_str(&ready_select(facets));
    body.push_str(&select("type", "Type", &facets.issue_type, &types_options));
    body.push_str(&priority_select(s, facets));
    body.push_str(APPLY);
    body
}

/// The query a page link carries: the five keys every bead list has always
/// written (even when empty), then any other facet set.
pub(crate) fn page_query(facets: &Facets) -> Vec<(String, String)> {
    let legacy = [
        ("q", &facets.q),
        ("state", &facets.state),
        ("ready", &facets.ready),
        ("type", &facets.issue_type),
        ("priority", &facets.priority),
    ];
    let mut out: Vec<(String, String)> = legacy
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    out.extend(
        facets
            .pairs()
            .into_iter()
            .filter(|(k, _)| !legacy.iter().any(|(l, _)| l == k)),
    );
    out
}

/// Bead indexes that pass `facets`, in display order, as one page of
/// numbered rows; also the match count, the page shown and the page count.
pub(crate) fn paged<'a>(
    s: &'a Snapshot,
    matching: Vec<usize>,
    facets: &Facets,
) -> (Vec<(usize, &'a Bead)>, usize, usize, usize) {
    let (current, pages) = super::page_of(matching.len(), PAGE_SIZE, facets.page());
    let rows = matching
        .iter()
        .enumerate()
        .skip(current * PAGE_SIZE)
        .take(PAGE_SIZE)
        .map(|(n, &i)| (n + 1, &s.beads[i]))
        .collect();
    (rows, matching.len(), current, pages)
}

pub fn beads(view: &View, facets: &Facets) -> String {
    let Some(s) = view.snapshot() else {
        return not_found(view);
    };
    let mut body = heading(
        "Work",
        "Beads",
        "Search every bead in priority order, including closed history.",
    );
    body.push_str(&beads_form(view, s, facets));
    let matching: Vec<usize> = s
        .order
        .iter()
        .copied()
        .filter(|&i| facets.matches(view, i))
        .collect();
    let (rows, total, current, pages) = paged(s, matching, facets);
    write!(
        body,
        "<p class=\"muted\">{total} matching beads · page {} of {pages}</p>",
        current + 1
    )
    .unwrap();
    body.push_str(&bead_table(s, view.provenance.as_ref(), &rows));
    body.push_str(&pagination("/beads", &page_query(facets), current, pages));
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

pub fn epics(view: &View, facets: &Facets) -> String {
    let Some(s) = view.snapshot() else {
        return not_found(view);
    };
    let mut body = heading(
        "Work",
        "Epics",
        "Follow each epic from its remaining children to the ones already closed.",
    );
    if profiled(&view.profile) {
        body.push_str(&facets_form(view, facets, "/epics"));
    } else {
        write!(body, "<form class=\"filters\" action=\"/epics\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Title or id…\"></label>", escape(&facets.q)).unwrap();
        body.push_str(&select(
            "state",
            "State",
            &facets.state,
            &status_options(s.beads.iter().filter(|b| b.is_epic())),
        ));
        body.push_str(APPLY);
    }
    let epics: Vec<&Bead> = s
        .order
        .iter()
        .filter(|&&i| s.beads[i].is_epic() && facets.matches(view, i))
        .map(|&i| &s.beads[i])
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
    // Children of every epic in one pass, items only.
    let mut children: std::collections::HashMap<&str, (usize, usize, usize)> =
        std::collections::HashMap::new();
    for &i in &s.order {
        if !s.derived.is_item(i) {
            continue;
        }
        let child = &s.beads[i];
        if let Some(parent) = child.parent() {
            let entry = children.entry(parent).or_default();
            entry.0 += 1;
            entry.1 += usize::from(child.is_closed());
            entry.2 += usize::from(child.ready);
        }
    }
    for epic in epics {
        let (all, closed, ready) = children.get(epic.id.as_str()).copied().unwrap_or_default();
        write!(body, "<article class=\"epic-card\"><div class=\"card-top\"><small>P{} · {}</small>{}</div><h2>{}</h2><p>{}</p><progress max=\"{}\" value=\"{closed}\" aria-label=\"Closed children\"></progress><div class=\"card-bottom\"><span>{closed} / {all} closed</span><span>{} open · {ready} ready</span></div></article>", epic.priority, escape(&epic.id), badge(&epic.status), bead_link(&epic.id, &epic.title), escape(&summary(&epic.description)), all.max(1), all - closed).unwrap();
    }
    body.push_str("</div>");
    page(view, "Epics", "/epics", &body)
}

#[cfg(test)]
mod tests {
    use super::{beads, epics, page_query};
    use crate::query::Facets;
    use crate::testing::work_view;

    fn facets(query: &str) -> Facets {
        let uri: axum::http::Uri = format!("/x?{query}").parse().unwrap();
        axum::extract::Query::<Facets>::try_from_uri(&uri)
            .unwrap()
            .0
    }

    #[test]
    fn the_profiled_list_filters_by_derived_state_and_group_and_keeps_old_keys() {
        let view = work_view();
        let page = beads(&view, &facets("dstate=held&area=reports"));
        assert!(page.contains("1 matching beads"), "{page}");
        assert!(page.contains("WORK-0003"));
        for key in [
            "name=\"dstate\"",
            "name=\"area\"",
            "name=\"ready\"",
            "name=\"priority\"",
        ] {
            assert!(page.contains(key), "{key}");
        }
        let all = beads(&view, &facets("state=all"));
        assert!(
            all.contains("8 matching beads"),
            "receipts and hidden rows are out"
        );
        assert!(!all.contains("wk-rc-gate"));
        assert!(!beads(&view, &facets("ready=yes")).contains("WORK-0003"));
        assert_eq!(
            page_query(&facets("area=reports&q=x")),
            [
                ("q".to_string(), "x".to_string()),
                ("state".into(), String::new()),
                ("ready".into(), String::new()),
                ("type".into(), String::new()),
                ("priority".into(), String::new()),
                ("area".into(), "reports".into()),
            ]
        );
    }

    #[test]
    fn epics_count_item_children_only() {
        let view = work_view();
        let page = epics(&view, &Facets::default());
        assert!(page.contains("Orders and invoices"));
        // wk-1 … wk-4 name EPIC-0001; wk-2 is closed.
        assert!(page.contains("1 / 4 closed"), "{page}");
        assert!(page.contains("name=\"dstate\""), "the facets form");
    }
}
