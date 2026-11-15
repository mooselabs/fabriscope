//! Server-rendered HTML. Every value from a store is escaped; Markdown is
//! rendered without raw HTML or images, and links only ever point at external
//! web pages or at other records on this server.
//!
//! Each page lives in its own module; the helpers every page shares live
//! here.

mod bead;
mod beads;
mod fmt;
mod layout;
mod markdown;
mod overview;
mod progress;
mod scrolls;
mod svg;
mod table;

pub use bead::bead;
pub use beads::{beads, epics};
pub use layout::not_found;
pub use overview::overview;
pub use scrolls::{scroll, scrolls};

use crate::beads::Bead;
use fmt::words;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use std::collections::BTreeMap;
use std::fmt::Write;

const PAGE_SIZE: usize = 100;

pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn url(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

fn bead_link(id: &str, title: &str) -> String {
    format!("<a href=\"/bead/{}\">{}</a>", url(id), escape(title))
}

fn scroll_link(id: &str, title: &str) -> String {
    format!("<a href=\"/scroll/{}\">{}</a>", url(id), escape(title))
}

/// A bead's status exactly as Beads stores it. The colour follows the
/// category `bd statuses` gives each built-in status; a custom status is
/// shown in the neutral colour.
fn badge(status: &str) -> String {
    let class = match status {
        "open" => "open",
        "in_progress" | "hooked" => "working",
        "blocked" => "waiting",
        "closed" => "done",
        _ => "neutral",
    };
    format!(
        "<span class=\"badge {class}\">{}</span>",
        escape(&words(status))
    )
}

/// Built-in Beads statuses in the order `bd statuses` lists them.
const STATUS_ORDER: [&str; 7] = [
    "open",
    "in_progress",
    "blocked",
    "deferred",
    "closed",
    "pinned",
    "hooked",
];

/// Each distinct value of `field` among `beads`, with how many beads hold
/// it, so a filter offers exactly what the store contains. Statuses keep the
/// Beads order; anything else, and any custom status, sorts by name.
fn tally<'a>(
    beads: impl Iterator<Item = &'a Bead>,
    field: impl Fn(&'a Bead) -> &'a str,
) -> Vec<(&'a str, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for bead in beads {
        let value = field(bead);
        if !value.is_empty() {
            *counts.entry(value).or_default() += 1;
        }
    }
    let mut out: Vec<_> = counts.into_iter().collect();
    out.sort_by_key(|(value, _)| {
        (
            STATUS_ORDER
                .iter()
                .position(|s| s == value)
                .unwrap_or(STATUS_ORDER.len()),
            *value,
        )
    });
    out
}

/// Filter options for every stored status: beads not closed (the default),
/// all beads, then each status the store holds.
fn status_options<'a>(beads: impl Iterator<Item = &'a Bead>) -> Vec<(String, String)> {
    let mut options = vec![
        (String::new(), "Not closed".to_string()),
        ("all".to_string(), "All".to_string()),
    ];
    options.extend(
        tally(beads, |b| b.status.as_str())
            .into_iter()
            .map(|(status, n)| (status.to_string(), format!("{} ({n})", words(status)))),
    );
    options
}

fn selected(on: bool) -> &'static str {
    if on { " selected" } else { "" }
}

fn heading(eyebrow: &str, title: &str, description: &str) -> String {
    format!(
        "<div class=\"page-heading\"><p class=\"eyebrow\">{}</p><h1>{}</h1><p class=\"lede\">{}</p></div>",
        escape(eyebrow),
        escape(title),
        escape(description)
    )
}

fn metrics(cards: &[(&str, usize, &str, String)]) -> String {
    let mut out = "<div class=\"metrics\">".to_string();
    for (label, value, detail, href) in cards {
        write!(out, "<a class=\"metric\" href=\"{}\"><span>{}</span><strong>{value}</strong><small>{}</small></a>", escape(href), escape(label), escape(detail)).unwrap();
    }
    out.push_str("</div>");
    out
}

fn select(name: &str, label: &str, current: &str, options: &[(String, String)]) -> String {
    let mut out = format!("<label>{}<select name=\"{name}\">", escape(label));
    for (value, text) in options {
        write!(
            out,
            "<option value=\"{}\"{}>{}</option>",
            escape(value),
            selected(current == value),
            escape(text)
        )
        .unwrap();
    }
    out.push_str("</select></label>");
    out
}

