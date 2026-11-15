//! Reads a Beads store, through the HTTP API of `bd serve` when one is
//! available and through `bd export` otherwise.
//!
//! Either way Beads supplies every issue with its dependencies and decides
//! which open issues are ready; Fabriscope never re-derives that rule.
//! `bd vc status` is the cheap change probe when a local directory is known.

use crate::profile::{Derived, Profile};
use crate::serve::Api;
use crate::tools::{Tool, run};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Bead {
    pub id: String,
    pub title: String,
    pub description: String,
    pub design: String,
    pub acceptance_criteria: String,
    pub notes: String,
    pub status: String,
    pub priority: i64,
    pub issue_type: String,
    pub assignee: String,
    pub labels: Vec<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub closed_at: Option<String>,
    pub close_reason: String,
    pub external_ref: String,
    pub dependencies: Vec<Dependency>,
    pub metadata: Value,
    /// Native columns a store may fill. On a store that leaves them out
    /// they stay empty; only `started_at` (kept after close) and
    /// `comment_count` are common.
    pub started_at: Option<String>,
    pub created_by: String,
    pub owner: String,
    pub estimated_minutes: Option<i64>,
    pub due_at: Option<String>,
    pub defer_until: Option<String>,
    pub lease_expires_at: Option<String>,
    pub heartbeat_at: Option<String>,
    pub is_blocked: Option<bool>,
    pub comment_count: Option<u64>,
    /// Comments, when the read carries them (`bd export` does; the list API
    /// does not). History events are parsed from these; see `history`.
    pub comments: Vec<Comment>,
    /// Computed: whether `bd ready` listed this bead. Readiness is Beads'
    /// answer about dependencies, shown beside the stored status, never
    /// instead of it.
    #[serde(skip)]
    pub ready: bool,
    /// Computed: the parent named by `--parent-key`, when the store keeps
    /// membership in metadata rather than in parent-child dependencies.
    #[serde(skip)]
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Dependency {
    pub issue_id: String,
    pub depends_on_id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// One comment on a bead, as `bd export` and `bd show` print it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Comment {
    pub id: Option<String>,
    pub author: Option<String>,
    pub text: String,
    pub created_at: Option<String>,
}

impl Bead {
    pub fn is_closed(&self) -> bool {
        self.status == "closed"
    }

    pub fn is_epic(&self) -> bool {
        self.issue_type == "epic"
    }

    /// The epic or other parent this bead belongs to: its parent-child
    /// dependency, or else the parent its metadata names.
    pub fn parent(&self) -> Option<&str> {
        self.dependencies
            .iter()
            .find(|d| d.kind == "parent-child")
            .map(|d| d.depends_on_id.as_str())
            .or(self.parent_id.as_deref())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub generated_at: String,
    /// How the store was read: `bd serve <url>` or `bd export`.
    pub source: String,
    /// The metadata path that names a bead's parent, if one was given.
    pub parent_key: Option<String>,
    /// The store's version-control commit, when `bd vc status` answers.
    pub commit: Option<String>,
    pub beads: Vec<Bead>,
    index: HashMap<String, usize>,
    /// External reference → index, first bead wins.
    by_ref: HashMap<String, usize>,
    /// Bead indexes in display order: priority, then age, then id.
    pub order: Vec<usize>,
    /// Ready bead ids in the order `bd ready` returned them.
    pub ready: Vec<String>,
    /// Counts per UTC day, over work items only.
    pub history: History,
    /// What the profile says about each bead, index-aligned with `beads`.
    pub derived: Derived,
}

impl Snapshot {
    #[cfg(test)]
    pub fn new(
        beads: Vec<Bead>,
        ready: Vec<String>,
        commit: Option<String>,
        now: DateTime<Utc>,
    ) -> Self {
        Self::build(beads, ready, commit, now, &Arc::default())
    }

    /// Builds a snapshot: marks what `bd ready` listed, resolves each bead's
    /// parent from the profile's `[parent]` source when it has no
    /// parent-child dependency (the value names the parent by id or external
    /// reference), applies the profile (`derived`), and counts history over
    /// items. A bead in a state with `overrides_ready`, and any row that is
    /// not an item, is never ready.
    pub fn build(
        mut beads: Vec<Bead>,
        ready: Vec<String>,
        commit: Option<String>,
        now: DateTime<Utc>,
        profile: &Arc<Profile>,
    ) -> Self {
        let ready_set: HashSet<&str> = ready.iter().map(String::as_str).collect();
        for bead in &mut beads {
            bead.ready = ready_set.contains(bead.id.as_str());
        }
        let index: HashMap<String, usize> = beads
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id.clone(), i))
            .collect();
        let mut by_ref: HashMap<String, usize> = HashMap::new();
        for (i, b) in beads.iter().enumerate() {
            if !b.external_ref.is_empty() {
                by_ref.entry(b.external_ref.clone()).or_insert(i);
            }
        }
        let parent_key = profile.parent.as_ref().map(|p| &p.path);
        if let Some(key) = parent_key {
            let parents: Vec<Option<String>> = beads
                .iter()
                .map(|b| {
                    let named = key.text(b)?;
                    let i = index.get(&named).or_else(|| by_ref.get(&named))?;
                    let id = &beads[*i].id;
                    (*id != b.id).then(|| id.clone())
                })
                .collect();
            for (bead, parent) in beads.iter_mut().zip(parents) {
                bead.parent_id = parent;
            }
        }
        let mut order: Vec<usize> = (0..beads.len()).collect();
        order.sort_by(|&a, &b| {
            let (a, b) = (&beads[a], &beads[b]);
            (a.priority, &a.created_at, &a.id).cmp(&(b.priority, &b.created_at, &b.id))
        });
        let mut snapshot = Self {
            generated_at: now.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            source: String::new(),
            parent_key: parent_key.map(|k| k.to_string()),
            commit,
            beads,
            index,
            by_ref,
            order,
            ready: Vec::new(),
            history: History::default(),
            derived: Derived::default(),
        };
        let derived = Derived::build(&snapshot, profile, now);
        for (i, bead) in snapshot.beads.iter_mut().enumerate() {
            let row = &derived.rows[i];
            let held = row
                .states
                .iter()
                .any(|k| profile.state(k).is_some_and(|s| s.overrides_ready));
            if held || !derived.is_item(i) {
                bead.ready = false;
            }
        }
        let items: Vec<Bead> = snapshot
            .beads
            .iter()
            .enumerate()
            .filter(|(i, _)| derived.is_item(*i))
            .map(|(_, b)| b.clone())
            .collect();
        snapshot.history = history(&items, now);
        // Keep bd's order, dropping any id the export did not include and
        // any bead the profile says is not ready.
        snapshot.ready = ready
            .into_iter()
            .filter(|id| snapshot.get(id).is_some_and(|b| b.ready))
            .collect();
        snapshot.derived = derived;
        snapshot
    }

    /// A bead's index in `beads` (and in `derived.rows`).
    pub fn position(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    pub fn get(&self, id: &str) -> Option<&Bead> {
        self.index.get(id).map(|&i| &self.beads[i])
    }

    pub fn ordered(&self) -> impl Iterator<Item = &Bead> {
        self.order.iter().map(|&i| &self.beads[i])
    }

    pub fn children(&self, id: &str) -> Vec<&Bead> {
        self.ordered().filter(|b| b.parent() == Some(id)).collect()
    }

    /// Beads whose dependencies name `id`, with the dependency type.
    pub fn dependents(&self, id: &str) -> Vec<(&Bead, &str)> {
        let mut out = Vec::new();
        for bead in self.ordered() {
            for dep in &bead.dependencies {
                if dep.depends_on_id == id {
                    out.push((bead, dep.kind.as_str()));
                }
            }
        }
        out
    }

    /// A bead by id, or by its external reference when nothing has that id.
    pub fn find(&self, key: &str) -> Option<&Bead> {
        self.get(key)
            .or_else(|| self.by_ref.get(key).map(|&i| &self.beads[i]))
    }
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub days: Vec<Day>,
    pub windows: Vec<Window>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Day {
    pub date: NaiveDate,
    pub start: usize,
    pub end: usize,
    pub added: usize,
    pub closed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub days: usize,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub added: usize,
    pub closed: usize,
    pub net_change: i64,
}

fn timestamp(value: &Option<String>) -> Option<DateTime<Utc>> {
    value
        .as_deref()
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// Open beads over the last 28 complete UTC days, from each bead's created
/// and closed timestamps. A bead that is open now counts as open since it was
/// created, even if it was closed and reopened; deleted beads are not seen.
pub fn history(beads: &[Bead], now: DateTime<Utc>) -> History {
    let spans: Vec<(DateTime<Utc>, Option<DateTime<Utc>>)> = beads
        .iter()
        .filter_map(|b| {
            let created = timestamp(&b.created_at)?;
            let closed = if b.is_closed() {
                timestamp(&b.closed_at)
            } else {
                None
            };
            Some((created, closed))
        })
        .collect();
    let Some(first) = spans.iter().map(|(c, _)| *c).min() else {
        return History::default();
    };
    let open_at = |t: DateTime<Utc>| {
        spans
            .iter()
            .filter(|(c, closed)| *c <= t && closed.is_none_or(|x| x > t))
            .count()
    };
    let in_range = |t: Option<DateTime<Utc>>, from: DateTime<Utc>, to: DateTime<Utc>| {
        t.is_some_and(|t| from < t && t <= to)
    };
    let today = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let mut days = Vec::new();
    for ago in (1..=28).rev() {
        let from = today - Duration::days(ago);
        let to = from + Duration::days(1);
        if to <= first {
            continue;
        }
        days.push(Day {
            date: from.date_naive(),
            start: open_at(from),
            end: open_at(to),
            added: spans
                .iter()
                .filter(|(c, _)| in_range(Some(*c), from, to))
                .count(),
            closed: spans.iter().filter(|(_, x)| in_range(*x, from, to)).count(),
        });
    }
    let mut windows = Vec::new();
    for size in [3, 7, 14, 28] {
        if days.len() < size {
            continue;
        }
        let sample = &days[days.len() - size..];
        windows.push(Window {
            days: size,
            start: sample[0].date,
            end: sample[size - 1].date,
            added: sample.iter().map(|d| d.added).sum(),
            closed: sample.iter().map(|d| d.closed).sum(),
            net_change: sample[size - 1].end as i64 - sample[0].start as i64,
        });
    }
    History { days, windows }
}

pub fn parse_export(text: &str) -> Result<Vec<Bead>, String> {
    let mut beads = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|e| format!("bd export line {}: {e}", number + 1))?;
        // Memories and other record types share the stream; keep issues.
        if value
            .get("_type")
            .and_then(Value::as_str)
            .is_some_and(|t| t != "issue")
        {
            continue;
        }
        beads.push(
            serde_json::from_value(value)
                .map_err(|e| format!("bd export line {}: {e}", number + 1))?,
        );
    }
    Ok(beads)
}

pub fn parse_ready(text: &str) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
    }
    let rows: Vec<Row> = serde_json::from_str(text).map_err(|e| format!("bd ready: {e}"))?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

