//! Browser source-tile cache: the native store's API over a bounded,
//! in-memory LRU for the session.
//!
//! Persistence across visits comes from the browser's HTTP cache: terrain
//! tiles carry long `max-age` headers and the vector proxy serves immutable,
//! build-named archives. Keeping raw bytes here therefore only avoids repeat
//! round trips within a session, and eviction is safe at any time.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use vearth_platform::thread::JoinHandle;

pub const DEFAULT_SOURCE_CACHE_BUDGET: u64 = 8 * 1024 * 1024 * 1024;

/// Upper bound on resident raw bytes in the browser, whatever budget the
/// caller configured for disk.
const MAX_MEMORY_BUDGET: u64 = 192 * 1024 * 1024;
/// Charged per entry so negative entries also count against the budget.
const ENTRY_OVERHEAD: u64 = 64;

pub type Result<T> = std::result::Result<T, io::Error>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CachedTile {
    Present(Vec<u8>),
    Missing,
}

impl CachedTile {
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Present(bytes) => Some(bytes),
            Self::Missing => None,
        }
    }

    fn charge(&self) -> u64 {
        ENTRY_OVERHEAD + self.bytes().map_or(0, |bytes| bytes.len() as u64)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub writes: u64,
    pub evictions: u64,
    pub corruptions: u64,
    pub write_errors: u64,
    pub dropped_writes: u64,
    pub logical_bytes: u64,
    pub entries: u64,
    pub pinned_entries: u64,
    pub pinned_bytes: u64,
    pub migrations: u64,
    pub physical_bytes: u64,
    pub budget_bytes: u64,
    pub queued_commands: usize,
}

/// Retained for API compatibility: the browser has no file-per-tile caches
/// to migrate, so namespaces never carry a layout.
#[derive(Clone, Debug)]
pub enum LegacyLayout {
    Terrarium { root: PathBuf },
    Vectors { root: PathBuf, build_tag: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub imported: u64,
    pub bytes: u64,
    pub errors: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Key {
    namespace: u32,
    z: u8,
    x: u32,
    y: u32,
}

struct Entry {
    tile: CachedTile,
    pinned: bool,
    stamp: u64,
}

#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
    /// Unpinned entries by last access, oldest first.
    recency: BTreeMap<u64, Key>,
    next_stamp: u64,
    bytes: u64,
    namespaces: HashMap<String, (u32, String)>,
    /// Never reused, so a replaced revision's id cannot alias another
    /// provider's entries.
    last_namespace: u32,
}

impl State {
    fn stamp(&mut self) -> u64 {
        self.next_stamp += 1;
        self.next_stamp
    }

    fn touch(&mut self, key: Key) {
        let stamp = self.stamp();
        if let Some(entry) = self.entries.get_mut(&key) {
            if !entry.pinned {
                self.recency.remove(&entry.stamp);
                self.recency.insert(stamp, key);
            }
            entry.stamp = stamp;
        }
    }

