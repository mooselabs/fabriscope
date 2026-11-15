//! Server-rendered HTML. Every value from a store is escaped; Markdown is
//! rendered without raw HTML or images, and links only ever point at external
//! web pages or at other records on this server.
//!
//! Each page lives in its own module, owned by one work package (see the
//! comment at the top of each file); the helpers every page shares live
//! here and in `fmt`, `values`, `svg` and `layout`, all `pub(crate)` so any
//! page module or `web::*` handler can call them.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "contract the packages call; helpers WP1–WP6 call (BKLG-20261115-4e2r)"
    )
)]

pub(crate) mod activity;
pub(crate) mod bead;
pub(crate) mod beads;
pub(crate) mod boards;
pub(crate) mod costs;
pub(crate) mod flow;
pub(crate) mod fmt;
pub(crate) mod groups;
pub(crate) mod held;
pub(crate) mod layout;
pub(crate) mod markdown;
pub(crate) mod overview;
pub(crate) mod panels;
pub(crate) mod people;
pub(crate) mod progress;
pub(crate) mod receipts;
pub(crate) mod runs;
pub(crate) mod scrolls;
pub(crate) mod search;
pub(crate) mod svg;
pub(crate) mod table;
pub(crate) mod timeline;
pub(crate) mod values;

pub use bead::bead;
pub use beads::{beads, epics};
pub use layout::not_found;
pub use overview::overview;
pub use scrolls::{scroll, scrolls};

use crate::beads::Bead;
use crate::model::View;
use crate::profile::{RowFacts, Tone};
use crate::query::Facets;
use fmt::words;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use std::collections::BTreeMap;
use std::fmt::Write;

/// Rows per page on every paged list.
pub(crate) const PAGE_SIZE: usize = 100;

/// HTML-escapes text for element content and attribute values.
pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Percent-encodes one path segment or query value.
pub(crate) fn url(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/// A link to a bead's page.
pub(crate) fn bead_link(id: &str, title: &str) -> String {
    format!("<a href=\"/bead/{}\">{}</a>", url(id), escape(title))
}

/// A link to a scroll's page.
pub(crate) fn scroll_link(id: &str, title: &str) -> String {
    format!("<a href=\"/scroll/{}\">{}</a>", url(id), escape(title))
}

/// A link to a person's page (`/person/{name}`), with the name as text.
pub(crate) fn person_link(name: &str) -> String {
    format!("<a href=\"/person/{}\">{}</a>", url(name), escape(name))
}

/// A link to one value of a profile group (`/by/{group}/{value}`).
pub(crate) fn group_link(group: &str, value: &str, text: &str) -> String {
    format!(
        "<a href=\"/by/{}/{}\">{}</a>",
        url(group),
        url(value),
        escape(text)
    )
}

/// A link to a run's page (`/run/{id}`).
pub(crate) fn run_link(run: &str, text: &str) -> String {
    format!("<a href=\"/run/{}\">{}</a>", url(run), escape(text))
}

/// The CSS class for a tone: badges, banners, facts and chart marks all
/// colour by it (`good` green, `warn` and `waiting` amber, `bad` red,
/// `neutral` grey).
pub(crate) fn tone_class(tone: Tone) -> &'static str {
    match tone {
        Tone::Good => "good",
        Tone::Warn => "warn",
        Tone::Bad => "bad",
        Tone::Waiting => "waiting",
        Tone::Neutral => "neutral",
    }
}

/// A badge with this text in this tone.
pub(crate) fn tone_badge(text: &str, tone: Tone) -> String {
    format!(
        "<span class=\"badge {}\">{}</span>",
        tone_class(tone),
        escape(text)
    )
}

/// One badge per derived state of a row, labelled and toned by the
/// profile; empty when the row is in no state.
pub(crate) fn state_badges(view: &View, row: &RowFacts) -> String {
    row.states
        .iter()
        .filter_map(|key| view.profile.state(key))
        .map(|state| tone_badge(&state.label, state.tone))
        .collect()
}

