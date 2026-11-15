//! A bead's provenance log, read with `bd provenance log`, and the stages it
//! shows the bead has reached.
//!
//! Beads records provenance as typed, append-only events that bind an issue
//! to an outside artifact: `claim`, `cut`, `commit`, `land`, `used`,
//! `suspend`, `resume` and `handoff`. `bd` reads them one issue at a time and
//! `bd serve` does not serve them, so Fabriscope reads each bead's log in the
//! background and keeps the last read. A page shows what was last read and
//! never waits for `bd`.

use crate::beads::Bead;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// One provenance event, as `bd provenance log --json` prints it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Event {
    pub id: String,
    pub kind: String,
    pub actor: String,
    pub source: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub ref_kind: String,
    /// Opaque to Beads; usually a JSON object encoded as a string.
    pub payload: Value,
    pub occurred_at: String,
    pub created_at: String,
}

impl Event {
    /// The payload as a JSON object, when it is one or a string holding one.
    fn object(&self) -> Option<serde_json::Map<String, Value>> {
        match &self.payload {
            Value::Object(map) => Some(map.clone()),
            Value::String(text) => serde_json::from_str(text).ok(),
            _ => None,
        }
    }

    /// The payload's fields, in key order; a payload that is not an object
    /// has none.
    pub fn fields(&self) -> Vec<(String, Value)> {
        self.object()
            .map(|m| m.into_iter().collect())
            .unwrap_or_default()
    }

    /// The payload's fields as display text, without the keys in `skip`
    /// and without empty values.
    pub fn field_text(&self, skip: &[&str]) -> Vec<(String, String)> {
        self.fields()
            .into_iter()
            .filter(|(key, _)| !skip.contains(&key.as_str()))
            .filter_map(|(key, value)| value_text(&key, &value).map(|text| (key, text)))
            .collect()
    }

    /// The payload as one line, for the log table: its fields when it is an
    /// object (an empty object says nothing), otherwise the payload itself.
    pub fn payload_line(&self) -> String {
        if self.object().is_some() {
            return self
                .field_text(&[])
                .iter()
                .map(|(k, v)| format!("{} {v}", k.replace('_', " ")))
                .collect::<Vec<_>>()
                .join(" · ");
        }
        match &self.payload {
            Value::Null => String::new(),
            Value::String(text) => text.clone(),
            other => other.to_string(),
        }
    }
}

/// Parses `bd provenance log --json`, oldest event first.
pub fn parse(text: &str) -> Result<Vec<Event>, String> {
    let text = text.trim();
    if text.is_empty() || text == "null" {
        return Ok(Vec::new());
    }
    let mut events: Vec<Event> =
        serde_json::from_str(text).map_err(|e| format!("bd provenance log: {e}"))?;
    // Stable, so events at the same time keep the order bd gave them.
    events.sort_by(|a, b| (&a.occurred_at, &a.created_at).cmp(&(&b.occurred_at, &b.created_at)));
    Ok(events)
}

/// Something shown about a stage, kept apart from its markup so every value
/// is escaped in one place.
#[derive(Clone, Debug, PartialEq)]
pub enum Fact {
    Text(String),
    /// An identifier, shown shortened with the full value on hover.
    Code {
        full: String,
        short: String,
    },
    /// An external web page; only `https://` references become links.
    Link {
        href: String,
        text: String,
    },
}

