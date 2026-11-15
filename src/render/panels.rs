// Owned by WP2 — Bead page. Frozen signatures:
// `banner(&View, &Bead) -> String`, `panels(&View, &Bead) -> String`.
//! The profile's typed panels on a bead's page (fields shown with
//! `render::values::value`), the held banner (why, next, commands), and the
//! runs and receipts joined to the bead.

use super::fmt::{count, money, seconds, tokens, when, words};
use super::values::{Ctx, value};
use super::{bead_link, escape, run_link, tone_badge, url};
use crate::beads::Bead;
use crate::model::View;
use crate::profile::{FieldRule, Source, Tone};
use crate::runs::{self, RunSummary, cache::Fetch};
use std::fmt::Write;
use std::sync::Arc;

/// A label for a source shown as a fact: `[labels]`, else its last path
/// segment in words (`work.suspended.stop_code` → `stop code`).
pub(crate) fn source_label(view: &View, source: &Source) -> String {
    let key = source.to_string();
    if let Some(label) = view.profile.fact_label(&key) {
        return label.to_string();
    }
    let tail = key.rsplit([':', '.']).next().unwrap_or(&key);
    words(tail)
}

/// A run-feed fact's label: its own, else the profile's, else the key.
pub(crate) fn feed_label(view: &View, fact: &runs::Fact) -> String {
    fact.label
        .clone()
        .or_else(|| view.profile.fact_label(&fact.key).map(String::from))
        .unwrap_or_else(|| fact.key.clone())
}

/// A run-feed fact's value as HTML, by its declared type.
pub(crate) fn feed_value(fact: &runs::Fact) -> String {
    let number = fact.value.as_f64();
    let text = crate::profile::text_of(&fact.value).unwrap_or_default();
    match (fact.kind.as_deref(), number) {
        (Some("usd"), Some(n)) => escape(&money(n)),
        (Some("seconds"), Some(n)) => escape(&seconds(n)),
        (Some("hours"), Some(n)) => escape(&seconds(n * 3600.0)),
        (Some("tokens"), Some(n)) => escape(&tokens(n.max(0.0) as u64)),
        (Some("count"), Some(n)) => escape(&count(n.max(0.0) as u64)),
        (Some("percent"), Some(n)) => escape(&format!("{n}%")),
        (Some("sha" | "tree"), _) => format!(
            "<code title=\"{}\">{}</code>",
            escape(&text),
            escape(text.get(..9).unwrap_or(&text))
        ),
        (Some("id"), _) => format!("<code>{}</code>", escape(&text)),
        (Some("timestamp"), _) => when(&text),
        (Some("bool"), _) => match text.as_str() {
            "true" => "yes".into(),
            "false" => "no".into(),
            other => escape(other),
        },
        _ => escape(&text),
    }
}

/// One field of a panel or receipt, or `None` when it has no value.
fn field(view: &View, bead: &Bead, rule: &FieldRule, base: Option<&str>) -> Option<String> {
    let v = rule.path.value(bead)?;
    let ctx = Ctx {
        view,
        unit: rule.unit.as_deref(),
        repo: rule.repo.as_deref(),
        base,
    };
    let html = value(rule.kind, &v, &ctx);
    (!html.is_empty()).then(|| format!("<dt>{}</dt><dd>{html}</dd>", escape(&rule.label)))
}

/// The bead's document base path, for relative Markdown links.
fn base_path(view: &View, bead: &Bead) -> Option<String> {
    view.profile
        .documents
        .as_ref()
        .and_then(|d| d.base_path.text(bead))
}

/// A run joined to a bead, with its doc as far as it has been read.
pub(crate) struct Joined<'a> {
    pub summary: &'a RunSummary,
    pub doc: Fetch<Arc<runs::Run>>,
}

/// Every run the feed joins to this bead, newest first, with its doc.
pub(crate) fn joined<'a>(view: &'a View, bead: &Bead) -> Vec<Joined<'a>> {
    view.runs_for(bead)
        .into_iter()
        .map(|summary| Joined {
            summary,
            doc: view.run_cache.get_run(&summary.run),
        })
        .collect()
}

impl Joined<'_> {
    /// The run's stop: the run doc's, else the stop of its attempt `attempt`
    /// (or its last attempt).
    fn stop(&self, attempt: Option<&str>) -> Option<runs::Stop> {
        let Fetch::Ready(doc) = &self.doc else {
            return None;
        };
        doc.stop.clone().or_else(|| {
            doc.attempts
                .iter()
                .rev()
                .find(|a| attempt.is_none_or(|id| a.id == id))
                .and_then(|a| a.stop.clone())
        })
    }

    /// Whether the run halted: its doc has a stop, or (unread) its index
    /// entry names a stop code.
    fn stopped(&self) -> bool {
        match &self.doc {
            Fetch::Ready(_) => self.stop(None).is_some(),
            _ => self.summary.stop_code.is_some(),
        }
    }

    /// Whether the run carries this attempt id (by the index's join, or in
    /// its doc).
    fn has_attempt(&self, view: &View, attempt: &str) -> bool {
        view.run_for_attempt(attempt)
            .is_some_and(|r| r.run == self.summary.run)
            || matches!(&self.doc, Fetch::Ready(doc) if doc.attempts.iter().any(|a| a.id == attempt))
    }
}

