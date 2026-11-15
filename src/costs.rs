// Owned by WP6 — Held, costs, receipts, sweeper. No frozen signature: WP6
// defines this module.
//! Cost totals from provenance usage (`used` events) and the run feed
//! (`runs::Cost`), de-duplicated by attempt id with the feed winning, plus
//! the coverage line that says how much of the work they cover.
//!
//! Two sources measure the same spend. The run feed reports a cost per run
//! (and per attempt and model when the run doc is read); provenance records
//! `used` events per attempt. When both have a dollar figure for an attempt,
//! the feed's figure counts and the provenance one is set aside as
//! `replaced`, so an attempt is counted once. An attempt joins a run by its
//! attempt id; an attempt the feed does not name joins a run of the same
//! item whose time window contains it, but only while that run has attempts
//! the feed has not named (`FeedRun::attempts` above the ids known).
//!
//! A value nobody recorded is `None` and is shown as unknown, never as 0.

use crate::model::View;
pub use crate::provenance::Spend;
use crate::provenance::{Event, Progress, attempt_events, plus};
use crate::query::Range;
use crate::runs::cache::Fetch;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Where a figure came from. The two are never mixed in one total without
/// saying so.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    Feed,
    Provenance,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Feed => "run feed",
            Source::Provenance => "provenance",
        }
    }
}

/// One attempt's spend as provenance recorded it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProvAttempt {
    /// The bead's index in the snapshot.
    pub item: usize,
    /// The attempt id (`provenance::Attempt::id`), when its events carry one.
    pub attempt: Option<String>,
    /// When the attempt began (its first event).
    pub started_at: Option<String>,
    /// When it last recorded usage: the time its spend counts at.
    pub at: Option<String>,
    pub spend: Spend,
    /// Dollars by the `model` a `used` event names.
    pub by_model: BTreeMap<String, f64>,
}

/// One run's spend as the run feed reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeedRun {
    pub run: String,
    /// The subject bead's index, when the subject is in the ledger.
    pub item: Option<usize>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    /// The time its spend counts at: when it ended, else its last update.
    pub at: Option<String>,
    /// How many attempts the feed says the run had.
    pub attempts: u32,
    /// The attempt ids known to belong to the run.
    pub attempt_ids: Vec<String>,
    pub spend: Spend,
    pub by_model: BTreeMap<String, f64>,
    pub by_role: BTreeMap<String, f64>,
    /// False when the feed says known usage is missing (a lower bound).
    pub complete: bool,
}

/// One counted figure: a run from the feed, or an attempt only provenance
/// knows.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub source: Source,
    pub item: Option<usize>,
    pub run: Option<String>,
    /// Attempt ids the figure covers, where known.
    pub attempt_ids: Vec<String>,
    /// How many attempts it covers.
    pub attempts: usize,
    pub at: Option<String>,
    pub spend: Spend,
    pub by_model: BTreeMap<String, f64>,
    pub by_role: BTreeMap<String, f64>,
    pub complete: bool,
}

/// Both sources joined: every figure counted once, the provenance figures
/// the feed replaced, and feed runs whose cost nobody reported.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Combined {
    pub figures: Vec<Figure>,
    pub replaced: Vec<ProvAttempt>,
    pub unknown_runs: Vec<FeedRun>,
}

