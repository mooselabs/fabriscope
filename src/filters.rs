// Owned by WP1 — Lists, groups, search, overview. No frozen signature: WP1
// defines this module.
//! List filtering beyond `query::Facets::matches`: the counts list pages
//! show, the search ranking behind `/search`, and lane ordering from a
//! group's `declared` values.
//!
//! Every helper works over bead indexes into `Snapshot::beads` and the
//! profile facts derived once per read, so a page over ~12,000 beads stays
//! one pass with no per-row history or provenance reads.

use crate::beads::{Bead, Snapshot};
use crate::model::View;
use crate::profile::{GroupRule, Profile, Source, StateRule};
use crate::query::Facets;
use std::collections::BTreeSet;

/// Whether a profile changes what lists show: it classes rows, or names
/// kinds, states, groups or roles. Without one, list pages render exactly
/// as they did before profiles existed.
pub(crate) fn profiled(profile: &Profile) -> bool {
    !(profile.rows.is_empty()
        && profile.kinds.is_empty()
        && profile.states.is_empty()
        && profile.groups.is_empty()
        && profile.people.roles.is_empty())
}

/// The states that hold items back (`overrides_ready`), in profile order.
pub(crate) fn holding(profile: &Profile) -> Vec<&StateRule> {
    profile
        .states
        .iter()
        .filter(|s| s.overrides_ready)
        .collect()
}

/// How a set of items stands. `open` counts every item not closed (so it
/// includes those in progress and held); `held[n]` counts items in the
/// n-th holding state; `gates` counts open gate-type items.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub total: usize,
    pub open: usize,
    pub ready: usize,
    pub working: usize,
    /// Status `open` and not ready.
    pub waiting: usize,
    pub held: Vec<usize>,
    pub closed: usize,
    pub gates: usize,
}

impl Counts {
    /// Zero counts for `holding` states.
    pub fn new(holding: &[&StateRule]) -> Self {
        Counts {
            held: vec![0; holding.len()],
            ..Default::default()
        }
    }

    /// Counts bead `index` of the snapshot when it is an item.
    pub fn add(&mut self, s: &Snapshot, index: usize, holding: &[&StateRule]) {
        if !s.derived.is_item(index) {
            return;
        }
        let Some(bead) = s.beads.get(index) else {
            return;
        };
        self.total += 1;
        if bead.is_closed() {
            self.closed += 1;
            return;
        }
        self.open += 1;
        self.ready += usize::from(bead.ready);
        self.working += usize::from(bead.status == "in_progress");
        self.waiting += usize::from(bead.status == "open" && !bead.ready);
        self.gates += usize::from(bead.issue_type == "gate");
        if let Some(row) = s.derived.rows.get(index) {
            for (n, state) in holding.iter().enumerate() {
                if row.states.contains(&state.key) {
                    self.held[n] += 1;
                }
            }
        }
    }

    /// Counts each index that passes `keep`.
    pub fn of(
        s: &Snapshot,
        indexes: impl IntoIterator<Item = usize>,
        holding: &[&StateRule],
        keep: impl Fn(usize) -> bool,
    ) -> Self {
        let mut counts = Counts::new(holding);
        for i in indexes {
            if keep(i) {
                counts.add(s, i, holding);
            }
        }
        counts
    }
}

/// The facets a count honours: every filter the person chose except the
/// stored state (counts show every state side by side) and, on a group's
/// own page, that group.
pub(crate) fn counting(facets: &Facets, group: Option<&str>) -> Facets {
    let mut f = facets.clone();
    f.state = "all".into();
    f.page.clear();
    if let Some(key) = group {
        f.groups.remove(key);
    }
    f
}

/// Items in a `read_ahead` state that are not in progress (in-progress
/// beads are read ahead already), so held work has its trail when a page
/// shows it.
pub(crate) fn read_ahead(s: &Snapshot, limit: usize) -> Vec<String> {
    s.order
        .iter()
        .filter(|&&i| s.beads[i].status != "in_progress" && s.derived.reads_ahead(i))
        .take(limit)
        .map(|&i| s.beads[i].id.clone())
        .collect()
}

/// The bead a group value names, when the group says values name beads:
/// a `ref:` source, or a `declared` list read from that bead.
pub(crate) fn group_bead<'a>(s: &'a Snapshot, rule: &GroupRule, value: &str) -> Option<&'a Bead> {
    let names_beads =
        rule.declared.is_some() || rule.from.iter().any(|src| matches!(src, Source::Ref(_)));
    names_beads.then(|| s.find(value)).flatten()
}

/// One lane (subgroup value) of a group value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lane {
    /// The subgroup value; `None` for members with none.
    pub name: Option<String>,
    /// Listed in the group bead's `declared` source.
    pub declared: bool,
    pub counts: Counts,
}

