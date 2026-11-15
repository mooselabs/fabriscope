// Owned by WP3 — History, activity, people. Frozen signature:
// `pub fn spawn(app: App, reader: &beads::Reader)` and
// `pub async fn want(app: &App, ids: Vec<String>)`.
//! Reads beads' history threads in the background into `View::history`.
//!
//! Three ways, by what the store offers:
//!
//! - **Snapshot** (`bd export` is the main read): every bead already carries
//!   its `comments`, so threads are built from each new snapshot. No second
//!   read.
//! - **Bulk** (`bd serve` is the main read and a local `bd` exists): the
//!   list carries no comments, so `bd export` runs in a blocking task when
//!   the store's commit moves, at most once every five minutes. Its output
//!   is streamed a line at a time and only each issue's id, `updated_at` and
//!   thread is kept.
//! - **Per issue** (only `bd serve`): `want` fetches
//!   `/v0/beads/issues/{id}?include_comments=true` for the beads a page is
//!   about to show, at most four at a time, honouring `Store::wants_read`.
//!   Beads in a `read_ahead` state, and those in progress, are asked for
//!   whenever the snapshot changes.
//!
//! Every finished read goes through `Store::record`; the view revision is
//! bumped only when a thread changed. A failed read keeps the last good
//! thread and shows as failed on /healthz.

use super::{Store, Thread};
use crate::app::App;
use crate::beads::{self, Comment, Snapshot};
use crate::model::View;
use crate::profile::EventsRule;
use crate::serve::Api;
use crate::tools::Tool;
use chrono::Utc;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use std::collections::HashSet;
use std::io::BufRead;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore};

/// The least time between two bulk `bd export` reads.
const BULK_EVERY: Duration = Duration::from_secs(300);
/// How long one bulk `bd export` may run.
const BULK_TIMEOUT: Duration = Duration::from_secs(300);
/// Per-issue reads in flight at once.
const PER_ISSUE_PERMITS: usize = 4;
/// Beads read ahead of a visit, per snapshot, in per-issue mode.
const AHEAD: usize = 100;

/// Starts the background history reads for `reader`'s store. Called once
/// at startup.
pub fn spawn(app: App, reader: &beads::Reader) {
    match reader {
        beads::Reader::Cli(_) => {
            tokio::spawn(from_snapshots(app));
        }
        beads::Reader::Api {
            local: Some(tool), ..
        } => {
            tokio::spawn(bulk(app, tool.clone()));
        }
        beads::Reader::Api { api, local: None } => {
            register(&app, api.clone());
            tokio::spawn(read_ahead(app));
        }
    }
}

/// Asks for these beads' threads to be read soon (a page about to show
/// them). Never waits for the read. Only per-issue stores read here; the
/// other modes read every thread in bulk.
pub async fn want(app: &App, ids: Vec<String>) {
    let Some(reads) = per_issue(app) else {
        return;
    };
    let started = {
        let mut view = app.view.write().await;
        due(&mut view, &ids, Utc::now())
    };
    for (id, updated_at) in started {
        tokio::spawn(read_one(app.clone(), reads.clone(), id, updated_at));
    }
}

/// The beads among `ids` (by id or external ref) whose thread needs a
/// read, marked as reading: `(bead id, updated_at)`.
fn due(
    view: &mut View,
    ids: &[String],
    now: chrono::DateTime<Utc>,
) -> Vec<(String, Option<String>)> {
    let View { beads, history, .. } = view;
    let (Some(feed), Some(store)) = (beads.as_ref(), history.as_mut()) else {
        return Vec::new();
    };
    let mut started = Vec::new();
    for key in ids {
        let Some(bead) = feed.data.find(key) else {
            continue;
        };
        if store.wants_read(&bead.id, bead.updated_at.as_deref(), now) {
            store.reading.insert(bead.id.clone());
            started.push((bead.id.clone(), bead.updated_at.clone()));
        }
    }
    started
}

// --- Snapshot mode -------------------------------------------------------

