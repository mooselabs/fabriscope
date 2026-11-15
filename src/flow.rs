// Owned by WP4 — Flow. No frozen signature: WP4 defines this module.
//! Flow measures over a snapshot (created, claimed, cycle time, ageing,
//! decisions): nearest-rank percentiles, and unknown claims counted apart
//! rather than shown as zero. Read `profile::FlowRule` for the sources.
//!
//! Everything here is a pure function of the `View`, the query and `now`;
//! `render::flow` only draws what these return. Times are UTC throughout and
//! a date range is whole UTC days.

use crate::beads::Bead;
use crate::model::View;
use crate::profile::{Match, Source, text_of};
use crate::query::Facets;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Utc};
use std::collections::{BTreeMap, BTreeSet};

/// The range a flow page shows when the query names none: the last 30 UTC
/// days, today included.
pub const DEFAULT_DAYS: i64 = 30;
/// Ranges longer than this are bucketed by ISO week, not by day.
pub const DAILY_MAX_DAYS: i64 = 31;
/// A decision waiting longer than this is highlighted.
pub const STALE_DAYS: i64 = 7;
/// Rows per page on the flow lists.
pub const PAGE_SIZE: usize = 25;

/// A stored time as UTC: RFC 3339, `YYYY-MM-DD HH:MM:SS` (read as UTC) or a
/// bare `YYYY-MM-DD` (its midnight). Anything else is no time at all.
pub fn parse_at(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(text) {
        return Some(t.with_timezone(&Utc));
    }
    for format in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(text, format) {
            return Some(t.and_utc());
        }
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|t| t.and_utc())
}

/// Whole days from `at` to `now`, never negative.
pub fn age_days(at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    (now - at).num_days().max(0)
}

// ---------------------------------------------------------------------------
// Date range and buckets
// ---------------------------------------------------------------------------

/// The whole UTC days a flow page measures, first and last included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

/// One bar of a throughput chart: a UTC day, or an ISO week.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bucket {
    /// The day, or the Monday of the week (which may fall before the range).
    pub start: NaiveDate,
    /// Short, for under a chart bar.
    pub label: String,
    /// In full, for the table.
    pub name: String,
}

fn monday(day: NaiveDate) -> NaiveDate {
    day - Duration::days(i64::from(day.weekday().num_days_from_monday()))
}

impl Window {
    /// The range the query names (`days` wins over `from`/`to`, as in
    /// `query::Range`); a lone `from` runs to today, a lone `to` starts
    /// `DEFAULT_DAYS` before it, and dates given backwards are swapped. With
    /// nothing set it is the last `DEFAULT_DAYS` days.
    pub fn resolve(facets: &Facets, now: DateTime<Utc>) -> Self {
        let today = now.date_naive();
        let back = |end: NaiveDate| end - Duration::days(DEFAULT_DAYS - 1);
        let (from, to) = match facets.range().bounds(now) {
            (Some(f), Some(t)) if f <= t => (f, t),
            (Some(f), Some(t)) => (t, f),
            (Some(f), None) => (f, today.max(f)),
            (None, Some(t)) => (back(t), t),
            (None, None) => (back(today), today),
        };
        Window { from, to }
    }

    /// How many days the range covers.
    pub fn days(&self) -> i64 {
        (self.to - self.from).num_days() + 1
    }

    /// Whether `at` falls on one of the range's days.
    pub fn contains(&self, at: DateTime<Utc>) -> bool {
        let day = at.date_naive();
        day >= self.from && day <= self.to
    }

    /// Whether throughput is bucketed by ISO week.
    pub fn weekly(&self) -> bool {
        self.days() > DAILY_MAX_DAYS
    }

    /// The buckets, oldest first.
    pub fn buckets(&self) -> Vec<Bucket> {
        let mut out = Vec::new();
        if self.weekly() {
            let mut start = monday(self.from);
            while start <= self.to {
                let week = start.iso_week();
                let first = out.is_empty() || week.week() == 1;
                out.push(Bucket {
                    start,
                    label: if first {
                        format!("{}-W{:02}", week.year(), week.week())
                    } else {
                        format!("W{:02}", week.week())
                    },
                    name: format!(
                        "{}-W{:02} (week of {})",
                        week.year(),
                        week.week(),
                        start.format("%-d %b")
                    ),
                });
                start += Duration::days(7);
            }
        } else {
            let mut day = self.from;
            while day <= self.to {
                let first = out.is_empty() || day.day() == 1;
                out.push(Bucket {
                    start: day,
                    label: day.format(if first { "%-d %b" } else { "%-d" }).to_string(),
                    name: day.format("%a %-d %b %Y").to_string(),
                });
                day += Duration::days(1);
            }
        }
        out
    }

    /// The bucket `at` falls in, or `None` outside the range.
    pub fn bucket(&self, at: DateTime<Utc>) -> Option<usize> {
        if !self.contains(at) {
            return None;
        }
        let day = at.date_naive();
        let index = if self.weekly() {
            (monday(day) - monday(self.from)).num_days() / 7
        } else {
            (day - self.from).num_days()
        };
        usize::try_from(index).ok()
    }

    /// The range in words: `15 Nov 2026`, or `1 Nov – 15 Nov 2026`.
    pub fn describe(&self) -> String {
        if self.from == self.to {
            return self.to.format("%-d %b %Y").to_string();
        }
        let from = if self.from.year() == self.to.year() {
            self.from.format("%-d %b")
        } else {
            self.from.format("%-d %b %Y")
        };
        format!("{from} – {}", self.to.format("%-d %b %Y"))
    }
}

// ---------------------------------------------------------------------------
// Percentiles
// ---------------------------------------------------------------------------

/// The nearest-rank percentile of sorted values: the value at rank
/// ⌈percent/100 × n⌉ (at least 1). `None` for no values, never 0.
pub fn nearest_rank(sorted: &[f64], percent: usize) -> Option<f64> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    let rank = (percent * n).div_ceil(100).clamp(1, n);
    Some(sorted[rank - 1])
}

/// How many values there were, and their median and 90th percentile by
/// nearest rank (`None` when there were none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub n: usize,
    pub p50: Option<f64>,
    pub p90: Option<f64>,
}

