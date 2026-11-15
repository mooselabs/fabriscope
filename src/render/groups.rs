// Owned by WP1 — Lists, groups, search, overview. Frozen signatures:
// `group(&View, &str, &Facets) -> Option<String>`,
// `lane(&View, &str, &str, &Facets) -> Option<String>`,
// `members_table(&View, &str, &str) -> String`.
//! Group pages: `/by/{group}` (every value of one `[[groups]]` key, with
//! counts) and `/by/{group}/{value}` (the members, as a lane table when the
//! group has a `subgroup`, ordered by its `declared` values).

use super::layout::page;
use super::{heading, placeholder};
use crate::model::View;
use crate::query::Facets;

/// `/by/{group}`; `None` when the profile has no such group.
pub(crate) fn group(view: &View, group: &str, _facets: &Facets) -> Option<String> {
    let rule = view.profile.group(group)?;
    Some(placeholder(
        view,
        &rule.label,
        &format!("/by/{group}"),
        "WP1",
    ))
}

/// `/by/{group}/{value}`; `None` when the profile has no such group.
pub(crate) fn lane(view: &View, group: &str, value: &str, _facets: &Facets) -> Option<String> {
    let rule = view.profile.group(group)?;
    let title = format!("{} {value}", rule.label);
    let body = format!(
        "{}<p class=\"placeholder muted\">This page is being built (WP1).</p>{}",
        heading("Coming soon", &title, ""),
        members_table(view, group, value)
    );
    Some(page(view, &title, &format!("/by/{group}"), &body))
}

/// The members of one group value as a table (a lane table when the group
/// has a subgroup). Placeholder: empty.
pub(crate) fn members_table(_view: &View, _group: &str, _value: &str) -> String {
    String::new()
}
