//! List filters that live in the URL and carry across pages.
//!
//! Every list page reads `Facets` straight from its query
//! (`Query<Facets>`), filters with `Facets::matches`, and builds its links
//! with `href` so the filters a person chose follow them from tab to tab.
//! Keys a profile's `[[groups]]` define arrive in `Facets::groups`.

use crate::model::View;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use std::collections::BTreeMap;

/// Query keys Fabriscope owns. A profile group may not use one.
pub const RESERVED: &[&str] = &[
    "q", "type", "kind", "priority", "state", "dstate", "ready", "person", "epic", "page", "from",
    "to", "days", "sort",
];

/// The filters of a list page. Every field is the raw query text (empty
/// when absent), so a bad value never fails the request; the accessors
/// parse.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "Vec<(String, String)>")]
pub struct Facets {
    /// Free text: id, title, external ref or label.
    pub q: String,
    /// The stored issue type.
    pub issue_type: String,
    /// A profile kind key.
    pub kind: String,
    pub priority: String,
    /// A stored status, `all`, or empty for every bead not closed.
    pub state: String,
    /// A profile state key (derived state).
    pub dstate: String,
    /// `yes` (ready as `bd ready` says, profile overrides applied), `no`, or
    /// empty.
    pub ready: String,
    /// A person name, in any role or as assignee.
    pub person: String,
    /// An epic (or other parent) by id or external ref.
    pub epic: String,
    pub page: String,
    pub from: String,
    pub to: String,
    pub days: String,
    pub sort: String,
    /// Profile group key → value.
    pub groups: BTreeMap<String, String>,
}

impl From<Vec<(String, String)>> for Facets {
    fn from(pairs: Vec<(String, String)>) -> Self {
        let mut f = Facets::default();
        for (key, value) in pairs {
            let slot = match key.as_str() {
                "q" => &mut f.q,
                "type" => &mut f.issue_type,
                "kind" => &mut f.kind,
                "priority" => &mut f.priority,
                "state" => &mut f.state,
                "dstate" => &mut f.dstate,
                "ready" => &mut f.ready,
                "person" => &mut f.person,
                "epic" => &mut f.epic,
                "page" => &mut f.page,
                "from" => &mut f.from,
                "to" => &mut f.to,
                "days" => &mut f.days,
                "sort" => &mut f.sort,
                _ => {
                    if !value.is_empty() {
                        f.groups.insert(key, value);
                    }
                    continue;
                }
            };
            *slot = value;
        }
        f
    }
}

impl Facets {
    /// The 0-based page number (0 when absent or bad).
    pub fn page(&self) -> usize {
        self.page.parse().unwrap_or(0)
    }

    /// The date range the query names.
    pub fn range(&self) -> Range {
        Range {
            from: NaiveDate::parse_from_str(&self.from, "%Y-%m-%d").ok(),
            to: NaiveDate::parse_from_str(&self.to, "%Y-%m-%d").ok(),
            days: self.days.parse().ok().filter(|d| *d > 0),
        }
    }

    /// The pairs `href` writes, in a fixed order, without `page` and
    /// without empty values.
    pub fn pairs(&self) -> Vec<(String, String)> {
        let fixed = [
            ("q", &self.q),
            ("type", &self.issue_type),
            ("kind", &self.kind),
            ("priority", &self.priority),
            ("state", &self.state),
            ("dstate", &self.dstate),
            ("ready", &self.ready),
            ("person", &self.person),
            ("epic", &self.epic),
            ("from", &self.from),
            ("to", &self.to),
            ("days", &self.days),
            ("sort", &self.sort),
        ];
        fixed
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .chain(self.groups.iter().map(|(k, v)| (k.clone(), v.clone())))
            .filter(|(_, v)| !v.is_empty())
            .collect()
    }

    /// Whether bead `index` of the view's snapshot passes every facet.
    /// Receipt and hidden rows never pass: list pages show items only. The
    /// date range is not applied here (each page knows which timestamp it
    /// means); use `Range::contains`.
    pub fn matches(&self, view: &View, index: usize) -> bool {
        let Some(feed) = &view.beads else {
            return false;
        };
        let s = &feed.data;
        let Some(bead) = s.beads.get(index) else {
            return false;
        };
        let row = s.derived.rows.get(index);
        if !s.derived.is_item(index) {
            return false;
        }
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
        let contains = |haystack: &[&str]| {
            let needle = self.q.to_lowercase();
            haystack.iter().any(|h| h.to_lowercase().contains(&needle))
        };
        let epic = self.epic.is_empty()
            || s.find(&self.epic)
                .is_some_and(|e| bead.parent() == Some(e.id.as_str()));
        state
            && ready
            && epic
            && (self.issue_type.is_empty() || bead.issue_type == self.issue_type)
            && (self.priority.is_empty() || bead.priority.to_string() == self.priority)
            && (self.kind.is_empty()
                || row.and_then(|r| r.kind.as_deref()) == Some(self.kind.as_str()))
            && (self.dstate.is_empty() || row.is_some_and(|r| r.states.contains(&self.dstate)))
            && (self.person.is_empty()
                || bead.assignee == self.person
                || row.is_some_and(|r| r.people.values().any(|p| *p == self.person)))
            && self.groups.iter().all(|(key, value)| {
                row.and_then(|r| r.groups.get(key))
                    .is_some_and(|values| values.contains(value))
            })
            && (self.q.is_empty()
                || contains(&[
                    &bead.id,
                    &bead.title,
                    &bead.external_ref,
                    &bead.labels.join(" "),
                ]))
    }
}

