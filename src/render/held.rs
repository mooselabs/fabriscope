// Owned by WP6 — Held, costs, receipts, sweeper. Frozen signature:
// `held(&View, &Facets) -> String`.
//! `/held`: beads in a state that holds them back, with why, since when and
//! what to do next, and the history of stops from every source read.
//!
//! A state holds work back when it `overrides_ready` or has the bad tone
//! (`Profile::has_held`). Rows group by any of the state's `facts` or by a
//! profile group (`?by=`). The run feed joins a row through its attempt id
//! (the provenance attempt, else a fact whose value names a run's attempt),
//! else through the newest stopped run of the item.

use super::costs::{hidden, range_controls, range_or_default, range_words, tiles};
use super::fmt::{seconds, when, words};
use super::layout::page;
use super::svg::{Bar, hbars};
use super::{bead_link, escape, facets_form, heading, run_link, select, url};
use crate::beads::Bead;
use crate::model::View;
use crate::profile::{Source, StateRule, Tone};
use crate::provenance::{self, Progress};
use crate::query::Facets;
use crate::runs::{RunSummary, Stop, cache::Fetch};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::fmt::Write;

/// Rows a stop-history list shows, newest first.
const SHOWN: usize = 50;

/// A fact's column heading: its `[labels]` entry, else the source's last
/// segment in words.
fn fact_label(view: &View, source: &Source) -> String {
    let key = source.to_string();
    if let Some(label) = view.profile.fact_label(&key) {
        return label.to_string();
    }
    let last = match source {
        Source::Meta(parts) | Source::Ref(parts) => parts.last().cloned().unwrap_or(key),
        Source::Field(name) => name.clone(),
        Source::Label(prefix) => prefix.trim_end_matches([':', '/', '-']).to_string(),
        _ => key,
    };
    let mut text = words(&last);
    if let Some(first) = text.get(..1) {
        text = first.to_uppercase() + &text[1..];
    }
    text
}

/// The nearest-rank percentile `p` (0–1) of sorted `values`.
pub(super) fn nearest_rank(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil().max(1.0) as usize;
    Some(sorted[rank.min(sorted.len()) - 1])
}

fn instant(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// What a row knows about its run.
struct Joined<'a> {
    attempt: Option<String>,
    /// Whether the provenance trail was read.
    trail: bool,
    run: Option<&'a RunSummary>,
}

fn join<'a>(view: &'a View, bead: &Bead, state: &StateRule) -> Joined<'a> {
    let trail = view.trail(&bead.id);
    let mut attempt = trail.and_then(|t| {
        Progress::new(bead, &t.events)
            .attempt
            .filter(|a| !a.is_empty())
    });
    let mut run = attempt.as_deref().and_then(|a| view.run_for_attempt(a));
    if run.is_none() {
        // A fact naming an attempt the run feed knows.
        for fact in &state.facts {
            for value in fact.texts(bead) {
                if let Some(found) = view.run_for_attempt(&value) {
                    attempt.get_or_insert(value);
                    run = Some(found);
                    break;
                }
            }
        }
    }
    if run.is_none() {
        run = view
            .runs_for(bead)
            .into_iter()
            .find(|r| r.stop_code.is_some() || r.state == "stopped");
    }
    Joined {
        attempt,
        trail: trail.is_some(),
        run,
    }
}

/// The run's stop, from its doc when read: the attempt's own stop, else
/// the run's.
fn run_stop(view: &View, run: &RunSummary, attempt: Option<&str>) -> Result<Option<Stop>, String> {
    match view.run_cache.get_run(&run.run) {
        Fetch::Ready(doc) => Ok(attempt
            .and_then(|a| doc.attempts.iter().find(|x| x.id == a))
            .and_then(|a| a.stop.clone())
            .filter(|s| s.title.is_some() || s.why.is_some())
            .or_else(|| doc.stop.clone())),
        Fetch::Pending => Err("Run details not read yet.".into()),
        Fetch::Failed(error) => Err(format!("Run details could not be read: {error}")),
    }
}

