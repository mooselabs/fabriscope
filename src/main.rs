//! Fabriscope: view any Scrolls or Beads store in a web browser.

mod beads;
mod model;
mod render;
mod scrolls;
mod serve;
mod tools;
mod update;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{
        Html, IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::get,
};
use model::{BeadFilter, Feed, ScrollFilter, View};
use std::{
    convert::Infallible,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{RwLock, watch};
use tokio_stream::{StreamExt, wrappers::WatchStream};
use tools::Tool;

const USAGE: &str = "Usage: fabriscope [--beads DIR | --beads-url URL] [--scrolls STORE] [options]
       fabriscope update

View a Beads store, a Scrolls store, or both, in a web browser. Release
builds update themselves in the background; `fabriscope update` updates now.

  --beads DIR          A directory where `bd` finds the Beads store; Fabriscope
                       starts a private `bd serve` there
  --beads-url URL      A `bd serve` that is already running
  --beads-token-file F Bearer token file for that `bd serve`
  --parent-key PATH    Metadata path naming a bead's parent (by id or external
                       reference), for stores without parent-child links
  --scrolls STORE      A Scrolls store directory (usually …/.scrolls)
  --bind ADDR          Address to listen on (default 127.0.0.1:8787)
  --title TEXT         A name shown in the header
  --bd PATH            The bd program (default: bd on PATH)
  --scrolls-bin PATH   The scrolls program (default: scrolls on PATH)
  --scrolls-dolt PATH  The Dolt binary Scrolls should use (passed as --dolt)
  --no-update          Do not check for a new release (or set FABRISCOPE_NO_UPDATE=1)
  -V, --version        Show the version
  -h, --help           Show this help";

#[derive(Clone)]
struct App {
    view: Arc<RwLock<View>>,
    updates: watch::Sender<u64>,
    /// A newer release installed on disk by the background update check.
    installed: Arc<Mutex<Option<String>>>,
}

impl App {
    fn new(view: View) -> Self {
        let (updates, _) = watch::channel(view.revision);
        Self {
            view: Arc::new(RwLock::new(view)),
            updates,
            installed: Arc::default(),
        }
    }

    async fn update(&self, change: impl FnOnce(&mut View)) {
        let mut view = self.view.write().await;
        change(&mut view);
        view.revision += 1;
        self.updates.send_replace(view.revision);
    }
}

#[derive(Debug, Default, PartialEq)]
struct Config {
    beads: Option<PathBuf>,
    beads_url: Option<String>,
    beads_token_file: Option<PathBuf>,
    parent_key: Option<String>,
    scrolls: Option<PathBuf>,
    bind: Option<SocketAddr>,
    title: Option<String>,
    bd: Option<PathBuf>,
    scrolls_bin: Option<PathBuf>,
    scrolls_dolt: Option<PathBuf>,
    no_update: bool,
    help: bool,
    version: bool,
    /// `fabriscope update`: install a newer release now and exit.
    update: bool,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Config, String> {
    let mut config = Config::default();
    let mut args = args.into_iter().peekable();
    if args.peek().map(String::as_str) == Some("update") {
        args.next();
        if let Some(extra) = args.next() {
            return Err(format!("fabriscope update takes no arguments, not {extra}"));
        }
        config.update = true;
        return Ok(config);
    }
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--beads" => config.beads = Some(value("--beads")?.into()),
            "--beads-url" => config.beads_url = Some(value("--beads-url")?),
            "--parent-key" => config.parent_key = Some(value("--parent-key")?),
            "--beads-token-file" => {
                config.beads_token_file = Some(value("--beads-token-file")?.into())
            }
            "--scrolls" => config.scrolls = Some(value("--scrolls")?.into()),
            "--bind" => {
                config.bind = Some(
                    value("--bind")?
                        .parse()
                        .map_err(|e| format!("--bind: {e}"))?,
                )
            }
            "--title" => config.title = Some(value("--title")?),
            "--bd" => config.bd = Some(value("--bd")?.into()),
            "--scrolls-bin" => config.scrolls_bin = Some(value("--scrolls-bin")?.into()),
            "--scrolls-dolt" => config.scrolls_dolt = Some(value("--scrolls-dolt")?.into()),
            "--no-update" => config.no_update = true,
            "--help" | "-h" => config.help = true,
            "--version" | "-V" => config.version = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if config.beads.is_some() && config.beads_url.is_some() {
        return Err("give --beads DIR or --beads-url URL, not both".into());
    }
    if !config.help
        && !config.version
        && config.beads.is_none()
        && config.beads_url.is_none()
        && config.scrolls.is_none()
    {
        return Err("give --beads DIR (or --beads-url URL), --scrolls STORE, or both".into());
    }
    Ok(config)
}

fn existing_dir(path: &std::path::Path, flag: &str) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("{flag} {}: {e}", path.display()))?;
    if !path.is_dir() {
        return Err(format!("{flag} {} is not a directory", path.display()));
    }
    Ok(path)
}

