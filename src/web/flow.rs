// Owned by WP4 — Flow. Frozen: `routes()` and its paths `/flow`,
// `/flow/created`, `/flow/claimed`, `/flow/cycle`, `/flow/ageing`,
// `/flow/decisions`.
//! The flow pages.

use super::{respond, with_beads};
use crate::app::App;
use crate::query::Facets;
use crate::render;
use crate::render::flow::FlowPage;
use axum::{
    Router,
    extract::{Query, State},
    response::Response,
    routing::get,
};

pub fn routes() -> Router<App> {
    let page = |which: FlowPage| {
        get(
            move |State(app): State<App>, Query(facets): Query<Facets>| async move {
                flow_page(app, which, facets).await
            },
        )
    };
    Router::new()
        .route("/flow", page(FlowPage::Overview))
        .route("/flow/created", page(FlowPage::Created))
        .route("/flow/claimed", page(FlowPage::Claimed))
        .route("/flow/cycle", page(FlowPage::Cycle))
        .route("/flow/ageing", page(FlowPage::Ageing))
        .route("/flow/decisions", page(FlowPage::Decisions))
}

async fn flow_page(app: App, which: FlowPage, facets: Facets) -> Response {
    let view = app.view.read().await;
    respond(
        &view,
        with_beads(&view, |v| render::flow::flow(v, which, &facets)),
    )
}
