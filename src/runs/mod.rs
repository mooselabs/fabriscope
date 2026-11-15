//! The run feed (`runfeed.v1`, spec in docs/runfeed-v1.md, schema in
//! schema/runfeed-v1.json): what a factory's runs did, read from a
//! directory or a URL.
//!
//! These types follow the spec field for field and are lenient as it asks:
//! unknown fields are ignored, and every enum (run state, step kind and
//! result, document state, fact type, link rel, …) is a `String`, so an
//! unknown value is kept and shown neutrally. The classifier functions
//! (`run_tone`, `step_tone`, `doc_readable`, `step_kind`) turn them into
//! display decisions. `Index` is the in-memory join from subjects, beads and
//! attempt ids to runs; `client` (WP5) keeps it current and `cache` holds run
//! docs and document pages.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "contract the packages call; WP5 and WP6 read it (BKLG-20261115-4e2r)"
    )
)]

pub mod cache;
pub mod client;

use crate::beads::Bead;
use crate::profile::Tone;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// `feed.json`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Feed {
    /// Always `runfeed.v1`; `load` refuses anything else.
    pub schema: String,
    pub producer: Producer,
    pub factory: String,
    /// Grows with every publish; poll this.
    pub revision: u64,
    pub generated_at: String,
    /// Href of `index.json`.
    pub index: String,
    /// Hrefs of older index shards.
    pub shards: Vec<String>,
    pub source_health: SourceHealth,
    /// Board name → href.
    pub boards: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Producer {
    pub name: String,
    pub version: String,
    pub commit: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SourceHealth {
    pub at: Option<String>,
    pub cursor: Option<String>,
    /// Inputs the producer could not read; empty means healthy.
    pub gaps: Vec<String>,
}

/// `index.json` or an `index/YYYY-MM.json` shard (`runfeed.index.v1`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct IndexFile {
    pub schema: String,
    pub revision: u64,
    pub runs: Vec<RunSummary>,
}

/// One run in an index: enough for an overview without the run doc.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RunSummary {
    pub run: String,
    pub subject: Subject,
    /// Free grouping dimensions (`rig`, `repository`, …).
    pub group: BTreeMap<String, String>,
    /// A run state; see `run_tone`.
    pub state: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub updated_at: String,
    pub digest: String,
    /// Href of the run doc.
    pub href: String,
    pub cost_usd: Option<f64>,
    pub attempts: u32,
    pub stop_code: Option<String>,
}

/// What a run worked on: at least one of `ref` (the human-facing reference,
/// an external ref) and `id`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Subject {
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub id: Option<String>,
    pub title: Option<String>,
}

/// `subjects/<key>.json` (`runfeed.subject.v1`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SubjectFile {
    pub schema: String,
    pub subject: Subject,
    pub runs: Vec<SubjectRun>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SubjectRun {
    pub run: String,
    /// The run's current attempt id.
    pub attempt: Option<String>,
    pub state: String,
    pub started_at: Option<String>,
    pub href: String,
}