impl Stats {
    pub fn of(mut values: Vec<f64>) -> Self {
        values.sort_by(f64::total_cmp);
        Stats {
            n: values.len(),
            p50: nearest_rank(&values, 50),
            p90: nearest_rank(&values, 90),
        }
    }
}

// ---------------------------------------------------------------------------
// Sources read at request time
// ---------------------------------------------------------------------------

/// What a source gave: a value, nothing, or nothing yet because the history
/// thread or provenance trail it reads has not been read.
#[derive(Clone, Debug, PartialEq)]
pub enum Found<T> {
    Value(T),
    Missing,
    Unread,
}

impl<T> Found<T> {
    fn then<U>(self, f: impl FnOnce(T) -> Option<U>) -> Found<U> {
        match self {
            Found::Value(v) => f(v).map_or(Found::Missing, Found::Value),
            Found::Missing => Found::Missing,
            Found::Unread => Found::Unread,
        }
    }
}

fn found<T>(value: Option<T>) -> Found<T> {
    value.map_or(Found::Missing, Found::Value)
}

fn history_event<'v>(view: &'v View, id: &str, kind: &str) -> Found<&'v crate::history::Event> {
    match &view.history {
        None => Found::Missing,
        Some(store) => match store.get(id) {
            None => Found::Unread,
            Some(thread) => found(thread.first(kind)),
        },
    }
}

fn provenance_event<'v>(
    view: &'v View,
    id: &str,
    kind: &str,
) -> Found<&'v crate::provenance::Event> {
    match &view.provenance {
        None => Found::Missing,
        Some(store) => match store.get(id) {
            None => Found::Unread,
            Some(trail) => found(trail.events.iter().find(|e| e.kind == kind)),
        },
    }
}

fn payload_text(event: &crate::provenance::Event, key: &str) -> Option<String> {
    event
        .fields()
        .into_iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| text_of(&v))
}

/// A time from one source: a bead field or metadata value, the time of the
/// first history event of a kind (`event:`), or the first provenance event
/// of a kind or one of its payload keys (`provenance:`).
pub fn source_time(view: &View, bead: &Bead, source: &Source) -> Found<DateTime<Utc>> {
    match source {
        Source::Event(kind) => {
            history_event(view, &bead.id, kind).then(|e| e.at.as_deref().and_then(parse_at))
        }
        Source::Provenance { kind, key } => {
            provenance_event(view, &bead.id, kind).then(|e| match key {
                Some(key) => payload_text(e, key).as_deref().and_then(parse_at),
                None => parse_at(&e.occurred_at).or_else(|| parse_at(&e.created_at)),
            })
        }
        other => found(other.text(bead).as_deref().and_then(parse_at)),
    }
}

/// A person from one source, as the profile names people: a bead value, the
/// actor of the first history event of a kind, or a provenance event's actor
/// (or payload key).
pub fn source_person(view: &View, bead: &Bead, source: &Source) -> Found<String> {
    let raw = match source {
        Source::Event(kind) => history_event(view, &bead.id, kind)
            .then(|e| (!e.actor.is_empty()).then(|| e.actor.clone())),
        Source::Provenance { kind, key } => {
            provenance_event(view, &bead.id, kind).then(|e| match key {
                Some(key) => payload_text(e, key),
                None => (!e.actor.is_empty()).then(|| e.actor.clone()),
            })
        }
        other => found(other.text(bead)),
    };
    raw.then(|name| Some(view.profile.person(&name).name))
}

/// The first value the sources give, in order, and whether a source before
/// it (or every source, when none gave one) was still unread.
fn first<T>(sources: &[Source], mut read: impl FnMut(&Source) -> Found<T>) -> (Option<T>, bool) {
    let mut unread = false;
    for source in sources {
        match read(source) {
            Found::Value(v) => return (Some(v), unread),
            Found::Unread => unread = true,
            Found::Missing => {}
        }
    }
    (None, unread)
}

// ---------------------------------------------------------------------------
// Claims
// ---------------------------------------------------------------------------

/// Whether, and how well, a claim is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimState {
    /// The claim time is known.
    Claimed,
    /// The item was claimed (it has a claimant or is in progress) or may
    /// have been (a source is not read yet), but no claim time is known.
    TimeUnknown,
    /// Nothing says it was ever claimed.
    NotClaimed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Claim {
    pub state: ClaimState,
    pub at: Option<DateTime<Utc>>,
    /// Who claimed it, when a source names them.
    pub by: Option<String>,
    /// A request-time source (history or provenance) is not read yet.
    pub unread: bool,
}

/// Resolves a bead's claim from `[flow] claim_at` and `claimant`, each
/// tried in order. Without `claimant` sources the assignee is the claimant.
pub fn claim(view: &View, bead: &Bead) -> Claim {
    let rule = &view.profile.flow;
    let (at, unread_at) = first(&rule.claim_at, |s| source_time(view, bead, s));
    let (by, _) = if rule.claimant.is_empty() {
        let assignee =
            (!bead.assignee.is_empty()).then(|| view.profile.person(&bead.assignee).name);
        (assignee, false)
    } else {
        first(&rule.claimant, |s| source_person(view, bead, s))
    };
    let in_progress = rule.wip_status.contains(&bead.status);
    let state = if at.is_some() {
        ClaimState::Claimed
    } else if by.is_some() || in_progress || unread_at {
        ClaimState::TimeUnknown
    } else {
        ClaimState::NotClaimed
    };
    Claim {
        state,
        at,
        by,
        unread: at.is_none() && unread_at,
    }
}

/// The profile role that names who created an item: the `creator` role, else
/// the first role read from `created_by` or from a history event kind marked
/// `created`.
fn creator_role(view: &View) -> Option<&str> {
    let p = &view.profile;
    if let Some(role) = p.role("creator") {
        return Some(&role.key);
    }
    let created_kind = |kind: &str| {
        p.events
            .as_ref()
            .and_then(|e| e.kinds.get(kind))
            .is_some_and(|k| k.created)
    };
    p.people
        .roles
        .iter()
        .find(|r| {
            r.from.iter().any(|s| match s {
                Source::Field(f) => f == "created_by",
                Source::Event(kind) => created_kind(kind),
                _ => false,
            })
        })
        .map(|r| r.key.as_str())
}

