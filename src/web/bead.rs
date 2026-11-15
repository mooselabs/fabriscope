// Owned by WP2 — Bead page. Frozen: `routes()` and its path `/bead/{id}`.
//! One bead's page.

use crate::app::App;
use crate::history;
use crate::render;
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};

pub fn routes() -> Router<App> {
    Router::new().route("/bead/{id}", get(bead_page))
}

async fn bead_page(State(app): State<App>, Path(id): Path<String>) -> Response {
    app.want_provenance(vec![id.clone()]).await;
    history::reader::want(&app, vec![id.clone()]).await;
    let view = app.view.read().await;
    match render::bead(&view, &id) {
        Some(page) => Html(page).into_response(),
        None => (StatusCode::NOT_FOUND, Html(render::not_found(&view))).into_response(),
    }
}
