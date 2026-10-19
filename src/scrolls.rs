//! Reads a Scrolls store through the `scrolls` command line.
//!
//! `scrolls render` writes a fresh export whose `scrolls/graph.json` holds
//! every record and every edge. Fabriscope renders into a private temporary
//! directory, reads that one file, and removes the directory again.

use crate::tools::{Tool, run};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Scroll {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub body: String,
    pub owner: Option<String>,
    pub created: Option<String>,
    pub last_reviewed: Option<String>,
    pub confidence: Option<String>,
    pub path: Option<String>,
    pub aliases: Vec<String>,
    pub fields: Value,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Edge {
    pub from: String,
    pub rel: String,
    pub to: String,
    pub why: Option<String>,
}

#[derive(Deserialize)]
struct Graph {
    records: HashMap<String, Scroll>,
    #[serde(default)]
    edges: HashMap<String, Edge>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub generated_at: String,
    /// The Dolt revision the render was taken at.
    pub revision: Option<String>,
    /// Scrolls in display order: newest first, then title.
    pub scrolls: Vec<Scroll>,
    index: HashMap<String, usize>,
    pub edges: Vec<Edge>,
    outgoing: HashMap<String, Vec<usize>>,
    incoming: HashMap<String, Vec<usize>>,
    by_path: HashMap<String, usize>,
}

impl Snapshot {
    pub fn new(records: Vec<Scroll>, mut edges: Vec<Edge>, revision: Option<String>) -> Self {
        let mut scrolls = records;
        scrolls.sort_by(|a, b| {
            b.created
                .cmp(&a.created)
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.id.cmp(&b.id))
        });
        let index: HashMap<_, _> = scrolls
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), i))
            .collect();
        let mut by_path = HashMap::new();
        for (i, scroll) in scrolls.iter().enumerate() {
            for path in scroll.path.iter().chain(&scroll.aliases) {
                by_path.entry(path.clone()).or_insert(i);
            }
        }
        edges.sort_by(|a, b| (&a.rel, &a.from, &a.to).cmp(&(&b.rel, &b.from, &b.to)));
        let mut outgoing: HashMap<String, Vec<usize>> = HashMap::new();
        let mut incoming: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, edge) in edges.iter().enumerate() {
            outgoing.entry(edge.from.clone()).or_default().push(i);
            incoming.entry(edge.to.clone()).or_default().push(i);
        }
        Self {
            generated_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            revision,
            scrolls,
            index,
            edges,
            outgoing,
            incoming,
            by_path,
        }
    }

    pub fn get(&self, id: &str) -> Option<&Scroll> {
        self.index.get(id).map(|&i| &self.scrolls[i])
    }

    pub fn by_path(&self, path: &str) -> Option<&Scroll> {
        self.by_path.get(path).map(|&i| &self.scrolls[i])
    }

    /// The one scroll whose path or alias ends with `/suffix` (or is it).
    pub fn by_path_suffix(&self, suffix: &str) -> Option<&Scroll> {
        if suffix.is_empty() {
            return None;
        }
        let tail = format!("/{suffix}");
        let mut found = None;
        for (path, &i) in &self.by_path {
            if path == suffix || path.ends_with(&tail) {
                match found {
                    Some(j) if j != i => return None,
                    _ => found = Some(i),
                }
            }
        }
        found.map(|i| &self.scrolls[i])
    }

    pub fn outgoing(&self, id: &str) -> Vec<&Edge> {
        self.outgoing
            .get(id)
            .into_iter()
            .flatten()
            .map(|&i| &self.edges[i])
            .collect()
    }

    pub fn incoming(&self, id: &str) -> Vec<&Edge> {
        self.incoming
            .get(id)
            .into_iter()
            .flatten()
            .map(|&i| &self.edges[i])
            .collect()
    }

    /// Kinds with their record counts, most common first.
    pub fn kinds(&self) -> Vec<(&str, usize)> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for scroll in &self.scrolls {
            *counts.entry(scroll.kind.as_str()).or_default() += 1;
        }
        let mut kinds: Vec<_> = counts.into_iter().collect();
        kinds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        kinds
    }

    /// Edges whose target is an external reference to this bead id.
    pub fn mentions_of_bead(&self, bead: &str) -> Vec<&Edge> {
        self.edges
            .iter()
            .filter(|e| bead_reference(&e.to) == Some(bead))
            .collect()
    }
}

/// The issue id inside a Beads reference: `beads://STORE/ID` or `NAME:ID`.
pub fn bead_reference(target: &str) -> Option<&str> {
    if let Some(rest) = target.strip_prefix("beads://") {
        return rest.rsplit('/').next().filter(|id| !id.is_empty());
    }
    if target.starts_with("sc-") || target.contains("://") {
        return None;
    }
    target
        .split_once(':')
        .map(|(_, id)| id)
        .filter(|id| !id.is_empty())
}

