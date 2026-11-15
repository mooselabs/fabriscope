// Owned by WP6 — Held, costs, receipts, sweeper. Frozen signature:
// `pub fn spawn(app: App)`.
//! Reads provenance (and history) ahead for the beads the factory pages
//! total: held packets and recently closed ones, with at most two reads
//! outstanding, through `App::want_provenance` and
//! `history::reader::want`.
//!
//! Each round plans a queue once, in priority order: beads in a profile
//! state with `read_ahead` (held work), then beads in progress, then items
//! closed in the last 30 days, newest first. It then hands the queue to
//! `App::want_provenance` two at a time, never with more than two of its
//! own reads outstanding, so a bead page's own read always finds a free
//! permit (`App` runs four at once). A bead whose trail is still fresh
//! (`provenance::Store::wants_read`) is skipped. Between rounds it sleeps:
//! briefly when it found work, a minute when it did not. Its progress shows
//! on /healthz as the provenance `read` and `reading` counts.
//!
//! The task ends with the runtime when the server shuts down; reads it
//! started end with their `bd` process (`kill_on_drop`).

use crate::app::App;
use crate::history;
use crate::model::View;
use chrono::{DateTime, Duration, Utc};
use std::collections::{HashSet, VecDeque};

/// The most provenance reads the sweeper keeps outstanding.
pub(crate) const OWN_READS: usize = 2;
/// Closed items read ahead: those closed within this many days.
const RECENT_DAYS: i64 = 30;
/// How often a round checks whether its reads have finished.
const POLL: std::time::Duration = std::time::Duration::from_millis(250);
const AFTER_WORK: std::time::Duration = std::time::Duration::from_secs(5);
const WHEN_IDLE: std::time::Duration = std::time::Duration::from_secs(60);

/// Starts the sweeper. Called once at startup when a local `bd` can read
/// provenance.
pub fn spawn(app: App) {
    tokio::spawn(run(app));
}

async fn run(app: App) {
    let mut sweep = Sweep::default();
    loop {
        let ahead = {
            let view = app.view.read().await;
            if view.provenance.is_none() {
                return;
            }
            sweep.plan(&view, Utc::now())
        };
        if !ahead.is_empty() {
            history::reader::want(&app, ahead).await;
        }
        let worked = !sweep.queue.is_empty();
        while !sweep.queue.is_empty() {
            let ids = {
                let view = app.view.read().await;
                sweep.next(&view, Utc::now())
            };
            if !ids.is_empty() {
                app.want_provenance(ids).await;
            }
            if !sweep.queue.is_empty() {
                tokio::time::sleep(POLL).await;
            }
        }
        tokio::time::sleep(if worked { AFTER_WORK } else { WHEN_IDLE }).await;
    }
}

/// The sweeper's state between ticks: the round's queue and the reads it
/// started that have not finished.
#[derive(Debug, Default)]
pub(crate) struct Sweep {
    pub queue: VecDeque<String>,
    pub mine: HashSet<String>,
}

impl Sweep {
    /// Plans a round at `now`: queues every bead that wants a read, most
    /// wanted first, and returns the read-ahead beads (for their history).
    pub fn plan(&mut self, view: &View, now: DateTime<Utc>) -> Vec<String> {
        self.queue.clear();
        let (Some(s), Some(store)) = (view.snapshot(), view.provenance.as_ref()) else {
            return Vec::new();
        };
        let p = &view.profile;
        let read_ahead = |i: usize| {
            s.derived.rows.get(i).is_some_and(|r| {
                r.states
                    .iter()
                    .any(|k| p.state(k).is_some_and(|st| st.read_ahead))
            })
        };
        let since = now - Duration::days(RECENT_DAYS);
        let mut ahead = Vec::new();
        let mut working = Vec::new();
        let mut closed: Vec<(DateTime<Utc>, usize)> = Vec::new();
        for &i in &s.order {
            let bead = &s.beads[i];
            if read_ahead(i) {
                ahead.push(i);
            } else if bead.status == "in_progress" {
                working.push(i);
            } else if bead.is_closed() && s.derived.is_item(i) {
                let at = bead
                    .closed_at
                    .as_deref()
                    .and_then(|a| DateTime::parse_from_rfc3339(a).ok())
                    .map(|t| t.with_timezone(&Utc));
                if let Some(at) = at.filter(|at| *at >= since && *at <= now) {
                    closed.push((at, i));
                }
            }
        }
        // Newest first; the sort is stable, so ties keep display order.
        closed.sort_by_key(|a| std::cmp::Reverse(a.0));
        let order = ahead
            .iter()
            .chain(&working)
            .chain(closed.iter().map(|(_, i)| i));
        for &i in order {
            if store.wants_read(&s.beads[i], now) {
                self.queue.push_back(s.beads[i].id.clone());
            }
        }
        ahead.into_iter().map(|i| s.beads[i].id.clone()).collect()
    }

