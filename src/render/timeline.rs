// Owned by WP2 — Bead page. Frozen signature:
// `timeline(&View, &Bead) -> String`.
//! A bead's merged timeline: its dates, history events, plain comments,
//! provenance events and run-feed steps in one time order, newest first.

use super::fmt::{when, words};
use super::markdown::{Links, markdown};
use super::panels::joined;
use super::progress::{fact, kind_words};
use super::values::{Ctx, value};
use super::{escape, person_link, tone_badge, tone_class, url};
use crate::beads::Bead;
use crate::history::Thread;
use crate::model::View;
use crate::profile::{FieldKind, Tone};
use crate::provenance;
use crate::runs::{self, cache::Fetch};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use std::fmt::Write;

/// Entries shown before the rest fold into a `<details>`.
const SHOWN: usize = 100;

/// Where an entry came from. At the same instant entries sort in this
/// order (oldest first), so a bead is created before anything happens to
/// it and closed after.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Origin {
    Created,
    History,
    Provenance,
    Comment,
    Step,
    Started,
    Closed,
}

/// One timeline entry: when, where from, a stable anchor id, its tone and
/// its markup (already escaped).
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub at: Option<DateTime<Utc>>,
    pub origin: Origin,
    pub anchor: String,
    pub tone: Tone,
    pub html: String,
}

/// An instant from an RFC 3339 time or a `YYYY-MM-DD` date (as midnight
/// UTC); `None` for anything else.
pub(crate) fn instant(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|t| t.and_utc())
        })
}

/// Sorts entries newest first. Entries at the same instant keep their
/// origin order reversed (a close shows above the step that closed it),
/// then the order they were given in reversed; undated entries go last.
pub(crate) fn merge(mut entries: Vec<Entry>) -> Vec<Entry> {
    // Oldest first, stable, then reversed.
    entries.sort_by_key(|a| (a.at, a.origin));
    let (dated, undated): (Vec<_>, Vec<_>) = entries.into_iter().partition(|e| e.at.is_some());
    let mut out: Vec<Entry> = dated.into_iter().rev().collect();
    out.extend(undated);
    out
}

/// An anchor id from parts: letters, digits, `-` and `_` kept, anything
/// else as `-`.
fn anchor(parts: &[&str]) -> String {
    let joined = parts.join("-");
    let clean: String = joined
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("t-{clean}")
}

fn line(label: &str, by: Option<&str>, at: Option<&str>, source: &str, anchor: &str) -> String {
    let mut out = format!("<strong>{label}</strong>");
    if let Some(by) = by.filter(|b| !b.is_empty()) {
        write!(out, " by {by}").unwrap();
    }
    write!(out, "<small><a href=\"#{anchor}\">").unwrap();
    match at {
        Some(at) => out.push_str(&when(at)),
        None => out.push_str("undated"),
    }
    write!(out, "</a> · {source}</small>").unwrap();
    out
}

fn dates(bead: &Bead, out: &mut Vec<Entry>) {
    for (origin, label, at) in [
        (Origin::Created, "Created", &bead.created_at),
        (Origin::Started, "Started", &bead.started_at),
        (Origin::Closed, "Closed", &bead.closed_at),
    ] {
        let Some(at) = at else { continue };
        let id = anchor(&[&label.to_lowercase()]);
        let mut html = line(label, None, Some(at), "bead", &id);
        if origin == Origin::Closed && !bead.close_reason.is_empty() {
            write!(html, "<p>{}</p>", escape(&bead.close_reason)).unwrap();
        }
        out.push(Entry {
            at: instant(at),
            origin,
            anchor: id,
            tone: if origin == Origin::Closed {
                Tone::Good
            } else {
                Tone::Neutral
            },
            html,
        });
    }
}

fn change_value(v: &Value, ctx: &Ctx) -> String {
    let html = value(FieldKind::Text, v, ctx);
    if html.is_empty() { "—".into() } else { html }
}

