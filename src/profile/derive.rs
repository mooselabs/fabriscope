//! What a profile says about each bead of a snapshot, worked out once per
//! read (inside `Snapshot::build`) so pages only look it up.

use super::{Profile, RowClass, Source};
use crate::beads::Snapshot;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap};

/// The profile's facts about one bead. `Derived::rows[i]` belongs to
/// `Snapshot::beads[i]`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowFacts {
    /// Item, receipt or hidden. Receipts and hidden rows are left out of
    /// default lists, counts, history and flow.
    pub class: RowClass,
    /// The first `[[kinds]]` key that matched.
    pub kind: Option<String>,
    /// Every `[[states]]` key that matched, in profile order.
    pub states: Vec<String>,
    /// The first matching state's `since`, as text.
    pub since: Option<String>,
    /// Group key → the bead's values in that group (none: not grouped).
    pub groups: BTreeMap<String, Vec<String>>,
    /// Group key → the subgroup value (a lane, say).
    pub subgroups: BTreeMap<String, String>,
    /// Role key → person name, from the bead's own fields. Roles read from
    /// history (`event:` sources) are not here; see `View::person_in_role`.
    pub people: BTreeMap<String, String>,
    /// Set on receipt rows a `[[receipts]]` rule matched.
    pub receipt: Option<ReceiptFacts>,
}

/// A receipt row's join keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReceiptFacts {
    /// The matching `[[receipts]]` rule's index and kind.
    pub rule: usize,
    pub kind: String,
    /// The subject bead's index in `Snapshot::beads`, when it resolved.
    pub subject: Option<usize>,
    pub attempt: Option<String>,
    pub at: Option<String>,
    pub actor: Option<String>,
}

/// Every bead's `RowFacts` and the indexes pages need. All indexes hold bead
/// indexes into `Snapshot::beads` and include items only, except the
/// receipt indexes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Derived {
    /// The profile these facts came from, so code given only a snapshot (a
    /// bead table, say) can label kinds, states and groups.
    pub profile: std::sync::Arc<Profile>,
    pub rows: Vec<RowFacts>,
    /// Group key → value → bead indexes, values sorted.
    pub groups: BTreeMap<String, BTreeMap<String, Vec<usize>>>,
    /// Person name → bead indexes naming them in any role.
    pub people: BTreeMap<String, Vec<usize>>,
    /// Subject bead index → receipt bead indexes.
    pub receipts_by_subject: HashMap<usize, Vec<usize>>,
    /// Attempt id → receipt bead indexes.
    pub receipts_by_attempt: HashMap<String, Vec<usize>>,
}

impl Derived {
    /// Applies `profile` to every bead of `snapshot` at `now`.
    pub fn build(snapshot: &Snapshot, profile: &Profile, now: DateTime<Utc>) -> Self {
        let mut out = Derived {
            profile: std::sync::Arc::new(profile.clone()),
            ..Default::default()
        };
        for (i, bead) in snapshot.beads.iter().enumerate() {
            let class = profile
                .rows
                .iter()
                .find(|r| r.when.matches(bead, now))
                .map(|r| r.class)
                .unwrap_or_default();
            let kind = profile
                .kinds
                .iter()
                .find(|k| k.when.matches(bead, now))
                .map(|k| k.key.clone());
            let matched: Vec<_> = profile
                .states
                .iter()
                .filter(|s| s.when.matches(bead, now))
                .collect();
            let since = matched
                .iter()
                .find_map(|s| s.since.as_ref().and_then(|src| src.text(bead)));
            let first = |sources: &[Source]| {
                sources
                    .iter()
                    .map(|s| s.texts(bead))
                    .find(|t| !t.is_empty())
                    .unwrap_or_default()
            };
            let mut row = RowFacts {
                class,
                kind,
                states: matched.iter().map(|s| s.key.clone()).collect(),
                since,
                ..Default::default()
            };
            for group in &profile.groups {
                let values = first(&group.from);
                if values.is_empty() {
                    continue;
                }
                if let Some(sub) = group.subgroup.as_ref().and_then(|s| s.text(bead)) {
                    row.subgroups.insert(group.key.clone(), sub);
                }
                row.groups.insert(group.key.clone(), values);
            }
            for role in &profile.people.roles {
                if let Some(name) = first(&role.from).into_iter().next() {
                    row.people
                        .insert(role.key.clone(), profile.person(&name).name);
                }
            }
            if class == RowClass::Receipt {
                row.receipt = profile
                    .receipts
                    .iter()
                    .enumerate()
                    .find(|(_, r)| r.when.matches(bead, now))
                    .map(|(n, r)| ReceiptFacts {
                        rule: n,
                        kind: r.kind.clone(),
                        subject: r
                            .subject
                            .iter()
                            .find_map(|s| s.bead(bead, snapshot))
                            .and_then(|b| snapshot.position(&b.id)),
                        attempt: r.attempt.as_ref().and_then(|s| s.text(bead)),
                        at: r.at.as_ref().and_then(|s| s.text(bead)),
                        actor: r.actor.as_ref().and_then(|s| s.text(bead)),
                    });
            }
            if let Some(receipt) = &row.receipt {
                if let Some(subject) = receipt.subject {
                    out.receipts_by_subject.entry(subject).or_default().push(i);
                }
                if let Some(attempt) = &receipt.attempt {
                    out.receipts_by_attempt
                        .entry(attempt.clone())
                        .or_default()
                        .push(i);
                }
            }
            if class == RowClass::Item {
                for (key, values) in &row.groups {
                    for value in values {
                        out.groups
                            .entry(key.clone())
                            .or_default()
                            .entry(value.clone())
                            .or_default()
                            .push(i);
                    }
                }
                let mut named: Vec<&String> = row.people.values().collect();
                named.sort();
                named.dedup();
                for name in named {
                    out.people.entry(name.clone()).or_default().push(i);
                }
            }
            out.rows.push(row);
        }
        out
    }

