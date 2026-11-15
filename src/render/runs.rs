// Owned by WP5 — Runs. Frozen signatures:
// `runs(&View, &Facets) -> String`, `run(&View, &str) -> Option<String>`,
// `doc(&View, &str, &str, usize) -> Option<String>`.
//! `/runs`, `/run/{id}` and `/run/{id}/doc/{doc}?page=N`: the run feed's
//! runs, one run's attempts and steps, and a document page (transcript
//! turns always escaped). Run docs and pages come from `View::run_cache`
//! and are never waited for: a miss shows "Fetching…" and the page fills
//! in when the fetch moves the view revision.

use super::fmt::{count, hours, money, percent, seconds, short, tokens, when, words};
use super::layout::{not_found, page};
use super::values::external;
use super::{
    PAGE_SIZE, bead_link, escape, heading, listed_group_link, run_link, select, tone_badge,
    tone_class, url,
};
use crate::model::View;
use crate::query::{Facets, href};
use crate::runs::cache::Fetch;
use crate::runs::{
    Cost, DocRef, Fact, Link, Run, RunSummary, Step, Stop, Subject, Turn, doc_readable, fact_tone,
    run_tone, step_kind, step_tone,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// The most of one document page rendered.
const RENDER_CAP: usize = 2 << 20;

/// The run states the summary tiles count, in order.
const TILES: [&str; 5] = ["running", "stopped", "passed", "failed", "superseded"];

/// A badge for a run or attempt state.
pub(crate) fn state_badge(state: &str) -> String {
    let text = if state.is_empty() { "unknown" } else { state };
    tone_badge(&words(text), run_tone(state))
}

fn parse(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Seconds from `start` to `end`, or to `now` while it runs.
fn span(start: Option<&str>, end: Option<&str>, now: DateTime<Utc>) -> Option<f64> {
    let start = parse(start?)?;
    let end = match end {
        Some(end) => parse(end)?,
        None => now,
    };
    Some((end - start).num_seconds() as f64)
}

/// A byte count: `220 B`, `1.6 KiB`, `3.2 MiB`.
pub(crate) fn size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KiB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MiB", bytes as f64 / 1_048_576.0),
    }
}

/// The `<time>` for an optional timestamp, else a dash.
fn at(value: Option<&str>) -> String {
    value.map(when).unwrap_or_else(|| "—".into())
}

/// A subject: its title (or ref, or id), linked to the bead it names when
/// the Beads store has one.
fn subject_html(view: &View, subject: &Subject) -> String {
    let key = subject.reference.as_deref().or(subject.id.as_deref());
    let text = subject
        .title
        .as_deref()
        .or(key)
        .unwrap_or("(no subject)")
        .to_string();
    let found = [subject.reference.as_deref(), subject.id.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|k| view.snapshot().and_then(|s| s.find(k)));
    let title = match found {
        Some(bead) => bead_link(&bead.id, &text),
        None => escape(&text),
    };
    match key.filter(|k| *k != text) {
        Some(key) => format!("{title} <small class=\"muted\">{}</small>", escape(key)),
        None => title,
    }
}

