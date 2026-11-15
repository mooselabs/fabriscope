// Owned by WP5 — Runs. Frozen signature: `Cache::get_run(&self, &str) ->
// Fetch<Arc<Run>>`, `Cache::get_doc_page(&self, &str, &str, usize) ->
// Fetch<Arc<String>>`.
//! Run docs, document pages and boards, fetched lazily and kept in memory.
//!
//! Pages never wait: a `get_*` call answers from what is cached, `Pending`
//! when it is not there yet, and a miss starts a background fetch (at most
//! four at once, one per key) whose arrival moves the view revision so the
//! open page refreshes. Once `runs::client` has installed a reader:
//!
//! - a run doc is fetched again when the index lists a different digest for
//!   it (its pages too); the old copy is served meanwhile;
//! - boards are fetched again after each new index;
//! - a failed fetch is retried 60 s after it failed;
//! - at most `max_runs` run docs (200) and `max_page_bytes` of document
//!   pages (64 MiB) are kept, the least recently read going first.
//!
//! Without a reader (unit tests, or no `--runs`), a miss stays `Pending`
//! until something calls `put_*`.

use super::{Board, Index, Run, doc_readable};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// A cached read: there, on its way, or failed (with the reason to show).
#[derive(Clone, Debug, PartialEq)]
pub enum Fetch<T> {
    Ready(T),
    Pending,
    Failed(String),
}

/// (run, document id, page number).
type PageKey = (String, String, usize);

/// How long a failed fetch is kept before a read tries again.
const RETRY: Duration = Duration::from_secs(60);

/// How many fetches run at once.
const IN_FLIGHT: usize = 4;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum Key {
    Run(String),
    Page(PageKey),
    Board(String),
}

#[derive(Debug)]
struct Slot<T> {
    fetch: Fetch<T>,
    /// The access tick of the last read, for least-recently-read eviction.
    used: u64,
    /// When the fetch finished.
    at: Instant,
    /// The index now names a different version: fetch again, serve this.
    stale: bool,
    /// The run doc digest this slot holds (empty for pages and boards).
    digest: String,
    /// Bytes held, for pages.
    bytes: usize,
}

impl<T> Slot<T> {
    fn due(&self) -> bool {
        self.stale || (matches!(self.fetch, Fetch::Failed(_)) && self.at.elapsed() >= RETRY)
    }
}

#[derive(Debug, Default)]
struct Inner {
    tick: u64,
    runs: HashMap<String, Slot<Arc<Run>>>,
    pages: HashMap<PageKey, Slot<Arc<String>>>,
    boards: HashMap<String, Slot<Arc<Board>>>,
    page_bytes: usize,
    /// Run id → (href, digest) from the current index.
    listed: HashMap<String, (String, String)>,
    /// Board name → href from the current feed.
    board_hrefs: BTreeMap<String, String>,
    /// Fetches under way.
    loading: HashSet<Key>,
}

type Reader = dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync;
type Notify = dyn Fn(Option<Arc<Run>>) + Send + Sync;

/// How fetches happen once `install` is called.
struct Hooks {
    me: Weak<Cache>,
    read: Arc<Reader>,
    notify: Arc<Notify>,
    permits: Arc<Semaphore>,
}

impl std::fmt::Debug for Hooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hooks")
    }
}

#[derive(Debug)]
pub struct Cache {
    inner: Mutex<Inner>,
    hooks: OnceLock<Hooks>,
    max_runs: usize,
    max_page_bytes: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self::with_limits(200, 64 << 20)
    }
}

impl Cache {
    /// A cache keeping at most `max_runs` run docs and `max_page_bytes` of
    /// document pages.
    pub fn with_limits(max_runs: usize, max_page_bytes: usize) -> Self {
        Self {
            inner: Mutex::default(),
            hooks: OnceLock::new(),
            max_runs: max_runs.max(1),
            max_page_bytes,
        }
    }

    /// Lets misses fetch: `read` reads one feed file by href (blocking; it
    /// runs in `spawn_blocking`), `notify` is called after each arrival, with
    /// the run doc when one arrived (so its attempt ids can be registered).
    /// Only the first call counts.
    pub fn install(
        self: &Arc<Self>,
        read: impl Fn(&str) -> Result<Vec<u8>, String> + Send + Sync + 'static,
        notify: impl Fn(Option<Arc<Run>>) + Send + Sync + 'static,
    ) {
        let _ = self.hooks.set(Hooks {
            me: Arc::downgrade(self),
            read: Arc::new(read),
            notify: Arc::new(notify),
            permits: Arc::new(Semaphore::new(IN_FLIGHT)),
        });
    }