/// `runs/<run>.json` (`runfeed.run.v1`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Run {
    pub schema: String,
    pub run: String,
    /// `attempt`, `contract`, `branch`, `commit`, `tree`, and free keys.
    pub refs: BTreeMap<String, String>,
    pub subject: Subject,
    pub group: BTreeMap<String, String>,
    pub state: String,
    pub current_step: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub updated_at: String,
    /// Numbered 1..n in order.
    pub attempts: Vec<Attempt>,
    pub costs: Option<Cost>,
    /// Role → which model played it.
    pub roles: BTreeMap<String, Role>,
    pub timing: Option<Timing>,
    pub sessions: Vec<Session>,
    pub outcome: Option<Outcome>,
    pub stop: Option<Stop>,
    pub links: Vec<Link>,
    /// Every document of the run; steps name them by id.
    pub documents: Vec<DocRef>,
    pub facts: Vec<Fact>,
    /// What the producer knows is missing from this doc.
    pub gaps: Vec<String>,
    pub digest: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Attempt {
    /// Equal to the ledger's attempt id when the producer knows it
    /// (`provenance::Attempt::id` joins here).
    pub id: String,
    pub number: u32,
    pub state: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub steps: Vec<Step>,
    pub facts: Vec<Fact>,
    pub costs: Option<Cost>,
    pub links: Vec<Link>,
    pub stop: Option<Stop>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Step {
    pub id: String,
    /// Open; see `step_kind` for the core set.
    pub kind: String,
    pub name: Option<String>,
    pub label: Option<String>,
    pub iteration: Option<u32>,
    /// `pending`, `running`, `done`, `unknown`.
    pub state: String,
    /// `pass`, `fail`, `running`, `skipped`, `cancelled`, `unknown`.
    pub result: Option<String>,
    pub result_detail: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub seconds: Option<f64>,
    pub failure: Option<Failure>,
    /// Id of an entry in `Run::sessions`.
    pub session: Option<String>,
    pub facts: Vec<Fact>,
    pub links: Vec<Link>,
    /// Ids of entries in `Run::documents`.
    pub documents: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Failure {
    pub code: Option<String>,
    pub text: Option<String>,
}

/// An open-ended fact. Show `label`, else `Profile::fact_label(key)`, else
/// the key.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Fact {
    pub key: String,
    /// A string, number or boolean.
    pub value: Value,
    /// `seconds`, `hours`, `usd`, `tokens`, `count`, `percent`, `sha`,
    /// `tree`, `bool`, `id`, `timestamp`, `text`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub label: Option<String>,
    /// `good`, `warn`, `bad`; see `fact_tone`.
    pub tone: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<FactRef>,
}

/// A fact's link to another item, run, attempt, commit or tree.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct FactRef {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Cost {
    pub usd: Option<f64>,
    pub usd_with_overhead: Option<f64>,
    pub tokens: Option<Tokens>,
    pub by_model: BTreeMap<String, ModelCost>,
    pub by_role: BTreeMap<String, RoleCost>,
    /// False whenever known usage is missing: show the total as a lower
    /// bound.
    pub complete: bool,
    pub gaps: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Tokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelCost {
    pub usd: Option<f64>,
    pub tokens: Option<Tokens>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RoleCost {
    pub usd: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Role {
    pub model: Option<String>,
    pub provider: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Timing {
    pub wall_hours: Option<f64>,
    pub active_hours: Option<f64>,
    /// The producer's size label.
    pub timebox: Option<String>,
    pub timebox_hours_max: Option<f64>,
    pub over_timebox: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Session {
    pub id: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub started_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Outcome {
    pub facts: Vec<Fact>,
}

/// Why a run (or attempt) halted for a person, and what they can do.
/// `commands` are shown verbatim to copy and never run.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Stop {
    pub code: String,
    pub cause: Option<String>,
    pub class: Option<String>,
    pub title: Option<String>,
    pub why: Option<String>,
    pub detail: Vec<String>,
    pub next: Option<String>,
    pub commands: Vec<String>,
    pub owner_ask: Option<OwnerAsk>,
    pub at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct OwnerAsk {
    pub problem: Option<String>,
    pub question: Option<String>,
    pub recommended: Option<String>,
    pub if_yes: Option<String>,
    pub if_not: Option<String>,
}

/// `href` is an absolute external URL (make only `https://` clickable);
/// `ref` an identifier to link internally.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Link {
    /// `pull_request`, `commit`, `branch`, `tree`, `ci`, `item`, `attempt`,
    /// `external`.
    pub rel: String,
    pub href: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub text: Option<String>,
}

/// One document of a run and its pages.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DocRef {
    pub id: String,
    /// `transcript`, `log`, `diagnostic`, `notes`, `contract`, `receipt`,
    /// `text`.
    pub kind: String,
    pub title: String,
    /// `text/plain`, `application/x-ndjson`, `application/json`.
    pub content_type: String,
    pub bytes: Option<u64>,
    pub pages: Vec<Page>,
    /// See `doc_readable`: only `ready`, `live` and `truncated` have pages.
    pub state: String,
    pub source: Option<DocSource>,
    pub redaction: Option<Redaction>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Page {
    pub href: String,
    pub bytes: Option<u64>,
    pub first_at: Option<String>,
    pub last_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DocSource {
    pub kind: Option<String>,
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Redaction {
    pub policy: String,
    pub replaced: u64,
}

/// `boards/<name>.json` (`runfeed.board.v1`): a producer read model passed
/// through; render `body` generically unless `source_schema` is known.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Board {
    pub schema: String,
    pub name: String,
    pub title: Option<String>,
    pub source_schema: Option<String>,
    pub generated_at: String,
    pub body: Value,
}

/// One transcript turn (a line of an `application/x-ndjson` page). `text`
/// is plain text: always escape it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Turn {
    pub at: Option<String>,
    /// `system`, `user`, `assistant`, `thinking`, `tool_call`,
    /// `tool_result`.
    pub role: String,
    pub tool: Option<String>,
    pub model: Option<String>,
    pub text: String,
    pub omitted_bytes: Option<u64>,
}

/// One line of `changes.jsonl`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Change {
    pub revision: u64,
    pub at: String,
    pub run: String,
    pub digest: String,
}

/// The tone for a run or attempt state; unknown states are neutral.
pub fn run_tone(state: &str) -> Tone {
    match state {
        "passed" => Tone::Good,
        "failed" => Tone::Bad,
        "stopped" => Tone::Warn,
        "running" => Tone::Waiting,
        _ => Tone::Neutral,
    }
}

/// The tone for a step result; unknown results are neutral.
pub fn step_tone(result: &str) -> Tone {
    match result {
        "pass" => Tone::Good,
        "fail" => Tone::Bad,
        "running" => Tone::Waiting,
        _ => Tone::Neutral,
    }
}

/// The tone a fact asks for (`good`, `warn`, `bad`), else neutral.
pub fn fact_tone(tone: Option<&str>) -> Tone {
    match tone {
        Some("good") => Tone::Good,
        Some("warn") => Tone::Warn,
        Some("bad") => Tone::Bad,
        _ => Tone::Neutral,
    }
}

/// The core step kinds; any other kind is shown as `other`.
pub const STEP_KINDS: &[&str] = &[
    "context",
    "implement",
    "gate",
    "review",
    "publish",
    "ci",
    "ship",
    "recovery",
    "plan",
    "other",
];

/// A step kind, or `other` when it is not a core kind.
pub fn step_kind(kind: &str) -> &str {
    if STEP_KINDS.contains(&kind) {
        kind
    } else {
        "other"
    }
}

/// Whether a document in this state has readable pages.
pub fn doc_readable(state: &str) -> bool {
    matches!(state, "ready" | "live" | "truncated")
}

/// Refuses an href that is not a relative path of plain file names (it
/// could escape the feed root).
pub fn check_href(href: &str) -> Result<(), String> {
    let ok = !href.is_empty()
        && href.len() <= 1024
        && href.split('/').all(|seg| {
            !seg.is_empty()
                && !seg.starts_with('.')
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.~%".contains(&b))
        });
    if ok {
        Ok(())
    } else {
        Err(format!("run feed href {href:?} leaves the feed root"))
    }
}

/// The file name for a subject ref or run id: every byte outside
/// `A-Z a-z 0-9 - _ . ~` as `%XX`, and a leading `.` as `%2E`.
pub fn file_name(id: &str) -> String {
    let mut out = String::new();
    for (i, b) in id.bytes().enumerate() {
        if b.is_ascii_alphanumeric() || (b"-_~".contains(&b)) || (b == b'.' && i > 0) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Where the feed is: a directory, or a base URL (with an optional bearer
/// token file).
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Dir(PathBuf),
    Url {
        base: String,
        token_file: Option<PathBuf>,
    },
}

impl Source {
    /// `--runs URL|DIR`: an `http://` or `https://` value is a URL, anything
    /// else an existing directory.
    pub fn parse(value: &str, token_file: Option<PathBuf>) -> Result<Self, String> {
        let lower = value.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return Ok(Source::Url {
                base: value.trim_end_matches('/').to_string(),
                token_file,
            });
        }
        if token_file.is_some() {
            return Err("--runs-token-file needs --runs URL".into());
        }
        let dir = Path::new(value)
            .canonicalize()
            .map_err(|e| format!("--runs {value}: {e}"))?;
        if !dir.is_dir() {
            return Err(format!("--runs {value} is not a directory"));
        }
        Ok(Source::Dir(dir))
    }

    pub fn describe(&self) -> String {
        match self {
            Source::Dir(dir) => dir.display().to_string(),
            Source::Url { base, .. } => base.clone(),
        }
    }

    /// Reads one feed file by href. Blocking: call it in `spawn_blocking`.
    /// Directories are read here; URL reads are WP5's (see `client`).
    pub fn read(&self, href: &str) -> Result<Vec<u8>, String> {
        check_href(href)?;
        match self {
            Source::Dir(dir) => {
                std::fs::read(dir.join(href)).map_err(|e| format!("run feed {href}: {e}"))
            }
            Source::Url { base, .. } => Err(format!(
                "run feed {base}/{href}: reading over HTTP is not built yet"
            )),
        }
    }

    /// Reads and parses one JSON feed file.
    pub fn json<T: for<'de> Deserialize<'de>>(&self, href: &str) -> Result<T, String> {
        let bytes = self.read(href)?;
        serde_json::from_slice(&bytes).map_err(|e| format!("run feed {href}: {e}"))
    }
}

/// Runs joined every way a page asks: by run id, by subject ref or id (so
/// by a bead's external ref or id), and by attempt id.
#[derive(Clone, Debug, Default)]
pub struct Index {
    pub feed: Feed,
    /// Every listed run, `index.json` then shards.
    pub runs: Vec<RunSummary>,
    by_run: HashMap<String, usize>,
    by_subject: HashMap<String, Vec<usize>>,
    by_attempt: HashMap<String, usize>,
}

impl Index {
    /// Joins listings and subject files. Subject files give each run's
    /// current attempt id; `with_attempts` adds every attempt of a run doc.
    pub fn build(feed: Feed, listings: Vec<IndexFile>, subjects: &[SubjectFile]) -> Self {
        let mut index = Index {
            feed,
            ..Default::default()
        };
        for listing in listings {
            for run in listing.runs {
                if index.by_run.contains_key(&run.run) {
                    continue;
                }
                let i = index.runs.len();
                index.by_run.insert(run.run.clone(), i);
                for key in run.subject.reference.iter().chain(&run.subject.id) {
                    index.by_subject.entry(key.clone()).or_default().push(i);
                }
                index.runs.push(run);
            }
        }
        for file in subjects {
            for entry in &file.runs {
                if let (Some(attempt), Some(&i)) = (&entry.attempt, index.by_run.get(&entry.run)) {
                    index.by_attempt.insert(attempt.clone(), i);
                }
            }
        }
        index
    }

    /// Registers every attempt id of these run docs.
    pub fn with_attempts<'a>(mut self, runs: impl IntoIterator<Item = &'a Run>) -> Self {
        for run in runs {
            if let Some(&i) = self.by_run.get(&run.run) {
                for attempt in &run.attempts {
                    if !attempt.id.is_empty() {
                        self.by_attempt.insert(attempt.id.clone(), i);
                    }
                }
            }
        }
        self
    }

    /// A run by id.
    pub fn run(&self, id: &str) -> Option<&RunSummary> {
        self.by_run.get(id).map(|&i| &self.runs[i])
    }

    /// Runs whose subject ref or id is `key`, newest start first.
    pub fn by_subject(&self, key: &str) -> Vec<&RunSummary> {
        let mut runs: Vec<&RunSummary> = self
            .by_subject
            .get(key)
            .into_iter()
            .flatten()
            .map(|&i| &self.runs[i])
            .collect();
        runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        runs
    }

    /// Runs for a bead: by its external ref, then by its id.
    pub fn for_bead(&self, bead: &Bead) -> Vec<&RunSummary> {
        let mut runs = if bead.external_ref.is_empty() {
            Vec::new()
        } else {
            self.by_subject(&bead.external_ref)
        };
        for run in self.by_subject(&bead.id) {
            if !runs.iter().any(|r| r.run == run.run) {
                runs.push(run);
            }
        }
        runs
    }

    /// The run an attempt id belongs to.
    pub fn by_attempt(&self, attempt: &str) -> Option<&RunSummary> {
        self.by_attempt.get(attempt).map(|&i| &self.runs[i])
    }

    /// For /healthz.
    pub fn status(&self) -> Value {
        serde_json::json!({
            "revision": self.feed.revision,
            "generated_at": self.feed.generated_at,
            "factory": self.feed.factory,
            "runs": self.runs.len(),
            "gaps": self.feed.source_health.gaps,
        })
    }
}

/// Reads `feed.json`, the index, every shard and the subject file of every
/// listed ref. Blocking: call it in `spawn_blocking`.
pub fn load(source: &Source) -> Result<Index, String> {
    let feed: Feed = source.json("feed.json")?;
    if feed.schema != "runfeed.v1" {
        return Err(format!(
            "run feed schema {:?}: only runfeed.v1 is understood",
            feed.schema
        ));
    }
    let mut listings = vec![source.json::<IndexFile>(&feed.index)?];
    for shard in &feed.shards {
        listings.push(source.json(shard)?);
    }
    let mut refs: Vec<&str> = listings
        .iter()
        .flat_map(|l| &l.runs)
        .filter_map(|r| r.subject.reference.as_deref())
        .collect();
    refs.sort();
    refs.dedup();
    let mut subjects = Vec::new();
    for reference in refs {
        subjects
            .push(source.json::<SubjectFile>(&format!("subjects/{}.json", file_name(reference)))?);
    }
    Ok(Index::build(feed.clone(), listings, &subjects))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden() -> Source {
        Source::Dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runfeed/v1"))
    }

    #[test]
    fn the_whole_golden_feed_parses() {
        let source = golden();
        let index = load(&source).unwrap();
        assert_eq!(index.feed.revision, 42);
        assert_eq!(index.runs.len(), 5);
        let mut docs = Vec::new();
        for summary in &index.runs {
            let run: Run = source.json(&summary.href).unwrap();
            assert_eq!(run.run, summary.run);
            assert_eq!(run.digest, summary.digest);
            for doc in &run.documents {
                for page in &doc.pages {
                    let bytes = source.read(&page.href).unwrap();
                    if doc.content_type == "application/x-ndjson" {
                        for line in String::from_utf8(bytes).unwrap().lines() {
                            let turn: Turn = serde_json::from_str(line).unwrap();
                            assert!(!turn.role.is_empty());
                        }
                    }
                }
            }
            docs.push(run);
        }
        for href in index.feed.boards.values() {
            let board: Board = source.json(href).unwrap();
            assert_eq!(board.schema, "runfeed.board.v1");
        }
        let changes = String::from_utf8(source.read("changes.jsonl").unwrap()).unwrap();
        for line in changes.lines() {
            serde_json::from_str::<Change>(line).unwrap();
        }
        let stopped = docs.iter().find(|r| r.state == "stopped").unwrap();
        assert!(!stopped.stop.as_ref().unwrap().code.is_empty());
        let index = index.with_attempts(&docs);
        assert_eq!(index.by_attempt("att-0002-1").unwrap().run, "run-0002");
        assert_eq!(index.by_subject("WORK-0002").len(), 2);
        assert_eq!(index.by_subject("item-7f3a").len(), 2, "by subject id too");
        assert_eq!(index.run("run-0005").unwrap().state, "passed");
        assert_eq!(run_tone("superseded"), Tone::Neutral);
        assert_eq!(run_tone("an-unknown-state"), Tone::Neutral);
        assert_eq!(step_kind("deploy"), "other");
        assert_eq!(step_tone("fail"), Tone::Bad);
        assert_eq!(fact_tone(Some("good")), Tone::Good);
        assert_eq!(fact_tone(Some("purple")), Tone::Neutral);
        assert!(doc_readable("truncated") && !doc_readable("withheld"));
        assert!(golden().describe().ends_with("runfeed/v1"));
    }

    #[test]
    fn hrefs_cannot_escape_and_names_are_percent_encoded() {
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a//b",
            ".hidden",
            "a b",
        ] {
            assert!(check_href(bad).is_err(), "{bad}");
        }
        assert!(check_href("subjects/a%2Fb.json").is_ok());
        assert_eq!(file_name("WORK-0001"), "WORK-0001");
        assert_eq!(file_name("a/b c"), "a%2Fb%20c");
        assert_eq!(file_name(".x.y"), "%2Ex.y");
        assert!(Source::parse("https://feed.example/x/", None).is_ok());
        assert!(Source::parse("/nonexistent-runs-dir", None).is_err());
        assert!(golden().read("../Cargo.toml").is_err());
    }
}