/// Group values, each linked to its `/by/…` page when the profile groups
/// by that key.
fn group_chips(view: &View, group: &BTreeMap<String, String>) -> String {
    group
        .iter()
        .map(|(key, value)| {
            let text = format!("{}: {value}", words(key));
            format!(
                "<span class=\"chip\">{}</span>",
                listed_group_link(view, key, value, &text)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `[links.repos]` name for a run's repository group, when the profile
/// has one: the value itself, or its last path segment.
fn repo_key(view: &View, group: &BTreeMap<String, String>) -> Option<String> {
    let repos = &view.profile.links.repos;
    ["repository", "repo"]
        .iter()
        .filter_map(|k| group.get(*k))
        .flat_map(|v| [v.as_str(), v.rsplit('/').next().unwrap_or(v)])
        .find(|v| repos.contains_key(*v))
        .map(String::from)
}

/// `html` linked through a profile template (`{repo}`, `{value}`), when
/// the repo and template are known and the result is `https://`.
fn templated(
    view: &View,
    repo: Option<&str>,
    template: Option<&String>,
    value: &str,
    html: String,
) -> String {
    let base = repo.and_then(|r| view.profile.links.repos.get(r));
    if let (Some(base), Some(template)) = (base, template) {
        let target = template
            .replace("{repo}", base)
            .replace("{value}", &url(value));
        if target.starts_with("https://") {
            return format!(
                "<a href=\"{}\" rel=\"noreferrer\">{html}</a>",
                escape(&target)
            );
        }
    }
    html
}

/// A commit or tree id as short code with the whole id on hover.
fn sha_code(value: &str) -> String {
    format!(
        "<code title=\"{}\">{}</code>",
        escape(value),
        escape(short(value))
    )
}

/// A run-feed fact's label: its own, else the profile's, else the key.
pub(crate) fn fact_label(view: &View, fact: &Fact) -> String {
    fact.label
        .clone()
        .or_else(|| view.profile.fact_label(&fact.key).map(String::from))
        .unwrap_or_else(|| fact.key.clone())
}

/// A run-feed fact's value as HTML, by its declared type (unlinked; a run
/// page also links it by its `ref`). A `percent` is a percentage (`42.5`
/// is 42.5%); a value that does not fit its type is shown as text.
pub(crate) fn fact_text(fact: &Fact) -> String {
    let Some(text) = crate::profile::text_of(&fact.value) else {
        return "—".into();
    };
    let number = fact.value.as_f64().or_else(|| text.parse().ok());
    let kind = fact.kind.as_deref().unwrap_or("text");
    match (kind, number) {
        ("seconds", Some(n)) => escape(&seconds(n)),
        ("hours", Some(n)) => escape(&hours(n)),
        ("usd", Some(n)) => escape(&money(n)),
        ("tokens", Some(n)) if n >= 0.0 => escape(&tokens(n as u64)),
        ("count", Some(n)) if n >= 0.0 && n.fract() == 0.0 => count(n as u64),
        ("percent", Some(n)) => escape(&percent(n / 100.0)),
        ("sha" | "tree", _) => sha_code(&text),
        ("bool", _) => match text.as_str() {
            "true" => "yes".into(),
            "false" => "no".into(),
            other => escape(other),
        },
        ("timestamp", _) => when(&text),
        ("id", _) => format!("<code>{}</code>", escape(&text)),
        _ => escape(&text),
    }
}

/// What a page needs to link a run's identifiers.
struct Ctx<'a> {
    view: &'a View,
    run: Option<&'a Run>,
    repo: Option<String>,
}

impl Ctx<'_> {
    fn commit(&self, value: &str) -> String {
        templated(
            self.view,
            self.repo.as_deref(),
            self.view.profile.links.commit.as_ref(),
            value,
            sha_code(value),
        )
    }

    fn tree(&self, value: &str) -> String {
        templated(
            self.view,
            self.repo.as_deref(),
            self.view.profile.links.tree.as_ref(),
            value,
            sha_code(value),
        )
    }

    /// A link to an attempt: its section on this run's page, else the run
    /// the index says it belongs to.
    fn attempt(&self, id: &str, html: String) -> String {
        if let Some((run, attempt)) = self
            .run
            .and_then(|r| r.attempts.iter().find(|a| a.id == id).map(|a| (r, a)))
        {
            return format!(
                "<a href=\"/run/{}#attempt-{}\">{html}</a>",
                url(&run.run),
                attempt.number
            );
        }
        match self.view.run_for_attempt(id) {
            Some(summary) => format!("<a href=\"/run/{}\">{html}</a>", url(&summary.run)),
            None => html,
        }
    }

    fn item(&self, id: &str, html: String) -> String {
        match self.view.snapshot().and_then(|s| s.find(id)) {
            Some(bead) => format!("<a href=\"/bead/{}\">{html}</a>", url(&bead.id)),
            None => html,
        }
    }

    fn run_ref(&self, id: &str, html: String) -> String {
        match self.view.run_index().and_then(|i| i.run(id)) {
            Some(_) => format!("<a href=\"/run/{}\">{html}</a>", url(id)),
            None => html,
        }
    }

    /// A fact's value, formatted by its type and linked by its `ref`.
    fn fact_value(&self, fact: &Fact) -> String {
        let html = fact_text(fact);
        let html = match &fact.reference {
            Some(r) => match r.kind.as_str() {
                "item" => self.item(&r.id, html),
                "run" => self.run_ref(&r.id, html),
                "attempt" => self.attempt(&r.id, html),
                "commit" if html.starts_with("<code") => self.commit(&r.id),
                "commit" => templated(
                    self.view,
                    self.repo.as_deref(),
                    self.view.profile.links.commit.as_ref(),
                    &r.id,
                    html,
                ),
                "tree" if html.starts_with("<code") => self.tree(&r.id),
                "tree" => templated(
                    self.view,
                    self.repo.as_deref(),
                    self.view.profile.links.tree.as_ref(),
                    &r.id,
                    html,
                ),
                _ => html,
            },
            None => html,
        };
        match fact_tone(fact.tone.as_deref()) {
            crate::profile::Tone::Neutral => html,
            tone => format!("<span class=\"tone-{}\">{html}</span>", tone_class(tone)),
        }
    }

    /// Facts as a `dl.facts`; empty when there are none.
    fn facts(&self, facts: &[Fact]) -> String {
        if facts.is_empty() {
            return String::new();
        }
        let mut out = "<dl class=\"facts\">".to_string();
        for fact in facts {
            write!(
                out,
                "<dt>{}</dt><dd>{}</dd>",
                escape(&fact_label(self.view, fact)),
                self.fact_value(fact)
            )
            .unwrap();
        }
        out.push_str("</dl>");
        out
    }

    /// A link: `https://` hrefs are clickable; any other href is shown as
    /// code; a ref alone links inside the server or through a template.
    fn link(&self, link: &Link) -> String {
        let text = link
            .text
            .clone()
            .or_else(|| link.reference.clone())
            .or_else(|| link.href.clone())
            .unwrap_or_else(|| words(&link.rel));
        let rel = format!(
            "<small class=\"muted\">{}</small> ",
            escape(&words(&link.rel))
        );
        if let Some(target) = &link.href {
            if target.to_ascii_lowercase().starts_with("https://") {
                return format!("{rel}{}", external(target, &text));
            }
            return format!(
                "{rel}{} <code class=\"unlinked\">{}</code>",
                escape(&text),
                escape(target)
            );
        }
        let Some(reference) = &link.reference else {
            return format!("{rel}{}", escape(&text));
        };
        let html = escape(&text);
        let linked = match link.rel.as_str() {
            "item" => self.item(reference, html),
            "attempt" => self.attempt(reference, html),
            "commit" => self.commit(reference),
            "tree" => self.tree(reference),
            "pull_request" => templated(
                self.view,
                self.repo.as_deref(),
                self.view.profile.links.pull_request.as_ref(),
                reference,
                html,
            ),
            _ => html,
        };
        format!("{rel}{linked}")
    }

    fn links(&self, links: &[Link]) -> String {
        if links.is_empty() {
            return String::new();
        }
        let items: String = links
            .iter()
            .map(|l| format!("<li>{}</li>", self.link(l)))
            .collect();
        format!("<ul class=\"run-links\">{items}</ul>")
    }
}

/// The text of a run state filter: empty for every state.
fn state_matches(facets: &Facets, run: &RunSummary) -> bool {
    facets.state.is_empty() || facets.state == "all" || run.state == facets.state
}

/// Whether a run passes every filter but the state one. Group filters
/// apply only for keys some run carries, so a filter carried from another
/// tab (a milestone, say) does not empty the list.
fn passes(facets: &Facets, keys: &BTreeSet<&str>, run: &RunSummary, now: DateTime<Utc>) -> bool {
    let needle = facets.q.to_lowercase();
    let text = needle.is_empty()
        || [
            Some(run.run.as_str()),
            run.subject.reference.as_deref(),
            run.subject.id.as_deref(),
            run.subject.title.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|h| h.to_lowercase().contains(&needle));
    let groups = facets
        .groups
        .iter()
        .filter(|(k, _)| keys.contains(k.as_str()))
        .all(|(k, v)| run.group.get(k) == Some(v));
    let range = facets.range();
    text && groups && range.contains(run.started_at.as_deref().unwrap_or(""), now)
}

fn page_link(path: &str, facets: &Facets, page: usize) -> String {
    let base = href(path, facets, &[]);
    let joiner = if base.contains('?') { '&' } else { '?' };
    format!("{base}{joiner}page={page}")
}

/// `/runs`.
pub(crate) fn runs(view: &View, facets: &Facets) -> String {
    let Some(index) = view.run_index() else {
        return not_found(view);
    };
    let now = Utc::now();
    let keys: BTreeSet<&str> = index
        .runs
        .iter()
        .flat_map(|r| r.group.keys().map(String::as_str))
        .collect();
    let base: Vec<&RunSummary> = index
        .runs
        .iter()
        .filter(|r| passes(facets, &keys, r, now))
        .collect();
    let mut listed: Vec<&RunSummary> = base
        .iter()
        .copied()
        .filter(|r| state_matches(facets, r))
        .collect();
    listed.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.run.cmp(&b.run))
    });
    let mut body = heading(
        "Factory",
        "Runs",
        "Every run the feed lists, newest first: what it worked on, where it is, what it cost and why it stopped.",
    );
    body.push_str("<div class=\"metrics run-tiles\">");
    for state in TILES {
        let n = base.iter().filter(|r| r.state == state).count();
        write!(
            body,
            "<a class=\"metric tile-{}\" href=\"{}\"><span>{}</span><strong>{n}</strong><small>runs {}</small></a>",
            tone_class(run_tone(state)),
            escape(&href("/runs", facets, &[("state", state)])),
            escape(&capital(state)),
            escape(state)
        )
        .unwrap();
    }
    body.push_str("</div>");
    body.push_str(&runs_form(facets, index, &keys));
    let costed: Vec<f64> = listed.iter().filter_map(|r| r.cost_usd).collect();
    let missing = listed.len() - costed.len();
    // No run with a cost is no total at all, never $0.
    let total = (!costed.is_empty()).then(|| costed.iter().sum::<f64>());
    write!(
        body,
        "<p class=\"run-cost\">Total cost of the {} matching runs: <strong>{}</strong>{}</p>",
        listed.len(),
        super::maybe(total, money),
        match missing {
            0 => String::new(),
            _ if total.is_none() => String::new(),
            1 => " · 1 run has no cost recorded, so this is a lower bound".into(),
            n => format!(" · {n} runs have no cost recorded, so this is a lower bound"),
        }
    )
    .unwrap();
    let (current, pages) = super::page_of(listed.len(), PAGE_SIZE, facets.page());
    if listed.is_empty() {
        body.push_str("<p class=\"empty\">No runs match this view.</p>");
    } else {
        body.push_str("<div class=\"table-wrap\"><table class=\"data runs\"><thead><tr><th>Run</th><th>State</th><th>Subject</th><th>Groups</th><th>Started</th><th class=\"num\">Duration</th><th class=\"num\">Attempts</th><th class=\"num\">Cost</th><th>Stop</th></tr></thead><tbody>");
        for run in listed.iter().skip(current * PAGE_SIZE).take(PAGE_SIZE) {
            let duration = span(run.started_at.as_deref(), run.ended_at.as_deref(), now)
                .map(seconds)
                .unwrap_or_else(|| "—".into());
            write!(
                body,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td></tr>",
                run_link(&run.run, &run.run),
                state_badge(&run.state),
                subject_html(view, &run.subject),
                group_chips(view, &run.group),
                at(run.started_at.as_deref()),
                escape(&duration),
                run.attempts,
                run.cost_usd
                    .map(|c| escape(&money(c)))
                    .unwrap_or_else(|| "<span class=\"muted\">not recorded</span>".into()),
                run.stop_code
                    .as_deref()
                    .map(|c| format!("<code>{}</code>", escape(c)))
                    .unwrap_or_default(),
            )
            .unwrap();
        }
        body.push_str("</tbody></table></div>");
    }
    if pages > 1 {
        write!(
            body,
            "<nav class=\"pagination\" aria-label=\"Results pages\"><span class=\"muted\">Page {} of {pages}</span>",
            current + 1
        )
        .unwrap();
        if current > 0 {
            write!(
                body,
                "<a href=\"{}\">← Newer</a>",
                escape(&page_link("/runs", facets, current - 1))
            )
            .unwrap();
        }
        if current + 1 < pages {
            write!(
                body,
                "<a href=\"{}\">Older →</a>",
                escape(&page_link("/runs", facets, current + 1))
            )
            .unwrap();
        }
        body.push_str("</nav>");
    }
    page(view, "Runs", "/runs", &body)
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn runs_form(facets: &Facets, index: &crate::runs::Index, keys: &BTreeSet<&str>) -> String {
    let mut out = format!(
        "<form class=\"filters\" action=\"/runs\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Run id or subject…\"></label>",
        escape(&facets.q)
    );
    let mut states: BTreeMap<&str, usize> = BTreeMap::new();
    for run in &index.runs {
        *states.entry(run.state.as_str()).or_default() += 1;
    }
    let mut options = vec![(String::new(), "Every state".to_string())];
    options.extend(
        states
            .iter()
            .map(|(s, n)| (s.to_string(), format!("{} ({n})", words(s)))),
    );
    out.push_str(&select("state", "State", &facets.state, &options));
    for key in keys {
        let mut values: BTreeMap<&str, usize> = BTreeMap::new();
        for run in &index.runs {
            if let Some(v) = run.group.get(*key) {
                *values.entry(v.as_str()).or_default() += 1;
            }
        }
        let mut options = vec![(String::new(), format!("Any {}", words(key)))];
        options.extend(
            values
                .iter()
                .map(|(v, n)| (v.to_string(), format!("{v} ({n})"))),
        );
        let current = facets.groups.get(*key).map(String::as_str).unwrap_or("");
        out.push_str(&select(key, &capital(&words(key)), current, &options));
    }
    write!(
        out,
        "<label>From<input type=\"date\" name=\"from\" value=\"{}\"></label><label>To<input type=\"date\" name=\"to\" value=\"{}\"></label>",
        escape(&facets.from),
        escape(&facets.to)
    )
    .unwrap();
    if !facets.days.is_empty() {
        write!(
            out,
            "<input type=\"hidden\" name=\"days\" value=\"{}\">",
            escape(&facets.days)
        )
        .unwrap();
    }
    out.push_str("<button type=\"submit\">Apply</button></form>");
    out
}

