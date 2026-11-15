// Owned by WP3 — History, activity, people. Frozen signature:
// `activity(&View, &Facets) -> String`.
//! `/activity`: one timeline across every item, newest first, built from
//! what is already in memory — history events, plain comments, created and
//! closed dates, provenance events read so far and run starts and ends from
//! the run feed index — filtered by the shared facets and a date range
//! (the last seven days unless the query names one).
//!
//! One pass collects the entries inside the range that pass the filters;
//! only those are ordered, and only as far as the page asked for.

use super::fmt::{when, words};
use super::layout::page;
use super::{
    PAGE_SIZE, bead_link, escape, heading, pagination, person_link, run_link, select, tone_badge,
};
use crate::beads::{Bead, Comment, Snapshot};
use crate::model::View;
use crate::profile::Tone;
use crate::query::{Facets, Range, href};
use crate::runs::{RunSummary, run_tone};
use crate::{history, provenance};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

/// The query key of the event-kind filter. It is not a reserved facet key,
/// so it arrives among `Facets::groups`; a profile group with this key
/// wins over it.
pub(crate) const EVENT_KEY: &str = "event";

/// Days shown when the query names no range.
const DEFAULT_DAYS: u32 = 7;

/// The date range a page measures: the query's, else the last
/// `DEFAULT_DAYS` days.
pub(crate) fn range(facets: &Facets) -> Range {
    let range = facets.range();
    if range.from.is_none() && range.to.is_none() && range.days.is_none() {
        Range {
            days: Some(DEFAULT_DAYS),
            ..range
        }
    } else {
        range
    }
}

/// Words for a range at `now`: "the last 7 days", "1 Nov – 2 Nov".
pub(crate) fn range_words(range: &Range, now: DateTime<Utc>) -> String {
    if let Some(days) = range.days {
        return if days == 1 {
            "today".into()
        } else {
            format!("the last {days} days")
        };
    }
    let day = |d: NaiveDate| d.format("%-d %b %Y").to_string();
    match range.bounds(now) {
        (Some(from), Some(to)) => format!("{} to {}", day(from), day(to)),
        (Some(from), None) => format!("since {}", day(from)),
        (None, Some(to)) => format!("up to {}", day(to)),
        (None, None) => "all time".into(),
    }
}