fn truncate(value: &str, chars: usize) -> String {
    if value.chars().count() > chars {
        format!("{}…", value.chars().take(chars).collect::<String>())
    } else {
        value.to_string()
    }
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The event's reference as a fact: a pull request or other web page as a
/// link, a commit SHA shortened, anything else as an identifier.
pub fn reference(event: &Event) -> Option<Fact> {
    let value = event.reference.trim();
    if value.is_empty() {
        return None;
    }
    if value.to_ascii_lowercase().starts_with("https://") {
        let parts: Vec<&str> = value.trim_end_matches('/').split('/').collect();
        let text = match parts.as_slice() {
            [.., "pull", number] if number.bytes().all(|b| b.is_ascii_digit()) => {
                format!("PR #{number}")
            }
            _ => truncate(value.trim_start_matches("https://"), 40),
        };
        return Some(Fact::Link {
            href: value.to_string(),
            text,
        });
    }
    let short = if event.ref_kind == "git-sha" || is_hex(value, 40) {
        value.chars().take(7).collect()
    } else {
        truncate(value, 32)
    };
    Some(Fact::Code {
        full: value.to_string(),
        short,
    })
}

fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// A payload value as short display text, or `None` when it is empty.
fn value_text(key: &str, value: &Value) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::Bool(b) => if *b { "yes" } else { "no" }.to_string(),
        Value::Number(n) if key == "dollars" => format!("${:.2}", n.as_f64()?),
        Value::Number(n) => match n.as_i64() {
            Some(i) => grouped(i),
            None => format!("{:.2}", n.as_f64()?),
        },
        Value::String(s) if s.is_empty() => return None,
        Value::String(s) if is_hex(s, 64) || is_hex(s, 40) => s.chars().take(9).collect(),
        Value::String(s) => truncate(s, 160),
        other => truncate(&other.to_string(), 160),
    };
    Some(text)
}

/// How far a stage got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Done,
    /// The next step, under way or awaited.
    Current,
    Pending,
    /// Not recorded, though a later step was.
    Skipped,
    /// Where the attempt stopped.
    Stopped,
}

impl Mark {
    pub fn class(self) -> &'static str {
        match self {
            Mark::Done => "done",
            Mark::Current => "current",
            Mark::Pending => "pending",
            Mark::Skipped => "skipped",
            Mark::Stopped => "stopped",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Mark::Done => "done",
            Mark::Current => "next",
            Mark::Pending => "not yet",
            Mark::Skipped => "not recorded",
            Mark::Stopped => "stopped here",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub label: &'static str,
    pub mark: Mark,
    pub at: Option<String>,
    pub facts: Vec<Fact>,
}

/// The steps toward closed, each with the event kind that shows it reached.
/// Created and Closed come from the bead itself.
const STEPS: [(&str, &str); 6] = [
    ("Created", ""),
    ("Claimed", "claim"),
    ("Started", "cut"),
    ("Committed", "commit"),
    ("Landed", "land"),
    ("Closed", ""),
];

/// Payload keys a `used` stage shows, in order; the log shows the rest.
const USAGE_KEYS: [&str; 3] = ["dollars", "model", "tokens"];

/// The stages one attempt reached. `closed` is whether the bead is closed
/// (only true for its latest attempt); `stopped` puts the stop marker on the
/// step after the furthest one reached; `active` marks that step as next.
fn stages(bead: &Bead, events: &[&Event], closed: bool, stopped: bool, active: bool) -> Vec<Stage> {
    let mut out: Vec<Stage> = STEPS
        .iter()
        .map(|(label, kind)| {
            let (done, at, facts) = match *label {
                "Created" => (true, bead.created_at.clone(), Vec::new()),
                "Closed" => (
                    closed,
                    if closed { bead.closed_at.clone() } else { None },
                    if closed && !bead.close_reason.is_empty() {
                        vec![Fact::Text(bead.close_reason.clone())]
                    } else {
                        Vec::new()
                    },
                ),
                _ => {
                    let matching: Vec<&&Event> =
                        events.iter().filter(|e| e.kind == *kind).collect();
                    let mut facts = Vec::new();
                    for event in &matching {
                        let fact = if *kind == "claim" && !event.actor.is_empty() {
                            Some(Fact::Text(format!("by {}", event.actor)))
                        } else {
                            reference(event)
                        };
                        if let Some(fact) = fact.filter(|f| !facts.contains(f)) {
                            facts.push(fact);
                        }
                    }
                    (
                        !matching.is_empty(),
                        matching.first().map(|e| e.occurred_at.clone()),
                        facts,
                    )
                }
            };
            Stage {
                label,
                mark: if done { Mark::Done } else { Mark::Pending },
                at,
                facts,
            }
        })
        .collect();
    let furthest = out.iter().rposition(|s| s.mark == Mark::Done).unwrap_or(0);
    for (i, stage) in out.iter_mut().enumerate() {
        if stage.mark == Mark::Done {
            continue;
        }
        stage.mark = if i < furthest {
            Mark::Skipped
        } else if i == furthest + 1 && stopped {
            Mark::Stopped
        } else if i == furthest + 1 && active {
            Mark::Current
        } else {
            Mark::Pending
        };
    }
    // Usage is recorded beside the steps, often after the bead closes or
    // the attempt stops, so it never moves the progress line.
    let used: Vec<&&Event> = events.iter().filter(|e| e.kind == "used").collect();
    let mut facts = Vec::new();
    for event in &used {
        let fields = event.field_text(&[]);
        let shown: Vec<Fact> = USAGE_KEYS
            .iter()
            .filter_map(|key| fields.iter().find(|(k, _)| k == key))
            .map(|(_, v)| Fact::Text(v.clone()))
            .collect();
        if shown.is_empty() {
            facts.extend(reference(event));
        } else {
            facts.extend(shown);
        }
    }
    out.push(Stage {
        label: "Used",
        mark: if used.is_empty() {
            Mark::Pending
        } else {
            Mark::Done
        },
        at: used.first().map(|e| e.occurred_at.clone()),
        facts,
    });
    out
}

/// A recorded event with its payload as display text.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub kind: String,
    pub at: String,
    pub actor: String,
    pub fields: Vec<(String, String)>,
}