/// Where Beads data comes from.
#[derive(Clone, Debug)]
pub enum Reader {
    /// A `bd serve` endpoint, and the local `bd` when there is one (for the
    /// commit probe).
    Api { api: Api, local: Option<Tool> },
    /// `bd export` and `bd ready`, for a store `bd serve` cannot open.
    Cli(Tool),
}

impl Reader {
    pub fn describe(&self) -> String {
        match self {
            Reader::Api { api, .. } => format!("bd serve {}", api.base()),
            Reader::Cli(_) => "bd export".into(),
        }
    }

    /// The local `bd`, when there is one: it answers what `bd serve` does
    /// not serve, such as the commit and provenance logs.
    pub fn local(&self) -> Option<&Tool> {
        match self {
            Reader::Api { local, .. } => local.as_ref(),
            Reader::Cli(tool) => Some(tool),
        }
    }

    /// A change signal, and whether it is reliable (a commit) or only a hint
    /// (API stats), in which case the caller also re-reads on a timer.
    pub async fn probe(&self) -> Option<(String, bool)> {
        if let Some(tool) = self.local() {
            if let Some(commit) = commit(tool).await {
                return Some((commit, true));
            }
        }
        match self {
            Reader::Api { api, .. } => api.probe().await.ok().map(|s| (s, false)),
            Reader::Cli(_) => None,
        }
    }
}