/// The heading and facts every run page starts with, from the index.
fn run_heading(view: &View, summary: &RunSummary) -> String {
    let title = summary
        .subject
        .title
        .clone()
        .unwrap_or_else(|| summary.run.clone());
    let mut out = heading("Run", &title, "");
    write!(
        out,
        "<p class=\"run-meta\">{} <code>{}</code> {}</p>",
        state_badge(&summary.state),
        escape(&summary.run),
        group_chips(view, &summary.group)
    )
    .unwrap();
    out
}

/// `/run/{id}`; `None` for a run the index does not list.
pub(crate) fn run(view: &View, id: &str) -> Option<String> {
    let summary = view.run_index()?.run(id)?;
    let mut body = run_heading(view, summary);
    match view.run_cache.get_run(id) {
        Fetch::Pending => {
            body.push_str(
                "<p class=\"fetching\" role=\"status\">Fetching run… this page fills in when it arrives.</p>",
            );
            body.push_str(&summary_facts(view, summary));
        }
        Fetch::Failed(error) => {
            write!(
                body,
                "<div class=\"banner bad\" role=\"alert\"><strong>This run could not be read.</strong> {}</div>",
                escape(&error)
            )
            .unwrap();
            body.push_str(&summary_facts(view, summary));
        }
        Fetch::Ready(doc) => body.push_str(&run_body(view, summary, &doc)),
    }
    Some(page(view, &summary.run, "/runs", &body))
}

/// What the index says, while the run doc is not here.
fn summary_facts(view: &View, summary: &RunSummary) -> String {
    let now = Utc::now();
    format!(
        "<dl class=\"facts\"><dt>Subject</dt><dd>{}</dd><dt>Started</dt><dd>{}</dd><dt>Ended</dt><dd>{}</dd><dt>Duration</dt><dd>{}</dd><dt>Attempts</dt><dd>{}</dd><dt>Cost</dt><dd>{}</dd></dl>",
        subject_html(view, &summary.subject),
        at(summary.started_at.as_deref()),
        at(summary.ended_at.as_deref()),
        span(
            summary.started_at.as_deref(),
            summary.ended_at.as_deref(),
            now
        )
        .map(seconds)
        .unwrap_or_else(|| "—".into()),
        summary.attempts,
        summary
            .cost_usd
            .map(money)
            .unwrap_or_else(|| "not recorded".into()),
    )
}