fn instant(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Whether `at` falls in a run's window (an open end runs to now).
fn within(at: Option<&str>, run: &FeedRun) -> bool {
    let (Some(at), Some(start)) = (
        at.and_then(instant),
        run.started_at.as_deref().and_then(instant),
    ) else {
        return false;
    };
    at >= start
        && run
            .ended_at
            .as_deref()
            .and_then(instant)
            .is_none_or(|end| at <= end)
}

/// Joins the two sources so each attempt is counted once, the feed winning.
pub fn combine(prov: Vec<ProvAttempt>, feed: Vec<FeedRun>) -> Combined {
    let mut out = Combined::default();
    // Every attempt id the feed names, priced or not.
    let named: BTreeSet<String> = feed
        .iter()
        .flat_map(|r| r.attempt_ids.iter().cloned())
        .collect();
    let mut priced: Vec<FeedRun> = Vec::new();
    for run in feed {
        if run.spend.dollars.is_some() {
            priced.push(run);
        } else {
            out.unknown_runs.push(run);
        }
    }
    let by_attempt: HashMap<String, usize> = priced
        .iter()
        .enumerate()
        .flat_map(|(i, r)| r.attempt_ids.iter().map(move |a| (a.clone(), i)))
        .collect();
    let mut claimed: Vec<Vec<ProvAttempt>> = vec![Vec::new(); priced.len()];
    let mut extra_ids: Vec<Vec<String>> = vec![Vec::new(); priced.len()];
    for attempt in prov {
        let target = match &attempt.attempt {
            Some(id) if by_attempt.contains_key(id) => Some(by_attempt[id]),
            // Named by the feed but without a price: provenance counts.
            Some(id) if named.contains(id) => None,
            _ => priced.iter().enumerate().position(|(i, run)| {
                run.item == Some(attempt.item)
                    && run.attempt_ids.len() + extra_ids[i].len() < run.attempts as usize
                    && within(attempt.started_at.as_deref().or(attempt.at.as_deref()), run)
            }),
        };
        match target {
            Some(i) => {
                if let Some(id) = &attempt.attempt {
                    if !priced[i].attempt_ids.contains(id) {
                        extra_ids[i].push(id.clone());
                    }
                }
                claimed[i].push(attempt);
            }
            None => out.figures.push(Figure {
                source: Source::Provenance,
                item: Some(attempt.item),
                run: None,
                attempt_ids: attempt.attempt.iter().cloned().collect(),
                attempts: 1,
                at: attempt.at.clone(),
                spend: attempt.spend.clone(),
                by_model: attempt.by_model.clone(),
                by_role: BTreeMap::new(),
                complete: true,
            }),
        }
    }
    for ((run, replaced), extra) in priced.into_iter().zip(claimed).zip(extra_ids) {
        // The feed's figures win; what it lacks, the attempts it replaced
        // may know.
        let mut fallback = Spend::default();
        let mut models = BTreeMap::new();
        for attempt in &replaced {
            fallback.add(&attempt.spend);
            for (model, usd) in &attempt.by_model {
                *models.entry(model.clone()).or_insert(0.0) += usd;
            }
        }
        let mut spend = run.spend.clone();
        spend.or(&fallback);
        let mut attempt_ids = run.attempt_ids.clone();
        attempt_ids.extend(extra);
        out.figures.push(Figure {
            source: Source::Feed,
            item: run.item,
            attempts: (run.attempts as usize).max(attempt_ids.len()).max(1),
            attempt_ids,
            run: Some(run.run.clone()),
            at: run.at.clone(),
            spend,
            by_model: if run.by_model.is_empty() {
                models
            } else {
                run.by_model.clone()
            },
            by_role: run.by_role.clone(),
            complete: run.complete,
        });
        out.replaced.extend(replaced);
    }
    out
}

/// A total over some figures.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Total {
    pub spend: Spend,
    pub figures: usize,
    pub attempts: usize,
    /// Some figure is a lower bound (the feed said usage is missing).
    pub partial: bool,
}

impl Total {
    pub fn add(&mut self, figure: &Figure) {
        self.spend.add(&figure.spend);
        self.figures += 1;
        self.attempts += figure.attempts;
        self.partial |= !figure.complete;
    }
}

/// One item's figures in range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemRow {
    pub item: Option<usize>,
    pub total: Total,
    pub sources: BTreeSet<Source>,
    pub runs: Vec<String>,
}

/// How much of the closed work has a known cost: `known` of `closed` items
/// (closed in range) have a dollar figure, over `attempts` attempts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub known: usize,
    pub closed: usize,
    pub attempts: usize,
}

