//! Test support: a `View` built from the neutral work fixture
//! (tests/fixtures/work/), its profile, and the golden run feed
//! (tests/fixtures/runfeed/v1/), all compiled in, so a unit test needs no
//! files or processes. See the fixture inventory in docs/profile.md.

use crate::beads::{self, Snapshot};
use crate::history::{self, Thread};
use crate::model::{Feed, View};
use crate::profile::Profile;
use crate::provenance;
use crate::runs::{self, Index};
use std::sync::Arc;

const ISSUES: &str = include_str!("../tests/fixtures/work/issues.jsonl");
const READY: &str = include_str!("../tests/fixtures/work/ready.json");
const PROFILE: &str = include_str!("../tests/fixtures/work/profile.toml");
const PROVENANCE: [(&str, &str); 3] = [
    (
        "wk-1",
        include_str!("../tests/fixtures/work/provenance-wk-1.json"),
    ),
    (
        "wk-2",
        include_str!("../tests/fixtures/work/provenance-wk-2.json"),
    ),
    (
        "wk-3",
        include_str!("../tests/fixtures/work/provenance-wk-3.json"),
    ),
];
const FEED: &str = include_str!("../tests/fixtures/runfeed/v1/feed.json");
const INDEXES: [&str; 2] = [
    include_str!("../tests/fixtures/runfeed/v1/index.json"),
    include_str!("../tests/fixtures/runfeed/v1/index/2026-10.json"),
];
const SUBJECTS: [&str; 4] = [
    include_str!("../tests/fixtures/runfeed/v1/subjects/WORK-0001.json"),
    include_str!("../tests/fixtures/runfeed/v1/subjects/WORK-0002.json"),
    include_str!("../tests/fixtures/runfeed/v1/subjects/WORK-0003.json"),
    include_str!("../tests/fixtures/runfeed/v1/subjects/WORK-0005.json"),
];
const RUNS: [&str; 5] = [
    include_str!("../tests/fixtures/runfeed/v1/runs/run-0001.json"),
    include_str!("../tests/fixtures/runfeed/v1/runs/run-0002.json"),
    include_str!("../tests/fixtures/runfeed/v1/runs/run-0003.json"),
    include_str!("../tests/fixtures/runfeed/v1/runs/run-0004.json"),
    include_str!("../tests/fixtures/runfeed/v1/runs/run-0005.json"),
];

/// The work fixture's profile.
pub fn work_profile() -> Profile {
    Profile::parse(PROFILE).unwrap()
}

/// A view of the work fixture as a running server would have it once every
/// background read finished: the snapshot built with the profile, the
/// provenance trails of wk-1, wk-2 and wk-3, every history thread (from the
/// export's comments), the golden run index with every attempt registered,
/// and every run doc in the run cache.
pub fn work_view() -> View {
    let profile = Arc::new(work_profile());
    let now = chrono::Utc::now();
    let mut snapshot = Snapshot::build(
        beads::parse_export(ISSUES).unwrap(),
        beads::parse_ready(READY).unwrap(),
        Some("fixture".into()),
        now,
        &profile,
    );
    snapshot.source = "bd export".into();
    let mut trails = provenance::Store::default();
    for (id, text) in PROVENANCE {
        trails.record(id, provenance::parse(text), None, now);
    }
    let mut threads = history::Store::default();
    for bead in &snapshot.beads {
        let thread = Thread::from_comments(&bead.comments, profile.events.as_ref());
        threads.record(&bead.id, Ok(thread), bead.updated_at.clone(), now);
    }
    let feed: runs::Feed = serde_json::from_str(FEED).unwrap();
    let listings = INDEXES
        .iter()
        .map(|t| serde_json::from_str(t).unwrap())
        .collect();
    let subjects: Vec<runs::SubjectFile> = SUBJECTS
        .iter()
        .map(|t| serde_json::from_str(t).unwrap())
        .collect();
    let docs: Vec<runs::Run> = RUNS
        .iter()
        .map(|t| serde_json::from_str(t).unwrap())
        .collect();
    let index = Index::build(feed, listings, &subjects).with_attempts(&docs);
    let cache = runs::cache::Cache::default();
    for doc in docs {
        let id = doc.run.clone();
        cache.put_run(&id, Ok(doc));
    }
    View {
        title: "Work".into(),
        beads: Some(Feed {
            data: Arc::new(snapshot),
            error: None,
        }),
        provenance: Some(trails),
        history: Some(threads),
        runs: Some(Feed {
            data: Arc::new(index),
            error: None,
        }),
        run_cache: Arc::new(cache),
        profile,
        revision: 1,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_work_view_joins_beads_runs_history_and_attempts() {
        let view = work_view();
        let s = view.snapshot().unwrap();
        let wk2 = s.find("WORK-0002").unwrap();
        let runs: Vec<&str> = view.runs_for(wk2).iter().map(|r| r.run.as_str()).collect();
        assert_eq!(runs, ["run-0002", "run-0004"]);
        let p = provenance::Progress::new(wk2, &view.trail("wk-2").unwrap().events);
        let first = p.earlier[0].id.as_deref().unwrap();
        assert_eq!(view.run_for_attempt(first).unwrap().run, "run-0002");
        assert_eq!(view.thread("wk-1").unwrap().events.len(), 2);
        assert_eq!(view.thread("wk-1").unwrap().comments.len(), 1);
        assert_eq!(
            view.person_in_role("wk-2", "creator").as_deref(),
            Some("alice")
        );
        assert_eq!(
            view.person_in_role("wk-1", "claimant").as_deref(),
            Some("factory/hostA/rigA")
        );
        assert!(matches!(
            view.run_cache.get_run("run-0003"),
            runs::cache::Fetch::Ready(_)
        ));
        assert_eq!(view.runs_for(s.find("WORK-0004").unwrap()).len(), 0);
        assert_eq!(view.runs_for_subject("WORK-0003")[0].run, "run-0003");
        let i = s.position("wk-3").unwrap();
        assert_eq!(view.row(i).unwrap().states, ["held"]);
        assert_eq!(view.group_index().unwrap()["area"].len(), 3);
        assert!(s.derived.people.contains_key("factory/hostA/rigA"));
    }
}
