// Owned by WP1 — Lists, groups, search, overview. Frozen signature:
// `bead_table(&beads::Snapshot, Option<&provenance::Store>, &[(usize, &Bead)]) -> String`.
//! The bead table every list of beads shares.
//!
//! With a profile, each row also shows its kind, its derived-state badges,
//! its group values (linking to `/by/{group}/{value}`) and who holds it;
//! the compact stage bar shows for beads in progress and for beads in a
//! `read_ahead` state (held work, say) once their trail is read. Without a
//! profile the table is exactly what it was before profiles.

use super::progress::compact;
use super::{badge, bead_link, escape, group_link, person_link, tone_badge};
use crate::beads::{self, Bead};
use crate::profile::{Profile, RowFacts};
use crate::provenance;
use std::fmt::Write;

pub(crate) fn bead_table(
    s: &beads::Snapshot,
    trails: Option<&provenance::Store>,
    rows: &[(usize, &Bead)],
) -> String {
    if rows.is_empty() {
        return "<p class=\"empty\">No beads match this view.</p>".into();
    }
    let profile = &*s.derived.profile;
    let now = chrono::Utc::now();
    let mut out = "<div class=\"table-wrap\"><table class=\"items\"><thead><tr><th>#</th><th>Bead</th><th>State</th><th>Priority</th></tr></thead><tbody>".to_string();
    for (position, bead) in rows {
        let facts = s.position(&bead.id).and_then(|i| s.derived.rows.get(i));
        let parent = bead
            .parent()
            .and_then(|p| s.get(p))
            .map(|p| format!(" · {}", escape(&p.title)))
            .unwrap_or_default();
        let reference = if bead.external_ref.is_empty() {
            String::new()
        } else {
            format!(" · {}", escape(&bead.external_ref))
        };
        let kind = facts
            .and_then(|r| r.kind.as_deref())
            .and_then(|k| profile.kind(k))
            .map_or(bead.issue_type.as_str(), |k| k.label.as_str());
        let read_ahead = facts.is_some_and(|r| {
            r.states
                .iter()
                .any(|k| profile.state(k).is_some_and(|st| st.read_ahead))
        });
        let stage = if bead.status == "in_progress" || read_ahead {
            trails
                .and_then(|t| compact(t, bead, now))
                .map(|(bar, now)| {
                    format!("<div class=\"stage-mini\">{bar}<small>{now}</small></div>")
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        let (states, extra) = facts
            .map(|r| (state_badges(profile, r), row_facts(profile, r)))
            .unwrap_or_default();
        write!(out, "<tr><td class=\"muted\">{position}</td><td>{}<small>{}{reference} · {}{parent}</small>{extra}</td><td>{}{states}{stage}</td><td>P{}</td></tr>", bead_link(&bead.id, &bead.title), escape(&bead.id), escape(kind), badge(&bead.status), bead.priority).unwrap();
    }
    out.push_str("</tbody></table></div>");
    out
}

/// One badge per derived state, labelled and toned by the profile (the
/// same badges `render::state_badges` draws from a `View`).
fn state_badges(profile: &Profile, row: &RowFacts) -> String {
    row.states
        .iter()
        .filter_map(|key| profile.state(key))
        .map(|state| tone_badge(&state.label, state.tone))
        .collect()
}

/// The group chips and the holder (the first role, in profile order, the
/// bead names someone for); empty when there are neither.
fn row_facts(profile: &Profile, row: &RowFacts) -> String {
    let mut chips = String::new();
    for group in &profile.groups {
        for value in row.groups.get(&group.key).into_iter().flatten() {
            write!(
                chips,
                "<span class=\"chip\" title=\"{}\">{}</span>",
                escape(&group.label),
                group_link(&group.key, value, value)
            )
            .unwrap();
        }
    }
    let holder = profile
        .people
        .roles
        .iter()
        .find_map(|role| Some((role, row.people.get(&role.key)?)))
        .map(|(role, name)| {
            format!(
                "<span class=\"holder\">{} {}</span>",
                escape(&role.label),
                person_link(name)
            )
        })
        .unwrap_or_default();
    if chips.is_empty() && holder.is_empty() {
        return String::new();
    }
    format!("<div class=\"row-facts\">{chips}{holder}</div>")
}

#[cfg(test)]
mod tests {
    use super::bead_table;
    use crate::testing::work_view;

    #[test]
    fn a_profiled_table_shows_kind_states_groups_holder_and_the_held_stage() {
        let view = work_view();
        let s = view.snapshot().unwrap();
        let rows: Vec<_> = ["wk-1", "wk-3"]
            .iter()
            .enumerate()
            .map(|(n, id)| (n + 1, s.get(id).unwrap()))
            .collect();
        let table = bead_table(s, view.provenance.as_ref(), &rows);
        assert!(table.contains("WORK-0001 · Packet"), "{table}");
        assert!(table.contains("<span class=\"badge bad\">Held</span>"));
        assert!(table.contains("<span class=\"badge good\">Working</span>"));
        assert!(table.contains("href=\"/by/milestone/v1\""));
        assert!(table.contains("href=\"/by/area/reports\""));
        assert!(table.contains(
            "Claimed by <a href=\"/person/factory%2FhostA%2FrigA\">factory/hostA/rigA</a>"
        ));
        assert_eq!(
            table.matches("class=\"stage-mini\"").count(),
            2,
            "in progress and held both show the bar: {table}"
        );
        assert!(!table.contains(" style="));
    }

    #[test]
    fn without_a_profile_the_table_is_unchanged() {
        let view = crate::render::tests::view();
        let s = view.snapshot().unwrap();
        let bead = s.get("fx-1").unwrap();
        assert_eq!(
            bead_table(s, None, &[(1, bead)]),
            "<div class=\"table-wrap\"><table class=\"items\"><thead><tr><th>#</th><th>Bead</th><th>State</th><th>Priority</th></tr></thead><tbody><tr><td class=\"muted\">1</td><td><a href=\"/bead/fx%2D1\">Fix login</a><small>fx-1 · TICKET-12 · </small></td><td><span class=\"badge open\">open</span></td><td>P0</td></tr></tbody></table></div>"
        );
    }
}