fn run_cell(view: &View, joined: &Joined) -> String {
    let Some(run) = joined.run else {
        return if view.runs.is_some() {
            "<span class=\"muted\">no run in the feed</span>".into()
        } else {
            "<span class=\"muted\">no run feed</span>".into()
        };
    };
    let mut out = format!(
        "{} <span class=\"muted\">{}</span>",
        run_link(&run.run, &run.run),
        escape(&words(&run.state))
    );
    match run_stop(view, run, joined.attempt.as_deref()) {
        Ok(Some(stop)) => {
            out.push_str("<div class=\"stop-note\">");
            write!(
                out,
                "<strong>{}</strong>",
                escape(stop.title.as_deref().unwrap_or(&stop.code))
            )
            .unwrap();
            if let Some(why) = &stop.why {
                write!(out, "<small>{}</small>", escape(why)).unwrap();
            }
            if let Some(next) = &stop.next {
                write!(out, "<small>Next: {}</small>", escape(next)).unwrap();
            }
            out.push_str("</div>");
        }
        Ok(None) => {
            if let Some(code) = &run.stop_code {
                write!(out, "<small>stop {}</small>", escape(code)).unwrap();
            }
        }
        Err(note) => {
            if let Some(code) = &run.stop_code {
                write!(out, "<small>stop {}</small>", escape(code)).unwrap();
            }
            write!(out, "<small class=\"muted\">{}</small>", escape(&note)).unwrap();
        }
    }
    out
}

/// What rows of a state group by: one of its facts, else a profile group.
enum Grouping<'a> {
    Fact(&'a Source),
    Group(&'a str, &'a str),
    None,
}

fn grouping<'a>(view: &'a View, state: &'a StateRule, by: &str) -> Grouping<'a> {
    if let Some(key) = by.strip_prefix("group:") {
        if let Some(group) = view.profile.group(key) {
            return Grouping::Group(&group.key, &group.label);
        }
    }
    if let Some(fact) = state.facts.iter().find(|f| f.to_string() == by) {
        return Grouping::Fact(fact);
    }
    match state.facts.first() {
        Some(fact) => Grouping::Fact(fact),
        None => match view.profile.groups.first() {
            Some(g) => Grouping::Group(&g.key, &g.label),
            None => Grouping::None,
        },
    }
}

/// `/held`.
pub(crate) fn held(view: &View, facets: &Facets) -> String {
    let p = &view.profile;
    // `by` chooses what rows group by: a fact source as the profile writes
    // it, or `group:<key>`.
    let by = facets.by.as_str();
    let held: Vec<&StateRule> = p
        .states
        .iter()
        .filter(|s| s.overrides_ready || s.tone == Tone::Bad)
        .collect();
    let mut body = heading(
        "Factory",
        "Held",
        "Work a state holds back: why, since when, and what the run feed says to do next.",
    );
    if held.is_empty() {
        body.push_str("<p class=\"empty\">No state in the profile holds work back, so nothing can be held. A profile marks one with <code>overrides_ready = true</code> or <code>tone = \"bad\"</code> under <code>[[states]]</code>; start Fabriscope with <code>--profile</code> to use one.</p>");
        return page(view, "Held", "/held", &body);
    }
    let Some(s) = view.snapshot() else {
        return page(view, "Held", "/held", &body);
    };
    let now = Utc::now();
    // Rows per held state, single pass.
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); held.len()];
    for i in 0..s.beads.len() {
        let states = &s.derived.rows[i].states;
        if states.is_empty() || !facets.matches(view, i) {
            continue;
        }
        for (n, state) in held.iter().enumerate() {
            if states.contains(&state.key) {
                rows[n].push(i);
            }
        }
    }
    body.push_str(&facets_form(view, facets, "/held"));
    // Tiles: each state, then the commonest values of the first state's
    // first fact.
    let mut cards: Vec<(&str, String, String)> = held
        .iter()
        .zip(&rows)
        .map(|(state, r)| (state.label.as_str(), r.len().to_string(), "held now".into()))
        .collect();
    let mut first_counts: Vec<(String, usize)> = Vec::new();
    if let Some(fact) = held[0].facts.first() {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for &i in &rows[0] {
            for value in values_or_none(fact.texts(&s.beads[i])) {
                *counts.entry(value).or_default() += 1;
            }
        }
        first_counts = counts.into_iter().collect();
        first_counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let label = fact_label(view, fact);
        for (value, n) in first_counts.iter().take(4) {
            cards.push((value.as_str(), n.to_string(), label.clone()));
        }
    }
    body.push_str(&tiles(&cards));
    if let Some(fact) = held[0].facts.first() {
        let bars: Vec<Bar> = first_counts
            .iter()
            .take(12)
            .map(|(value, n)| Bar {
                label: value.clone(),
                value: *n as f64,
                title: format!("{value}: {n}"),
                tone: held[0].tone,
            })
            .collect();
        body.push_str(&hbars(
            &format!(
                "{} by {}",
                held[0].label,
                fact_label(view, fact).to_lowercase()
            ),
            &bars,
            |v| format!("{v:.0}"),
        ));
    }
    body.push_str(&group_form(view, &held, facets, by));
    for (state, members) in held.iter().zip(&rows) {
        body.push_str(&state_section(view, state, members, by, now));
    }
    body.push_str(&stop_history(view, facets, now));
    page(view, "Held", "/held", &body)
}

