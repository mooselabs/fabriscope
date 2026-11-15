// Owned by WP6 — Held, costs, receipts, sweeper. Frozen signature:
// `receipts(&View, &Facets) -> String`.
//! `/receipts`: receipt rows by kind, joined to their subjects and attempts
//! (`Derived::receipts_by_subject`, `receipts_by_attempt`).

use super::placeholder;
use crate::model::View;
use crate::query::Facets;

/// `/receipts`. Placeholder page.
pub(crate) fn receipts(view: &View, _facets: &Facets) -> String {
    placeholder(view, "Receipts", "/receipts", "WP6")
}