fn beads_tool(config: &Config, dir: PathBuf) -> Tool {
    Tool {
        name: "bd",
        program: config.bd.clone().unwrap_or_else(|| "bd".into()),
        args: vec![],
        cwd: dir,
    }
}

/// How to read Beads: an existing `bd serve`, a private one started in the
/// directory, or `bd export` when `bd serve` refuses that directory (it
/// needs a Git repository).
async fn beads_reader(config: &Config) -> Result<Option<beads::Reader>, String> {
    if let Some(url) = &config.beads_url {
        let api = serve::Api::new(url, config.beads_token_file.clone())?;
        return Ok(Some(beads::Reader::Api { api, local: None }));
    }
    let Some(dir) = &config.beads else {
        return Ok(None);
    };
    let tool = beads_tool(config, existing_dir(dir, "--beads")?);
    match serve::start(&tool, config.beads_token_file.clone()).await {
        Ok(api) => Ok(Some(beads::Reader::Api {
            api,
            local: Some(tool),
        })),
        Err(reason) => {
            eprintln!(
                "fabriscope: bd serve did not start, so reading with bd export instead. {reason}"
            );
            Ok(Some(beads::Reader::Cli(tool)))
        }
    }
}

fn scrolls_tool(config: &Config, store: &std::path::Path) -> Tool {
    let mut args = vec!["--store".to_string(), store.to_string_lossy().into_owned()];
    if let Some(dolt) = &config.scrolls_dolt {
        args.extend(["--dolt".to_string(), dolt.to_string_lossy().into_owned()]);
    }
    Tool {
        name: "scrolls",
        program: config
            .scrolls_bin
            .clone()
            .unwrap_or_else(|| "scrolls".into()),
        args,
        cwd: store.parent().unwrap_or(store).to_path_buf(),
    }
}

/// Re-reads Beads when its change signal moves. A commit is a reliable
/// signal, so a timed re-read is only a slow safety net; API stats are a
/// hint, so the timer is shorter; with no signal at all, re-read every 30s.
async fn watch_beads(
    app: App,
    reader: beads::Reader,
    parent_key: Option<String>,
    mut last: Option<String>,
) {
    let mut refreshed = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let probe = reader.probe().await;
        let due = match &probe {
            Some((signal, strong)) => {
                let every = Duration::from_secs(if *strong { 300 } else { 60 });
                last.as_ref() != Some(signal) || refreshed.elapsed() >= every
            }
            None => refreshed.elapsed() >= Duration::from_secs(30),
        };
        if !due {
            continue;
        }
        // The signal is taken before the read, so a change during the read
        // shows up as a new signal next time round.
        last = probe.map(|(signal, _)| signal);
        let result = beads::load(&reader, parent_key.as_deref()).await;
        refreshed = Instant::now();
        app.update(|view| {
            if let Some(feed) = view.beads.as_mut() {
                match result {
                    Ok(snapshot) => {
                        *feed = Feed {
                            data: Arc::new(snapshot),
                            error: None,
                        }
                    }
                    Err(error) => {
                        feed.error = Some(format!(
                            "Beads read failed; showing the last good read. {error}"
                        ))
                    }
                }
            }
        })
        .await;
    }
}