fn history(view: &View, thread: &Thread, links: &Links, out: &mut Vec<Entry>) {
    let ctx = Ctx::new(view);
    for (n, event) in thread.events.iter().enumerate() {
        let id = anchor(&["event", &(n + 1).to_string(), &event.kind]);
        let actor =
            (!event.actor.is_empty()).then(|| person_link(&view.profile.person(&event.actor).name));
        let mut html = line(
            &escape(&event.label),
            actor.as_deref(),
            event.at.as_deref(),
            "history",
            &id,
        );
        if !event.changes.is_empty() {
            html.push_str("<ul class=\"changes\">");
            for (field, old, new) in &event.changes {
                write!(
                    html,
                    "<li>{}: {} → {}</li>",
                    escape(&words(field)),
                    change_value(old, &ctx),
                    change_value(new, &ctx)
                )
                .unwrap();
            }
            html.push_str("</ul>");
        }
        for note in &event.notes {
            html.push_str(&markdown(note, None, links));
        }
        out.push(Entry {
            at: event.at.as_deref().and_then(instant),
            origin: Origin::History,
            anchor: id,
            tone: event.tone,
            html,
        });
    }
    for (n, comment) in thread.comments.iter().enumerate() {
        let key = comment.id.clone().unwrap_or_else(|| (n + 1).to_string());
        let id = anchor(&["comment", &key]);
        let author = comment
            .author
            .as_deref()
            .map(|a| person_link(&view.profile.person(a).name));
        let mut html = line(
            "Comment",
            author.as_deref(),
            comment.created_at.as_deref(),
            "comment",
            &id,
        );
        write!(
            html,
            "<div class=\"comment\">{}</div>",
            markdown(&comment.text, None, links)
        )
        .unwrap();
        out.push(Entry {
            at: comment.created_at.as_deref().and_then(instant),
            origin: Origin::Comment,
            anchor: id,
            tone: Tone::Neutral,
            html,
        });
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn provenance_tone(kind: &str) -> Tone {
    match kind {
        "suspend" => Tone::Bad,
        "handoff" => Tone::Warn,
        "land" | "resume" => Tone::Good,
        _ => Tone::Neutral,
    }
}

fn provenance(view: &View, trail: &provenance::Trail, out: &mut Vec<Entry>) {
    for (n, event) in trail.events.iter().enumerate() {
        let key = if event.id.is_empty() {
            (n + 1).to_string()
        } else {
            event.id.clone()
        };
        let id = anchor(&["prov", &key]);
        let actor =
            (!event.actor.is_empty()).then(|| person_link(&view.profile.person(&event.actor).name));
        let mut html = line(
            &escape(&capitalised(&kind_words(&event.kind))),
            actor.as_deref(),
            Some(&event.occurred_at),
            "provenance",
            &id,
        );
        let reference = provenance::reference(event)
            .map(|f| fact(&f))
            .unwrap_or_default();
        let payload = event.payload_line();
        if !reference.is_empty() || !payload.is_empty() {
            write!(html, "<p>{reference} {}</p>", escape(&payload)).unwrap();
        }
        out.push(Entry {
            at: instant(&event.occurred_at),
            origin: Origin::Provenance,
            anchor: id,
            tone: provenance_tone(&event.kind),
            html,
        });
    }
}

fn steps(run: &runs::Run, out: &mut Vec<Entry>) {
    for attempt in &run.attempts {
        for step in &attempt.steps {
            let at = step.started_at.as_deref().or(step.ended_at.as_deref());
            let id = anchor(&["step", &run.run, &step.id]);
            let name = step
                .label
                .as_deref()
                .or(step.name.as_deref())
                .unwrap_or(&step.kind);
            let result = step.result.as_deref().unwrap_or(&step.state);
            let mut html = line(
                &format!("{} step", escape(&words(name))),
                None,
                at,
                &format!(
                    "<a href=\"/run/{}#attempt-{}\">run {} · attempt {}</a>",
                    url(&run.run),
                    attempt.number,
                    escape(&run.run),
                    attempt.number
                ),
                &id,
            );
            write!(
                html,
                " {}",
                tone_badge(&words(result), runs::step_tone(result))
            )
            .unwrap();
            if let Some(text) = step.failure.as_ref().and_then(|f| f.text.as_deref()) {
                write!(html, "<p>{}</p>", escape(text)).unwrap();
            }
            out.push(Entry {
                at: at.and_then(instant),
                origin: Origin::Step,
                anchor: id,
                tone: runs::step_tone(result),
                html,
            });
        }
    }
}

fn list(entries: &[Entry]) -> String {
    let mut out = "<ol class=\"timeline\">".to_string();
    for e in entries {
        write!(
            out,
            "<li id=\"{}\" class=\"{}\">{}</li>",
            e.anchor,
            tone_class(e.tone),
            e.html
        )
        .unwrap();
    }
    out.push_str("</ol>");
    out
}

/// The timeline section of a bead's page: newest first, the newest 100
/// shown and the rest folded away.
pub(crate) fn timeline(view: &View, bead: &Bead) -> String {
    if !super::bead::enriched(view) {
        return String::new();
    }
    let links = Links::new(view);
    let mut entries = Vec::new();
    let mut notes = Vec::new();
    dates(bead, &mut entries);
    // A thread read in the background, else the comments the bead's own
    // read carried.
    let parsed;
    let thread = match view.thread(&bead.id) {
        Some(thread) => Some(thread),
        None if !bead.comments.is_empty() => {
            parsed = Thread::from_comments(&bead.comments, view.profile.events.as_ref());
            Some(&parsed)
        }
        None => None,
    };
    match thread {
        Some(thread) => history(view, thread, &links, &mut entries),
        None if bead.comment_count == Some(0) => {}
        None => notes.push("History not read yet.".to_string()),
    }
    match view.trail(&bead.id) {
        Some(trail) => provenance(view, trail, &mut entries),
        None if view.provenance.is_some() => notes.push("Provenance not read yet.".to_string()),
        None => {}
    }
    for run in joined(view, bead) {
        match &run.doc {
            Fetch::Ready(doc) => steps(doc, &mut entries),
            Fetch::Pending => notes.push(format!(
                "The steps of run {} have not been read yet.",
                run.summary.run
            )),
            Fetch::Failed(error) => notes.push(format!(
                "The steps of run {} could not be read. {error}",
                run.summary.run
            )),
        }
    }
    let entries = merge(entries);
    let mut out = format!(
        "<section id=\"timeline\"><h2>Timeline · {}</h2><p class=\"muted\">Newest first: dates, history, comments, provenance and run steps.</p>",
        entries.len()
    );
    for note in notes {
        write!(
            out,
            "<p class=\"muted timeline-note\">{}</p>",
            escape(&note)
        )
        .unwrap();
    }
    let (shown, older) = entries.split_at(entries.len().min(SHOWN));
    out.push_str(&list(shown));
    if !older.is_empty() {
        write!(
            out,
            "<details id=\"timeline-older\"><summary>Older entries · {}</summary>{}</details>",
            older.len(),
            list(older)
        )
        .unwrap();
    }
    out.push_str("</section>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::work_view;

    fn entry(at: Option<&str>, origin: Origin, name: &str) -> Entry {
        Entry {
            at: at.and_then(instant),
            origin,
            anchor: name.into(),
            tone: Tone::Neutral,
            html: String::new(),
        }
    }

    #[test]
    fn merge_orders_newest_first_with_ties_by_origin_and_undated_last() {
        let merged = merge(vec![
            entry(Some("2026-11-14T15:35:00Z"), Origin::Closed, "closed"),
            entry(None, Origin::Comment, "undated"),
            entry(Some("2026-11-01T09:00:00Z"), Origin::Created, "created"),
            entry(Some("2026-11-14T15:35:00Z"), Origin::History, "completion"),
            entry(Some("2026-11-01T09:00:00Z"), Origin::History, "authored"),
            entry(Some("2026-11-14T11:00:00+02:00"), Origin::Step, "step"),
            entry(Some("2026-11-14"), Origin::Provenance, "midnight"),
            entry(Some("2026-11-14T09:00:00Z"), Origin::Provenance, "claim"),
        ]);
        let names: Vec<&str> = merged.iter().map(|e| e.anchor.as_str()).collect();
        assert_eq!(
            names,
            [
                "closed",
                "completion",
                "step",
                "claim",
                "midnight",
                "authored",
                "created",
                "undated"
            ]
        );
    }

    #[test]
    fn the_timeline_merges_dates_history_comments_provenance_and_steps() {
        let view = work_view();
        let page = crate::render::bead(&view, "wk-1").unwrap();
        let at = |needle: &str| {
            page.find(needle)
                .unwrap_or_else(|| panic!("{needle} missing: {page}"))
        };
        // Newest first: the gate step (11:53), bob's comment (10:30), the
        // started date, provenance claim and history claim (all 10:00),
        // then authored and created (11-10, 09:00).
        let gate = at("id=\"t-step-run-0001-s-0001-gate-1\"");
        let comment = at("id=\"t-comment-c-1c\"");
        let claim_event = at("id=\"t-event-2-claim\"");
        let claim_prov = at("id=\"t-prov-p1-1\"");
        let authored = at("id=\"t-event-1-authored\"");
        let created = at("id=\"t-created\"");
        let started = at("id=\"t-started\"");
        assert!(gate < comment && comment < started);
        assert!(started < claim_prov && claim_prov < claim_event);
        assert!(claim_event < authored && authored < created);
        assert!(page.contains("status: open → working"));
        assert!(
            page.contains("<strong>header row</strong>"),
            "comments are Markdown"
        );
        assert!(page.contains("{mostly}"));
        // Steps of run-0001 join through the run feed.
        let step = page.find("id=\"t-step-run-0001-").expect("a run-0001 step");
        assert!(step < claim_event, "{page}");
        assert!(page.contains("href=\"/run/run%2D0001#attempt-1\""));
        assert!(!page.contains("History not read yet"));
    }

    #[test]
    fn an_unread_thread_says_so() {
        let mut view = work_view();
        view.history = Some(crate::history::Store::default());
        // wk-q1's export carries no comments in this copy.
        let mut snapshot = (*view.beads.as_ref().unwrap().data).clone();
        let i = snapshot.position("wk-q1").unwrap();
        snapshot.beads[i].comments.clear();
        snapshot.beads[i].comment_count = None;
        view.beads.as_mut().unwrap().data = std::sync::Arc::new(snapshot);
        let page = crate::render::bead(&view, "wk-q1").unwrap();
        assert!(page.contains("History not read yet."), "{page}");
    }
}
