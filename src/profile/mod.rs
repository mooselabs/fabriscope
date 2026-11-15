//! A profile (`fabriscope.profile.v1`, TOML) tells Fabriscope how a store
//! uses its metadata: which rows are work items and which are receipts,
//! which kinds and states to show, how to group and whom to name, which
//! comments are history events and how identifiers become links.
//!
//! Fabriscope stays generic: without a profile every row is an item and the
//! pages show what Beads itself stores. The full format, with an example, is
//! in docs/profile.md; `fabriscope profile check FILE` validates one.

pub mod cond;
pub mod derive;

pub use cond::Match;
pub use derive::{Derived, RowFacts};

use crate::beads::{Bead, Snapshot};
use cond::one_or_many;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// The schema tag every profile file carries.
pub const SCHEMA: &str = "fabriscope.profile.v1";

/// A loaded profile. `Profile::default()` is the generic one: no rules, every
/// row an item, and generic flow defaults.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub schema: String,
    /// A name shown by `profile check` and on /healthz.
    pub name: String,
    /// Where a bead names its parent, for stores without parent-child links.
    /// `--parent-key` replaces it.
    pub parent: Option<ParentRule>,
    /// Where a bead's documents live, so relative Markdown links resolve.
    pub documents: Option<DocumentsRule>,
    /// Row classes, first match wins; a bead no rule matches is an item.
    pub rows: Vec<RowRule>,
    /// Display kinds, first match wins.
    pub kinds: Vec<KindRule>,
    /// Derived states; a bead can be in several.
    pub states: Vec<StateRule>,
    /// Group-bys, also offered as URL facets under their keys.
    pub groups: Vec<GroupRule>,
    pub people: PeopleRule,
    /// Typed field panels on the bead page.
    pub panels: Vec<PanelRule>,
    /// Which comments are history events, and how to read them.
    pub events: Option<EventsRule>,
    /// How receipt rows name their kind, subject and attempt.
    pub receipts: Vec<ReceiptRule>,
    pub links: LinksRule,
    pub flow: FlowRule,
    /// Labels for opaque fact keys (from a run feed or a panel).
    pub labels: BTreeMap<String, String>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            name: String::new(),
            parent: None,
            documents: None,
            rows: Vec::new(),
            kinds: Vec::new(),
            states: Vec::new(),
            groups: Vec::new(),
            people: PeopleRule::default(),
            panels: Vec::new(),
            events: None,
            receipts: Vec::new(),
            links: LinksRule::default(),
            flow: FlowRule::default(),
            labels: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ParentRule {
    /// A source whose value names the parent by id or external reference.
    pub path: Source,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentsRule {
    /// A source holding the bead's document path; relative links in its
    /// Markdown resolve from there.
    pub base_path: Source,
}

/// What a row is: a work item (the default), a receipt (shown on receipt
/// pages, joined to its subject) or hidden (shown nowhere but its own page).
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum RowClass {
    #[default]
    Item,
    Receipt,
    Hidden,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RowRule {
    pub class: RowClass,
    #[serde(default)]
    pub when: Match,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KindRule {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub when: Match,
}

/// A colour family. Each maps to one CSS class (see `render::tone_class`).
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Good,
    Warn,
    Bad,
    Waiting,
    #[default]
    Neutral,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StateRule {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub tone: Tone,
    #[serde(default)]
    pub when: Match,
    /// Since when the bead has been in this state.
    pub since: Option<Source>,
    /// Read this bead's provenance and history ahead of a visit.
    #[serde(default)]
    pub read_ahead: bool,
    /// A bead in this state is never shown or counted as ready, whatever
    /// `bd ready` says.
    #[serde(default)]
    pub overrides_ready: bool,
    /// Sources shown beside the state (on a banner or a held list).
    #[serde(default, deserialize_with = "one_or_many")]
    pub facts: Vec<Source>,
    /// A sentence with `{source}` holes, filled by `Profile::banner`.
    pub banner: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GroupRule {
    /// The URL facet key and the `/by/{key}` path segment.
    pub key: String,
    pub label: String,
    /// Sources tried in order; the first with a value gives the group value
    /// (a list value puts the bead in several groups).
    #[serde(deserialize_with = "one_or_many")]
    pub from: Vec<Source>,
    /// A value within the group (a lane within a milestone, say).
    pub subgroup: Option<Source>,
    /// Read from the bead the group value names (by id or external ref):
    /// the declared, ordered list of subgroup values.
    pub declared: Option<Source>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PeopleRule {
    /// How many leading `/` segments of a name identify a person (all when
    /// unset).
    pub segments: Option<usize>,
    pub roles: Vec<RoleRule>,
    /// Names with a prefix are classed apart (factory seats, say).
    pub classes: Vec<PersonClassRule>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RoleRule {
    pub key: String,
    pub label: String,
    /// Sources tried in order. `event:<kind>` sources name the actor of the
    /// first history event of that kind, once the bead's thread is read.
    #[serde(deserialize_with = "one_or_many")]
    pub from: Vec<Source>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PersonClassRule {
    pub prefix: String,
    pub class: String,
    pub segments: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PanelRule {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub when: Match,
    pub fields: Vec<FieldRule>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FieldRule {
    pub path: Source,
    pub label: String,
    #[serde(default)]
    pub kind: FieldKind,
    /// A unit shown after a count or duration.
    pub unit: Option<String>,
    /// A `[links.repos]` name, for `code` values that are commits or trees.
    pub repo: Option<String>,
}

/// How a field value is shown; see `render::values::value`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum FieldKind {
    #[default]
    Text,
    List,
    Markdown,
    Duration,
    Money,
    Link,
    IdRef,
    EnumBadge,
    Json,
    Timestamp,
    Code,
    Count,
    Bool,
    Person,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct EventsRule {
    /// A comment is an event when one of its lines starts with this; the
    /// last such line carries the event as compact JSON after the marker.
    pub marker: String,
    /// When set, the JSON must have this key equal to this value.
    pub schema: Option<SchemaCheck>,
    /// JSON keys for the kind, time and actor.
    pub kind: String,
    pub at: String,
    pub actor: String,
    /// Numeric JSON keys that order events with the same time.
    pub order: Vec<String>,
    /// A JSON key holding `{field: [old, new]}`.
    pub changes: Option<String>,
    /// JSON keys whose text becomes notes on the event.
    pub notes: Vec<String>,
    pub kinds: BTreeMap<String, EventKindRule>,
}

impl Default for EventsRule {
    fn default() -> Self {
        Self {
            marker: String::new(),
            schema: None,
            kind: "kind".into(),
            at: "at".into(),
            actor: "actor".into(),
            order: Vec::new(),
            changes: None,
            notes: Vec::new(),
            kinds: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SchemaCheck {
    pub key: String,
    pub equals: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct EventKindRule {
    pub label: String,
    pub tone: Option<Tone>,
    /// The event claims the bead (starts work).
    pub claim: bool,
    /// The event stops work (holds the bead).
    pub stop: bool,
    /// The event created the bead.
    pub created: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReceiptRule {
    #[serde(default)]
    pub when: Match,
    /// A receipt kind such as `gate` or `ship`.
    pub kind: String,
    /// Grouping dimensions shown with the receipt.
    #[serde(default)]
    pub group: BTreeMap<String, Source>,
    pub at: Option<Source>,
    pub actor: Option<Source>,
    /// Sources naming the subject, tried in order, each by id or external
    /// reference (so both joins work).
    #[serde(default, deserialize_with = "one_or_many")]
    pub subject: Vec<Source>,
    /// The attempt the receipt belongs to.
    pub attempt: Option<Source>,
    #[serde(default)]
    pub fields: Vec<FieldRule>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LinksRule {
    /// Repository name → `https://` base.
    pub repos: BTreeMap<String, String>,
    /// Templates with `{repo}` (the base) and `{value}` holes.
    pub commit: Option<String>,
    pub pull_request: Option<String>,
    pub tree: Option<String>,
    /// How identifiers with a prefix become links.
    pub ids: Vec<IdRule>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IdRule {
    pub prefix: String,
    pub target: IdTarget,
    /// For `target = "template"`: an `https://` URL with a `{value}` hole.
    pub template: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IdTarget {
    /// A bead found by id or external reference.
    Bead,
    /// A scroll found by alias.
    ScrollAlias,
    /// An external page from `template`.
    Template,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FlowRule {
    /// When a bead was claimed, tried in order (request-time sources allowed).
    #[serde(deserialize_with = "one_or_many")]
    pub claim_at: Vec<Source>,
    /// Who claimed it.
    #[serde(deserialize_with = "one_or_many")]
    pub claimant: Vec<Source>,
    /// Which items flow pages count (all items when unset).
    pub items: Option<Match>,
    /// Statuses that count as work in progress.
    pub wip_status: Vec<String>,
    pub decisions: DecisionsRule,
}

impl Default for FlowRule {
    fn default() -> Self {
        Self {
            claim_at: vec![Source::Field("started_at".into())],
            claimant: Vec::new(),
            items: None,
            wip_status: vec!["in_progress".into()],
            decisions: DecisionsRule::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DecisionsRule {
    pub when: Match,
    pub since: Option<Source>,
    pub assignee: Option<Source>,
    pub due: Option<Source>,
}

impl Default for DecisionsRule {
    fn default() -> Self {
        Self {
            when: Match {
                issue_type: vec!["decision".into()],
                not: Some(Box::new(Match {
                    status: vec!["closed".into()],
                    ..Default::default()
                })),
                ..Default::default()
            },
            since: Some(Source::Field("created_at".into())),
            assignee: None,
            due: None,
        }
    }
}

/// A person as a profile classes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    /// The name the person is known by (the kept segments).
    pub name: String,
    /// `person`, or the class of the first `[[people.classes]]` prefix the
    /// raw name starts with (a factory seat, say).
    pub class: String,
}

impl Profile {
    /// Reads and validates a profile file.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parses and validates profile text. Errors name the bad key.
    pub fn parse(text: &str) -> Result<Self, String> {
        let profile: Profile = toml::from_str(text).map_err(|e| e.to_string())?;
        if profile.schema != SCHEMA {
            return Err(format!(
                "schema: expected \"{SCHEMA}\", found {:?}",
                profile.schema
            ));
        }
        profile.validate()?;
        Ok(profile)
    }

    /// `--parent-key` wins over `[parent]`.
    pub fn set_parent_key(&mut self, parent_key: Option<&str>) -> Result<(), String> {
        if let Some(key) = parent_key {
            self.parent = Some(ParentRule {
                path: Source::parse(key).map_err(|e| format!("--parent-key: {e}"))?,
            });
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        let fixed = |at: String, s: &Source| -> Result<(), String> {
            if s.is_request_time() {
                Err(format!(
                    "{at}: `{s}` is read from history at request time; it is only allowed in [flow] and [[people.roles]]"
                ))
            } else {
                Ok(())
            }
        };
        let cond = |at: String, m: &Match| -> Result<(), String> {
            m.sources()
                .into_iter()
                .try_for_each(|s| fixed(format!("{at}.when"), s))
        };
        let unique = |section: &str, keys: Vec<&str>| -> Result<(), String> {
            let mut seen = std::collections::HashSet::new();
            for (i, key) in keys.iter().enumerate() {
                if key.is_empty() {
                    return Err(format!("{section}[{i}].key: must not be empty"));
                }
                if !seen.insert(*key) {
                    return Err(format!("{section}[{i}].key: `{key}` is used twice"));
                }
            }
            Ok(())
        };
        if let Some(p) = &self.parent {
            fixed("parent.path".into(), &p.path)?;
        }
        if let Some(d) = &self.documents {
            fixed("documents.base_path".into(), &d.base_path)?;
        }
        for (i, r) in self.rows.iter().enumerate() {
            cond(format!("rows[{i}]"), &r.when)?;
        }
        unique("kinds", self.kinds.iter().map(|k| k.key.as_str()).collect())?;
        for (i, k) in self.kinds.iter().enumerate() {
            cond(format!("kinds[{i}]"), &k.when)?;
        }
        unique(
            "states",
            self.states.iter().map(|k| k.key.as_str()).collect(),
        )?;
        for (i, s) in self.states.iter().enumerate() {
            cond(format!("states[{i}]"), &s.when)?;
            if let Some(since) = &s.since {
                fixed(format!("states[{i}].since"), since)?;
            }
            for f in &s.facts {
                fixed(format!("states[{i}].facts"), f)?;
            }
        }
        unique(
            "groups",
            self.groups.iter().map(|k| k.key.as_str()).collect(),
        )?;
        for (i, g) in self.groups.iter().enumerate() {
            if crate::query::RESERVED.contains(&g.key.as_str()) {
                return Err(format!(
                    "groups[{i}].key: `{}` is a reserved query key ({})",
                    g.key,
                    crate::query::RESERVED.join(", ")
                ));
            }
            if !g.key.starts_with(|c: char| c.is_ascii_lowercase())
                || !g
                    .key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
                return Err(format!(
                    "groups[{i}].key: `{}` must be lower-case letters, digits, `_` or `-`",
                    g.key
                ));
            }
            if g.from.is_empty() {
                return Err(format!("groups[{i}].from: needs at least one source"));
            }
            for s in g.from.iter().chain(&g.subgroup).chain(&g.declared) {
                fixed(format!("groups[{i}].from"), s)?;
            }
        }
        unique(
            "people.roles",
            self.people.roles.iter().map(|k| k.key.as_str()).collect(),
        )?;
        unique(
            "panels",
            self.panels.iter().map(|k| k.key.as_str()).collect(),
        )?;
        for (i, p) in self.panels.iter().enumerate() {
            cond(format!("panels[{i}]"), &p.when)?;
            for (j, f) in p.fields.iter().enumerate() {
                fixed(format!("panels[{i}].fields[{j}].path"), &f.path)?;
                if let Some(repo) = &f.repo {
                    if !self.links.repos.contains_key(repo) {
                        return Err(format!(
                            "panels[{i}].fields[{j}].repo: `{repo}` is not in [links.repos]"
                        ));
                    }
                }
            }
        }
        if let Some(e) = &self.events {
            if e.marker.trim().is_empty() {
                return Err("events.marker: must not be empty".into());
            }
        }
        for (i, r) in self.receipts.iter().enumerate() {
            cond(format!("receipts[{i}]"), &r.when)?;
            for s in r
                .group
                .values()
                .chain(&r.at)
                .chain(&r.actor)
                .chain(&r.subject)
                .chain(&r.attempt)
                .chain(r.fields.iter().map(|f| &f.path))
            {
                fixed(format!("receipts[{i}]"), s)?;
            }
        }
        for (name, base) in &self.links.repos {
            if !base.starts_with("https://") {
                return Err(format!("links.repos.{name}: must start with https://"));
            }
        }
        for (i, rule) in self.links.ids.iter().enumerate() {
            if rule.target == IdTarget::Template
                && !rule
                    .template
                    .as_deref()
                    .is_some_and(|t| t.starts_with("https://") && t.contains("{value}"))
            {
                return Err(format!(
                    "links.ids[{i}].template: needs an https:// template with a {{value}} hole"
                ));
            }
        }
        if let Some(items) = &self.flow.items {
            cond("flow.items".into(), items)?;
        }
        cond("flow.decisions".into(), &self.flow.decisions.when)?;
        Ok(())
    }

    /// The profile as resolved, defaults included, for `profile check`.
    pub fn describe(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn kind(&self, key: &str) -> Option<&KindRule> {
        self.kinds.iter().find(|k| k.key == key)
    }

    pub fn state(&self, key: &str) -> Option<&StateRule> {
        self.states.iter().find(|s| s.key == key)
    }

    pub fn group(&self, key: &str) -> Option<&GroupRule> {
        self.groups.iter().find(|g| g.key == key)
    }

    pub fn role(&self, key: &str) -> Option<&RoleRule> {
        self.people.roles.iter().find(|r| r.key == key)
    }

    /// Whether any state holds beads back: one that overrides ready or has
    /// the bad tone. The Held tab shows only then.
    pub fn has_held(&self) -> bool {
        self.states
            .iter()
            .any(|s| s.overrides_ready || s.tone == Tone::Bad)
    }

    /// The label for an opaque fact key, from `[labels]`.
    pub fn fact_label(&self, key: &str) -> Option<&str> {
        self.labels.get(key).map(String::as_str)
    }

    /// The label and tone for a history event kind; unknown kinds are
    /// labelled with their own name and neutral.
    pub fn event_kind(&self, kind: &str) -> (String, Tone) {
        match self.events.as_ref().and_then(|e| e.kinds.get(kind)) {
            Some(rule) if !rule.label.is_empty() => {
                (rule.label.clone(), rule.tone.unwrap_or_default())
            }
            Some(rule) => (kind.replace('_', " "), rule.tone.unwrap_or_default()),
            None => (kind.replace('_', " "), Tone::Neutral),
        }
    }

    /// A raw name (an assignee, comment author or event actor) as a person:
    /// the class of the first matching `[[people.classes]]` prefix, and the
    /// name cut to that class's (or `[people]`'s) segment count.
    pub fn person(&self, raw: &str) -> Person {
        let class = self
            .people
            .classes
            .iter()
            .find(|c| raw.starts_with(c.prefix.as_str()));
        let segments = class.and_then(|c| c.segments).or(self.people.segments);
        let name = match segments {
            Some(n) if n > 0 => raw.split('/').take(n).collect::<Vec<_>>().join("/"),
            _ => raw.to_string(),
        };
        Person {
            name,
            class: class.map_or_else(|| "person".to_string(), |c| c.class.clone()),
        }
    }

    /// A state's banner with each `{source}` hole filled from the bead (an
    /// empty value leaves `—`). The result is plain text: escape it.
    pub fn banner(&self, state: &str, bead: &Bead) -> Option<String> {
        let template = self.state(state)?.banner.as_deref()?;
        let mut out = String::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let Some(close) = rest[open..].find('}') else {
                out.push_str(&rest[open..]);
                return Some(out);
            };
            let hole = &rest[open + 1..open + close];
            let value = Source::parse(hole)
                .ok()
                .and_then(|s| s.text(bead))
                .unwrap_or_else(|| "—".into());
            out.push_str(&value);
            rest = &rest[open + close + 1..];
        }
        out.push_str(rest);
        Some(out)
    }
}

/// Where a value comes from: `meta:a.b.c` (the default without a prefix),
/// `field:<native column>`, `label:<prefix>`, `seat:<n>`, `ref:<path>`, and
/// the request-time `event:<kind>` and `provenance:<kind>[.<key>]`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(try_from = "String")]
pub enum Source {
    /// A metadata path. Lookup tries nested keys first, then the remaining
    /// path as one literal dotted key at each level.
    Meta(Vec<String>),
    /// A native column, one of `FIELDS`.
    Field(String),
    /// The rest of the first label with this prefix.
    Label(String),
    /// The n-th `/` segment of the assignee, 1-based.
    Seat(usize),
    /// A metadata value naming another bead by id or external reference.
    Ref(Vec<String>),
    /// The actor of the first history event of this kind (request time).
    Event(String),
    /// The first provenance event of this kind, or one payload key of it
    /// (request time).
    Provenance { kind: String, key: Option<String> },
}

/// The native columns a `field:` source may name.
pub const FIELDS: &[&str] = &[
    "id",
    "title",
    "status",
    "issue_type",
    "priority",
    "assignee",
    "external_ref",
    "created_at",
    "updated_at",
    "closed_at",
    "close_reason",
    "started_at",
    "created_by",
    "owner",
    "estimated_minutes",
    "due_at",
    "defer_until",
    "lease_expires_at",
    "heartbeat_at",
    "is_blocked",
    "comment_count",
];

impl TryFrom<String> for Source {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Source::parse(&value)
    }
}

impl Serialize for Source {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

fn path(text: &str) -> Result<Vec<String>, String> {
    let parts: Vec<String> = text.split('.').map(String::from).collect();
    if text.is_empty() || parts.iter().any(String::is_empty) {
        return Err(format!("`{text}` is not a dotted path"));
    }
    Ok(parts)
}

impl Source {
    /// Parses one source; the error says what was wrong.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let Some((prefix, rest)) = text.split_once(':') else {
            return path(text).map(Source::Meta);
        };
        match prefix {
            "meta" => path(rest).map(Source::Meta),
            "field" if FIELDS.contains(&rest) => Ok(Source::Field(rest.into())),
            "field" => Err(format!(
                "`field:{rest}`: unknown field; one of {}",
                FIELDS.join(", ")
            )),
            "label" if !rest.is_empty() => Ok(Source::Label(rest.into())),
            "seat" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Ok(Source::Seat(n)),
                _ => Err(format!("`seat:{rest}`: needs a segment number from 1")),
            },
            "ref" => path(rest).map(Source::Ref),
            "event" if !rest.is_empty() => Ok(Source::Event(rest.into())),
            "provenance" if !rest.is_empty() => {
                let (kind, key) = match rest.split_once('.') {
                    Some((k, v)) => (k.to_string(), Some(v.to_string())),
                    None => (rest.to_string(), None),
                };
                Ok(Source::Provenance { kind, key })
            }
            _ => Err(format!(
                "`{text}`: unknown source; use meta:, field:, label:, seat:, ref:, event: or provenance:"
            )),
        }
    }

    /// Read from history or provenance when a page is drawn, so never part
    /// of a snapshot's derivation.
    pub fn is_request_time(&self) -> bool {
        matches!(self, Source::Event(_) | Source::Provenance { .. })
    }

    /// The raw value for a bead, or `None` when it is missing, null, an
    /// empty string or an empty list. Request-time sources give `None`.
    pub fn value(&self, bead: &Bead) -> Option<Value> {
        let value = match self {
            Source::Meta(parts) | Source::Ref(parts) => meta(&bead.metadata, parts)?.clone(),
            Source::Field(name) => field(bead, name)?,
            Source::Label(prefix) => Value::String(
                bead.labels
                    .iter()
                    .find_map(|l| l.strip_prefix(prefix.as_str()))?
                    .to_string(),
            ),
            Source::Seat(n) => Value::String(bead.assignee.split('/').nth(n - 1)?.to_string()),
            Source::Event(_) | Source::Provenance { .. } => return None,
        };
        match &value {
            Value::Null => None,
            Value::String(s) if s.is_empty() => None,
            Value::Array(a) if a.is_empty() => None,
            _ => Some(value),
        }
    }

    /// The value as one line of text: strings as they are, numbers and
    /// booleans as written, a list joined with `, `.
    pub fn text(&self, bead: &Bead) -> Option<String> {
        let texts = self.texts(bead);
        (!texts.is_empty()).then(|| texts.join(", "))
    }

    /// Each value as text: a list gives one text per element.
    pub fn texts(&self, bead: &Bead) -> Vec<String> {
        match self.value(bead) {
            None => Vec::new(),
            Some(Value::Array(items)) => items.iter().filter_map(text_of).collect(),
            Some(other) => text_of(&other).into_iter().collect(),
        }
    }

    /// The bead this source names, by id or external reference.
    pub fn bead<'a>(&self, bead: &Bead, snapshot: &'a Snapshot) -> Option<&'a Bead> {
        snapshot.find(&self.text(bead)?)
    }
}

/// Text for a scalar JSON value; objects as compact JSON.
pub fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(text_of).collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
        other => Some(other.to_string()),
    }
}

fn meta<'a>(value: &'a Value, parts: &[String]) -> Option<&'a Value> {
    if parts.is_empty() {
        return Some(value);
    }
    let object = value.as_object()?;
    // Nested first (k = 1), then ever longer literal dotted keys.
    for k in 1..=parts.len() {
        if let Some(found) = object
            .get(&parts[..k].join("."))
            .and_then(|next| meta(next, &parts[k..]))
        {
            return Some(found);
        }
    }
    None
}

fn field(bead: &Bead, name: &str) -> Option<Value> {
    let text = |s: &str| Value::String(s.to_string());
    let opt = |s: &Option<String>| s.as_deref().map(text);
    Some(match name {
        "id" => text(&bead.id),
        "title" => text(&bead.title),
        "status" => text(&bead.status),
        "issue_type" => text(&bead.issue_type),
        "priority" => Value::from(bead.priority),
        "assignee" => text(&bead.assignee),
        "external_ref" => text(&bead.external_ref),
        "created_at" => opt(&bead.created_at)?,
        "updated_at" => opt(&bead.updated_at)?,
        "closed_at" => opt(&bead.closed_at)?,
        "close_reason" => text(&bead.close_reason),
        "started_at" => opt(&bead.started_at)?,
        "created_by" => text(&bead.created_by),
        "owner" => text(&bead.owner),
        "estimated_minutes" => Value::from(bead.estimated_minutes?),
        "due_at" => opt(&bead.due_at)?,
        "defer_until" => opt(&bead.defer_until)?,
        "lease_expires_at" => opt(&bead.lease_expires_at)?,
        "heartbeat_at" => opt(&bead.heartbeat_at)?,
        "is_blocked" => Value::from(bead.is_blocked?),
        "comment_count" => Value::from(bead.comment_count?),
        _ => return None,
    })
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Source::Meta(parts) => write!(f, "{}", parts.join(".")),
            Source::Field(name) => write!(f, "field:{name}"),
            Source::Label(prefix) => write!(f, "label:{prefix}"),
            Source::Seat(n) => write!(f, "seat:{n}"),
            Source::Ref(parts) => write!(f, "ref:{}", parts.join(".")),
            Source::Event(kind) => write!(f, "event:{kind}"),
            Source::Provenance { kind, key: None } => write!(f, "provenance:{kind}"),
            Source::Provenance { kind, key: Some(k) } => write!(f, "provenance:{kind}.{k}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bead() -> Bead {
        Bead {
            id: "wk-1".into(),
            assignee: "factory/hostA/rigA".into(),
            labels: vec!["docs".into(), "area:billing".into(), "area:other".into()],
            started_at: Some("2026-11-15T10:00:00Z".into()),
            metadata: json!({
                "work": {"fields": {"milestone": "v1", "gate": false}},
                "work.flat": {"key": "dotted"},
                "a": {"b.c": {"d": 4}},
                "empty": "",
                "list": [],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn sources_parse_and_look_up_with_the_dotted_key_fallback() {
        let b = bead();
        let text = |s: &str| Source::parse(s).unwrap().text(&b);
        assert_eq!(text("work.fields.milestone").as_deref(), Some("v1"));
        assert_eq!(text("meta:work.fields.gate").as_deref(), Some("false"));
        assert_eq!(
            text("work.flat.key").as_deref(),
            Some("dotted"),
            "literal dotted key"
        );
        assert_eq!(text("a.b.c.d").as_deref(), Some("4"), "dotted key mid-path");
        assert_eq!(text("empty"), None);
        assert_eq!(text("list"), None);
        assert_eq!(
            text("label:area:").as_deref(),
            Some("billing"),
            "first label"
        );
        assert_eq!(text("seat:3").as_deref(), Some("rigA"));
        assert_eq!(text("seat:9"), None);
        assert_eq!(
            text("field:started_at").as_deref(),
            Some("2026-11-15T10:00:00Z")
        );
        assert_eq!(text("field:due_at"), None);
        assert!(
            Source::parse("field:nope")
                .unwrap_err()
                .contains("unknown field")
        );
        assert!(Source::parse("seat:0").is_err());
        assert!(Source::parse("bogus:x").is_err());
        assert!(Source::parse("a..b").is_err());
        assert!(Source::parse("event:claim").unwrap().is_request_time());
        assert_eq!(
            Source::parse("provenance:used.dollars").unwrap(),
            Source::Provenance {
                kind: "used".into(),
                key: Some("dollars".into())
            }
        );
        assert_eq!(Source::parse("meta:x.y").unwrap().to_string(), "x.y");
        assert_eq!(
            text("event:claim"),
            None,
            "request-time sources need a thread"
        );
    }

    #[test]
    fn ref_sources_find_the_bead_they_name() {
        let mut epic = Bead {
            id: "wk-e".into(),
            external_ref: "EPIC-1".into(),
            ..Default::default()
        };
        epic.issue_type = "epic".into();
        let mut child = bead();
        child.metadata = json!({"work": {"parent": "EPIC-1"}});
        let s = Snapshot::build(
            vec![epic, child.clone()],
            vec![],
            None,
            chrono::Utc::now(),
            &Default::default(),
        );
        let named = Source::parse("ref:work.parent").unwrap().bead(&child, &s);
        assert_eq!(named.unwrap().id, "wk-e");
    }

    #[test]
    fn profile_errors_name_the_bad_key() {
        let base = "schema = \"fabriscope.profile.v1\"\n";
        for (text, says) in [
            ("schema = \"other\"", "schema"),
            ("schema = \"fabriscope.profile.v1\"\nbogus = 1", "bogus"),
            (
                &format!("{base}[[states]]\nkey = \"held\"\nlabel = \"Held\"\ntone = \"purple\""),
                "purple",
            ),
            (
                &format!(
                    "{base}[[states]]\nkey = \"held\"\nlabel = \"Held\"\nsince = \"field:nope\""
                ),
                "field:nope",
            ),
            (
                &format!(
                    "{base}[[states]]\nkey = \"held\"\nlabel = \"Held\"\nsince = \"event:claim\""
                ),
                "states[0].since",
            ),
            (
                &format!("{base}[[rows]]\nclass = \"item\"\nwhen = {{ has = \"event:claim\" }}"),
                "rows[0].when",
            ),
            (
                &format!("{base}[[groups]]\nkey = \"q\"\nlabel = \"Q\"\nfrom = \"x\""),
                "reserved",
            ),
            (
                &format!("{base}[[groups]]\nkey = \"state\"\nlabel = \"S\"\nfrom = \"x\""),
                "groups[0].key",
            ),
            (
                &format!("{base}[links]\nrepos = {{ a = \"http://x\" }}"),
                "links.repos.a",
            ),
            (
                &format!(
                    "{base}[[kinds]]\nkey = \"a\"\nlabel = \"A\"\n[[kinds]]\nkey = \"a\"\nlabel = \"B\""
                ),
                "kinds[1].key",
            ),
        ] {
            let error = Profile::parse(text).unwrap_err();
            assert!(error.contains(says), "{says}: {error}");
        }
        assert!(Profile::parse(base).is_ok());
    }

    #[test]
    fn people_are_classed_and_cut_to_their_segments() {
        let p = Profile::parse(
            "schema = \"fabriscope.profile.v1\"\n[people]\nsegments = 1\n[[people.classes]]\nprefix = \"factory/\"\nclass = \"seat\"\nsegments = 3\n",
        )
        .unwrap();
        assert_eq!(
            p.person("factory/hostA/rigA/extra"),
            Person {
                name: "factory/hostA/rigA".into(),
                class: "seat".into()
            }
        );
        assert_eq!(p.person("alice/laptop").name, "alice");
        assert_eq!(p.person("alice").class, "person");
    }

    #[test]
    fn the_fixture_profile_checks_and_fills_its_banner() {
        let p = Profile::parse(include_str!("../../tests/fixtures/work/profile.toml")).unwrap();
        assert!(p.has_held());
        assert_eq!(p.kind("gate").unwrap().label, "Gate");
        assert_eq!(p.fact_label("acme.gate_seconds"), Some("Gate time"));
        assert_eq!(p.event_kind("suspended"), ("Held".to_string(), Tone::Bad));
        assert_eq!(
            p.event_kind("re_staged"),
            ("re staged".to_string(), Tone::Neutral)
        );
        assert!(p.describe().contains("\"overrides_ready\": true"));
        let view = crate::testing::work_view();
        let s = &view.beads.as_ref().unwrap().data;
        let held = s.find("WORK-0003").unwrap();
        assert_eq!(
            p.banner("held", held).unwrap(),
            "Held since 2026-11-15T09:15:00Z: scope-exceeded (scope)"
        );
        let mut generic = Profile::default();
        assert!(generic.set_parent_key(Some("a..b")).is_err());
        generic.set_parent_key(Some("work.parent")).unwrap();
        assert_eq!(
            generic.parent.unwrap().path,
            Source::Meta(vec!["work".into(), "parent".into()])
        );
    }
}
