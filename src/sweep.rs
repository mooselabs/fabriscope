// Owned by WP6 — Held, costs, receipts, sweeper. Frozen signature:
// `pub fn spawn(app: App)`.
//! Reads provenance (and history) ahead for the beads the factory pages
//! total: held packets and recently closed ones, with at most two reads
//! outstanding, through `App::want_provenance` and
//! `history::reader::want`.

use crate::app::App;

/// Starts the sweeper. Called once at startup when a local `bd` can read
/// provenance. Placeholder: does nothing yet.
pub fn spawn(_app: App) {}
