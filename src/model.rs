//! The view every page renders: the last good read of each store, the
//! profile, and what background reads have gathered.

use crate::profile::{Profile, RowFacts};
use crate::{beads, history, provenance, runs, scrolls};
use serde::Deserialize;
use std::sync::Arc;

/// The last successful read of one store, and the error from the most recent
/// read if it failed. A failed read never replaces the last good data.
#[derive(Clone, Debug, Default)]
pub struct Feed<T> {
    pub data: Arc<T>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    /// A name for the stores on show, from `--title`.
    pub title: String,
    pub beads: Option<Feed<beads::Snapshot>>,
    pub scrolls: Option<Feed<scrolls::Snapshot>>,
    pub scrolls_checked_at: Option<String>,
    /// Provenance logs read so far, when a local `bd` can read them.
    pub provenance: Option<provenance::Store>,
    /// Bumped on changed content or health; open pages reload when it changes.
    pub revision: u64,
    /// The profile in use (the generic one without `--profile`).
    pub profile: Arc<Profile>,
    /// History threads read so far, when a Beads store is shown.
    pub history: Option<history::Store>,
    /// The run feed index, when `--runs` is given.
    pub runs: Option<Feed<runs::Index>>,
    /// Run docs and document pages, fetched on demand.
    pub run_cache: Arc<runs::cache::Cache>,
}

impl View {
    /// Every store's last read error, for the alert banner and /healthz.
    pub fn errors(&self) -> Vec<&str> {
        let beads = self.beads.as_ref().and_then(|f| f.error.as_deref());
        let scrolls = self.scrolls.as_ref().and_then(|f| f.error.as_deref());
        let runs = self.runs.as_ref().and_then(|f| f.error.as_deref());
        beads.into_iter().chain(scrolls).chain(runs).collect()
    }

    /// The Beads snapshot, when a Beads store is shown.
    pub fn snapshot(&self) -> Option<&beads::Snapshot> {
        self.beads.as_ref().map(|f| &*f.data)
    }

    /// The profile's facts about bead `index` of the snapshot.
    pub fn row(&self, index: usize) -> Option<&RowFacts> {
        self.snapshot()?.derived.rows.get(index)
    }

    /// The profile's facts about a bead, by id.
    pub fn facts(&self, id: &str) -> Option<&RowFacts> {
        let s = self.snapshot()?;
        s.derived.rows.get(s.position(id)?)
    }

    /// Group key → value → bead indexes (items only).
    pub fn group_index(
        &self,
    ) -> Option<&std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<usize>>>>
    {
        self.snapshot().map(|s| &s.derived.groups)
    }

    /// A bead's history thread, once read.
    pub fn thread(&self, id: &str) -> Option<&history::Thread> {
        self.history.as_ref()?.get(id)
    }

    /// A bead's provenance trail, once read (and when a local `bd` can).
    pub fn trail(&self, id: &str) -> Option<&provenance::Trail> {
        self.provenance.as_ref()?.get(id)
    }

    /// Who holds `role` for a bead: its bead-level value, else the actor of
    /// the first history event an `event:<kind>` source names.
    pub fn person_in_role(&self, id: &str, role: &str) -> Option<String> {
        if let Some(name) = self.facts(id).and_then(|r| r.people.get(role)) {
            return Some(name.clone());
        }
        let thread = self.thread(id)?;
        self.profile
            .role(role)?
            .from
            .iter()
            .find_map(|source| match source {
                crate::profile::Source::Event(kind) => {
                    thread.actor(kind).map(|a| self.profile.person(a).name)
                }
                _ => None,
            })
    }

    /// The run index, when one has been read.
    pub fn run_index(&self) -> Option<&runs::Index> {
        self.runs.as_ref().map(|f| &*f.data)
    }

    /// Runs whose subject is this bead (by external ref or id), newest
    /// first.
    pub fn runs_for(&self, bead: &beads::Bead) -> Vec<&runs::RunSummary> {
        self.run_index()
            .map(|i| i.for_bead(bead))
            .unwrap_or_default()
    }

    /// Runs for a subject ref (or subject id).
    pub fn runs_for_subject(&self, key: &str) -> Vec<&runs::RunSummary> {
        self.run_index()
            .map(|i| i.by_subject(key))
            .unwrap_or_default()
    }

    /// The run an attempt id belongs to.
    pub fn run_for_attempt(&self, attempt: &str) -> Option<&runs::RunSummary> {
        self.run_index()?.by_attempt(attempt)
    }
}

fn contains(haystack: &[&str], needle: &str) -> bool {
    let needle = needle.to_lowercase();
    haystack.iter().any(|h| h.to_lowercase().contains(&needle))
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct ScrollFilter {
    pub q: String,
    pub kind: String,
    pub status: String,
    pub page: usize,
}

impl ScrollFilter {
    pub fn matches(&self, scroll: &scrolls::Scroll) -> bool {
        (self.kind.is_empty() || scroll.kind == self.kind)
            && (self.status.is_empty() || scroll.status == self.status)
            && (self.q.is_empty()
                || contains(
                    &[
                        &scroll.id,
                        &scroll.title,
                        scroll.path.as_deref().unwrap_or(""),
                        &scroll.aliases.join(" "),
                    ],
                    &self.q,
                ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_filter_matches_kind_status_and_text() {
        let s = scrolls::Scroll {
            id: "sc-1".into(),
            kind: "decision".into(),
            status: "accepted".into(),
            title: "Use Dolt".into(),
            aliases: vec!["ADR-7".into()],
            ..Default::default()
        };
        assert!(ScrollFilter::default().matches(&s));
        assert!(
            ScrollFilter {
                kind: "decision".into(),
                status: "accepted".into(),
                ..Default::default()
            }
            .matches(&s)
        );
        assert!(
            !ScrollFilter {
                kind: "note".into(),
                ..Default::default()
            }
            .matches(&s)
        );
        assert!(
            ScrollFilter {
                q: "adr-7".into(),
                ..Default::default()
            }
            .matches(&s)
        );
    }
}