/// The run whose stop explains a state: the run carrying an attempt id the
/// state's facts name, else the newest stopped run.
fn stopped_run<'a, 'b>(
    view: &View,
    runs: &'b [Joined<'a>],
    attempts: &[String],
) -> Option<(&'b Joined<'a>, Option<String>)> {
    for attempt in attempts {
        if let Some(run) = runs
            .iter()
            .find(|r| r.has_attempt(view, attempt) && r.stopped())
        {
            return Some((run, Some(attempt.clone())));
        }
    }
    runs.iter().find(|r| r.stopped()).map(|r| (r, None))
}

fn stop_block(run: &Joined, attempt: Option<&str>) -> String {
    let link = run_link(&run.summary.run, &format!("Run {}", run.summary.run));
    let stop = match &run.doc {
        Fetch::Ready(_) => run.stop(attempt),
        Fetch::Pending => {
            return format!(
                "<p class=\"stop-pending\">{link} stopped{}. Its details have not been read yet.</p>",
                run.summary
                    .stop_code
                    .as_deref()
                    .map(|c| format!(" with <code>{}</code>", escape(c)))
                    .unwrap_or_default()
            );
        }
        Fetch::Failed(error) => {
            return format!(
                "<p class=\"alert\">The details of {link} could not be read. {}</p>",
                escape(error)
            );
        }
    };
    let Some(stop) = stop else {
        return String::new();
    };
    let mut out = "<div class=\"stop\" id=\"stop\">".to_string();
    let title = stop.title.as_deref().unwrap_or(&stop.code);
    write!(
        out,
        "<h3>{}</h3><p class=\"stop-meta\">Stop <code>{}</code>",
        escape(title),
        escape(&stop.code)
    )
    .unwrap();
    for (label, text) in [("cause", &stop.cause), ("class", &stop.class)] {
        if let Some(text) = text {
            write!(out, " · {label} {}", escape(text)).unwrap();
        }
    }
    if let Some(at) = &stop.at {
        write!(out, " · {}", when(at)).unwrap();
    }
    write!(out, " · {link}</p>").unwrap();
    if let Some(why) = &stop.why {
        write!(out, "<p>{}</p>", escape(why)).unwrap();
    }
    if !stop.detail.is_empty() {
        out.push_str("<ul class=\"stop-detail\">");
        for line in &stop.detail {
            write!(out, "<li>{}</li>", escape(line)).unwrap();
        }
        out.push_str("</ul>");
    }
    if let Some(next) = &stop.next {
        write!(
            out,
            "<p class=\"stop-next\"><strong>Next:</strong> {}</p>",
            escape(next)
        )
        .unwrap();
    }
    if !stop.commands.is_empty() {
        out.push_str("<p class=\"muted\">Commands to copy; nothing here runs them.</p><ol class=\"commands\">");
        for command in &stop.commands {
            write!(out, "<li><code>{}</code></li>", escape(command)).unwrap();
        }
        out.push_str("</ol>");
    }
    if let Some(ask) = &stop.owner_ask {
        out.push_str("<div class=\"owner-ask\"><h3>Owner ask</h3><dl class=\"facts\">");
        for (label, text) in [
            ("Problem", &ask.problem),
            ("Question", &ask.question),
            ("Recommended", &ask.recommended),
            ("If yes", &ask.if_yes),
            ("If not", &ask.if_not),
        ] {
            if let Some(text) = text {
                write!(out, "<dt>{label}</dt><dd>{}</dd>", escape(text)).unwrap();
            }
        }
        out.push_str("</dl></div>");
    }
    out.push_str("</div>");
    out
}

