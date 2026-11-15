// Owned by WP3 — History, activity, people. Frozen signature:
// `activity(&View, &Facets) -> String`.
//! `/activity`: history events across beads, newest first, filtered by the
//! shared facets and date range.

use super::placeholder;
use crate::model::View;
use crate::query::Facets;

/// `/activity`. Placeholder page.
pub(crate) fn activity(view: &View, _facets: &Facets) -> String {
    placeholder(view, "Activity", "/activity", "WP3")
}
