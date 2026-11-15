// Owned by WP1 — Lists, groups, search, overview. Frozen signature:
// `search(&View, &Facets) -> String`.
//! `/search?q=…`: beads, scrolls, people and runs matching the text.

use super::placeholder;
use crate::model::View;
use crate::query::Facets;

/// `/search`. Placeholder page.
pub(crate) fn search(view: &View, _facets: &Facets) -> String {
    placeholder(view, "Search", "", "WP1")
}