impl Note {
    fn of(event: &Event) -> Self {
        Self {
            kind: event.kind.clone(),
            at: event.occurred_at.clone(),
            actor: event.actor.clone(),
            fields: event
                .field_text(&["attempt", "digest", "incident"])
                .into_iter()
                // A `by` that repeats the actor says nothing new.
                .filter(|(key, value)| !(key == "by" && *value == event.actor))
                .collect(),
        }
    }
}

/// How an attempt ended: a `suspend` (held until a `resume`) or a `handoff`
/// (the claim was given up and the bead is free again).
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub stop: Note,
    pub resumed: Option<Note>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attempt {
    pub number: usize,
    pub stages: Vec<Stage>,
    pub stop: Option<Stop>,
}

/// Where a bead is up to, from its provenance.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// The latest attempt, or a fresh one when the last attempt ended and
    /// the bead is free to be claimed again.
    pub stages: Vec<Stage>,
    /// When the next step became due: the last step's time, or when the
    /// bead was freed for a new claim.
    pub since: Option<String>,
    /// Why the latest attempt is stopped, while it is held there.
    pub held: Option<Stop>,
    /// Attempts before the one shown, oldest first.
    pub earlier: Vec<Attempt>,
    /// How many times the bead was claimed.
    pub claims: usize,
}

fn ending(events: &[&Event]) -> Option<Stop> {
    let at = events
        .iter()
        .position(|e| e.kind == "suspend" || e.kind == "handoff")?;
    Some(Stop {
        stop: Note::of(events[at]),
        resumed: events[at + 1..]
            .iter()
            .find(|e| e.kind == "resume")
            .map(|e| Note::of(e)),
    })
}

fn latest_done(stages: &[Stage]) -> Option<String> {
    stages[..STEPS.len()]
        .iter()
        .rev()
        .find(|s| s.mark == Mark::Done)
        .and_then(|s| s.at.clone())
}