/// Who created a bead: the profile's creator role (which may read history),
/// else the native `created_by`.
pub fn creator(view: &View, bead: &Bead) -> Option<String> {
    if let Some(role) = creator_role(view) {
        if let Some(name) = view.person_in_role(&bead.id, role) {
            return Some(name);
        }
    }
    (!bead.created_by.is_empty()).then(|| view.profile.person(&bead.created_by).name)
}

// ---------------------------------------------------------------------------
// The counted population
// ---------------------------------------------------------------------------

/// One counted item with everything the flow pages measure about it.
#[derive(Clone, Debug)]
pub struct Item<'v> {
    pub index: usize,
    pub bead: &'v Bead,
    pub created: Option<DateTime<Utc>>,
    /// When it closed; only set while its status is closed.
    pub closed: Option<DateTime<Utc>>,
    pub claim: Claim,
    pub creator: Option<String>,
    /// Who holds it now (its assignee), as the profile names people.
    pub holder: Option<String>,
}

impl<'v> Item<'v> {
    pub fn new(view: &'v View, index: usize) -> Option<Self> {
        let bead = view.snapshot()?.beads.get(index)?;
        Some(Item {
            index,
            bead,
            created: bead.created_at.as_deref().and_then(parse_at),
            closed: bead
                .is_closed()
                .then(|| bead.closed_at.as_deref().and_then(parse_at))
                .flatten(),
            claim: claim(view, bead),
            creator: creator(view, bead),
            holder: (!bead.assignee.is_empty()).then(|| view.profile.person(&bead.assignee).name),
        })
    }

    /// Whether the flow pages should list its claim for this range: a known
    /// claim inside it, or an unknown one on an item created in it or still
    /// in progress.
    pub fn claimed_in(&self, window: &Window, view: &View) -> bool {
        match self.claim.state {
            ClaimState::Claimed => self.claim.at.is_some_and(|at| window.contains(at)),
            ClaimState::TimeUnknown => {
                self.created.is_some_and(|at| window.contains(at))
                    || view.profile.flow.wip_status.contains(&self.bead.status)
            }
            ClaimState::NotClaimed => false,
        }
    }

    /// Held now by someone other than the person who claimed it.
    pub fn held_by_other(&self) -> bool {
        matches!((&self.claim.by, &self.holder), (Some(c), Some(h)) if c != h)
    }
}

/// The items the facets select, before the flow's own rules: an empty
/// `state` means every status (flow counts closed work) and `person` also
/// matches the creator and the claimant.
fn selected<'v>(
    view: &'v View,
    facets: &Facets,
    now: DateTime<Utc>,
    rule: Option<&Match>,
) -> Vec<Item<'v>> {
    let Some(s) = view.snapshot() else {
        return Vec::new();
    };
    let mut base = facets.any_status();
    let person = std::mem::take(&mut base.person);
    let mut out = Vec::new();
    for (index, bead) in s.beads.iter().enumerate() {
        if !base.matches(view, index) || rule.is_some_and(|m| !m.matches(bead, now)) {
            continue;
        }
        let Some(item) = Item::new(view, index) else {
            continue;
        };
        if !person.is_empty() {
            let named = |n: &Option<String>| n.as_deref() == Some(person.as_str());
            let by_fields = bead.assignee == person
                || view
                    .row(index)
                    .is_some_and(|r| r.people.values().any(|p| *p == person));
            if !(by_fields || named(&item.creator) || named(&item.claim.by) || named(&item.holder))
            {
                continue;
            }
        }
        out.push(item);
    }
    out
}

/// The counted population: items matching `[flow] items` and the facets.
pub fn items<'v>(view: &'v View, facets: &Facets, now: DateTime<Utc>) -> Vec<Item<'v>> {
    selected(view, facets, now, view.profile.flow.items.as_ref())
}

/// Items matching `[flow.decisions] when` and the facets.
pub fn decision_items<'v>(view: &'v View, facets: &Facets, now: DateTime<Utc>) -> Vec<Item<'v>> {
    selected(view, facets, now, Some(&view.profile.flow.decisions.when))
}

// ---------------------------------------------------------------------------
// Status now
// ---------------------------------------------------------------------------

/// Items per stored status, whatever the range, in Beads' status order.
pub fn statuses(items: &[Item]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for item in items {
        *counts.entry(item.bead.status.as_str()).or_default() += 1;
    }
    let order = &crate::render::STATUS_ORDER;
    let mut out: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(s, n)| (s.to_string(), n))
        .collect();
    out.sort_by_key(|(s, _)| {
        (
            order.iter().position(|o| o == s).unwrap_or(order.len()),
            s.clone(),
        )
    });
    out
}

/// Items not closed, per priority, highest priority (lowest number) first.
pub fn open_by_priority(items: &[Item]) -> Vec<(i64, usize)> {
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    for item in items.iter().filter(|i| !i.bead.is_closed()) {
        *counts.entry(item.bead.priority).or_default() += 1;
    }
    counts.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Throughput
// ---------------------------------------------------------------------------

/// Created, claimed and closed per bucket of the range.
#[derive(Clone, Debug, PartialEq)]
pub struct Throughput {
    pub buckets: Vec<Bucket>,
    pub created: Vec<usize>,
    pub claimed: Vec<usize>,
    pub closed: Vec<usize>,
    /// Claims listed for the range whose time is unknown, so in no bucket.
    pub claim_unknown: usize,
}

pub fn throughput(view: &View, window: &Window, items: &[Item]) -> Throughput {
    let buckets = window.buckets();
    let n = buckets.len();
    let mut t = Throughput {
        buckets,
        created: vec![0; n],
        claimed: vec![0; n],
        closed: vec![0; n],
        claim_unknown: 0,
    };
    let add = |counts: &mut Vec<usize>, at: Option<DateTime<Utc>>| {
        if let Some(b) = at.and_then(|at| window.bucket(at)) {
            if let Some(slot) = counts.get_mut(b) {
                *slot += 1;
            }
        }
    };
    for item in items {
        add(&mut t.created, item.created);
        add(&mut t.closed, item.closed);
        add(&mut t.claimed, item.claim.at);
        if item.claim.state == ClaimState::TimeUnknown && item.claimed_in(window, view) {
            t.claim_unknown += 1;
        }
    }
    t
}

// ---------------------------------------------------------------------------
// Cycle and lead time
// ---------------------------------------------------------------------------

/// Why an item closed in the range has no cycle time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    /// No claim time is known.
    ClaimUnknown,
    /// The claim time is after the close (bad data); not shown as 0.
    ClaimAfterClose,
}

