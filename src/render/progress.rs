// Owned by WP2 — Bead page. Frozen signatures:
// `compact(&provenance::Store, &Bead, DateTime<Utc>) -> Option<(String, String)>`,
// `progress_section(&View, &Bead) -> String`.
//! Where a bead is up to, from its provenance log: the stage bar, the
//! compact bar in tables, and the progress section of a bead's page.

use super::fmt::{count, money, seconds, tokens, when, words};
use super::panels::joined;
use super::runs::{fact_label, fact_text};
use super::{escape, maybe, run_link, tone_badge, url};
use crate::beads::Bead;
use crate::model::View;
use crate::provenance::{self, Fact, Mark, Note, Progress, Stage, Usage};
use crate::runs::{self, cache::Fetch};
use std::fmt::Write;

pub(super) fn fact(fact: &Fact) -> String {
    match fact {
        Fact::Text(text) => format!("<span class=\"fact\">{}</span>", escape(text)),
        Fact::Code { full, short } => format!(
            "<code class=\"fact\" title=\"{}\">{}</code>",
            escape(full),
            escape(short)
        ),
        Fact::Link { href, text } => format!(
            "<a class=\"fact\" href=\"{}\" rel=\"noreferrer\">{}</a>",
            escape(href),
            escape(text)
        ),
    }
}

/// The stages as a line of steps. The compact form is dots only, each named
/// on hover and to screen readers; `details` adds each stage's payload
/// details (a claim's rig, a cut's formula).
fn stage_bar(stages: &[Stage], compact: bool, details: bool) -> String {
    let mut out = format!(
        "<ol class=\"stages{}\">",
        if compact { " compact" } else { "" }
    );
    for stage in stages {
        let (class, words) = (stage.mark.class(), stage.mark.words());
        if compact {
            let mut tip = format!("{} · {words}", stage.label);
            if let Some(at) = &stage.at {
                tip.push_str(&format!(" · {at}"));
            }
            write!(out, "<li class=\"stage {class}\" title=\"{}\"><span class=\"dot\" aria-hidden=\"true\"></span><span class=\"sr-only\">{}: {words}</span></li>", escape(&tip), stage.label).unwrap();
            continue;
        }
        let at = stage.at.as_deref().map(when).unwrap_or_default();
        let mut facts: String = stage.facts.iter().map(fact).collect();
        if details && !stage.details.is_empty() {
            let text: Vec<String> = stage
                .details
                .iter()
                .map(|(key, value)| {
                    format!("{} {}", escape(&super::fmt::words(key)), escape(value))
                })
                .collect();
            write!(
                facts,
                "<small class=\"stage-detail\">{}</small>",
                text.join(" · ")
            )
            .unwrap();
        }
        write!(out, "<li class=\"stage {class}\"><span class=\"dot\" aria-hidden=\"true\"></span><strong>{}</strong><span class=\"sr-only\"> ({words})</span>{at}{facts}</li>", stage.label).unwrap();
    }
    out.push_str("</ol>");
    out
}

/// What the step now due or stopped at says in a sentence, with how long.
fn now_line(p: &Progress, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    let stage = p.now()?;
    let wait = p
        .since
        .as_deref()
        .and_then(|since| provenance::elapsed(since, now))
        .map(|e| format!(" · {e}"))
        .unwrap_or_default();
    Some(match stage.mark {
        Mark::Stopped => format!("Stopped before {}{wait}", stage.label),
        _ if stage.label == "Claimed" => format!("Waiting for a claim{wait}"),
        _ => format!("{} is next{wait}", stage.label),
    })
}

pub(super) fn kind_words(kind: &str) -> String {
    match kind {
        "suspend" => "Suspended".into(),
        "resume" => "Resumed".into(),
        "handoff" => "Handed off".into(),
        other => words(other),
    }
}

fn note(note: &Note) -> String {
    let mut out = format!("{} {}", escape(&kind_words(&note.kind)), when(&note.at));
    if !note.actor.is_empty() {
        write!(out, " by {}", escape(&note.actor)).unwrap();
    }
    for (key, value) in &note.fields {
        write!(
            out,
            " · {} <strong>{}</strong>",
            escape(&words(key)),
            escape(value)
        )
        .unwrap();
    }
    out
}

