//! The HTTP surface: every area's pages, plus health, live updates, static
//! assets and the headers that guard every response.

use crate::app::App;
use crate::{update, web};
use axum::{
    Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::get,
};
use std::convert::Infallible;
use std::sync::OnceLock;
use tokio_stream::{StreamExt, wrappers::WatchStream};

/// The stylesheet served at `/app.css` is these files joined in this order,
/// so each area keeps its own file. Append new area files here.
const CSS_PARTS: &[&str] = &[include_str!("../static/css/base.css")];

/// `CSS_PARTS` joined once, on first use.
fn stylesheet() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();
    CSS.get_or_init(|| CSS_PARTS.concat())
}

async fn health(State(app): State<App>) -> Response {
    let view = app.view.read().await;
    let status = if view.errors().is_empty() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let beads = view.beads.as_ref().map(|f| {
        serde_json::json!({
            "read_at": f.data.generated_at,
            "source": f.data.source,
            "commit": f.data.commit,
            "beads": f.data.beads.len(),
            "ready": f.data.ready.len(),
            "error": f.error,
        })
    });
    let scrolls = view.scrolls.as_ref().map(|f| {
        serde_json::json!({
            "read_at": f.data.generated_at,
            "revision": f.data.revision,
            "source": f.data.source,
            "store_id": f.data.store_id,
            "branch": f.data.branch,
            "schema_revision": f.data.schema_revision,
            "last_successful_check": view.scrolls_checked_at,
            "scrolls": f.data.scrolls.len(),
            "edges": f.data.edges.len(),
            "error": f.error,
        })
    });
    let provenance = view.provenance.as_ref().map(|p| {
        serde_json::json!({
            "read": p.trails.len(),
            "reading": p.reading.len(),
            "failed": p.trails.values().filter(|t| t.error.is_some()).count(),
        })
    });
    let update = app
        .installed
        .lock()
        .unwrap()
        .as_ref()
        .map(|v| serde_json::json!({ "installed": v }));
    (
        status,
        axum::Json(serde_json::json!({
            "version": update::VERSION,
            "update": update,
            "revision": view.revision,
            "beads": beads,
            "provenance": provenance,
            "scrolls": scrolls,
        })),
    )
        .into_response()
}

async fn events(State(app): State<App>) -> impl IntoResponse {
    let stream = WatchStream::new(app.updates.subscribe())
        .map(|revision| Ok::<_, Infallible>(Event::default().data(revision.to_string())));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

pub fn router(app: App) -> Router {
    Router::new()
        .merge(web::lists::routes())
        .merge(web::bead::routes())
        .merge(web::scrolls::routes())
        .route("/events", get(events))
        .route("/healthz", get(health))
        .route(
            "/app.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    stylesheet(),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../static/app.js"),
                )
            }),
        )
        .route(
            "/theme.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../static/theme.js"),
                )
            }),
        )
        .fallback(|| async { (StatusCode::NOT_FOUND, "Not found") })
        .layer(middleware::from_fn(headers))
        .with_state(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Feed, View};
    use tower::ServiceExt;

    async fn get(app: &App, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let response = router(app.clone())
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn only_known_routes_are_served_and_every_response_is_guarded() {
        let app = App::new(View {
            beads: Some(Feed::default()),
            ..Default::default()
        });
        for (path, status) in [
            ("/", 200),
            ("/beads", 200),
            ("/epics", 200),
            ("/scrolls", 404),
            ("/bead/no-such-id", 404),
            ("/scroll/no-such-id", 404),
            ("/healthz", 200),
            ("/app.css", 200),
            ("/app.js", 200),
            ("/theme.js", 200),
            ("/.git/config", 404),
            ("/../.secret-test", 404),
        ] {
            let (actual, headers, _) = get(&app, path).await;
            assert_eq!(actual.as_u16(), status, "{path}");
            assert!(headers.contains_key("content-security-policy"), "{path}");
        }
    }

    #[tokio::test]
    async fn healthz_reports_each_store_and_turns_unhealthy_on_error() {
        let app = App::new(View {
            beads: Some(Feed::default()),
            ..Default::default()
        });
        let (status, _, body) = get(&app, "/healthz").await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["beads"]["beads"], 0);
        assert!(json["scrolls"].is_null());
        assert_eq!(json["version"], update::VERSION);
        assert!(json["update"].is_null());
        app.update(|view| view.beads.as_mut().unwrap().error = Some("bd unreachable".into()))
            .await;
        let (status, _, body) = get(&app, "/healthz").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.contains("bd unreachable"));
        let (_, _, page) = get(&app, "/").await;
        assert!(page.contains("bd unreachable"));
    }
}