/// The cycle (claim → close) and lead (created → close) time, in seconds, of
/// one item closed in the range.
#[derive(Clone, Debug)]
pub struct Timing<'i, 'v> {
    pub item: &'i Item<'v>,
    pub cycle: Result<f64, Gap>,
    /// `None` when the created time is unknown or after the close.
    pub lead: Option<f64>,
}

/// Every item closed in the range, with its cycle and lead time.
pub fn timings<'i, 'v>(window: &Window, items: &'i [Item<'v>]) -> Vec<Timing<'i, 'v>> {
    items
        .iter()
        .filter_map(|item| {
            let closed = item.closed.filter(|at| window.contains(*at))?;
            let seconds = |from: DateTime<Utc>| (closed - from).num_seconds() as f64;
            let cycle = match item.claim.at {
                None => Err(Gap::ClaimUnknown),
                Some(at) if at > closed => Err(Gap::ClaimAfterClose),
                Some(at) => Ok(seconds(at)),
            };
            let lead = item.created.filter(|c| *c <= closed).map(seconds);
            Some(Timing { item, cycle, lead })
        })
        .collect()
}

/// Cycle and lead percentiles over some timings, with what was left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measures {
    /// Items closed in the range.
    pub closed: usize,
    pub cycle: Stats,
    pub lead: Stats,
    /// Left out of the cycle time: claim time unknown.
    pub claim_unknown: usize,
    /// Left out of the cycle time: claimed after the close.
    pub claim_after_close: usize,
    /// Left out of the lead time: created time unknown (or after the close).
    pub lead_unknown: usize,
}

pub fn measures<'a, 'i: 'a, 'v: 'i>(
    rows: impl IntoIterator<Item = &'a Timing<'i, 'v>>,
) -> Measures {
    let mut m = Measures::default();
    let (mut cycles, mut leads) = (Vec::new(), Vec::new());
    for row in rows {
        m.closed += 1;
        match row.cycle {
            Ok(s) => cycles.push(s),
            Err(Gap::ClaimUnknown) => m.claim_unknown += 1,
            Err(Gap::ClaimAfterClose) => m.claim_after_close += 1,
        }
        match row.lead {
            Some(s) => leads.push(s),
            None => m.lead_unknown += 1,
        }
    }
    m.cycle = Stats::of(cycles);
    m.lead = Stats::of(leads);
    m
}

// ---------------------------------------------------------------------------
// Breakdowns
// ---------------------------------------------------------------------------

/// What a breakdown groups items by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum By {
    /// A profile `[[groups]]` key.
    Group(String),
    Type,
    Kind,
    Priority,
    Claimant,
    Creator,
}

/// The value an item has no value for.
pub const NONE: &str = "none";
/// The claimant or creator nobody names.
pub const UNKNOWN: &str = "unknown";

impl By {
    /// Every breakdown this profile offers: its groups, then type, kind
    /// (with `[[kinds]]`), priority, claimant and creator.
    pub fn options(view: &View) -> Vec<By> {
        let mut out: Vec<By> = view
            .profile
            .groups
            .iter()
            .map(|g| By::Group(g.key.clone()))
            .collect();
        out.push(By::Type);
        if !view.profile.kinds.is_empty() {
            out.push(By::Kind);
        }
        out.extend([By::Priority, By::Claimant, By::Creator]);
        out
    }

    /// The breakdown the query names, else the profile's first group, else
    /// type. An unknown name falls back the same way.
    pub fn chosen(view: &View, facets: &Facets) -> By {
        let options = By::options(view);
        options
            .iter()
            .find(|o| o.key() == facets.by)
            .cloned()
            .unwrap_or_else(|| options[0].clone())
    }

    /// Its value in the URL.
    pub fn key(&self) -> &str {
        match self {
            By::Group(key) => key,
            By::Type => "type",
            By::Kind => "kind",
            By::Priority => "priority",
            By::Claimant => "claimant",
            By::Creator => "creator",
        }
    }

    pub fn label(&self, view: &View) -> String {
        match self {
            By::Group(key) => view
                .profile
                .group(key)
                .map_or_else(|| key.clone(), |g| g.label.clone()),
            By::Type => "Type".into(),
            By::Kind => "Kind".into(),
            By::Priority => "Priority".into(),
            By::Claimant => "Claimant".into(),
            By::Creator => "Creator".into(),
        }
    }

    /// An item's values: never empty, so every item lands in a row. A group
    /// can give several values, so group totals may count an item twice;
    /// every other breakdown gives exactly one.
    pub fn values(&self, view: &View, item: &Item) -> Vec<String> {
        let one = |v: Option<String>, missing: &str| vec![v.unwrap_or_else(|| missing.into())];
        match self {
            By::Group(key) => {
                let values = view
                    .row(item.index)
                    .and_then(|r| r.groups.get(key))
                    .cloned()
                    .unwrap_or_default();
                if values.is_empty() {
                    vec![NONE.into()]
                } else {
                    values
                }
            }
            By::Type => one(
                (!item.bead.issue_type.is_empty()).then(|| item.bead.issue_type.clone()),
                NONE,
            ),
            By::Kind => one(
                view.row(item.index)
                    .and_then(|r| r.kind.as_deref())
                    .map(|k| {
                        view.profile
                            .kind(k)
                            .map_or(k, |r| r.label.as_str())
                            .to_string()
                    }),
                NONE,
            ),
            By::Priority => vec![format!("P{}", item.bead.priority)],
            By::Claimant => one(item.claim.by.clone(), UNKNOWN),
            By::Creator => one(item.creator.clone(), UNKNOWN),
        }
    }
}