fn run_body(view: &View, summary: &RunSummary, run: &Run) -> String {
    let now = Utc::now();
    let ctx = Ctx {
        view,
        run: Some(run),
        repo: repo_key(view, &run.group),
    };
    let mut out = String::new();
    if let Some(stop) = &run.stop {
        out.push_str(&stop_block(stop, "This run stopped"));
    }
    out.push_str("<section class=\"run-head\"><dl class=\"facts\">");
    write!(
        out,
        "<dt>Subject</dt><dd>{}</dd><dt>Started</dt><dd>{}</dd><dt>Ended</dt><dd>{}</dd><dt>Updated</dt><dd>{}</dd><dt>Duration</dt><dd>{}</dd>",
        subject_html(view, &run.subject),
        at(run.started_at.as_deref()),
        at(run.ended_at.as_deref()),
        if run.updated_at.is_empty() {
            "—".into()
        } else {
            when(&run.updated_at)
        },
        span(run.started_at.as_deref(), run.ended_at.as_deref(), now)
            .map(seconds)
            .unwrap_or_else(|| "—".into()),
    )
    .unwrap();
    if let Some(step) = &run.current_step {
        write!(
            out,
            "<dt>Current step</dt><dd><a href=\"#step-{}\">{}</a></dd>",
            escape(&url(step)),
            escape(step)
        )
        .unwrap();
    }
    for (key, value) in &run.refs {
        let html = match key.as_str() {
            "commit" => ctx.commit(value),
            "tree" => ctx.tree(value),
            "attempt" => ctx.attempt(value, format!("<code>{}</code>", escape(value))),
            _ => format!("<code>{}</code>", escape(value)),
        };
        write!(
            out,
            "<dt>{}</dt><dd>{html}</dd>",
            escape(&capital(&words(key)))
        )
        .unwrap();
    }
    out.push_str("</dl>");
    out.push_str(&ctx.links(&run.links));
    out.push_str(&ctx.facts(&run.facts));
    out.push_str("</section>");
    if let Some(outcome) = run.outcome.as_ref().filter(|o| !o.facts.is_empty()) {
        write!(
            out,
            "<section><h2>Outcome</h2>{}</section>",
            ctx.facts(&outcome.facts)
        )
        .unwrap();
    }
    if let Some(costs) = &run.costs {
        out.push_str("<section class=\"run-costs\"><h2>Cost</h2>");
        out.push_str(&cost_block(costs));
        out.push_str(&breakdown(costs, run));
        out.push_str("</section>");
    } else {
        out.push_str("<section class=\"run-costs\"><h2>Cost</h2><p class=\"muted\">No cost recorded for this run.</p></section>");
    }
    if !run.roles.is_empty() {
        out.push_str("<section><h2>Roles</h2><div class=\"table-wrap\"><table class=\"data\"><thead><tr><th>Role</th><th>Model</th><th>Provider</th></tr></thead><tbody>");
        for (role, who) in &run.roles {
            write!(
                out,
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&words(role)),
                escape(who.model.as_deref().unwrap_or("—")),
                escape(who.provider.as_deref().unwrap_or("—"))
            )
            .unwrap();
        }
        out.push_str("</tbody></table></div></section>");
    }
    if let Some(timing) = &run.timing {
        out.push_str("<section><h2>Timing</h2><dl class=\"facts\">");
        if let Some(h) = timing.wall_hours {
            write!(out, "<dt>Wall clock</dt><dd>{}</dd>", escape(&hours(h))).unwrap();
        }
        if let Some(h) = timing.active_hours {
            write!(out, "<dt>Active</dt><dd>{}", escape(&hours(h))).unwrap();
            if let Some(max) = timing.timebox_hours_max.filter(|m| *m > 0.0) {
                write!(
                    out,
                    " <meter min=\"0\" max=\"{max}\" high=\"{max}\" value=\"{}\" title=\"{} of {}\"></meter>",
                    h.min(max),
                    escape(&hours(h)),
                    escape(&hours(max))
                )
                .unwrap();
            }
            out.push_str("</dd>");
        }
        if timing.timebox.is_some() || timing.timebox_hours_max.is_some() {
            write!(
                out,
                "<dt>Timebox</dt><dd>{}{}</dd>",
                escape(timing.timebox.as_deref().unwrap_or("")),
                timing
                    .timebox_hours_max
                    .map(|m| format!(" (up to {})", escape(&hours(m))))
                    .unwrap_or_default()
            )
            .unwrap();
        }
        match timing.over_timebox {
            Some(true) => out.push_str(&format!(
                "<dt>Over timebox</dt><dd>{}</dd>",
                tone_badge("over timebox", crate::profile::Tone::Bad)
            )),
            Some(false) => out.push_str("<dt>Over timebox</dt><dd>no</dd>"),
            None => {}
        }
        out.push_str("</dl></section>");
    }
    if !run.sessions.is_empty() {
        out.push_str("<section><h2>Sessions</h2><div class=\"table-wrap\"><table class=\"data\"><thead><tr><th>Session</th><th>Provider</th><th>Model</th><th>Started</th></tr></thead><tbody>");
        for session in &run.sessions {
            write!(
                out,
                "<tr id=\"session-{}\"><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&url(&session.id)),
                escape(&session.id),
                escape(session.provider.as_deref().unwrap_or("—")),
                escape(session.model.as_deref().unwrap_or("—")),
                at(session.started_at.as_deref())
            )
            .unwrap();
        }
        out.push_str("</tbody></table></div></section>");
    }
    for attempt in &run.attempts {
        write!(
            out,
            "<section class=\"attempt\" id=\"attempt-{}\"><h2>Attempt {} {}</h2><dl class=\"facts\"><dt>Attempt id</dt><dd><code>{}</code></dd><dt>Started</dt><dd>{}</dd><dt>Ended</dt><dd>{}</dd>",
            attempt.number,
            attempt.number,
            state_badge(&attempt.state),
            escape(if attempt.id.is_empty() { "—" } else { &attempt.id }),
            at(attempt.started_at.as_deref()),
            at(attempt.ended_at.as_deref()),
        )
        .unwrap();
        if let Some(costs) = &attempt.costs {
            write!(out, "<dt>Cost</dt><dd>{}</dd>", cost_line(costs)).unwrap();
        }
        out.push_str("</dl>");
        if let Some(stop) = &attempt.stop {
            out.push_str(&stop_block(stop, "This attempt stopped"));
        }
        out.push_str(&ctx.facts(&attempt.facts));
        out.push_str(&ctx.links(&attempt.links));
        if attempt.steps.is_empty() {
            out.push_str("<p class=\"muted\">No steps recorded for this attempt.</p>");
        } else {
            out.push_str("<ol class=\"steps\">");
            for step in &attempt.steps {
                out.push_str(&step_item(&ctx, run, step, now));
            }
            out.push_str("</ol>");
        }
        out.push_str("</section>");
    }
    out.push_str("<section><h2>Documents</h2>");
    if run.documents.is_empty() {
        out.push_str("<p class=\"muted\">This run has no documents.</p>");
    } else {
        out.push_str("<div class=\"table-wrap\"><table class=\"data\"><thead><tr><th>Document</th><th>Kind</th><th class=\"num\">Size</th><th>State</th></tr></thead><tbody>");
        for doc in &run.documents {
            write!(
                out,
                "<tr><td>{}</td><td>{}</td><td class=\"num\">{}</td><td>{}</td></tr>",
                doc_link(&run.run, doc),
                escape(&words(&doc.kind)),
                doc.bytes
                    .map(|b| escape(&size(b)))
                    .unwrap_or_else(|| "—".into()),
                doc_state(&doc.state),
            )
            .unwrap();
        }
        out.push_str("</tbody></table></div>");
    }
    out.push_str("</section>");
    if !run.gaps.is_empty() {
        out.push_str("<section><h2>Known gaps</h2><p class=\"muted\">What the producer knows is missing from this run.</p><ul class=\"values\">");
        for gap in &run.gaps {
            write!(out, "<li><code>{}</code></li>", escape(gap)).unwrap();
        }
        out.push_str("</ul></section>");
    }
    out.push_str(&related(view, summary));
    out
}

/// A document's title, linked to its first page when it has readable
/// pages; otherwise the title alone.
fn doc_link(run: &str, doc: &DocRef) -> String {
    let title = if doc.title.is_empty() {
        &doc.id
    } else {
        &doc.title
    };
    if doc_readable(&doc.state) && !doc.pages.is_empty() {
        format!(
            "<a href=\"/run/{}/doc/{}\">{}</a>",
            url(run),
            url(&doc.id),
            escape(title)
        )
    } else {
        escape(title)
    }
}

