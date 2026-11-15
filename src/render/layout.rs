//! The page shell: header, navigation, alerts and footer.

use super::fmt::short;
use super::{escape, heading};
use crate::model::View;
use std::fmt::Write;

/// One section of the main navigation and the tabs under it.
pub(crate) struct Section {
    pub label: &'static str,
    /// (path, label) of each tab shown, in order.
    pub tabs: Vec<(String, &'static str)>,
}

/// The navigation for this view. A section or tab appears only when its
/// data source or profile section exists: Work and Flow with a Beads store;
/// Groups with any `[[groups]]`; Held when a state holds beads back; Runs
/// and Boards with `--runs`; Receipts with `[[receipts]]`; Knowledge with a
/// Scrolls store.
pub(crate) fn sections(view: &View) -> Vec<Section> {
    let p = &view.profile;
    let beads = view.beads.is_some();
    let runs = view.runs.is_some();
    let tab = |path: &str, label| (path.to_string(), label);
    let mut out = vec![Section {
        label: "Overview",
        tabs: vec![tab("/", "Overview")],
    }];
    if beads {
        let mut work = vec![tab("/beads", "Beads"), tab("/epics", "Epics")];
        if let Some(group) = p.groups.first() {
            work.push((format!("/by/{}", super::url(&group.key)), "Groups"));
        }
        work.extend([tab("/people", "People"), tab("/activity", "Activity")]);
        out.push(Section {
            label: "Work",
            tabs: work,
        });
        out.push(Section {
            label: "Flow",
            tabs: vec![
                tab("/flow", "Overview"),
                tab("/flow/created", "Created"),
                tab("/flow/claimed", "Claimed"),
                tab("/flow/cycle", "Cycle"),
                tab("/flow/ageing", "Ageing"),
                tab("/flow/decisions", "Decisions"),
            ],
        });
    }
    let mut factory = Vec::new();
    if beads && p.has_held() {
        factory.push(tab("/held", "Held"));
    }
    if runs {
        factory.push(tab("/runs", "Runs"));
    }
    if beads || runs {
        factory.push(tab("/costs", "Costs"));
    }
    if beads && !p.receipts.is_empty() {
        factory.push(tab("/receipts", "Receipts"));
    }
    if runs {
        factory.push(tab("/boards", "Boards"));
    }
    if !factory.is_empty() {
        out.push(Section {
            label: "Factory",
            tabs: factory,
        });
    }
    if view.scrolls.is_some() {
        out.push(Section {
            label: "Knowledge",
            tabs: vec![tab("/scrolls", "Scrolls")],
        });
    }
    out
}

/// Whether the page whose tab is `tab` sits under the nav tab at `path`:
/// the same path or one below it. Every `/by/…` page belongs to Groups.
fn under(tab: &str, path: &str) -> bool {
    if path == "/" {
        return tab == "/";
    }
    if path.starts_with("/by/") {
        return tab == "/by" || tab.starts_with("/by/");
    }
    !tab.is_empty() && (tab == path || tab.starts_with(&format!("{path}/")))
}

/// The whole page. `tab` is the path of the tab the page belongs to (`/beads`
/// for a bead, say), which marks the current section and tab; `""` marks
/// none.
pub(crate) fn page(view: &View, title: &str, tab: &str, body: &str) -> String {
    let sections = sections(view);
    let current = sections
        .iter()
        .position(|s| s.tabs.iter().any(|(path, _)| under(tab, path)));
    let mut nav = String::new();
    for (i, section) in sections.iter().enumerate() {
        let here = if current == Some(i) {
            " aria-current=\"page\""
        } else {
            ""
        };
        write!(
            nav,
            "<a href=\"{}\"{here}>{}</a>",
            escape(&section.tabs[0].0),
            section.label
        )
        .unwrap();
    }
    let mut subnav = String::new();
    if let Some(section) = current.map(|i| &sections[i]).filter(|s| s.tabs.len() > 1) {
        write!(
            subnav,
            "<nav class=\"subnav\" aria-label=\"{} pages\">",
            section.label
        )
        .unwrap();
        let best = section
            .tabs
            .iter()
            .filter(|(path, _)| under(tab, path))
            .max_by_key(|(path, _)| path.len())
            .map(|(path, _)| path.clone());
        for (path, label) in &section.tabs {
            let here = if best.as_deref() == Some(path.as_str()) {
                " aria-current=\"page\""
            } else {
                ""
            };
            write!(subnav, "<a href=\"{}\"{here}>{label}</a>", escape(path)).unwrap();
        }
        subnav.push_str("</nav>");
    }
    let search = "<form class=\"site-search\" role=\"search\" action=\"/search\" method=\"get\"><label class=\"sr-only\" for=\"site-search\">Search everything</label><input id=\"site-search\" type=\"search\" name=\"q\" placeholder=\"Search…\"></form>";
    let mut alerts = String::new();
    for error in view.errors() {
        write!(
            alerts,
            "<p class=\"alert\" role=\"alert\">{}</p>",
            escape(error)
        )
        .unwrap();
    }
    let mut provenance = Vec::new();
    if let Some(feed) = &view.beads {
        let commit = feed
            .data
            .commit
            .as_deref()
            .map(|c| format!(" · commit {}", short(c)))
            .unwrap_or_default();
        let via = if feed.data.source.is_empty() {
            String::new()
        } else {
            format!(" via {}", feed.data.source)
        };
        provenance.push(format!(
            "Beads{}{} · read {}",
            escape(&via),
            escape(&commit),
            escape(&feed.data.generated_at)
        ));
    }
    if let Some(feed) = &view.scrolls {
        let revision = feed
            .data
            .revision
            .as_deref()
            .map(|r| format!(" · revision {}", short(r)))
            .unwrap_or_default();
        provenance.push(format!(
            "Scrolls{} · read {}",
            escape(&revision),
            escape(&feed.data.generated_at)
        ));
    }
    if let Some(feed) = &view.runs {
        let index = &feed.data;
        if !index.feed.schema.is_empty() {
            provenance.push(format!(
                "Run feed {} · revision {} · generated {}",
                escape(&index.feed.factory),
                index.feed.revision,
                escape(&index.feed.generated_at)
            ));
        }
    }
    let store = if view.title.is_empty() {
        String::new()
    } else {
        format!(" <span class=\"muted\">/ {}</span>", escape(&view.title))
    };
    let suffix = if view.title.is_empty() {
        "Fabriscope".to_string()
    } else {
        format!("{} · Fabriscope", view.title)
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} · {}</title><script src=\"/theme.js\"></script><link rel=\"stylesheet\" href=\"/app.css\"><script defer src=\"/app.js\"></script></head><body><a class=\"skip\" href=\"#content\">Skip to content</a><header><a class=\"brand\" href=\"/\"><span class=\"brand-mark\">F</span>Fabriscope{store}</a><nav aria-label=\"Main navigation\">{nav}</nav>{search}<div class=\"theme-control\" hidden><label for=\"color-theme\">Theme</label><select id=\"color-theme\"><option value=\"system\">System</option><option value=\"light\">Light</option><option value=\"dark\">Dark</option></select></div><span id=\"connection\" role=\"status\">Connecting…</span></header>{subnav}<main id=\"content\" data-revision=\"{}\">{alerts}{body}<footer>{}<br>Refreshes automatically · read-only</footer></main></body></html>",
        escape(title),
        escape(&suffix),
        view.revision,
        provenance.join("<br>"),
    )
}

pub fn not_found(view: &View) -> String {
    page(
        view,
        "Not found",
        "",
        &heading(
            "404",
            "This isn’t in the current read.",
            "It may have been removed or renamed, or its store isn’t being shown.",
        ),
    )
}
