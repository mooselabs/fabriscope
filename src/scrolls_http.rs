//! Read-only Scrolls HTTP transport and ownership of private servers.
use crate::{scrolls, tools::Tool};
use http_body_util::{BodyExt, Empty};
use hyper::{Uri, body::Bytes, header};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    net::{IpAddr, SocketAddr},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    sync::mpsc,
};

const UPGRADE: &str = "Upgrade Scrolls to a build with `scrolls serve` and the `graph` capability (GET /v0/graph), then restart the server. Fabriscope has no render fallback.";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE: usize = 256 * 1024 * 1024;
struct Owned {
    child: Mutex<Option<Child>>,
    drains: Vec<tokio::task::AbortHandle>,
}
impl Owned {
    async fn stop(&self) {
        let child = self.child.lock().unwrap().take();
        if let Some(mut child) = child {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        for drain in &self.drains {
            drain.abort();
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.child.get_mut().unwrap().take(); // Child is kill_on_drop.
        for drain in &self.drains {
            drain.abort();
        }
    }
}
#[derive(Clone)]
pub struct Api {
    base: String,
    client: Client<HttpConnector, Empty<Bytes>>,
    server: Option<Arc<Owned>>,
    identity: Option<(String, String)>,
    etag: Option<String>,
}
#[derive(Deserialize)]
struct Envelope<T> {
    schema_version: u32,
    ok: bool,
    store_id: String,
    branch: String,
    revision: String,
    schema_revision: String,
    data: T,
}
impl<T> Envelope<T> {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 2 || !self.ok {
            return Err(format!("Incompatible Scrolls envelope. {UPGRADE}"));
        }
        let id = self.store_id.as_bytes();
        let uuid = id.len() == 36
            && id.iter().enumerate().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    *c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }
            });
        if !uuid
            || !(self.branch == "main"
                || self
                    .branch
                    .strip_prefix("proposal/")
                    .is_some_and(|s| !s.is_empty()))
            || self.revision.is_empty()
            || self.schema_revision.is_empty()
        {
            return Err("Invalid Scrolls snapshot identity or revision".into());
        }
        Ok(())
    }
}
#[derive(Deserialize)]
struct Discovery {
    api_version: String,
    read_only: bool,
    operations: Vec<String>,
    store_id: String,
    branch: String,
}
fn endpoint(base: &str) -> Result<String, String> {
    let uri: Uri = base
        .parse()
        .map_err(|_| "--scrolls-url must be a loopback HTTP URL".to_string())?;
    let host = uri.host().unwrap_or("").trim_matches(['[', ']']);
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if uri.scheme_str() != Some("http")
        || !loopback
        || uri.authority().is_none_or(|a| a.as_str().contains('@'))
        || !matches!(uri.path(), "" | "/")
        || uri.query().is_some()
        || base.contains('#')
        || uri.port_u16() == Some(0)
    {
        return Err("--scrolls-url requires http://LOOPBACK:PORT with no credentials, path, query or fragment".into());
    }
    // Avoid DNS for localhost: the connection must stay on loopback.
    let host = if host.eq_ignore_ascii_case("localhost") {
        "127.0.0.1"
    } else {
        uri.host().unwrap()
    };
    Ok(format!("http://{host}:{}", uri.port_u16().unwrap_or(80)))
}
impl Api {
    pub fn new(base: &str) -> Result<Self, String> {
        Ok(Self {
            base: endpoint(base)?,
            client: Client::builder(TokioExecutor::new()).build_http(),
            server: None,
            identity: None,
            etag: None,
        })
    }
    pub async fn connect(base: &str) -> Result<Self, String> {
        let mut api = Self::new(base)?;
        api.discover().await?;
        Ok(api)
    }
    pub async fn stop(&self) {
        if let Some(server) = &self.server {
            server.stop().await;
        }
    }
    async fn get(
        &self,
        path: &str,
        etag: Option<&str>,
    ) -> Result<(u16, Option<String>, Bytes), String> {
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            let mut req = hyper::Request::get(format!("{}{path}", self.base))
                .header(header::ACCEPT, "application/json");
            if let Some(etag) = etag {
                req = req.header(header::IF_NONE_MATCH, etag);
            }
            let response = self
                .client
                .request(req.body(Empty::new()).map_err(|e| e.to_string())?)
                .await
                .map_err(|_| format!("Cannot reach scrolls serve at {}", self.base))?;
            let status = response.status().as_u16();
            let tag = response
                .headers()
                .get(header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            if status == 200
                && !response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.split(';').next() == Some("application/json"))
            {
                return Err("Scrolls returned a non-JSON response".into());
            }
            let body = http_body_util::Limited::new(response.into_body(), MAX_RESPONSE)
                .collect()
                .await
                .map_err(|_| "Scrolls response was incomplete or exceeded 256 MiB".to_string())?
                .to_bytes();
            Ok((status, tag, body))
        })
        .await
        .map_err(|_| format!("Scrolls GET {path} timed out after 120s"))?
    }
    async fn discover(&mut self) -> Result<(), String> {
        let (status, _, body) = self.get("/v0/context", None).await?;
        if status != 200 {
            return Err(format!(
                "Scrolls discovery returned HTTP {status}. {UPGRADE}"
            ));
        }
        let e: Envelope<Discovery> = serde_json::from_slice(&body)
            .map_err(|_| format!("Invalid Scrolls discovery. {UPGRADE}"))?;
        e.validate()?;
        if e.data.api_version != "v0"
            || !e.data.read_only
            || !e.data.operations.iter().any(|s| s == "graph")
            || e.data.store_id != e.store_id
            || e.data.branch != e.branch
        {
            return Err(format!("Incompatible Scrolls discovery. {UPGRADE}"));
        }
        self.identity = Some((e.store_id, e.branch));
        Ok(())
    }
    /// The validator moves only after validation of the complete snapshot.
    pub async fn refresh(&mut self) -> Result<Option<scrolls::Snapshot>, String> {
        let (status, tag, body) = self.get("/v0/graph", self.etag.as_deref()).await?;
        if status == 304 {
            if self.etag.is_none() || tag != self.etag || !body.is_empty() {
                return Err("Invalid Scrolls 304 response".into());
            }
            return Ok(None);
        }
        if status != 200 {
            return Err(if status == 404 {
                format!("Scrolls graph endpoint is missing. {UPGRADE}")
            } else {
                format!("Scrolls graph read returned HTTP {status}")
            });
        }
        let tag = tag
            .filter(|t| t.starts_with('"') && t.ends_with('"') && t.len() > 2)
            .ok_or("Scrolls graph response has no strong ETag")?;
        let e: Envelope<Value> =
            serde_json::from_slice(&body).map_err(|_| "Invalid Scrolls graph envelope")?;
        e.validate()?;
        if self.identity.as_ref() != Some(&(e.store_id.clone(), e.branch.clone())) {
            return Err("Scrolls store or branch changed; restart Fabriscope to select it".into());
        }
        let mut snapshot = scrolls::parse_graph_value(e.data, Some(e.revision))?;
        snapshot.source = format!("scrolls serve {}", self.base);
        snapshot.store_id = e.store_id;
        snapshot.branch = e.branch;
        snapshot.schema_revision = e.schema_revision;
        let unchanged = self.etag.as_ref() == Some(&tag);
        self.etag = Some(tag);
        Ok((!unchanged).then_some(snapshot))
    }
}
fn startup(line: &[u8]) -> Result<String, String> {
    let v: Value = serde_json::from_slice(line)
        .map_err(|_| format!("Invalid Scrolls startup JSON. {UPGRADE}"))?;
    if v["schema_version"] != 2
        || v["ok"] != true
        || v["event"] != "listening"
        || v["api_version"] != "v0"
        || v["read_only"] != true
    {
        return Err(format!("Incompatible Scrolls startup event. {UPGRADE}"));
    }
    let address: SocketAddr = v["address"]
        .as_str()
        .ok_or("Scrolls startup lacks an address")?
        .parse()
        .map_err(|_| "Invalid Scrolls startup address")?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("Scrolls startup address is not a bound loopback listener".into());
    }
    Ok(format!("http://{address}"))
}
/// Drain bounded chunks; only stdout's first nonempty line is a startup event.
/// Do not retain stderr's raw backend diagnostics or document text.
fn drain(
    mut pipe: impl AsyncRead + Unpin + Send + 'static,
    first: Option<mpsc::Sender<Result<String, String>>>,
) -> tokio::task::AbortHandle {
    tokio::spawn(async move {
        let mut first = first;
        let mut line = Vec::new();
        let mut buf = [0; 4096];
        while let Ok(n) = pipe.read(&mut buf).await {
            if n == 0 {
                break;
            }
            if first.is_none() {
                continue;
            }
            for &byte in &buf[..n] {
                if first.is_none() {
                    break;
                }
                if byte == b'\n' {
                    if !line.is_empty() {
                        let _ = first.take().unwrap().try_send(startup(&line));
                    }
                } else if line.len() == 65536 {
                    let _ = first.take().unwrap().try_send(Err(format!(
                        "Scrolls startup event exceeds 64 KiB. {UPGRADE}"
                    )));
                } else {
                    line.push(byte);
                }
            }
        }
    })
    .abort_handle()
}
pub async fn start(tool: &Tool) -> Result<Api, String> {
    start_with_timeout(tool, STARTUP_TIMEOUT).await
}
async fn start_with_timeout(tool: &Tool, timeout: Duration) -> Result<Api, String> {
    let mut cmd = Command::new(&tool.program);
    cmd.args(&tool.args)
        .args(["--json", "serve", "--bind", "127.0.0.1:0"])
        .current_dir(&tool.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    unsafe {
        let parent = std::process::id() as libc::pid_t;
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::raise(libc::SIGTERM);
            }
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start Scrolls: {e}. {UPGRADE}"))?;
    let (send, mut receive) = mpsc::channel(1);
    let drains = vec![
        drain(child.stdout.take().unwrap(), Some(send)),
        drain(child.stderr.take().unwrap(), None),
    ];
    let owner = Arc::new(Owned {
        child: Mutex::new(Some(child)),
        drains,
    });
    let result = tokio::time::timeout(timeout, async {
        let url = receive.recv().await.ok_or_else(|| format!("Scrolls exited without a startup event. Check the store and --scrolls-dolt. {UPGRADE}"))??;
        let mut api = Api::new(&url)?; api.server = Some(owner.clone()); api.discover().await?; Ok(api)
    }).await.unwrap_or_else(|_| Err(format!("Scrolls startup timed out. Check the store and --scrolls-dolt. {UPGRADE}")));
    if result.is_err() {
        owner.stop().await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        App,
        model::{Feed, View},
        refresh_scrolls,
    };
    use axum::{Router, extract::State, http::HeaderMap, response::IntoResponse, routing::get};
    use serde_json::json;

    #[test]
    fn endpoints_and_startup_are_strict() {
        for url in [
            "https://127.0.0.1:5",
            "http://example.org:5",
            "http://0.0.0.0:5",
            "http://127.0.0.1:5/x",
            "http://user@127.0.0.1:5",
            "http://127.0.0.1:5?x",
            "http://127.0.0.1:5#x",
            "http://127.0.0.1:0",
        ] {
            assert!(endpoint(url).is_err(), "{url}");
        }
        for url in [
            "http://127.0.0.1:5",
            "http://[::1]:5",
            "http://localhost:5/",
        ] {
            assert!(endpoint(url).is_ok(), "{url}");
        }
        let event = json!({"schema_version":2,"ok":true,"event":"listening","address":"127.0.0.1:5","api_version":"v0","read_only":true});
        assert_eq!(
            startup(event.to_string().as_bytes()).unwrap(),
            "http://127.0.0.1:5"
        );
        for (key, val) in [
            ("schema_version", json!(1)),
            ("ok", json!(false)),
            ("event", json!("ready")),
            ("read_only", json!(false)),
            ("address", json!("0.0.0.0:5")),
            ("address", json!("127.0.0.1:0")),
        ] {
            let mut bad = event.clone();
            bad[key] = val;
            assert!(startup(bad.to_string().as_bytes()).is_err());
        }
        assert!(
            startup(b"not json")
                .unwrap_err()
                .contains("Upgrade Scrolls")
        );
    }
    struct Fixture {
        graph: Value,
        tag: String,
        status: u16,
        identity: String,
        seen: Vec<Option<String>>,
    }
    fn envelope(data: Value) -> Value {
        json!({"schema_version":2,"ok":true,"store_id":"00000000-0000-4000-8000-000000000001","branch":"main","revision":"rev","schema_revision":"schema","data":data})
    }
    async fn discovery() -> impl IntoResponse {
        axum::Json(envelope(
            json!({"api_version":"v0","read_only":true,"operations":["graph"],"store_id":"00000000-0000-4000-8000-000000000001","branch":"main"}),
        ))
    }
    async fn graph(
        State(state): State<Arc<Mutex<Fixture>>>,
        headers: HeaderMap,
    ) -> axum::response::Response {
        let mut s = state.lock().unwrap();
        let tag = headers
            .get("if-none-match")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        s.seen.push(tag.clone());
        let status = if s.status == 200 && tag.as_ref() == Some(&s.tag) {
            304
        } else {
            s.status
        };
        let mut value = envelope(s.graph.clone());
        value["store_id"] = json!(s.identity);
        (
            axum::http::StatusCode::from_u16(status).unwrap(),
            [("content-type", "application/json"), ("etag", &s.tag)],
            if status == 304 {
                String::new()
            } else {
                value.to_string()
            },
        )
            .into_response()
    }
    #[tokio::test]
    async fn conditional_validation_failure_and_recovery_are_atomic() {
        let state = Arc::new(Mutex::new(Fixture {
            graph: serde_json::from_str(include_str!("../tests/fixtures/graph.json")).unwrap(),
            tag: "\"one\"".into(),
            status: 200,
            identity: "00000000-0000-4000-8000-000000000001".into(),
            seen: vec![],
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let routes = Router::new()
            .route("/v0/context", get(discovery))
            .route("/v0/graph", get(graph))
            .with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        let mut api = Api::connect(&url).await.unwrap();
        let first = api.refresh().await.unwrap().unwrap();
        let app = App::new(View {
            scrolls: Some(Feed {
                data: Arc::new(first),
                error: None,
            }),
            revision: 1,
            ..Default::default()
        });
        let data = app.view.read().await.scrolls.as_ref().unwrap().data.clone();
        refresh_scrolls(&app, &mut api).await;
        assert_eq!(app.view.read().await.revision, 1);
        assert!(app.view.read().await.scrolls_checked_at.is_some());
        assert!(Arc::ptr_eq(
            &data,
            &app.view.read().await.scrolls.as_ref().unwrap().data
        ));
        let last_check = app.view.read().await.scrolls_checked_at.clone();
        let good = state.lock().unwrap().graph.clone();
        for bad in [
            json!({}),
            json!({"records":{},"edges":[]}),
            json!({"records":{"bad":{"id":"wrong"}},"edges":{}}),
        ] {
            {
                let mut s = state.lock().unwrap();
                s.graph = bad;
                s.tag = "\"invalid\"".into();
            }
            refresh_scrolls(&app, &mut api).await;
            let view = app.view.read().await;
            assert!(view.scrolls.as_ref().unwrap().error.is_some());
            assert!(Arc::ptr_eq(&data, &view.scrolls.as_ref().unwrap().data));
            assert_eq!(view.scrolls_checked_at, last_check);
            assert_eq!(api.etag.as_deref(), Some("\"one\""));
        }
        {
            let mut s = state.lock().unwrap();
            s.graph = good.clone();
            s.tag = "\"one\"".into();
        }
        refresh_scrolls(&app, &mut api).await; // 304 clears the warning.
        assert!(
            app.view
                .read()
                .await
                .scrolls
                .as_ref()
                .unwrap()
                .error
                .is_none()
        );
        state.lock().unwrap().status = 503;
        refresh_scrolls(&app, &mut api).await;
        assert!(
            app.view
                .read()
                .await
                .scrolls
                .as_ref()
                .unwrap()
                .error
                .is_some()
        );
        {
            let mut s = state.lock().unwrap();
            s.status = 200;
            s.tag = "\"two\"".into();
            s.graph["records"]["sc-d1"]["title"] = json!("Changed");
        }
        refresh_scrolls(&app, &mut api).await;
        let view = app.view.read().await;
        assert!(view.scrolls.as_ref().unwrap().error.is_none());
        assert_eq!(
            view.scrolls
                .as_ref()
                .unwrap()
                .data
                .get("sc-d1")
                .unwrap()
                .title,
            "Changed"
        );
        drop(view);
        {
            let mut s = state.lock().unwrap();
            s.identity = "00000000-0000-4000-8000-000000000002".into();
            s.tag = "\"three\"".into();
        }
        refresh_scrolls(&app, &mut api).await;
        assert!(
            app.view
                .read()
                .await
                .scrolls
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("store or branch changed")
        );
        assert_eq!(api.etag.as_deref(), Some("\"two\""));
        assert_eq!(state.lock().unwrap().seen[1].as_deref(), Some("\"one\""));
        api.stop().await;
        assert!(
            Api::connect(&url).await.is_ok(),
            "attached server must survive stop"
        );
        task.abort();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn startup_timeout_and_invalid_event_reap_children() {
        let root = std::env::temp_dir().join(format!(
            "fabriscope-start-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        for (name, code) in [
            ("hang", "import time; time.sleep(300)"),
            (
                "bad",
                "import time; print('broken',flush=True); time.sleep(300)",
            ),
        ] {
            let pid = root.join(name);
            let tool = Tool {
                name: "scrolls",
                program: "python3".into(),
                args: vec![
                    "-c".into(),
                    format!(
                        "import os,pathlib; pathlib.Path({:?}).write_text(str(os.getpid())); {code}",
                        pid.to_str().unwrap()
                    ),
                ],
                cwd: root.clone(),
            };
            let result = start_with_timeout(&tool, Duration::from_secs(2)).await;
            assert!(result.is_err());
            #[cfg(target_os = "linux")]
            {
                let pid = std::fs::read_to_string(pid).unwrap();
                assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