pub fn parse_graph(json: &str, revision: Option<String>) -> Result<Snapshot, String> {
    let graph: Graph =
        serde_json::from_str(json).map_err(|e| format!("scrolls graph.json: {e}"))?;
    Ok(Snapshot::new(
        graph.records.into_values().collect(),
        graph.edges.into_values().collect(),
        revision,
    ))
}

fn revision_from(rendered: &str) -> Option<String> {
    rendered
        .lines()
        .find_map(|line| line.strip_prefix("Dolt revision:"))
        .map(|r| r.trim().to_string())
}

/// Removes the render directory however the read ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub async fn load(tool: &Tool) -> Result<Snapshot, String> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let scratch =
        Scratch(std::env::temp_dir().join(format!("fabriscope-{}-{nonce}", std::process::id())));
    let out = scratch.0.to_string_lossy().into_owned();
    run(tool, &["--json", "render", "--out", &out], 600).await?;
    let graph = tokio::fs::read_to_string(scratch.0.join("scrolls/graph.json"))
        .await
        .map_err(|e| format!("scrolls render wrote no scrolls/graph.json: {e}"))?;
    let rendered = tokio::fs::read_to_string(scratch.0.join("RENDERED"))
        .await
        .unwrap_or_default();
    let revision = revision_from(&rendered);
    tokio::task::spawn_blocking(move || parse_graph(&graph, revision))
        .await
        .map_err(|e| e.to_string())?
}

/// A cheap change probe: sizes and modification times of the store's files.
pub fn fingerprint(store: &Path) -> Result<u64, String> {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for entry in walkdir::WalkDir::new(store).sort_by_file_name() {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().is_file() {
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            entry.path().hash(&mut hash);
            meta.len().hash(&mut hash);
            meta.modified().map_err(|e| e.to_string())?.hash(&mut hash);
        }
    }
    Ok(hash.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str = r#"{
      "records": {
        "sc-a": {"id":"sc-a","kind":"decision","status":"accepted","title":"Use Dolt","body":"See [b](../notes/b.md).","created":"2026-10-02","path":"decisions/a.md","aliases":["docs/decisions/a.md","ADR-1"],"original":{"ignored":true}},
        "sc-b": {"id":"sc-b","kind":"note","status":"recorded","title":"Notes","body":"","created":"2026-10-03","path":"notes/b.md"},
        "sc-c": {"id":"sc-c","kind":"note","status":"recorded","title":"Native","body":"","created":null,"path":null}
      },
      "edges": {
        "1": {"from":"sc-b","rel":"supports","to":"sc-a","why":"measured"},
        "2": {"from":"sc-a","rel":"related_work","to":"beads://0b9e/fx-7"},
        "3": {"from":"sc-c","rel":"related_work","to":"local:fx-8"}
      },
      "schema": {}, "schema_revision": "x"
    }"#;

    #[test]
    fn graph_parses_records_edges_and_paths() {
        let s = parse_graph(GRAPH, Some("rev1".into())).unwrap();
        assert_eq!(s.scrolls.len(), 3);
        // Newest first; a record without a date sorts last.
        assert_eq!(s.scrolls[0].id, "sc-b");
        assert_eq!(s.scrolls[2].id, "sc-c");
        assert_eq!(s.by_path("docs/decisions/a.md").unwrap().id, "sc-a");
        assert_eq!(s.by_path_suffix("notes/b.md").unwrap().id, "sc-b");
        assert_eq!(s.by_path_suffix("b.md").unwrap().id, "sc-b");
        assert!(s.by_path_suffix("").is_none());
        assert_eq!(s.outgoing("sc-b")[0].rel, "supports");
        assert_eq!(s.incoming("sc-a")[0].from, "sc-b");
        assert_eq!(s.mentions_of_bead("fx-7").len(), 1);
        assert_eq!(s.mentions_of_bead("fx-8").len(), 1);
        assert_eq!(s.kinds()[0], ("note", 2));
        assert!(parse_graph("{}", None).is_err());
        assert!(parse_graph("[]", None).is_err());
    }

    #[test]
    fn ambiguous_path_suffixes_resolve_to_nothing() {
        let records = vec![
            Scroll {
                id: "sc-1".into(),
                path: Some("a/readme.md".into()),
                ..Default::default()
            },
            Scroll {
                id: "sc-2".into(),
                path: Some("b/readme.md".into()),
                ..Default::default()
            },
        ];
        let s = Snapshot::new(records, vec![], None);
        assert!(s.by_path_suffix("readme.md").is_none());
        assert_eq!(s.by_path_suffix("a/readme.md").unwrap().id, "sc-1");
    }

    #[test]
    fn bead_references_and_render_revision() {
        assert_eq!(bead_reference("beads://store-uuid/fx-1"), Some("fx-1"));
        assert_eq!(bead_reference("work:fx-2"), Some("fx-2"));
        assert_eq!(bead_reference("sc-123"), None);
        assert_eq!(bead_reference("https://example.org/x"), None);
        assert_eq!(
            revision_from("Scrolls 0.3.0\nDolt revision: abc\n"),
            Some("abc".into())
        );
    }
}