/// A range of whole UTC days. `days` counts back from today and wins over
/// `from`/`to`; with nothing set the range is unbounded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub days: Option<u32>,
}

impl Range {
    /// The first and last day included, at `now`.
    pub fn bounds(&self, now: DateTime<Utc>) -> (Option<NaiveDate>, Option<NaiveDate>) {
        match self.days {
            Some(days) => {
                let today = now.date_naive();
                (
                    Some(today - Duration::days(i64::from(days) - 1)),
                    Some(today),
                )
            }
            None => (self.from, self.to),
        }
    }

    /// Whether an RFC 3339 timestamp (or `YYYY-MM-DD` date) falls in the
    /// range. A value that does not parse is outside a bounded range.
    pub fn contains(&self, at: &str, now: DateTime<Utc>) -> bool {
        let (from, to) = self.bounds(now);
        if from.is_none() && to.is_none() {
            return true;
        }
        let date = DateTime::parse_from_rfc3339(at)
            .map(|t| t.with_timezone(&Utc).date_naive())
            .ok()
            .or_else(|| NaiveDate::parse_from_str(at.get(..10).unwrap_or(at), "%Y-%m-%d").ok());
        date.is_some_and(|d| from.is_none_or(|f| d >= f) && to.is_none_or(|t| d <= t))
    }
}

/// A link to `path` carrying `facets` (never `page`), with `overrides`
/// replacing or adding keys; an override with an empty value removes the
/// key. The result is a raw URL: escape it before putting it in markup.
pub fn href(path: &str, facets: &Facets, overrides: &[(&str, &str)]) -> String {
    let mut pairs = facets.pairs();
    for (key, value) in overrides {
        pairs.retain(|(k, _)| k != key);
        if !value.is_empty() && *key != "page" {
            pairs.push((key.to_string(), value.to_string()));
        }
    }
    if pairs.is_empty() {
        return path.to_string();
    }
    let query: Vec<String> = pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, NON_ALPHANUMERIC),
                utf8_percent_encode(v, NON_ALPHANUMERIC)
            )
        })
        .collect();
    format!("{path}?{}", query.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facets(query: &str) -> Facets {
        let uri: axum::http::Uri = format!("/x?{query}").parse().unwrap();
        axum::extract::Query::<Facets>::try_from_uri(&uri)
            .unwrap()
            .0
    }

    #[test]
    fn facets_read_any_query_and_links_carry_them_without_the_page() {
        let f = facets("q=csv&type=task&page=3&milestone=v1&rig=&state=all&days=x");
        assert_eq!(f.issue_type, "task");
        assert_eq!(f.page(), 3);
        assert_eq!(f.groups.len(), 1, "an empty group value is no filter");
        assert_eq!(f.range().days, None, "a bad number is ignored");
        assert_eq!(
            href("/beads", &f, &[]),
            "/beads?q=csv&type=task&state=all&days=x&milestone=v1"
        );
        assert_eq!(
            href(
                "/people",
                &f,
                &[("q", ""), ("person", "a b"), ("page", "9")]
            ),
            "/people?type=task&state=all&days=x&milestone=v1&person=a%20b"
        );
        assert_eq!(href("/", &Facets::default(), &[]), "/");
        assert_eq!(facets("page=-1").page(), 0);
    }

    #[test]
    fn ranges_are_whole_utc_days() {
        let now = DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let week = facets("days=7").range();
        assert!(week.contains("2026-11-09T00:00:00Z", now));
        assert!(!week.contains("2026-11-08T23:59:59Z", now));
        let span = facets("from=2026-11-01&to=2026-11-02").range();
        assert!(span.contains("2026-11-02", now));
        assert!(!span.contains("2026-11-03T00:00:00Z", now));
        assert!(!span.contains("garbage", now));
        assert!(Range::default().contains("garbage", now));
    }

    #[test]
    fn facets_filter_the_work_fixture_by_profile_facts() {
        let view = crate::testing::work_view();
        let s = &view.beads.as_ref().unwrap().data;
        let ids = |q: &str| -> Vec<String> {
            let f = facets(q);
            (0..s.beads.len())
                .filter(|&i| f.matches(&view, i))
                .map(|i| s.beads[i].external_ref.clone())
                .collect()
        };
        assert_eq!(ids("dstate=held"), ["WORK-0003"]);
        assert_eq!(ids("milestone=v1&state=all").len(), 3);
        assert_eq!(ids("kind=gate"), ["WORK-0004"]);
        assert_eq!(ids("person=bob"), ["Q-0001"]);
        assert_eq!(ids("epic=EPIC-0001&state=all").len(), 4);
        assert!(
            ids("state=all").iter().all(|r| !r.is_empty()),
            "receipts and hidden rows are left out"
        );
        assert!(!ids("ready=yes").contains(&"WORK-0003".to_string()));
    }
}
