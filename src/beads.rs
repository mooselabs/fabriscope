//! Reads a Beads store, through the HTTP API of `bd serve` when one is
//! available and through `bd export` otherwise.
//!
//! Either way Beads supplies every issue with its dependencies and decides
//! which open issues are ready; Fabriscope never re-derives that rule.
//! `bd vc status` is the cheap change probe when a local directory is known.

use crate::serve::Api;
use crate::tools::{Tool, run};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

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
    /// Computed: `ready` / `waiting` for open beads, otherwise the status.
    #[serde(skip)]
    pub state: String,
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

/// The string at a dot-separated path inside a metadata object.
fn metadata_str<'a>(metadata: &'a Value, path: &str) -> Option<&'a str> {
    path.split('.')
        .try_fold(metadata, |value, key| value.get(key))?
        .as_str()
        .filter(|s| !s.is_empty())
}

/// A bead's state from its status and whether `bd ready` listed it.
pub fn state(status: &str, ready: bool) -> String {
    match status {
        "open" if ready => "ready".into(),
        "open" => "waiting".into(),
        other => other.into(),
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
    /// Bead indexes in display order: priority, then age, then id.
    pub order: Vec<usize>,
    /// Ready bead ids in the order `bd ready` returned them.
    pub ready: Vec<String>,
    pub history: History,
}

impl Snapshot {
    #[cfg(test)]
    pub fn new(
        beads: Vec<Bead>,
        ready: Vec<String>,
        commit: Option<String>,
        now: DateTime<Utc>,
    ) -> Self {
        Self::with_parent_key(beads, ready, commit, now, None)
    }

    /// As `new`, resolving each bead's parent from the metadata value at
    /// `parent_key` (a dot-separated path) when it has no parent-child
    /// dependency. The value names the parent by id or external reference.
    pub fn with_parent_key(
        mut beads: Vec<Bead>,
        ready: Vec<String>,
        commit: Option<String>,
        now: DateTime<Utc>,
        parent_key: Option<&str>,
    ) -> Self {
        let ready_set: HashSet<&str> = ready.iter().map(String::as_str).collect();
        for bead in &mut beads {
            bead.state = state(&bead.status, ready_set.contains(bead.id.as_str()));
        }
        let index: HashMap<String, usize> = beads
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id.clone(), i))
            .collect();
        if let Some(key) = parent_key {
            let by_ref: HashMap<&str, &str> = beads
                .iter()
                .filter(|b| !b.external_ref.is_empty())
                .map(|b| (b.external_ref.as_str(), b.id.as_str()))
                .collect();
            let parents: Vec<Option<String>> = beads
                .iter()
                .map(|b| {
                    let named = metadata_str(&b.metadata, key)?;
                    let id = if index.contains_key(named) {
                        named
                    } else {
                        by_ref.get(named)?
                    };
                    (id != b.id).then(|| id.to_string())
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
        let history = history(&beads, now);
        // Keep bd's order, dropping any id the export did not include.
        let ready = ready
            .into_iter()
            .filter(|id| index.contains_key(id))
            .collect();
        Self {
            generated_at: now.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            source: String::new(),
            parent_key: parent_key.map(String::from),
            commit,
            beads,
            index,
            order,
            ready,
            history,
        }
    }

    pub fn get(&self, id: &str) -> Option<&Bead> {
        self.index.get(id).map(|&i| &self.beads[i])
    }

    pub fn ordered(&self) -> impl Iterator<Item = &Bead> {
        self.order.iter().map(|&i| &self.beads[i])
    }

    pub fn count(&self, predicate: impl Fn(&Bead) -> bool) -> usize {
        self.beads.iter().filter(|b| predicate(b)).count()
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
        self.get(key).or_else(|| {
            self.beads
                .iter()
                .find(|b| !b.external_ref.is_empty() && b.external_ref == key)
        })
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

    /// A change signal, and whether it is reliable (a commit) or only a hint
    /// (API stats), in which case the caller also re-reads on a timer.
    pub async fn probe(&self) -> Option<(String, bool)> {
        let local = match self {
            Reader::Api { local, .. } => local.as_ref(),
            Reader::Cli(tool) => Some(tool),
        };
        if let Some(tool) = local {
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

pub async fn load(reader: &Reader, parent_key: Option<&str>) -> Result<Snapshot, String> {
    let (beads, ready, commit) = match reader {
        Reader::Api { api, local } => {
            let (beads, ready, commit) = tokio::join!(
                api.list::<Bead>("/v0/beads/issues", "all=true"),
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
    let mut snapshot = Snapshot::with_parent_key(beads, ready, commit, Utc::now(), parent_key);
    snapshot.source = reader.describe();
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
    fn ready_comes_from_bd_and_other_statuses_pass_through() {
        assert_eq!(state("open", true), "ready");
        assert_eq!(state("open", false), "waiting");
        assert_eq!(state("in_progress", true), "in_progress");
        assert_eq!(state("deferred", false), "deferred");
        let snapshot = Snapshot::new(
            vec![
                bead("a", "open", "2026-10-01T00:00:00Z", None),
                bead("b", "open", "2026-10-01T00:00:00Z", None),
            ],
            vec!["b".into(), "not-in-export".into()],
            None,
            Utc::now(),
        );
        assert_eq!(snapshot.get("a").unwrap().state, "waiting");
        assert_eq!(snapshot.get("b").unwrap().state, "ready");
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
        let s = Snapshot::with_parent_key(
            beads.clone(),
            vec![],
            None,
            Utc::now(),
            Some("tracker.fields.parent"),
        );
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
}