    /// Whether bead `index` is a work item (not a receipt or hidden row).
    pub fn is_item(&self, index: usize) -> bool {
        self.rows
            .get(index)
            .is_none_or(|r| r.class == RowClass::Item)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_fixture_derives_held_groups_people_and_receipt_joins() {
        let view = crate::testing::work_view();
        let s = &view.beads.as_ref().unwrap().data;
        let d = &s.derived;
        let row = |r: &str| &d.rows[s.position(&s.find(r).unwrap().id).unwrap()];
        let held = row("WORK-0003");
        assert_eq!(held.states, ["held"]);
        assert_eq!(held.since.as_deref(), Some("2026-11-15T09:15:00Z"));
        assert!(
            !s.find("WORK-0003").unwrap().ready,
            "held overrides bd ready"
        );
        assert!(!s.ready.contains(&s.find("WORK-0003").unwrap().id));
        assert_eq!(row("WORK-0001").kind.as_deref(), Some("packet"));
        assert_eq!(row("WORK-0004").kind.as_deref(), Some("gate"));
        assert_eq!(row("Q-0001").states, ["overdue"]);
        assert_eq!(row("WORK-0001").groups["milestone"], ["v1"]);
        assert_eq!(row("WORK-0001").subgroups["milestone"], "build");
        assert_eq!(row("WORK-0001").groups["rig"], ["rigA"]);
        assert_eq!(row("WORK-0003").groups["rig"], ["rigB"], "second source");
        assert_eq!(
            row("WORK-0001").people["claimant"],
            "factory/hostA/rigA",
            "a seat keeps its three segments"
        );
        assert_eq!(row("Q-0001").people["asked"], "bob");
        assert!(d.people.contains_key("bob"));
        assert_eq!(view.profile.person("factory/hostA/rigA").class, "seat");
        // Receipts join by external ref, by bead id and by attempt.
        let wk2 = s.position(&s.find("WORK-0002").unwrap().id).unwrap();
        let kinds: Vec<&str> = d.receipts_by_subject[&wk2]
            .iter()
            .map(|&i| d.rows[i].receipt.as_ref().unwrap().kind.as_str())
            .collect();
        assert_eq!(kinds, ["gate", "ship", "operation"]);
        let wk1 = s.position(&s.find("WORK-0001").unwrap().id).unwrap();
        assert_eq!(d.receipts_by_subject[&wk1].len(), 1, "joined by bead id");
        assert_eq!(d.receipts_by_attempt["att-0002-2"].len(), 2);
        // Receipts and hidden rows are not items.
        for id in ["wk-rc-gate", "wk-op-1", "wk-coordination", "wk-slot"] {
            let i = s.position(id).unwrap();
            assert!(!d.is_item(i), "{id}");
        }
        assert!(!d.groups["milestone"]["v1"].iter().any(|&i| !d.is_item(i)));
        assert_eq!(d.rows.len(), s.beads.len());
    }
}
