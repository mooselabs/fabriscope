// Owned by WP1 — Lists, groups, search, overview. Frozen signatures:
// `group(&View, &str, &Facets) -> Option<String>`,
// `lane(&View, &str, &str, &Facets) -> Option<String>`,
// `members_table(&View, &str, &str) -> String`.
//! Group pages: `/by/{group}` (every value of one `[[groups]]` key, with
//! counts) and `/by/{group}/{value}` (the members, as a lane table when the
//! group has a `subgroup`, ordered by its `declared` values).
//!
//! Counts honour every facet in the URL except the stored state, so each
//! state shows side by side; the members table honours them all.

use super::beads::paged;
use super::layout::page;
use super::table::bead_table;
use super::{
    PAGE_SIZE, badge, bead_link, escape, facets_form, fmt::words, heading, state_badges, tone_badge,
};
use crate::beads::Snapshot;
use crate::filters::{Counts, Lane, counting, group_bead, holding, lane_order, lanes};
use crate::model::View;
use crate::profile::{GroupRule, Source, StateRule, Tone};
use crate::query::{Facets, href};
use std::fmt::Write;

/// The count columns' headings: open, ready, in progress, each holding
/// state, closed, and open gates when the store has gate-type items.
fn count_head(first: &str, hold: &[&StateRule], gates: bool) -> String {
    let mut out = format!(
        "<thead><tr><th>{}</th><th class=\"num\">Open</th><th class=\"num\">Ready</th><th class=\"num\">In progress</th>",
        escape(first)
    );
    for state in hold {
        write!(out, "<th class=\"num\">{}</th>", escape(&state.label)).unwrap();
    }
    out.push_str("<th class=\"num\">Closed</th>");
    if gates {
        out.push_str("<th class=\"num\">Open gates</th>");
    }
    out.push_str("</tr></thead>");
    out
}

fn count_cells(c: &Counts, gates: bool) -> String {
    let mut out = format!(
        "<td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>",
        c.open, c.ready, c.working
    );
    for n in &c.held {
        write!(out, "<td class=\"num\">{n}</td>").unwrap();
    }
    write!(out, "<td class=\"num\">{}</td>", c.closed).unwrap();
    if gates {
        write!(out, "<td class=\"num\">{}</td>", c.gates).unwrap();
    }
    out
}

/// Whether the store holds any gate-type item.
fn has_gates(s: &Snapshot) -> bool {
    s.beads
        .iter()
        .enumerate()
        .any(|(i, b)| b.issue_type == "gate" && s.derived.is_item(i))
}

/// One tab per profile group, the current one marked.
fn group_tabs(view: &View, current: &str, facets: &Facets) -> String {
    let mut out = "<nav class=\"group-tabs\" aria-label=\"Groups\">".to_string();
    for rule in &view.profile.groups {
        let here = if rule.key == current {
            " aria-current=\"page\""
        } else {
            ""
        };
        write!(
            out,
            "<a href=\"{}\"{here}>{}</a>",
            escape(&href(
                &format!("/by/{}", super::url(&rule.key)),
                &counting(facets, Some(&rule.key)),
                &[("state", "")]
            )),
            escape(&rule.label)
        )
        .unwrap();
    }
    out.push_str("</nav>");
    out
}

/// A line naming the filters a count honours (never the stored state,
/// which counts ignore), with a link that clears them.
fn filter_note(path: &str, facets: &Facets) -> String {
    let named: Vec<String> = facets
        .pairs()
        .iter()
        .filter(|(k, _)| k != "state")
        .map(|(k, v)| format!("{}={}", escape(k), escape(v)))
        .collect();
    if named.is_empty() {
        return String::new();
    }
    format!(
        "<p class=\"muted filter-note\">Filtered by {} · <a href=\"{}\">Clear</a></p>",
        named.join(", "),
        escape(path)
    )
}

