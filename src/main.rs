//! Fabriscope: view any Scrolls or Beads store in a web browser.

mod app;
mod beads;
mod costs;
mod filters;
mod flow;
mod history;
mod model;
mod profile;
mod provenance;
mod query;
mod render;
mod routes;
mod runs;
mod scrolls;
mod scrolls_http;
mod serve;
mod sweep;
#[cfg(test)]
mod testing;
mod tools;
mod update;
mod web;

use app::{App, in_progress, watch_beads, watch_scrolls};
use model::{Feed, View};
use routes::router;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tools::Tool;

const USAGE: &str = "Usage: fabriscope [--beads DIR | --beads-url URL] [--scrolls STORE | --scrolls-url URL] [options]
       fabriscope update
       fabriscope profile check FILE

View a Beads store, a Scrolls store, or both, in a web browser. Release
builds update themselves in the background; `fabriscope update` updates now.

  --beads DIR          A directory where `bd` finds the Beads store; Fabriscope
                       starts a private `bd serve` there
  --beads-url URL      A `bd serve` that is already running
  --beads-token-file F Bearer token file for that `bd serve`
  --parent-key PATH    Metadata path naming a bead's parent (by id or external
                       reference), for stores without parent-child links;
                       wins over the profile's [parent]
  --profile FILE       A fabriscope.profile.v1 file saying how the store uses
                       its metadata (see docs/profile.md)
  --runs URL|DIR       A run feed (runfeed.v1, see docs/runfeed-v1.md)
  --runs-token-file F  Bearer token file for a --runs URL
  --scrolls STORE      Start private scrolls serve for this store
  --scrolls-url URL    Attach to an existing loopback scrolls serve
  --bind ADDR          Address to listen on (default 127.0.0.1:8787)
  --title TEXT         A name shown in the header
  --bd PATH            The bd program (default: bd on PATH)
  --scrolls-bin PATH   The scrolls program (default: scrolls on PATH)
  --scrolls-dolt PATH  The Dolt binary Scrolls should use (passed as --dolt)
  --no-update          Do not check for a new release (or set FABRISCOPE_NO_UPDATE=1)
  -V, --version        Show the version
  -h, --help           Show this help";

