//! Talks to the Beads HTTP API that `bd serve` provides, and can start a
//! private `bd serve` for a directory.

use crate::tools::Tool;
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;
use serde::Deserialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

/// A `bd serve` endpoint.
#[derive(Clone)]
pub struct Api {
    base: String,
    token_file: Option<PathBuf>,
    client: Client<HttpConnector, Empty<Bytes>>,
    /// Keeps a server Fabriscope started alive; it stops when the last
    /// clone is dropped.
    _server: Option<Arc<Child>>,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api").field("base", &self.base).finish()
    }
}

/// An HTTP answer other than 200, with the API's problem code if it sent one.
#[derive(Debug)]
pub struct Problem {
    pub status: u16,
    pub code: String,
    pub param: String,
    pub detail: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {} {}: {}", self.status, self.code, self.detail)
    }
}

#[derive(Deserialize)]
struct Page<T> {
    items: Vec<T>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct ProblemBody {
    #[serde(default)]
    code: String,
    #[serde(default)]
    param: String,
    #[serde(default)]
    detail: String,
}

impl Api {
    pub fn new(base: &str, token_file: Option<PathBuf>) -> Result<Self, String> {
        let base = base.trim_end_matches('/').to_string();
        let uri: hyper::Uri = base
            .parse()
            .map_err(|e| format!("--beads-url {base}: {e}"))?;
        if uri.scheme_str() != Some("http") {
            return Err(format!(
                "--beads-url {base}: only http:// is supported (bd serve has no TLS)"
            ));
        }
        Ok(Self {
            base,
            token_file,
            client: Client::builder(TokioExecutor::new()).build_http(),
            _server: None,
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn token(&self) -> Result<Option<String>, String> {
        let Some(path) = &self.token_file else {
            return Ok(None);
        };
        // Re-read every time, so a rotated token file takes effect at once.
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("--beads-token-file {}: {e}", path.display()))?;
        Ok(text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(String::from))
    }

    /// GET a path; `Ok(Err(problem))` is an answer the server gave.
    pub async fn get(
        &self,
        path_and_query: &str,
        timeout_secs: u64,
    ) -> Result<Result<Bytes, Problem>, String> {
        let mut request = hyper::Request::get(format!("{}{path_and_query}", self.base))
            .header(hyper::header::ACCEPT, "application/json");
        if let Some(token) = self.token()? {
            request = request.header(hyper::header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = request.body(Empty::new()).map_err(|e| e.to_string())?;
        let response = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
            let response = self.client.request(request).await.map_err(|e| {
                if e.is_connect() {
                    format!("cannot reach bd serve at {} (is it running?)", self.base)
                } else {
                    e.to_string()
                }
            })?;
            let status = response.status().as_u16();
            let body = response
                .into_body()
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .to_bytes();
            Ok::<_, String>((status, body))
        })
        .await
        .map_err(|_| format!("GET {path_and_query} took longer than {timeout_secs}s"))?
        .map_err(|e| format!("GET {path_and_query}: {e}"))?;
        let (status, body) = response;
        if status == 200 {
            return Ok(Ok(body));
        }
        let problem: ProblemBody = serde_json::from_slice(&body).unwrap_or(ProblemBody {
            code: String::new(),
            param: String::new(),
            detail: String::from_utf8_lossy(&body).chars().take(300).collect(),
        });
        Ok(Err(Problem {
            status,
            code: problem.code,
            param: problem.param,
            detail: problem.detail,
        }))
    }

    async fn get_ok(&self, path_and_query: &str, timeout_secs: u64) -> Result<Bytes, String> {
        self.get(path_and_query, timeout_secs)
            .await?
            .map_err(|p| format!("bd serve GET {path_and_query}: {p}"))
    }

    /// Every item of a list operation. Asks for one unlimited page, and pages
    /// with the cursor when the server refuses unlimited reads (a server
    /// bound beyond loopback does).
    pub async fn list<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &str,
    ) -> Result<Vec<T>, String> {
        let unlimited = format!("{path}?{query}&limit=0");
        match self.get(&unlimited, 300).await? {
            Ok(body) => return parse_page::<T>(&body, path).map(|p| p.items),
            Err(p) if p.status == 400 && p.param == "limit" => {}
            Err(p) => return Err(format!("bd serve GET {unlimited}: {p}")),
        }
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut request = format!("{path}?{query}&limit=500");
            if let Some(cursor) = &cursor {
                request.push_str(&format!(
                    "&cursor={}",
                    percent_encoding::utf8_percent_encode(
                        cursor,
                        percent_encoding::NON_ALPHANUMERIC
                    )
                ));
            }
            let page = parse_page::<T>(&self.get_ok(&request, 300).await?, path)?;
            items.extend(page.items);
            match (page.has_more, page.next_cursor) {
                (true, Some(next)) => cursor = Some(next),
                (true, None) => {
                    return Err(format!("bd serve {path}: has_more without next_cursor"));
                }
                (false, _) => return Ok(items),
            }
        }
    }

