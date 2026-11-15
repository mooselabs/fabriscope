// Owned by WP5 — Runs. Frozen: `routes()` and its paths `/runs`,
// `/run/{id}`, `/run/{id}/doc/{doc}` (query `page`), `/boards`,
// `/boards/{name}`.
//! The run feed's pages. Without `--runs` they are 404s.

use super::{respond, with_runs};
use crate::app::App;
use crate::query::Facets;
use crate::render;
use axum::{
    Router,
    extract::{Path, Query, State},
    response::Response,
    routing::get,
};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/runs", get(runs_page))
        .route("/run/{id}", get(run_page))
        .route("/run/{id}/doc/{doc}", get(doc_page))
        .route("/boards", get(boards_page))
        .route("/boards/{name}", get(board_page))
}

async fn runs_page(State(app): State<App>, Query(facets): Query<Facets>) -> Response {
    let view = app.view.read().await;
    respond(&view, with_runs(&view, |v| render::runs::runs(v, &facets)))
}

async fn run_page(State(app): State<App>, Path(id): Path<String>) -> Response {
    let view = app.view.read().await;
    let page = view
        .runs
        .is_some()
        .then(|| render::runs::run(&view, &id))
        .flatten();
    respond(&view, page)
}

async fn doc_page(
    State(app): State<App>,
    Path((id, doc)): Path<(String, String)>,
    Query(facets): Query<Facets>,
) -> Response {
    let view = app.view.read().await;
    let page = view
        .runs
        .is_some()
        .then(|| render::runs::doc(&view, &id, &doc, facets.page()))
        .flatten();
    respond(&view, page)
}

async fn boards_page(State(app): State<App>) -> Response {
    let view = app.view.read().await;
    respond(&view, with_runs(&view, render::boards::boards))
}

async fn board_page(State(app): State<App>, Path(name): Path<String>) -> Response {
    let view = app.view.read().await;
    let page = view
        .runs
        .is_some()
        .then(|| render::boards::board(&view, &name))
        .flatten();
    respond(&view, page)
}
