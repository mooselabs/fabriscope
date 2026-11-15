//! The bead list and the epics page.

use super::layout::{not_found, page};
use super::table::bead_table;
use super::{
    PAGE_SIZE, badge, bead_link, escape, fmt::words, heading, pagination, select, status_options,
    tally,
};
use crate::beads::Bead;
use crate::model::{BeadFilter, View};
use std::collections::BTreeSet;
use std::fmt::Write;

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
    body.push_str(&bead_table(s, view.provenance.as_ref(), &rows));
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