    /// A cheap change signal: the stats summary, plus the events journal
    /// head when the journal is enabled. Neither catches every edit (stats
    /// are counts; the journal records only writes made through this
    /// server), so the caller also re-reads on a timer.
    pub async fn probe(&self) -> Result<String, String> {
        let stats = self.get_ok("/v0/beads/stats", 15).await?;
        let mut signal = String::from_utf8_lossy(&stats).into_owned();
        if let Ok(Ok(body)) = self.get("/v0/beads/events?since=0&limit=1", 15).await {
            if let Ok(value) = serde_json::from_slice::<Value>(&body) {
                signal.push_str(&format!(" head={}", value["head"]));
            }
        }
        Ok(signal)
    }
}

fn parse_page<T: serde::de::DeserializeOwned>(body: &[u8], path: &str) -> Result<Page<T>, String> {
    serde_json::from_slice(body).map_err(|e| format!("bd serve {path}: {e}"))
}

/// The last lines a starting server printed, for an error message.
#[derive(Clone, Default)]
struct Tail(Arc<Mutex<VecDeque<String>>>);

impl Tail {
    fn push(&self, line: String) {
        let mut lines = self.0.lock().unwrap();
        if lines.len() == 200 {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    /// bd's own `Error:` lines when it printed any (it follows them with
    /// its usage text), otherwise the last few lines it printed.
    fn text(&self) -> String {
        let lines = self.0.lock().unwrap();
        let errors: Vec<&str> = lines
            .iter()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("Error:") && !l.starts_with("Error: exit code"))
            .collect();
        if !errors.is_empty() {
            return errors.join(" ");
        }
        let rest: Vec<&str> = lines
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with("Warning:"))
            .collect();
        rest[rest.len().saturating_sub(3)..].join(" ")
    }
}

fn drain(
    stream: impl AsyncRead + Unpin + Send + 'static,
    tail: Tail,
    found: mpsc::Sender<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = line.split("listening on ").nth(1) {
                let _ = found.send(url.trim().to_string()).await;
            }
            tail.push(line);
        }
    })
}

/// Starts `bd serve` on a free loopback port in the tool's directory and
/// waits for its address. The error is bd's own reason for refusing.
pub async fn start(tool: &Tool, token_file: Option<PathBuf>) -> Result<Api, String> {
    let mut command = Command::new(&tool.program);
    command
        .args(&tool.args)
        .args(["serve", "--addr", "127.0.0.1:0"])
        .current_dir(&tool.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // `bd serve` accepts writes, so it must not outlive Fabriscope even when
    // Fabriscope is killed outright. The kernel signal is tied to the thread
    // that spawns the child, which is the main thread here (startup runs on
    // it before any worker takes over).
    #[cfg(target_os = "linux")]
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not run {}: {e}", tool.program.display()))?;
    let tail = Tail::default();
    let (found, mut address) = mpsc::channel(2);
    let readers = [
        drain(child.stdout.take().unwrap(), tail.clone(), found.clone()),
        drain(child.stderr.take().unwrap(), tail.clone(), found),
    ];
    let started = tokio::time::timeout(Duration::from_secs(60), async {
        // No address can mean the process exited (its pipes closed first), so
        // either way wait for the exit and report what bd printed.
        let url = tokio::select! {
            url = address.recv() => url,
            _ = child.wait() => None,
        };
        if let Some(url) = url {
            return Ok(url);
        }
        let status = child
            .wait()
            .await
            .map(|s| s.to_string())
            .unwrap_or_default();
        for reader in readers {
            let _ = tokio::time::timeout(Duration::from_secs(2), reader).await;
        }
        Err(format!("bd serve exited ({status}): {}", tail.text()))
    })
    .await
    .map_err(|_| "bd serve did not report an address within 60s".to_string())??;
    let mut api = Api::new(&started, token_file)?;
    api._server = Some(Arc::new(child));
    Ok(api)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_http_urls_are_accepted() {
        assert!(Api::new("http://127.0.0.1:4000/", None).is_ok());
        assert_eq!(
            Api::new("http://127.0.0.1:4000/", None).unwrap().base(),
            "http://127.0.0.1:4000"
        );
        assert!(Api::new("https://beads.example", None).is_err());
        assert!(Api::new("not a url", None).is_err());
    }

    #[test]
    fn pages_parse_and_tails_skip_noise() {
        let page: Page<Value> = parse_page(
            br#"{"items":[{"id":"a"}],"has_more":true,"next_cursor":"c"}"#,
            "/x",
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(page.has_more);
        assert!(parse_page::<Value>(b"[]", "/x").is_err());
        let tail = Tail::default();
        for line in [
            "Warning: permissions",
            "Error: not a git repository",
            "Error: exit code 1",
            "Usage:",
            "  bd serve [flags]",
            "      --json   Output in JSON format",
        ] {
            tail.push(line.into());
        }
        assert_eq!(tail.text(), "Error: not a git repository");
        let quiet = Tail::default();
        for line in ["one", "two", "", "three", "four"] {
            quiet.push(line.into());
        }
        assert_eq!(quiet.text(), "two three four");
    }
}
