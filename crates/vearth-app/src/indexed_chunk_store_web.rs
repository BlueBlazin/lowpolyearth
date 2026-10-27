//! Browser storage for the generated-surface cache: one origin-private file
//! per cell in the cache generation's directory, with an in-memory LRU index.
//!
//! The index is rebuilt from the directory listing at startup, ordered by
//! each file's last write; reads in the current session refresh recency.
//! Writes and evictions run asynchronously on the reactor worker. Payloads
//! are opaque; `ChunkCache` validates them before use.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use vearth_core::cell::CellId;
use vearth_platform::web::opfs;

/// Stamps for entries found at startup stay below every in-session stamp.
const SESSION_STAMP_BASE: u64 = 1 << 40;
/// Never claim more than this share of the origin's storage quota.
const QUOTA_SHARE: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    face: u8,
    level: u8,
    i: u32,
    j: u32,
}

impl Key {
    fn new(cell: CellId) -> Self {
        Self {
            face: cell.face.0,
            level: cell.level,
            i: cell.i,
            j: cell.j,
        }
    }

    fn file_name(self) -> String {
        format!("{}-{}-{}-{}.vxs", self.face, self.level, self.i, self.j)
    }

    fn parse(name: &str) -> Option<Self> {
        let mut parts = name.strip_suffix(".vxs")?.split('-');
        let key = Self {
            face: parts.next()?.parse().ok()?,
            level: parts.next()?.parse().ok()?,
            i: parts.next()?.parse().ok()?,
            j: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(key)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IndexedStoreStats {
    pub writes: u64,
    pub evictions: u64,
    pub corruptions: u64,
    pub write_errors: u64,
    pub dropped_writes: u64,
    pub logical_bytes: u64,
    pub entries: u64,
    pub pinned_entries: u64,
    pub pinned_bytes: u64,
    pub disk_bytes: u64,
    pub budget_bytes: u64,
    pub queued_commands: usize,
}

struct Meta {
    bytes: u64,
    stamp: u64,
    pinned: bool,
}

#[derive(Default)]
struct Index {
    entries: HashMap<Key, Meta>,
    /// Unpinned entries by recency, oldest first.
    recency: BTreeMap<u64, Key>,
    next_stamp: u64,
    bytes: u64,
}

impl Index {
    fn insert(&mut self, key: Key, bytes: u64, stamp: u64, pinned: bool) {
        self.remove(key);
        if !pinned {
            self.recency.insert(stamp, key);
        }
        self.entries.insert(
            key,
            Meta {
                bytes,
                stamp,
                pinned,
            },
        );
        self.bytes += bytes;
    }

    fn remove(&mut self, key: Key) -> Option<Meta> {
        let meta = self.entries.remove(&key)?;
        self.recency.remove(&meta.stamp);
        self.bytes -= meta.bytes;
        Some(meta)
    }

    fn session_stamp(&mut self) -> u64 {
        self.next_stamp += 1;
        SESSION_STAMP_BASE + self.next_stamp
    }

    fn touch(&mut self, key: Key) {
        let stamp = self.session_stamp();
        if let Some(meta) = self.entries.get_mut(&key) {
            if !meta.pinned {
                self.recency.remove(&meta.stamp);
                self.recency.insert(stamp, key);
            }
            meta.stamp = stamp;
        }
    }
}

struct Inner {
    directory: String,
    /// Set once the startup scan has indexed every existing file; from then
    /// on the index alone answers misses.
    scanned: AtomicBool,
    budget: AtomicU64,
    index: Mutex<Index>,
    pending: Mutex<usize>,
    settled: Condvar,
    writes: AtomicU64,
    evictions: AtomicU64,
    corruptions: AtomicU64,
    write_errors: AtomicU64,
}

impl Inner {
    fn path(&self, key: Key) -> String {
        format!("{}/{}", self.directory, key.file_name())
    }

    /// Runs `operation` on the reactor, counting it until it settles.
    fn background<F>(self: &Arc<Self>, operation: impl FnOnce() -> F + Send + 'static)
    where
        F: std::future::Future<Output = io::Result<()>> + 'static,
    {
        *self.pending.lock().unwrap() += 1;
        let inner = self.clone();
        vearth_platform::rt::spawn_local_on_reactor(move || async move {
            if let Err(error) = operation().await {
                inner.write_errors.fetch_add(1, Ordering::Relaxed);
                log::warn!("chunk cache: storage operation failed: {error}");
            }
            let mut pending = inner.pending.lock().unwrap();
            *pending -= 1;
            if *pending == 0 {
                inner.settled.notify_all();
            }
        });
    }

    fn evict_to_budget(self: &Arc<Self>) {
        let budget = self.budget.load(Ordering::Relaxed);
        let mut victims = Vec::new();
        {
            let mut index = self.index.lock().unwrap();
            while index.bytes > budget {
                let Some((_, key)) = index.recency.pop_first() else {
                    break;
                };
                index.remove(key);
                victims.push(key);
            }
        }
        for key in victims {
            self.evictions.fetch_add(1, Ordering::Relaxed);
            let path = self.path(key);
            self.background(move || async move { opfs::remove(&path).await });
        }
    }
}

pub(crate) struct IndexedChunkStore {
    inner: Arc<Inner>,
}

impl IndexedChunkStore {
    /// `path` is the native database path. Its parent, the cache
    /// generation's directory, holds the cell files; sibling generations are
    /// removed.
    pub(crate) fn open(path: PathBuf, budget_bytes: u64) -> io::Result<Self> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "invalid cache path");
        let directory = path.parent().ok_or_else(invalid)?;
        let tag = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(invalid)?
            .to_owned();
        let generations = directory
            .parent()
            .and_then(Path::to_str)
            .ok_or_else(invalid)?
            .to_owned();
        let directory = directory.to_str().ok_or_else(invalid)?.to_owned();
        let inner = Arc::new(Inner {
            directory,
            scanned: AtomicBool::new(false),
            budget: AtomicU64::new(budget_bytes),
            index: Mutex::new(Index::default()),
            pending: Mutex::new(0),
            settled: Condvar::new(),
            writes: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            corruptions: AtomicU64::new(0),
            write_errors: AtomicU64::new(0),
        });
        let scan = inner.clone();
        inner.background(move || async move {
            scan_existing(scan, &generations, &tag, budget_bytes).await
        });
        Ok(Self { inner })
    }

    pub(crate) fn load(&self, cell: CellId) -> io::Result<Option<Vec<u8>>> {
        let key = Key::new(cell);
        if self.inner.scanned.load(Ordering::Acquire)
            && !self.inner.index.lock().unwrap().entries.contains_key(&key)
        {
            return Ok(None);
        }
        let bytes = opfs::blocking::read(&self.inner.path(key))?;
        let mut index = self.inner.index.lock().unwrap();
        match &bytes {
            Some(bytes) if !index.entries.contains_key(&key) => {
                // Found before the startup scan reached it.
                let stamp = index.session_stamp();
                index.insert(key, bytes.len() as u64, stamp, false);
            }
            Some(_) => index.touch(key),
            None => {
                index.remove(key);
            }
        }
        Ok(bytes)
    }

    pub(crate) fn store(&self, cell: CellId, payload: Arc<[u8]>, pin: bool) -> bool {
        let key = Key::new(cell);
        {
            let mut index = self.inner.index.lock().unwrap();
            let pinned = pin || index.entries.get(&key).is_some_and(|meta| meta.pinned);
            let stamp = index.session_stamp();
            index.insert(key, payload.len() as u64, stamp, pinned);
        }
        self.inner.writes.fetch_add(1, Ordering::Relaxed);
        let path = self.inner.path(key);
        self.inner
            .background(move || async move { opfs::write(&path, &payload).await });
        self.inner.evict_to_budget();
        true
    }

    pub(crate) fn remove(&self, cell: CellId) {
        let key = Key::new(cell);
        self.inner.index.lock().unwrap().remove(key);
        let path = self.inner.path(key);
        self.inner
            .background(move || async move { opfs::remove(&path).await });
    }

    pub(crate) fn set_pinned(&self, cells: &[CellId], pinned: bool) {
        let mut index = self.inner.index.lock().unwrap();
        for &cell in cells {
            let key = Key::new(cell);
            let Some(meta) = index.entries.get_mut(&key) else {
                continue;
            };
            if meta.pinned == pinned {
                continue;
            }
            meta.pinned = pinned;
            let stamp = meta.stamp;
            if pinned {
                index.recency.remove(&stamp);
            } else {
                index.recency.insert(stamp, key);
            }
        }
    }

    pub(crate) fn note_corruption(&self) {
        self.inner.corruptions.fetch_add(1, Ordering::Relaxed);
    }

    /// Waits until queued writes and removals have settled.
    pub(crate) fn flush(&self) {
        let mut pending = self.inner.pending.lock().unwrap();
        while *pending > 0 {
            pending = self.inner.settled.wait(pending).unwrap();
        }
    }

    pub(crate) fn stats(&self) -> IndexedStoreStats {
        let index = self.inner.index.lock().unwrap();
        let (pinned_entries, pinned_bytes) = index
            .entries
            .values()
            .filter(|meta| meta.pinned)
            .fold((0, 0), |(count, bytes), meta| {
                (count + 1, bytes + meta.bytes)
            });
        IndexedStoreStats {
            writes: self.inner.writes.load(Ordering::Relaxed),
            evictions: self.inner.evictions.load(Ordering::Relaxed),
            corruptions: self.inner.corruptions.load(Ordering::Relaxed),
            write_errors: self.inner.write_errors.load(Ordering::Relaxed),
            dropped_writes: 0,
            logical_bytes: index.bytes,
            entries: index.entries.len() as u64,
            pinned_entries,
            pinned_bytes,
            disk_bytes: index.bytes,
            budget_bytes: self.inner.budget.load(Ordering::Relaxed),
            queued_commands: *self.inner.pending.lock().unwrap(),
        }
    }
}

/// Removes other cache generations, bounds the budget by the storage quota
/// and indexes the files already present.
async fn scan_existing(
    inner: Arc<Inner>,
    generations: &str,
    tag: &str,
    configured_budget: u64,
) -> io::Result<()> {
    for stale in opfs::list_directories(generations).await? {
        if stale != tag {
            opfs::remove(&format!("{generations}/{stale}")).await?;
            log::info!("chunk cache: removed stale browser cache {stale}");
        }
    }
    if let Some(quota) = storage_quota().await {
        let budget = configured_budget.min((quota as f64 * QUOTA_SHARE) as u64);
        inner.budget.store(budget, Ordering::Relaxed);
    }
    let mut files = opfs::list_files(&inner.directory).await?;
    files.sort_by(|a, b| a.modified_ms.total_cmp(&b.modified_ms));
    {
        let mut index = inner.index.lock().unwrap();
        for (order, file) in files.iter().enumerate() {
            let Some(key) = Key::parse(&file.name) else {
                continue;
            };
            // Entries touched in this session already hold newer stamps.
            if !index.entries.contains_key(&key) {
                index.insert(key, file.size, order as u64 + 1, false);
            }
        }
        inner.scanned.store(true, Ordering::Release);
        log::info!(
            "chunk cache: {} browser entries, {:.1} MiB of {:.0} MiB",
            index.entries.len(),
            index.bytes as f64 / 1048576.0,
            inner.budget.load(Ordering::Relaxed) as f64 / 1048576.0
        );
    }
    inner.evict_to_budget();
    Ok(())
}

async fn storage_quota() -> Option<f64> {
    use wasm_bindgen::JsCast;
    let navigator = js_sys::global()
        .unchecked_into::<web_sys::WorkerGlobalScope>()
        .navigator();
    let estimate = wasm_bindgen_futures::JsFuture::from(navigator.storage().estimate().ok()?)
        .await
        .ok()?;
    js_sys::Reflect::get(&estimate, &"quota".into())
        .ok()?
        .as_f64()
}