    /// Takes the hrefs and digests of a new index: a cached run doc whose
    /// digest changed, and its pages, are fetched again on their next read;
    /// so are boards.
    pub fn index(&self, index: &Index) {
        let mut inner = self.inner.lock().unwrap();
        inner.listed = index
            .runs
            .iter()
            .map(|r| (r.run.clone(), (r.href.clone(), r.digest.clone())))
            .collect();
        let Inner {
            runs,
            pages,
            listed,
            boards,
            board_hrefs,
            ..
        } = &mut *inner;
        let mut changed = HashSet::new();
        for (id, slot) in runs.iter_mut() {
            if let Some((_, digest)) = listed.get(id) {
                if *digest != slot.digest || matches!(slot.fetch, Fetch::Failed(_)) {
                    slot.stale = true;
                    changed.insert(id.clone());
                }
            }
        }
        for (key, slot) in pages.iter_mut() {
            if changed.contains(&key.0) {
                slot.stale = true;
            }
        }
        *board_hrefs = index.feed.boards.clone();
        boards.retain(|name, _| board_hrefs.contains_key(name));
        for slot in boards.values_mut() {
            slot.stale = true;
        }
    }

    /// The run doc for `run`, without waiting.
    pub fn get_run(&self, run: &str) -> Fetch<Arc<Run>> {
        let mut inner = self.inner.lock().unwrap();
        let tick = inner.next_tick();
        let (answer, due) = match inner.runs.get_mut(run) {
            Some(slot) => {
                slot.used = tick;
                (slot.fetch.clone(), slot.due())
            }
            None => (Fetch::Pending, true),
        };
        if due {
            self.start(&mut inner, Key::Run(run.to_string()));
        }
        answer
    }

    /// Page `page` (0-based) of document `doc` of `run`, as text, without
    /// waiting. `Failed` when the run doc names no such readable page.
    pub fn get_doc_page(&self, run: &str, doc: &str, page: usize) -> Fetch<Arc<String>> {
        let key = (run.to_string(), doc.to_string(), page);
        let mut inner = self.inner.lock().unwrap();
        let tick = inner.next_tick();
        let (answer, due) = match inner.pages.get_mut(&key) {
            Some(slot) => {
                slot.used = tick;
                (slot.fetch.clone(), slot.due())
            }
            None => (Fetch::Pending, true),
        };
        if !due {
            return answer;
        }
        match page_href(&inner, &key) {
            Ok(_) => self.start(&mut inner, Key::Page(key)),
            // The run doc is not here yet: fetch it; its arrival brings the
            // reader back for the page.
            Err(None) => self.start(&mut inner, Key::Run(run.to_string())),
            Err(Some(error)) if answer == Fetch::Pending => return Fetch::Failed(error),
            Err(Some(_)) => {}
        }
        answer
    }

    /// Board `name` of the feed, without waiting.
    pub fn get_board(&self, name: &str) -> Fetch<Arc<Board>> {
        let mut inner = self.inner.lock().unwrap();
        let tick = inner.next_tick();
        let (answer, due) = match inner.boards.get_mut(name) {
            Some(slot) => {
                slot.used = tick;
                (slot.fetch.clone(), slot.due())
            }
            None => (Fetch::Pending, true),
        };
        if due {
            self.start(&mut inner, Key::Board(name.to_string()));
        }
        answer
    }

    /// Delivers a run doc read (or its failure).
    pub fn put_run(&self, run: &str, result: Result<Run, String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.loading.remove(&Key::Run(run.to_string()));
        let digest = match &result {
            Ok(doc) => doc.digest.clone(),
            Err(_) => String::new(),
        };
        let slot = inner.slot(result.map(Arc::new), digest, 0);
        inner.runs.insert(run.to_string(), slot);
        while inner.runs.len() > self.max_runs {
            let Some(oldest) = least_used(&inner.runs, run) else {
                break;
            };
            inner.runs.remove(&oldest);
        }
    }

