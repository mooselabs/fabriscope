// Owned by WP2 — Bead page. Frozen signature:
// `bead(&View, &str) -> Option<String>`.
//! One bead's page.

use super::fmt::words;
use super::groups::members_table;
use super::layout::page;
use super::markdown::{Links, markdown};
use super::panels::{banner, panels, receipt_of, receipts_section, runs_section};
use super::progress::{cost_section, progress_section};
use super::table::bead_table;
use super::timeline::timeline;
use super::{
    badge, bead_link, escape, group_link, heading, person_link, scroll_link, state_badges,
};
use crate::beads::Bead;
use crate::model::View;
use crate::profile::{Profile, Source};
use std::fmt::Write;

/// Whether the page shows more than Beads itself stores: a profile is in
/// use (`--parent-key` alone does not count) or a run feed is read. Without
/// either the page is exactly what it was before profiles.
pub(crate) fn enriched(view: &View) -> bool {
    if view.runs.is_some() {
        return true;
    }
    let profile = &*view.profile;
    let generic = Profile {
        parent: profile.parent.clone(),
        ..Profile::default()
    };
    *profile != generic
}

/// The profile's view of the bead in its header: kind, derived states,
/// group values and who holds each role.
fn profile_meta(view: &View, bead: &Bead) -> (String, String) {
    let Some(row) = view.facts(&bead.id) else {
        return Default::default();
    };
    let mut before = String::new();
    if let Some(kind) = row.kind.as_deref().and_then(|k| view.profile.kind(k)) {
        write!(
            before,
            "<span class=\"kind\">{}</span>",
            escape(&kind.label)
        )
        .unwrap();
    }
    before.push_str(&state_badges(view, row));
    let mut after = String::new();
    for group in &view.profile.groups {
        let Some(values) = row.groups.get(&group.key) else {
            continue;
        };
        let lane = row
            .subgroups
            .get(&group.key)
            .map(|l| format!(" · {}", escape(l)))
            .unwrap_or_default();
        let links: Vec<String> = values
            .iter()
            .map(|v| group_link(&group.key, v, v))
            .collect();
        write!(
            after,
            "<span class=\"chip\">{} {}{lane}</span>",
            escape(&group.label),
            links.join(", ")
        )
        .unwrap();
    }
    for role in &view.profile.people.roles {
        if let Some(name) = view.person_in_role(&bead.id, &role.key) {
            write!(
                after,
                "<span class=\"role\">{} {}</span>",
                escape(&role.label),
                person_link(&name)
            )
            .unwrap();
        }
    }
    (before, after)
}

/// Whether a state the bead is in says it is never ready.
fn ready_overridden(view: &View, bead: &Bead) -> bool {
    view.facts(&bead.id).is_some_and(|row| {
        row.states
            .iter()
            .filter_map(|k| view.profile.state(k))
            .any(|s| s.overrides_ready)
    })
}

/// The members of each group whose value this bead is (a milestone a
/// `ref:` or `declared` group names, say), as that group's members table.
fn group_members(view: &View, bead: &Bead) -> String {
    let Some(index) = view.group_index() else {
        return String::new();
    };
    let mut out = String::new();
    for group in &view.profile.groups {
        let names_beads =
            group.declared.is_some() || group.from.iter().any(|s| matches!(s, Source::Ref(_)));
        if !names_beads {
            continue;
        }
        let Some(values) = index.get(&group.key) else {
            continue;
        };
        for key in [&bead.external_ref, &bead.id] {
            let Some(members) = values.get(key.as_str()).filter(|_| !key.is_empty()) else {
                continue;
            };
            write!(
                out,
                "<section class=\"group-members\" id=\"members-{}\"><h2>{} {} · {}</h2><p>{}</p>{}</section>",
                super::url(&group.key),
                escape(&group.label),
                escape(key),
                members.len(),
                group_link(&group.key, key, &format!("Open the {} page", group.label.to_lowercase())),
                members_table(view, &group.key, key)
            )
            .unwrap();
            break;
        }
    }
    out
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
    let (kind_states, groups_people) = profile_meta(view, bead);
    write!(
        body,
        "<div class=\"record-meta\">{kind_states}{}<span>P{}</span><span>{}</span>",
        badge(&bead.status),
        bead.priority,
        escape(&bead.issue_type)
    )
    .unwrap();
    if bead.ready && !ready_overridden(view, bead) {
        body.push_str("<span>Ready, as bd ready decides</span>");
    }
    if !bead.assignee.is_empty() {
        write!(body, "<span>Assigned to {}</span>", escape(&bead.assignee)).unwrap();
    }
    body.push_str(&groups_people);
    for label in &bead.labels {
        write!(body, "<span class=\"label\">{}</span>", escape(label)).unwrap();
    }
    body.push_str("</div>");
    body.push_str(&receipt_of(view, bead));
    body.push_str(&banner(view, bead));
    body.push_str(&progress_section(view, bead));
    body.push_str(&cost_section(view, bead));
    body.push_str(&panels(view, bead));
    body.push_str(&runs_section(view, bead));
    body.push_str(&receipts_section(view, bead));
    body.push_str(&group_members(view, bead));
    body.push_str(&timeline(view, bead));
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