fn doc_state(state: &str) -> String {
    let tone = match state {
        "ready" => crate::profile::Tone::Good,
        "live" | "pending" => crate::profile::Tone::Waiting,
        "truncated" => crate::profile::Tone::Warn,
        _ => crate::profile::Tone::Neutral,
    };
    tone_badge(
        &words(if state.is_empty() { "unknown" } else { state }),
        tone,
    )
}

/// Back-links: the bead, and the other runs of the same subject.
fn related(view: &View, summary: &RunSummary) -> String {
    let key = summary
        .subject
        .reference
        .as_deref()
        .or(summary.subject.id.as_deref());
    let mut out = "<section class=\"related\"><h2>Related</h2><ul class=\"values\">".to_string();
    let bead = [
        summary.subject.reference.as_deref(),
        summary.subject.id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find_map(|k| view.snapshot().and_then(|s| s.find(k)));
    if let Some(bead) = bead {
        write!(out, "<li>Bead: {}</li>", bead_link(&bead.id, &bead.title)).unwrap();
    }
    let mut others: Vec<&RunSummary> = Vec::new();
    for k in [
        summary.subject.reference.as_deref(),
        summary.subject.id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        for r in view.runs_for_subject(k) {
            if r.run != summary.run && !others.iter().any(|o| o.run == r.run) {
                others.push(r);
            }
        }
    }
    for other in &others {
        write!(
            out,
            "<li>Run {} {} <small class=\"muted\">{}</small></li>",
            run_link(&other.run, &other.run),
            state_badge(&other.state),
            other.started_at.as_deref().map(when).unwrap_or_default()
        )
        .unwrap();
    }
    if bead.is_none() && others.is_empty() {
        write!(
            out,
            "<li class=\"muted\">No bead or other run for {}.</li>",
            escape(key.unwrap_or("this subject"))
        )
        .unwrap();
    }
    out.push_str("</ul><p><a href=\"/runs\">← All runs</a></p></section>");
    out
}

fn step_item(ctx: &Ctx, run: &Run, step: &Step, now: DateTime<Utc>) -> String {
    let kind = step_kind(&step.kind);
    let name = step
        .label
        .clone()
        .or_else(|| step.name.clone())
        .unwrap_or_else(|| capital(&words(&step.kind)));
    let name = match step.iteration {
        Some(n) => format!("{name} {n}"),
        None => name,
    };
    let result = step.result.as_deref().unwrap_or(&step.state);
    let tone = step_tone(result);
    let seconds_taken = step.seconds.or_else(|| {
        span(step.started_at.as_deref(), step.ended_at.as_deref(), now)
            .filter(|_| step.ended_at.is_some() || step.state == "running")
    });
    let current = run.current_step.as_deref() == Some(step.id.as_str());
    let mut out = format!(
        "<li class=\"step {}{}\" id=\"step-{}\"><div class=\"step-head\"><span class=\"kind kind-{kind}\" title=\"{}\">{}</span><strong>{}</strong> {}",
        tone_class(tone),
        if current { " current" } else { "" },
        escape(&url(&step.id)),
        escape(&step.kind),
        escape(&words(kind)),
        escape(&name),
        tone_badge(
            &words(if result.is_empty() { "unknown" } else { result }),
            tone
        ),
    );
    if let Some(detail) = &step.result_detail {
        write!(out, " <small>{}</small>", escape(&words(detail))).unwrap();
    }
    if let Some(s) = seconds_taken {
        write!(
            out,
            " <small class=\"muted\">{}</small>",
            escape(&seconds(s))
        )
        .unwrap();
    }
    out.push_str("</div>");
    let mut meta = Vec::new();
    if let Some(start) = &step.started_at {
        meta.push(format!("started {}", when(start)));
    }
    if let Some(end) = &step.ended_at {
        meta.push(format!("ended {}", when(end)));
    }
    if let Some(session) = &step.session {
        let model = run
            .sessions
            .iter()
            .find(|s| s.id == *session)
            .and_then(|s| s.model.as_deref())
            .map(|m| format!(" ({})", escape(m)))
            .unwrap_or_default();
        meta.push(format!(
            "session <a href=\"#session-{}\"><code>{}</code></a>{model}",
            escape(&url(session)),
            escape(session)
        ));
    }
    if !meta.is_empty() {
        write!(
            out,
            "<small class=\"step-meta\">{}</small>",
            meta.join(" · ")
        )
        .unwrap();
    }
    if let Some(failure) = &step.failure {
        out.push_str("<p class=\"failure\">");
        if let Some(code) = &failure.code {
            write!(out, "<code>{}</code> ", escape(code)).unwrap();
        }
        if let Some(text) = &failure.text {
            out.push_str(&escape(text));
        }
        out.push_str("</p>");
    }
    out.push_str(&ctx.facts(&step.facts));
    out.push_str(&ctx.links(&step.links));
    if !step.documents.is_empty() {
        out.push_str("<ul class=\"step-docs\">");
        for id in &step.documents {
            match run.documents.iter().find(|d| d.id == *id) {
                Some(doc) => {
                    let state = if doc_readable(&doc.state) {
                        String::new()
                    } else {
                        format!(" {}", doc_state(&doc.state))
                    };
                    write!(out, "<li>{}{state}</li>", doc_link(&run.run, doc)).unwrap();
                }
                None => write!(
                    out,
                    "<li><code>{}</code> <small class=\"muted\">not in this run's documents</small></li>",
                    escape(id)
                )
                .unwrap(),
            }
        }
        out.push_str("</ul>");
    }
    out.push_str("</li>");
    out
}

/// The total of a cost, marked as a lower bound when incomplete.
fn cost_line(cost: &Cost) -> String {
    let total = cost
        .usd
        .map(|u| escape(&money(u)))
        .unwrap_or_else(|| "not recorded".into());
    if cost.complete {
        total
    } else {
        format!(
            "at least {total} {}",
            tone_badge("incomplete", crate::profile::Tone::Warn)
        )
    }
}

fn cost_block(cost: &Cost) -> String {
    let mut out = format!(
        "<dl class=\"facts\"><dt>Total</dt><dd>{}</dd>",
        cost_line(cost)
    );
    if let Some(with) = cost.usd_with_overhead {
        write!(
            out,
            "<dt>With overhead</dt><dd>{}</dd>",
            escape(&money(with))
        )
        .unwrap();
    }
    if let Some(t) = &cost.tokens {
        let parts: Vec<String> = [
            ("input", t.input),
            ("output", t.output),
            ("cache read", t.cache_read),
            ("cache write", t.cache_write),
        ]
        .into_iter()
        .filter_map(|(label, n)| n.map(|n| format!("{} {label}", tokens(n))))
        .collect();
        if !parts.is_empty() {
            write!(
                out,
                "<dt>Tokens</dt><dd>{}</dd>",
                escape(&parts.join(" · "))
            )
            .unwrap();
        }
    }
    out.push_str("</dl>");
    if !cost.complete {
        out.push_str("<p class=\"muted\">Usage the producer knows of is missing, so the total is a lower bound.</p>");
        if !cost.gaps.is_empty() {
            out.push_str("<ul class=\"values\">");
            for gap in &cost.gaps {
                write!(out, "<li><code>{}</code></li>", escape(gap)).unwrap();
            }
            out.push_str("</ul>");
        }
    }
    out
}

fn breakdown(cost: &Cost, run: &Run) -> String {
    let mut out = String::new();
    if !cost.by_model.is_empty() {
        out.push_str("<h3>By model</h3><div class=\"table-wrap\"><table class=\"data\"><thead><tr><th>Model</th><th class=\"num\">Cost</th><th class=\"num\">Input</th><th class=\"num\">Output</th><th class=\"num\">Cache read</th><th class=\"num\">Cache write</th></tr></thead><tbody>");
        for (model, c) in &cost.by_model {
            let t = c.tokens.clone().unwrap_or_default();
            let n = |v: Option<u64>| v.map(tokens).unwrap_or_else(|| "—".into());
            write!(
                out,
                "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                escape(model),
                c.usd.map(money).unwrap_or_else(|| "—".into()),
                n(t.input),
                n(t.output),
                n(t.cache_read),
                n(t.cache_write)
            )
            .unwrap();
        }
        out.push_str("</tbody></table></div>");
    }
    if !cost.by_role.is_empty() {
        out.push_str("<h3>By role</h3><div class=\"table-wrap\"><table class=\"data\"><thead><tr><th>Role</th><th>Model</th><th class=\"num\">Cost</th></tr></thead><tbody>");
        for (role, c) in &cost.by_role {
            write!(
                out,
                "<tr><td>{}</td><td>{}</td><td class=\"num\">{}</td></tr>",
                escape(&words(role)),
                escape(
                    run.roles
                        .get(role)
                        .and_then(|r| r.model.as_deref())
                        .unwrap_or("—")
                ),
                c.usd.map(money).unwrap_or_else(|| "—".into())
            )
            .unwrap();
        }
        out.push_str("</tbody></table></div>");
    }
    out
}

/// A stop: code, class, why, detail, what to do next, the commands to
/// copy (never run), and the question for the owner.
fn stop_block(stop: &Stop, lead: &str) -> String {
    let title = stop.title.as_deref().unwrap_or(lead);
    let mut out = format!(
        "<section class=\"banner warn stop\" aria-label=\"Stop\"><h2>{}</h2><dl class=\"facts\"><dt>Stop code</dt><dd><code>{}</code></dd>",
        escape(title),
        escape(&stop.code)
    );
    for (label, value) in [("Class", &stop.class), ("Cause", &stop.cause)] {
        if let Some(v) = value {
            write!(out, "<dt>{label}</dt><dd>{}</dd>", escape(v)).unwrap();
        }
    }
    if let Some(at) = &stop.at {
        write!(out, "<dt>At</dt><dd>{}</dd>", when(at)).unwrap();
    }
    out.push_str("</dl>");
    if let Some(why) = &stop.why {
        write!(out, "<p>{}</p>", escape(why)).unwrap();
    }
    if !stop.detail.is_empty() {
        out.push_str("<ul class=\"values\">");
        for line in &stop.detail {
            write!(out, "<li>{}</li>", escape(line)).unwrap();
        }
        out.push_str("</ul>");
    }
    if let Some(next) = &stop.next {
        write!(out, "<p><strong>Next:</strong> {}</p>", escape(next)).unwrap();
    }
    if !stop.commands.is_empty() {
        out.push_str(
            "<p class=\"commands-note\">Commands to copy (Fabriscope never runs them):</p>",
        );
        for command in &stop.commands {
            write!(
                out,
                "<pre class=\"command\"><code>{}</code></pre>",
                escape(command)
            )
            .unwrap();
        }
    }
    if let Some(ask) = &stop.owner_ask {
        out.push_str("<h3>Owner ask</h3><dl class=\"facts\">");
        for (label, value) in [
            ("Problem", &ask.problem),
            ("Question", &ask.question),
            ("Recommended", &ask.recommended),
            ("If yes", &ask.if_yes),
            ("If not", &ask.if_not),
        ] {
            if let Some(v) = value {
                write!(out, "<dt>{label}</dt><dd>{}</dd>", escape(v)).unwrap();
            }
        }
        out.push_str("</dl>");
    }
    out.push_str("</section>");
    out
}

/// `/run/{id}/doc/{doc}`, page `page` (0-based); `None` for a run or
/// document the feed does not name, or a page past the last.
pub(crate) fn doc(view: &View, run: &str, doc: &str, page_no: usize) -> Option<String> {
    let summary = view.run_index()?.run(run)?;
    let title = format!("{run} · {doc}");
    let body = match view.run_cache.get_run(run) {
        Fetch::Pending => format!(
            "{}<p class=\"fetching\" role=\"status\">Fetching run… this page fills in when it arrives.</p>",
            heading("Document", doc, "")
        ),
        Fetch::Failed(error) => format!(
            "{}<div class=\"banner bad\" role=\"alert\"><strong>This run could not be read.</strong> {}</div>",
            heading("Document", doc, ""),
            escape(&error)
        ),
        Fetch::Ready(found) => {
            let doc_ref = found.documents.iter().find(|d| d.id == doc)?;
            if doc_readable(&doc_ref.state)
                && !doc_ref.pages.is_empty()
                && page_no >= doc_ref.pages.len()
            {
                return None;
            }
            doc_body(view, summary, doc_ref, page_no)
        }
    };
    Some(page(view, &title, "/runs", &body))
}

fn doc_body(view: &View, summary: &RunSummary, doc: &DocRef, page_no: usize) -> String {
    let title = if doc.title.is_empty() {
        doc.id.clone()
    } else {
        doc.title.clone()
    };
    let mut out = heading("Document", &title, "");
    write!(
        out,
        "<dl class=\"facts\"><dt>Run</dt><dd>{} {}</dd><dt>Kind</dt><dd>{}</dd><dt>Content</dt><dd><code>{}</code></dd><dt>Size</dt><dd>{}</dd><dt>State</dt><dd>{}</dd>",
        run_link(&summary.run, &summary.run),
        summary
            .subject
            .title
            .as_deref()
            .map(|t| format!("<small class=\"muted\">{}</small>", escape(t)))
            .unwrap_or_default(),
        escape(&words(&doc.kind)),
        escape(&doc.content_type),
        doc.bytes
            .map(|b| escape(&size(b)))
            .unwrap_or_else(|| "—".into()),
        doc_state(&doc.state),
    )
    .unwrap();
    if let Some(r) = &doc.redaction {
        write!(
            out,
            "<dt>Redaction</dt><dd>policy <code>{}</code>, {} {}</dd>",
            escape(&r.policy),
            r.replaced,
            if r.replaced == 1 {
                "replacement"
            } else {
                "replacements"
            }
        )
        .unwrap();
    }
    if let Some(kind) = doc.source.as_ref().and_then(|s| s.kind.as_deref()) {
        write!(out, "<dt>Source</dt><dd>{}</dd>", escape(kind)).unwrap();
    }
    out.push_str("</dl>");
    if !doc_readable(&doc.state) {
        write!(
            out,
            "<p class=\"doc-state\">This document is {}: {}</p>",
            escape(&doc.state),
            escape(match doc.state.as_str() {
                "pending" => "it has not been produced yet.",
                "unavailable" => "its source is gone or unreadable.",
                "withheld" => "the producer will not publish it.",
                "expired" => "it is past retention.",
                _ => "it has no pages to show.",
            })
        )
        .unwrap();
        return out;
    }
    if doc.pages.is_empty() {
        out.push_str("<p class=\"doc-state\">This document has no pages yet.</p>");
        return out;
    }
    if doc.state == "truncated" {
        out.push_str("<p class=\"muted\">The producer cut this document at its size limit.</p>");
    }
    let pager = pager(&summary.run, doc, page_no);
    out.push_str(&pager);
    match view.run_cache.get_doc_page(&summary.run, &doc.id, page_no) {
        Fetch::Pending => out.push_str(
            "<p class=\"fetching\" role=\"status\">Fetching page… it appears here when it arrives.</p>",
        ),
        Fetch::Failed(error) => write!(
            out,
            "<div class=\"banner bad\" role=\"alert\"><strong>This page could not be read.</strong> {}</div>",
            escape(&error)
        )
        .unwrap(),
        Fetch::Ready(text) => {
            let (shown, cut) = capped(&text);
            if cut {
                write!(
                    out,
                    "<p class=\"banner warn\">This page is {}; only the first {} are shown.</p>",
                    escape(&size(text.len() as u64)),
                    escape(&size(RENDER_CAP as u64))
                )
                .unwrap();
            }
            out.push_str(&match doc.content_type.as_str() {
                "text/plain" => plain(shown),
                "application/x-ndjson" => transcript(shown, page_no),
                "application/json" => json(shown),
                other => format!(
                    "<p class=\"doc-state\">Fabriscope cannot show <code>{}</code> documents.</p>",
                    escape(other)
                ),
            });
            out.push_str(&pager);
        }
    }
    out
}

/// At most `RENDER_CAP` bytes of a page, cut at a line end when there is
/// one.
fn capped(text: &str) -> (&str, bool) {
    if text.len() <= RENDER_CAP {
        return (text, false);
    }
    let mut end = RENDER_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let end = text[..end].rfind('\n').map_or(end, |n| n + 1);
    (&text[..end], true)
}

fn pager(run: &str, doc: &DocRef, current: usize) -> String {
    let last = doc.pages.len() - 1;
    let link = |n: usize, label: &str| {
        let p = &doc.pages[n];
        let range = match (&p.first_at, &p.last_at) {
            (Some(a), Some(b)) => format!(" title=\"{} – {}\"", escape(a), escape(b)),
            _ => String::new(),
        };
        format!(
            "<a href=\"/run/{}/doc/{}?page={n}\"{range}>{label}</a>",
            url(run),
            url(&doc.id)
        )
    };
    let mut out = "<nav class=\"pagination doc-pager\" aria-label=\"Document pages\">".to_string();
    if current > 0 {
        out.push_str(&link(0, "« First"));
        out.push_str(&link(current - 1, "‹ Previous"));
    }
    let p = &doc.pages[current];
    let range = match (&p.first_at, &p.last_at) {
        (Some(a), Some(b)) => format!(" · {} – {}", when(a), when(b)),
        (Some(a), None) => format!(" · from {}", when(a)),
        _ => String::new(),
    };
    write!(
        out,
        "<span>Page {} of {}{range}</span>",
        current + 1,
        doc.pages.len()
    )
    .unwrap();
    if current < last {
        out.push_str(&link(current + 1, "Next ›"));
        out.push_str(&link(last, "Last »"));
    }
    out.push_str("</nav>");
    out
}

/// Plain text as numbered lines, each with an `L<n>` anchor.
fn plain(text: &str) -> String {
    let mut out = "<pre class=\"doc-lines\">".to_string();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        write!(
            out,
            "<span class=\"line\" id=\"L{n}\"><a class=\"ln\" href=\"#L{n}\">{n}</a><span class=\"lt\">{}</span></span>",
            escape(line)
        )
        .unwrap();
    }
    out.push_str("</pre>");
    out
}