fn values_or_none(values: Vec<String>) -> Vec<String> {
    if values.is_empty() {
        vec!["(none)".into()]
    } else {
        values
    }
}

/// The group-by and range controls, carrying the list filters.
fn group_form(view: &View, held: &[&StateRule], facets: &Facets, by: &str) -> String {
    let mut options: Vec<(String, String)> = Vec::new();
    for state in held {
        for fact in &state.facts {
            let key = fact.to_string();
            if !options.iter().any(|(k, _)| *k == key) {
                options.push((key, fact_label(view, fact)));
            }
        }
    }
    for group in &view.profile.groups {
        options.push((format!("group:{}", group.key), group.label.clone()));
    }
    let mut out =
        "<form class=\"filters held-controls\" action=\"/held\" method=\"get\">".to_string();
    if !options.is_empty() {
        let current = if by.is_empty() { &options[0].0 } else { by };
        out.push_str(&select("by", "Group by", current, &options));
    }
    out.push_str(&range_controls(facets));
    out.push_str(&hidden(facets, &["days", "from", "to", "by"]));
    out.push_str("<button type=\"submit\">Apply</button></form>");
    out
}

fn state_section(
    view: &View,
    state: &StateRule,
    members: &[usize],
    by: &str,
    now: DateTime<Utc>,
) -> String {
    let s = view.snapshot().expect("checked by the caller");
    let mut out = format!(
        "<section class=\"held-state\" id=\"held-{}\"><h2>{} <span class=\"muted\">{}</span></h2>",
        url(&state.key),
        escape(&state.label),
        members.len()
    );
    if members.is_empty() {
        out.push_str("<p class=\"empty\">Nothing is in this state now.</p></section>");
        return out;
    }
    let grouping = grouping(view, state, by);
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for &i in members {
        let keys = match &grouping {
            Grouping::Fact(fact) => values_or_none(fact.texts(&s.beads[i])),
            Grouping::Group(key, _) => values_or_none(
                s.derived.rows[i]
                    .groups
                    .get(*key)
                    .cloned()
                    .unwrap_or_default(),
            ),
            Grouping::None => vec![String::new()],
        };
        for key in keys {
            groups.entry(key).or_default().push(i);
        }
    }
    let mut groups: Vec<(String, Vec<usize>)> = groups.into_iter().collect();
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    let heading = match &grouping {
        Grouping::Fact(fact) => fact_label(view, fact),
        Grouping::Group(_, label) => label.to_string(),
        Grouping::None => String::new(),
    };
    for (value, items) in groups {
        write!(
            out,
            "<details class=\"held-group\" id=\"held-{}-{}\" open><summary>{}{} <span class=\"muted\">· {}</span></summary>",
            url(&state.key),
            url(&value),
            if heading.is_empty() {
                String::new()
            } else {
                format!("{}: ", escape(&heading))
            },
            escape(if value.is_empty() { "all" } else { &value }),
            items.len()
        )
        .unwrap();
        out.push_str(&held_table(view, state, &items, now));
        out.push_str("</details>");
    }
    out.push_str("</section>");
    out
}