/// Measures per breakdown value, most closed items first, then by name.
pub fn breakdown<'a, 'i: 'a, 'v: 'i>(
    view: &View,
    by: &By,
    rows: &'a [Timing<'i, 'v>],
) -> Vec<(String, Measures)> {
    let mut groups: BTreeMap<String, Vec<&'a Timing<'i, 'v>>> = BTreeMap::new();
    for row in rows {
        let values: BTreeSet<String> = by.values(view, row.item).into_iter().collect();
        for value in values {
            groups.entry(value).or_default().push(row);
        }
    }
    let mut out: Vec<(String, Measures)> = groups
        .into_iter()
        .map(|(value, rows)| (value, measures(rows)))
        .collect();
    out.sort_by(|a, b| b.1.closed.cmp(&a.1.closed).then_with(|| a.0.cmp(&b.0)));
    out
}

/// A count per value of `key` (each value once per item), most first.
pub fn tally<'i, 'v: 'i>(
    items: impl IntoIterator<Item = &'i Item<'v>>,
    key: impl Fn(&Item) -> String,
) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for item in items {
        *counts.entry(key(item)).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

// ---------------------------------------------------------------------------
// Ageing work in progress
// ---------------------------------------------------------------------------

/// Items in a `[flow] wip_status`, oldest claim first; items whose claim
/// time is unknown come last.
pub fn wip<'i, 'v>(view: &View, items: &'i [Item<'v>]) -> Vec<&'i Item<'v>> {
    let mut out: Vec<&Item> = items
        .iter()
        .filter(|i| view.profile.flow.wip_status.contains(&i.bead.status))
        .collect();
    out.sort_by(|a, b| match (a.claim.at, b.claim.at) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.bead.id.cmp(&b.bead.id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.bead.id.cmp(&b.bead.id),
    });
    out
}

/// The age buckets of the ageing chart: (label, first day, last day).
pub const AGE_BUCKETS: [(&str, i64, i64); 6] = [
    ("under 1 d", 0, 0),
    ("1–2 d", 1, 2),
    ("3–6 d", 3, 6),
    ("7–13 d", 7, 13),
    ("14–29 d", 14, 29),
    ("30 d or more", 30, i64::MAX),
];