    fn remove(&mut self, key: Key) -> bool {
        match self.entries.remove(&key) {
            Some(entry) => {
                self.recency.remove(&entry.stamp);
                self.bytes -= entry.tile.charge();
                true
            }
            None => false,
        }
    }
}

struct StoreInner {
    state: Mutex<State>,
    budget: u64,
    hits: AtomicU64,
    misses: AtomicU64,
    writes: AtomicU64,
    evictions: AtomicU64,
    corruptions: AtomicU64,
}

impl StoreInner {
    fn store(&self, key: Key, tile: CachedTile) -> bool {
        let charge = tile.charge();
        if charge > self.budget {
            return false;
        }
        let mut state = self.state.lock().unwrap();
        let pinned = state.entries.get(&key).is_some_and(|entry| entry.pinned);
        state.remove(key);
        let stamp = state.stamp();
        if !pinned {
            state.recency.insert(stamp, key);
        }
        state.entries.insert(
            key,
            Entry {
                tile,
                pinned,
                stamp,
            },
        );
        state.bytes += charge;
        while state.bytes > self.budget {
            let Some((_, oldest)) = state.recency.pop_first() else {
                break;
            };
            state.remove(oldest);
            self.evictions.fetch_add(1, Ordering::Relaxed);
        }
        self.writes.fetch_add(1, Ordering::Relaxed);
        true
    }
}

pub struct SourceTileStore {
    inner: Arc<StoreInner>,
}

impl SourceTileStore {
    /// `path` is ignored; `budget_bytes` is capped for memory residency.
    pub fn open(_path: impl Into<PathBuf>, budget_bytes: u64) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(StoreInner {
                state: Mutex::new(State::default()),
                budget: budget_bytes.min(MAX_MEMORY_BUDGET),
                hits: AtomicU64::new(0),
                misses: AtomicU64::new(0),
                writes: AtomicU64::new(0),
                evictions: AtomicU64::new(0),
                corruptions: AtomicU64::new(0),
            }),
        })
    }

    /// Selects a provider's sole active revision, dropping its other
    /// revisions' entries.
    pub fn namespace(
        &self,
        provider: impl Into<String>,
        revision: impl Into<String>,
        _content_type: impl Into<String>,
        _legacy: Option<LegacyLayout>,
    ) -> Result<SourceTileCache> {
        let provider = provider.into();
        let revision = revision.into();
        if provider.trim().is_empty() || revision.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source-cache provider and revision must be non-empty",
            ));
        }
        let mut state = self.inner.state.lock().unwrap();
        let (namespace, stale) = match state.namespaces.get(&provider) {
            Some((id, current)) if *current == revision => (*id, None),
            current => {
                let stale = current.map(|(id, _)| *id);
                state.last_namespace += 1;
                (state.last_namespace, stale)
            }
        };
        if let Some(stale) = stale {
            let keys: Vec<_> = state
                .entries
                .keys()
                .copied()
                .filter(|key| key.namespace == stale)
                .collect();
            for key in keys {
                state.remove(key);
            }
        }
        state
            .namespaces
            .insert(provider.clone(), (namespace, revision.clone()));
        drop(state);
        Ok(SourceTileCache {
            inner: self.inner.clone(),
            namespace,
            provider,
            revision,
        })
    }

    pub fn flush(&self) -> bool {
        true
    }

    pub fn stats(&self) -> SourceCacheStats {
        let state = self.inner.state.lock().unwrap();
        let pinned = state.entries.values().filter(|entry| entry.pinned);
        let (pinned_entries, pinned_bytes) = pinned.fold((0, 0), |(count, bytes), entry| {
            (count + 1, bytes + entry.tile.charge())
        });
        let inner = &self.inner;
        SourceCacheStats {
            hits: inner.hits.load(Ordering::Relaxed),
            misses: inner.misses.load(Ordering::Relaxed),
            writes: inner.writes.load(Ordering::Relaxed),
            evictions: inner.evictions.load(Ordering::Relaxed),
            corruptions: inner.corruptions.load(Ordering::Relaxed),
            logical_bytes: state.bytes,
            entries: state.entries.len() as u64,
            pinned_entries,
            pinned_bytes,
            budget_bytes: inner.budget,
            ..SourceCacheStats::default()
        }
    }
}

#[derive(Clone)]
pub struct SourceTileCache {
    inner: Arc<StoreInner>,
    namespace: u32,
    provider: String,
    revision: String,
}

impl SourceTileCache {
    fn key(&self, z: u8, x: u32, y: u32) -> Key {
        Key {
            namespace: self.namespace,
            z,
            x,
            y,
        }
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn legacy_layout(&self) -> Option<&LegacyLayout> {
        None
    }

    pub fn load(&self, z: u8, x: u32, y: u32) -> Result<Option<CachedTile>> {
        let key = self.key(z, x, y);
        let mut state = self.inner.state.lock().unwrap();
        let tile = state.entries.get(&key).map(|entry| entry.tile.clone());
        if tile.is_some() {
            state.touch(key);
            self.inner.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.inner.misses.fetch_add(1, Ordering::Relaxed);
        }
        Ok(tile)
    }

    pub fn store_present(&self, z: u8, x: u32, y: u32, bytes: Vec<u8>) -> bool {
        self.inner
            .store(self.key(z, x, y), CachedTile::Present(bytes))
    }

    pub fn store_missing(&self, z: u8, x: u32, y: u32) -> bool {
        self.inner.store(self.key(z, x, y), CachedTile::Missing)
    }

    pub fn remove(&self, z: u8, x: u32, y: u32) {
        self.inner.state.lock().unwrap().remove(self.key(z, x, y));
    }

    pub fn remove_corrupt(&self, z: u8, x: u32, y: u32) {
        self.inner.corruptions.fetch_add(1, Ordering::Relaxed);
        self.remove(z, x, y);
    }

    pub fn set_pinned(&self, tiles: &[(u8, u32, u32)], pinned: bool) {
        let mut state = self.inner.state.lock().unwrap();
        for &(z, x, y) in tiles {
            let key = self.key(z, x, y);
            let Some(entry) = state.entries.get_mut(&key) else {
                continue;
            };
            if entry.pinned == pinned {
                continue;
            }
            entry.pinned = pinned;
            let stamp = entry.stamp;
            if pinned {
                state.recency.remove(&stamp);
            } else {
                state.recency.insert(stamp, key);
            }
        }
    }

    pub fn flush(&self) -> bool {
        true
    }

    pub fn migrate_legacy(&self) -> MigrationReport {
        MigrationReport::default()
    }

    pub fn start_legacy_migration(&self) -> io::Result<JoinHandle<MigrationReport>> {
        vearth_platform::thread::Builder::new()
            .name("source-cache-migration".into())
            .spawn(MigrationReport::default)
    }
}