/// `/by/{group}`; `None` when the profile has no such group.
pub(crate) fn group(view: &View, group: &str, facets: &Facets) -> Option<String> {
    let rule = view.profile.group(group)?;
    let s = view.snapshot()?;
    let path = format!("/by/{}", super::url(group));
    let hold = holding(&view.profile);
    let gates = has_gates(s);
    let count = counting(facets, Some(group));
    let mut body = heading(
        "Groups",
        &rule.label,
        &format!(
            "Every {} with how its items stand.",
            rule.label.to_lowercase()
        ),
    );
    body.push_str(&group_tabs(view, group, facets));
    body.push_str(&filter_note(&path, &count));
    let values = s.derived.groups.get(group);
    let rows: Vec<(&String, Counts)> = values
        .into_iter()
        .flatten()
        .map(|(value, members)| {
            let c = Counts::of(s, members.iter().copied(), &hold, |i| {
                count.matches(view, i)
            });
            (value, c)
        })
        .filter(|(_, c)| c.total > 0)
        .collect();
    if rows.is_empty() {
        body.push_str("<p class=\"empty\">No items have a value in this group.</p>");
        return Some(page(view, &rule.label, &path, &body));
    }
    write!(
        body,
        "<p class=\"muted\">{} values</p><div class=\"table-wrap\"><table class=\"data group-values\">{}<tbody>",
        rows.len(),
        count_head(&rule.label, &hold, gates)
    )
    .unwrap();
    for (value, c) in rows {
        let link = href(
            &format!("/by/{}/{}", super::url(group), super::url(value)),
            &count,
            &[("state", "")],
        );
        let named = group_bead(s, rule, value)
            .map(|b| format!("<small>{} {}</small>", escape(&b.title), badge(&b.status)))
            .unwrap_or_default();
        write!(
            body,
            "<tr><td><a href=\"{}\">{}</a>{named}</td>{}</tr>",
            escape(&link),
            escape(value),
            count_cells(&c, gates)
        )
        .unwrap();
    }
    body.push_str("</tbody></table></div>");
    Some(page(view, &rule.label, &path, &body))
}

/// The heading of a value's page: the bead the value names, when it names
/// one, with its title and status.
fn lane_heading(view: &View, s: &Snapshot, rule: &GroupRule, value: &str) -> String {
    let title = format!("{} {value}", rule.label);
    let Some(bead) = group_bead(s, rule, value) else {
        return heading(&rule.label, &title, "");
    };
    let states = s
        .position(&bead.id)
        .and_then(|i| s.derived.rows.get(i))
        .map(|r| state_badges(view, r))
        .unwrap_or_default();
    format!(
        "{}<p class=\"group-bead\">{} {}{states}</p>",
        heading(&rule.label, &title, &bead.title),
        bead_link(&bead.id, &format!("Open {}", bead.id)),
        badge(&bead.status)
    )
}

/// The lane table of one group value.
fn lane_table(
    rule: &GroupRule,
    lanes: &[Lane],
    hold: &[&StateRule],
    gates: bool,
    subgroup: &str,
) -> String {
    let mut out = format!(
        "<section id=\"lanes\"><h2>Lanes</h2><div class=\"table-wrap\"><table class=\"data lane-table\">{}<tbody>",
        count_head(subgroup, hold, gates)
    );
    for lane in lanes {
        let name = match &lane.name {
            Some(name) if lane.declared => escape(name),
            Some(name) => format!("{} {}", escape(name), tone_badge("stray", Tone::Warn)),
            None => "<span class=\"muted\">None</span>".to_string(),
        };
        write!(
            out,
            "<tr><td>{name}</td>{}</tr>",
            count_cells(&lane.counts, gates)
        )
        .unwrap();
    }
    write!(
        out,
        "</tbody></table></div><p class=\"method\">Values the {} declares come first, in its order; a stray value is found on members but not declared.</p></section>",
        escape(&rule.label.to_lowercase())
    )
    .unwrap();
    out
}

/// A value's members that pass `keep`, in lane then display order, and
/// the lanes (empty when the group has no subgroup).
fn members(
    s: &Snapshot,
    view: &View,
    rule: &GroupRule,
    value: &str,
    keep: impl Fn(usize) -> bool + Copy,
) -> (Vec<usize>, Vec<Lane>) {
    let all: &[usize] = s
        .derived
        .groups
        .get(&rule.key)
        .and_then(|m| m.get(value))
        .map_or(&[], Vec::as_slice);
    let hold = holding(&view.profile);
    let lanes = if rule.subgroup.is_some() {
        lanes(s, rule, value, all, &hold, keep)
    } else {
        Vec::new()
    };
    let kept: Vec<usize> = all.iter().copied().filter(|&i| keep(i)).collect();
    (lane_order(s, rule, &kept, &lanes), lanes)
}

