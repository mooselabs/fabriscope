// Owned by WP3 — History, activity, people. Frozen signature:
// `pub fn spawn(app: App, reader: &beads::Reader)` and
// `pub async fn want(app: &App, ids: Vec<String>)`.
//! Reads beads' history threads in the background into `View::history`.
//!
//! WP3 fills these bodies: a bulk read (`bd export` carries `comments` on
//! every issue that has any; `Bead::comments` already holds them after a CLI
//! read) and a per-issue read for API stores
//! (`/v0/beads/issues/{id}?include_comments=true`), recorded with
//! `history::Store::record`, bumping the revision on change like
//! `app::read_provenance` does.

use crate::app::App;
use crate::beads;

/// Starts the background history reads for `reader`'s store. Called once
/// at startup. Placeholder: reads nothing yet.
pub fn spawn(_app: App, _reader: &beads::Reader) {}

/// Asks for these beads' threads to be read soon (a page about to show
/// them). Never waits for the read. Placeholder: does nothing yet.
pub async fn want(_app: &App, _ids: Vec<String>) {}
