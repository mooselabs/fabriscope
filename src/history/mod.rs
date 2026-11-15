//! A bead's history: the events its comments record, read with the
//! profile's `[events]` rule, and the comments that are plain discussion.
//!
//! An event comment is a readable summary whose LAST line starting with the
//! marker carries the event as compact JSON. A comment without such a line,
//! or whose JSON does not parse, is a plain Markdown comment. Threads are
//! read in the background (`history::reader`, owned by WP3) and kept in a
//! `Store`, like provenance trails: pages show the last read and never wait.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "contract the packages call; WP2 and WP3 read and fill it (BKLG-20261115-4e2r)"
    )
)]

pub mod reader;

use crate::beads::Comment;
use crate::profile::{EventsRule, Tone};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// One history event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    /// The event kind as recorded (`claim`, `suspended`, …).
    pub kind: String,
    /// The `[events.kinds]` label, else the kind in words.
    pub label: String,
    pub tone: Tone,
    /// When it happened: the JSON `at`, else the comment's time.
    pub at: Option<String>,
    /// Who did it: the JSON `actor`, else the comment author.
    pub actor: String,
    /// `(field, old, new)` from the `changes` key, in key order.
    pub changes: Vec<(String, Value, Value)>,
    /// Text from the `notes` keys.
    pub notes: Vec<String>,
    /// The whole event JSON.
    pub raw: Value,
    pub flags: Flags,
}

/// What an event means for flow, from `[events.kinds]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub claim: bool,
    pub stop: bool,
    pub created: bool,
}

/// A comment read as an event, or not.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Event(Box<Event>),
    Plain,
}

/// Reads one comment with `rule`.
pub fn parse(comment: &Comment, rule: &EventsRule) -> Parsed {
    let marker = rule.marker.trim();
    if marker.is_empty() {
        return Parsed::Plain;
    }
    let Some(json) = comment
        .text
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix(marker))
        .map(str::trim)
    else {
        return Parsed::Plain;
    };
    let Ok(raw @ Value::Object(_)) = serde_json::from_str::<Value>(json) else {
        return Parsed::Plain;
    };
    if let Some(check) = &rule.schema {
        if raw.get(&check.key).and_then(Value::as_str) != Some(check.equals.as_str()) {
            return Parsed::Plain;
        }
    }
    let text = |key: &str| {
        raw.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let kind = text(&rule.kind).unwrap_or_default();
    let kind_rule = rule.kinds.get(&kind);
    let changes = rule
        .changes
        .as_deref()
        .and_then(|key| raw.get(key))
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| match v.as_array().map(Vec::as_slice) {
                    Some([old, new]) => (k.clone(), old.clone(), new.clone()),
                    _ => (k.clone(), Value::Null, v.clone()),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut notes = Vec::new();
    for key in &rule.notes {
        match raw.get(key) {
            Some(Value::String(s)) if !s.is_empty() => notes.push(s.clone()),
            Some(Value::Array(items)) => {
                notes.extend(items.iter().filter_map(Value::as_str).map(String::from))
            }
            _ => {}
        }
    }
    Parsed::Event(Box::new(Event {
        label: kind_rule
            .map(|k| k.label.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| kind.replace('_', " ")),
        tone: kind_rule.and_then(|k| k.tone).unwrap_or_default(),
        at: text(&rule.at).or_else(|| comment.created_at.clone()),
        actor: text(&rule.actor)
            .or_else(|| comment.author.clone())
            .unwrap_or_default(),
        changes,
        notes,
        flags: kind_rule
            .map(|k| Flags {
                claim: k.claim,
                stop: k.stop,
                created: k.created,
            })
            .unwrap_or_default(),
        kind,
        raw,
    }))
}

/// A bead's events (oldest first) and its plain comments (oldest first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Thread {
    pub events: Vec<Event>,
    pub comments: Vec<Comment>,
}

impl Thread {
    /// Splits comments into events and plain comments. Without a rule every
    /// comment is plain. Events sort by time, then by the rule's `order` keys.
    pub fn from_comments(comments: &[Comment], rule: Option<&EventsRule>) -> Self {
        let mut thread = Thread::default();
        for comment in comments {
            match rule.map(|r| parse(comment, r)) {
                Some(Parsed::Event(event)) => thread.events.push(*event),
                _ => thread.comments.push(comment.clone()),
            }
        }
        let order: Vec<String> = rule.map(|r| r.order.clone()).unwrap_or_default();
        let key = |e: &Event| -> (Option<String>, Vec<i64>) {
            (
                e.at.clone(),
                order
                    .iter()
                    .map(|k| e.raw.get(k).and_then(Value::as_i64).unwrap_or(0))
                    .collect(),
            )
        };
        thread.events.sort_by_key(key);
        thread
            .comments
            .sort_by(|a, b| a.created_at.cmp(&b.created_at));
        thread
    }

    /// The first event of this kind.
    pub fn first(&self, kind: &str) -> Option<&Event> {
        self.events.iter().find(|e| e.kind == kind)
    }

    /// The actor of the first event of this kind (an `event:<kind>` source).
    pub fn actor(&self, kind: &str) -> Option<&str> {
        self.first(kind)
            .map(|e| e.actor.as_str())
            .filter(|a| !a.is_empty())
    }
}

/// The last read of one bead's thread.
#[derive(Clone, Debug)]
pub struct Entry {
    pub thread: Thread,
    pub error: Option<String>,
    pub read_at: DateTime<Utc>,
    /// The bead's `updated_at` when read; a change asks for a re-read.
    pub updated_at: Option<String>,
}

