//! Each area's pages: its handlers and the routes that reach them. Every
//! area file has `pub fn routes() -> Router<App>`, merged once in
//! `routes::router`, so a package only edits its own area file.

pub mod activity;
pub mod bead;
pub mod factory;
pub mod flow;
pub mod lists;
pub mod people;
pub mod runs;
pub mod scrolls;

use crate::model::View;
use crate::render;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

/// 200 with the page, or 404 with the shared not-found page.
pub(crate) fn respond(view: &View, page: Option<String>) -> Response {
    match page {
        Some(page) => Html(page).into_response(),
        None => (StatusCode::NOT_FOUND, Html(render::not_found(view))).into_response(),
    }
}

/// The page when a Beads store is shown, else `None` (a 404).
pub(crate) fn with_beads(view: &View, render: impl FnOnce(&View) -> String) -> Option<String> {
    view.beads.is_some().then(|| render(view))
}

/// The page when a run feed is shown, else `None` (a 404).
pub(crate) fn with_runs(view: &View, render: impl FnOnce(&View) -> String) -> Option<String> {
    view.runs.is_some().then(|| render(view))
}
