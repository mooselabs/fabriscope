//! Where a bead is up to, from its provenance log: the stage bar, the
//! compact bar in tables, and the progress section of a bead's page.

use super::escape;
use super::fmt::{when, words};
use crate::beads::Bead;
use crate::model::View;
use crate::provenance::{self, Fact, Mark, Note, Progress, Stage};
use std::fmt::Write;

fn fact(fact: &Fact) -> String {
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
/// on hover and to screen readers.
fn stage_bar(stages: &[Stage], compact: bool) -> String {
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
        let facts: String = stage.facts.iter().map(fact).collect();
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

fn kind_words(kind: &str) -> String {
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
    Some((stage_bar(&p.stages, true), escape(&line)))
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
    out.push_str(&stage_bar(&p.stages, false));
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
                stage_bar(&attempt.stages, true)
            )
            .unwrap();
        }
        out.push_str("</ul></details>");
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
}