/// A compact bar and its one-line state for a bead whose log has been read.
pub(crate) fn compact(
    trails: &provenance::Store,
    bead: &Bead,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<(String, String)> {
    let trail = trails.get(&bead.id)?;
    if trail.events.is_empty() {
        return None;
    }
    let p = Progress::new(bead, &trail.events);
    let line = now_line(&p, now).unwrap_or_default();
    Some((stage_bar(&p.stages, true, false), escape(&line)))
}

pub(crate) fn progress_section(view: &View, bead: &Bead) -> String {
    let Some(trails) = &view.provenance else {
        return String::new();
    };
    let mut out = "<section class=\"progress\" id=\"progress\"><h2>Progress</h2>".to_string();
    let Some(trail) = trails.get(&bead.id) else {
        out.push_str("<p class=\"muted\">Reading the provenance log…</p></section>");
        return out;
    };
    if trail.events.is_empty() {
        match &trail.error {
            Some(error) => write!(
                out,
                "<p class=\"alert\">The provenance log could not be read. {}</p>",
                escape(error)
            )
            .unwrap(),
            None => out.push_str("<p class=\"muted\">No provenance recorded for this bead.</p>"),
        }
        out.push_str("</section>");
        return out;
    }
    let now = chrono::Utc::now();
    let p = Progress::new(bead, &trail.events);
    let claims = match p.claims {
        0 => "Never claimed".to_string(),
        1 => "Claimed once".to_string(),
        n => format!("Claimed {n} times"),
    };
    write!(
        out,
        "<p class=\"muted\">{claims} · from <code>bd provenance log</code>, read {}</p>",
        when(&trail.read_at.format("%Y-%m-%dT%H:%M:%SZ").to_string())
    )
    .unwrap();
    if let Some(held) = &p.held {
        write!(
            out,
            "<p class=\"stop-note\">{}. It stays here until it is resumed.</p>",
            note(&held.stop)
        )
        .unwrap();
    }
    let enriched = super::bead::enriched(view);
    out.push_str(&stage_bar(&p.stages, false, enriched));
    if let Some(line) = now_line(&p, now) {
        write!(out, "<p class=\"now\">{}</p>", escape(&line)).unwrap();
    }
    if let Some(error) = &trail.error {
        write!(
            out,
            "<p class=\"alert\">The last read failed, so this is an earlier one. {}</p>",
            escape(error)
        )
        .unwrap();
    }
    if !p.earlier.is_empty() {
        write!(
            out,
            "<details id=\"earlier-attempts\"><summary>Earlier attempts · {}</summary><ul class=\"attempts\">",
            p.earlier.len()
        )
        .unwrap();
        for attempt in &p.earlier {
            let mut ending = match &attempt.stop {
                Some(stop) => note(&stop.stop),
                None => "Ended without a recorded stop".to_string(),
            };
            if let Some(resumed) = attempt.stop.as_ref().and_then(|s| s.resumed.as_ref()) {
                write!(ending, "<br>{}", note(resumed)).unwrap();
            }
            write!(
                out,
                "<li><strong>Attempt {}</strong>{}<small>{ending}</small></li>",
                attempt.number,
                stage_bar(&attempt.stages, true, false)
            )
            .unwrap();
        }
        out.push_str("</ul></details>");
    }
    if enriched {
        out.push_str(&attempts(view, bead, &p));
    }
    write!(out, "<details id=\"provenance-log\"><summary>Provenance log · {} events</summary><div class=\"table-wrap\"><table class=\"log\"><thead><tr><th>When</th><th>Kind</th><th>Reference</th><th>Detail</th><th>By</th></tr></thead><tbody>", trail.events.len()).unwrap();
    for event in &trail.events {
        let reference = provenance::reference(event)
            .map(|f| fact(&f))
            .unwrap_or_default();
        write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{reference}</td><td>{}</td><td>{}<small>{}</small></td></tr>",
            when(&event.occurred_at),
            escape(&event.kind),
            escape(&event.payload_line()),
            escape(&event.actor),
            escape(&event.source)
        )
        .unwrap();
    }
    out.push_str("</tbody></table></div></details></section>");
    out
}