fn pagination(path: &str, query: &[(&str, &str)], current: usize, pages: usize) -> String {
    let mut out = "<nav class=\"pagination\" aria-label=\"Results pages\">".to_string();
    let base: String = query
        .iter()
        .map(|(k, v)| format!("{k}={}&amp;", url(v)))
        .collect();
    for (label, target) in [
        ("← Previous", current.checked_sub(1)),
        ("Next →", (current + 1 < pages).then_some(current + 1)),
    ] {
        if let Some(target) = target {
            write!(out, "<a href=\"{path}?{base}page={target}\">{label}</a>").unwrap();
        }
    }
    out.push_str("</nav>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beads::{self, Bead};
    use crate::model::{Feed, ScrollFilter, View};
    use crate::provenance;
    use crate::scrolls::{self, Scroll};
    use std::sync::Arc;

    pub(super) fn view() -> View {
        let mut bead = Bead {
            id: "fx-1".into(),
            title: "Fix login".into(),
            status: "open".into(),
            external_ref: "TICKET-12".into(),
            ..Default::default()
        };
        bead.created_at = Some("2026-10-01T00:00:00Z".into());
        let beads = beads::Snapshot::new(
            vec![bead],
            vec!["fx-1".into()],
            Some("abcdef0123456789".into()),
            chrono::Utc::now(),
        );
        let records = vec![
            Scroll {
                id: "sc-a".into(),
                kind: "decision".into(),
                title: "Use Dolt".into(),
                path: Some("decisions/a.md".into()),
                ..Default::default()
            },
            Scroll {
                id: "sc-b".into(),
                kind: "note".into(),
                title: "Notes".into(),
                path: Some("notes/b.md".into()),
                ..Default::default()
            },
        ];
        let edges = vec![scrolls::Edge {
            from: "sc-a".into(),
            rel: "related_work".into(),
            to: "beads://store/fx-1".into(),
            why: None,
        }];
        View {
            title: "demo".into(),
            beads: Some(Feed {
                data: Arc::new(beads),
                error: None,
            }),
            scrolls: Some(Feed {
                data: Arc::new(scrolls::Snapshot::new(records, edges, Some("rev".into()))),
                error: None,
            }),
            revision: 3,
            scrolls_checked_at: None,
            provenance: None,
        }
    }

    pub(super) fn with_trail(mut view: View, events: &str) -> View {
        let mut store = provenance::Store::default();
        store.record("fx-1", provenance::parse(events), None, chrono::Utc::now());
        view.provenance = Some(store);
        view
    }

    #[test]
    fn pages_show_both_stores_and_cross_links() {
        let view = view();
        let page = overview(&view);
        assert!(page.contains("Fabriscope <span class=\"muted\">/ demo</span>"));
        assert!(page.contains("Beads · commit abcdef012"));
        let mut served = view.clone();
        let mut data = (*served.beads.as_ref().unwrap().data).clone();
        data.source = "bd serve http://127.0.0.1:9".into();
        served.beads = Some(Feed {
            data: Arc::new(data),
            error: None,
        });
        assert!(
            overview(&served).contains("Beads via bd serve http://127.0.0.1:9 · commit abcdef012")
        );
        assert!(page.contains("Scrolls · revision rev"));
        assert!(page.contains("/bead/fx%2D1"));
        let bead = bead(&view, "fx-1").unwrap();
        assert!(bead.contains("Scrolls about this bead"));
        assert!(bead.contains("/scroll/sc%2Da"));
        let scroll = scroll(&view, "sc-a").unwrap();
        assert!(scroll.contains("bead fx-1"));
        assert!(bead_page_absent(&view));
    }

    fn bead_page_absent(view: &View) -> bool {
        bead(view, "nope").is_none() && scroll(view, "nope").is_none()
    }

    #[test]
    fn a_beads_only_view_hides_scroll_navigation() {
        let mut view = view();
        view.scrolls = None;
        let page = overview(&view);
        assert!(!page.contains("href=\"/scrolls\""));
        assert!(page.contains("href=\"/epics\""));
        assert!(scrolls(&view, &ScrollFilter::default()).contains("404"));
    }
}
