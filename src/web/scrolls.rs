//! The scroll list and one scroll's page.

use crate::app::App;
use crate::model::ScrollFilter;
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
        .route("/scrolls", get(scrolls_page))
        .route("/scroll/{id}", get(scroll_page))
}

async fn scrolls_page(State(app): State<App>, Query(filter): Query<ScrollFilter>) -> Response {
    let view = app.view.read().await;
    if view.scrolls.is_none() {
        return (StatusCode::NOT_FOUND, Html(render::not_found(&view))).into_response();
    }
    Html(render::scrolls(&view, &filter)).into_response()
}

async fn scroll_page(State(app): State<App>, Path(id): Path<String>) -> Response {
    let view = app.view.read().await;
    match render::scroll(&view, &id) {
        Some(page) => Html(page).into_response(),
        None => (StatusCode::NOT_FOUND, Html(render::not_found(&view))).into_response(),
    }
}