/// What an attempt used, as facts; nothing recorded is said plainly, never
/// shown as zero.
fn usage(u: &Usage) -> String {
    if u.events == 0 {
        return "<p class=\"muted\">No usage recorded.</p>".into();
    }
    let u = &u.spend;
    let mut out = format!(
        "<dl class=\"facts usage\"><dt>Cost</dt><dd>{}</dd><dt>Tokens</dt><dd{}>{}</dd><dt>Wall time</dt><dd>{}</dd>",
        maybe(u.dollars, money),
        u.tokens
            .map(|t| format!(" title=\"{}\"", count(t)))
            .unwrap_or_default(),
        maybe(u.tokens, tokens),
        maybe(u.wall_seconds, seconds)
    );
    if u.gate_runs.is_some() || u.gate_seconds.is_some() {
        write!(
            out,
            "<dt>Gates</dt><dd>{} · {}</dd>",
            maybe(u.gate_runs, |n| format!(
                "{n} {}",
                if n == 1 { "run" } else { "runs" }
            )),
            maybe(u.gate_seconds, seconds)
        )
        .unwrap();
    }
    if u.gate_reused.is_some() || u.gate_saved_seconds.is_some() {
        write!(
            out,
            "<dt>Gates reused</dt><dd>{} · saved {}</dd>",
            maybe(u.gate_reused, |n| n.to_string()),
            maybe(u.gate_saved_seconds, seconds)
        )
        .unwrap();
    }
    out.push_str("</dl>");
    out
}

/// The run-feed side of one attempt: a link to the attempt on its run's
/// page, its state, the feed's cost for it, its facts and a strip of its
/// steps.
fn feed_attempt(view: &View, bead: &Bead, id: &str) -> String {
    let runs = joined(view, bead);
    let run = view.run_for_attempt(id).map(|r| r.run.clone()).or_else(|| {
        runs.iter().find_map(|r| match &r.doc {
            Fetch::Ready(doc) if doc.attempts.iter().any(|a| a.id == id) => {
                Some(r.summary.run.clone())
            }
            _ => None,
        })
    });
    let Some(run) = run else {
        return String::new();
    };
    let doc = match view.run_cache.get_run(&run) {
        Fetch::Ready(doc) => doc,
        Fetch::Pending => {
            return format!(
                "<p class=\"feed-join\">Run feed: {} <small>its details have not been read yet</small></p>",
                run_link(&run, &run)
            );
        }
        Fetch::Failed(error) => {
            return format!(
                "<p class=\"feed-join\">Run feed: {} <small>its details could not be read: {}</small></p>",
                run_link(&run, &run),
                escape(&error)
            );
        }
    };
    let Some(attempt) = doc.attempts.iter().find(|a| a.id == id) else {
        return format!(
            "<p class=\"feed-join\">Run feed: {} <small>does not list this attempt</small></p>",
            run_link(&run, &run)
        );
    };
    let mut out = format!(
        "<p class=\"feed-join\">Run feed: <a href=\"/run/{}#attempt-{n}\">{} · attempt {n}</a> {}",
        url(&run),
        escape(&run),
        tone_badge(&words(&attempt.state), runs::run_tone(&attempt.state)),
        n = attempt.number
    );
    if let Some(usd) = attempt.costs.as_ref().and_then(|c| c.usd) {
        write!(out, " <small>feed cost {}</small>", escape(&money(usd))).unwrap();
    }
    out.push_str("</p>");
    if !attempt.facts.is_empty() {
        out.push_str("<dl class=\"facts feed-facts\">");
        for f in &attempt.facts {
            write!(
                out,
                "<dt>{}</dt><dd>{}</dd>",
                escape(&fact_label(view, f)),
                fact_text(f)
            )
            .unwrap();
        }
        out.push_str("</dl>");
    }
    if !attempt.steps.is_empty() {
        out.push_str("<ol class=\"step-strip\" aria-label=\"Steps\">");
        for step in &attempt.steps {
            let result = step.result.as_deref().unwrap_or(&step.state);
            write!(
                out,
                "<li title=\"{}\">{}</li>",
                escape(&format!("{} · {}", step.kind, result)),
                tone_badge(&words(&step.kind), runs::step_tone(result))
            )
            .unwrap();
        }
        out.push_str("</ol>");
    }
    out
}

