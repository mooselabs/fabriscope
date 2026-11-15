// Owned by WP5 — Runs. Frozen signature: `pub fn spawn(app: App, source:
// runs::Source)`.
//! Keeps `View::runs` current from the feed and lets `View::run_cache`
//! fetch what pages ask for.
//!
//! One poll at a time, every 30 s (`FABRISCOPE_RUNS_POLL_MS` shortens it for
//! tests): read `feed.json` (over HTTP with `If-None-Match` once the server
//! has sent an ETag), and when its revision moved, or nothing has been read
//! yet, read the index, shards and subject files as `runs::load` does and
//! install the new index. A failed poll keeps the last good index and sets
//! the feed error, so `/healthz` answers 503 and every page warns; the next
//! good poll clears it. The view revision moves only when something shown
//! changed. Every attempt id of each run doc the cache fetches is added to
//! the index (and carried into each new index), so `View::run_for_attempt`
//! finds earlier attempts, not only the current one subject files name.
//!
//! Feed files are read from a directory (never outside its canonical root,
//! symlinks included) or over plain HTTP with the same hyper client the
//! Beads API uses: a bearer token re-read from `--runs-token-file` on every
//! request, a 120 s timeout, a response cap, and each href segment
//! percent-encoded so a file name is never read as a path.

use super::{Feed, Index, Source};
use crate::app::App;
use crate::model;
use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper::header;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// How often `feed.json` is read.
const POLL: Duration = Duration::from_secs(30);

/// How long one HTTP read may take.
const TIMEOUT: Duration = Duration::from_secs(120);

/// The largest feed file read, over HTTP or from disk. The spec keeps pages
/// under 512 KiB and run docs under 1 MiB; an index can be larger.
pub(crate) const MAX_BYTES: usize = 32 << 20;

/// Starts keeping the run feed current. Called once at startup when
/// `--runs` is given: installs the cache's reader, then polls.
pub fn spawn(app: App, source: Source) {
    let installed = app.view.try_read().ok().map(|view| {
        let cache = view.run_cache.clone();
        install(&app, &cache, &source, &view);
        cache
    });
    tokio::spawn(async move {
        let cache = match installed {
            Some(cache) => cache,
            None => {
                let view = app.view.read().await;
                let cache = view.run_cache.clone();
                install(&app, &cache, &source, &view);
                cache
            }
        };
        poll(app, source, cache).await;
    });
}

/// Gives the cache a way to read feed files and to tell open pages that a
/// fetch arrived, and the hrefs of the index read at startup.
fn install(app: &App, cache: &Arc<super::cache::Cache>, source: &Source, view: &model::View) {
    let reader = source.clone();
    let notify = app.clone();
    cache.install(
        move |href| reader.read(href),
        move |arrived: Option<Arc<super::Run>>| {
            let app = notify.clone();
            tokio::spawn(async move {
                app.update(|view| {
                    if let (Some(run), Some(feed)) = (arrived, view.runs.as_mut()) {
                        register(feed, &run);
                    }
                })
                .await
            });
        },
    );
    if let Some(index) = view.run_index() {
        cache.index(index);
    }
}

/// Adds every attempt id of a fetched run doc to the index, so
/// `View::run_for_attempt` finds older attempts too (subject files name only
/// the current one). The index is copied only when an id is new.
fn register(feed: &mut model::Feed<Index>, run: &super::Run) {
    let known = run.attempts.iter().all(|a| {
        a.id.is_empty()
            || feed
                .data
                .by_attempt(&a.id)
                .is_some_and(|r| r.run == run.run)
    });
    if known || feed.data.run(&run.run).is_none() {
        return;
    }
    let index = (*feed.data).clone().with_attempts([run]);
    feed.data = Arc::new(index);
}

