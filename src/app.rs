//! The shared state every page reads, and the background work that keeps it
//! current: re-reading the stores and reading provenance logs ahead.

use crate::model::{Feed, View};
use crate::tools::{self, Tool};
use crate::{beads, provenance, scrolls_http};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore, watch};

#[derive(Clone)]
pub(crate) struct App {
    pub(crate) view: Arc<RwLock<View>>,
    pub(crate) updates: watch::Sender<u64>,
    /// A newer release installed on disk by the background update check.
    pub(crate) installed: Arc<Mutex<Option<String>>>,
    /// Reads provenance logs, when a local `bd` can.
    provenance: Option<Arc<Reads>>,
}

/// The local `bd` and how many provenance reads may run at once. Each read
/// is its own `bd` process, which can take seconds against a remote server.
struct Reads {
    tool: Tool,
    permits: Semaphore,
}

/// Beads whose provenance is read ahead of any visit: the ones in progress.
const AHEAD: usize = 100;

impl App {
    pub(crate) fn new(view: View) -> Self {
        let (updates, _) = watch::channel(view.revision);
        Self {
            view: Arc::new(RwLock::new(view)),
            updates,
            installed: Arc::default(),
            provenance: None,
        }
    }

    pub(crate) fn with_provenance(mut self, tool: Tool) -> Self {
        self.provenance = Some(Arc::new(Reads {
            tool,
            permits: Semaphore::new(4),
        }));
        self
    }

    /// Starts a background read of each named bead's provenance log that is
    /// missing or stale. Pages render the last read straight away; a read
    /// that changes what they show bumps the revision, and they refresh.
    pub(crate) async fn want_provenance(&self, ids: Vec<String>) {
        let Some(reads) = &self.provenance else {
            return;
        };
        let now = chrono::Utc::now();
        let mut started = Vec::new();
        {
            let mut view = self.view.write().await;
            let View {
                beads, provenance, ..
            } = &mut *view;
            let (Some(feed), Some(store)) = (beads.as_ref(), provenance.as_mut()) else {
                return;
            };
            for id in ids {
                if let Some(bead) = feed.data.get(&id) {
                    if store.wants_read(bead, now) {
                        store.reading.insert(id.clone());
                        started.push((id, bead.updated_at.clone()));
                    }
                }
            }
        }
        for (id, updated_at) in started {
            tokio::spawn(read_provenance(self.clone(), reads.clone(), id, updated_at));
        }
    }

    pub(crate) async fn update(&self, change: impl FnOnce(&mut View)) {
        let mut view = self.view.write().await;
        change(&mut view);
        view.revision += 1;
        self.updates.send_replace(view.revision);
    }
}

async fn read_provenance(app: App, reads: Arc<Reads>, id: String, updated_at: Option<String>) {
    let result = match reads.permits.acquire().await {
        // `--` ends bd's options, so no id is read as one.
        Ok(_permit) => tools::run(
            &reads.tool,
            &["provenance", "log", "--json", "--", &id],
            120,
        )
        .await
        .and_then(|text| provenance::parse(&text)),
        Err(_) => Err("provenance reads stopped".into()),
    };
    let mut view = app.view.write().await;
    let Some(store) = view.provenance.as_mut() else {
        return;
    };
    if store.record(&id, result, updated_at, chrono::Utc::now()) {
        view.revision += 1;
        app.updates.send_replace(view.revision);
    }
}

/// The beads to read provenance for ahead of a visit: those in progress.
pub(crate) fn in_progress(snapshot: &beads::Snapshot) -> Vec<String> {
    snapshot
        .ordered()
        .filter(|b| b.status == "in_progress")
        .take(AHEAD)
        .map(|b| b.id.clone())
        .collect()
}

pub(crate) async fn view_in_progress(app: &App) -> Vec<String> {
    let view = app.view.read().await;
    view.beads
        .as_ref()
        .map(|f| in_progress(&f.data))
        .unwrap_or_default()
}

/// Re-reads Beads when its change signal moves. A commit is a reliable
/// signal, so a timed re-read is only a slow safety net; API stats are a
/// hint, so the timer is shorter; with no signal at all, re-read every 30s.
pub(crate) async fn watch_beads(app: App, reader: beads::Reader, mut last: Option<String>) {
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
        let profile = app.view.read().await.profile.clone();
        let result = beads::load(&reader, profile).await;
        refreshed = Instant::now();
        let ahead = result.as_ref().map(in_progress).unwrap_or_default();
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
        app.want_provenance(ahead).await;
    }
}

/// Single-flight conditional refresh, with the delay measured after completion.
pub(crate) async fn watch_scrolls(app: App, mut api: scrolls_http::Api) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        refresh_scrolls(&app, &mut api).await;
    }
}

pub(crate) async fn refresh_scrolls(app: &App, api: &mut scrolls_http::Api) {
    let result = api.refresh().await;
    let mut view = app.view.write().await;
    let Some(feed) = view.scrolls.as_mut() else {
        return;
    };
    let changed = match result {
        Ok(snapshot) => {
            let recovered = feed.error.take().is_some();
            let changed = snapshot.is_some() || recovered;
            if let Some(snapshot) = snapshot {
                feed.data = Arc::new(snapshot);
            }
            view.scrolls_checked_at = Some(chrono::Utc::now().to_rfc3339());
            changed
        }
        Err(error) => {
            let error = format!("Scrolls read failed; showing the last good read. {error}");
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