impl Progress {
    /// `events` oldest first, as `parse` returns them. Each `claim` starts
    /// an attempt; events before the first claim belong to the first.
    pub fn new(bead: &Bead, events: &[Event]) -> Self {
        let mut attempts: Vec<Vec<&Event>> = Vec::new();
        for event in events {
            let starts_new = event.kind == "claim"
                && attempts
                    .last()
                    .is_some_and(|a| a.iter().any(|e| e.kind == "claim"));
            if attempts.is_empty() || starts_new {
                attempts.push(Vec::new());
            }
            attempts.last_mut().unwrap().push(event);
        }
        let claims = events.iter().filter(|e| e.kind == "claim").count();
        let latest = attempts.pop().unwrap_or_default();
        let mut earlier: Vec<Attempt> = attempts
            .iter()
            .enumerate()
            .map(|(i, events)| {
                let stop = ending(events);
                Attempt {
                    number: i + 1,
                    stages: stages(bead, events, false, stop.is_some(), false),
                    stop,
                }
            })
            .collect();
        let closed = bead.is_closed();
        let stop = if closed { None } else { ending(&latest) };
        let (shown, since, held) = match stop {
            Some(stop) if stop.resumed.is_some() || stop.stop.kind == "handoff" => {
                let freed = stop
                    .resumed
                    .as_ref()
                    .map(|r| r.at.clone())
                    .unwrap_or_else(|| stop.stop.at.clone());
                earlier.push(Attempt {
                    number: earlier.len() + 1,
                    stages: stages(bead, &latest, false, true, false),
                    stop: Some(stop),
                });
                (stages(bead, &[], false, false, true), Some(freed), None)
            }
            Some(stop) => {
                let shown = stages(bead, &latest, false, true, false);
                let since = Some(stop.stop.at.clone());
                (shown, since, Some(stop))
            }
            None => {
                let shown = stages(bead, &latest, closed, false, !closed);
                let since = if closed { None } else { latest_done(&shown) };
                (shown, since, None)
            }
        };
        Self {
            stages: shown,
            since,
            held,
            earlier,
            claims,
        }
    }

    /// The step now due or stopped at, if any.
    pub fn now(&self) -> Option<&Stage> {
        self.stages
            .iter()
            .find(|s| matches!(s.mark, Mark::Current | Mark::Stopped))
    }
}

/// Elapsed time since an RFC 3339 instant, in the two largest units.
pub fn elapsed(since: &str, now: DateTime<Utc>) -> Option<String> {
    let then = DateTime::parse_from_rfc3339(since)
        .ok()?
        .with_timezone(&Utc);
    let minutes = (now - then).num_minutes().max(0);
    let (days, hours, mins) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    Some(if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    })
}

/// The last read of one bead's log.
#[derive(Clone, Debug)]
pub struct Trail {
    /// Oldest first. A failed read keeps the events of the last good one.
    pub events: Vec<Event>,
    pub error: Option<String>,
    pub read_at: DateTime<Utc>,
    /// The bead's `updated_at` when it was read; a change asks for a re-read.
    pub updated_at: Option<String>,
}

/// Every log read so far, and the reads under way.
#[derive(Clone, Debug, Default)]
pub struct Store {
    pub trails: HashMap<String, Trail>,
    pub reading: HashSet<String>,
}

impl Store {
    pub fn get(&self, id: &str) -> Option<&Trail> {
        self.trails.get(id)
    }

    /// Whether this bead needs a read: never read, changed since, or older
    /// than its freshness window (closed beads change rarely, failed reads
    /// are retried sooner).
    pub fn wants_read(&self, bead: &Bead, now: DateTime<Utc>) -> bool {
        if self.reading.contains(&bead.id) {
            return false;
        }
        let Some(trail) = self.trails.get(&bead.id) else {
            return true;
        };
        let window = if trail.error.is_some() {
            Duration::minutes(1)
        } else if bead.is_closed() {
            Duration::minutes(30)
        } else {
            Duration::minutes(2)
        };
        trail.updated_at != bead.updated_at || now - trail.read_at >= window
    }