fn json(text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => format!(
            "<pre class=\"json doc-json\">{}</pre>",
            escape(&serde_json::to_string_pretty(&value).unwrap_or_default())
        ),
        Err(error) => format!(
            "<p class=\"banner warn\">This page is not valid JSON ({}); it is shown as text.</p>{}",
            escape(&error.to_string()),
            plain(text)
        ),
    }
}

/// The label for a transcript role.
fn role_label(role: &str) -> String {
    match role {
        "system" => "System".into(),
        "user" => "User".into(),
        "assistant" => "Assistant".into(),
        "thinking" => "Thinking".into(),
        "tool_call" => "Tool call".into(),
        "tool_result" => "Tool result".into(),
        other => capital(&words(other)),
    }
}

/// Who and when, for a turn's header.
fn turn_meta(turn: &Turn) -> String {
    let mut parts = Vec::new();
    if let Some(tool) = &turn.tool {
        parts.push(format!("<code>{}</code>", escape(tool)));
    }
    if let Some(model) = &turn.model {
        parts.push(escape(model));
    }
    if let Some(at) = &turn.at {
        parts.push(when(at));
    }
    parts.join(" · ")
}

/// A turn's text, escaped with its whitespace kept (never Markdown, never
/// markup), and what the producer cut from it.
fn turn_text(turn: &Turn) -> String {
    let mut out = format!("<div class=\"turn-text\">{}</div>", escape(&turn.text));
    if let Some(n) = turn.omitted_bytes.filter(|n| *n > 0) {
        write!(out, "<p class=\"omitted\">… {} bytes omitted</p>", count(n)).unwrap();
    }
    out
}

