// Owned by WP4 — Flow. Frozen signature:
// `flow(&View, FlowPage, &Facets) -> String`.
//! The flow pages: created and closed per day, claims, cycle time (p50/p90,
//! nearest rank), ageing of open work, and open decisions.

use super::placeholder;
use crate::model::View;
use crate::query::Facets;

/// Which flow page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowPage {
    Overview,
    Created,
    Claimed,
    Cycle,
    Ageing,
    Decisions,
}

impl FlowPage {
    /// The page's path and title.
    pub(crate) fn path_and_title(self) -> (&'static str, &'static str) {
        match self {
            FlowPage::Overview => ("/flow", "Flow"),
            FlowPage::Created => ("/flow/created", "Created"),
            FlowPage::Claimed => ("/flow/claimed", "Claimed"),
            FlowPage::Cycle => ("/flow/cycle", "Cycle time"),
            FlowPage::Ageing => ("/flow/ageing", "Ageing"),
            FlowPage::Decisions => ("/flow/decisions", "Decisions"),
        }
    }
}

/// One flow page. Placeholder page.
pub(crate) fn flow(view: &View, page: FlowPage, _facets: &Facets) -> String {
    let (path, title) = page.path_and_title();
    placeholder(view, title, path, "WP4")
}