/// Where an entry came from; borrowed from the view, so collecting costs
/// no copies.
#[derive(Clone, Copy)]
enum Source<'a> {
    Event(&'a history::Event),
    Comment(&'a Comment),
    Created(&'a Bead),
    Closed(&'a Bead),
    Trail(&'a provenance::Event),
    RunStart(&'a RunSummary),
    RunEnd(&'a RunSummary),
}

impl<'a> Source<'a> {
    /// The event-kind filter value: a history kind as recorded, `comment`,
    /// `created`, `closed`, `trail:<kind>`, `run:start` or `run:end`.
    fn kind(&self) -> Cow<'a, str> {
        match self {
            Source::Event(e) => Cow::Borrowed(e.kind.as_str()),
            Source::Comment(_) => "comment".into(),
            Source::Created(_) => "created".into(),
            Source::Closed(_) => "closed".into(),
            Source::Trail(e) => format!("trail:{}", e.kind).into(),
            Source::RunStart(_) => "run:start".into(),
            Source::RunEnd(_) => "run:end".into(),
        }
    }

    fn is_kind(&self, want: &str) -> bool {
        match self {
            Source::Trail(e) => want.strip_prefix("trail:") == Some(e.kind.as_str()),
            other => other.kind() == want,
        }
    }

    /// The raw name of who did it, when known.
    fn actor(&self) -> Option<&'a str> {
        let name = match self {
            Source::Event(e) => e.actor.as_str(),
            Source::Comment(c) => c.author.as_deref().unwrap_or(""),
            Source::Created(b) => b.created_by.as_str(),
            Source::Trail(e) => e.actor.as_str(),
            Source::Closed(_) | Source::RunStart(_) | Source::RunEnd(_) => "",
        };
        (!name.is_empty()).then_some(name)
    }

    /// Ties at the same instant: later sources and later thread positions
    /// sort first.
    fn rank(&self) -> u8 {
        match self {
            Source::Created(_) => 0,
            Source::RunStart(_) => 1,
            Source::Trail(_) => 2,
            Source::Event(_) => 3,
            Source::Comment(_) => 4,
            Source::RunEnd(_) => 5,
            Source::Closed(_) => 6,
        }
    }
}

/// The label for a filter value, for the event-kind select.
fn kind_label(view: &View, kind: &str) -> String {
    match kind {
        "comment" => "Comment".into(),
        "created" => "Created".into(),
        "closed" => "Closed".into(),
        "run:start" => "Run started".into(),
        "run:end" => "Run ended".into(),
        _ => match kind.strip_prefix("trail:") {
            Some(k) => format!("{} (provenance)", words(k)),
            None => view.profile.event_kind(kind).0,
        },
    }
}

/// One timeline entry.
struct Entry<'a> {
    at: DateTime<Utc>,
    at_text: &'a str,
    /// The item's index in the snapshot, when it resolved.
    bead: Option<usize>,
    /// Position within its source (a thread's event order), for ties.
    position: usize,
    source: Source<'a>,
}

/// Newest first; ties by source, item and position, all descending, so the
/// order never depends on hash order.
fn newest_first(a: &Entry, b: &Entry) -> Ordering {
    (b.at, b.source.rank(), b.bead, b.position).cmp(&(a.at, a.source.rank(), a.bead, a.position))
}

/// What the filters ask of an entry, worked out once per request.
pub(crate) struct Filter {
    /// The facets that select items: `person` removed (here it means the
    /// actor), `state` defaulting to every state, and the event key removed
    /// unless a profile group owns it.
    items: Facets,
    /// Whether any item facet is set (an entry with no item then fails).
    narrowed: bool,
    person: Option<String>,
    event: Option<String>,
    bounds: (Option<NaiveDate>, Option<NaiveDate>),
}

impl Filter {
    pub(crate) fn new(view: &View, facets: &Facets, now: DateTime<Utc>) -> Self {
        let mut items = facets.clone();
        items.person.clear();
        items.page.clear();
        let mut event = None;
        if view.profile.group(EVENT_KEY).is_none() {
            event = items.groups.remove(EVENT_KEY).filter(|e| !e.is_empty());
        }
        let narrowed = !items.state.is_empty()
            || [
                &items.q,
                &items.issue_type,
                &items.kind,
                &items.priority,
                &items.dstate,
                &items.ready,
                &items.epic,
            ]
            .iter()
            .any(|v| !v.is_empty())
            || !items.groups.is_empty();
        if items.state.is_empty() {
            items.state = "all".into();
        }
        Filter {
            items,
            narrowed,
            person: Some(facets.person.clone()).filter(|p| !p.is_empty()),
            event,
            bounds: range(facets).bounds(now),
        }
    }

    fn in_range(&self, at: &DateTime<Utc>) -> bool {
        let day = at.date_naive();
        self.bounds.0.is_none_or(|f| day >= f) && self.bounds.1.is_none_or(|t| day <= t)
    }
}

/// The entries of one request, and what the filter selects could offer.
pub(crate) struct Timeline<'a> {
    entries: Vec<Entry<'a>>,
    /// Event-kind filter value → entries, before the kind filter.
    pub(crate) kinds: BTreeMap<Cow<'a, str>, usize>,
    /// Every person seen as an actor, before the person filter.
    pub(crate) people: BTreeSet<String>,
}

impl<'a> Timeline<'a> {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Orders just enough entries to show page `page` and returns them.
    fn page(&mut self, page: usize, size: usize) -> &[Entry<'a>] {
        let start = (page * size).min(self.entries.len());
        let end = (start + size).min(self.entries.len());
        if end == 0 {
            return &[];
        }
        if end < self.entries.len() {
            self.entries.select_nth_unstable_by(end - 1, newest_first);
        }
        self.entries[..end].sort_unstable_by(newest_first);
        &self.entries[start..end]
    }
}

fn parse_time(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// One pass over everything in memory, keeping the entries that pass
/// `filter`.
pub(crate) fn collect<'a>(view: &'a View, s: &'a Snapshot, filter: &Filter) -> Timeline<'a> {
    let mut timeline = Timeline {
        entries: Vec::new(),
        kinds: BTreeMap::new(),
        people: BTreeSet::new(),
    };
    // 0 unknown, 1 passes, 2 fails: each item is checked at most once.
    let mut items = vec![0u8; s.beads.len()];
    let mut item_ok = |bead: Option<usize>| match bead {
        None => !filter.narrowed,
        Some(i) => {
            if items[i] == 0 {
                items[i] = if filter.items.matches(view, i) { 1 } else { 2 };
            }
            items[i] == 1
        }
    };
    let mut classed: HashMap<&'a str, String> = HashMap::new();
    let mut actors: BTreeSet<&'a str> = BTreeSet::new();
    let mut offer = |at_text: &'a str, bead: Option<usize>, position: usize, source: Source<'a>| {
        let Some(at) = parse_time(at_text) else {
            return;
        };
        if !filter.in_range(&at) || !item_ok(bead) {
            return;
        }
        let actor = source.actor();
        if let Some(a) = actor {
            actors.insert(a);
        }
        if let Some(person) = &filter.person {
            let Some(raw) = actor else { return };
            let name = classed
                .entry(raw)
                .or_insert_with(|| view.profile.person(raw).name);
            if name != person {
                return;
            }
        }
        *timeline.kinds.entry(source.kind()).or_default() += 1;
        if filter.event.as_ref().is_some_and(|k| !source.is_kind(k)) {
            return;
        }
        timeline.entries.push(Entry {
            at,
            at_text,
            bead,
            position,
            source,
        });
    };
    let mut created_events = vec![false; s.beads.len()];
    if let Some(store) = &view.history {
        for (id, entry) in &store.threads {
            let Some(i) = s.position(id) else { continue };
            for (n, event) in entry.thread.events.iter().enumerate() {
                created_events[i] |= event.flags.created;
                if let Some(at) = &event.at {
                    offer(at, Some(i), n, Source::Event(event));
                }
            }
            for (n, comment) in entry.thread.comments.iter().enumerate() {
                if let Some(at) = &comment.created_at {
                    offer(at, Some(i), n, Source::Comment(comment));
                }
            }
        }
    }
    for (i, bead) in s.beads.iter().enumerate() {
        if let Some(at) = bead.created_at.as_deref().filter(|_| !created_events[i]) {
            offer(at, Some(i), 0, Source::Created(bead));
        }
        if let Some(at) = bead.closed_at.as_deref().filter(|_| bead.is_closed()) {
            offer(at, Some(i), 0, Source::Closed(bead));
        }
    }
    if let Some(store) = &view.provenance {
        for (id, trail) in &store.trails {
            let Some(i) = s.position(id) else { continue };
            for (n, event) in trail.events.iter().enumerate() {
                let at = if event.occurred_at.is_empty() {
                    &event.created_at
                } else {
                    &event.occurred_at
                };
                offer(at, Some(i), n, Source::Trail(event));
            }
        }
    }
    if let Some(index) = view.run_index() {
        for run in &index.runs {
            let bead = [&run.subject.reference, &run.subject.id]
                .into_iter()
                .flatten()
                .find_map(|key| s.find(key))
                .and_then(|b| s.position(&b.id));
            if let Some(at) = &run.started_at {
                offer(at, bead, 0, Source::RunStart(run));
            }
            if let Some(at) = &run.ended_at {
                offer(at, bead, 1, Source::RunEnd(run));
            }
        }
    }
    timeline.people = actors
        .into_iter()
        .map(|raw| {
            classed
                .get(raw)
                .cloned()
                .unwrap_or_else(|| view.profile.person(raw).name)
        })
        .collect();
    timeline
}

/// A value from a `changes` pair as text.
fn value_text(value: &Value) -> String {
    match value {
        Value::Null => "—".into(),
        Value::String(s) if s.is_empty() => "—".into(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(value_text).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// At most `max` characters of the first non-empty line.
fn line(text: &str, max: usize) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = first.chars().take(max).collect();
    if first.chars().count() > max {
        out.push('…');
    }
    out
}

const DETAIL: usize = 160;

/// The badge text and tone for an entry.
fn what(entry: &Entry) -> (String, Tone) {
    match entry.source {
        Source::Event(e) => (e.label.clone(), e.tone),
        Source::Comment(_) => ("Comment".into(), Tone::Neutral),
        Source::Created(_) => ("Created".into(), Tone::Neutral),
        Source::Closed(_) => ("Closed".into(), Tone::Good),
        Source::Trail(e) => (words(&e.kind), Tone::Neutral),
        Source::RunStart(_) => ("Run started".into(), Tone::Waiting),
        Source::RunEnd(r) => (format!("Run {}", words(&r.state)), run_tone(&r.state)),
    }
}

/// The one-line detail, as HTML.
fn detail(view: &View, s: &Snapshot, entry: &Entry) -> String {
    match entry.source {
        Source::Event(e) => {
            let text = e
                .changes
                .first()
                .map(|(field, old, new)| {
                    format!("{}: {} → {}", field, value_text(old), value_text(new))
                })
                .or_else(|| e.notes.first().cloned())
                .unwrap_or_default();
            escape(&line(&text, DETAIL))
        }
        Source::Comment(c) => escape(&line(&c.text, DETAIL)),
        Source::Created(_) => {
            let kind = entry
                .bead
                .and_then(|i| s.derived.rows.get(i))
                .and_then(|r| r.kind.as_deref())
                .and_then(|k| view.profile.kind(k))
                .map(|k| k.label.clone());
            let bead = entry.bead.map(|i| &s.beads[i]);
            escape(&kind.unwrap_or_else(|| bead.map(|b| words(&b.issue_type)).unwrap_or_default()))
        }
        Source::Closed(b) => escape(&line(&b.close_reason, DETAIL)),
        Source::Trail(e) => {
            let text = if e.reference.is_empty() {
                e.payload_line()
            } else {
                e.reference.clone()
            };
            format!("provenance · {}", escape(&line(&text, DETAIL)))
        }
        Source::RunStart(r) | Source::RunEnd(r) => {
            let mut out = run_link(&r.run, &r.run);
            if let Some(code) = r
                .stop_code
                .as_deref()
                .filter(|_| matches!(entry.source, Source::RunEnd(_)))
            {
                write!(out, " · {}", escape(code)).unwrap();
            }
            out
        }
    }
}

/// The entries as a list.
fn rows(view: &View, s: &Snapshot, entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "<p class=\"empty\">Nothing recorded in this range matches.</p>".into();
    }
    let mut out = "<ol class=\"activity-list\">".to_string();
    for entry in entries {
        let (label, tone) = what(entry);
        let item = match entry.bead.map(|i| &s.beads[i]) {
            Some(b) => {
                let reference = if b.external_ref.is_empty() {
                    &b.id
                } else {
                    &b.external_ref
                };
                format!(
                    "{} <span class=\"muted\">{}</span>",
                    bead_link(&b.id, &b.title),
                    escape(reference)
                )
            }
            None => match entry.source {
                Source::RunStart(r) | Source::RunEnd(r) => {
                    let subject = r
                        .subject
                        .title
                        .as_deref()
                        .or(r.subject.reference.as_deref())
                        .or(r.subject.id.as_deref())
                        .unwrap_or(&r.run);
                    format!("<span>{}</span>", escape(subject))
                }
                _ => String::new(),
            },
        };
        let actor = entry
            .source
            .actor()
            .map(|raw| {
                format!(
                    " <span class=\"activity-actor\">by {}</span>",
                    person_link(&view.profile.person(raw).name)
                )
            })
            .unwrap_or_default();
        let detail = detail(view, s, entry);
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!("<p class=\"activity-detail\">{detail}</p>")
        };
        write!(
            out,
            "<li>{}<span class=\"activity-what\">{}</span><div class=\"activity-main\">{item}{actor}</div>{detail}</li>",
            when(entry.at_text),
            tone_badge(&label, tone),
        )
        .unwrap();
    }
    out.push_str("</ol>");
    out
}

/// The latest `limit` entries for `facets` (a person page's recent
/// activity), with how many there are in all.
pub(crate) fn recent(view: &View, facets: &Facets, limit: usize) -> (String, usize) {
    let Some(s) = view.snapshot() else {
        return (String::new(), 0);
    };
    let filter = Filter::new(view, facets, Utc::now());
    let mut timeline = collect(view, s, &filter);
    let total = timeline.len();
    let shown = timeline.page(0, limit);
    (rows(view, s, shown), total)
}

/// Links to common ranges, the current one marked.
pub(crate) fn range_links(path: &str, facets: &Facets) -> String {
    let current = range(facets);
    let mut out = "<nav class=\"range-links\" aria-label=\"Date range\">".to_string();
    for (label, days, from) in [
        ("7 days", "7", ""),
        ("30 days", "30", ""),
        ("90 days", "90", ""),
        ("1 year", "365", ""),
        ("All time", "", "1970-01-01"),
    ] {
        let on = if days.is_empty() {
            current.days.is_none()
                && current.from.map(|f| f.to_string()).as_deref() == Some(from)
                && current.to.is_none()
        } else {
            current.days.map(|d| d.to_string()).as_deref() == Some(days)
        };
        let link = href(path, facets, &[("days", days), ("from", from), ("to", "")]);
        write!(
            out,
            "<a href=\"{}\"{}>{label}</a>",
            escape(&link),
            if on { " aria-current=\"true\"" } else { "" }
        )
        .unwrap();
    }
    out.push_str("</nav>");
    out
}

/// The filter form shared by /activity and /people: search, item kind,
/// person (when `people` is given), event kind (when `kinds` is given), one
/// select per profile group, and a from/to range.
pub(crate) fn filter_form(
    view: &View,
    facets: &Facets,
    path: &str,
    people: Option<&BTreeSet<String>>,
    kinds: Option<&BTreeMap<Cow<str>, usize>>,
) -> String {
    let mut out = format!(
        "<form class=\"filters\" action=\"{}\" method=\"get\"><label>Search items<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Title, id or label…\"></label>",
        escape(path),
        escape(&facets.q)
    );
    let any = |label: &str| vec![(String::new(), label.to_string())];
    if let Some(people) = people {
        let mut options = any("Anyone");
        options.extend(people.iter().map(|p| (p.clone(), p.clone())));
        if !facets.person.is_empty() && !people.contains(&facets.person) {
            options.push((facets.person.clone(), facets.person.clone()));
        }
        out.push_str(&select("person", "Person", &facets.person, &options));
    }
    if let Some(kinds) = kinds.filter(|_| view.profile.group(EVENT_KEY).is_none()) {
        let current = facets
            .groups
            .get(EVENT_KEY)
            .map(String::as_str)
            .unwrap_or("");
        let mut options = any("Any event");
        options.extend(
            kinds
                .iter()
                .map(|(k, n)| (k.to_string(), format!("{} ({n})", kind_label(view, k)))),
        );
        if !current.is_empty() && !kinds.contains_key(current) {
            options.push((current.to_string(), kind_label(view, current)));
        }
        out.push_str(&select(EVENT_KEY, "Event", current, &options));
    }
    if !view.profile.kinds.is_empty() {
        let mut options = any("All kinds");
        options.extend(
            view.profile
                .kinds
                .iter()
                .map(|k| (k.key.clone(), k.label.clone())),
        );
        out.push_str(&select("kind", "Item kind", &facets.kind, &options));
    }
    if let Some(s) = view.snapshot() {
        for group in &view.profile.groups {
            let mut values = any(&format!("Any {}", group.label.to_lowercase()));
            if let Some(index) = s.derived.groups.get(&group.key) {
                values.extend(index.keys().map(|v| (v.clone(), v.clone())));
            }
            let current = facets
                .groups
                .get(&group.key)
                .map(String::as_str)
                .unwrap_or("");
            out.push_str(&select(&group.key, &group.label, current, &values));
        }
    }
    write!(
        out,
        "<label>From<input type=\"date\" name=\"from\" value=\"{}\"></label><label>To<input type=\"date\" name=\"to\" value=\"{}\"></label><button type=\"submit\">Apply</button></form>",
        escape(&facets.from),
        escape(&facets.to)
    )
    .unwrap();
    out
}

/// `/activity`.
pub(crate) fn activity(view: &View, facets: &Facets) -> String {
    let Some(s) = view.snapshot() else {
        return super::layout::not_found(view);
    };
    let now = Utc::now();
    let filter = Filter::new(view, facets, now);
    let mut timeline = collect(view, s, &filter);
    let mut body = heading(
        "Work",
        "Activity",
        "Everything recorded across the items, newest first: history events, comments, created and closed dates, provenance and runs.",
    );
    body.push_str(&filter_form(
        view,
        facets,
        "/activity",
        Some(&timeline.people),
        Some(&timeline.kinds),
    ));
    body.push_str(&range_links("/activity", facets));
    let total = timeline.len();
    let pages = total.div_ceil(PAGE_SIZE).max(1);
    let current = facets.page().min(pages - 1);
    write!(
        body,
        "<p class=\"muted\">{total} entries · {} · page {} of {pages}{}</p>",
        escape(&range_words(&range(facets), now)),
        current + 1,
        coverage(view, s)
    )
    .unwrap();
    let shown = timeline.page(current, PAGE_SIZE);
    body.push_str(&rows(view, s, shown));
    let query = facets.pairs();
    let query: Vec<(&str, &str)> = query
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    body.push_str(&pagination("/activity", &query, current, pages));
    page(view, "Activity", "/activity", &body)
}

/// " · history read for N of M items" while threads are still being read.
pub(crate) fn coverage(view: &View, s: &Snapshot) -> String {
    match &view.history {
        None => " · history is not read for this store".into(),
        Some(store) if store.threads.len() < s.beads.len() => format!(
            " · history read for {} of {} beads so far",
            store.threads.len(),
            s.beads.len()
        ),
        Some(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facets(query: &str) -> Facets {
        let uri: axum::http::Uri = format!("/x?{query}").parse().unwrap();
        axum::extract::Query::<Facets>::try_from_uri(&uri)
            .unwrap()
            .0
    }

    fn kinds(view: &View, query: &str) -> Vec<String> {
        let s = view.snapshot().unwrap();
        let filter = Filter::new(view, &facets(query), Utc::now());
        let mut t = collect(view, s, &filter);
        let n = t.len();
        t.page(0, n)
            .iter()
            .map(|e| format!("{}@{}", e.source.kind(), e.at_text))
            .collect()
    }

    #[test]
    fn sources_merge_newest_first_with_stable_ties() {
        let view = crate::testing::work_view();
        let all = kinds(&view, "from=2026-10-01&to=2026-11-30");
        let times: Vec<&str> = all.iter().map(|k| k.split('@').nth(1).unwrap()).collect();
        let mut sorted = times.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(times, sorted, "newest first: {all:?}");
        // Every source is present.
        for kind in [
            "claim",
            "comment",
            "authored",
            "closed",
            "suspended",
            "run:start",
            "run:end",
        ] {
            assert!(
                all.iter().any(|k| k.starts_with(&format!("{kind}@"))),
                "{kind}: {all:?}"
            );
        }
        assert!(all.iter().any(|k| k.starts_with("trail:")), "{all:?}");
        // A created date gives way to the history event that records it.
        assert!(
            !all.iter().any(|k| k == "created@2026-11-10T09:00:00Z"),
            "{all:?}"
        );
        // Same order again: ties never depend on hash order.
        assert_eq!(all, kinds(&view, "from=2026-10-01&to=2026-11-30"));
        // Paging orders only what it shows, and agrees with the full order.
        let s = view.snapshot().unwrap();
        let filter = Filter::new(&view, &facets("from=2026-10-01&to=2026-11-30"), Utc::now());
        let mut t = collect(&view, s, &filter);
        let second: Vec<&str> = t.page(1, 3).iter().map(|e| e.at_text).collect();
        assert_eq!(second, times[3..6]);
    }

    #[test]
    fn filters_select_person_kind_items_and_range() {
        let view = crate::testing::work_view();
        let alice = kinds(&view, "person=alice&from=2026-10-01");
        assert!(!alice.is_empty());
        let seat = kinds(&view, "person=factory%2FhostA%2FrigA&from=2026-10-01");
        assert!(seat.iter().all(|k| !k.starts_with("comment@")), "{seat:?}");
        assert!(seat.iter().any(|k| k.starts_with("claim@")));
        assert_eq!(
            kinds(&view, "event=suspended&from=2026-10-01"),
            ["suspended@2026-11-15T09:15:00Z"]
        );
        // A plain comment is its own kind.
        assert!(kinds(&view, "event=comment&from=2026-10-01").len() >= 2);
        // Item facets narrow by item; runs with no item then drop out.
        let rig_b = kinds(&view, "rig=rigB&from=2026-10-01");
        assert!(
            rig_b.iter().any(|k| k.starts_with("suspended@")),
            "{rig_b:?}"
        );
        assert!(!rig_b.iter().any(|k| k.starts_with("authored@")));
        // The default range is the last seven days.
        let (from, _) = Filter::new(&view, &Facets::default(), Utc::now()).bounds;
        assert_eq!(
            from,
            Some(Utc::now().date_naive() - chrono::Duration::days(6))
        );
        assert!(kinds(&view, "from=2030-01-01").is_empty());
    }

    #[test]
    fn the_page_escapes_and_links_people_and_items() {
        let view = crate::testing::work_view();
        let html = activity(&view, &facets("from=2026-10-01"));
        assert!(html.contains("href=\"/person/factory%2FhostA%2FrigA\""));
        assert!(html.contains("href=\"/bead/wk%2D1\""));
        assert!(html.contains("Looks right to me {mostly}; check the **header row**."));
        assert!(
            html.contains("claimed_by: factory/hostA/rigB → —"),
            "first change"
        );
        assert!(html.contains("status: working → done"));
        assert!(html.contains("<select name=\"event\""));
        assert!(!html.contains(" style="));
        assert!(html.contains("href=\"/run/run%2D0003\""));
    }
}