    /// Keeps a finished read and says whether what a page shows changed.
    pub fn record(
        &mut self,
        id: &str,
        result: Result<Vec<Event>, String>,
        updated_at: Option<String>,
        now: DateTime<Utc>,
    ) -> bool {
        self.reading.remove(id);
        let previous = self.trails.get(id);
        let trail = match result {
            Ok(events) => Trail {
                events,
                error: None,
                read_at: now,
                updated_at,
            },
            Err(error) => Trail {
                events: previous.map(|p| p.events.clone()).unwrap_or_default(),
                error: Some(error),
                read_at: now,
                updated_at,
            },
        };
        let changed = previous.is_none_or(|p| p.events != trail.events || p.error != trail.error);
        self.trails.insert(id.to_string(), trail);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, at: &str, reference: &str, ref_kind: &str, payload: &str) -> Event {
        Event {
            id: format!("{kind}-{at}-{reference}"),
            kind: kind.into(),
            actor: "factory/rig".into(),
            source: "test".into(),
            reference: reference.into(),
            ref_kind: ref_kind.into(),
            payload: if payload.is_empty() {
                Value::Null
            } else {
                Value::String(payload.into())
            },
            occurred_at: at.into(),
            created_at: at.into(),
        }
    }

    fn bead(status: &str) -> Bead {
        Bead {
            id: "fx-1".into(),
            status: status.into(),
            created_at: Some("2026-11-15T03:00:00Z".into()),
            closed_at: (status == "closed").then(|| "2026-11-15T05:30:00Z".into()),
            close_reason: if status == "closed" {
                "done".into()
            } else {
                String::new()
            },
            ..Default::default()
        }
    }

    fn marks(stages: &[Stage]) -> Vec<(&str, Mark)> {
        stages.iter().map(|s| (s.label, s.mark)).collect()
    }

    const SHA: &str = "7e40693d44991d5a0fff54b74961976bab3bd672";

