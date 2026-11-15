// Owned by WP2 — Bead page. Frozen signatures:
// `banner(&View, &Bead) -> String`, `panels(&View, &Bead) -> String`.
//! The profile's typed panels on a bead's page (fields shown with
//! `render::values::value`), the held banner (why, next, commands), and the
//! attempts with their usage, joined to runs and receipts by attempt id.

use crate::beads::Bead;
use crate::model::View;

/// The banner for a bead in a state with a `banner` template (a held
/// packet, say). Placeholder: empty.
pub(crate) fn banner(_view: &View, _bead: &Bead) -> String {
    String::new()
}

/// Every `[[panels]]` whose `when` matches this bead. Placeholder: empty.
pub(crate) fn panels(_view: &View, _bead: &Bead) -> String {
    String::new()
}
