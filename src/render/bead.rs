//! One bead's page.

use super::fmt::words;
use super::layout::page;
use super::markdown::{Links, markdown};
use super::progress::progress_section;
use super::table::bead_table;
use super::{badge, bead_link, escape, heading, scroll_link};
use crate::beads::Bead;
use crate::model::View;
use std::fmt::Write;

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
    body.push_str(&progress_section(view, bead));
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
        write!(body, "<section><h2>Open children · {}</h2>{}<details id=\"closed-children\"><summary>Closed children · {}</summary>{}</details></section>", open.len(), bead_table(s, view.provenance.as_ref(), &open), closed.len(), bead_table(s, None, &closed)).unwrap();
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