/// The banner for each state the bead is in that has a `banner` template
/// or the bad tone: the state, the filled template, the state's facts, how
/// long, and the stop of a run the feed joins to the bead (the first banner
/// only).
pub(crate) fn banner(view: &View, bead: &Bead) -> String {
    let Some(row) = view.facts(&bead.id) else {
        return String::new();
    };
    let now = chrono::Utc::now();
    let runs = joined(view, bead);
    let mut out = String::new();
    let mut stop_shown = false;
    for key in &row.states {
        let Some(state) = view.profile.state(key) else {
            continue;
        };
        if state.banner.is_none() && state.tone != Tone::Bad {
            continue;
        }
        write!(
            out,
            "<section class=\"banner {}\" id=\"banner-{}\" aria-label=\"{}\"><p class=\"banner-title\"><strong>{}</strong>",
            super::tone_class(state.tone),
            url(key),
            escape(&state.label),
            escape(&state.label)
        )
        .unwrap();
        if let Some(since) = state.since.as_ref().and_then(|s| s.text(bead)) {
            let long = crate::provenance::elapsed(&since, now)
                .map(|e| format!(" · for {}", escape(&e)))
                .unwrap_or_default();
            write!(out, " since {}{long}", when(&since)).unwrap();
        }
        out.push_str("</p>");
        if let Some(text) = view.profile.banner(key, bead) {
            write!(out, "<p>{}</p>", escape(&text)).unwrap();
        }
        let mut facts = String::new();
        let mut values = Vec::new();
        for source in &state.facts {
            if let Some(text) = source.text(bead) {
                write!(
                    facts,
                    "<dt>{}</dt><dd>{}</dd>",
                    escape(&source_label(view, source)),
                    escape(&text)
                )
                .unwrap();
                values.extend(source.texts(bead));
            }
        }
        if !facts.is_empty() {
            write!(out, "<dl class=\"facts\">{facts}</dl>").unwrap();
        }
        if !stop_shown {
            if let Some((run, attempt)) = stopped_run(view, &runs, &values) {
                out.push_str(&stop_block(run, attempt.as_deref()));
                stop_shown = true;
            }
        }
        out.push_str("</section>");
    }
    out
}

/// Every `[[panels]]` whose `when` matches this bead, as a titled list of
/// its fields that have a value; a panel with none is left out.
pub(crate) fn panels(view: &View, bead: &Bead) -> String {
    let now = chrono::Utc::now();
    let base = base_path(view, bead);
    let mut out = String::new();
    for panel in &view.profile.panels {
        if !panel.when.matches(bead, now) {
            continue;
        }
        let fields: String = panel
            .fields
            .iter()
            .filter_map(|f| field(view, bead, f, base.as_deref()))
            .collect();
        if fields.is_empty() {
            continue;
        }
        write!(
            out,
            "<section class=\"panel\" id=\"panel-{}\"><h2>{}</h2><dl class=\"facts\">{fields}</dl></section>",
            url(&panel.key),
            escape(&panel.title)
        )
        .unwrap();
    }
    out
}

/// The runs the feed joins to this bead: state, start, attempts, cost,
/// stop code and a link to each.
pub(crate) fn runs_section(view: &View, bead: &Bead) -> String {
    if view.runs.is_none() {
        return String::new();
    }
    let runs = view.runs_for(bead);
    let mut out = format!("<section id=\"runs\"><h2>Runs · {}</h2>", runs.len());
    if runs.is_empty() {
        out.push_str("<p class=\"muted\">The run feed has no runs for this bead.</p></section>");
        return out;
    }
    out.push_str("<div class=\"table-wrap\"><table class=\"data runs-joined\"><thead><tr><th>Run</th><th>State</th><th>Started</th><th class=\"num\">Attempts</th><th class=\"num\">Cost (run feed)</th><th>Stop</th></tr></thead><tbody>");
    for run in runs {
        write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td></tr>",
            run_link(&run.run, &run.run),
            tone_badge(&words(&run.state), runs::run_tone(&run.state)),
            run.started_at.as_deref().map(when).unwrap_or_else(|| "<span class=\"muted\">unknown</span>".into()),
            run.attempts,
            run.cost_usd.map(|c| escape(&money(c))).unwrap_or_else(|| "<span class=\"muted\">unknown</span>".into()),
            run.stop_code.as_deref().map(|c| format!("<code>{}</code>", escape(c))).unwrap_or_default()
        )
        .unwrap();
    }
    out.push_str("</tbody></table></div></section>");
    out
}