/// Builds threads from each new snapshot's comments.
async fn from_snapshots(app: App) {
    let mut updates = app.updates.subscribe();
    let mut last: Option<Arc<Snapshot>> = None;
    loop {
        let (snapshot, rule) = current(&app).await;
        if let Some(snapshot) =
            snapshot.filter(|s| !last.as_ref().is_some_and(|l| Arc::ptr_eq(l, s)))
        {
            last = Some(snapshot.clone());
            let built = tokio::task::spawn_blocking(move || {
                snapshot
                    .beads
                    .iter()
                    .map(|b| {
                        let thread = Thread::from_comments(&b.comments, rule.as_deref());
                        (b.id.clone(), thread, b.updated_at.clone())
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            if let Ok(threads) = built {
                commit(&app, Ok(threads)).await;
            }
        }
        if updates.changed().await.is_err() {
            return;
        }
    }
}

/// The shown snapshot and the profile's events rule.
async fn current(app: &App) -> (Option<Arc<Snapshot>>, Option<Arc<EventsRule>>) {
    let view = app.view.read().await;
    (
        view.beads.as_ref().map(|f| f.data.clone()),
        view.profile.events.clone().map(Arc::new),
    )
}

/// Records a whole read: every thread it carried, and drops threads of
/// beads it no longer lists. A failure marks every known thread as failed
/// and keeps its last good read.
async fn commit(app: &App, read: Result<Vec<(String, Thread, Option<String>)>, String>) {
    let now = Utc::now();
    let mut view = app.view.write().await;
    let known: Vec<(String, Option<String>)> = view
        .snapshot()
        .map(|s| {
            s.beads
                .iter()
                .map(|b| (b.id.clone(), b.updated_at.clone()))
                .collect()
        })
        .unwrap_or_default();
    let Some(store) = view.history.as_mut() else {
        return;
    };
    let changed = record_all(store, read, known, now);
    if changed {
        view.revision += 1;
        app.updates.send_replace(view.revision);
    }
}

fn record_all(
    store: &mut Store,
    read: Result<Vec<(String, Thread, Option<String>)>, String>,
    known: Vec<(String, Option<String>)>,
    now: chrono::DateTime<Utc>,
) -> bool {
    let mut changed = false;
    match read {
        Ok(threads) => {
            let seen: HashSet<&str> = threads.iter().map(|(id, ..)| id.as_str()).collect();
            let before = store.threads.len();
            store.threads.retain(|id, _| seen.contains(id.as_str()));
            changed |= store.threads.len() != before;
            for (id, thread, updated_at) in threads {
                changed |= store.record(&id, Ok(thread), updated_at, now);
            }
        }
        Err(error) => {
            for (id, updated_at) in known {
                changed |= store.record(&id, Err(error.clone()), updated_at, now);
            }
        }
    }
    store.reading.clear();
    changed
}

// --- Bulk mode -----------------------------------------------------------

/// Runs `bd export` when the store's commit moves, at most once every
/// `BULK_EVERY`, and records every thread it carries.
async fn bulk(app: App, tool: Tool) {
    let mut done: Option<String> = None;
    let mut started: Option<Instant> = None;
    loop {
        let key = {
            let view = app.view.read().await;
            view.snapshot()
                .map(|s| s.commit.clone().unwrap_or_else(|| s.generated_at.clone()))
        };
        let due = key.is_some()
            && key != done
            && started.is_none_or(|t: Instant| t.elapsed() >= BULK_EVERY);
        if due {
            started = Some(Instant::now());
            {
                let mut view = app.view.write().await;
                let ids: Vec<String> = view
                    .snapshot()
                    .map(|s| s.beads.iter().map(|b| b.id.clone()).collect())
                    .unwrap_or_default();
                if let Some(store) = view.history.as_mut() {
                    store.reading.extend(ids);
                }
            }
            let rule = current(&app).await.1;
            let read = export(&tool, rule).await;
            if let Err(error) = &read {
                eprintln!(
                    "fabriscope: history read failed; keeping the last good threads. {error}"
                );
            } else {
                done = key;
            }
            commit(&app, read).await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Runs `bd export` and reads its threads as the lines arrive, in a
/// blocking task, killing it after `BULK_TIMEOUT`.
async fn export(
    tool: &Tool,
    rule: Option<Arc<EventsRule>>,
) -> Result<Vec<(String, Thread, Option<String>)>, String> {
    let mut child = Command::new(&tool.program)
        .args(&tool.args)
        .arg("export")
        .current_dir(&tool.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run {}: {e}", tool.program.display()))?;
    let stdout = child.stdout.take().ok_or("bd export: no output pipe")?;
    let stderr = child.stderr.take().ok_or("bd export: no error pipe")?;
    // Read stderr apart, so a chatty bd cannot fill its pipe and stall.
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty() && !l.starts_with("Warning:"))
            .take(6)
        {
            text.push_str(line.trim());
            text.push(' ');
        }
        text
    });
    let child = Arc::new(Mutex::new(child));
    let reading = tokio::task::spawn_blocking(move || {
        read_export(std::io::BufReader::new(stdout), rule.as_deref())
    });
    tokio::pin!(reading);
    let threads = match tokio::time::timeout(BULK_TIMEOUT, &mut reading).await {
        Ok(joined) => joined,
        Err(_) => {
            // Killing closes the pipe, so the reader ends.
            let _ = child.lock().unwrap().kill();
            let _ = reading.await;
            let _ = child.lock().unwrap().wait();
            return Err(format!(
                "`bd export` took longer than {}s",
                BULK_TIMEOUT.as_secs()
            ));
        }
    }
    .map_err(|e| format!("bd export reader: {e}"))?;
    let status = child.lock().unwrap().wait().map_err(|e| e.to_string())?;
    let detail = errors.join().unwrap_or_default();
    if !status.success() {
        return Err(format!("`bd export` failed ({status}): {}", detail.trim()));
    }
    threads
}

/// One `bd export` line, keeping only what a thread needs.
#[derive(Deserialize)]
struct ExportLine {
    #[serde(default, rename = "_type")]
    kind: Option<String>,
    id: String,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    comments: Vec<Comment>,
}

/// Reads `bd export` output a line at a time into `(id, thread,
/// updated_at)`, skipping records that are not issues. Only threads are
/// kept, never the whole export.
fn read_export(
    input: impl BufRead,
    rule: Option<&EventsRule>,
) -> Result<Vec<(String, Thread, Option<String>)>, String> {
    let mut out = Vec::new();
    for (number, line) in input.lines().enumerate() {
        let line = line.map_err(|e| format!("bd export line {}: {e}", number + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let issue: ExportLine = serde_json::from_str(&line)
            .map_err(|e| format!("bd export line {}: {e}", number + 1))?;
        if issue.kind.as_deref().is_some_and(|k| k != "issue") {
            continue;
        }
        let thread = Thread::from_comments(&issue.comments, rule);
        out.push((issue.id, thread, issue.updated_at));
    }
    Ok(out)
}

// --- Per-issue mode ------------------------------------------------------

/// The API a per-issue store reads through, and the read permits.
struct PerIssue {
    api: Api,
    permits: Semaphore,
}

/// Per-issue readers by the view they fill. `App` is shared and frozen, so
/// the reader is found by the view's address.
type Registry = Vec<(Weak<RwLock<View>>, Arc<PerIssue>)>;
static PER_ISSUE: Mutex<Registry> = Mutex::new(Vec::new());

fn register(app: &App, api: Api) {
    let mut registry = PER_ISSUE.lock().unwrap();
    registry.retain(|(view, _)| view.strong_count() > 0);
    registry.push((
        Arc::downgrade(&app.view),
        Arc::new(PerIssue {
            api,
            permits: Semaphore::new(PER_ISSUE_PERMITS),
        }),
    ));
}

fn per_issue(app: &App) -> Option<Arc<PerIssue>> {
    let registry = PER_ISSUE.lock().unwrap();
    registry
        .iter()
        .find(|(view, _)| std::ptr::eq(view.as_ptr(), Arc::as_ptr(&app.view)))
        .map(|(_, reads)| reads.clone())
}

/// Ids keep `-`, `_`, `.` and `~` as they are; anything else is encoded.
const ID: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

async fn read_one(app: App, reads: Arc<PerIssue>, id: String, updated_at: Option<String>) {
    let path = format!(
        "/v0/beads/issues/{}?include_comments=true",
        utf8_percent_encode(&id, ID)
    );
    let comments = match reads.permits.acquire().await {
        Ok(_permit) => match reads.api.get(&path, 60).await {
            Ok(Ok(body)) => parse_issue(&body),
            Ok(Err(problem)) => Err(format!("bd serve GET {path}: {problem}")),
            Err(error) => Err(error),
        },
        Err(_) => Err("history reads stopped".into()),
    };
    let rule = app.view.read().await.profile.events.clone();
    let result = comments.map(|c| Thread::from_comments(&c, rule.as_ref()));
    let mut view = app.view.write().await;
    let Some(store) = view.history.as_mut() else {
        return;
    };
    if store.record(&id, result, updated_at, Utc::now()) {
        view.revision += 1;
        app.updates.send_replace(view.revision);
    }
}

/// One issue's comments from `GET /v0/beads/issues/{id}`: the issue itself,
/// or wrapped in `issue` or `item`.
fn parse_issue(body: &[u8]) -> Result<Vec<Comment>, String> {
    #[derive(Deserialize)]
    struct Issue {
        #[serde(default)]
        comments: Vec<Comment>,
        #[serde(default)]
        issue: Option<Box<Issue>>,
        #[serde(default)]
        item: Option<Box<Issue>>,
    }
    let issue: Issue = serde_json::from_slice(body).map_err(|e| format!("bd serve issue: {e}"))?;
    Ok(match (issue.issue, issue.item) {
        (Some(inner), _) | (None, Some(inner)) => inner.comments,
        (None, None) => issue.comments,
    })
}

/// Asks for the threads of beads in a `read_ahead` state, and of beads in
/// progress, whenever the snapshot changes.
async fn read_ahead(app: App) {
    let mut updates = app.updates.subscribe();
    let mut last: Option<Arc<Snapshot>> = None;
    loop {
        let ids = {
            let view = app.view.read().await;
            match view.beads.as_ref().map(|f| f.data.clone()) {
                Some(s) if !last.as_ref().is_some_and(|l| Arc::ptr_eq(l, &s)) => {
                    let ids = ahead(&view, &s);
                    last = Some(s);
                    ids
                }
                _ => Vec::new(),
            }
        };
        if !ids.is_empty() {
            want(&app, ids).await;
        }
        if updates.changed().await.is_err() {
            return;
        }
    }
}

/// Items in a `read_ahead` state or in progress, in display order.
fn ahead(view: &View, s: &Snapshot) -> Vec<String> {
    let reads_ahead = |i: usize| {
        s.derived.is_item(i)
            && (s.beads[i].status == "in_progress"
                || s.derived.rows.get(i).is_some_and(|r| {
                    r.states
                        .iter()
                        .any(|k| view.profile.state(k).is_some_and(|st| st.read_ahead))
                }))
    };
    s.order
        .iter()
        .copied()
        .filter(|&i| reads_ahead(i))
        .take(AHEAD)
        .map(|i| s.beads[i].id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Parsed;

    fn rule() -> EventsRule {
        crate::testing::work_profile().events.unwrap()
    }

    #[test]
    fn the_export_is_read_a_line_at_a_time_into_threads() {
        // An event whose summary has a brace, the marker on its last line; a
        // plain comment; a memory record; an issue with no comments.
        let event = "claim · factory/hostA/rigA · ledger r5\\nnote: {not json} here\\nwork.event.v1 {\\\"schema\\\":\\\"work.event.v1\\\",\\\"kind\\\":\\\"claim\\\",\\\"actor\\\":\\\"factory/hostA/rigA\\\",\\\"at\\\":\\\"2026-11-15T10:00:00Z\\\"}";
        let text = format!(
            "{{\"_type\":\"issue\",\"id\":\"a\",\"updated_at\":\"2026-11-15T10:00:00Z\",\"description\":\"{{big}}\",\"metadata\":{{\"x\":[1,2]}},\"comments\":[{{\"id\":\"c1\",\"author\":\"bob\",\"text\":\"{event}\",\"created_at\":\"2026-11-15T10:00:01Z\"}},{{\"id\":\"c2\",\"author\":\"bob\",\"text\":\"Plain words {{and braces}}\",\"created_at\":\"2026-11-15T10:01:00Z\"}}]}}\n{{\"_type\":\"memory\",\"id\":\"m\"}}\n\n{{\"id\":\"b\"}}\n"
        );
        let threads = read_export(text.as_bytes(), Some(&rule())).unwrap();
        assert_eq!(threads.len(), 2);
        let (id, thread, updated) = &threads[0];
        assert_eq!(id, "a");
        assert_eq!(updated.as_deref(), Some("2026-11-15T10:00:00Z"));
        assert_eq!(thread.events.len(), 1);
        assert_eq!(thread.events[0].kind, "claim");
        assert_eq!(thread.events[0].actor, "factory/hostA/rigA");
        assert_eq!(thread.comments.len(), 1);
        assert_eq!(
            crate::history::parse(&thread.comments[0], &rule()),
            Parsed::Plain
        );
        assert_eq!(threads[1].0, "b");
        assert!(threads[1].1.events.is_empty() && threads[1].1.comments.is_empty());
        assert!(read_export("{broken\n".as_bytes(), None).is_err());
    }

    #[test]
    fn a_whole_read_replaces_threads_and_a_failure_keeps_them() {
        let now = Utc::now();
        let mut store = Store::default();
        let thread = |n: usize| Thread {
            comments: vec![
                Comment {
                    text: "x".into(),
                    ..Default::default()
                };
                n
            ],
            ..Default::default()
        };
        store.reading.insert("a".into());
        let read = vec![
            ("a".to_string(), thread(1), None),
            ("gone".to_string(), thread(0), None),
        ];
        assert!(record_all(&mut store, Ok(read), vec![], now));
        assert!(store.reading.is_empty());
        let same = vec![("a".to_string(), thread(1), None)];
        assert!(
            record_all(&mut store, Ok(same.clone()), vec![], now),
            "gone was dropped"
        );
        assert!(
            !record_all(&mut store, Ok(same), vec![], now),
            "nothing changed"
        );
        let known = vec![("a".to_string(), None)];
        assert!(record_all(&mut store, Err("bd failed".into()), known, now));
        assert_eq!(store.status()["failed"], 1);
        assert_eq!(store.get("a").unwrap().comments.len(), 1, "last good kept");
    }

    #[test]
    fn per_issue_reads_honour_freshness() {
        let mut view = crate::testing::work_view();
        view.history = Some(Store::default());
        let now = Utc::now();
        let ids = vec!["WORK-0001".to_string(), "nope".to_string()];
        let first = due(&mut view, &ids, now);
        assert_eq!(first.len(), 1, "by external ref; unknown ids skipped");
        assert_eq!(first[0].0, "wk-1");
        assert!(due(&mut view, &ids, now).is_empty(), "already reading");
        let store = view.history.as_mut().unwrap();
        store.record("wk-1", Ok(Thread::default()), first[0].1.clone(), now);
        assert!(due(&mut view, &ids, now).is_empty(), "fresh");
        let later = now + chrono::Duration::minutes(11);
        assert_eq!(
            due(&mut view, &ids, later).len(),
            1,
            "stale after ten minutes"
        );
    }

    #[test]
    fn issues_parse_bare_or_wrapped_and_ids_are_encoded() {
        let bare = br#"{"id":"a","comments":[{"text":"hi"}]}"#;
        assert_eq!(parse_issue(bare).unwrap()[0].text, "hi");
        let wrapped = br#"{"issue":{"id":"a","comments":[{"text":"yo"}]}}"#;
        assert_eq!(parse_issue(wrapped).unwrap()[0].text, "yo");
        assert!(parse_issue(br#"{"id":"a"}"#).unwrap().is_empty());
        assert!(parse_issue(b"nope").is_err());
        assert_eq!(
            utf8_percent_encode("pw-a_b.c/d e", ID).to_string(),
            "pw-a_b.c%2Fd%20e"
        );
    }

    #[test]
    fn read_ahead_takes_held_and_working_items() {
        let view = crate::testing::work_view();
        let ids = ahead(&view, view.snapshot().unwrap());
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"wk-1".to_string()) && ids.contains(&"wk-3".to_string()));
    }
}
