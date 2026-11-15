// Owned by WP6 — Held, costs, receipts, sweeper. Frozen signature:
// `held(&View, &Facets) -> String`.
//! `/held`: beads in a state that holds them back, with why, since when and
//! what to do next.

use super::placeholder;
use crate::model::View;
use crate::query::Facets;

/// `/held`. Placeholder page.
pub(crate) fn held(view: &View, _facets: &Facets) -> String {
    placeholder(view, "Held", "/held", "WP6")
}