/// How many ages fall in each `AGE_BUCKETS` bucket.
pub fn age_buckets(ages: &[i64]) -> Vec<(String, usize)> {
    AGE_BUCKETS
        .iter()
        .map(|(label, lo, hi)| {
            (
                label.to_string(),
                ages.iter().filter(|a| *a >= lo && *a <= hi).count(),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Open decisions
// ---------------------------------------------------------------------------

/// One open decision.
#[derive(Clone, Debug)]
pub struct Decision<'i, 'v> {
    pub item: &'i Item<'v>,
    /// When it started waiting: `[flow.decisions] since`, else created.
    pub since: Option<DateTime<Utc>>,
    /// Who it waits on: `assignee`, else the bead's assignee.
    pub assignee: Option<String>,
    /// The due value as stored, when there is one.
    pub due: Option<String>,
    pub past_due: bool,
}

impl Decision<'_, '_> {
    pub fn age(&self, now: DateTime<Utc>) -> Option<i64> {
        self.since.map(|at| age_days(at, now))
    }

    pub fn stale(&self, now: DateTime<Utc>) -> bool {
        self.age(now).is_some_and(|d| d > STALE_DAYS)
    }
}

/// The decisions among `items`, oldest first (unknown ages last).
pub fn decisions<'i, 'v>(
    view: &View,
    items: &'i [Item<'v>],
    now: DateTime<Utc>,
) -> Vec<Decision<'i, 'v>> {
    let rule = &view.profile.flow.decisions;
    let mut out: Vec<Decision> = items
        .iter()
        .map(|item| {
            let bead = item.bead;
            let since = match rule.since.as_ref().map(|s| source_time(view, bead, s)) {
                Some(Found::Value(at)) => Some(at),
                _ => item.created,
            };
            let assignee = match rule.assignee.as_ref().map(|s| source_person(view, bead, s)) {
                Some(Found::Value(name)) => Some(name),
                _ => item.holder.clone(),
            };
            let due = rule.due.as_ref().and_then(|s| match s {
                Source::Event(_) | Source::Provenance { .. } => match source_time(view, bead, s) {
                    Found::Value(at) => Some(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                    _ => None,
                },
                other => other.text(bead),
            });
            let past_due = due
                .as_deref()
                .is_some_and(|d| crate::profile::cond::is_past(d, now));
            Decision {
                item,
                since,
                assignee,
                due,
                past_due,
            }
        })
        .collect();
    out.sort_by(|a, b| match (a.since, b.since) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.item.bead.id.cmp(&b.item.bead.id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.item.bead.id.cmp(&b.item.bead.id),
    });
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::beads::{Bead, Snapshot};
    use crate::model::Feed;
    use crate::profile::Profile;
    use std::sync::Arc;

    pub(crate) fn at(text: &str) -> DateTime<Utc> {
        parse_at(text).unwrap()
    }

    pub(crate) fn facets(query: &str) -> Facets {
        let uri: axum::http::Uri = format!("/flow?{query}").parse().unwrap();
        axum::extract::Query::<Facets>::try_from_uri(&uri)
            .unwrap()
            .0
    }

    /// A view of `beads` with `profile` (the generic one when `None`).
    pub(crate) fn view_of(beads: Vec<Bead>, profile: Option<Profile>, now: DateTime<Utc>) -> View {
        let profile = Arc::new(profile.unwrap_or_default());
        let snapshot = Snapshot::build(beads, Vec::new(), None, now, &profile);
        View {
            beads: Some(Feed {
                data: Arc::new(snapshot),
                error: None,
            }),
            profile,
            ..Default::default()
        }
    }

    pub(crate) fn bead(id: &str, status: &str, created: &str) -> Bead {
        Bead {
            id: id.into(),
            title: format!("Item {id}"),
            status: status.into(),
            issue_type: "task".into(),
            created_at: Some(created.into()),
            ..Default::default()
        }
    }

    #[test]
    fn nearest_rank_on_small_sets_and_no_data_for_none() {
        assert_eq!(nearest_rank(&[], 50), None, "no data, never 0");
        assert_eq!(Stats::of(vec![]), Stats::default());
        assert_eq!(Stats::default().p50, None);
        let one = Stats::of(vec![7.0]);
        assert_eq!((one.n, one.p50, one.p90), (1, Some(7.0), Some(7.0)));
        let two = Stats::of(vec![9.0, 3.0]);
        assert_eq!(two.p50, Some(3.0), "rank ⌈0.5 × 2⌉ = 1");
        assert_eq!(two.p90, Some(9.0), "rank ⌈0.9 × 2⌉ = 2");
        let ten = Stats::of((1..=10).rev().map(f64::from).collect());
        assert_eq!(ten.p50, Some(5.0), "rank 5 of 10");
        assert_eq!(ten.p90, Some(9.0), "rank 9 of 10, not interpolated");
        let eleven: Vec<f64> = (1..=11).map(f64::from).collect();
        assert_eq!(nearest_rank(&eleven, 50), Some(6.0));
        assert_eq!(nearest_rank(&eleven, 90), Some(10.0), "⌈9.9⌉ = 10");
        assert_eq!(nearest_rank(&eleven, 100), Some(11.0));
        assert_eq!(nearest_rank(&eleven, 0), Some(1.0));
        // Integer ranks: a float 0.9 × 30 would round up to rank 28.
        let thirty: Vec<f64> = (1..=30).map(f64::from).collect();
        assert_eq!(nearest_rank(&thirty, 90), Some(27.0));
    }

    #[test]
    fn times_parse_as_utc_and_ranges_are_whole_utc_days() {
        assert_eq!(at("2026-11-15T01:30:00+02:00"), at("2026-11-14T23:30:00Z"));
        assert_eq!(at("2026-11-15 10:00:00"), at("2026-11-15T10:00:00Z"));
        assert_eq!(at("2026-11-15"), at("2026-11-15T00:00:00Z"));
        assert_eq!(parse_at("soon"), None);
        let now = at("2026-11-15T12:00:00Z");
        let default = Window::resolve(&facets(""), now);
        assert_eq!(default.days(), DEFAULT_DAYS);
        assert_eq!(default.to, now.date_naive());
        let today = Window::resolve(&facets("days=1"), now);
        assert!(today.contains(at("2026-11-15T00:00:00Z")));
        assert!(today.contains(at("2026-11-15T23:59:59Z")));
        assert!(!today.contains(at("2026-11-14T23:59:59Z")));
        assert!(!today.contains(at("2026-11-16T00:00:00Z")));
        // An offset moves the instant onto its UTC day.
        assert!(!today.contains(at("2026-11-15T01:00:00+02:00")));
        let span = Window::resolve(&facets("from=2026-11-10&to=2026-11-01"), now);
        assert_eq!(span.from.to_string(), "2026-11-01", "swapped");
        assert_eq!(span.describe(), "1 Nov – 10 Nov 2026");
        let lone = Window::resolve(&facets("from=2026-11-10"), now);
        assert_eq!(
            (lone.from.to_string(), lone.to),
            ("2026-11-10".into(), now.date_naive())
        );
        let days_win = Window::resolve(&facets("days=7&from=2020-01-01"), now);
        assert_eq!(days_win.days(), 7, "days wins over from/to");
        assert_eq!(today.describe(), "15 Nov 2026");
        assert_eq!(
            Window::resolve(&facets("from=2025-12-20&to=2026-01-05"), now).describe(),
            "20 Dec 2025 – 5 Jan 2026"
        );
    }

    #[test]
    fn daily_buckets_up_to_31_days_then_iso_weeks_across_a_year_end() {
        let now = at("2027-01-15T12:00:00Z");
        let month = Window::resolve(&facets("from=2026-12-01&to=2026-12-31"), now);
        assert!(!month.weekly());
        let days = month.buckets();
        assert_eq!(days.len(), 31);
        assert_eq!(days[0].label, "1 Dec");
        assert_eq!(days[1].label, "2");
        assert_eq!(month.bucket(at("2026-12-31T23:59:59Z")), Some(30));
        assert_eq!(month.bucket(at("2027-01-01T00:00:00Z")), None);
        // 2026-12-28 (Mon) … 2027-01-03 (Sun) is ISO week 2026-W53; 2027-W01
        // starts on 4 January.
        let span = Window::resolve(&facets("from=2026-12-01&to=2027-01-10"), now);
        assert!(span.weekly());
        let weeks = span.buckets();
        let names: Vec<&str> = weeks.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(names, ["2026-W49", "W50", "W51", "W52", "W53", "2027-W01"]);
        assert_eq!(
            weeks[0].start.to_string(),
            "2026-11-30",
            "the Monday before"
        );
        assert_eq!(span.bucket(at("2026-12-01T00:00:00Z")), Some(0));
        assert_eq!(span.bucket(at("2026-12-31T10:00:00Z")), Some(4));
        assert_eq!(
            span.bucket(at("2027-01-03T23:59:59Z")),
            Some(4),
            "Sunday of W53"
        );
        assert_eq!(span.bucket(at("2027-01-04T00:00:00Z")), Some(5));
        assert_eq!(
            span.bucket(at("2026-11-30T12:00:00Z")),
            None,
            "before the range"
        );
        // 2024-12-30 is in ISO week 2025-W01.
        let other = Window::resolve(&facets("from=2024-12-02&to=2025-01-12"), now);
        let labels: Vec<String> = other.buckets().into_iter().map(|b| b.label).collect();
        assert_eq!(labels.last().unwrap(), "W02");
        assert!(labels.contains(&"2025-W01".to_string()));
        assert_eq!(other.bucket(at("2024-12-31T00:00:00Z")), Some(4));
    }

    #[test]
    fn three_claim_states_and_unknown_claims_are_counted_not_zero() {
        let now = at("2026-11-15T12:00:00Z");
        let mut claimed = bead("a-1", "closed", "2026-11-01T00:00:00Z");
        claimed.started_at = Some("2026-11-02T00:00:00Z".into());
        claimed.closed_at = Some("2026-11-04T00:00:00Z".into());
        claimed.assignee = "ann".into();
        let mut unknown = bead("a-2", "in_progress", "2026-11-03T00:00:00Z");
        unknown.assignee = "bo".into();
        let mut closed_unclaimed = bead("a-3", "closed", "2026-11-03T00:00:00Z");
        closed_unclaimed.closed_at = Some("2026-11-05T00:00:00Z".into());
        let open = bead("a-4", "open", "2026-11-03T00:00:00Z");
        let view = view_of(vec![claimed, unknown, closed_unclaimed, open], None, now);
        let items = items(&view, &facets(""), now);
        let states: Vec<ClaimState> = items.iter().map(|i| i.claim.state).collect();
        assert_eq!(
            states,
            [
                ClaimState::Claimed,
                ClaimState::TimeUnknown,
                ClaimState::NotClaimed,
                ClaimState::NotClaimed
            ]
        );
        assert_eq!(
            items[0].claim.by.as_deref(),
            Some("ann"),
            "assignee by default"
        );
        let window = Window::resolve(&facets(""), now);
        let t = throughput(&view, &window, &items);
        assert_eq!(t.claimed.iter().sum::<usize>(), 1);
        assert_eq!(t.claim_unknown, 1, "the unknown claim is counted apart");
        assert_eq!(t.created.iter().sum::<usize>(), 4);
        assert_eq!(t.closed.iter().sum::<usize>(), 2);
        let rows = timings(&window, &items);
        let m = measures(&rows);
        assert_eq!(m.closed, 2);
        assert_eq!(m.cycle.n, 1);
        assert_eq!(m.claim_unknown, 1, "left out, never 0 seconds");
        assert_eq!(m.cycle.p50, Some(2.0 * 86_400.0));
        assert_eq!(m.lead.n, 2);
        assert_eq!(m.lead.p90, Some(3.0 * 86_400.0));
        // A claim after the close is left out too, with its own reason.
        let mut late = bead("b-1", "closed", "2026-11-01T00:00:00Z");
        late.started_at = Some("2026-11-06T00:00:00Z".into());
        late.closed_at = Some("2026-11-05T00:00:00Z".into());
        let v2 = view_of(vec![late], None, now);
        let i2 = super::items(&v2, &facets(""), now);
        let m2 = measures(&timings(&window, &i2));
        assert_eq!((m2.cycle.n, m2.claim_after_close), (0, 1));
        assert_eq!(m2.cycle.p50, None);
    }

    #[test]
    fn request_time_sources_read_history_and_say_when_unread() {
        let view = crate::testing::work_view();
        let s = view.snapshot().unwrap();
        let wk1 = s.get("wk-1").unwrap();
        // The profile reads work.fields.claimed_at first.
        let c = claim(&view, wk1);
        assert_eq!(c.state, ClaimState::Claimed);
        assert_eq!(c.at, Some(at("2026-11-15T10:00:00Z")));
        assert_eq!(c.by.as_deref(), Some("factory/hostA/rigA"));
        // An event source on its own reads the thread.
        let ev = Source::parse("event:claim").unwrap();
        assert!(matches!(source_time(&view, wk1, &ev), Found::Value(_)));
        assert_eq!(
            source_person(&view, wk1, &ev),
            Found::Value("factory/hostA/rigA".into())
        );
        let mut unread = view.clone();
        unread.history = Some(crate::history::Store::default());
        assert_eq!(source_time(&unread, wk1, &ev), Found::Unread);
        unread.history = None;
        assert_eq!(source_time(&unread, wk1, &ev), Found::Missing);
        // A gate with nothing read yet is unknown, not "not claimed".
        let mut pending = view.clone();
        pending.history = Some(crate::history::Store::default());
        let wk4 = s.get("wk-4").unwrap();
        let c4 = claim(&pending, wk4);
        assert_eq!((c4.state, c4.unread), (ClaimState::TimeUnknown, true));
        assert_eq!(claim(&view, wk4).state, ClaimState::NotClaimed);
        // Provenance sources read the trail.
        let prov = Source::parse("provenance:claim").unwrap();
        let wk2 = s.get("wk-2").unwrap();
        assert!(matches!(source_time(&view, wk2, &prov), Found::Value(_)));
        assert_eq!(
            source_time(&view, s.get("wk-5").unwrap(), &prov),
            Found::Unread,
            "no trail read for wk-5"
        );
        // The creator role reads bead fields, then history.
        assert_eq!(creator(&view, wk2).as_deref(), Some("alice"));
    }

    #[test]
    fn the_fixture_population_follows_flow_items_and_facets() {
        let view = crate::testing::work_view();
        let now = at("2026-11-15T12:00:00Z");
        let ids = |q: &str| -> Vec<String> {
            items(&view, &facets(q), now)
                .iter()
                .map(|i| i.bead.id.clone())
                .collect()
        };
        let all = ids("");
        assert_eq!(
            all,
            ["wk-1", "wk-2", "wk-3", "wk-4", "wk-5"],
            "tasks and gates, closed too"
        );
        assert_eq!(ids("state=closed"), ["wk-2", "wk-5"]);
        assert_eq!(
            ids("milestone=v1&by=rig"),
            ["wk-1", "wk-2", "wk-4"],
            "by is not a filter"
        );
        assert_eq!(ids("person=factory/hostA/rigA"), ["wk-1"], "the claimant");
        let alice = ids("person=alice");
        assert!(
            alice.contains(&"wk-2".to_string()),
            "the creator: {alice:?}"
        );
        let decisions: Vec<String> = decision_items(&view, &facets(""), now)
            .iter()
            .map(|i| i.bead.id.clone())
            .collect();
        assert_eq!(decisions, ["wk-q1"]);
        let open = decision_items(&view, &facets(""), now);
        let d = super::decisions(&view, &open, now);
        assert_eq!(d[0].assignee.as_deref(), Some("bob"));
        assert_eq!(d[0].due.as_deref(), Some("2026-11-01"));
        assert!(d[0].past_due && d[0].stale(now));
        assert_eq!(d[0].age(now), Some(26));
    }

    #[test]
    fn breakdown_totals_add_up_and_missing_values_have_a_row() {
        let now = at("2026-11-15T12:00:00Z");
        let mut beads = Vec::new();
        for i in 0..10 {
            let mut b = bead(&format!("c-{i}"), "closed", "2026-11-01T00:00:00Z");
            b.issue_type = if i % 3 == 0 {
                "bug".into()
            } else {
                "task".into()
            };
            b.closed_at = Some(format!("2026-11-{:02}T00:00:00Z", 5 + i));
            if i % 4 != 0 {
                b.started_at = Some("2026-11-03T00:00:00Z".into());
                b.assignee = format!("p{}", i % 2);
            }
            beads.push(b);
        }
        let view = view_of(beads, None, now);
        let items = items(&view, &facets(""), now);
        let window = Window::resolve(&facets(""), now);
        let rows = timings(&window, &items);
        let total = measures(&rows);
        for by in By::options(&view) {
            let parts = breakdown(&view, &by, &rows);
            let sum = |f: fn(&Measures) -> usize| parts.iter().map(|(_, m)| f(m)).sum::<usize>();
            assert_eq!(sum(|m| m.closed), total.closed, "{by:?}");
            assert_eq!(sum(|m| m.cycle.n), total.cycle.n, "{by:?}");
            assert_eq!(sum(|m| m.claim_unknown), total.claim_unknown, "{by:?}");
            assert_eq!(sum(|m| m.lead.n), total.lead.n, "{by:?}");
        }
        assert_eq!(total.claim_unknown, 3, "c-0, c-4, c-8");
        let by_claimant = breakdown(&view, &By::Claimant, &rows);
        assert!(
            by_claimant
                .iter()
                .any(|(v, m)| v == UNKNOWN && m.closed == 3)
        );
        assert_eq!(By::chosen(&view, &facets("by=claimant")), By::Claimant);
        assert_eq!(
            By::chosen(&view, &facets("by=bogus")),
            By::Type,
            "no groups: type"
        );
        let work = crate::testing::work_view();
        assert_eq!(
            By::chosen(&work, &facets("")),
            By::Group("milestone".into())
        );
        let created = tally(&items, |i| i.bead.issue_type.clone());
        assert_eq!(created, [("task".to_string(), 6), ("bug".to_string(), 4)]);
    }

    #[test]
    fn ageing_puts_unknown_claims_last_and_buckets_ages() {
        let now = at("2026-11-15T12:00:00Z");
        let mut old = bead("w-1", "in_progress", "2026-10-01T00:00:00Z");
        old.started_at = Some("2026-10-10T00:00:00Z".into());
        let mut new = bead("w-2", "in_progress", "2026-11-01T00:00:00Z");
        new.started_at = Some("2026-11-15T06:00:00Z".into());
        let unknown = bead("w-0", "in_progress", "2026-11-01T00:00:00Z");
        let open = bead("w-3", "open", "2026-11-01T00:00:00Z");
        let view = view_of(vec![new, unknown, old, open], None, now);
        let items = items(&view, &facets(""), now);
        let wip: Vec<&str> = wip(&view, &items)
            .iter()
            .map(|i| i.bead.id.as_str())
            .collect();
        assert_eq!(wip, ["w-1", "w-2", "w-0"]);
        assert_eq!(age_days(at("2026-10-10T00:00:00Z"), now), 36);
        assert_eq!(
            age_days(at("2026-11-16T00:00:00Z"), now),
            0,
            "never negative"
        );
        let b = age_buckets(&[0, 1, 2, 3, 13, 14, 36]);
        let counts: Vec<usize> = b.iter().map(|x| x.1).collect();
        assert_eq!(counts, [1, 2, 1, 1, 1, 1]);
    }

    /// A store of `n` generated beads over 120 days, with the work profile's
    /// shapes (groups, claims in metadata, kinds), for the timing test.
    pub(crate) fn generated(n: usize, now: DateTime<Utc>) -> View {
        let statuses = [
            "open",
            "in_progress",
            "closed",
            "closed",
            "blocked",
            "closed",
        ];
        let mut beads = Vec::with_capacity(n);
        for i in 0..n {
            let created = now - Duration::minutes((i as i64 * 14_401) % (120 * 1440));
            let status = statuses[i % statuses.len()];
            let mut b = Bead {
                id: format!("gen-{i}"),
                title: format!("Generated item {i}"),
                status: status.into(),
                issue_type: if i % 7 == 0 {
                    "decision".into()
                } else {
                    "task".into()
                },
                priority: (i % 5) as i64,
                created_at: Some(created.to_rfc3339()),
                created_by: format!("person{}", i % 13),
                labels: vec![format!("area:a{}", i % 6)],
                ..Default::default()
            };
            if status != "open" && i % 9 != 0 {
                let claimed = created + Duration::hours((i % 50) as i64);
                b.started_at = Some(claimed.to_rfc3339());
                b.assignee = format!("factory/host/rig{}", i % 4);
                b.metadata = serde_json::json!({"work": {"fields": {
                    "claimed_at": claimed.to_rfc3339(),
                    "claimed_by": format!("factory/host/rig{}", i % 4),
                    "milestone": format!("v{}", i % 3),
                    "write_scope": ["x"],
                }}});
            }
            if status == "closed" {
                b.closed_at = Some((created + Duration::hours(30 + (i % 200) as i64)).to_rfc3339());
            }
            beads.push(b);
        }
        let mut profile = crate::testing::work_profile();
        profile.flow.items = None;
        view_of(beads, Some(profile), now)
    }

    #[test]
    fn the_measures_add_up_on_a_generated_store() {
        let now = at("2026-11-15T12:00:00Z");
        let view = generated(600, now);
        let window = Window::resolve(&facets("days=90"), now);
        assert!(window.weekly());
        let items = items(&view, &facets("days=90"), now);
        assert_eq!(items.len(), 600);
        let t = throughput(&view, &window, &items);
        let in_range = items
            .iter()
            .filter(|i| i.created.is_some_and(|c| window.contains(c)))
            .count();
        assert_eq!(t.created.iter().sum::<usize>(), in_range);
        let by_status: usize = statuses(&items).iter().map(|s| s.1).sum();
        assert_eq!(by_status, 600);
        let open: usize = open_by_priority(&items).iter().map(|p| p.1).sum();
        assert_eq!(open, items.iter().filter(|i| !i.bead.is_closed()).count());
    }
}