/// Re-reads Scrolls when the store's files change, and every ten minutes.
async fn watch_scrolls(app: App, tool: Tool, store: PathBuf) {
    let fingerprint = |store: PathBuf| async move {
        tokio::task::spawn_blocking(move || scrolls::fingerprint(&store))
            .await
            .map_err(|e| e.to_string())?
    };
    let mut last = fingerprint(store.clone()).await.ok();
    let mut refreshed = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let current = fingerprint(store.clone()).await;
        // A failing fingerprint falls back to the timer rather than re-reading
        // every two seconds.
        let changed = match (&current, &last) {
            (Ok(now), Some(before)) => now != before,
            (Ok(_), None) => true,
            (Err(_), _) => false,
        };
        if !changed && refreshed.elapsed() < Duration::from_secs(600) {
            continue;
        }
        let result = scrolls::load(&tool).await;
        refreshed = Instant::now();
        // Take the fingerprint after the read, so the read's own files settle first.
        last = fingerprint(store.clone()).await.ok();
        app.update(|view| {
            if let Some(feed) = view.scrolls.as_mut() {
                match result {
                    Ok(snapshot) => {
                        *feed = Feed {
                            data: Arc::new(snapshot),
                            error: None,
                        }
                    }
                    Err(error) => {
                        feed.error = Some(format!(
                            "Scrolls read failed; showing the last good read. {error}"
                        ))
                    }
                }
            }
        })
        .await;
    }
}

