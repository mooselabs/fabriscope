// Owned by WP6 — Held, costs, receipts, sweeper. Frozen: `routes()` and its
// paths `/held`, `/costs`, `/receipts`.
//! The factory pages that read the ledger.

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
    Router::new()
        .route("/held", get(held_page))
        .route("/costs", get(costs_page))
        .route("/receipts", get(receipts_page))
}

async fn held_page(State(app): State<App>, Query(facets): Query<Facets>) -> Response {
    let view = app.view.read().await;
    respond(&view, with_beads(&view, |v| render::held::held(v, &facets)))
}

/// Costs read provenance usage and, when given, the run feed.
async fn costs_page(State(app): State<App>, Query(facets): Query<Facets>) -> Response {
    let view = app.view.read().await;
    let page =
        (view.beads.is_some() || view.runs.is_some()).then(|| render::costs::costs(&view, &facets));
    respond(&view, page)
}

async fn receipts_page(State(app): State<App>, Query(facets): Query<Facets>) -> Response {
    let view = app.view.read().await;
    respond(
        &view,
        with_beads(&view, |v| render::receipts::receipts(v, &facets)),
    )
}
