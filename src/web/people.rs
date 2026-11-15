// Owned by WP3 — History, activity, people. Frozen: `routes()` and its
// paths `/people`, `/person/{name}`.
//! People and one person's page.

use super::{respond, with_beads};
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
        .route("/people", get(people_page))
        .route("/person/{name}", get(person_page))
}

async fn people_page(State(app): State<App>, Query(facets): Query<Facets>) -> Response {
    let view = app.view.read().await;
    respond(
        &view,
        with_beads(&view, |v| render::people::people(v, &facets)),
    )
}

/// A name with `/` in it (a factory seat) arrives percent-encoded as one
/// segment; axum decodes it.
async fn person_page(
    State(app): State<App>,
    Path(name): Path<String>,
    Query(facets): Query<Facets>,
) -> Response {
    let view = app.view.read().await;
    let page = view
        .beads
        .is_some()
        .then(|| render::people::person(&view, &name, &facets))
        .flatten();
    respond(&view, page)
}
