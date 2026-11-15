// Owned by WP5 — Runs. Frozen signature: `pub fn spawn(app: App, source:
// runs::Source)`.
//! Keeps `View::runs` current from the feed and fills `View::run_cache`.
//!
//! WP5 fills this: poll `feed.json` every 30 s, reread the index when the
//! revision moves (`runs::load` already reads a directory; add HTTP reads in
//! `Source::read`), store it with `App::update`, and serve cache misses.
//! Until then `View::runs` stays the empty index main.rs starts with.

use super::Source;
use crate::app::App;

/// Starts keeping the run feed current. Called once at startup when
/// `--runs` is given. Placeholder: does nothing yet.
pub fn spawn(_app: App, _source: Source) {}
