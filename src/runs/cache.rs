// Owned by WP5 — Runs. Frozen signature: `Cache::get_run(&self, &str) ->
// Fetch<Arc<Run>>`, `Cache::get_doc_page(&self, &str, &str, usize) ->
// Fetch<Arc<String>>`.
//! Run docs and document pages, fetched lazily and kept in memory.
//!
//! Pages never wait: a `get_*` call answers from what is cached, `Pending`
//! when it is not there yet. WP5 adds the fetch a miss starts (through
//! `runs::client`) and the bounds that evict old entries; `put_*` is how a
//! fetch delivers.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "contract the packages call; WP5 fills and reads it (BKLG-20261115-4e2r)"
    )
)]

use super::Run;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A cached read: there, on its way, or failed (with the reason to show).
#[derive(Clone, Debug, PartialEq)]
pub enum Fetch<T> {
    Ready(T),
    Pending,
    Failed(String),
}

/// (run, document id, page number).
type PageKey = (String, String, usize);

#[derive(Debug, Default)]
pub struct Cache {
    runs: Mutex<HashMap<String, Fetch<Arc<Run>>>>,
    pages: Mutex<HashMap<PageKey, Fetch<Arc<String>>>>,
}

impl Cache {
    /// The run doc for `run`, without waiting. Placeholder: a miss stays
    /// `Pending` until something calls `put_run`.
    pub fn get_run(&self, run: &str) -> Fetch<Arc<Run>> {
        self.runs
            .lock()
            .unwrap()
            .get(run)
            .cloned()
            .unwrap_or(Fetch::Pending)
    }

    /// Page `page` (0-based) of document `doc` of `run`, as text, without
    /// waiting. Placeholder: a miss stays `Pending`.
    pub fn get_doc_page(&self, run: &str, doc: &str, page: usize) -> Fetch<Arc<String>> {
        self.pages
            .lock()
            .unwrap()
            .get(&(run.to_string(), doc.to_string(), page))
            .cloned()
            .unwrap_or(Fetch::Pending)
    }

    /// Delivers a run doc read (or its failure).
    pub fn put_run(&self, run: &str, result: Result<Run, String>) {
        let entry = match result {
            Ok(doc) => Fetch::Ready(Arc::new(doc)),
            Err(e) => Fetch::Failed(e),
        };
        self.runs.lock().unwrap().insert(run.to_string(), entry);
    }

    /// Delivers a document page read (or its failure).
    pub fn put_doc_page(&self, run: &str, doc: &str, page: usize, result: Result<String, String>) {
        let entry = match result {
            Ok(text) => Fetch::Ready(Arc::new(text)),
            Err(e) => Fetch::Failed(e),
        };
        self.pages
            .lock()
            .unwrap()
            .insert((run.to_string(), doc.to_string(), page), entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(cache.get_doc_page("r", "d", 1), Fetch::Pending);
    }
}