/// Every thread read so far, and the reads under way.
#[derive(Clone, Debug, Default)]
pub struct Store {
    pub threads: HashMap<String, Entry>,
    pub reading: HashSet<String>,
}

impl Store {
    /// The thread last read for a bead.
    pub fn get(&self, id: &str) -> Option<&Thread> {
        self.threads.get(id).map(|e| &e.thread)
    }

    /// Whether a bead needs a (re-)read: never read, changed since, failed
    /// more than a minute ago, or older than ten minutes.
    pub fn wants_read(&self, id: &str, updated_at: Option<&str>, now: DateTime<Utc>) -> bool {
        if self.reading.contains(id) {
            return false;
        }
        let Some(entry) = self.threads.get(id) else {
            return true;
        };
        let window = if entry.error.is_some() {
            Duration::minutes(1)
        } else {
            Duration::minutes(10)
        };
        entry.updated_at.as_deref() != updated_at || now - entry.read_at >= window
    }

    /// Keeps a finished read (a failure keeps the last good thread) and says
    /// whether what a page shows changed.
    pub fn record(
        &mut self,
        id: &str,
        result: Result<Thread, String>,
        updated_at: Option<String>,
        now: DateTime<Utc>,
    ) -> bool {
        self.reading.remove(id);
        let previous = self.threads.get(id);
        let entry = match result {
            Ok(thread) => Entry {
                thread,
                error: None,
                read_at: now,
                updated_at,
            },
            Err(error) => Entry {
                thread: previous.map(|p| p.thread.clone()).unwrap_or_default(),
                error: Some(error),
                read_at: now,
                updated_at,
            },
        };
        let changed = previous.is_none_or(|p| p.thread != entry.thread || p.error != entry.error);
        self.threads.insert(id.to_string(), entry);
        changed
    }

    /// For /healthz.
    pub fn status(&self) -> Value {
        serde_json::json!({
            "read": self.threads.len(),
            "reading": self.reading.len(),
            "failed": self.threads.values().filter(|e| e.error.is_some()).count(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;

    fn rule() -> EventsRule {
        Profile::parse(include_str!("../../tests/fixtures/work/profile.toml"))
            .unwrap()
            .events
            .unwrap()
    }

    fn comment(text: &str) -> Comment {
        Comment {
            id: Some("c".into()),
            author: Some("alice".into()),
            text: text.into(),
            created_at: Some("2026-11-15T10:00:00Z".into()),
        }
    }

    #[test]
    fn marker_comments_become_events_and_the_rest_stay_plain() {
        let rule = rule();
        let event = comment(
            "suspended · rigB · ledger r9\nstatus: working → open\nnote with {brace}\nwork.event.v1 {\"schema\":\"work.event.v1\",\"kind\":\"suspended\",\"actor\":\"factory/hostA/rigB\",\"at\":\"2026-11-15T09:15:00Z\",\"changes\":{\"status\":[\"working\",\"open\"],\"claimed_by\":[\"factory/hostA/rigB\",null]},\"reason\":\"scope exceeded\",\"ledger_revision\":9}",
        );
        let Parsed::Event(e) = parse(&event, &rule) else {
            panic!("not an event")
        };
        assert_eq!(e.kind, "suspended");
        assert_eq!(e.label, "Held");
        assert_eq!(e.tone, Tone::Bad);
        assert!(e.flags.stop && !e.flags.claim);
        assert_eq!(e.at.as_deref(), Some("2026-11-15T09:15:00Z"));
        assert_eq!(e.actor, "factory/hostA/rigB");
        assert_eq!(
            e.changes[1],
            ("status".into(), Value::from("working"), Value::from("open"))
        );
        assert_eq!(e.changes[0].2, Value::Null);
        assert_eq!(e.notes, ["scope exceeded"]);
        // Plain: no marker, a brace in prose, wrong schema, bad JSON.
        for text in [
            "Looks good to me {mostly}.",
            "work.event.v1 {\"schema\":\"other\",\"kind\":\"x\"}",
            "work.event.v1 {not json",
        ] {
            assert_eq!(parse(&comment(text), &rule), Parsed::Plain, "{text}");
        }
        // The last marker line wins; time and actor fall back to the comment.
        let Parsed::Event(e) = parse(
            &comment(
                "work.event.v1 {\"schema\":\"work.event.v1\",\"kind\":\"old\"}\nwork.event.v1 {\"schema\":\"work.event.v1\",\"kind\":\"claim\"}",
            ),
            &rule,
        ) else {
            panic!()
        };
        assert_eq!((e.kind.as_str(), e.actor.as_str()), ("claim", "alice"));
        assert_eq!(e.at.as_deref(), Some("2026-11-15T10:00:00Z"));
        assert!(e.flags.claim);
        let thread = Thread::from_comments(&[event, comment("plain")], Some(&rule));
        assert_eq!((thread.events.len(), thread.comments.len()), (1, 1));
        assert_eq!(thread.actor("suspended"), Some("factory/hostA/rigB"));
        assert_eq!(
            Thread::from_comments(&[comment("x")], None).comments.len(),
            1
        );
    }

    #[test]
    fn the_store_keeps_the_last_good_thread() {
        let now = Utc::now();
        let mut store = Store::default();
        assert!(store.wants_read("a", None, now));
        assert!(store.record("a", Ok(Thread::default()), None, now));
        assert!(!store.wants_read("a", None, now));
        assert!(store.wants_read("a", Some("2026-11-15T00:00:00Z"), now));
        assert!(store.record("a", Err("bd failed".into()), None, now));
        assert_eq!(store.status()["failed"], 1);
        assert!(store.get("a").unwrap().events.is_empty());
    }
}