/// Every attempt, oldest first: its id, what it used (from provenance),
/// and the run feed's view of it when an attempt there has the same id.
fn attempts(view: &View, bead: &Bead, p: &Progress) -> String {
    let mut rows: Vec<(usize, Option<&str>, &Usage, String)> = p
        .earlier
        .iter()
        .map(|a| {
            let ending = match &a.stop {
                Some(stop) if stop.resumed.is_some() => "stopped, then resumed".to_string(),
                Some(stop) => kind_words(&stop.stop.kind).to_lowercase(),
                None => "ended".to_string(),
            };
            (a.number, a.id.as_deref(), &a.usage, ending)
        })
        .collect();
    if p.attempt.is_some() || p.current_usage.events > 0 {
        let state = if p.held.is_some() {
            "held here"
        } else if bead.is_closed() {
            "closed the bead"
        } else {
            "current"
        };
        rows.push((
            p.earlier.len() + 1,
            p.attempt.as_deref(),
            &p.current_usage,
            state.to_string(),
        ));
    }
    if rows.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "<h3>Attempts · {}</h3><ol class=\"attempt-cards\">",
        rows.len()
    );
    for (number, id, used, state) in rows {
        write!(
            out,
            "<li id=\"attempt-{number}\"><p class=\"attempt-head\"><strong>Attempt {number}</strong>"
        )
        .unwrap();
        if let Some(id) = id {
            write!(out, " <code>{}</code>", escape(id)).unwrap();
        }
        write!(out, " <small>{}</small></p>", escape(&state)).unwrap();
        out.push_str(&usage(used));
        if let Some(id) = id {
            out.push_str(&feed_attempt(view, bead, id));
        }
        out.push_str("</li>");
    }
    out.push_str("</ol>");
    out
}

/// What the bead's work cost, from each source on its own: the provenance
/// log's `used` events and the run feed's figure for the runs joined to it.
/// The two are never added together.
pub(crate) fn cost_section(view: &View, bead: &Bead) -> String {
    if !super::bead::enriched(view) {
        return String::new();
    }
    let mut out = String::new();
    match view.trail(&bead.id) {
        Some(trail) if !trail.events.is_empty() => {
            let p = Progress::new(bead, &trail.events);
            let total = p.usage();
            if total.events == 0 {
                out.push_str("<dt>Ledger usage</dt><dd><span class=\"muted\">No usage recorded</span> <small>Source: the provenance log has no <code>used</code> events.</small></dd>");
            } else {
                let attempts = p.earlier.iter().filter(|a| a.usage.events > 0).count()
                    + usize::from(p.current_usage.events > 0);
                write!(
                    out,
                    "<dt>Ledger usage</dt><dd><strong>{}</strong> · {} tokens · {} wall <small>Source: the provenance log's <code>used</code> events, summed over {attempts} {}.</small></dd>",
                    maybe(total.spend.dollars, money),
                    maybe(total.spend.tokens, tokens),
                    maybe(total.spend.wall_seconds, seconds),
                    if attempts == 1 { "attempt" } else { "attempts" }
                )
                .unwrap();
            }
        }
        Some(_) => {}
        None if view.provenance.is_some() => out.push_str(
            "<dt>Ledger usage</dt><dd><span class=\"muted\">Provenance not read yet</span></dd>",
        ),
        None => {}
    }
    let runs = view.runs_for(bead);
    let known: Vec<_> = runs
        .iter()
        .filter_map(|r| r.cost_usd.map(|c| (r, c)))
        .collect();
    if !known.is_empty() {
        let total: f64 = known.iter().map(|(_, c)| c).sum();
        let partial = known.iter().any(|(r, _)| {
            matches!(view.run_cache.get_run(&r.run), Fetch::Ready(doc) if doc.costs.as_ref().is_some_and(|c| !c.complete))
        });
        let each: Vec<String> = known
            .iter()
            .map(|(r, c)| format!("{} {}", run_link(&r.run, &r.run), escape(&money(*c))))
            .collect();
        let unknown = runs.len() - known.len();
        write!(
            out,
            "<dt>Run feed</dt><dd><strong>{}{}</strong> <small>Source: the run feed's cost for {} joined {} ({}){}.</small></dd>",
            if partial { "at least " } else { "" },
            escape(&money(total)),
            known.len(),
            if known.len() == 1 { "run" } else { "runs" },
            each.join(", "),
            if unknown > 0 {
                format!("; {unknown} more without a cost")
            } else {
                String::new()
            }
        )
        .unwrap();
    }
    if out.is_empty() {
        return String::new();
    }
    format!(
        "<section id=\"cost\"><h2>Cost</h2><dl class=\"facts cost-sources\">{out}</dl><p class=\"muted\">Each figure comes from its own source; they are never added together.</p></section>"
    )
}