    /// Delivers a document page read (or its failure).
    pub fn put_doc_page(&self, run: &str, doc: &str, page: usize, result: Result<String, String>) {
        let key = (run.to_string(), doc.to_string(), page);
        let mut inner = self.inner.lock().unwrap();
        inner.loading.remove(&Key::Page(key.clone()));
        let bytes = result.as_ref().map_or(0, String::len);
        let slot = inner.slot(result.map(Arc::new), String::new(), bytes);
        if let Some(old) = inner.pages.insert(key.clone(), slot) {
            inner.page_bytes -= old.bytes;
        }
        inner.page_bytes += bytes;
        while inner.page_bytes > self.max_page_bytes {
            let Some(oldest) = least_used(&inner.pages, &key) else {
                break;
            };
            if let Some(old) = inner.pages.remove(&oldest) {
                inner.page_bytes -= old.bytes;
            }
        }
    }

    /// Delivers a board read (or its failure).
    pub fn put_board(&self, name: &str, result: Result<Board, String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.loading.remove(&Key::Board(name.to_string()));
        let slot = inner.slot(result.map(Arc::new), String::new(), 0);
        inner.boards.insert(name.to_string(), slot);
    }

    /// How many run docs and page bytes are held.
    pub fn held(&self) -> (usize, usize) {
        let inner = self.inner.lock().unwrap();
        (inner.runs.len(), inner.page_bytes)
    }

