//! The scroll list and one scroll's page.

use super::fmt::words;
use super::layout::{not_found, page};
use super::markdown::{Links, markdown};
use super::{PAGE_SIZE, badge, bead_link, escape, heading, pagination, scroll_link, select, url};
use crate::model::{ScrollFilter, View};
use crate::scrolls::{self, Scroll};
use std::collections::BTreeSet;
use std::fmt::Write;

pub(crate) fn scroll_table(rows: &[&Scroll]) -> String {
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
