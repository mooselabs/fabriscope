// Owned by WP5 — Runs. Frozen signatures:
// `runs(&View, &Facets) -> String`, `run(&View, &str) -> Option<String>`,
// `doc(&View, &str, &str, usize) -> Option<String>`.
//! `/runs`, `/run/{id}` and `/run/{id}/doc/{doc}?page=N`: the run feed's
//! runs, one run's attempts and steps, and a document page (transcript
//! turns always escaped). Run docs and pages come from `View::run_cache`
//! and are never waited for.

use super::placeholder;
use crate::model::View;
use crate::query::Facets;

/// `/runs`. Placeholder page.
pub(crate) fn runs(view: &View, _facets: &Facets) -> String {
    placeholder(view, "Runs", "/runs", "WP5")
}

/// `/run/{id}`; `None` for a run the index does not list. Placeholder: a
/// page for any id.
pub(crate) fn run(view: &View, id: &str) -> Option<String> {
    Some(placeholder(view, id, "/runs", "WP5"))
}

/// `/run/{id}/doc/{doc}`, page `page` (0-based). Placeholder: a page for
/// any document.
pub(crate) fn doc(view: &View, run: &str, doc: &str, _page: usize) -> Option<String> {
    Some(placeholder(view, &format!("{run} · {doc}"), "/runs", "WP5"))
}