/// The receipt rows whose subject is this bead: kind, time, actor,
/// attempt, the receipt rule's fields, and a link to each row.
pub(crate) fn receipts_section(view: &View, bead: &Bead) -> String {
    let Some(s) = view.snapshot() else {
        return String::new();
    };
    let Some(receipts) = s
        .position(&bead.id)
        .and_then(|i| s.derived.receipts_by_subject.get(&i))
    else {
        return String::new();
    };
    let mut out = format!(
        "<section id=\"receipts\"><h2>Receipts · {}</h2><div class=\"table-wrap\"><table class=\"data receipts-joined\"><thead><tr><th>Kind</th><th>At</th><th>By</th><th>Attempt</th><th>Fields</th><th>Row</th></tr></thead><tbody>",
        receipts.len()
    );
    for &i in receipts {
        let (Some(receipt_bead), Some(facts)) = (
            s.beads.get(i),
            s.derived.rows.get(i).and_then(|r| r.receipt.as_ref()),
        ) else {
            continue;
        };
        let rule = view.profile.receipts.get(facts.rule);
        let mut fields = String::new();
        for f in rule.map(|r| r.fields.as_slice()).unwrap_or_default() {
            fields.push_str(&field(view, receipt_bead, f, None).unwrap_or_default());
        }
        if let Some(rule) = rule {
            for (key, source) in &rule.group {
                if let Some(text) = source.text(receipt_bead) {
                    write!(
                        fields,
                        "<dt>{}</dt><dd>{}</dd>",
                        escape(&words(key)),
                        escape(&text)
                    )
                    .unwrap();
                }
            }
        }
        let attempt = facts
            .attempt
            .as_deref()
            .map(|a| match view.run_for_attempt(a) {
                Some(run) => format!(
                    "<code>{}</code> <small>{}</small>",
                    escape(a),
                    run_link(&run.run, &run.run)
                ),
                None => format!("<code>{}</code>", escape(a)),
            })
            .unwrap_or_default();
        write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{attempt}</td><td>{}</td><td>{}</td></tr>",
            escape(&words(&facts.kind)),
            facts.at.as_deref().map(when).unwrap_or_default(),
            facts
                .actor
                .as_deref()
                .map(|a| super::person_link(&view.profile.person(a).name))
                .unwrap_or_default(),
            if fields.is_empty() {
                String::new()
            } else {
                format!("<dl class=\"facts compact\">{fields}</dl>")
            },
            bead_link(&receipt_bead.id, &receipt_bead.id)
        )
        .unwrap();
    }
    out.push_str("</tbody></table></div></section>");
    out
}

#[cfg(test)]
mod tests {
    use crate::testing::work_view;

    fn page(id: &str) -> String {
        crate::render::bead(&work_view(), id).unwrap()
    }

    #[test]
    fn a_held_bead_shows_its_banner_facts_and_the_feed_stop() {
        let page = page("wk-3");
        assert!(
            page.contains("<section class=\"banner bad\" id=\"banner-held\""),
            "{page}"
        );
        assert!(page.contains("Held since"));
        assert!(
            page.contains("scope-exceeded (scope)"),
            "the banner template is filled"
        );
        assert!(page.contains("<dt>stop code</dt><dd>scope-exceeded</dd>"));
        assert!(page.contains("<dt>attempt</dt><dd>att-0003-1</dd>"));
        assert!(page.contains("The change needs a file outside its write scope"));
        assert!(page.contains("Widen the write scope, or cut the migration"));
        assert!(page.contains(
            "<li><code>workctl edit WORK-0003 --add-scope &quot;migrations/**&quot;</code></li>"
        ));
        assert!(page.contains("<dt>Question</dt><dd>Widen WORK-0003&#39;s write scope"));
        assert!(page.contains("<dt>If not</dt>"));
        assert!(page.contains("href=\"/run/run%2D0003\""));
        assert!(
            !page.contains("Ready, as bd ready decides"),
            "held overrides ready"
        );
    }

    #[test]
    fn panels_render_every_kind_and_skip_missing_fields() {
        let page = page("wk-1");
        assert!(page.contains("id=\"panel-packet\""), "{page}");
        assert!(page.contains("<dt>Timebox</dt><dd>2–3 h</dd>"));
        assert!(page.contains("<dt>Spend cap</dt><dd>$4.50</dd>"));
        assert!(page.contains("<li>widgets:src/export.rs</li>"));
        assert!(page.contains("href=\"https://example.com/adr/ADR-0007\""));
        assert!(page.contains("<a href=\"/bead/wk%2D2\">WORK-0002</a>"));
        assert!(page.contains("<dt>Gate</dt><dd>no</dd>"));
        assert!(page.contains("<a href=\"/person/factory%2FhostA%2FrigA\">"));
        assert!(page.contains("<strong>column order</strong>"));
        assert!(page.contains("<pre class=\"json\">"));
        assert!(
            !page.contains("<dt>Evidence</dt>"),
            "a field without a value is skipped"
        );
        // The decision panel does not match a task.
        assert!(!page.contains("id=\"panel-decision\""));
    }

    #[test]
    fn runs_and_receipts_join_to_the_bead() {
        let page = page("wk-2");
        assert!(page.contains("<h2>Runs · 2</h2>"), "{page}");
        assert!(page.contains("<h2>Receipts · 3</h2>"), "{page}");
        assert!(page.contains("<a href=\"/bead/wk%2Drc%2Dgate\">wk-rc-gate</a>"));
        assert!(page.contains("<a href=\"/bead/wk%2Dop%2D1\">wk-op-1</a>"));
        assert!(page.contains("<dt>Gate time</dt><dd>6m 52s</dd>"));
        assert!(page.contains("superseded"));
    }
}
