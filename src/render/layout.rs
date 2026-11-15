//! The page shell: header, navigation, alerts and footer.

use super::fmt::short;
use super::{escape, heading};
use crate::model::View;
use std::fmt::Write;

pub(crate) fn page(view: &View, title: &str, tab: &str, body: &str) -> String {
    let mut nav = String::new();
    let mut tabs = vec![("/", "Overview")];
    if view.beads.is_some() {
        tabs.extend([("/beads", "Beads"), ("/epics", "Epics")]);
    }
    if view.scrolls.is_some() {
        tabs.push(("/scrolls", "Scrolls"));
    }
    for (path, text) in tabs {
        let current = if tab == path {
            " aria-current=\"page\""
        } else {
            ""
        };
        write!(nav, "<a href=\"{path}\"{current}>{text}</a>").unwrap();
    }
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
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} · {}</title><script src=\"/theme.js\"></script><link rel=\"stylesheet\" href=\"/app.css\"><script defer src=\"/app.js\"></script></head><body><a class=\"skip\" href=\"#content\">Skip to content</a><header><a class=\"brand\" href=\"/\"><span class=\"brand-mark\">F</span>Fabriscope{store}</a><nav aria-label=\"Main navigation\">{nav}</nav><div class=\"theme-control\" hidden><label for=\"color-theme\">Theme</label><select id=\"color-theme\"><option value=\"system\">System</option><option value=\"light\">Light</option><option value=\"dark\">Dark</option></select></div><span id=\"connection\" role=\"status\">Connecting…</span></header><main id=\"content\" data-revision=\"{}\">{alerts}{body}<footer>{}<br>Refreshes automatically · read-only</footer></main></body></html>",
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
