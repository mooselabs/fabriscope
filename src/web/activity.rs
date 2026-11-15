// Owned by WP3 — History, activity, people. Frozen: `routes()` and its path
// `/activity`.
//! The activity feed.

use super::{respond, with_beads};
use crate::app::App;
use crate::query::Facets;
use crate::render;
use axum::{
    Router,
    extract::{Query, State},
    response::Response,
    routing::get,
};

pub fn routes() -> Router<App> {
    Router::new().route("/activity", get(activity_page))
}

async fn activity_page(State(app): State<App>, Query(facets): Query<Facets>) -> Response {
    let view = app.view.read().await;
    respond(
        &view,
        with_beads(&view, |v| render::activity::activity(v, &facets)),
    )
}