fn poll_interval() -> Duration {
    std::env::var("FABRISCOPE_RUNS_POLL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(POLL)
}

/// What the poller remembers between reads.
#[derive(Default)]
struct Cursor {
    /// The revision of the installed index, once one is.
    revision: Option<u64>,
    /// The ETag of the `feed.json` that index came from.
    etag: Option<String>,
}

async fn poll(app: App, source: Source, cache: Arc<super::cache::Cache>) {
    let http = match &source {
        Source::Url { base, token_file } => Some(Http::new(base, token_file.clone())),
        Source::Dir(_) => None,
    };
    let mut cursor = Cursor::default();
    {
        let view = app.view.read().await;
        if let Some(feed) = view.runs.as_ref() {
            if feed.error.is_none() && !feed.data.feed.schema.is_empty() {
                cursor.revision = Some(feed.data.feed.revision);
            }
        }
    }
    let every = poll_interval();
    loop {
        let result = match &http {
            Some(Err(error)) => Err(error.clone()),
            Some(Ok(http)) => read(&source, Some(http), &mut cursor).await,
            None => read(&source, None, &mut cursor).await,
        };
        apply(&app, &cache, result).await;
        tokio::time::sleep(every).await;
    }
}

/// One poll: `Ok(None)` when the feed has not moved, `Ok(Some(index))` for
/// a new index.
async fn read(
    source: &Source,
    http: Option<&Http>,
    cursor: &mut Cursor,
) -> Result<Option<Index>, String> {
    let (bytes, etag) = match http {
        Some(http) => {
            let etag = cursor.revision.and(cursor.etag.as_deref());
            match http.get("feed.json", etag).await? {
                Got::NotModified => return Ok(None),
                Got::Body(bytes, etag) => (bytes, etag),
            }
        }
        None => (blocking(source, |s| s.read("feed.json")).await?, None),
    };
    let feed: Feed =
        serde_json::from_slice(&bytes).map_err(|e| format!("run feed feed.json: {e}"))?;
    if feed.schema != "runfeed.v1" {
        return Err(format!(
            "run feed schema {:?}: only runfeed.v1 is understood",
            feed.schema
        ));
    }
    if cursor.revision == Some(feed.revision) {
        cursor.etag = etag;
        return Ok(None);
    }
    let index = blocking(source, super::load).await?;
    cursor.revision = Some(index.feed.revision);
    cursor.etag = etag.filter(|_| index.feed.revision == feed.revision);
    Ok(Some(index))
}

async fn blocking<T: Send + 'static>(
    source: &Source,
    read: impl FnOnce(&Source) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let source = source.clone();
    tokio::task::spawn_blocking(move || read(&source))
        .await
        .map_err(|e| format!("run feed read stopped: {e}"))?
}

/// Installs a poll's outcome, moving the view revision only on a change.
async fn apply(app: &App, cache: &super::cache::Cache, result: Result<Option<Index>, String>) {
    let mut view = app.view.write().await;
    let Some(feed) = view.runs.as_mut() else {
        return;
    };
    let changed = match result {
        Ok(Some(index)) => {
            cache.index(&index);
            let held = cache.ready_runs();
            let index = index.with_attempts(held.iter().map(|r| &**r));
            *feed = model::Feed {
                data: Arc::new(index),
                error: None,
            };
            true
        }
        Ok(None) => feed.error.take().is_some(),
        Err(error) => {
            let error = format!("Run feed read failed; showing the last good read. {error}");
            let changed = feed.error.as_ref() != Some(&error);
            feed.error = Some(error);
            changed
        }
    };
    if changed {
        view.revision += 1;
        app.updates.send_replace(view.revision);
    }
}

/// Reads `href` under a feed directory. The href is checked already; this
/// also refuses any path, symlinks included, whose real location is outside
/// the directory's real location, and any file over `MAX_BYTES`.
pub(super) fn read_dir(root: &Path, href: &str) -> Result<Vec<u8>, String> {
    let fail = |e: std::io::Error| format!("run feed {href}: {e}");
    let root = root.canonicalize().map_err(fail)?;
    let real = root.join(href).canonicalize().map_err(fail)?;
    if !real.starts_with(&root) {
        return Err(format!("run feed href {href:?} leaves the feed root"));
    }
    let size = std::fs::metadata(&real).map_err(fail)?.len();
    if size > MAX_BYTES as u64 {
        return Err(format!(
            "run feed {href}: {size} bytes is more than the {MAX_BYTES} a file may have"
        ));
    }
    std::fs::read(&real).map_err(fail)
}

/// Reads `href` from a feed served over HTTP. Blocking: run it where
/// `Source::read` runs, in `spawn_blocking`.
pub(super) fn read_url(
    base: &str,
    token_file: Option<&Path>,
    href: &str,
) -> Result<Vec<u8>, String> {
    let http = Http::new(base, token_file.map(Path::to_path_buf))?;
    let get = async {
        match http.get(href, None).await? {
            Got::Body(bytes, _) => Ok(bytes),
            Got::NotModified => Err(format!("run feed {href}: 304 to an unconditional read")),
        }
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(get),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(get),
    }
}

