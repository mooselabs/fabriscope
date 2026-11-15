// Owned by WP1 — Lists, groups, search, overview. Frozen: `routes()` and
// its paths `/`, `/beads`, `/epics`, `/by/{group}`, `/by/{group}/{value}`,
// `/search`.
//! The overview, the bead and epic lists, group pages and search.

use super::respond;
use crate::app::{App, view_in_progress};
use crate::model::BeadFilter;
use crate::query::Facets;
use crate::render;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/", get(overview))
        .route("/beads", get(beads_page))
        .route("/epics", get(epics_page))
        .route("/by/{group}", get(group_page))
        .route("/by/{group}/{value}", get(lane_page))
        .route("/search", get(search_page))
}

async fn overview(State(app): State<App>) -> Html<String> {
    app.want_provenance(view_in_progress(&app).await).await;
    Html(render::overview(&*app.view.read().await))
}

async fn beads_page(State(app): State<App>, Query(filter): Query<BeadFilter>) -> Response {
    let view = app.view.read().await;
    if view.beads.is_none() {
        return (StatusCode::NOT_FOUND, Html(render::not_found(&view))).into_response();
    }
    Html(render::beads(&view, &filter)).into_response()
}

async fn epics_page(State(app): State<App>, Query(filter): Query<BeadFilter>) -> Response {
    let view = app.view.read().await;
    if view.beads.is_none() {
        return (StatusCode::NOT_FOUND, Html(render::not_found(&view))).into_response();
    }
    Html(render::epics(&view, &filter)).into_response()
}

async fn group_page(
    State(app): State<App>,
    Path(group): Path<String>,
    Query(facets): Query<Facets>,
) -> Response {
    let view = app.view.read().await;
    let page = view
        .beads
        .is_some()
        .then(|| render::groups::group(&view, &group, &facets))
        .flatten();
    respond(&view, page)
}

async fn lane_page(
    State(app): State<App>,
    Path((group, value)): Path<(String, String)>,
    Query(facets): Query<Facets>,
) -> Response {
    let view = app.view.read().await;
    let page = view
        .beads
        .is_some()
        .then(|| render::groups::lane(&view, &group, &value, &facets))
        .flatten();
    respond(&view, page)
}

async fn search_page(State(app): State<App>, Query(facets): Query<Facets>) -> Response {
    let view = app.view.read().await;
    respond(&view, Some(render::search::search(&view, &facets)))
}