/// A bead's status exactly as Beads stores it. The colour follows the
/// category `bd statuses` gives each built-in status; a custom status is
/// shown in the neutral colour.
pub(crate) fn badge(status: &str) -> String {
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
pub(crate) const STATUS_ORDER: [&str; 7] = [
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
pub(crate) fn tally<'a>(
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
pub(crate) fn status_options<'a>(beads: impl Iterator<Item = &'a Bead>) -> Vec<(String, String)> {
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

pub(crate) fn selected(on: bool) -> &'static str {
    if on { " selected" } else { "" }
}

/// The page heading: a small eyebrow line, the title and a lede.
pub(crate) fn heading(eyebrow: &str, title: &str, description: &str) -> String {
    format!(
        "<div class=\"page-heading\"><p class=\"eyebrow\">{}</p><h1>{}</h1><p class=\"lede\">{}</p></div>",
        escape(eyebrow),
        escape(title),
        escape(description)
    )
}

/// A row of metric cards: (label, value, detail, href).
pub(crate) fn metrics(cards: &[(&str, usize, &str, String)]) -> String {
    let mut out = "<div class=\"metrics\">".to_string();
    for (label, value, detail, href) in cards {
        write!(out, "<a class=\"metric\" href=\"{}\"><span>{}</span><strong>{value}</strong><small>{}</small></a>", escape(href), escape(label), escape(detail)).unwrap();
    }
    out.push_str("</div>");
    out
}

/// A labelled `<select>` named `name`, with `current` selected; options
/// are (value, text).
pub(crate) fn select(
    name: &str,
    label: &str,
    current: &str,
    options: &[(String, String)],
) -> String {
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

/// Previous/next links for page `current` of `pages`, carrying `query`.
pub(crate) fn pagination(
    path: &str,
    query: &[(&str, &str)],
    current: usize,
    pages: usize,
) -> String {
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

/// The shared filter form for list pages: search, state, type, kind,
/// derived state, person, epic and one select per profile group, each with
/// the values the store holds. Submits to `path` with GET, so the filters
/// live in the URL.
pub(crate) fn facets_form(view: &View, facets: &Facets, path: &str) -> String {
    let mut out = format!(
        "<form class=\"filters\" action=\"{}\" method=\"get\"><label>Search<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"Title, id or label…\"></label>",
        escape(path),
        escape(&facets.q)
    );
    let Some(s) = view.snapshot() else {
        out.push_str("<button type=\"submit\">Apply</button></form>");
        return out;
    };
    let items = || {
        s.beads
            .iter()
            .enumerate()
            .filter(|(i, _)| s.derived.is_item(*i))
            .map(|(_, b)| b)
    };
    out.push_str(&select(
        "state",
        "State",
        &facets.state,
        &status_options(items()),
    ));
    let mut types = vec![(String::new(), "All types".to_string())];
    types.extend(
        tally(items(), |b| b.issue_type.as_str())
            .into_iter()
            .map(|(t, n)| (t.to_string(), format!("{} ({n})", words(t)))),
    );
    out.push_str(&select("type", "Type", &facets.issue_type, &types));
    let all = |label: &str| vec![(String::new(), label.to_string())];
    if !view.profile.kinds.is_empty() {
        let mut kinds = all("All kinds");
        kinds.extend(
            view.profile
                .kinds
                .iter()
                .map(|k| (k.key.clone(), k.label.clone())),
        );
        out.push_str(&select("kind", "Kind", &facets.kind, &kinds));
    }
    if !view.profile.states.is_empty() {
        let mut states = all("Any");
        states.extend(
            view.profile
                .states
                .iter()
                .map(|k| (k.key.clone(), k.label.clone())),
        );
        out.push_str(&select("dstate", "Derived state", &facets.dstate, &states));
    }
    if !s.derived.people.is_empty() {
        let mut people = all("Anyone");
        people.extend(s.derived.people.keys().map(|p| (p.clone(), p.clone())));
        out.push_str(&select("person", "Person", &facets.person, &people));
    }
    let mut epics = all("Any epic");
    epics.extend(
        s.ordered()
            .filter(|b| b.is_epic())
            .map(|b| (b.id.clone(), b.title.clone())),
    );
    if epics.len() > 1 {
        out.push_str(&select("epic", "Epic", &facets.epic, &epics));
    }
    for group in &view.profile.groups {
        let mut values = all(&format!("Any {}", group.label.to_lowercase()));
        if let Some(index) = s.derived.groups.get(&group.key) {
            values.extend(
                index
                    .iter()
                    .map(|(v, members)| (v.clone(), format!("{v} ({})", members.len()))),
            );
        }
        let current = facets
            .groups
            .get(&group.key)
            .map(String::as_str)
            .unwrap_or("");
        out.push_str(&select(&group.key, &group.label, current, &values));
    }
    out.push_str("<button type=\"submit\">Apply</button></form>");
    out
}

/// A placeholder page for a route whose package has not landed yet: the
/// shell, a heading, and one line saying who builds it.
pub(crate) fn placeholder(view: &View, title: &str, tab: &str, owner: &str) -> String {
    layout::page(
        view,
        title,
        tab,
        &format!(
            "{}<p class=\"placeholder muted\">This page is being built ({}).</p>",
            heading("Coming soon", title, ""),
            escape(owner)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beads::{self, Bead};
    use crate::model::{BeadFilter, Feed, ScrollFilter, View};
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
            ..Default::default()
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
        assert!(page.contains("href=\"/beads\""), "the Work section");
        assert!(beads(&view, &BeadFilter::default()).contains("href=\"/epics\""));
        assert!(scrolls(&view, &ScrollFilter::default()).contains("404"));
    }

    #[test]
    fn shared_helpers_link_badge_and_build_the_filter_form() {
        let view = crate::testing::work_view();
        let s = view.snapshot().unwrap();
        let held = &s.derived.rows[s.position("wk-3").unwrap()];
        assert_eq!(
            state_badges(&view, held),
            "<span class=\"badge bad\">Held</span>"
        );
        assert_eq!(
            group_link("milestone", "v 1", "<v1>"),
            "<a href=\"/by/milestone/v%201\">&lt;v1&gt;</a>"
        );
        assert_eq!(
            run_link("run-0002", "Run 2"),
            "<a href=\"/run/run%2D0002\">Run 2</a>"
        );
        let form = facets_form(&view, &crate::query::Facets::default(), "/beads");
        for wanted in [
            "name=\"dstate\"",
            "name=\"kind\"",
            "name=\"milestone\"",
            "<option value=\"v1\">v1 (3)</option>",
            "name=\"person\"",
            "name=\"epic\"",
        ] {
            assert!(form.contains(wanted), "{wanted}: {form}");
        }
        assert!(
            !form.contains("work-receipt"),
            "receipts are not offered as a type"
        );
    }
}