/// Everything the costs page shows, for the figures it keeps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// Figures in range, each counted once.
    pub figures: Vec<Figure>,
    pub combined: Total,
    pub feed: Total,
    pub provenance: Total,
    /// Provenance figures in range that a feed figure replaced.
    pub replaced: Total,
    /// Feed runs in range with no reported cost.
    pub unknown_runs: usize,
    /// One row per item (or `None` for runs whose subject is not in the
    /// ledger), largest spend first.
    pub items: Vec<ItemRow>,
    pub coverage: Coverage,
}

/// Keeps the figures `keep` accepts whose time is in `range` at `now`, and
/// measures coverage over `closed` (the items closed in range that pass the
/// page's filters). Coverage counts an item's figures whatever their time:
/// an item's cost is known or not.
pub fn report(
    combined: &Combined,
    keep: impl Fn(Option<usize>) -> bool,
    range: Range,
    now: DateTime<Utc>,
    closed: &[usize],
) -> Report {
    let in_range = |at: Option<&str>| at.is_some_and(|at| range.contains(at, now));
    let unbounded = range.bounds(now) == (None, None);
    let mut out = Report::default();
    let mut rows: BTreeMap<Option<usize>, ItemRow> = BTreeMap::new();
    for figure in &combined.figures {
        if !keep(figure.item) || !(unbounded || in_range(figure.at.as_deref())) {
            continue;
        }
        out.combined.add(figure);
        match figure.source {
            Source::Feed => out.feed.add(figure),
            Source::Provenance => out.provenance.add(figure),
        }
        let row = rows.entry(figure.item).or_insert_with(|| ItemRow {
            item: figure.item,
            ..Default::default()
        });
        row.total.add(figure);
        row.sources.insert(figure.source);
        row.runs.extend(figure.run.clone());
        out.figures.push(figure.clone());
    }
    for attempt in &combined.replaced {
        if keep(Some(attempt.item)) && (unbounded || in_range(attempt.at.as_deref())) {
            out.replaced.spend.add(&attempt.spend);
            out.replaced.figures += 1;
            out.replaced.attempts += 1;
        }
    }
    out.unknown_runs = combined
        .unknown_runs
        .iter()
        .filter(|r| keep(r.item) && (unbounded || in_range(r.at.as_deref())))
        .count();
    let mut known: HashMap<usize, usize> = HashMap::new();
    for figure in &combined.figures {
        if let (Some(item), Some(_)) = (figure.item, figure.spend.dollars) {
            *known.entry(item).or_default() += figure.attempts;
        }
    }
    out.coverage.closed = closed.len();
    for item in closed {
        if let Some(attempts) = known.get(item) {
            out.coverage.known += 1;
            out.coverage.attempts += attempts;
        }
    }
    out.items = rows.into_values().collect();
    out.items.sort_by(|a, b| {
        b.total
            .spend
            .dollars
            .unwrap_or(-1.0)
            .total_cmp(&a.total.spend.dollars.unwrap_or(-1.0))
            .then(a.item.cmp(&b.item))
    });
    out
}

/// One row of a breakdown: a value, its dollars (unknown when no figure
/// had any), and how many items and attempts it covers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Share {
    pub key: String,
    pub dollars: Option<f64>,
    pub items: usize,
    pub attempts: usize,
}

/// Dollars by whatever `keys` says each figure belongs to (a figure in
/// several groups counts in each), largest first.
pub fn breakdown(figures: &[Figure], keys: impl Fn(&Figure) -> Vec<String>) -> Vec<Share> {
    let mut shares: BTreeMap<String, (Share, BTreeSet<Option<usize>>)> = BTreeMap::new();
    for figure in figures {
        for key in keys(figure) {
            let (share, items) = shares.entry(key.clone()).or_insert_with(|| {
                (
                    Share {
                        key,
                        ..Default::default()
                    },
                    BTreeSet::new(),
                )
            });
            share.dollars = plus(share.dollars, figure.spend.dollars);
            share.attempts += figure.attempts;
            items.insert(figure.item);
        }
    }
    sorted(shares)
}

