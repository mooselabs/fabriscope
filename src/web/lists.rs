//! The overview and the bead and epic lists.

use crate::app::{App, view_in_progress};
use crate::model::BeadFilter;
use crate::render;
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/", get(overview))
        .route("/beads", get(beads_page))
        .route("/epics", get(epics_page))
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