    /// The next beads to read: as many from the queue as keep the
    /// sweeper's own outstanding reads at `OWN_READS` or fewer, skipping
    /// any that no longer want a read. The caller starts them.
    pub fn next(&mut self, view: &View, now: DateTime<Utc>) -> Vec<String> {
        let (Some(s), Some(store)) = (view.snapshot(), view.provenance.as_ref()) else {
            self.queue.clear();
            return Vec::new();
        };
        // A read is finished once the store no longer lists it.
        self.mine.retain(|id| store.reading.contains(id));
        let mut out = Vec::new();
        while self.mine.len() < OWN_READS {
            let Some(id) = self.queue.pop_front() else {
                break;
            };
            if s.get(&id).is_some_and(|bead| store.wants_read(bead, now)) {
                self.mine.insert(id.clone());
                out.push(id);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::Store;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// The work fixture with nothing read yet.
    fn fresh() -> View {
        let mut view = crate::testing::work_view();
        view.provenance = Some(Store::default());
        view
    }

    fn store(view: &mut View) -> &mut Store {
        view.provenance.as_mut().unwrap()
    }

    #[test]
    fn held_beads_come_first_then_work_in_progress_then_recently_closed_items() {
        let view = fresh();
        let mut sweep = Sweep::default();
        let ahead = sweep.plan(&view, at("2026-11-20T12:00:00Z"));
        // wk-3 is held, wk-1 is working (both `read_ahead`), wk-2 closed six
        // days ago; wk-5 closed in October, and receipts are not items.
        assert_eq!(ahead, ["wk-1", "wk-3"]);
        let queued: Vec<&str> = sweep.queue.iter().map(String::as_str).collect();
        assert_eq!(queued, ["wk-1", "wk-3", "wk-2"]);
        sweep.plan(&view, at("2026-10-10T00:00:00Z"));
        assert!(
            sweep.queue.contains(&"wk-5".to_string()),
            "closed within 30 days of then"
        );
        assert!(!sweep.queue.contains(&"wk-2".to_string()), "not closed yet");
        // No provenance store: nothing to do.
        let mut bare = fresh();
        bare.provenance = None;
        assert!(sweep.plan(&bare, at("2026-11-20T12:00:00Z")).is_empty());
        assert!(sweep.queue.is_empty());
    }

    #[test]
    fn never_more_than_two_of_its_own_reads_outstanding_and_fresh_trails_wait() {
        let mut view = fresh();
        let mut now = at("2026-11-20T12:00:00Z");
        let mut sweep = Sweep::default();
        sweep.plan(&view, now);
        // Someone else's read (a bead page) is under way too.
        store(&mut view).reading.insert("wk-4".into());
        let mut started = Vec::new();
        let mut ticks = 0;
        while !sweep.queue.is_empty() || !sweep.mine.is_empty() {
            ticks += 1;
            assert!(ticks < 20);
            let ids = sweep.next(&view, now);
            // What `App::want_provenance` does for each id.
            store(&mut view).reading.extend(ids.iter().cloned());
            started.extend(ids);
            let own = store(&mut view)
                .reading
                .iter()
                .filter(|id| sweep.mine.contains(*id))
                .count();
            assert!(own <= OWN_READS, "{own} outstanding");
            assert!(sweep.mine.len() <= OWN_READS);
            // One read finishes per tick, a second later.
            now += Duration::seconds(1);
            let done = sweep.mine.iter().min().cloned();
            if let Some(id) = done {
                let updated = view
                    .snapshot()
                    .unwrap()
                    .get(&id)
                    .unwrap()
                    .updated_at
                    .clone();
                store(&mut view).record(&id, Ok(Vec::new()), updated, now);
            }
        }
        assert_eq!(started, ["wk-1", "wk-3", "wk-2"]);
        assert!(
            store(&mut view).reading.contains("wk-4"),
            "not the sweeper's"
        );
        // Ten minutes on, only the open beads' trails (two-minute window)
        // are stale; the closed one (thirty minutes) is still fresh.
        now += Duration::minutes(10);
        sweep.plan(&view, now);
        let queued: Vec<&str> = sweep.queue.iter().map(String::as_str).collect();
        assert_eq!(queued, ["wk-1", "wk-3"]);
        now += Duration::minutes(30);
        sweep.plan(&view, now);
        assert_eq!(sweep.queue.len(), 3);
        // A bead already being read is skipped when its turn comes.
        store(&mut view).reading.insert("wk-1".into());
        assert_eq!(sweep.next(&view, now), ["wk-3", "wk-2"]);
    }
}