fn sorted(shares: BTreeMap<String, (Share, BTreeSet<Option<usize>>)>) -> Vec<Share> {
    let mut out: Vec<Share> = shares
        .into_values()
        .map(|(mut share, items)| {
            share.items = items.len();
            share
        })
        .collect();
    out.sort_by(|a, b| {
        b.dollars
            .unwrap_or(-1.0)
            .total_cmp(&a.dollars.unwrap_or(-1.0))
            .then(a.key.cmp(&b.key))
    });
    out
}

/// What no model or role accounts for in a figure, under this name.
pub const UNATTRIBUTED: &str = "not attributed";

/// Dollars by model (or by role, with `roles`): each figure's own split,
/// and whatever its split leaves of its total under `UNATTRIBUTED`.
pub fn split(figures: &[Figure], roles: bool) -> Vec<Share> {
    let mut shares: BTreeMap<String, (Share, BTreeSet<Option<usize>>)> = BTreeMap::new();
    for figure in figures {
        let parts = if roles {
            &figure.by_role
        } else {
            &figure.by_model
        };
        let mut rest = figure.spend.dollars;
        let mut add = |key: &str, usd: Option<f64>| {
            let (share, items) = shares.entry(key.to_string()).or_insert_with(|| {
                (
                    Share {
                        key: key.to_string(),
                        ..Default::default()
                    },
                    BTreeSet::new(),
                )
            });
            share.dollars = plus(share.dollars, usd);
            share.attempts += figure.attempts;
            items.insert(figure.item);
        };
        for (key, usd) in parts {
            add(key, Some(*usd));
            rest = rest.map(|r| r - usd);
        }
        // Cents of rounding are not a share of their own.
        if parts.is_empty() || rest.is_some_and(|r| r.abs() >= 0.005) {
            add(UNATTRIBUTED, rest);
        }
    }
    sorted(shares)
}

