use crate::{beads, scrolls};
use serde::Deserialize;
use std::sync::Arc;

/// The last successful read of one store, and the error from the most recent
/// read if it failed. A failed read never replaces the last good data.
#[derive(Clone, Debug, Default)]
pub struct Feed<T> {
    pub data: Arc<T>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    /// A name for the stores on show, from `--title`.
    pub title: String,
    pub beads: Option<Feed<beads::Snapshot>>,
    pub scrolls: Option<Feed<scrolls::Snapshot>>,
    pub scrolls_checked_at: Option<String>,
    /// Bumped on changed content or health; open pages reload when it changes.
    pub revision: u64,
}

impl View {
    pub fn errors(&self) -> Vec<&str> {
        let beads = self.beads.as_ref().and_then(|f| f.error.as_deref());
        let scrolls = self.scrolls.as_ref().and_then(|f| f.error.as_deref());
        beads.into_iter().chain(scrolls).collect()
    }
}

fn contains(haystack: &[&str], needle: &str) -> bool {
    let needle = needle.to_lowercase();
    haystack.iter().any(|h| h.to_lowercase().contains(&needle))
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct BeadFilter {
    pub q: String,
    pub state: String,
    #[serde(rename = "type")]
    pub issue_type: String,
    pub priority: String,
    /// `yes` for beads `bd ready` lists, `no` for the rest, empty for any.
    pub ready: String,
    pub page: usize,
}

impl BeadFilter {
    /// `state` is a status exactly as stored, `all`, or empty for every bead
    /// that is not closed.
    pub fn matches(&self, bead: &beads::Bead) -> bool {
        let state = match self.state.as_str() {
            "" => !bead.is_closed(),
            "all" => true,
            status => bead.status == status,
        };
        let ready = match self.ready.as_str() {
            "yes" => bead.ready,
            "no" => !bead.ready,
            _ => true,
        };
        state
            && ready
            && (self.issue_type.is_empty() || bead.issue_type == self.issue_type)
            && (self.priority.is_empty() || bead.priority.to_string() == self.priority)
            && (self.q.is_empty()
                || contains(
                    &[
                        &bead.id,
                        &bead.title,
                        &bead.external_ref,
                        &bead.labels.join(" "),
                    ],
                    &self.q,
                ))
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct ScrollFilter {
    pub q: String,
    pub kind: String,
    pub status: String,
    pub page: usize,
}

impl ScrollFilter {
    pub fn matches(&self, scroll: &scrolls::Scroll) -> bool {
        (self.kind.is_empty() || scroll.kind == self.kind)
            && (self.status.is_empty() || scroll.status == self.status)
            && (self.q.is_empty()
                || contains(
                    &[
                        &scroll.id,
                        &scroll.title,
                        scroll.path.as_deref().unwrap_or(""),
                        &scroll.aliases.join(" "),
                    ],
                    &self.q,
                ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bead_filter_defaults_to_beads_not_closed() {
        let open = beads::Bead {
            id: "fx-1".into(),
            title: "Fix login".into(),
            status: "open".into(),
            issue_type: "bug".into(),
            priority: 1,
            ..Default::default()
        };
        let closed = beads::Bead {
            id: "fx-2".into(),
            status: "closed".into(),
            ..Default::default()
        };
        let all = BeadFilter {
            state: "all".into(),
            ..Default::default()
        };
        assert!(BeadFilter::default().matches(&open));
        assert!(!BeadFilter::default().matches(&closed));
        assert!(all.matches(&closed));
        // A status filter compares with the stored status, so `open` means
        // status open, not "anything not closed".
        let status = |state: &str| BeadFilter {
            state: state.into(),
            ..Default::default()
        };
        assert!(status("open").matches(&open));
        assert!(!status("open").matches(&closed));
        assert!(status("closed").matches(&closed));
        assert!(!status("in_progress").matches(&open));
        let ready = |ready: &str| BeadFilter {
            ready: ready.into(),
            ..Default::default()
        };
        assert!(ready("no").matches(&open));
        assert!(!ready("yes").matches(&open));
        assert!(ready("yes").matches(&beads::Bead {
            ready: true,
            ..open.clone()
        }));
        assert!(
            BeadFilter {
                q: "LOGIN".into(),
                ..Default::default()
            }
            .matches(&open)
        );
        assert!(
            BeadFilter {
                issue_type: "bug".into(),
                priority: "1".into(),
                ..Default::default()
            }
            .matches(&open)
        );
        assert!(
            !BeadFilter {
                priority: "0".into(),
                ..Default::default()
            }
            .matches(&open)
        );
    }

    #[test]
    fn scroll_filter_matches_kind_status_and_text() {
        let s = scrolls::Scroll {
            id: "sc-1".into(),
            kind: "decision".into(),
            status: "accepted".into(),
            title: "Use Dolt".into(),
            aliases: vec!["ADR-7".into()],
            ..Default::default()
        };
        assert!(ScrollFilter::default().matches(&s));
        assert!(
            ScrollFilter {
                kind: "decision".into(),
                status: "accepted".into(),
                ..Default::default()
            }
            .matches(&s)
        );
        assert!(
            !ScrollFilter {
                kind: "note".into(),
                ..Default::default()
            }
            .matches(&s)
        );
        assert!(
            ScrollFilter {
                q: "adr-7".into(),
                ..Default::default()
            }
            .matches(&s)
        );
    }
}
