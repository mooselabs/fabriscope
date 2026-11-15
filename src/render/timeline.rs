// Owned by WP2 — Bead page. Frozen signature:
// `timeline(&View, &Bead) -> String`.
//! A bead's merged timeline: its dates, history events, provenance events
//! and run-feed steps in one time order.

use crate::beads::Bead;
use crate::model::View;

/// The timeline section of a bead's page. Placeholder: empty, so the page
/// shows nothing new.
pub(crate) fn timeline(_view: &View, _bead: &Bead) -> String {
    String::new()
}
