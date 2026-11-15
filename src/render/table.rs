//! The bead table every list of beads shares.

use super::progress::compact;
use super::{badge, bead_link, escape};
use crate::beads::{self, Bead};
use crate::provenance;
use std::fmt::Write;

pub(crate) fn bead_table(
    s: &beads::Snapshot,
    trails: Option<&provenance::Store>,
    rows: &[(usize, &Bead)],
) -> String {
    if rows.is_empty() {
        return "<p class=\"empty\">No beads match this view.</p>".into();
    }
    let mut out = "<div class=\"table-wrap\"><table class=\"items\"><thead><tr><th>#</th><th>Bead</th><th>State</th><th>Priority</th></tr></thead><tbody>".to_string();
    for (position, bead) in rows {
        let parent = bead
            .parent()
            .and_then(|p| s.get(p))
            .map(|p| format!(" · {}", escape(&p.title)))
            .unwrap_or_default();
        let reference = if bead.external_ref.is_empty() {
            String::new()
        } else {
            format!(" · {}", escape(&bead.external_ref))
        };
        let stage = if bead.status == "in_progress" {
            trails
                .and_then(|t| compact(t, bead, chrono::Utc::now()))
                .map(|(bar, now)| {
                    format!("<div class=\"stage-mini\">{bar}<small>{now}</small></div>")
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        write!(out, "<tr><td class=\"muted\">{position}</td><td>{}<small>{}{reference} · {}{parent}</small></td><td>{}{stage}</td><td>P{}</td></tr>", bead_link(&bead.id, &bead.title), escape(&bead.id), escape(&bead.issue_type), badge(&bead.status), bead.priority).unwrap();
    }
    out.push_str("</tbody></table></div>");
    out
}