async fn overview(State(app): State<App>) -> Html<String> {
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

async fn scrolls_page(State(app): State<App>, Query(filter): Query<ScrollFilter>) -> Response {
    let view = app.view.read().await;
    if view.scrolls.is_none() {
        return (StatusCode::NOT_FOUND, Html(render::not_found(&view))).into_response();
    }
    Html(render::scrolls(&view, &filter)).into_response()
}

async fn bead_page(State(app): State<App>, Path(id): Path<String>) -> Response {
    let view = app.view.read().await;
    match render::bead(&view, &id) {
        Some(page) => Html(page).into_response(),
        None => (StatusCode::NOT_FOUND, Html(render::not_found(&view))).into_response(),
    }
}

async fn scroll_page(State(app): State<App>, Path(id): Path<String>) -> Response {
    let view = app.view.read().await;
    match render::scroll(&view, &id) {
        Some(page) => Html(page).into_response(),
        None => (StatusCode::NOT_FOUND, Html(render::not_found(&view))).into_response(),
    }
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
            "scrolls": f.data.scrolls.len(),
            "edges": f.data.edges.len(),
            "error": f.error,
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

fn router(app: App) -> Router {
    Router::new()
        .route("/", get(overview))
        .route("/beads", get(beads_page))
        .route("/epics", get(epics_page))
        .route("/bead/{id}", get(bead_page))
        .route("/scrolls", get(scrolls_page))
        .route("/scroll/{id}", get(scroll_page))
        .route("/events", get(events))
        .route("/healthz", get(health))
        .route(
            "/app.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../static/app.css"),
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
        .fallback(|| async { (StatusCode::NOT_FOUND, "Not found") })
        .layer(middleware::from_fn(headers))
        .with_state(app)
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("fabriscope: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let config = parse_args(std::env::args().skip(1)).map_err(|e| format!("{e}\n\n{USAGE}"))?;
    if config.help {
        println!("{USAGE}");
        return Ok(());
    }
    if config.version {
        println!("{}", update::version_line());
        return Ok(());
    }
    if config.update {
        return update_now().await;
    }
    let beads_reader = beads_reader(&config).await?;
    let scrolls_store = config
        .scrolls
        .as_deref()
        .map(|s| existing_dir(s, "--scrolls"))
        .transpose()?;
    let scrolls_tool = scrolls_store
        .as_deref()
        .map(|store| scrolls_tool(&config, store));
    // An unreadable store at startup is an error, not an empty page.
    eprintln!("fabriscope: reading the stores…");
    let first_signal = match &beads_reader {
        Some(reader) => reader.probe().await.map(|(signal, _)| signal),
        None => None,
    };
    let (beads, scrolls) = tokio::join!(
        async {
            match &beads_reader {
                Some(reader) => beads::load(reader, config.parent_key.as_deref())
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
        async {
            match &scrolls_tool {
                Some(tool) => scrolls::load(tool).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let (beads, scrolls) = (beads?, scrolls?);
    let title = config.title.clone().unwrap_or_default();
    let summary = format!(
        "{}{}",
        beads
            .as_ref()
            .map(|b| format!(" · {} beads via {}", b.beads.len(), b.source))
            .unwrap_or_default(),
        scrolls
            .as_ref()
            .map(|s| format!(" · {} scrolls", s.scrolls.len()))
            .unwrap_or_default()
    );
    let app = App::new(View {
        title,
        beads: beads.map(|data| Feed {
            data: Arc::new(data),
            error: None,
        }),
        scrolls: scrolls.map(|data| Feed {
            data: Arc::new(data),
            error: None,
        }),
        revision: 1,
    });
    let bind = config
        .bind
        .unwrap_or_else(|| "127.0.0.1:8787".parse().unwrap());
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| format!("cannot listen on {bind}: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    println!("Fabriscope: http://{address}{summary}");
    if let Some(reader) = beads_reader {
        tokio::spawn(watch_beads(
            app.clone(),
            reader,
            config.parent_key.clone(),
            first_signal,
        ));
    }
    if let (Some(tool), Some(store)) = (scrolls_tool, scrolls_store) {
        tokio::spawn(watch_scrolls(app.clone(), tool, store));
    }
    if update::automatic(config.no_update) {
        tokio::spawn(update::watch(app.installed.clone()));
    }
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())
}

/// `fabriscope update`: install a newer release, if there is one, and exit.
async fn update_now() -> Result<(), String> {
    let current = semver::Version::parse(update::VERSION).map_err(|e| e.to_string())?;
    let outcome = tokio::task::spawn_blocking(move || update::check_and_install(&current))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|failure| failure.message)?;
    match outcome {
        update::Outcome::Current(version) => {
            println!("fabriscope {version} is the latest release.")
        }
        update::Outcome::Updated(version) => println!(
            "fabriscope updated to {version}. Running servers keep the old version until they restart."
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn args(list: &[&str]) -> Result<Config, String> {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn arguments_need_at_least_one_store() {
        assert!(args(&[]).is_err());
        assert!(args(&["--help"]).unwrap().help);
        let config = args(&[
            "--beads",
            "/tmp",
            "--bind",
            "127.0.0.1:9000",
            "--title",
            "Demo",
        ])
        .unwrap();
        assert_eq!(config.beads, Some("/tmp".into()));
        assert_eq!(config.bind, Some("127.0.0.1:9000".parse().unwrap()));
        assert!(args(&["--beads"]).is_err());
        assert!(args(&["--scrolls", "/tmp", "--bind", "nowhere"]).is_err());
        assert!(args(&["--frobnicate"]).is_err());
    }

    #[test]
    fn version_and_update_need_no_store() {
        assert!(args(&["--version"]).unwrap().version);
        assert!(args(&["-V"]).unwrap().version);
        assert!(args(&["update"]).unwrap().update);
        assert!(args(&["update", "--now"]).is_err());
        // `update` is a command only in first place, not a store path.
        assert!(args(&["--scrolls", "/tmp", "update"]).is_err());
        assert!(
            args(&["--scrolls", "/tmp", "--no-update"])
                .unwrap()
                .no_update
        );
    }

    #[test]
    fn scrolls_runs_from_the_store_parent_with_its_dolt() {
        let config = Config {
            scrolls_dolt: Some("/opt/dolt".into()),
            ..Default::default()
        };
        let tool = scrolls_tool(&config, std::path::Path::new("/work/project/.scrolls"));
        assert_eq!(tool.cwd, PathBuf::from("/work/project"));
        assert_eq!(
            tool.args,
            ["--store", "/work/project/.scrolls", "--dolt", "/opt/dolt"]
        );
    }

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