    #[test]
    fn parses_bd_output_oldest_first_and_reads_string_payloads() {
        let text = r#"[
          {"id":"b","issue_id":"fx-1","kind":"cut","actor":"f","payload":"{\"formula\":\"mol\"}","source":"s","occurred_at":"2026-11-15T03:55:36Z","created_at":"2026-11-15T03:55:36Z","ref":"w:1@x","ref_kind":"work-id"},
          {"id":"a","issue_id":"fx-1","kind":"claim","actor":"f","payload":"{\"generation\":1,\"rig\":\"r\"}","source":"s","occurred_at":"2026-11-15T03:53:05Z","created_at":"2026-11-15T03:53:07Z"}
        ]"#;
        let events = parse(text).unwrap();
        assert_eq!(events[0].kind, "claim");
        assert_eq!(events[1].reference, "w:1@x");
        assert_eq!(
            events[0].field_text(&[]),
            [
                ("generation".into(), "1".into()),
                ("rig".into(), "r".into())
            ]
        );
        assert_eq!(events[0].payload_line(), "generation 1 · rig r");
        let mut empty = events[0].clone();
        empty.payload = Value::String("{}".into());
        assert_eq!(empty.payload_line(), "");
        empty.payload = Value::String("plain words".into());
        assert_eq!(empty.payload_line(), "plain words");
        assert!(parse("[]").unwrap().is_empty());
        assert!(parse("null").unwrap().is_empty());
        assert!(parse("{\"error\":\"no issue\"}").is_err());
    }

    #[test]
    fn references_become_links_shortened_shas_or_identifiers() {
        let pr = event("land", "t", "https://github.com/o/r/pull/2079", "pr", "");
        assert_eq!(
            reference(&pr),
            Some(Fact::Link {
                href: "https://github.com/o/r/pull/2079".into(),
                text: "PR #2079".into()
            })
        );
        let sha = event("commit", "t", SHA, "git-sha", "");
        assert!(matches!(reference(&sha), Some(Fact::Code { short, .. }) if short == "7e40693"));
        // Only https:// becomes a link; anything else stays inert text.
        let js = event("commit", "t", "javascript:alert(1)", "branch", "");
        assert!(matches!(reference(&js), Some(Fact::Code { .. })));
        assert_eq!(reference(&event("claim", "t", "", "", "")), None);
        let used = event(
            "used",
            "t",
            "w",
            "work-id",
            r#"{"dollars":3.5068,"tokens":25720083}"#,
        );
        assert_eq!(
            used.field_text(&[]),
            [
                ("dollars".into(), "$3.51".into()),
                ("tokens".into(), "25,720,083".into())
            ]
        );
    }

    #[test]
    fn a_landed_and_closed_bead_has_every_step_done() {
        let events = vec![
            event("claim", "2026-11-15T03:53:05Z", "", "", r#"{"rig":"r"}"#),
            event("cut", "2026-11-15T03:55:36Z", "w:1", "work-id", ""),
            event("commit", "2026-11-15T05:15:34Z", "agent/fx-1", "branch", ""),
            event("commit", "2026-11-15T05:15:34Z", SHA, "git-sha", ""),
            event(
                "land",
                "2026-11-15T05:15:48Z",
                "https://github.com/o/r/pull/9",
                "pr",
                "",
            ),
            event(
                "used",
                "2026-11-15T07:00:01Z",
                "w:1",
                "work-id",
                r#"{"dollars":3.5,"model":"m"}"#,
            ),
        ];
        let p = Progress::new(&bead("closed"), &events);
        assert!(
            p.stages.iter().all(|s| s.mark == Mark::Done),
            "{:?}",
            marks(&p.stages)
        );
        assert_eq!(p.stages[1].facts, [Fact::Text("by factory/rig".into())]);
        assert_eq!(p.stages[3].facts.len(), 2, "branch and SHA");
        assert_eq!(p.stages[5].facts, [Fact::Text("done".into())]);
        assert_eq!(
            p.stages[6].facts,
            [Fact::Text("$3.50".into()), Fact::Text("m".into())]
        );
        assert_eq!((p.now(), p.since.as_deref(), p.claims), (None, None, 1));
    }

    #[test]
    fn a_running_bead_marks_the_next_step_and_since_when() {
        let events = vec![
            event("claim", "2026-11-15T07:49:48Z", "", "", ""),
            event("cut", "2026-11-15T07:52:52Z", "w:2", "work-id", ""),
        ];
        let p = Progress::new(&bead("in_progress"), &events);
        assert_eq!(
            marks(&p.stages),
            [
                ("Created", Mark::Done),
                ("Claimed", Mark::Done),
                ("Started", Mark::Done),
                ("Committed", Mark::Current),
                ("Landed", Mark::Pending),
                ("Closed", Mark::Pending),
                ("Used", Mark::Pending),
            ]
        );
        assert_eq!(p.since.as_deref(), Some("2026-11-15T07:52:52Z"));
        let later = DateTime::parse_from_rfc3339("2026-11-15T10:03:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            elapsed(p.since.as_deref().unwrap(), later).as_deref(),
            Some("2h 10m")
        );
    }

    #[test]
    fn a_suspended_attempt_is_held_where_it_stopped_and_usage_does_not_move_the_line() {
        let events = vec![
            event("claim", "2026-11-09T17:46:07Z", "", "", ""),
            event("cut", "2026-11-09T17:47:21Z", "w:1", "work-id", ""),
            event(
                "used",
                "2026-11-09T18:56:17Z",
                "w:1",
                "work-id",
                r#"{"dollars":5.01}"#,
            ),
            event(
                "suspend",
                "2026-11-09T18:56:31Z",
                "",
                "",
                r#"{"attempt":"w:1","cause":"unknown","stop_code":"drift","digest":"5bb1fd732efeb3016603a5c6671f2525fabfcfdd54c8cc2b4b0a2c754d037dcc"}"#,
            ),
        ];
        let p = Progress::new(&bead("open"), &events);
        assert_eq!(p.stages[3].mark, Mark::Stopped, "{:?}", marks(&p.stages));
        assert_eq!(p.stages[6].mark, Mark::Done);
        assert_eq!(p.now().unwrap().label, "Committed");
        let held = p.held.unwrap();
        assert_eq!(
            held.stop.fields,
            [
                ("cause".into(), "unknown".into()),
                ("stop_code".into(), "drift".into())
            ]
        );
        assert!(held.resumed.is_none());
        assert!(p.earlier.is_empty());
    }

    #[test]
    fn a_resumed_or_handed_off_attempt_moves_to_earlier_and_a_fresh_one_awaits_a_claim() {
        let mut events = vec![
            event("claim", "2026-11-09T17:46:07Z", "", "", ""),
            event("cut", "2026-11-09T17:47:21Z", "w:1", "work-id", ""),
            event(
                "suspend",
                "2026-11-09T18:56:31Z",
                "",
                "",
                r#"{"cause":"unknown"}"#,
            ),
            event(
                "resume",
                "2026-11-09T20:01:06Z",
                "",
                "",
                r#"{"by":"owner","why":"fixed"}"#,
            ),
        ];
        let p = Progress::new(&bead("open"), &events);
        assert_eq!(p.now().unwrap().label, "Claimed");
        assert_eq!(p.now().unwrap().mark, Mark::Current);
        assert_eq!(p.since.as_deref(), Some("2026-11-09T20:01:06Z"));
        assert_eq!(p.earlier.len(), 1);
        assert_eq!(p.earlier[0].stages[3].mark, Mark::Stopped);
        assert_eq!(
            p.earlier[0]
                .stop
                .as_ref()
                .unwrap()
                .resumed
                .as_ref()
                .unwrap()
                .fields[0]
                .1,
            "owner"
        );
        let own = event(
            "resume",
            "t",
            "",
            "",
            r#"{"by":"factory/rig","why":"retake"}"#,
        );
        assert_eq!(Note::of(&own).fields, [("why".into(), "retake".into())]);
        // A second claim makes a new attempt; the first stays earlier.
        events.push(event("claim", "2026-11-09T21:55:44Z", "", "", ""));
        let p = Progress::new(&bead("in_progress"), &events);
        assert_eq!((p.claims, p.earlier.len()), (2, 1));
        assert_eq!(p.now().unwrap().label, "Started");
        // A handoff frees the bead without a resume.
        events.push(event(
            "handoff",
            "2026-11-09T22:30:00Z",
            "",
            "",
            r#"{"reason":"lease lost"}"#,
        ));
        let p = Progress::new(&bead("open"), &events);
        assert_eq!((p.earlier.len(), p.now().unwrap().label), (2, "Claimed"));
        assert!(p.held.is_none());
    }

    #[test]
    fn a_closed_bead_without_landing_shows_the_gap_as_not_recorded() {
        let events = vec![event("claim", "2026-11-15T03:53:05Z", "", "", "")];
        let p = Progress::new(&bead("closed"), &events);
        assert_eq!(
            marks(&p.stages)[2..6],
            [
                ("Started", Mark::Skipped),
                ("Committed", Mark::Skipped),
                ("Landed", Mark::Skipped),
                ("Closed", Mark::Done)
            ]
        );
    }

    #[test]
    fn the_store_rereads_when_a_bead_changes_or_ages_and_keeps_good_events_on_failure() {
        let now = Utc::now();
        let mut store = Store::default();
        let mut b = bead("in_progress");
        b.updated_at = Some("u1".into());
        assert!(store.wants_read(&b, now));
        store.reading.insert(b.id.clone());
        assert!(!store.wants_read(&b, now), "one read at a time");
        let events = vec![event("claim", "2026-11-15T03:53:05Z", "", "", "")];
        assert!(store.record(&b.id, Ok(events.clone()), b.updated_at.clone(), now));
        assert!(!store.wants_read(&b, now));
        assert!(store.wants_read(&b, now + Duration::minutes(3)));
        b.updated_at = Some("u2".into());
        assert!(store.wants_read(&b, now));
        assert!(
            !store.record(&b.id, Ok(events.clone()), b.updated_at.clone(), now),
            "same events"
        );
        assert!(store.record(&b.id, Err("bd failed".into()), b.updated_at.clone(), now));
        let trail = store.get(&b.id).unwrap();
        assert_eq!(
            (trail.events.len(), trail.error.as_deref()),
            (1, Some("bd failed"))
        );
        assert!(
            store.wants_read(&b, now + Duration::minutes(1)),
            "failures retry sooner"
        );
    }
}