/// The lanes of one group value: the declared subgroups in their declared
/// order, then any value members carry that is not declared ("stray"),
/// sorted, then members with no subgroup. Counts take every member index
/// that passes `keep`.
pub(crate) fn lanes(
    s: &Snapshot,
    rule: &GroupRule,
    value: &str,
    members: &[usize],
    holding: &[&StateRule],
    keep: impl Fn(usize) -> bool,
) -> Vec<Lane> {
    let declared: Vec<String> = match (&rule.declared, group_bead(s, rule, value)) {
        (Some(source), Some(bead)) => source.texts(bead),
        _ => Vec::new(),
    };
    let lane_of = |i: usize| {
        s.derived
            .rows
            .get(i)
            .and_then(|r| r.subgroups.get(&rule.key))
    };
    let stray: BTreeSet<&String> = members
        .iter()
        .filter_map(|&i| lane_of(i))
        .filter(|l| !declared.contains(l))
        .collect();
    let mut out: Vec<Lane> = declared
        .iter()
        .map(|l| (Some(l.clone()), true))
        .chain(stray.into_iter().map(|l| (Some(l.clone()), false)))
        .chain(
            members
                .iter()
                .any(|&i| lane_of(i).is_none())
                .then_some((None, false)),
        )
        .map(|(name, declared)| Lane {
            name,
            declared,
            counts: Counts::new(holding),
        })
        .collect();
    for &i in members {
        if !keep(i) {
            continue;
        }
        let lane = lane_of(i);
        if let Some(slot) = out.iter_mut().find(|l| l.name.as_ref() == lane) {
            slot.counts.add(s, i, holding);
        }
    }
    out
}

/// `members` in display order (priority, then age), then stably by lane:
/// declared lanes first in their order, then stray lanes, then none.
pub(crate) fn lane_order(
    s: &Snapshot,
    rule: &GroupRule,
    members: &[usize],
    lanes: &[Lane],
) -> Vec<usize> {
    let mut wanted = vec![false; s.beads.len()];
    for &i in members {
        if let Some(slot) = wanted.get_mut(i) {
            *slot = true;
        }
    }
    let mut out: Vec<usize> = s.order.iter().copied().filter(|&i| wanted[i]).collect();
    if rule.subgroup.is_some() {
        let rank = |i: usize| {
            let lane = s
                .derived
                .rows
                .get(i)
                .and_then(|r| r.subgroups.get(&rule.key));
            lanes
                .iter()
                .position(|l| l.name.as_ref() == lane)
                .unwrap_or(lanes.len())
        };
        out.sort_by_key(|&i| rank(i));
    }
    out
}

/// How well a bead matches lower-case search text, best first: 0 an exact
/// id or external ref, 1 within the id or ref, 2 the title, 3 a label, 4
/// the description. `None` when nothing matches.
pub(crate) fn rank(bead: &Bead, needle: &str) -> Option<u8> {
    let has = |hay: &str| hay.to_lowercase().contains(needle);
    if bead.id.to_lowercase() == needle || bead.external_ref.to_lowercase() == needle {
        Some(0)
    } else if has(&bead.id) || has(&bead.external_ref) {
        Some(1)
    } else if has(&bead.title) {
        Some(2)
    } else if bead.labels.iter().any(|l| has(l)) {
        Some(3)
    } else if has(&bead.description) {
        Some(4)
    } else {
        None
    }
}

/// Items matching `q` (any state), best rank first and display order
/// within a rank: the first `limit` indexes and how many matched in all.
pub(crate) fn search_beads(view: &View, q: &str, limit: usize) -> (Vec<usize>, usize) {
    let Some(s) = view.snapshot() else {
        return (Vec::new(), 0);
    };
    let needle = q.trim().to_lowercase();
    if needle.is_empty() {
        return (Vec::new(), 0);
    }
    let mut found: Vec<(u8, usize)> = s
        .order
        .iter()
        .filter(|&&i| s.derived.is_item(i))
        .filter_map(|&i| rank(&s.beads[i], &needle).map(|r| (r, i)))
        .collect();
    let total = found.len();
    found.sort_by_key(|(r, _)| *r);
    (
        found.into_iter().take(limit).map(|(_, i)| i).collect(),
        total,
    )
}