fn held_table(view: &View, state: &StateRule, items: &[usize], now: DateTime<Utc>) -> String {
    let s = view.snapshot().expect("checked by the caller");
    let mut out = "<div class=\"table-wrap\"><table class=\"data held\"><thead><tr><th>Item</th><th>Kind</th>".to_string();
    for fact in &state.facts {
        write!(out, "<th>{}</th>", escape(&fact_label(view, fact))).unwrap();
    }
    out.push_str("<th>Held for</th><th>Attempt</th><th>Run</th></tr></thead><tbody>");
    // Longest held first.
    let mut items = items.to_vec();
    items.sort_by(|&a, &b| s.derived.rows[a].since.cmp(&s.derived.rows[b].since));
    for i in items {
        let bead = &s.beads[i];
        let row = &s.derived.rows[i];
        let reference = if bead.external_ref.is_empty() {
            &bead.id
        } else {
            &bead.external_ref
        };
        let kind = row
            .kind
            .as_deref()
            .and_then(|k| view.profile.kind(k))
            .map(|k| escape(&k.label))
            .unwrap_or_else(|| escape(&words(&bead.issue_type)));
        write!(
            out,
            "<tr><td>{}<small>{}</small></td><td>{kind}</td>",
            bead_link(&bead.id, &bead.title),
            escape(reference)
        )
        .unwrap();
        for fact in &state.facts {
            let text = fact.text(bead);
            write!(
                out,
                "<td>{}</td>",
                text.map(|t| escape(&t))
                    .unwrap_or_else(|| "<span class=\"muted\">—</span>".into())
            )
            .unwrap();
        }
        let held_for = match row.since.as_deref() {
            Some(since) => match provenance::elapsed(since, now) {
                Some(age) => format!("{}<small>since {}</small>", escape(&age), when(since)),
                None => escape(since),
            },
            None => super::unknown().into(),
        };
        let joined = join(view, bead, state);
        let attempt = match (&joined.attempt, joined.trail) {
            (Some(a), _) => format!("<code>{}</code>", escape(a)),
            (None, false) if view.provenance.is_some() => {
                "<span class=\"muted\">not read yet</span>".into()
            }
            _ => "<span class=\"muted\">—</span>".into(),
        };
        write!(
            out,
            "<td>{held_for}</td><td>{attempt}</td><td>{}</td></tr>",
            run_cell(view, &joined)
        )
        .unwrap();
    }
    out.push_str("</tbody></table></div>");
    out
}

/// One stop from any source.
struct StopRow {
    at: String,
    item: Option<usize>,
    what: String,
    detail: String,
}

