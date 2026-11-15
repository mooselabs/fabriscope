// Owned by WP3 — History, activity, people. Frozen signatures:
// `people(&View, &Facets) -> String`,
// `person(&View, &str, &Facets) -> Option<String>`.
//! `/people` and `/person/{name}`: who did what, factory seats classed apart
//! from people (`Profile::person`).

use super::placeholder;
use crate::model::View;
use crate::query::Facets;

/// `/people`. Placeholder page.
pub(crate) fn people(view: &View, _facets: &Facets) -> String {
    placeholder(view, "People", "/people", "WP3")
}

/// `/person/{name}`; `None` for a name nothing mentions. Placeholder: a
/// page for any name.
pub(crate) fn person(view: &View, name: &str, _facets: &Facets) -> Option<String> {
    Some(placeholder(view, name, "/people", "WP3"))
}
