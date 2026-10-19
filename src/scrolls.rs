//! Validated Scrolls graph snapshots and document/link indexes.

use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Default, Deserialize)]
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
pub struct Edge {
    pub from: String,
    pub rel: String,
    pub to: String,
    pub why: Option<String>,
}

#[derive(Deserialize)]
struct Graph {
    records: HashMap<String, Scroll>,
    edges: HashMap<String, Edge>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub generated_at: String,
    pub source: String,
    pub store_id: String,
    pub branch: String,
    pub schema_revision: String,
    /// The Dolt revision of this HTTP snapshot.
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
            source: String::new(),
            store_id: String::new(),
            branch: String::new(),
            schema_revision: String::new(),
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

#[cfg(test)]
pub fn parse_graph(json: &str, revision: Option<String>) -> Result<Snapshot, String> {
    parse_graph_value(
        serde_json::from_str(json).map_err(|e| format!("Scrolls graph: {e}"))?,
        revision,
    )
}

pub fn parse_graph_value(value: Value, revision: Option<String>) -> Result<Snapshot, String> {
    let graph: Graph =
        serde_json::from_value(value).map_err(|_| "Invalid Scrolls graph structure".to_string())?;
    for (key, record) in &graph.records {
        if key.is_empty()
            || key != &record.id
            || record.kind.is_empty()
            || record.revision == 0
            || !record.fields.is_object()
        {
            return Err("Invalid Scrolls graph record identity, revision or fields".into());
        }
    }
    if graph
        .edges
        .iter()
        .any(|(key, e)| key.is_empty() || e.from.is_empty() || e.rel.is_empty() || e.to.is_empty())
    {
        return Err("Invalid Scrolls graph relationship".into());
    }
    Ok(Snapshot::new(
        graph.records.into_values().collect(),
        graph.edges.into_values().collect(),
        revision,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str = r#"{
      "records": {
        "sc-a": {"id":"sc-a","kind":"decision","status":"accepted","title":"Use Dolt","body":"See [b](../notes/b.md).","fields":{},"revision":1,"created":"2026-10-02","path":"decisions/a.md","aliases":["docs/decisions/a.md","ADR-1"],"original":{"ignored":true}},
        "sc-b": {"id":"sc-b","kind":"note","status":"recorded","title":"Notes","body":"","fields":{},"revision":1,"aliases":[],"created":"2026-10-03","path":"notes/b.md"},
        "sc-c": {"id":"sc-c","kind":"note","status":"recorded","title":"Native","body":"","fields":{},"revision":1,"aliases":[],"created":null,"path":null}
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
    fn bead_references() {
        assert_eq!(bead_reference("beads://store-uuid/fx-1"), Some("fx-1"));
        assert_eq!(bead_reference("work:fx-2"), Some("fx-2"));
        assert_eq!(bead_reference("sc-123"), None);
        assert_eq!(bead_reference("https://example.org/x"), None);
    }
}