/// Stops in range from history, provenance and the run feed, each source
/// apart and each saying how much of the work it covers.
fn stop_history(view: &View, facets: &Facets, now: DateTime<Utc>) -> String {
    let Some(s) = view.snapshot() else {
        return String::new();
    };
    let (range, default) = range_or_default(facets);
    // Stops are history: no state filter means every state.
    let items = facets.any_status();
    let in_range = |at: &str| range.contains(at, now);
    let mut considered = 0;
    let mut threads = 0;
    let mut trails = 0;
    let mut history_rows = Vec::new();
    let mut suspends = Vec::new();
    let mut resumed: Vec<f64> = Vec::new();
    let mut open = 0;
    for i in 0..s.beads.len() {
        if !items.matches(view, i) {
            continue;
        }
        considered += 1;
        let bead = &s.beads[i];
        if let Some(thread) = view.thread(&bead.id) {
            threads += 1;
            for event in thread.events.iter().filter(|e| e.flags.stop) {
                let Some(at) = event.at.as_deref().filter(|a| in_range(a)) else {
                    continue;
                };
                history_rows.push(StopRow {
                    at: at.to_string(),
                    item: Some(i),
                    what: event.label.clone(),
                    detail: [event.actor.clone()]
                        .into_iter()
                        .chain(event.notes.iter().cloned())
                        .filter(|t| !t.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · "),
                });
            }
        }
        if let Some(trail) = view.trail(&bead.id) {
            trails += 1;
            let mut pending: Option<&provenance::Event> = None;
            let mut pairs: Vec<(&provenance::Event, Option<&provenance::Event>)> = Vec::new();
            for event in &trail.events {
                match event.kind.as_str() {
                    "suspend" => {
                        if let Some(stop) = pending.replace(event) {
                            pairs.push((stop, None));
                        }
                    }
                    "resume" => {
                        if let Some(stop) = pending.take() {
                            pairs.push((stop, Some(event)));
                        }
                    }
                    _ => {}
                }
            }
            pairs.extend(pending.map(|stop| (stop, None)));
            for (stop, resume) in pairs {
                if !in_range(&stop.occurred_at) {
                    continue;
                }
                let took = resume.and_then(|r| {
                    Some(
                        (instant(&r.occurred_at)? - instant(&stop.occurred_at)?).num_seconds()
                            as f64,
                    )
                });
                match took {
                    Some(t) => resumed.push(t.max(0.0)),
                    None if resume.is_none() => open += 1,
                    None => {}
                }
                let fields: Vec<String> = stop
                    .field_text(&["attempt", "digest", "incident"])
                    .into_iter()
                    .map(|(k, v)| format!("{} {v}", words(&k)))
                    .collect();
                suspends.push(StopRow {
                    at: stop.occurred_at.clone(),
                    item: Some(i),
                    what: fields.join(" · "),
                    detail: match (resume, took) {
                        (Some(r), Some(t)) => {
                            format!("resumed after {} by {}", seconds(t), r.actor)
                        }
                        (Some(r), None) => format!("resumed by {}", r.actor),
                        (None, _) => "not resumed".into(),
                    },
                });
            }
        }
    }
    let mut feed_rows = Vec::new();
    let mut by_code: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(index) = view.run_index() {
        let picking = super::costs::picks_items(facets);
        for run in &index.runs {
            let Some(code) = &run.stop_code else {
                continue;
            };
            let at = run
                .ended_at
                .clone()
                .unwrap_or_else(|| run.updated_at.clone());
            if !in_range(&at) {
                continue;
            }
            let item = run
                .subject
                .reference
                .iter()
                .chain(&run.subject.id)
                .find_map(|k| s.find(k))
                .and_then(|b| s.position(&b.id));
            let keep = match item {
                Some(i) => items.matches(view, i),
                None => !picking,
            };
            if !keep {
                continue;
            }
            *by_code.entry(code.clone()).or_default() += 1;
            feed_rows.push(StopRow {
                at,
                item,
                what: code.clone(),
                detail: run.run.clone(),
            });
        }
    }
    resumed.sort_by(f64::total_cmp);
    let mut out = format!(
        "<section id=\"stop-history\"><h2>Stop history</h2><p class=\"muted\">Stops in {}{}, from every source read. History read for {threads} of {considered} items; provenance read for {trails} of {considered} items{}.</p>",
        escape(&range_words(range, now)),
        if default { " (the default)" } else { "" },
        if view.run_index().is_some() {
            "; the run feed lists its own stops"
        } else {
            "; no run feed"
        }
    );
    let p50 = nearest_rank(&resumed, 0.5);
    let p90 = nearest_rank(&resumed, 0.9);
    let span = |v: Option<f64>| v.map(seconds).unwrap_or_else(|| "unknown".into());
    out.push_str(&tiles(&[
        (
            "History stop events",
            history_rows.len().to_string(),
            "events the profile flags as stops".into(),
        ),
        (
            "Provenance suspends",
            suspends.len().to_string(),
            format!("{} resumed · {open} still open", resumed.len()),
        ),
        (
            "Time to resume",
            escape(&span(p50)),
            format!("median · p90 {} · nearest rank", span(p90)),
        ),
        (
            "Run feed stops",
            feed_rows.len().to_string(),
            format!("{} stop codes", by_code.len()),
        ),
    ]));
    if !by_code.is_empty() {
        let mut codes: Vec<(String, usize)> = by_code.into_iter().collect();
        codes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let bars: Vec<Bar> = codes
            .iter()
            .take(12)
            .map(|(code, n)| Bar {
                label: code.clone(),
                value: *n as f64,
                title: format!("{code}: {n}"),
                tone: Tone::Warn,
            })
            .collect();
        out.push_str(&hbars("Run feed stops by stop code", &bars, |v| {
            format!("{v:.0}")
        }));
    }
    for (id, title, mut rows, what, detail) in [
        (
            "stops-history",
            "From history",
            history_rows,
            "Event",
            "By · notes",
        ),
        (
            "stops-provenance",
            "From provenance",
            suspends,
            "Suspended with",
            "Resumed",
        ),
        (
            "stops-feed",
            "From the run feed",
            feed_rows,
            "Stop code",
            "Run",
        ),
    ] {
        write!(
            out,
            "<details class=\"stop-list\" id=\"{id}\"><summary>{title} <span class=\"muted\">· {}</span></summary>",
            rows.len()
        )
        .unwrap();
        if rows.is_empty() {
            out.push_str("<p class=\"empty\">None in this range.</p></details>");
            continue;
        }
        rows.sort_by(|a, b| b.at.cmp(&a.at));
        write!(out, "<div class=\"table-wrap\"><table class=\"data\"><thead><tr><th>At</th><th>Item</th><th>{what}</th><th>{detail}</th></tr></thead><tbody>").unwrap();
        for row in rows.iter().take(SHOWN) {
            let item = row
                .item
                .map(|i| bead_link(&s.beads[i].id, &s.beads[i].title))
                .unwrap_or_else(|| "<span class=\"muted\">(not in the ledger)</span>".into());
            let detail = if id == "stops-feed" {
                run_link(&row.detail, &row.detail)
            } else {
                escape(&row.detail)
            };
            write!(
                out,
                "<tr><td>{}</td><td>{item}</td><td>{}</td><td>{detail}</td></tr>",
                when(&row.at),
                escape(&row.what)
            )
            .unwrap();
        }
        out.push_str("</tbody></table></div>");
        if rows.len() > SHOWN {
            write!(
                out,
                "<p class=\"muted\">The newest {SHOWN} of {}.</p>",
                rows.len()
            )
            .unwrap();
        }
        out.push_str("</details>");
    }
    out.push_str("</section>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facets(query: &str) -> Facets {
        let uri: axum::http::Uri = format!("/held?{query}").parse().unwrap();
        axum::extract::Query::<Facets>::try_from_uri(&uri)
            .unwrap()
            .0
    }

    #[test]
    fn nearest_rank_percentiles() {
        assert_eq!(nearest_rank(&[], 0.5), None);
        assert_eq!(nearest_rank(&[7.0], 0.9), Some(7.0));
        let v = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(nearest_rank(&v, 0.5), Some(5.0));
        assert_eq!(nearest_rank(&v, 0.9), Some(9.0));
        assert_eq!(nearest_rank(&[1.0, 2.0, 3.0], 0.5), Some(2.0));
    }

    #[test]
    fn held_work_groups_by_a_fact_and_joins_its_run_stop() {
        let view = crate::testing::work_view();
        let page = held(&view, &facets("from=2026-11-01&to=2026-11-30"));
        assert!(page.contains("id=\"held-held\""), "{page}");
        assert!(page.contains("Move reports to the new table"));
        assert!(
            page.contains("id=\"held-held-scope%2Dexceeded\""),
            "grouped by the first fact"
        );
        assert!(page.contains("<code>att-0003-1</code>"));
        assert!(page.contains("The change needs a file outside its write scope"));
        assert!(page.contains("Next: Widen the write scope"));
        assert!(page.contains("since <time"));
        assert!(page.contains("History read for"));
        // wk-2 was suspended and resumed 14 minutes later.
        assert!(page.contains("resumed after 14m 0s by alice"), "{page}");
        assert!(page.contains("/run/run%2D0003"));
        let by_cause = held(&view, &facets("by=work.suspended.cause"));
        assert!(by_cause.contains("id=\"held-held-scope\""));
        let by_rig = held(&view, &facets("by=group:rig"));
        assert!(by_rig.contains("id=\"held-held-rigB\""), "{by_rig}");
        // `by` is not a group filter: the row is still listed.
        assert!(by_rig.contains("Move reports to the new table"));
        assert!(!page.contains(" style="));
    }

    #[test]
    fn without_a_holding_state_the_page_explains() {
        let mut view = crate::testing::work_view();
        view.profile = std::sync::Arc::new(crate::profile::Profile::default());
        let page = held(&view, &Facets::default());
        assert!(page.contains("No state in the profile holds work back"));
    }
}