/// Each read provenance trail's attempts, with the time and the spend of
/// their `used` events. Attempt ids are the ones `provenance::Progress`
/// gives; attempts split where `Progress` splits them (a second `claim`).
pub fn provenance_attempts(view: &View) -> Vec<ProvAttempt> {
    let (Some(s), Some(store)) = (view.snapshot(), view.provenance.as_ref()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (id, trail) in &store.trails {
        let Some(item) = s.position(id).filter(|&i| s.derived.is_item(i)) else {
            continue;
        };
        if !trail.events.iter().any(|e| e.kind == "used") {
            continue;
        }
        let progress = Progress::new(&s.beads[item], &trail.events);
        let mut ids: Vec<Option<String>> = progress.earlier.iter().map(|a| a.id.clone()).collect();
        let segments = attempt_events(&trail.events);
        if segments.len() > ids.len() {
            ids.push(progress.attempt.clone());
        }
        for (n, events) in segments.iter().enumerate() {
            let used: Vec<&&Event> = events.iter().filter(|e| e.kind == "used").collect();
            if used.is_empty() {
                continue;
            }
            let mut attempt = ProvAttempt {
                item,
                attempt: ids.get(n).cloned().flatten(),
                started_at: events.first().map(|e| e.occurred_at.clone()),
                at: used.last().map(|e| e.occurred_at.clone()),
                ..Default::default()
            };
            for event in used {
                let spend = Spend::of_used(event);
                if let (Some(usd), Some(model)) = (
                    spend.dollars,
                    event
                        .fields()
                        .into_iter()
                        .find(|(k, _)| k == "model")
                        .and_then(|(_, v)| v.as_str().map(String::from)),
                ) {
                    *attempt.by_model.entry(model).or_insert(0.0) += usd;
                }
                attempt.spend.add(&spend);
            }
            out.push(attempt);
        }
    }
    out
}

/// Every run of the feed with its reported cost. For runs whose time is in
/// `range`, a run doc already held (`Cache::peek_run`; never fetched here,
/// so a wide range cannot read the whole feed) adds its attempt ids, model
/// split and completeness; `attempts` adds the run every provenance attempt
/// id joins through `View::run_for_attempt`. A run whose doc is not held
/// joins its unnamed attempts by time window (see the module comment).
pub fn feed_runs(
    view: &View,
    attempts: &[ProvAttempt],
    range: Range,
    now: DateTime<Utc>,
) -> Vec<FeedRun> {
    let Some(index) = view.run_index() else {
        return Vec::new();
    };
    let s = view.snapshot();
    let mut joined: HashMap<&str, Vec<String>> = HashMap::new();
    for attempt in attempts {
        if let Some(id) = &attempt.attempt {
            if let Some(run) = view.run_for_attempt(id) {
                joined.entry(run.run.as_str()).or_default().push(id.clone());
            }
        }
    }
    let unbounded = range.bounds(now) == (None, None);
    index
        .runs
        .iter()
        .map(|summary| {
            let at = summary
                .ended_at
                .clone()
                .or_else(|| Some(summary.updated_at.clone()).filter(|u| !u.is_empty()));
            let item = s.and_then(|s| {
                summary
                    .subject
                    .reference
                    .iter()
                    .chain(&summary.subject.id)
                    .find_map(|key| s.find(key))
                    .and_then(|b| s.position(&b.id))
            });
            let mut run = FeedRun {
                run: summary.run.clone(),
                item,
                started_at: summary.started_at.clone(),
                ended_at: summary.ended_at.clone(),
                attempts: summary.attempts,
                attempt_ids: joined.remove(summary.run.as_str()).unwrap_or_default(),
                spend: Spend {
                    dollars: summary.cost_usd,
                    ..Default::default()
                },
                complete: true,
                at,
                ..Default::default()
            };
            let wanted = unbounded || run.at.as_deref().is_some_and(|a| range.contains(a, now));
            if wanted {
                if let Fetch::Ready(doc) = view.run_cache.peek_run(&summary.run) {
                    for attempt in &doc.attempts {
                        if !attempt.id.is_empty() && !run.attempt_ids.contains(&attempt.id) {
                            run.attempt_ids.push(attempt.id.clone());
                        }
                    }
                    run.attempts = run.attempts.max(doc.attempts.len() as u32);
                    if let Some(cost) = &doc.costs {
                        run.spend.dollars = cost.usd.or(run.spend.dollars);
                        run.complete = cost.complete;
                        run.spend.tokens =
                            cost.tokens.as_ref().and_then(|t| plus(t.input, t.output));
                        run.by_model = cost
                            .by_model
                            .iter()
                            .filter_map(|(m, c)| Some((m.clone(), c.usd?)))
                            .collect();
                        run.by_role = cost
                            .by_role
                            .iter()
                            .filter_map(|(r, c)| Some((r.clone(), c.usd?)))
                            .collect();
                    }
                    run.spend.wall_seconds = doc
                        .timing
                        .as_ref()
                        .and_then(|t| t.wall_hours)
                        .map(|h| h * 3600.0);
                    let gates: Vec<f64> = doc
                        .attempts
                        .iter()
                        .flat_map(|a| &a.steps)
                        .filter(|step| step.kind == "gate")
                        .filter_map(|step| step.seconds)
                        .collect();
                    if !gates.is_empty() {
                        run.spend.gate_runs = Some(gates.len() as u64);
                        run.spend.gate_seconds = Some(gates.iter().sum());
                    }
                }
            }
            run
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(at: &str) -> Option<String> {
        Some(at.to_string())
    }

    fn usd(dollars: f64) -> Spend {
        Spend {
            dollars: Some(dollars),
            ..Default::default()
        }
    }

    fn prov(item: usize, id: Option<&str>, at: &str, dollars: f64) -> ProvAttempt {
        ProvAttempt {
            item,
            attempt: id.map(String::from),
            started_at: t(at),
            at: t(at),
            spend: Spend {
                dollars: Some(dollars),
                tokens: Some(100),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn run(name: &str, item: usize, ids: &[&str], attempts: u32, dollars: Option<f64>) -> FeedRun {
        FeedRun {
            run: name.into(),
            item: Some(item),
            started_at: t("2026-11-14T08:00:00Z"),
            ended_at: t("2026-11-14T16:00:00Z"),
            at: t("2026-11-14T16:00:00Z"),
            attempts,
            attempt_ids: ids.iter().map(|s| s.to_string()).collect(),
            spend: Spend {
                dollars,
                ..Default::default()
            },
            complete: true,
            ..Default::default()
        }
    }

    fn now() -> DateTime<Utc> {
        instant("2026-11-15T12:00:00Z").unwrap()
    }

    #[test]
    fn an_attempt_both_sources_price_counts_once_and_the_feed_wins() {
        let combined = combine(
            vec![
                prov(1, Some("a-1"), "2026-11-14T10:00:00Z", 2.0),
                // Not named by the feed, but inside the run's window while
                // the run has an attempt the feed did not name.
                prov(1, Some("a-2"), "2026-11-14T12:00:00Z", 3.0),
                // Provenance only: its own figure.
                prov(2, Some("b-1"), "2026-11-14T12:00:00Z", 1.0),
            ],
            vec![run("r-1", 1, &["a-1"], 2, Some(5.5))],
        );
        assert_eq!(combined.replaced.len(), 2);
        assert_eq!(combined.figures.len(), 2);
        let feed = combined
            .figures
            .iter()
            .find(|f| f.source == Source::Feed)
            .unwrap();
        assert_eq!(feed.spend.dollars, Some(5.5), "the feed's dollars win");
        assert_eq!(
            feed.spend.tokens,
            Some(200),
            "what it lacks, provenance gives"
        );
        assert_eq!(feed.attempts, 2);
        assert_eq!(feed.attempt_ids, ["a-1", "a-2"]);
        let r = report(&combined, |_| true, Range::default(), now(), &[1, 2, 3]);
        assert_eq!(r.combined.spend.dollars, Some(6.5));
        assert_eq!(r.combined.attempts, 3, "three attempts, each once");
        assert_eq!(r.feed.spend.dollars, Some(5.5));
        assert_eq!(r.provenance.spend.dollars, Some(1.0));
        assert_eq!(r.replaced.spend.dollars, Some(5.0));
        assert_eq!(
            r.coverage,
            Coverage {
                known: 2,
                closed: 3,
                attempts: 3
            }
        );
        assert_eq!(r.items[0].item, Some(1), "largest spend first");
    }

    #[test]
    fn a_run_without_a_price_leaves_its_attempts_to_provenance() {
        let combined = combine(
            vec![prov(1, Some("a-1"), "2026-11-14T10:00:00Z", 2.0)],
            vec![run("r-1", 1, &["a-1"], 1, None)],
        );
        assert_eq!(combined.unknown_runs.len(), 1);
        assert_eq!(combined.figures[0].source, Source::Provenance);
        // A full run (every attempt named) takes no stranger by time.
        let combined = combine(
            vec![prov(1, Some("z-9"), "2026-11-14T10:00:00Z", 2.0)],
            vec![run("r-1", 1, &["a-1"], 1, Some(4.0))],
        );
        assert_eq!(combined.figures.len(), 2);
        // Nor one outside its window, nor one of another item.
        let combined = combine(
            vec![
                prov(1, None, "2026-11-13T10:00:00Z", 2.0),
                prov(2, None, "2026-11-14T10:00:00Z", 2.0),
            ],
            vec![run("r-1", 1, &[], 2, Some(4.0))],
        );
        assert!(combined.replaced.is_empty());
    }

    #[test]
    fn missing_figures_stay_unknown_never_zero() {
        let mut a = Spend::default();
        a.add(&Spend::default());
        assert_eq!(a.dollars, None);
        a.add(&usd(0.0));
        assert_eq!(a.dollars, Some(0.0), "a recorded zero is a zero");
        let mut gate_only = prov(1, Some("a-1"), "2026-11-14T10:00:00Z", 0.0);
        gate_only.spend = Spend {
            gate_runs: Some(1),
            ..Default::default()
        };
        let combined = combine(vec![gate_only], vec![]);
        let r = report(&combined, |_| true, Range::default(), now(), &[1]);
        assert_eq!(r.combined.spend.dollars, None);
        assert_eq!(r.coverage.known, 0, "no dollar figure, no known cost");
        let shares = breakdown(&r.figures, |_| vec!["x".into()]);
        assert_eq!(shares[0].dollars, None);
        let event = Event {
            kind: "used".into(),
            payload: serde_json::Value::String("{\"tokens\":5}".into()),
            ..Default::default()
        };
        let spend = Spend::of_used(&event);
        assert_eq!((spend.dollars, spend.tokens), (None, Some(5)));
    }

    #[test]
    fn range_edges_are_whole_days_and_coverage_ignores_the_range() {
        let combined = combine(
            vec![
                prov(1, Some("a"), "2026-11-09T00:00:00Z", 1.0),
                prov(2, Some("b"), "2026-11-08T23:59:59Z", 2.0),
                prov(3, Some("c"), "2026-11-15T23:59:59Z", 4.0),
            ],
            vec![],
        );
        let week = Range {
            days: Some(7),
            ..Default::default()
        };
        let r = report(&combined, |_| true, week, now(), &[2]);
        assert_eq!(
            r.combined.spend.dollars,
            Some(5.0),
            "first and last day kept"
        );
        assert_eq!(r.coverage.known, 1, "item 2's cost is known though older");
        let none = report(&combined, |i| i == Some(9), week, now(), &[]);
        assert_eq!(none.combined, Total::default());
    }

    #[test]
    fn splits_by_model_keep_what_no_model_accounts_for() {
        let mut feed = run("r-1", 1, &[], 1, Some(5.0));
        feed.by_model = BTreeMap::from([("large".to_string(), 4.0)]);
        let combined = combine(
            vec![ProvAttempt {
                by_model: BTreeMap::from([("small".to_string(), 1.0)]),
                ..prov(2, None, "2026-11-10T00:00:00Z", 1.0)
            }],
            vec![feed],
        );
        let shares = split(&combined.figures, false);
        let get = |k: &str| shares.iter().find(|s| s.key == k).unwrap().dollars;
        assert_eq!(get("large"), Some(4.0));
        assert_eq!(get("small"), Some(1.0));
        assert_eq!(get(UNATTRIBUTED), Some(1.0));
        let roles = split(&combined.figures, true);
        assert_eq!(roles.len(), 1, "no roles known: all unattributed");
        assert_eq!(roles[0].dollars, Some(6.0));
    }

    #[test]
    fn the_work_fixture_counts_wk2_attempts_once() {
        let view = crate::testing::work_view();
        let s = view.snapshot().unwrap();
        let prov = provenance_attempts(&view);
        let wk2 = s.position("wk-2").unwrap();
        let ids: Vec<_> = prov
            .iter()
            .filter(|a| a.item == wk2)
            .map(|a| a.attempt.clone().unwrap())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"att-0002-1".to_string()));
        let range = Range::default();
        let feed = feed_runs(&view, &prov, range, now());
        let combined = combine(prov, feed);
        let figures: Vec<&Figure> = combined
            .figures
            .iter()
            .filter(|f| f.item == Some(wk2))
            .collect();
        // run-0002 (both attempts) and run-0004 (its own attempt).
        assert_eq!(figures.len(), 2, "{figures:?}");
        let run2 = figures
            .iter()
            .find(|f| f.run.as_deref() == Some("run-0002"))
            .unwrap();
        assert_eq!(run2.spend.dollars, Some(5.37));
        assert_eq!(run2.attempts, 2);
        assert_eq!(run2.spend.tokens, Some(1_932_000));
        assert_eq!(run2.by_model.len(), 2, "the run doc splits by model");
        let r = report(&combined, |i| i == Some(wk2), range, now(), &[wk2]);
        assert!((r.combined.spend.dollars.unwrap() - 6.01).abs() < 1e-9);
        assert_eq!(r.combined.attempts, 3);
        assert_eq!(r.coverage.attempts, 3);
    }
}