#[derive(Debug, Default, PartialEq)]
struct Config {
    beads: Option<PathBuf>,
    beads_url: Option<String>,
    beads_token_file: Option<PathBuf>,
    parent_key: Option<String>,
    scrolls: Option<PathBuf>,
    scrolls_url: Option<String>,
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
    profile: Option<PathBuf>,
    runs: Option<String>,
    runs_token_file: Option<PathBuf>,
    /// `fabriscope profile check FILE`: validate, print and exit.
    profile_check: Option<PathBuf>,
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
    if args.peek().map(String::as_str) == Some("profile") {
        args.next();
        return match (args.next().as_deref(), args.next(), args.next()) {
            (Some("check"), Some(file), None) => {
                config.profile_check = Some(file.into());
                Ok(config)
            }
            _ => Err("use: fabriscope profile check FILE".into()),
        };
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
            "--scrolls-url" => config.scrolls_url = Some(value("--scrolls-url")?),
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
            "--profile" => config.profile = Some(value("--profile")?.into()),
            "--runs" => config.runs = Some(value("--runs")?),
            "--runs-token-file" => {
                config.runs_token_file = Some(value("--runs-token-file")?.into())
            }
            "--no-update" => config.no_update = true,
            "--help" | "-h" => config.help = true,
            "--version" | "-V" => config.version = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if config.beads.is_some() && config.beads_url.is_some() {
        return Err("give --beads DIR or --beads-url URL, not both".into());
    }
    if config.scrolls.is_some() && config.scrolls_url.is_some() {
        return Err("give --scrolls STORE or --scrolls-url URL, not both".into());
    }
    if !config.help
        && !config.version
        && config.beads.is_none()
        && config.beads_url.is_none()
        && config.scrolls.is_none()
        && config.scrolls_url.is_none()
    {
        return Err("give --beads DIR (or --beads-url URL), --scrolls STORE (or --scrolls-url URL), or both".into());
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
    if let Some(file) = &config.profile_check {
        let profile = profile::Profile::load(file)?;
        println!("{}: a valid {} profile", file.display(), profile::SCHEMA);
        println!("{}", profile.describe());
        return Ok(());
    }
    let mut profile = match &config.profile {
        Some(file) => profile::Profile::load(file)?,
        None => profile::Profile::default(),
    };
    profile.set_parent_key(config.parent_key.as_deref())?;
    let profile = Arc::new(profile);
    let runs_source = config
        .runs
        .as_deref()
        .map(|r| runs::Source::parse(r, config.runs_token_file.clone()))
        .transpose()?;
    if runs_source.is_none() && config.runs_token_file.is_some() {
        return Err("--runs-token-file needs --runs URL".into());
    }
    let beads_reader = beads_reader(&config).await?;
    let scrolls_store = config
        .scrolls
        .as_deref()
        .map(|s| existing_dir(s, "--scrolls"))
        .transpose()?;
    let mut scrolls_api = if let Some(url) = &config.scrolls_url {
        Some(scrolls_http::Api::connect(url).await?)
    } else if let Some(store) = &scrolls_store {
        Some(scrolls_http::start(&scrolls_tool(&config, store)).await?)
    } else {
        None
    };
    // An unreadable store at startup is an error, not an empty page.
    eprintln!("fabriscope: reading the stores…");
    let first_signal = match &beads_reader {
        Some(reader) => reader.probe().await.map(|(signal, _)| signal),
        None => None,
    };
    let (beads, scrolls) = tokio::join!(
        async {
            match &beads_reader {
                Some(reader) => beads::load(reader, profile.clone()).await.map(Some),
                None => Ok(None),
            }
        },
        async {
            match &mut scrolls_api {
                Some(api) => api.refresh().await,
                None => Ok(None),
            }
        },
    );
    let (beads, scrolls) = match (beads, scrolls) {
        (Ok(beads), Ok(scrolls)) => (beads, scrolls),
        (Err(error), _) | (_, Err(error)) => {
            if let Some(api) = &scrolls_api {
                api.stop().await;
            }
            return Err(error);
        }
    };
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
    let scrolls_checked_at = scrolls.as_ref().map(|s| s.generated_at.clone());
    let provenance_tool = beads_reader.as_ref().and_then(|r| r.local()).cloned();
    let ahead = beads.as_ref().map(in_progress).unwrap_or_default();
    // A directory feed is read once here so pages have it from the start;
    // `runs::client` (WP5) keeps it current. A feed that cannot be read is
    // shown as an error, not refused: it may not be published yet.
    let runs = match &runs_source {
        Some(dir @ runs::Source::Dir(_)) => {
            let source = dir.clone();
            let read = tokio::task::spawn_blocking(move || runs::load(&source))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            Some(match read {
                Ok(index) => Feed {
                    data: Arc::new(index),
                    error: None,
                },
                Err(error) => Feed {
                    data: Arc::default(),
                    error: Some(format!("Run feed read failed. {error}")),
                },
            })
        }
        Some(runs::Source::Url { .. }) => Some(Feed::default()),
        None => None,
    };
    let mut app = App::new(View {
        title,
        beads: beads.map(|data| Feed {
            data: Arc::new(data),
            error: None,
        }),
        scrolls: scrolls.map(|data| Feed {
            data: Arc::new(data),
            error: None,
        }),
        scrolls_checked_at,
        provenance: provenance_tool
            .as_ref()
            .map(|_| provenance::Store::default()),
        revision: 1,
        history: beads_reader.as_ref().map(|_| history::Store::default()),
        runs,
        profile,
        run_cache: Arc::default(),
    });
    let provenance_ready = provenance_tool.is_some();
    if let Some(tool) = provenance_tool {
        app = app.with_provenance(tool);
    }
    let bind = config
        .bind
        .unwrap_or_else(|| "127.0.0.1:8787".parse().unwrap());
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(error) => {
            if let Some(api) = &scrolls_api {
                api.stop().await;
            }
            return Err(format!("cannot listen on {bind}: {error}"));
        }
    };
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    println!("Fabriscope: http://{address}{summary}");
    app.want_provenance(ahead).await;
    if provenance_ready {
        sweep::spawn(app.clone());
    }
    if let Some(reader) = beads_reader {
        history::reader::spawn(app.clone(), &reader);
        tokio::spawn(watch_beads(app.clone(), reader, first_signal));
    }
    if let Some(source) = runs_source {
        runs::client::spawn(app.clone(), source);
    }
    let scrolls_watch = scrolls_api
        .as_ref()
        .map(|api| tokio::spawn(watch_scrolls(app.clone(), api.clone())));
    if update::automatic(config.no_update) {
        tokio::spawn(update::watch(app.installed.clone()));
    }
    // Dropping the HTTP server also closes long-lived SSE connections, so
    // shutdown cannot leave an owned child alive waiting for a browser.
    let result = tokio::select! {
        result = axum::serve(listener, router(app)) => result.map_err(|e| e.to_string()),
        _ = shutdown_signal() => Ok(()),
    };
    if let Some(task) = scrolls_watch {
        task.abort();
        let _ = task.await;
    }
    if let Some(api) = scrolls_api {
        api.stop().await;
    }
    result
}

/// Handle the same termination signals as the owned Scrolls server.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        let mut hup = signal(SignalKind::hangup()).expect("SIGHUP handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {}, _ = hup.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
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
        assert!(args(&["--scrolls-url", "http://127.0.0.1:9"]).is_ok());
        assert!(args(&["--scrolls", "/tmp", "--scrolls-url", "http://127.0.0.1:9"]).is_err());
    }

    #[test]
    fn profile_and_runs_flags_and_the_profile_check_command() {
        let config = args(&["--beads", "/tmp", "--profile", "p.toml", "--runs", "feed"]).unwrap();
        assert_eq!(config.profile, Some("p.toml".into()));
        assert_eq!(config.runs.as_deref(), Some("feed"));
        let check = args(&["profile", "check", "p.toml"]).unwrap();
        assert_eq!(check.profile_check, Some("p.toml".into()));
        assert!(args(&["profile", "check"]).is_err());
        assert!(args(&["profile", "lint", "p.toml"]).is_err());
        assert!(args(&["profile", "check", "a", "b"]).is_err());
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
}