/// The bead an exact id or external ref names, for `/search` to go
/// straight to.
pub(crate) fn exact<'a>(view: &'a View, q: &str) -> Option<&'a Bead> {
    let q = q.trim();
    (!q.is_empty()).then(|| view.snapshot()?.find(q)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::work_view;

    fn index(view: &View, key: &str) -> usize {
        let s = view.snapshot().unwrap();
        s.position(&s.find(key).unwrap().id).unwrap()
    }

    #[test]
    fn counts_take_items_only_and_split_open_ready_working_held_closed() {
        let view = work_view();
        let s = view.snapshot().unwrap();
        let hold = holding(&view.profile);
        assert_eq!(hold.len(), 1);
        let all = Counts::of(s, 0..s.beads.len(), &hold, |_| true);
        // 8 items: wk-e1, wk-m1, wk-1 … wk-5, wk-q1; receipts and hidden
        // rows are not counted.
        assert_eq!(all.total, 8);
        assert_eq!(all.closed, 2);
        assert_eq!(all.open, 6);
        assert_eq!(all.working, 1);
        assert_eq!(all.held, [1], "WORK-0003");
        assert_eq!(all.gates, 1, "WORK-0004");
        assert_eq!(all.ready, s.ready.len());
        assert_eq!(all.waiting, all.open - all.working - all.ready);
        let none = Counts::of(s, 0..s.beads.len(), &hold, |_| false);
        assert_eq!(none, Counts::new(&hold));
    }

    #[test]
    fn lanes_list_declared_then_stray_then_none() {
        let view = work_view();
        let s = view.snapshot().unwrap();
        let rule = view.profile.group("milestone").unwrap();
        let hold = holding(&view.profile);
        let mut members = s.derived.groups["milestone"]["v1"].clone();
        // A member with no lane and the held bead (lane `build` of v2) as
        // if it were in v1 too.
        members.push(index(&view, "WORK-0003"));
        members.push(index(&view, "EPIC-0001"));
        let lanes = lanes(s, rule, "v1", &members, &hold, |_| true);
        let names: Vec<(Option<&str>, bool)> = lanes
            .iter()
            .map(|l| (l.name.as_deref(), l.declared))
            .collect();
        assert_eq!(
            names,
            [(Some("build"), true), (Some("polish"), true), (None, false)]
        );
        assert_eq!(lanes[0].counts.total, 3, "wk-1, wk-4 and wk-3");
        assert_eq!(lanes[0].counts.held, [1]);
        assert_eq!(lanes[0].counts.gates, 1);
        assert_eq!(lanes[1].counts.closed, 1, "wk-2");
        // v2 names no bead, so its one lane is stray.
        let v2 = &s.derived.groups["milestone"]["v2"];
        let lanes = super::lanes(s, rule, "v2", v2, &hold, |_| true);
        assert_eq!(lanes.len(), 1);
        assert!(!lanes[0].declared);
        // Ordered by lane, then display order.
        let order = lane_order(
            s,
            rule,
            &s.derived.groups["milestone"]["v1"],
            &super::lanes(s, rule, "v1", &members, &hold, |_| true),
        );
        let refs: Vec<&str> = order
            .iter()
            .map(|&i| s.beads[i].external_ref.as_str())
            .collect();
        assert_eq!(refs.last(), Some(&"WORK-0002"), "polish after build");
        assert!(group_bead(s, rule, "v1").is_some_and(|b| b.id == "wk-m1"));
        let rig = view.profile.group("rig").unwrap();
        assert!(
            group_bead(s, rig, "wk-1").is_none(),
            "rig values name no bead"
        );
    }

    #[test]
    fn search_ranks_ids_before_titles_before_descriptions_and_skips_receipts() {
        let view = work_view();
        let s = view.snapshot().unwrap();
        let (found, total) = search_beads(&view, "WORK-0003", 50);
        assert_eq!(total, 1);
        assert_eq!(s.beads[found[0]].external_ref, "WORK-0003");
        let (found, total) = search_beads(&view, "work-", 2);
        assert_eq!(found.len(), 2);
        assert!(total >= 5);
        assert!(found.iter().all(|&i| s.derived.is_item(i)));
        assert!(search_beads(&view, "  ", 50).1 == 0);
        let bead = Bead {
            id: "x-1".into(),
            title: "Alpha".into(),
            description: "beta".into(),
            labels: vec!["gamma".into()],
            ..Default::default()
        };
        assert_eq!(rank(&bead, "x-1"), Some(0));
        assert_eq!(rank(&bead, "x-"), Some(1));
        assert_eq!(rank(&bead, "alp"), Some(2));
        assert_eq!(rank(&bead, "gam"), Some(3));
        assert_eq!(rank(&bead, "bet"), Some(4));
        assert_eq!(rank(&bead, "zzz"), None);
        assert_eq!(exact(&view, " WORK-0003 ").unwrap().id, "wk-3");
        assert!(exact(&view, "WORK-000").is_none());
    }

    #[test]
    fn read_ahead_names_held_items_and_profiles_are_told_apart() {
        let view = work_view();
        let s = view.snapshot().unwrap();
        assert_eq!(read_ahead(s, 10), ["wk-3"]);
        assert!(profiled(&view.profile));
        assert!(!profiled(&Profile::default()));
        let generic = Snapshot::build(
            s.beads.clone(),
            vec![],
            None,
            chrono::Utc::now(),
            &Default::default(),
        );
        assert!(read_ahead(&generic, 10).is_empty());
        let mut f = Facets::default();
        f.groups.insert("milestone".into(), "v1".into());
        f.state = "open".into();
        let c = counting(&f, Some("milestone"));
        assert_eq!(c.state, "all");
        assert!(c.groups.is_empty());
    }
}