    /// Starts fetching `key` unless it is under way or nothing can fetch.
    fn start(&self, inner: &mut Inner, key: Key) {
        let Some(hooks) = self.hooks.get() else {
            return;
        };
        if inner.loading.contains(&key) {
            return;
        }
        let href = match &key {
            Key::Run(run) => inner.listed.get(run).map(|(href, _)| href.clone()),
            Key::Page(page) => page_href(inner, page).ok(),
            Key::Board(name) => inner.board_hrefs.get(name).cloned(),
        };
        let Some(href) = href else {
            let error = "the run feed index does not list it".to_string();
            match key {
                Key::Run(run) => {
                    let slot = inner.slot(Err(error), String::new(), 0);
                    inner.runs.insert(run, slot);
                }
                Key::Board(name) => {
                    let slot = inner.slot(Err(error), String::new(), 0);
                    inner.boards.insert(name, slot);
                }
                Key::Page(_) => {}
            }
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        inner.loading.insert(key.clone());
        let me = hooks.me.clone();
        let read = hooks.read.clone();
        let notify = hooks.notify.clone();
        let permits = hooks.permits.clone();
        handle.spawn(async move {
            let _permit = permits.acquire_owned().await;
            let bytes = tokio::task::spawn_blocking(move || read(&href))
                .await
                .unwrap_or_else(|e| Err(format!("run feed read stopped: {e}")));
            let Some(cache) = me.upgrade() else {
                return;
            };
            let arrived = cache.deliver(key, bytes);
            notify(arrived);
        });
    }

    /// Stores a fetch; the run doc when one arrived.
    fn deliver(&self, key: Key, bytes: Result<Vec<u8>, String>) -> Option<Arc<Run>> {
        match key {
            Key::Run(run) => {
                let doc = bytes.and_then(|b| {
                    serde_json::from_slice::<Run>(&b).map_err(|e| format!("run doc {run}: {e}"))
                });
                self.put_run(&run, doc);
                return match self.inner.lock().unwrap().runs.get(&run).map(|s| &s.fetch) {
                    Some(Fetch::Ready(doc)) => Some(doc.clone()),
                    _ => None,
                };
            }
            Key::Page((run, doc, page)) => {
                let text = bytes.map(|b| String::from_utf8_lossy(&b).into_owned());
                self.put_doc_page(&run, &doc, page, text);
            }
            Key::Board(name) => {
                let board = bytes.and_then(|b| {
                    serde_json::from_slice::<Board>(&b).map_err(|e| format!("board {name}: {e}"))
                });
                self.put_board(&name, board);
            }
        }
        None
    }

    /// Every run doc held, for registering attempt ids in a new index.
    pub fn ready_runs(&self) -> Vec<Arc<Run>> {
        self.inner
            .lock()
            .unwrap()
            .runs
            .values()
            .filter_map(|s| match &s.fetch {
                Fetch::Ready(doc) => Some(doc.clone()),
                _ => None,
            })
            .collect()
    }
}

impl Inner {
    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    fn slot<T>(&mut self, result: Result<T, String>, digest: String, bytes: usize) -> Slot<T> {
        let used = self.next_tick();
        Slot {
            fetch: match result {
                Ok(value) => Fetch::Ready(value),
                Err(error) => Fetch::Failed(error),
            },
            used,
            at: Instant::now(),
            stale: false,
            digest,
            bytes,
        }
    }
}

/// The key read least recently, other than `keep`.
fn least_used<K: Clone + Eq + std::hash::Hash + std::borrow::Borrow<Q>, Q: Eq + ?Sized, T>(
    map: &HashMap<K, Slot<T>>,
    keep: &Q,
) -> Option<K> {
    map.iter()
        .filter(|(k, _)| (*k).borrow() != keep)
        .min_by_key(|(_, slot)| slot.used)
        .map(|(k, _)| k.clone())
}

/// The href of a page from the cached run doc; `Err(None)` while the run
/// doc is not here, `Err(Some(why))` when it names no such readable page.
fn page_href(inner: &Inner, (run, doc, page): &PageKey) -> Result<String, Option<String>> {
    let found = match inner.runs.get(run).map(|s| &s.fetch) {
        Some(Fetch::Ready(found)) => found,
        Some(Fetch::Failed(error)) => return Err(Some(error.clone())),
        Some(Fetch::Pending) | None => return Err(None),
    };
    let doc_ref = found
        .documents
        .iter()
        .find(|d| d.id == *doc)
        .ok_or_else(|| Some(format!("run {run} has no document {doc:?}")))?;
    if !doc_readable(&doc_ref.state) {
        return Err(Some(format!("document {doc:?} is {}", doc_ref.state)));
    }
    doc_ref
        .pages
        .get(*page)
        .map(|p| p.href.clone())
        .ok_or_else(|| Some(format!("document {doc:?} has no page {}", page + 1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::{Feed, IndexFile, RunSummary};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn misses_are_pending_and_puts_are_served() {
        let cache = Cache::default();
        assert_eq!(cache.get_run("r"), Fetch::Pending);
        cache.put_run("r", Err("gone".into()));
        assert_eq!(cache.get_run("r"), Fetch::Failed("gone".into()));
        cache.put_doc_page("r", "d", 0, Ok("text".into()));
        assert_eq!(
            cache.get_doc_page("r", "d", 0),
            Fetch::Ready(Arc::new("text".into()))
        );
        assert_eq!(
            cache.get_doc_page("r", "d", 1),
            Fetch::Failed("gone".into()),
            "the run doc failed, so there is no page to ask for"
        );
        assert_eq!(cache.get_doc_page("s", "d", 0), Fetch::Pending);
    }

    #[test]
    fn the_bounds_evict_the_least_recently_read() {
        let cache = Cache::with_limits(2, 10);
        for id in ["a", "b"] {
            cache.put_run(id, Ok(Run::default()));
        }
        let _ = cache.get_run("a");
        cache.put_run("c", Ok(Run::default()));
        assert_eq!(cache.held().0, 2);
        assert!(matches!(cache.get_run("a"), Fetch::Ready(_)));
        assert_eq!(
            cache.get_run("b"),
            Fetch::Pending,
            "b was read least recently"
        );
        cache.put_doc_page("a", "d", 0, Ok("12345".into()));
        cache.put_doc_page("a", "d", 1, Ok("12345".into()));
        let _ = cache.get_doc_page("a", "d", 0);
        cache.put_doc_page("a", "d", 2, Ok("123".into()));
        assert_eq!(cache.held().1, 8);
        assert!(matches!(cache.get_doc_page("a", "d", 0), Fetch::Ready(_)));
        cache.put_doc_page("a", "d", 3, Ok("x".repeat(40)));
        assert_eq!(
            cache.held().1,
            40,
            "one page over the bound is still kept alone"
        );
    }

    fn index(digest: &str) -> Index {
        let summary = RunSummary {
            run: "r1".into(),
            href: "runs/r1.json".into(),
            digest: digest.into(),
            ..Default::default()
        };
        let mut feed = Feed::default();
        feed.boards.insert("b".into(), "boards/b.json".into());
        Index::build(
            feed,
            vec![IndexFile {
                runs: vec![summary],
                ..Default::default()
            }],
            &[],
        )
    }

    async fn settle<T: Clone>(mut read: impl FnMut() -> Fetch<T>) -> Fetch<T> {
        for _ in 0..200 {
            match read() {
                Fetch::Pending => tokio::time::sleep(Duration::from_millis(10)).await,
                other => return other,
            }
        }
        Fetch::Pending
    }

    #[tokio::test]
    async fn misses_fetch_once_and_a_new_digest_fetches_again() {
        let files: Arc<Mutex<HashMap<String, String>>> = Arc::default();
        let reads = Arc::new(AtomicUsize::new(0));
        let notified = Arc::new(AtomicUsize::new(0));
        let doc = |digest: &str| {
            serde_json::json!({"run": "r1", "digest": digest, "documents": [
                {"id": "log", "state": "ready", "content_type": "text/plain",
                 "pages": [{"href": "docs/r1/log/1.txt"}]},
                {"id": "secret", "state": "withheld", "pages": []}]})
            .to_string()
        };
        files
            .lock()
            .unwrap()
            .insert("runs/r1.json".into(), doc("one"));
        files
            .lock()
            .unwrap()
            .insert("docs/r1/log/1.txt".into(), "line".into());
        files.lock().unwrap().insert(
            "boards/b.json".into(),
            r#"{"schema":"runfeed.board.v1","name":"b","body":[1]}"#.into(),
        );
        let cache = Arc::new(Cache::default());
        let (f, r, n) = (files.clone(), reads.clone(), notified.clone());
        cache.install(
            move |href| {
                r.fetch_add(1, Ordering::SeqCst);
                f.lock()
                    .unwrap()
                    .get(href)
                    .cloned()
                    .map(String::into_bytes)
                    .ok_or_else(|| format!("{href}: not found"))
            },
            move |_| {
                n.fetch_add(1, Ordering::SeqCst);
            },
        );
        cache.index(&index("one"));
        assert_eq!(cache.get_run("r1"), Fetch::Pending);
        assert_eq!(
            cache.get_run("r1"),
            Fetch::Pending,
            "asked twice, read once"
        );
        let Fetch::Ready(first) = settle(|| cache.get_run("r1")).await else {
            panic!("the run doc arrives")
        };
        assert_eq!(first.digest, "one");
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(notified.load(Ordering::SeqCst) >= 1);
        let page = settle(|| cache.get_doc_page("r1", "log", 0)).await;
        assert_eq!(page, Fetch::Ready(Arc::new("line".into())));
        assert!(matches!(
            cache.get_doc_page("r1", "secret", 0),
            Fetch::Failed(_)
        ));
        assert!(matches!(
            settle(|| cache.get_board("b")).await,
            Fetch::Ready(_)
        ));
        assert!(matches!(
            settle(|| cache.get_run("unlisted")).await,
            Fetch::Failed(_)
        ));
        let before = reads.load(Ordering::SeqCst);
        cache.index(&index("one"));
        let _ = cache.get_run("r1");
        files
            .lock()
            .unwrap()
            .insert("runs/r1.json".into(), doc("two"));
        cache.index(&index("two"));
        assert!(
            matches!(cache.get_run("r1"), Fetch::Ready(d) if d.digest == "one"),
            "the old copy is served while the new one is fetched"
        );
        for _ in 0..200 {
            if matches!(cache.get_run("r1"), Fetch::Ready(d) if d.digest == "two") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(matches!(cache.get_run("r1"), Fetch::Ready(d) if d.digest == "two"));
        // The same digest again reads nothing more for the run; the new
        // index refreshes the board.
        assert!(reads.load(Ordering::SeqCst) > before);
    }

    #[tokio::test]
    async fn a_failed_fetch_waits_before_trying_again() {
        let reads = Arc::new(AtomicUsize::new(0));
        let cache = Arc::new(Cache::default());
        let r = reads.clone();
        cache.install(
            move |_| {
                r.fetch_add(1, Ordering::SeqCst);
                Err("down".into())
            },
            |_| {},
        );
        cache.index(&index("one"));
        assert!(matches!(
            settle(|| cache.get_run("r1")).await,
            Fetch::Failed(e) if e == "down"
        ));
        for _ in 0..5 {
            let _ = cache.get_run("r1");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        cache.inner.lock().unwrap().runs.get_mut("r1").unwrap().at =
            Instant::now().checked_sub(RETRY).unwrap();
        let _ = cache.get_run("r1");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }
}
