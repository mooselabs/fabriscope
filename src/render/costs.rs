// Owned by WP6 — Held, costs, receipts, sweeper. Frozen signature:
// `costs(&View, &Facets) -> String`.
//! `/costs`: spend by attempt, run, group and model, from provenance usage
//! and the run feed (see `crate::costs`).

use super::placeholder;
use crate::model::View;
use crate::query::Facets;

/// `/costs`. Placeholder page.
pub(crate) fn costs(view: &View, _facets: &Facets) -> String {
    placeholder(view, "Costs", "/costs", "WP6")
}