/// The store's current commit, or `None` when the backend has no version
/// control.
pub async fn commit(tool: &Tool) -> Option<String> {
    #[derive(Deserialize)]
    struct Status {
        commit: String,
    }
    let out = run(tool, &["vc", "status", "--json"], 15).await.ok()?;
    serde_json::from_str::<Status>(&out).ok().map(|s| s.commit)
}

#[derive(Deserialize)]
struct IdRow {
    id: String,
}

/// The query of the full list read. Gate-type issues are left out of the
/// list API unless asked for; `bd export` includes them already.
pub const LIST_QUERY: &str = "all=true&include_gates=true";

/// Reads the store and builds a snapshot with `profile` off the async
/// threads (the derivation walks every bead).
pub async fn load(reader: &Reader, profile: Arc<Profile>) -> Result<Snapshot, String> {
    let (beads, ready, commit) = match reader {
        Reader::Api { api, local } => {
            let (beads, ready, commit) = tokio::join!(
                api.list::<Bead>("/v0/beads/issues", LIST_QUERY),
                api.list::<IdRow>("/v0/beads/ready", "brief=true"),
                async {
                    match local {
                        Some(tool) => commit(tool).await,
                        None => None,
                    }
                },
            );
            (beads?, ready?.into_iter().map(|r| r.id).collect(), commit)
        }
        Reader::Cli(tool) => {
            let (export, ready, commit) = tokio::join!(
                run(tool, &["export"], 300),
                run(tool, &["ready", "--json", "--limit", "0"], 300),
                commit(tool),
            );
            (parse_export(&export?)?, parse_ready(&ready?)?, commit)
        }
    };
    let source = reader.describe();
    let mut snapshot = tokio::task::spawn_blocking(move || {
        Snapshot::build(beads, ready, commit, Utc::now(), &profile)
    })
    .await
    .map_err(|e| format!("building the snapshot failed: {e}"))?;
    snapshot.source = source;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bead(id: &str, status: &str, created: &str, closed: Option<&str>) -> Bead {
        Bead {
            id: id.into(),
            title: id.into(),
            status: status.into(),
            created_at: Some(created.into()),
            closed_at: closed.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn export_keeps_issues_and_reads_dependencies() {
        let text = r#"{"_type":"issue","id":"fx-1","title":"One","status":"open","priority":1,"issue_type":"task","dependencies":[{"issue_id":"fx-1","depends_on_id":"fx-e","type":"parent-child"}]}
{"_type":"memory","id":"m-1","title":"Not an issue"}

{"_type":"issue","id":"fx-e","title":"Epic","status":"open","priority":0,"issue_type":"epic"}"#;
        let beads = parse_export(text).unwrap();
        assert_eq!(beads.len(), 2);
        assert_eq!(beads[0].parent(), Some("fx-e"));
        assert!(beads[1].is_epic());
        assert!(parse_export("{not json").is_err());
    }

    #[test]
    fn ready_comes_from_bd_and_the_status_is_kept_as_stored() {
        let snapshot = Snapshot::new(
            vec![
                bead("a", "open", "2026-10-01T00:00:00Z", None),
                bead("b", "open", "2026-10-01T00:00:00Z", None),
            ],
            vec!["b".into(), "not-in-export".into()],
            None,
            Utc::now(),
        );
        assert!(!snapshot.get("a").unwrap().ready);
        assert!(snapshot.get("b").unwrap().ready);
        assert_eq!(snapshot.get("b").unwrap().status, "open");
        assert_eq!(snapshot.ready, vec!["b".to_string()]);
    }

    #[test]
    fn display_order_is_priority_then_age_then_id() {
        let mut low = bead("z", "open", "2026-10-01T00:00:00Z", None);
        low.priority = 3;
        let mut old = bead("y", "open", "2026-09-01T00:00:00Z", None);
        old.priority = 1;
        let mut new = bead("x", "open", "2026-10-02T00:00:00Z", None);
        new.priority = 1;
        let s = Snapshot::new(vec![low, new, old], vec![], None, Utc::now());
        let ids: Vec<_> = s.ordered().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["y", "x", "z"]);
    }

    #[test]
    fn history_counts_open_added_and_closed_per_utc_day() {
        let now = DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let beads = vec![
            bead(
                "a",
                "closed",
                "2026-10-17T09:00:00Z",
                Some("2026-10-18T10:00:00Z"),
            ),
            bead("b", "open", "2026-10-18T09:00:00Z", None),
            // Reopened: the stale closed_at is ignored while it is open.
            bead(
                "c",
                "open",
                "2026-10-17T08:00:00Z",
                Some("2026-10-18T08:00:00Z"),
            ),
        ];
        let h = history(&beads, now);
        let dates: Vec<_> = h.days.iter().map(|d| d.date.to_string()).collect();
        assert_eq!(dates, ["2026-10-17", "2026-10-18", "2026-10-19"]);
        assert_eq!((h.days[0].start, h.days[0].end, h.days[0].added), (0, 2, 2));
        assert_eq!(
            (h.days[1].end, h.days[1].added, h.days[1].closed),
            (2, 1, 1)
        );
        assert_eq!(h.days[2].end, 2);
        assert_eq!(h.windows.len(), 1);
        assert_eq!(h.windows[0].net_change, 2);
        assert!(history(&[], now).days.is_empty());
    }

    #[test]
    fn a_parent_key_reads_membership_from_metadata() {
        let mut epic = bead("fx-e", "open", "2026-10-01T00:00:00Z", None);
        epic.issue_type = "epic".into();
        epic.external_ref = "EPIC-1".into();
        let mut by_ref = bead("fx-1", "open", "2026-10-01T00:00:00Z", None);
        by_ref.metadata = serde_json::json!({"tracker": {"fields": {"parent": "EPIC-1"}}});
        let mut by_id = bead("fx-2", "closed", "2026-10-01T00:00:00Z", None);
        by_id.metadata = serde_json::json!({"tracker": {"fields": {"parent": "fx-e"}}});
        let mut native = bead("fx-3", "open", "2026-10-01T00:00:00Z", None);
        native.dependencies = vec![Dependency {
            issue_id: "fx-3".into(),
            depends_on_id: "fx-other".into(),
            kind: "parent-child".into(),
        }];
        native.metadata = serde_json::json!({"tracker": {"fields": {"parent": "fx-e"}}});
        let mut unknown = bead("fx-4", "open", "2026-10-01T00:00:00Z", None);
        unknown.metadata = serde_json::json!({"tracker": {"fields": {"parent": "EPIC-404"}}});
        let beads = vec![epic, by_ref, by_id, native, unknown];
        let s = Snapshot::build(beads.clone(), vec![], None, Utc::now(), &{
            let mut p = Profile::default();
            p.set_parent_key(Some("tracker.fields.parent")).unwrap();
            Arc::new(p)
        });
        assert_eq!(s.parent_key.as_deref(), Some("tracker.fields.parent"));
        let children: Vec<_> = s.children("fx-e").iter().map(|b| b.id.as_str()).collect();
        assert_eq!(children, ["fx-1", "fx-2"]);
        assert_eq!(
            s.get("fx-3").unwrap().parent(),
            Some("fx-other"),
            "parent-child wins"
        );
        assert_eq!(s.get("fx-4").unwrap().parent(), None);
        assert!(
            Snapshot::new(beads, vec![], None, Utc::now())
                .children("fx-e")
                .is_empty()
        );
    }

    #[test]
    fn find_falls_back_to_the_external_reference() {
        let mut b = bead("fx-9", "open", "2026-10-01T00:00:00Z", None);
        b.external_ref = "TICKET-12".into();
        let s = Snapshot::new(vec![b], vec![], None, Utc::now());
        assert_eq!(s.find("TICKET-12").unwrap().id, "fx-9");
        assert_eq!(s.find("fx-9").unwrap().id, "fx-9");
        assert!(s.find("").is_none());
    }

    #[test]
    fn the_list_read_asks_for_gates_and_comments_parse() {
        assert!(LIST_QUERY.split('&').any(|p| p == "include_gates=true"));
        let text = r#"{"_type":"issue","id":"g-1","title":"Gate","status":"open","issue_type":"gate","started_at":"2026-11-15T10:00:00Z","comment_count":1,"comments":[{"id":"c1","issue_id":"g-1","author":"alice","text":"hello","created_at":"2026-11-15T10:01:00Z"}]}"#;
        let beads = parse_export(text).unwrap();
        assert_eq!(beads[0].comments[0].author.as_deref(), Some("alice"));
        assert_eq!(beads[0].comment_count, Some(1));
        assert_eq!(beads[0].started_at.as_deref(), Some("2026-11-15T10:00:00Z"));
    }
}