#[cfg(test)]
mod tests {
    use crate::provenance;
    use crate::render::bead::bead;
    use crate::render::tests::{view, with_trail};

    #[test]
    fn a_bead_page_shows_its_stages_from_provenance_and_escapes_them() {
        // Without a local bd there is no provenance section at all.
        assert!(!bead(&view(), "fx-1").unwrap().contains("id=\"progress\""));
        let mut reading = view();
        reading.provenance = Some(provenance::Store::default());
        assert!(
            bead(&reading, "fx-1")
                .unwrap()
                .contains("Reading the provenance log")
        );
        let page = bead(
            &with_trail(
                view(),
                r#"[
                  {"id":"1","kind":"claim","actor":"rig <x>","occurred_at":"2026-11-15T03:53:05Z","payload":"{\"rig\":\"r\"}"},
                  {"id":"2","kind":"cut","actor":"rig","occurred_at":"2026-11-15T03:55:36Z","ref":"w:1","ref_kind":"work-id"},
                  {"id":"3","kind":"commit","actor":"rig","occurred_at":"2026-11-15T05:15:34Z","ref":"7e40693d44991d5a0fff54b74961976bab3bd672","ref_kind":"git-sha"},
                  {"id":"4","kind":"land","actor":"rig","occurred_at":"2026-11-15T05:15:48Z","ref":"https://github.com/o/r/pull/9","ref_kind":"pr"}
                ]"#,
            ),
            "fx-1",
        )
        .unwrap();
        assert!(page.contains("<li class=\"stage done\">"), "{page}");
        assert!(page.contains("<li class=\"stage current\">"));
        assert!(page.contains("Closed is next"));
        assert!(page.contains("by rig &lt;x&gt;"), "actors are escaped");
        assert!(page.contains(
            "<code class=\"fact\" title=\"7e40693d44991d5a0fff54b74961976bab3bd672\">7e40693</code>"
        ));
        assert!(
            page.contains("href=\"https://github.com/o/r/pull/9\" rel=\"noreferrer\">PR #9</a>")
        );
        assert!(page.contains("Provenance log · 4 events"));
        assert!(page.contains("Claimed once"));
    }

    #[test]
    fn a_held_bead_names_its_stop_and_earlier_attempts_are_listed() {
        let page = bead(
            &with_trail(
                view(),
                r#"[
                  {"id":"1","kind":"claim","occurred_at":"2026-11-09T17:46:07Z"},
                  {"id":"2","kind":"suspend","actor":"rig","occurred_at":"2026-11-09T18:56:31Z","payload":"{\"cause\":\"unknown\",\"stop_code\":\"drift\"}"},
                  {"id":"3","kind":"resume","actor":"owner","occurred_at":"2026-11-09T20:01:06Z"},
                  {"id":"4","kind":"claim","occurred_at":"2026-11-10T09:00:00Z"},
                  {"id":"5","kind":"suspend","actor":"rig","occurred_at":"2026-11-10T09:05:00Z","payload":"{\"stop_code\":\"<b>\"}"}
                ]"#,
            ),
            "fx-1",
        )
        .unwrap();
        assert!(page.contains("class=\"stop-note\""), "{page}");
        assert!(page.contains("stop code <strong>&lt;b&gt;</strong>"));
        assert!(page.contains("<li class=\"stage stopped\">"));
        assert!(page.contains("Stopped before Started"));
        assert!(page.contains("Earlier attempts · 1"));
        assert!(page.contains("Resumed <time"));
        assert!(page.contains("Claimed 2 times"));
    }

    #[test]
    fn attempts_show_ids_usage_and_the_feed_attempt_they_join() {
        let page = bead(&crate::testing::work_view(), "wk-2").unwrap();
        assert!(page.contains("<h3>Attempts · 2</h3>"), "{page}");
        assert!(page.contains("<strong>Attempt 1</strong> <code>att-0002-1</code>"));
        assert!(page.contains("<strong>Attempt 2</strong> <code>att-0002-2</code>"));
        assert!(page.contains("<dt>Cost</dt><dd>$2.10</dd>"));
        assert!(page.contains("<dt>Cost</dt><dd>$3.27</dd>"));
        assert!(page.contains("<dt>Gates reused</dt><dd>1 · saved 5m 0s</dd>"));
        assert!(page.contains("href=\"/run/run%2D0002#attempt-1\""));
        assert!(page.contains("href=\"/run/run%2D0002#attempt-2\""));
        assert!(page.contains("<dt>Retried because</dt><dd>gate-failed</dd>"));
        assert!(
            page.contains("<span class=\"badge bad\">gate</span>"),
            "failed gate step"
        );
        assert!(page.contains("<small class=\"stage-detail\">rig rigA · generation 2</small>"));
        assert!(page.contains("<small class=\"stage-detail\">formula build</small>"));
    }

    #[test]
    fn usage_without_dollars_is_unknown_never_zero() {
        let mut view = with_trail(
            view(),
            r#"[
              {"id":"1","kind":"claim","occurred_at":"2026-11-15T01:00:00Z"},
              {"id":"2","kind":"used","occurred_at":"2026-11-15T02:00:00Z","payload":"{\"tokens\":5000}"}
            ]"#,
        );
        view.profile = crate::testing::work_view().profile;
        let page = bead(&view, "fx-1").unwrap();
        assert!(
            page.contains("<dt>Cost</dt><dd><span class=\"muted\">unknown</span></dd>"),
            "{page}"
        );
        assert!(page.contains("<dd title=\"5,000\">"), "{page}");
        assert!(
            page.contains(
                "<dt>Ledger usage</dt><dd><strong><span class=\"muted\">unknown</span></strong>"
            ),
            "{page}"
        );
        assert!(!page.contains("$0.00"), "{page}");
    }

    #[test]
    fn cost_sources_are_labelled_and_never_added() {
        let view = crate::testing::work_view();
        let page = bead(&view, "wk-2").unwrap();
        let cost = &page[page.find("<section id=\"cost\">").expect("a cost section")..];
        let cost = &cost[..cost.find("</section>").unwrap()];
        // Provenance: 2.10 + 3.27; feed: run-0002 5.37 + run-0004 0.64.
        assert!(
            cost.contains("<dt>Ledger usage</dt><dd><strong>$5.37</strong>"),
            "{cost}"
        );
        assert!(cost.contains("Source: the provenance log's <code>used</code> events"));
        assert!(
            cost.contains("<dt>Run feed</dt><dd><strong>$6.01</strong>"),
            "{cost}"
        );
        assert!(cost.contains("Source: the run feed's cost for 2 joined runs"));
        assert!(!cost.contains("$11.38"), "the sources are never summed");
        // A run doc whose costs are incomplete makes the feed figure a floor.
        let held = bead(&view, "wk-3").unwrap();
        assert!(held.contains("<strong>at least $0.88</strong>"), "{held}");
        // No profile and no run feed: no cost section.
        let mut plain = view.clone();
        plain.profile = std::sync::Arc::new(crate::profile::Profile::default());
        plain.runs = None;
        assert!(!bead(&plain, "wk-2").unwrap().contains("id=\"cost\""));
    }
}