/// `/by/{group}/{value}`; `None` when the profile has no such group, or no
/// item has that value and it names no bead.
pub(crate) fn lane(view: &View, group: &str, value: &str, facets: &Facets) -> Option<String> {
    let rule = view.profile.group(group)?;
    let s = view.snapshot()?;
    let known = s
        .derived
        .groups
        .get(group)
        .is_some_and(|m| m.contains_key(value));
    if !known && group_bead(s, rule, value).is_none() {
        return None;
    }
    let path = format!("/by/{}/{}", super::url(group), super::url(value));
    let title = format!("{} {value}", rule.label);
    let hold = holding(&view.profile);
    let gates = has_gates(s);
    let count = counting(facets, Some(group));
    let mut body = lane_heading(view, s, rule, value);
    body.push_str(&group_tabs(view, group, facets));
    write!(
        body,
        "<p><a href=\"{}\">← Every {}</a></p>",
        escape(&href(
            &format!("/by/{}", super::url(group)),
            &count,
            &[("state", "")]
        )),
        escape(&rule.label.to_lowercase())
    )
    .unwrap();
    let (_, lanes) = members(s, view, rule, value, |i| count.matches(view, i));
    if let Some(sub) = &rule.subgroup {
        body.push_str(&lane_table(rule, &lanes, &hold, gates, &subgroup_name(sub)));
    }
    let (kept, _) = members(s, view, rule, value, |i| facets.matches(view, i));
    body.push_str("<section id=\"members\"><h2>Members</h2>");
    body.push_str(&facets_form(view, facets, &path));
    let (rows, total, current, pages) = paged(s, kept, facets);
    write!(
        body,
        "<p class=\"muted\">{total} matching items · page {} of {pages}</p>",
        current + 1
    )
    .unwrap();
    body.push_str(&bead_table(s, view.provenance.as_ref(), &rows));
    body.push_str(&super::pagination(&path, &facets.pairs(), current, pages));
    body.push_str("</section>");
    Some(page(
        view,
        &title,
        &format!("/by/{}", super::url(group)),
        &body,
    ))
}

/// A heading for a subgroup column, from its source: the last segment of a
/// metadata path (`work.fields.lane` → `Lane`), else `Subgroup`.
fn subgroup_name(source: &Source) -> String {
    let name = match source {
        Source::Meta(parts) | Source::Ref(parts) => parts.last().map(|p| words(p)),
        _ => None,
    }
    .unwrap_or_else(|| "subgroup".into());
    capital(&name)
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The members of one group value as a table, every state, in lane order
/// (declared lanes first) then display order; the first `PAGE_SIZE`. Empty
/// text when the profile has no such group.
pub(crate) fn members_table(view: &View, group: &str, value: &str) -> String {
    let (Some(rule), Some(s)) = (view.profile.group(group), view.snapshot()) else {
        return String::new();
    };
    let (kept, _) = members(s, view, rule, value, |i| s.derived.is_item(i));
    let rows: Vec<_> = kept
        .iter()
        .take(PAGE_SIZE)
        .enumerate()
        .map(|(n, &i)| (n + 1, &s.beads[i]))
        .collect();
    bead_table(s, view.provenance.as_ref(), &rows)
}

#[cfg(test)]
mod tests {
    use super::{group, lane, members_table};
    use crate::query::Facets;
    use crate::testing::work_view;

    #[test]
    fn a_group_lists_every_value_with_counts_and_tabs() {
        let view = work_view();
        let page = group(&view, "milestone", &Facets::default()).unwrap();
        for wanted in [
            "href=\"/by/milestone/v1\"",
            "href=\"/by/milestone/v2\"",
            "href=\"/by/rig\"",
            "<th class=\"num\">Held</th>",
            "<th class=\"num\">Open gates</th>",
        ] {
            assert!(page.contains(wanted), "{wanted}: {page}");
        }
        // v1: wk-1 (working), wk-2 (closed), wk-4 (open gate).
        assert!(page.contains("<td class=\"num\">2</td><td class=\"num\">0</td><td class=\"num\">1</td><td class=\"num\">0</td><td class=\"num\">1</td><td class=\"num\">1</td>"), "{page}");
        assert!(group(&view, "nope", &Facets::default()).is_none());
        let mut f = Facets::default();
        f.groups.insert("area".into(), "reports".into());
        let filtered = group(&view, "milestone", &f).unwrap();
        assert!(!filtered.contains("/by/milestone/v1"), "{filtered}");
        assert!(filtered.contains("Filtered by area=reports"));
    }

    #[test]
    fn a_value_page_names_its_bead_and_lists_lanes_and_members() {
        let view = work_view();
        let page = lane(&view, "milestone", "v1", &Facets::default()).unwrap();
        assert!(page.contains("href=\"/bead/wk%2Dm1\""), "{page}");
        assert!(page.contains("<h2>Lanes</h2>"));
        assert!(page.contains("<td>build</td>"));
        assert!(page.contains("<td>polish</td>"));
        assert!(
            page.contains("2 matching items"),
            "open members only by default"
        );
        assert!(lane(&view, "milestone", "v9", &Facets::default()).is_none());
        let v2 = lane(&view, "milestone", "v2", &Facets::default()).unwrap();
        assert!(
            v2.contains("build <span class=\"badge warn\">stray</span>"),
            "{v2}"
        );
        let rig = lane(&view, "rig", "rigB", &Facets::default()).unwrap();
        assert!(!rig.contains("<h2>Lanes</h2>"));
        assert!(rig.contains("WORK-0003"));
        let table = members_table(&view, "milestone", "v1");
        assert_eq!(table.matches("<tr><td class=\"muted\">").count(), 3);
        assert_eq!(members_table(&view, "nope", "v1"), "");
        assert!(!page.contains(" style="));
    }
}
