// Owned by WP5 — Runs. Frozen signatures: `boards(&View) -> String`,
// `board(&View, &str) -> Option<String>`.
//! `/boards` and `/boards/{name}`: the run feed's boards, rendered
//! generically unless their `source_schema` is known.

use super::placeholder;
use crate::model::View;

/// `/boards`. Placeholder page.
pub(crate) fn boards(view: &View) -> String {
    placeholder(view, "Boards", "/boards", "WP5")
}

/// `/boards/{name}`; `None` for a board the feed does not name.
/// Placeholder: a page for any name.
pub(crate) fn board(view: &View, name: &str) -> Option<String> {
    Some(placeholder(view, name, "/boards", "WP5"))
}