/// Every byte of each segment outside `A-Z a-z 0-9 - . _ ~` as `%XX`, so a
/// file named `a%2Fb.json` is asked for as `a%252Fb.json`.
pub(crate) fn encode_href(href: &str) -> String {
    href.split('/')
        .map(|segment| {
            let mut out = String::new();
            for b in segment.bytes() {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    out.push(b as char);
                } else {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A feed served over plain HTTP: on loopback, or a tailnet, which
/// encrypts on its own.
struct Http {
    base: String,
    token_file: Option<PathBuf>,
    client: Client<HttpConnector, Empty<Bytes>>,
}

enum Got {
    Body(Vec<u8>, Option<String>),
    NotModified,
}

impl Http {
    fn new(base: &str, token_file: Option<PathBuf>) -> Result<Self, String> {
        let uri: hyper::Uri = base.parse().map_err(|e| format!("--runs {base}: {e}"))?;
        if uri.scheme_str() != Some("http") {
            return Err(format!(
                "--runs {base}: only http:// feeds can be read (serve the feed on loopback or a tailnet)"
            ));
        }
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token_file,
            client: Client::builder(TokioExecutor::new())
                .pool_max_idle_per_host(0)
                .build_http(),
        })
    }

    fn token(&self) -> Result<Option<String>, String> {
        let Some(path) = &self.token_file else {
            return Ok(None);
        };
        // Re-read every time, so a rotated token file takes effect at once.
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("--runs-token-file {}: {e}", path.display()))?;
        Ok(text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(String::from))
    }

    async fn get(&self, href: &str, etag: Option<&str>) -> Result<Got, String> {
        super::check_href(href)?;
        let url = format!("{}/{}", self.base, encode_href(href));
        let mut request = hyper::Request::get(&url).header(header::ACCEPT, "application/json, */*");
        if let Some(token) = self.token()? {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let request = request.body(Empty::new()).map_err(|e| e.to_string())?;
        let fail = |e: String| format!("run feed GET {url}: {e}");
        tokio::time::timeout(TIMEOUT, async {
            let response = self.client.request(request).await.map_err(|e| {
                if e.is_connect() {
                    format!("cannot reach the run feed at {} (is it served?)", self.base)
                } else {
                    fail(e.to_string())
                }
            })?;
            let status = response.status();
            if status == hyper::StatusCode::NOT_MODIFIED && etag.is_some() {
                return Ok(Got::NotModified);
            }
            let tag = response
                .headers()
                .get(header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let body = Limited::new(response.into_body(), MAX_BYTES)
                .collect()
                .await
                .map_err(|e| fail(format!("{e} (at most {MAX_BYTES} bytes are read)")))?
                .to_bytes();
            if status != hyper::StatusCode::OK {
                let text: String = String::from_utf8_lossy(&body).chars().take(200).collect();
                return Err(fail(format!("HTTP {status} {}", text.trim())));
            }
            Ok(Got::Body(body.to_vec(), tag))
        })
        .await
        .map_err(|_| fail(format!("no answer within {}s", TIMEOUT.as_secs())))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetched_run_docs_register_every_attempt() {
        let source =
            Source::Dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runfeed/v1"));
        let mut feed = model::Feed {
            data: Arc::new(super::super::load(&source).unwrap()),
            error: None,
        };
        assert!(
            feed.data.by_attempt("att-0002-1").is_none(),
            "only current attempts"
        );
        assert_eq!(feed.data.by_attempt("att-0002-2").unwrap().run, "run-0002");
        let run: super::super::Run = source.json("runs/run-0002.json").unwrap();
        register(&mut feed, &run);
        assert_eq!(feed.data.by_attempt("att-0002-1").unwrap().run, "run-0002");
        let before = Arc::as_ptr(&feed.data);
        register(&mut feed, &run);
        assert_eq!(before, Arc::as_ptr(&feed.data), "nothing new, no copy");
    }

    #[test]
    fn segments_are_encoded_and_files_stay_under_the_root() {
        assert_eq!(encode_href("runs/run-0001.json"), "runs/run-0001.json");
        assert_eq!(encode_href("subjects/a%2Fb.json"), "subjects/a%252Fb.json");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runfeed/v1");
        assert!(read_dir(&root, "feed.json").is_ok());
        let scratch =
            std::env::temp_dir().join(format!("fabriscope-read-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.join("feed")).unwrap();
        std::fs::write(scratch.join("secret.json"), "{}").unwrap();
        std::os::unix::fs::symlink(scratch.join("secret.json"), scratch.join("feed/out.json"))
            .unwrap();
        std::fs::write(scratch.join("feed/in.json"), "{}").unwrap();
        std::os::unix::fs::symlink(
            scratch.join("feed/in.json"),
            scratch.join("feed/alias.json"),
        )
        .unwrap();
        let feed = scratch.join("feed");
        assert!(
            read_dir(&feed, "out.json")
                .unwrap_err()
                .contains("leaves the feed root")
        );
        assert_eq!(read_dir(&feed, "alias.json").unwrap(), b"{}");
        let source = Source::Dir(feed);
        assert!(source.read("../secret.json").is_err());
        assert!(source.read("out.json").is_err());
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(Http::new("https://feed.example", None).is_err());
        assert!(
            read_url("http://127.0.0.1:9", None, "feed.json")
                .unwrap_err()
                .contains("cannot reach")
        );
    }
}