/// Transcript turns: messages in full; thinking, and each tool call with
/// the result that follows it, collapsed.
fn transcript(text: &str, page_no: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let turns: Vec<Result<Turn, String>> = lines
        .iter()
        .map(|l| serde_json::from_str::<Turn>(l).map_err(|e| e.to_string()))
        .collect();
    let mut out = "<div class=\"transcript\">".to_string();
    let mut i = 0;
    while i < turns.len() {
        let id = format!("p{page_no}-t{i}");
        match &turns[i] {
            Err(error) => {
                write!(
                    out,
                    "<div class=\"turn turn-unreadable\"><p class=\"muted\">An unreadable line ({}):</p><div class=\"turn-text\">{}</div></div>",
                    escape(error),
                    escape(lines[i])
                )
                .unwrap();
            }
            Ok(turn) if turn.role == "tool_call" => {
                let result = match turns.get(i + 1) {
                    Some(Ok(next))
                        if next.role == "tool_result"
                            && (next.tool.is_none() || next.tool == turn.tool) =>
                    {
                        Some(next)
                    }
                    _ => None,
                };
                write!(
                    out,
                    "<details class=\"turn tool\" id=\"{id}\"><summary><strong>{}</strong> <span class=\"turn-meta\">{}</span></summary><div class=\"tool-call\"><p class=\"turn-role\">Call</p>{}</div>",
                    if result.is_some() {
                        "Tool call and result"
                    } else {
                        "Tool call"
                    },
                    turn_meta(turn),
                    turn_text(turn)
                )
                .unwrap();
                if let Some(result) = result {
                    write!(
                        out,
                        "<div class=\"tool-result\"><p class=\"turn-role\">Result <span class=\"turn-meta\">{}</span></p>{}</div>",
                        turn_meta(result),
                        turn_text(result)
                    )
                    .unwrap();
                    i += 1;
                }
                out.push_str("</details>");
            }
            Ok(turn) if turn.role == "tool_result" || turn.role == "thinking" => {
                write!(
                    out,
                    "<details class=\"turn turn-{}\" id=\"{id}\"><summary><strong>{}</strong> <span class=\"turn-meta\">{}</span></summary>{}</details>",
                    escape(&turn.role),
                    escape(&role_label(&turn.role)),
                    turn_meta(turn),
                    turn_text(turn)
                )
                .unwrap();
            }
            Ok(turn) => {
                let class = match turn.role.as_str() {
                    "system" | "user" | "assistant" => turn.role.as_str(),
                    _ => "other",
                };
                write!(
                    out,
                    "<article class=\"turn turn-{class}\"><div class=\"turn-head\"><strong>{}</strong> <span class=\"turn-meta\">{}</span></div>{}</article>",
                    escape(&role_label(&turn.role)),
                    turn_meta(turn),
                    turn_text(turn)
                )
                .unwrap();
            }
        }
        i += 1;
    }
    if turns.is_empty() {
        out.push_str("<p class=\"muted\">This page has no turns.</p>");
    }
    out.push_str("</div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::work_view;

    const TRANSCRIPT: &str =
        include_str!("../../tests/fixtures/runfeed/v1/docs/run-0002/transcript/0002.ndjson");
    const LOG: &str =
        include_str!("../../tests/fixtures/runfeed/v1/docs/run-0002/gate-log/0001.txt");

    #[test]
    fn the_runs_page_lists_filters_and_totals() {
        let view = work_view();
        let all = runs(&view, &Facets::default());
        for run in ["run-0001", "run-0002", "run-0003", "run-0004", "run-0005"] {
            assert!(all.contains(&format!("/run/{}", url(run))), "{run}");
        }
        let first = all.find("run%2D0001").unwrap();
        assert!(first < all.find("run%2D0005").unwrap(), "newest first");
        assert!(
            all.contains("href=\"/bead/wk%2D2\""),
            "subject links to its bead"
        );
        // `rig` is a profile group, but no item names the feed's `alpha`,
        // so the chip is text, never a link to a page that does not exist.
        assert!(all.contains("rig: alpha"), "{all}");
        assert!(!all.contains("href=\"/by/rig/alpha\""));
        assert!(all.contains("<code>scope-exceeded</code>"));
        assert!(all.contains("$11.41"), "{all}");
        assert!(!all.contains("no cost recorded"));
        let q = |query: &str| {
            let uri: axum::http::Uri = format!("/runs?{query}").parse().unwrap();
            runs(
                &view,
                &axum::extract::Query::<Facets>::try_from_uri(&uri)
                    .unwrap()
                    .0,
            )
        };
        let stopped = q("state=stopped");
        assert!(stopped.contains("run%2D0003") && !stopped.contains("/run/run%2D0002\""));
        let beta = q("rig=beta&milestone=v1");
        assert!(beta.contains("run%2D0003") && !beta.contains("/run/run%2D0001\""));
        let october = q("from=2026-10-01&to=2026-10-31");
        assert!(october.contains("run%2D0004") && !october.contains("/run/run%2D0001\""));
        assert!(q("q=rounding").contains("run%2D0004"));
        assert!(!all.contains(" style="));
    }

    #[test]
    fn a_run_page_shows_steps_costs_stops_and_documents() {
        let view = work_view();
        let page = run(&view, "run-0002").unwrap();
        for wanted in [
            "id=\"attempt-1\"",
            "id=\"attempt-2\"",
            "Gate 2",
            "<code>tests-failed</code> 2 of 148 tests failed",
            "Gate time",
            "Tests failed",
            "Retried because",
            "href=\"/run/run%2D0002/doc/transcript\"",
            "href=\"/run/run%2D0002/doc/gate%2Dlog\"",
            "href=\"https://example.com/acme/widgets/pull/42\"",
            "href=\"https://example.com/acme/widgets/commit/9eedcb590af984bdf4520a8939840bd61d4979fe\"",
            "href=\"https://example.com/acme/widgets/tree/e9e83fd20809ef695d3056dda5bc16804003bcc4\"",
            "over timebox",
            "$6.02",
            "model-small",
            "Landed without a stop",
            "/run/run%2D0004",
        ] {
            assert!(page.contains(wanted), "{wanted}");
        }
        let stopped = run(&view, "run-0003").unwrap();
        for wanted in [
            "<code>scope-exceeded</code>",
            "Widen WORK-0003&#39;s write scope",
            "<pre class=\"command\"><code>workctl edit WORK-0003 --add-scope &quot;migrations/**&quot;</code></pre>",
            "at least $0.88",
            "usage-missing:ses-0003-a",
        ] {
            assert!(stopped.contains(wanted), "{wanted}");
        }
        assert!(
            !stopped.contains("/run/run%2D0003/doc/transcript"),
            "a withheld document has no link"
        );
        assert!(stopped.contains("withheld"));
        assert!(
            run(&view, "run-0005")
                .unwrap()
                .contains("steps-not-retained")
        );
        assert!(run(&view, "nope").is_none());
        let mut fresh = view.clone();
        fresh.run_cache = Default::default();
        assert!(run(&fresh, "run-0002").unwrap().contains("Fetching run…"));
        fresh.run_cache.put_run("run-0002", Err("gone".into()));
        assert!(
            run(&fresh, "run-0002")
                .unwrap()
                .contains("could not be read.</strong> gone")
        );
    }

    #[test]
    fn links_only_click_through_https() {
        let view = work_view();
        let ctx = Ctx {
            view: &view,
            run: None,
            repo: None,
        };
        let plain = ctx.link(&Link {
            rel: "ci".into(),
            href: Some("http://ci.example/1".into()),
            text: Some("CI".into()),
            ..Default::default()
        });
        assert!(!plain.contains("href="), "{plain}");
        assert!(plain.contains("<code class=\"unlinked\">http://ci.example/1</code>"));
        let script = ctx.link(&Link {
            rel: "external".into(),
            href: Some("javascript:alert(1)".into()),
            ..Default::default()
        });
        assert!(!script.contains("href="), "{script}");
    }

    #[test]
    fn document_pages_number_lines_and_escape_turns() {
        let view = work_view();
        assert!(
            doc(&view, "run-0002", "transcript", 1)
                .unwrap()
                .contains("Fetching page…")
        );
        view.run_cache
            .put_doc_page("run-0002", "transcript", 1, Ok(TRANSCRIPT.into()));
        let page = doc(&view, "run-0002", "transcript", 1).unwrap();
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!page.contains("<script>alert"));
        assert!(page.contains("Tool call and result"));
        assert!(page.contains("<details class=\"turn tool\" id=\"p1-t0\">"));
        assert!(page.contains("policy <code>credentials-v1</code>, 1 replacement"));
        assert!(page.contains("Page 2 of 2"));
        assert!(page.contains("?page=0\""));
        view.run_cache
            .put_doc_page("run-0002", "gate-log", 0, Ok(LOG.into()));
        let log = doc(&view, "run-0002", "gate-log", 0).unwrap();
        assert!(log.contains("id=\"L10\"") && !log.contains("id=\"L11\""));
        assert!(log.contains("href=\"#L3\">3</a>"));
        let withheld = doc(&view, "run-0003", "transcript", 0).unwrap();
        assert!(withheld.contains("the producer will not publish it"));
        assert!(doc(&view, "run-0002", "transcript", 7).is_none());
        assert!(doc(&view, "run-0002", "nope", 0).is_none());
        let first = transcript(
            r#"{"role":"thinking","text":"hmm"}
{"role":"tool_result","tool":"x","text":"<b>","omitted_bytes":18234}
not json"#,
            0,
        );
        assert!(first.contains("<details class=\"turn turn-thinking\" id=\"p0-t0\">"));
        assert!(first.contains("… 18,234 bytes omitted"));
        assert!(first.contains("&lt;b&gt;"));
        assert!(first.contains("An unreadable line"));
        let big = "x\n".repeat(RENDER_CAP);
        let (shown, cut) = capped(&big);
        assert!(cut && shown.len() <= RENDER_CAP && shown.ends_with('\n'));
        assert!(json("{\"a\":[1]}").contains("&quot;a&quot;"));
    }
}
