//! Native SQLite-backed cache for raw source tiles.
//!
//! All source providers share one byte-budgeted database. Provider and
//! dataset revision form an explicit namespace, so changing an upstream
//! dataset invalidates only that provider and never relies on a manually
//! cleared directory. Raw downloaded bytes are stored unchanged.
//!
//! Reads use a small lazy connection pool in WAL mode. Mutations pass through
//! one bounded batching thread; access touches are lossy and coalesced so a
//! cache hit never performs a SQLite write inline. Existing file-per-tile
//! caches are accepted only as migration inputs: a hit is transactionally
//! imported before its file is removed, and the app starts a resumable
//! background migration for cold entries. Payload-index schema changes also
//! migrate in bounded writer transactions; both tables remain readable until
//! every row has moved, without blocking startup on a whole-cache rewrite.

use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use vearth_platform::thread::{self, JoinHandle};
use vearth_platform::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_SOURCE_CACHE_BUDGET: u64 = 8 * 1024 * 1024 * 1024;

const WRITER_QUEUE: usize = 4096;
const MAX_BATCH_COMMANDS: usize = 256;
const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;
const BATCH_DELAY: Duration = Duration::from_millis(12);
const TOUCH_FLUSH_INTERVAL: Duration = Duration::from_secs(30);
const MAX_PENDING_TOUCHES: usize = 8192;
const EVICT_BATCH: usize = 512;
const PAYLOAD_BUDGET_PERCENT: u64 = 88;
const MIGRATION_BATCH_ENTRIES: usize = 256;
const MIGRATION_BATCH_BYTES: usize = 8 * 1024 * 1024;
const SCHEMA_MIGRATION_ENTRIES: usize = 32;
const SCHEMA_MIGRATION_BYTES: usize = 4 * 1024 * 1024;
const SCHEMA_MIGRATION_INTERVAL: Duration = Duration::from_millis(50);
const INCREMENTAL_VACUUM_PAGES: i64 = 2048;
const VACUUM_FREELIST_THRESHOLD: i64 = 4096;
const MIN_DATABASE_ALLOCATION: u64 = 16 * 1024 * 1024;
/// Conservative charge for the table row, primary key, and LRU index. This
/// keeps zero-payload negative entries inside the same byte budget instead of
/// allowing their metadata to grow until SQLite reaches `max_page_count`.
const ENTRY_BUDGET_BYTES: i64 = 256;

const TILE_PRESENT: i64 = 1;
const TILE_MISSING: i64 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    namespace_id: i64,
    z: u8,
    x: u32,
    y: u32,
}

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

    fn into_write(self) -> WriteValue {
        match self {
            Self::Present(bytes) => WriteValue {
                state: TILE_PRESENT,
                payload: Some(Arc::from(bytes)),
                pinned: false,
            },
            Self::Missing => WriteValue {
                state: TILE_MISSING,
                payload: None,
                pinned: false,
            },
        }
    }
}

#[derive(Clone)]
struct WriteValue {
    state: i64,
    payload: Option<Arc<[u8]>>,
    pinned: bool,
}

impl WriteValue {
    fn payload_len(&self) -> usize {
        self.payload.as_ref().map_or(0, |payload| payload.len())
    }
}

enum Command {
    Write { key: Key, value: WriteValue },
    Import { key: Key, value: WriteValue },
    Touch(Key),
    Promote(Key),
    Remove(Key),
    SetPinned { keys: Vec<Key>, pinned: bool },
    Flush(mpsc::Sender<bool>),
    Shutdown(mpsc::Sender<()>),
}

enum Mutation {
    Upsert(WriteValue),
    InsertIfAbsent(WriteValue),
    Remove,
}

#[derive(Default)]
struct StoreMetrics {
    hits: AtomicU64,
    misses: AtomicU64,
    writes: AtomicU64,
    evictions: AtomicU64,
    corruptions: AtomicU64,
    write_errors: AtomicU64,
    dropped_writes: AtomicU64,
    logical_bytes: AtomicU64,
    entries: AtomicU64,
    pinned_entries: AtomicU64,
    pinned_bytes: AtomicU64,
    migrations: AtomicU64,
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
    /// Payload bytes plus the fixed per-entry database-overhead charge.
    pub logical_bytes: u64,
    pub entries: u64,
    pub pinned_entries: u64,
    pub pinned_bytes: u64,
    pub migrations: u64,
    pub physical_bytes: u64,
    pub budget_bytes: u64,
    pub queued_commands: usize,
}

struct StoreInner {
    path: PathBuf,
    readers: Vec<Mutex<Option<Connection>>>,
    tx: SyncSender<Command>,
    queued: Arc<AtomicUsize>,
    metrics: Arc<StoreMetrics>,
    budget_bytes: u64,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl StoreInner {
    fn read(&self, key: Key) -> rusqlite::Result<Option<CachedTile>> {
        let mut hasher = DefaultHasher::new();
        thread::current().id().hash(&mut hasher);
        let slot = hasher.finish() as usize % self.readers.len();
        let mut reader = self.readers[slot].lock().unwrap();
        if reader.is_none() {
            *reader = Some(open_connection(&self.path, false)?);
        }
        let result = reader
            .as_ref()
            .unwrap()
            .prepare_cached(
                "SELECT state,payload,legacy FROM source_tile_entries
                 WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4 LIMIT 1",
            )?
            .query_row(params![key.namespace_id, key.z, key.x, key.y], |row| {
                let state: i64 = row.get(0)?;
                let tile = if state == TILE_MISSING {
                    CachedTile::Missing
                } else {
                    CachedTile::Present(row.get(1)?)
                };
                Ok((tile, row.get::<_, bool>(2)?))
            })
            .optional()?;
        if result.is_some() {
            self.metrics.hits.fetch_add(1, Ordering::Relaxed);
            let command = if result.as_ref().is_some_and(|(_, legacy)| *legacy) {
                Command::Promote(key)
            } else {
                Command::Touch(key)
            };
            let _ = self.try_send(command, false);
        } else {
            self.metrics.misses.fetch_add(1, Ordering::Relaxed);
        }
        Ok(result.map(|(tile, _)| tile))
    }

    fn store(&self, key: Key, tile: CachedTile) -> bool {
        self.send_durable(Command::Write {
            key,
            value: tile.into_write(),
        })
    }

    fn import(&self, key: Key, tile: CachedTile) -> bool {
        self.send_durable(Command::Import {
            key,
            value: tile.into_write(),
        })
    }

    fn remove(&self, key: Key) {
        let _ = self.send_durable(Command::Remove(key));
    }

    fn set_pinned(&self, keys: Vec<Key>, pinned: bool) {
        for batch in keys.chunks(MAX_BATCH_COMMANDS) {
            let _ = self.send_durable(Command::SetPinned {
                keys: batch.to_vec(),
                pinned,
            });
        }
    }

    fn flush_checked(&self) -> bool {
        let (reply, response) = mpsc::channel();
        self.send_blocking(Command::Flush(reply))
            && response
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or(false)
    }

    fn try_send(&self, command: Command, count_drop: bool) -> bool {
        self.queued.fetch_add(1, Ordering::Relaxed);
        match self.tx.try_send(command) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                if count_drop {
                    self.metrics.dropped_writes.fetch_add(1, Ordering::Relaxed);
                }
                false
            }
        }
    }

    fn send_durable(&self, command: Command) -> bool {
        if self.send_blocking(command) {
            true
        } else {
            self.metrics.dropped_writes.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    fn send_blocking(&self, command: Command) -> bool {
        self.queued.fetch_add(1, Ordering::Relaxed);
        if self.tx.send(command).is_ok() {
            true
        } else {
            self.queued.fetch_sub(1, Ordering::Relaxed);
            false
        }
    }
}

impl Drop for StoreInner {
    fn drop(&mut self) {
        let (reply, response) = mpsc::channel();
        if self.send_blocking(Command::Shutdown(reply)) {
            let _ = response.recv_timeout(Duration::from_secs(30));
        }
        if let Some(writer) = self.writer.lock().unwrap().take() {
            let _ = writer.join();
        }
    }
}

#[derive(Clone)]
pub struct SourceTileStore {
    inner: Arc<StoreInner>,
}

impl SourceTileStore {
    pub fn open(path: impl Into<PathBuf>, budget_bytes: u64) -> rusqlite::Result<Self> {
        let path = path.into();
        let recovered_corruption = open_schema_with_recovery(&path)?;
        let reader_count = thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4)
            .clamp(2, 8);
        let readers = (0..reader_count).map(|_| Mutex::new(None)).collect();
        let (tx, rx) = mpsc::sync_channel(WRITER_QUEUE);
        let payload_budget = budget_bytes.saturating_mul(PAYLOAD_BUDGET_PERCENT) / 100;
        let queued = Arc::new(AtomicUsize::new(0));
        let metrics = Arc::new(StoreMetrics::default());
        if recovered_corruption {
            metrics.corruptions.store(1, Ordering::Relaxed);
        }
        let inner = Arc::new(StoreInner {
            path: path.clone(),
            readers,
            tx,
            queued: queued.clone(),
            metrics: metrics.clone(),
            budget_bytes,
            writer: Mutex::new(None),
        });
        let writer = thread::Builder::new()
            .name("source-tile-cache-writer".into())
            .spawn(move || {
                if let Err(error) =
                    writer_loop(&path, budget_bytes, payload_budget, rx, &queued, &metrics)
                {
                    metrics.write_errors.fetch_add(1, Ordering::Relaxed);
                    log::error!("source cache: writer stopped: {error}");
                }
            })
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        *inner.writer.lock().unwrap() = Some(writer);
        Ok(Self { inner })
    }

    /// Create the sole active revision for a provider. Rows from older
    /// revisions of that provider are deleted transactionally at startup.
    pub fn namespace(
        &self,
        provider: impl Into<String>,
        revision: impl Into<String>,
        content_type: impl Into<String>,
        legacy: Option<LegacyLayout>,
    ) -> rusqlite::Result<SourceTileCache> {
        let provider = provider.into();
        let revision = revision.into();
        let content_type = content_type.into();
        if provider.trim().is_empty() || revision.trim().is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "source-cache provider and revision must be non-empty".into(),
            ));
        }

        let mut connection = open_connection(&self.inner.path, true)?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO source_namespaces(provider,revision,content_type,created_at)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(provider,revision) DO UPDATE SET content_type=excluded.content_type",
            params![provider, revision, content_type, now_epoch_seconds()],
        )?;
        let namespace_id: i64 = transaction.query_row(
            "SELECT id FROM source_namespaces WHERE provider=?1 AND revision=?2",
            params![provider, revision],
            |row| row.get(0),
        )?;
        let pruned = transaction.execute(
            "DELETE FROM source_namespaces WHERE provider=?1 AND revision<>?2",
            params![provider, revision],
        )?;
        transaction.commit()?;
        if pruned > 0 {
            incremental_vacuum(&connection)?;
            log::info!(
                "source cache: pruned {pruned} obsolete namespace(s) for {provider}; keeping {revision}"
            );
        }
        refresh_metrics(&connection, &self.inner.metrics)?;

        Ok(SourceTileCache {
            inner: self.inner.clone(),
            namespace_id,
            provider,
            revision,
            legacy,
        })
    }

    pub fn flush(&self) -> bool {
        self.inner.flush_checked()
    }

    pub fn stats(&self) -> SourceCacheStats {
        let file_bytes = |path: &Path| path.metadata().map_or(0, |meta| meta.len());
        let mut physical_bytes = file_bytes(&self.inner.path);
        physical_bytes = physical_bytes.saturating_add(file_bytes(&PathBuf::from(format!(
            "{}-wal",
            self.inner.path.display()
        ))));
        physical_bytes = physical_bytes.saturating_add(file_bytes(&PathBuf::from(format!(
            "{}-shm",
            self.inner.path.display()
        ))));
        let metrics = &self.inner.metrics;
        SourceCacheStats {
            hits: metrics.hits.load(Ordering::Relaxed),
            misses: metrics.misses.load(Ordering::Relaxed),
            writes: metrics.writes.load(Ordering::Relaxed),
            evictions: metrics.evictions.load(Ordering::Relaxed),
            corruptions: metrics.corruptions.load(Ordering::Relaxed),
            write_errors: metrics.write_errors.load(Ordering::Relaxed),
            dropped_writes: metrics.dropped_writes.load(Ordering::Relaxed),
            logical_bytes: metrics.logical_bytes.load(Ordering::Relaxed),
            entries: metrics.entries.load(Ordering::Relaxed),
            pinned_entries: metrics.pinned_entries.load(Ordering::Relaxed),
            pinned_bytes: metrics.pinned_bytes.load(Ordering::Relaxed),
            migrations: metrics.migrations.load(Ordering::Relaxed),
            physical_bytes,
            budget_bytes: self.inner.budget_bytes,
            queued_commands: self.inner.queued.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Debug)]
pub enum LegacyLayout {
    Terrarium { root: PathBuf },
    Vectors { root: PathBuf, build_tag: String },
}

impl LegacyLayout {
    fn scan_root(&self) -> PathBuf {
        match self {
            Self::Terrarium { root } => root.clone(),
            Self::Vectors { root, build_tag } => root.join(build_tag),
        }
    }

    fn path(&self, z: u8, x: u32, y: u32) -> PathBuf {
        match self {
            Self::Terrarium { root } => root.join(format!("{z}/{x}/{y}.png")),
            Self::Vectors { root, build_tag } => {
                root.join(build_tag).join(format!("{z}/{x}_{y}.mvt"))
            }
        }
    }

    fn parse(&self, path: &Path) -> Option<(u8, u32, u32)> {
        let relative = path.strip_prefix(self.scan_root()).ok()?;
        let components: Vec<_> = relative.components().collect();
        if components.len() != 2 {
            return None;
        }
        let z = components[0].as_os_str().to_str()?.parse().ok()?;
        let filename = components[1].as_os_str().to_str()?;
        match self {
            // Terrarium uses z/x/y.png and is parsed by `parse_legacy_path`.
            Self::Terrarium { .. } => None,
            Self::Vectors { .. } => parse_joined_tile(z, filename, ".mvt"),
        }
    }

    fn tile_from_bytes(&self, bytes: Vec<u8>) -> CachedTile {
        if matches!(self, Self::Vectors { .. }) && bytes.is_empty() {
            CachedTile::Missing
        } else {
            CachedTile::Present(bytes)
        }
    }

    fn prune_obsolete_vector_builds(&self) {
        let Self::Vectors { root, build_tag } = self else {
            return;
        };
        if !regular_directory(root) {
            return;
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let is_build = name.len() == 8 && name.bytes().all(|byte| byte.is_ascii_digit());
            if is_build && name != build_tag {
                match std::fs::remove_dir_all(entry.path()) {
                    Ok(()) => log::info!("source cache: removed obsolete vector build {name}"),
                    Err(error) => {
                        log::warn!("source cache: could not remove vector build {name}: {error}")
                    }
                }
            }
        }
    }
}

fn parse_joined_tile(z: u8, filename: &str, suffix: &str) -> Option<(u8, u32, u32)> {
    let stem = filename.strip_suffix(suffix)?;
    let (x, y) = stem.split_once('_')?;
    let coordinates = (z, x.parse().ok()?, y.parse().ok()?);
    valid_tile_coordinates(coordinates).then_some(coordinates)
}

fn valid_tile_coordinates((z, x, y): (u8, u32, u32)) -> bool {
    z <= 30 && (x as u64) < (1u64 << z) && (y as u64) < (1u64 << z)
}

#[derive(Clone)]
pub struct SourceTileCache {
    inner: Arc<StoreInner>,
    namespace_id: i64,
    provider: String,
    revision: String,
    legacy: Option<LegacyLayout>,
}

impl SourceTileCache {
    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn legacy_layout(&self) -> Option<&LegacyLayout> {
        self.legacy.as_ref()
    }

    pub fn load(&self, z: u8, x: u32, y: u32) -> rusqlite::Result<Option<CachedTile>> {
        let key = self.key(z, x, y);
        if let Some(tile) = self.inner.read(key)? {
            return Ok(Some(tile));
        }

        let Some(legacy) = &self.legacy else {
            return Ok(None);
        };
        let path = legacy.path(z, x, y);
        if !regular_legacy_file(&legacy.scan_root(), &path) {
            // A background migration may have removed the committed file, or
            // this path may cross a symlink. Recheck the database for the
            // former and never follow the latter.
            return self.inner.read(key);
        }
        match std::fs::read(&path) {
            Ok(bytes) => {
                let tile = legacy.tile_from_bytes(bytes);
                if self.inner.import(key, tile.clone()) && self.inner.flush_checked() {
                    self.inner
                        .metrics
                        .migrations
                        .fetch_add(1, Ordering::Relaxed);
                    let _ = std::fs::remove_file(&path);
                    cleanup_empty_parents(path.parent(), &legacy.scan_root());
                }
                // If another acquisition won the race, return its newer row;
                // otherwise the just-read legacy bytes still satisfy this
                // request even if the byte budget evicted them immediately.
                Ok(self.inner.read(key)?.or(Some(tile)))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A background migrator may commit and remove the file after
                // the first database miss. Recheck once before reporting a
                // cold miss.
                self.inner.read(key)
            }
            Err(error) => {
                log::warn!("source cache: legacy read {}: {error}", path.display());
                Ok(None)
            }
        }
    }

    pub fn store_present(&self, z: u8, x: u32, y: u32, bytes: Vec<u8>) -> bool {
        self.inner
            .store(self.key(z, x, y), CachedTile::Present(bytes))
    }

    pub fn store_missing(&self, z: u8, x: u32, y: u32) -> bool {
        self.inner.store(self.key(z, x, y), CachedTile::Missing)
    }

    pub fn remove(&self, z: u8, x: u32, y: u32) {
        self.inner.remove(self.key(z, x, y));
        if let Some(legacy) = &self.legacy {
            let path = legacy.path(z, x, y);
            let _ = std::fs::remove_file(&path);
            cleanup_empty_parents(path.parent(), &legacy.scan_root());
        }
    }

    pub fn remove_corrupt(&self, z: u8, x: u32, y: u32) {
        self.inner
            .metrics
            .corruptions
            .fetch_add(1, Ordering::Relaxed);
        self.remove(z, x, y);
    }

    pub fn set_pinned(&self, tiles: &[(u8, u32, u32)], pinned: bool) {
        self.inner.set_pinned(
            tiles.iter().map(|&(z, x, y)| self.key(z, x, y)).collect(),
            pinned,
        );
    }

    pub fn flush(&self) -> bool {
        self.inner.flush_checked()
    }

    pub fn migrate_legacy(&self) -> MigrationReport {
        let Some(layout) = self.legacy.clone() else {
            return MigrationReport::default();
        };
        let root = layout.scan_root();
        if !regular_directory(&root) {
            layout.prune_obsolete_vector_builds();
            return MigrationReport::default();
        }

        let mut report = MigrationReport::default();
        let mut pending = Vec::new();
        let mut pending_bytes = 0usize;
        let mut stack = vec![root.clone()];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                if file_type.is_dir() {
                    stack.push(path);
                    continue;
                }
                // Do not follow symlinks out of a user-controlled legacy
                // tree and then delete their targets after migration.
                if !file_type.is_file() {
                    continue;
                }
                let Some((z, x, y)) = parse_legacy_path(&layout, &path) else {
                    continue;
                };
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        pending_bytes = pending_bytes.saturating_add(bytes.len());
                        pending.push((self.key(z, x, y), layout.tile_from_bytes(bytes), path));
                    }
                    Err(error) => {
                        report.errors += 1;
                        log::warn!("source cache: migration read {}: {error}", path.display());
                    }
                }
                if pending.len() >= MIGRATION_BATCH_ENTRIES
                    || pending_bytes >= MIGRATION_BATCH_BYTES
                {
                    self.import_migration_batch(&root, &mut pending, &mut report);
                    pending_bytes = 0;
                }
            }
        }
        self.import_migration_batch(&root, &mut pending, &mut report);
        layout.prune_obsolete_vector_builds();
        if report.imported > 0 || report.errors > 0 {
            log::info!(
                "source cache: migrated {} {} tiles ({} bytes, {} errors)",
                report.imported,
                self.provider,
                report.bytes,
                report.errors
            );
        }
        report
    }

    pub fn start_legacy_migration(&self) -> std::io::Result<JoinHandle<MigrationReport>> {
        let cache = self.clone();
        thread::Builder::new()
            .name(format!(
                "source-cache-migrate-{}",
                sanitize_thread_name(&self.provider)
            ))
            .spawn(move || cache.migrate_legacy())
    }

    fn import_migration_batch(
        &self,
        root: &Path,
        pending: &mut Vec<(Key, CachedTile, PathBuf)>,
        report: &mut MigrationReport,
    ) {
        if pending.is_empty() {
            return;
        }
        let mut queued = true;
        for (key, tile, _) in pending.iter() {
            queued &= self.inner.import(*key, tile.clone());
        }
        if queued && self.inner.flush_checked() {
            for (_, tile, path) in pending.drain(..) {
                report.imported += 1;
                report.bytes = report
                    .bytes
                    .saturating_add(tile.bytes().map_or(0, |bytes| bytes.len()) as u64);
                self.inner
                    .metrics
                    .migrations
                    .fetch_add(1, Ordering::Relaxed);
                if let Err(error) = std::fs::remove_file(&path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    report.errors += 1;
                    log::warn!("source cache: migration remove {}: {error}", path.display());
                }
                cleanup_empty_parents(path.parent(), root);
            }
        } else {
            report.errors = report.errors.saturating_add(pending.len() as u64);
            pending.clear();
        }
    }

    fn key(&self, z: u8, x: u32, y: u32) -> Key {
        Key {
            namespace_id: self.namespace_id,
            z,
            x,
            y,
        }
    }
}

fn parse_legacy_path(layout: &LegacyLayout, path: &Path) -> Option<(u8, u32, u32)> {
    if matches!(layout, LegacyLayout::Terrarium { .. }) {
        let relative = path.strip_prefix(layout.scan_root()).ok()?;
        let components: Vec<_> = relative.components().collect();
        if components.len() != 3 {
            return None;
        }
        let z = components[0].as_os_str().to_str()?.parse().ok()?;
        let x = components[1].as_os_str().to_str()?.parse().ok()?;
        let y = components[2]
            .as_os_str()
            .to_str()?
            .strip_suffix(".png")?
            .parse()
            .ok()?;
        valid_tile_coordinates((z, x, y)).then_some((z, x, y))
    } else {
        layout.parse(path)
    }
}

fn sanitize_thread_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .take(32)
        .collect()
}

fn regular_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

fn regular_legacy_file(root: &Path, path: &Path) -> bool {
    if !regular_directory(root) {
        return false;
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut current = root.to_path_buf();
    let mut last_type = None;
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return false;
        };
        current.push(component);
        let Ok(metadata) = std::fs::symlink_metadata(&current) else {
            return false;
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return false;
        }
        last_type = Some(file_type);
    }
    last_type.is_some_and(|file_type| file_type.is_file())
}

fn cleanup_empty_parents(mut parent: Option<&Path>, stop: &Path) {
    while let Some(directory) = parent {
        if directory == stop || !directory.starts_with(stop) {
            break;
        }
        if std::fs::remove_dir(directory).is_err() {
            break;
        }
        parent = directory.parent();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub imported: u64,
    pub bytes: u64,
    pub errors: u64,
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn open_connection(path: &Path, writer: bool) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    connection.pragma_update(None, "cache_size", if writer { -16384 } else { -4096 })?;
    connection.pragma_update(None, "wal_autocheckpoint", 1000)?;
    Ok(connection)
}

fn initialize_schema(path: &Path) -> rusqlite::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let new_database = !path.exists() || path.metadata().map_or(true, |meta| meta.len() == 0);
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    if new_database {
        connection.pragma_update(None, "page_size", 4096)?;
        connection.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        connection.pragma_update(None, "application_id", 0x5645_5354_i64)?; // "VEST"
    }
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", true)?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let migrate_tiles = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='source_tiles'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some_and(|sql| sql.to_ascii_uppercase().contains("WITHOUT ROWID"));
    if migrate_tiles {
        // Keep the old LRU index: rebuilding an index on the payload tree is
        // itself a whole-cache scan. Only the empty replacement needs an index.
        transaction.execute_batch(
            "DROP TRIGGER IF EXISTS source_tiles_insert_meta;
             DROP TRIGGER IF EXISTS source_tiles_delete_meta;
             DROP TRIGGER IF EXISTS source_tiles_update_meta;
             ALTER TABLE source_tiles RENAME TO source_tiles_legacy;",
        )?;
    }
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS source_namespaces (
             id           INTEGER PRIMARY KEY,
             provider     TEXT NOT NULL,
             revision     TEXT NOT NULL,
             content_type TEXT NOT NULL,
             created_at   INTEGER NOT NULL,
             UNIQUE(provider, revision)
         );
         CREATE TABLE IF NOT EXISTS source_tiles (
             namespace_id INTEGER NOT NULL REFERENCES source_namespaces(id) ON DELETE CASCADE,
             z            INTEGER NOT NULL CHECK(z BETWEEN 0 AND 30),
             x            INTEGER NOT NULL CHECK(x >= 0),
             y            INTEGER NOT NULL CHECK(y >= 0),
             state        INTEGER NOT NULL CHECK(state IN (0, 1)),
             payload_bytes INTEGER NOT NULL CHECK(payload_bytes >= 0),
             last_access  INTEGER NOT NULL,
             inserted_at  INTEGER NOT NULL,
             pinned       INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
             payload      BLOB,
             PRIMARY KEY(namespace_id, z, x, y),
             CHECK((state=0 AND payload IS NULL AND payload_bytes=0)
                   OR (state=1 AND payload IS NOT NULL))
         );
         CREATE TABLE IF NOT EXISTS source_cache_meta (
             id            INTEGER PRIMARY KEY CHECK(id = 0),
             logical_bytes INTEGER NOT NULL,
             entry_count   INTEGER NOT NULL
         );",
    )?;
    let has_lru: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='index'
         AND tbl_name='source_tiles' AND name IN ('source_tiles_lru','source_tiles_rowid_lru'))",
        [],
        |row| row.get(0),
    )?;
    if !has_lru {
        transaction.execute_batch(
            "CREATE INDEX source_tiles_rowid_lru ON source_tiles(pinned,last_access);",
        )?;
    }
    let pending = schema_migration_pending(&transaction)?;
    install_metadata_triggers(&transaction, "source_tiles")?;
    if pending {
        install_metadata_triggers(&transaction, "source_tiles_legacy")?;
    }
    let has_view: bool = transaction.query_row(
        "SELECT COUNT(*)=2 FROM sqlite_schema WHERE type='view'
         AND name IN ('source_tile_entries','source_tile_metadata')",
        [],
        |row| row.get(0),
    )?;
    if !has_view || migrate_tiles {
        install_source_view(&transaction, pending)?;
    }
    // Both stores contribute to one accounting row. Moving an entry deletes
    // and reinserts it in the same transaction, leaving the total unchanged.
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO source_cache_meta(id, logical_bytes, entry_count) VALUES(0, 0, 0)",
        [],
    )?;
    if inserted != 0 {
        transaction.execute_batch(&format!(
            "UPDATE source_cache_meta SET
             logical_bytes=COALESCE(
                 (SELECT SUM(payload_bytes+{ENTRY_BUDGET_BYTES}) FROM source_tile_metadata), 0
             ),
             entry_count=(SELECT COUNT(*) FROM source_tile_metadata)
             WHERE id=0;"
        ))?;
    }
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 3 {
        transaction.pragma_update(None, "user_version", 3)?;
    }
    transaction.commit()?;
    Ok(())
}

fn schema_migration_pending(connection: &Connection) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='source_tiles_legacy')",
        [],
        |row| row.get(0),
    )
}

fn install_metadata_triggers(connection: &Connection, table: &str) -> rusqlite::Result<()> {
    debug_assert!(matches!(table, "source_tiles" | "source_tiles_legacy"));
    connection.execute_batch(&format!(
        "CREATE TRIGGER IF NOT EXISTS {table}_insert_meta AFTER INSERT ON {table} BEGIN
             UPDATE source_cache_meta
             SET logical_bytes=logical_bytes+NEW.payload_bytes+{ENTRY_BUDGET_BYTES},
                 entry_count=entry_count+1 WHERE id=0;
         END;
         CREATE TRIGGER IF NOT EXISTS {table}_delete_meta AFTER DELETE ON {table} BEGIN
             UPDATE source_cache_meta
             SET logical_bytes=logical_bytes-OLD.payload_bytes-{ENTRY_BUDGET_BYTES},
                 entry_count=entry_count-1 WHERE id=0;
         END;
         CREATE TRIGGER IF NOT EXISTS {table}_update_meta
         AFTER UPDATE OF payload_bytes ON {table} BEGIN
             UPDATE source_cache_meta
             SET logical_bytes=logical_bytes+NEW.payload_bytes-OLD.payload_bytes WHERE id=0;
         END;"
    ))
}

fn install_source_view(connection: &Connection, pending: bool) -> rusqlite::Result<()> {
    let legacy = if pending {
        " UNION ALL SELECT namespace_id,z,x,y,state,payload_bytes,last_access,inserted_at,pinned,payload,
           1 AS legacy FROM source_tiles_legacy"
    } else {
        ""
    };
    let legacy_metadata = if pending {
        " UNION ALL SELECT namespace_id,z,x,y,payload_bytes,last_access,pinned FROM source_tiles_legacy"
    } else {
        ""
    };
    connection.execute_batch(&format!(
        "DROP VIEW IF EXISTS source_tile_entries;
         DROP VIEW IF EXISTS source_tile_metadata;
         CREATE VIEW source_tile_entries AS
         SELECT namespace_id,z,x,y,state,payload_bytes,last_access,inserted_at,pinned,payload,
         0 AS legacy FROM source_tiles{legacy};
         CREATE VIEW source_tile_metadata AS
         SELECT namespace_id,z,x,y,payload_bytes,last_access,pinned
         FROM source_tiles{legacy_metadata};"
    ))
}

struct MigratingTile {
    key: Key,
    state: i64,
    payload: Option<Vec<u8>>,
    payload_bytes: i64,
    last_access: i64,
    inserted_at: i64,
    pinned: bool,
}

fn migrating_tile(row: &rusqlite::Row<'_>) -> rusqlite::Result<MigratingTile> {
    Ok(MigratingTile {
        key: Key {
            namespace_id: row.get(0)?,
            z: row.get(1)?,
            x: row.get(2)?,
            y: row.get(3)?,
        },
        state: row.get(4)?,
        payload: row.get(5)?,
        payload_bytes: row.get(6)?,
        last_access: row.get(7)?,
        inserted_at: row.get(8)?,
        pinned: row.get(9)?,
    })
}

fn migrate_schema_batch(
    connection: &mut Connection,
    preferred: &mut HashSet<Key>,
    metrics: &StoreMetrics,
) -> rusqlite::Result<bool> {
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if !schema_migration_pending(&transaction)? {
        preferred.clear();
        return Ok(false);
    }
    let mut rows = Vec::new();
    let mut bytes = 0usize;
    let mut selected = HashSet::new();
    let keys: Vec<_> = preferred
        .iter()
        .copied()
        .take(SCHEMA_MIGRATION_ENTRIES)
        .collect();
    for key in keys {
        preferred.remove(&key);
        if let Some(mut tile) = transaction.query_row(
            "SELECT namespace_id,z,x,y,state,payload,payload_bytes,last_access,inserted_at,pinned
             FROM source_tiles_legacy WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
            params![key.namespace_id, key.z, key.x, key.y],
            migrating_tile,
        ).optional()? {
            tile.last_access = tile.last_access.max(now_epoch_seconds());
            let next_bytes = bytes.saturating_add(tile.payload.as_ref().map_or(0, Vec::len));
            if !rows.is_empty() && next_bytes > SCHEMA_MIGRATION_BYTES {
                preferred.insert(key);
                break;
            }
            bytes = next_bytes;
            selected.insert(tile.key);
            rows.push(tile);
            if bytes >= SCHEMA_MIGRATION_BYTES {
                break;
            }
        }
    }
    if rows.len() < SCHEMA_MIGRATION_ENTRIES && bytes < SCHEMA_MIGRATION_BYTES {
        let mut statement = transaction.prepare(
            "SELECT namespace_id,z,x,y,state,payload,payload_bytes,last_access,inserted_at,pinned
             FROM source_tiles_legacy ORDER BY namespace_id,z,x,y LIMIT ?1",
        )?;
        let mut candidates = statement.query([SCHEMA_MIGRATION_ENTRIES as i64])?;
        while let Some(row) = candidates.next()? {
            let tile = migrating_tile(row)?;
            if !selected.insert(tile.key) {
                continue;
            }
            let next_bytes = bytes.saturating_add(tile.payload.as_ref().map_or(0, Vec::len));
            if !rows.is_empty() && next_bytes > SCHEMA_MIGRATION_BYTES {
                break;
            }
            bytes = next_bytes;
            rows.push(tile);
            if rows.len() >= SCHEMA_MIGRATION_ENTRIES || bytes >= SCHEMA_MIGRATION_BYTES {
                break;
            }
        }
    }
    let moved = rows.len();
    for tile in rows {
        // Release each old payload before inserting its replacement. SQLite
        // reuses those pages inside this atomic transaction, so migration does
        // not need two full copies or a temporary whole-cache allocation cap.
        transaction.execute(
            "DELETE FROM source_tiles_legacy WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
            params![tile.key.namespace_id, tile.key.z, tile.key.x, tile.key.y],
        )?;
        transaction.execute(
            "INSERT INTO source_tiles
             (namespace_id,z,x,y,state,payload,payload_bytes,last_access,inserted_at,pinned)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(namespace_id,z,x,y) DO NOTHING",
            params![
                tile.key.namespace_id,
                tile.key.z,
                tile.key.x,
                tile.key.y,
                tile.state,
                tile.payload,
                tile.payload_bytes,
                tile.last_access,
                tile.inserted_at,
                tile.pinned
            ],
        )?;
    }
    let remaining: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM source_tiles_legacy LIMIT 1)",
        [],
        |row| row.get(0),
    )?;
    if !remaining {
        install_source_view(&transaction, false)?;
        transaction.execute_batch("DROP TABLE source_tiles_legacy;")?;
    }
    transaction.commit()?;
    metrics
        .migrations
        .fetch_add(moved as u64, Ordering::Relaxed);
    if !remaining {
        preferred.clear();
        log::info!("source cache: payload-index migration complete");
    }
    Ok(remaining)
}

fn open_schema_with_recovery(path: &Path) -> rusqlite::Result<bool> {
    match initialize_schema(path) {
        Ok(()) => Ok(false),
        Err(error) if sqlite_corruption(&error) => {
            let suffix = now_epoch_seconds();
            log::error!(
                "source cache: quarantining corrupt indexed store {}: {error}",
                path.display()
            );
            for candidate in [
                path.to_path_buf(),
                PathBuf::from(format!("{}-wal", path.display())),
                PathBuf::from(format!("{}-shm", path.display())),
            ] {
                if candidate.exists() {
                    let quarantined =
                        PathBuf::from(format!("{}.corrupt-{suffix}", candidate.display()));
                    let _ = std::fs::rename(candidate, quarantined);
                }
            }
            initialize_schema(path).map(|()| true)
        }
        Err(error) => Err(error),
    }
}

fn sqlite_corruption(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(code, _)
            if matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            )
    )
}

fn writer_loop(
    path: &Path,
    total_budget: u64,
    payload_budget: u64,
    rx: Receiver<Command>,
    queued: &AtomicUsize,
    metrics: &StoreMetrics,
) -> rusqlite::Result<()> {
    let mut connection = open_connection(path, true)?;
    let startup_evictions = evict_to_budget(&mut connection, payload_budget, metrics)?;
    if startup_evictions > 0 {
        // A lower configured budget must return old high-water allocation
        // even if the user performs no subsequent writes.
        incremental_vacuum(&connection)?;
    }
    let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
    let max_pages = total_budget
        .max(MIN_DATABASE_ALLOCATION)
        .div_ceil(page_size.max(1) as u64)
        .max(1)
        .min(i64::MAX as u64) as i64;
    connection.pragma_update(None, "max_page_count", max_pages)?;
    refresh_metrics(&connection, metrics)?;
    let mut pending_touches = HashSet::new();
    let mut preferred_migrations = HashSet::new();
    let mut schema_pending = schema_migration_pending(&connection)?;
    let mut last_schema_batch = Instant::now();
    let mut last_reclamation = Instant::now();
    let mut last_touch_flush = Instant::now();
    // A flush is a durability barrier for every earlier command, even when
    // the failing write was processed in a previous writer batch.
    let mut unreported_failure = false;
    let mut running = true;

    while running {
        if schema_pending && last_schema_batch.elapsed() >= SCHEMA_MIGRATION_INTERVAL {
            match migrate_schema_batch(&mut connection, &mut preferred_migrations, metrics) {
                Ok(pending) => schema_pending = pending,
                Err(error) => {
                    // The failed transaction rolls back without discarding
                    // source bytes. Foreground cache access continues and a
                    // future open resumes from the remaining legacy rows.
                    schema_pending = false;
                    log::warn!("source cache: payload-index migration paused: {error}");
                }
            }
            last_schema_batch = Instant::now();
        }
        if !schema_pending
            && queued.load(Ordering::Relaxed) == 0
            && last_reclamation.elapsed() >= Duration::from_millis(500)
        {
            if let Err(error) = incremental_vacuum(&connection) {
                log::warn!("source cache: page reclamation deferred: {error}");
            }
            last_reclamation = Instant::now();
        }
        let first = match rx.recv_timeout(BATCH_DELAY) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !pending_touches.is_empty() && last_touch_flush.elapsed() >= TOUCH_FLUSH_INTERVAL
                {
                    flush_batch(
                        &mut connection,
                        payload_budget,
                        HashMap::new(),
                        Vec::new(),
                        &mut pending_touches,
                        metrics,
                    )?;
                    last_touch_flush = Instant::now();
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        let mut bytes = command_bytes(&first);
        let mut commands = vec![first];
        let deadline = Instant::now() + BATCH_DELAY;
        while commands.len() < MAX_BATCH_COMMANDS && bytes < MAX_BATCH_BYTES {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(command) => {
                    bytes = bytes.saturating_add(command_bytes(&command));
                    commands.push(command);
                }
                Err(_) => break,
            }
        }
        queued.fetch_sub(commands.len(), Ordering::Relaxed);

        // Preserve sequential semantics for each key within one batch. A
        // corrupt entry can be removed and replaced before the writer wakes,
        // while a conditional legacy import must never replace a normal
        // acquisition queued just before it.
        let mut mutations = HashMap::new();
        let mut pins = Vec::new();
        let mut flush_replies = Vec::new();
        let mut shutdown_replies = Vec::new();
        let mut force_touch_flush = false;
        for command in commands {
            match command {
                Command::Write { key, value } => {
                    coalesce_mutation(&mut mutations, key, Mutation::Upsert(value));
                }
                Command::Import { key, value } => {
                    coalesce_mutation(&mut mutations, key, Mutation::InsertIfAbsent(value));
                }
                Command::Touch(key) => {
                    pending_touches.insert(key);
                }
                Command::Promote(key) => {
                    pending_touches.insert(key);
                    if preferred_migrations.len() < MAX_PENDING_TOUCHES {
                        preferred_migrations.insert(key);
                    }
                }
                Command::Remove(key) => {
                    coalesce_mutation(&mut mutations, key, Mutation::Remove);
                }
                Command::SetPinned { keys, pinned } => pins.push((keys, pinned)),
                Command::Flush(reply) => {
                    force_touch_flush = true;
                    flush_replies.push(reply);
                }
                Command::Shutdown(reply) => {
                    force_touch_flush = true;
                    running = false;
                    shutdown_replies.push(reply);
                }
            }
        }

        let incoming_bytes: u64 = mutations
            .values()
            .filter_map(|mutation| match mutation {
                Mutation::Upsert(value) | Mutation::InsertIfAbsent(value) => Some(value),
                Mutation::Remove => None,
            })
            .map(|value| value.payload_len() as u64)
            .sum();
        let eviction_possible = metrics
            .logical_bytes
            .load(Ordering::Relaxed)
            .saturating_add(incoming_bytes)
            > payload_budget;
        let touch_due = force_touch_flush
            || eviction_possible
            || pending_touches.len() >= MAX_PENDING_TOUCHES
            || last_touch_flush.elapsed() >= TOUCH_FLUSH_INTERVAL;
        let mut no_touches = HashSet::new();
        let touches = if touch_due {
            &mut pending_touches
        } else {
            &mut no_touches
        };
        let batch_result = if mutations.is_empty() && pins.is_empty() && touches.is_empty() {
            Ok(())
        } else {
            flush_batch(
                &mut connection,
                payload_budget,
                mutations,
                pins,
                touches,
                metrics,
            )
        };
        let batch_ok = batch_result.is_ok();
        if let Err(error) = batch_result {
            unreported_failure = true;
            metrics.write_errors.fetch_add(1, Ordering::Relaxed);
            log::warn!("source cache: indexed write batch failed: {error}");
        }
        if touch_due {
            pending_touches.clear();
            last_touch_flush = Instant::now();
        }
        let flush_ok = batch_ok && !unreported_failure;
        let acknowledged_failure = !flush_replies.is_empty();
        for reply in flush_replies {
            let _ = reply.send(flush_ok);
        }
        if acknowledged_failure {
            unreported_failure = false;
        }
        for reply in shutdown_replies {
            let _ = reply.send(());
        }
    }
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    Ok(())
}

fn coalesce_mutation(mutations: &mut HashMap<Key, Mutation>, key: Key, next: Mutation) {
    match next {
        // A normal acquisition is authoritative and overwrites every earlier
        // pending state for this key.
        Mutation::Upsert(_) | Mutation::Remove => {
            mutations.insert(key, next);
        }
        Mutation::InsertIfAbsent(_) => match mutations.get(&key) {
            // An earlier upsert already makes the later conditional import a
            // no-op. The first of multiple conditional imports wins too.
            Some(Mutation::Upsert(_)) | Some(Mutation::InsertIfAbsent(_)) => {}
            // A remove followed by an import should insert if the row is now
            // absent, matching the unbatched command sequence.
            Some(Mutation::Remove) | None => {
                mutations.insert(key, next);
            }
        },
    }
}

fn command_bytes(command: &Command) -> usize {
    match command {
        Command::Write { value, .. } | Command::Import { value, .. } => value.payload_len(),
        _ => 0,
    }
}

fn flush_batch(
    connection: &mut Connection,
    payload_budget: u64,
    mutations: HashMap<Key, Mutation>,
    pins: Vec<(Vec<Key>, bool)>,
    touches: &mut HashSet<Key>,
    metrics: &StoreMetrics,
) -> rusqlite::Result<()> {
    let now = now_epoch_seconds();
    let write_count = mutations
        .values()
        .filter(|mutation| !matches!(mutation, Mutation::Remove))
        .count() as u64;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let pending = schema_migration_pending(&transaction)?;
    for (key, mutation) in mutations {
        match mutation {
            Mutation::Upsert(value) => {
                let legacy_pinned = if pending {
                    transaction
                        .query_row(
                            "SELECT pinned FROM source_tiles_legacy
                         WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
                            params![key.namespace_id, key.z, key.x, key.y],
                            |row| row.get::<_, bool>(0),
                        )
                        .optional()?
                        .unwrap_or(false)
                } else {
                    false
                };
                let payload_bytes = value.payload_len() as i64;
                transaction.execute(
                    "INSERT INTO source_tiles(
                 namespace_id,z,x,y,state,payload,payload_bytes,last_access,inserted_at,pinned
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8,?9)
             ON CONFLICT(namespace_id,z,x,y) DO UPDATE SET
                 state=excluded.state,
                 payload=excluded.payload,
                 payload_bytes=excluded.payload_bytes,
                 last_access=excluded.last_access,
                 inserted_at=excluded.inserted_at,
                 pinned=MAX(source_tiles.pinned, excluded.pinned)",
                    params![
                        key.namespace_id,
                        key.z,
                        key.x,
                        key.y,
                        value.state,
                        value.payload.as_deref(),
                        payload_bytes,
                        now,
                        value.pinned || legacy_pinned
                    ],
                )?;
                if pending {
                    transaction.execute(
                        "DELETE FROM source_tiles_legacy
                         WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
                        params![key.namespace_id, key.z, key.x, key.y],
                    )?;
                }
            }
            Mutation::InsertIfAbsent(value) => {
                if pending
                    && transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM source_tiles_legacy
                     WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4)",
                        params![key.namespace_id, key.z, key.x, key.y],
                        |row| row.get::<_, bool>(0),
                    )?
                {
                    continue;
                }
                let payload_bytes = value.payload_len() as i64;
                transaction.execute(
                    "INSERT INTO source_tiles(
                         namespace_id,z,x,y,state,payload,payload_bytes,last_access,inserted_at,pinned
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8,?9)
                     ON CONFLICT(namespace_id,z,x,y) DO NOTHING",
                    params![
                        key.namespace_id,
                        key.z,
                        key.x,
                        key.y,
                        value.state,
                        value.payload.as_deref(),
                        payload_bytes,
                        now,
                        value.pinned
                    ],
                )?;
            }
            Mutation::Remove => {
                delete_cached_key(&transaction, key, pending)?;
            }
        }
    }
    for (keys, pinned) in pins {
        for key in keys {
            transaction.execute(
                "UPDATE source_tiles SET pinned=?5
                 WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
                params![key.namespace_id, key.z, key.x, key.y, pinned],
            )?;
            if pending {
                transaction.execute(
                    "UPDATE source_tiles_legacy SET pinned=?5
                     WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
                    params![key.namespace_id, key.z, key.x, key.y, pinned],
                )?;
            }
        }
    }
    for key in touches.drain() {
        transaction.execute(
            "UPDATE source_tiles SET last_access=?5
             WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
            params![key.namespace_id, key.z, key.x, key.y, now],
        )?;
        if pending {
            transaction.execute(
                "UPDATE source_tiles_legacy SET last_access=?5
                 WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
                params![key.namespace_id, key.z, key.x, key.y, now],
            )?;
        }
    }
    transaction.commit()?;
    metrics.writes.fetch_add(write_count, Ordering::Relaxed);
    let evicted = evict_to_budget(connection, payload_budget, metrics)?;
    if evicted > 0 {
        incremental_vacuum(connection)?;
    }
    refresh_metrics(connection, metrics)
}

fn delete_cached_key(connection: &Connection, key: Key, pending: bool) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM source_tiles WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
        params![key.namespace_id, key.z, key.x, key.y],
    )?;
    if pending {
        connection.execute(
            "DELETE FROM source_tiles_legacy WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4",
            params![key.namespace_id, key.z, key.x, key.y],
        )?;
    }
    Ok(())
}

fn evict_to_budget(
    connection: &mut Connection,
    payload_budget: u64,
    metrics: &StoreMetrics,
) -> rusqlite::Result<usize> {
    let mut total_removed = 0usize;
    let mut logical_bytes = connection.query_row(
        "SELECT logical_bytes FROM source_cache_meta WHERE id=0",
        [],
        |row| row.get::<_, i64>(0),
    )? as u64;
    while logical_bytes > payload_budget {
        let bytes_needed = logical_bytes - payload_budget;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let pending = schema_migration_pending(&transaction)?;
        let mut victims = Vec::new();
        let mut reclaimed = 0u64;
        {
            // A compound query merges the two already ordered LRU indexes.
            // Ordering the union view instead would materialize every unpinned
            // entry before selecting this small eviction batch.
            let query = if pending {
                "SELECT namespace_id,z,x,y,payload_bytes+?2,last_access FROM source_tiles
                 WHERE pinned=0 UNION ALL
                 SELECT namespace_id,z,x,y,payload_bytes+?2,last_access FROM source_tiles_legacy
                 WHERE pinned=0 ORDER BY last_access ASC LIMIT ?1"
            } else {
                "SELECT namespace_id,z,x,y,payload_bytes+?2 FROM source_tiles
                 WHERE pinned=0 ORDER BY last_access ASC LIMIT ?1"
            };
            let mut statement = transaction.prepare(query)?;
            let rows =
                statement.query_map(params![EVICT_BATCH as i64, ENTRY_BUDGET_BYTES], |row| {
                    Ok((
                        Key {
                            namespace_id: row.get(0)?,
                            z: row.get(1)?,
                            x: row.get(2)?,
                            y: row.get(3)?,
                        },
                        row.get::<_, i64>(4)?.max(0) as u64,
                    ))
                })?;
            for row in rows {
                let (key, bytes) = row?;
                victims.push(key);
                reclaimed = reclaimed.saturating_add(bytes);
                if reclaimed >= bytes_needed {
                    break;
                }
            }
        }
        for key in &victims {
            delete_cached_key(&transaction, *key, pending)?;
        }
        let removed = victims.len();
        transaction.commit()?;
        if removed == 0 {
            log::warn!(
                "source cache: pinned payload ({logical_bytes} bytes) exceeds disk budget ({payload_budget} bytes)"
            );
            break;
        }
        total_removed += removed;
        metrics
            .evictions
            .fetch_add(removed as u64, Ordering::Relaxed);
        logical_bytes = connection.query_row(
            "SELECT logical_bytes FROM source_cache_meta WHERE id=0",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64;
    }
    Ok(total_removed)
}

fn incremental_vacuum(connection: &Connection) -> rusqlite::Result<()> {
    let freelist: i64 = connection.pragma_query_value(None, "freelist_count", |row| row.get(0))?;
    if freelist >= VACUUM_FREELIST_THRESHOLD {
        // SQLite yields one row per reclaimed page; execute_batch steps only
        // once, silently reducing the requested reclamation batch to one page.
        let mut statement = connection.prepare(&format!(
            "PRAGMA incremental_vacuum({INCREMENTAL_VACUUM_PAGES})"
        ))?;
        let mut rows = statement.query([])?;
        while rows.next()?.is_some() {}
    }
    Ok(())
}

fn refresh_metrics(connection: &Connection, metrics: &StoreMetrics) -> rusqlite::Result<()> {
    let (logical_bytes, entries): (i64, i64) = connection.query_row(
        "SELECT logical_bytes,entry_count FROM source_cache_meta WHERE id=0",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let (pinned_entries, pinned_bytes): (i64, i64) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(payload_bytes+?1),0)
         FROM source_tile_metadata WHERE pinned=1",
        [ENTRY_BUDGET_BYTES],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    metrics
        .logical_bytes
        .store(logical_bytes.max(0) as u64, Ordering::Relaxed);
    metrics
        .entries
        .store(entries.max(0) as u64, Ordering::Relaxed);
    metrics
        .pinned_entries
        .store(pinned_entries.max(0) as u64, Ordering::Relaxed);
    metrics
        .pinned_bytes
        .store(pinned_bytes.max(0) as u64, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vearth-source-cache-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn migrates_payload_btree_without_losing_namespaces_pins_or_metadata() {
        let dir = temp_dir("payload-btree-migration");
        let path = dir.join("tiles.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "PRAGMA user_version=1;
             CREATE TABLE source_namespaces (
                id INTEGER PRIMARY KEY, provider TEXT NOT NULL, revision TEXT NOT NULL,
                content_type TEXT NOT NULL, created_at INTEGER NOT NULL,
                UNIQUE(provider, revision));
             CREATE TABLE source_tiles (
                namespace_id INTEGER NOT NULL REFERENCES source_namespaces(id) ON DELETE CASCADE,
                z INTEGER NOT NULL, x INTEGER NOT NULL, y INTEGER NOT NULL,
                state INTEGER NOT NULL, payload BLOB, payload_bytes INTEGER NOT NULL,
                last_access INTEGER NOT NULL, inserted_at INTEGER NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(namespace_id,z,x,y)) WITHOUT ROWID;
             CREATE INDEX source_tiles_lru ON source_tiles(pinned,last_access);
             CREATE TABLE source_cache_meta (
                id INTEGER PRIMARY KEY, logical_bytes INTEGER NOT NULL, entry_count INTEGER NOT NULL);
             INSERT INTO source_namespaces VALUES(17,'terrain','v1','png',123);
             INSERT INTO source_namespaces VALUES(29,'vectors','v2','mvt',456);",
        ).unwrap();
        let payload: Vec<u8> = (0..192 * 1024).map(|i| (i * 37) as u8).collect();
        connection
            .execute(
                "INSERT INTO source_tiles VALUES(17,3,1,2,1,?1,?2,100,90,1)",
                params![payload, payload.len() as i64],
            )
            .unwrap();
        connection
            .execute_batch("INSERT INTO source_tiles VALUES(29,4,8,7,0,NULL,0,200,190,0);")
            .unwrap();
        let expected_bytes = payload.len() as i64 + 2 * ENTRY_BUDGET_BYTES;
        connection
            .execute(
                "INSERT INTO source_cache_meta VALUES(0,?1,2)",
                [expected_bytes],
            )
            .unwrap();
        drop(connection);

        initialize_schema(&path).unwrap();
        let mut connection = open_connection(&path, true).unwrap();
        assert!(schema_migration_pending(&connection).unwrap());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM source_tiles", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "schema initialization must not copy the payload table"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM source_tile_entries", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2,
        );
        assert!(
            !migrate_schema_batch(
                &mut connection,
                &mut HashSet::new(),
                &StoreMetrics::default()
            )
            .unwrap()
        );
        let (bytes, accessed, inserted, pinned): (Vec<u8>, i64, i64, i64) = connection
            .query_row(
                "SELECT payload,last_access,inserted_at,pinned FROM source_tiles
             WHERE namespace_id=17 AND z=3 AND x=1 AND y=2",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(bytes, payload);
        assert_eq!((accessed, inserted, pinned), (100, 90, 1));
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM source_tiles WHERE namespace_id=29",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            TILE_MISSING
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT logical_bytes,entry_count FROM source_cache_meta WHERE id=0",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap(),
            (expected_bytes, 2)
        );
        let schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='source_tiles'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!schema.to_ascii_uppercase().contains("WITHOUT ROWID"));
        let query_plan: String = connection
            .query_row(
                "EXPLAIN QUERY PLAN SELECT state,payload FROM source_tiles
             WHERE namespace_id=17 AND z=3 AND x=1 AND y=2",
                [],
                |row| row.get(3),
            )
            .unwrap();
        assert!(
            query_plan.contains("USING INDEX sqlite_autoindex_source_tiles"),
            "{query_plan}"
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        connection
            .execute("DELETE FROM source_namespaces WHERE id=29", [])
            .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT logical_bytes,entry_count FROM source_cache_meta WHERE id=0",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap(),
            (expected_bytes - ENTRY_BUDGET_BYTES, 1)
        );
        drop(connection);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn legacy_schema_fixture(path: &Path, entries: u32, payload_size: usize) {
        let mut connection = Connection::open(path).unwrap();
        connection.execute_batch(
            "PRAGMA auto_vacuum=INCREMENTAL;
             PRAGMA user_version=1;
             CREATE TABLE source_namespaces (
                id INTEGER PRIMARY KEY, provider TEXT NOT NULL, revision TEXT NOT NULL,
                content_type TEXT NOT NULL, created_at INTEGER NOT NULL,
                UNIQUE(provider,revision));
             INSERT INTO source_namespaces VALUES(1,'terrain','v1','png',10);
             CREATE TABLE source_tiles (
                namespace_id INTEGER NOT NULL REFERENCES source_namespaces(id) ON DELETE CASCADE,
                z INTEGER NOT NULL,x INTEGER NOT NULL,y INTEGER NOT NULL,state INTEGER NOT NULL,
                payload BLOB,payload_bytes INTEGER NOT NULL,last_access INTEGER NOT NULL,
                inserted_at INTEGER NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(namespace_id,z,x,y)) WITHOUT ROWID;
             CREATE INDEX source_tiles_lru ON source_tiles(pinned,last_access);
             CREATE TABLE source_cache_meta (
                id INTEGER PRIMARY KEY,logical_bytes INTEGER NOT NULL,entry_count INTEGER NOT NULL);",
        ).unwrap();
        let transaction = connection.transaction().unwrap();
        for x in 0..entries {
            transaction
                .execute(
                    "INSERT INTO source_tiles VALUES(1,8,?1,0,1,?2,?3,?1,5,0)",
                    params![x, vec![x as u8; payload_size], payload_size as i64],
                )
                .unwrap();
        }
        transaction
            .execute(
                "INSERT INTO source_cache_meta VALUES(0,?1,?2)",
                params![
                    (payload_size as i64 + ENTRY_BUDGET_BYTES) * i64::from(entries),
                    entries
                ],
            )
            .unwrap();
        transaction.commit().unwrap();
    }

    fn assert_source_totals(connection: &Connection) {
        let counted: (i64, i64) = connection
            .query_row(
                "SELECT COALESCE(SUM(payload_bytes+?1),0),COUNT(*) FROM source_tile_entries",
                [ENTRY_BUDGET_BYTES],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let tracked: (i64, i64) = connection
            .query_row(
                "SELECT logical_bytes,entry_count FROM source_cache_meta WHERE id=0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(tracked, counted);
    }

    #[test]
    fn schema_migration_resumes_bounded_batches_after_reopening() {
        let dir = temp_dir("schema-resume");
        let path = dir.join("tiles.sqlite3");
        let entries = SCHEMA_MIGRATION_ENTRIES as u32 * 3 + 1;
        legacy_schema_fixture(&path, entries, 64);
        initialize_schema(&path).unwrap();
        let mut connection = open_connection(&path, true).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM source_tiles", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let metrics = StoreMetrics::default();
        assert!(migrate_schema_batch(&mut connection, &mut HashSet::new(), &metrics).unwrap());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM source_tiles", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            SCHEMA_MIGRATION_ENTRIES as i64
        );
        assert_source_totals(&connection);
        drop(connection);

        initialize_schema(&path).unwrap();
        let mut connection = open_connection(&path, true).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM source_tiles", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            SCHEMA_MIGRATION_ENTRIES as i64
        );
        for x in 0..entries {
            let payload: Vec<u8> = connection.query_row(
                "SELECT payload FROM source_tile_entries WHERE namespace_id=1 AND z=8 AND x=?1 AND y=0",
                [x], |row| row.get(0),
            ).unwrap();
            assert_eq!(payload, vec![x as u8; 64]);
        }
        let mut batches = 1;
        loop {
            let pending =
                migrate_schema_batch(&mut connection, &mut HashSet::new(), &metrics).unwrap();
            batches += 1;
            assert_source_totals(&connection);
            if !pending {
                break;
            }
        }
        assert_eq!(batches, 4);
        assert!(!schema_migration_pending(&connection).unwrap());
        assert_source_totals(&connection);
        assert_eq!(
            metrics.migrations.load(Ordering::Relaxed),
            u64::from(entries)
        );
        drop(connection);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_migration_rolls_back_a_failed_batch_without_losing_old_rows() {
        let dir = temp_dir("schema-rollback");
        let path = dir.join("tiles.sqlite3");
        legacy_schema_fixture(&path, 4, 64);
        initialize_schema(&path).unwrap();
        let mut connection = open_connection(&path, true).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_migration BEFORE INSERT ON source_tiles WHEN NEW.x=1 BEGIN
             SELECT RAISE(ABORT,'injected failure'); END;",
            )
            .unwrap();
        let metrics = StoreMetrics::default();
        assert!(migrate_schema_batch(&mut connection, &mut HashSet::new(), &metrics).is_err());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM source_tiles", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM source_tiles_legacy", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            4
        );
        assert_source_totals(&connection);
        drop(connection);
        initialize_schema(&path).unwrap();
        let mut connection = open_connection(&path, true).unwrap();
        connection
            .execute_batch("DROP TRIGGER reject_migration")
            .unwrap();
        assert!(!migrate_schema_batch(&mut connection, &mut HashSet::new(), &metrics).unwrap());
        assert_source_totals(&connection);
        drop(connection);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_migration_prioritizes_demand_and_bounds_payload_bytes() {
        let dir = temp_dir("schema-byte-bound");
        let path = dir.join("tiles.sqlite3");
        let size = SCHEMA_MIGRATION_BYTES / 2 + 1;
        legacy_schema_fixture(&path, 3, size);
        initialize_schema(&path).unwrap();
        let mut connection = open_connection(&path, true).unwrap();
        let hot = Key {
            namespace_id: 1,
            z: 8,
            x: 2,
            y: 0,
        };
        let mut preferred = HashSet::from([hot]);
        assert!(
            migrate_schema_batch(&mut connection, &mut preferred, &StoreMetrics::default())
                .unwrap()
        );
        let moved: (i64, i64) = connection
            .query_row("SELECT COUNT(*),MAX(x) FROM source_tiles", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(moved, (1, 2));
        assert!(preferred.is_empty());
        assert_source_totals(&connection);
        drop(connection);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_migration_preserves_mutations_pins_eviction_and_revision_cascade() {
        let dir = temp_dir("schema-mutations");
        let path = dir.join("tiles.sqlite3");
        legacy_schema_fixture(&path, 5, 64);
        initialize_schema(&path).unwrap();
        let mut connection = open_connection(&path, true).unwrap();
        connection
            .execute("UPDATE source_tiles_legacy SET pinned=1 WHERE x=1", [])
            .unwrap();
        let key = |x| Key {
            namespace_id: 1,
            z: 8,
            x,
            y: 0,
        };
        let value = |byte| WriteValue {
            state: TILE_PRESENT,
            payload: Some(Arc::from(vec![byte; 64])),
            pinned: false,
        };
        let metrics = StoreMetrics::default();
        flush_batch(
            &mut connection,
            1 << 20,
            HashMap::from([
                (key(0), Mutation::InsertIfAbsent(value(99))),
                (key(1), Mutation::Upsert(value(42))),
                (key(2), Mutation::Remove),
                (key(4), Mutation::Upsert(value(44))),
            ]),
            vec![(vec![key(3)], true)],
            &mut HashSet::from([key(0)]),
            &metrics,
        )
        .unwrap();
        let old: (Vec<u8>, i64) = connection
            .query_row(
                "SELECT payload,last_access FROM source_tile_entries WHERE x=0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(old.0, vec![0; 64]);
        assert!(old.1 > 5);
        let new: (Vec<u8>, bool) = connection
            .query_row(
                "SELECT payload,pinned FROM source_tiles WHERE x=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(new, (vec![42; 64], true));
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM source_tile_entries WHERE x=2",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_source_totals(&connection);
        assert_eq!(
            evict_to_budget(
                &mut connection,
                2 * (64 + ENTRY_BUDGET_BYTES as u64),
                &metrics
            )
            .unwrap(),
            2
        );
        let survivors: Vec<u32> = connection
            .prepare("SELECT x FROM source_tile_entries ORDER BY x")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(survivors, [1, 3]);
        assert!(schema_migration_pending(&connection).unwrap());
        assert_source_totals(&connection);

        // Revision invalidation cascades through both halves while migration
        // is incomplete, so a later batch cannot resurrect an obsolete tile.
        connection
            .execute(
                "DELETE FROM source_namespaces WHERE provider='terrain' AND revision<>'v2'",
                [],
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM source_tile_entries", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_source_totals(&connection);
        assert!(!migrate_schema_batch(&mut connection, &mut HashSet::new(), &metrics).unwrap());
        assert_source_totals(&connection);
        drop(connection);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn incremental_reclamation_drains_the_requested_page_batch() {
        let dir = temp_dir("incremental-reclamation");
        let path = dir.join("tiles.sqlite3");
        initialize_schema(&path).unwrap();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE garbage(payload BLOB)")
            .unwrap();
        connection
            .execute(
                "INSERT INTO garbage VALUES(zeroblob(?1))",
                [(VACUUM_FREELIST_THRESHOLD + 512) * 4096],
            )
            .unwrap();
        connection.execute("DELETE FROM garbage", []).unwrap();
        let free_pages = || {
            connection
                .pragma_query_value(None, "freelist_count", |row| row.get::<_, i64>(0))
                .unwrap()
        };
        let before = free_pages();
        assert!(before >= VACUUM_FREELIST_THRESHOLD);
        incremental_vacuum(&connection).unwrap();
        let reclaimed = before - free_pages();
        assert!(reclaimed > 1, "only {reclaimed} page was reclaimed");
        assert!(reclaimed <= INCREMENTAL_VACUUM_PAGES);
        drop(connection);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reopening_schema_preserves_triggers_and_rebuilds_only_missing_totals() {
        let dir = temp_dir("schema-reopen");
        let path = dir.join("tiles.sqlite3");
        let store = SourceTileStore::open(&path, 8 * 1024 * 1024).unwrap();
        let cache = store.namespace("terrain", "v1", "png", None).unwrap();
        assert!(cache.store_present(1, 0, 0, vec![1, 2, 3]));
        assert!(cache.store_missing(1, 0, 1));
        assert!(cache.flush());
        drop((cache, store));
        let connection = Connection::open(&path).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "schema_version", |row| row.get(0))
            .unwrap();
        let totals = || {
            connection
                .query_row(
                    "SELECT logical_bytes, entry_count FROM source_cache_meta WHERE id=0",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap()
        };
        let expected = (3 + 2 * ENTRY_BUDGET_BYTES as i64, 2);
        assert_eq!(totals(), expected);
        initialize_schema(&path).unwrap();
        assert_eq!(totals(), expected);
        assert_eq!(
            connection
                .pragma_query_value(None, "schema_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            version
        );
        connection
            .execute("DELETE FROM source_cache_meta", [])
            .unwrap();
        initialize_schema(&path).unwrap();
        assert_eq!(totals(), expected);
        connection
            .execute("DELETE FROM source_tiles WHERE y=1", [])
            .unwrap();
        assert_eq!(totals(), (3 + ENTRY_BUDGET_BYTES as i64, 1));
        drop(connection);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn round_trips_payloads_and_known_missing_tiles() {
        let dir = temp_dir("round-trip");
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 8 * 1024 * 1024).unwrap();
        let cache = store
            .namespace(
                "vectors",
                "20260720",
                "application/vnd.mapbox-vector-tile",
                None,
            )
            .unwrap();
        assert_eq!(cache.load(12, 10, 11).unwrap(), None);
        assert!(cache.store_present(12, 10, 11, vec![1, 2, 3]));
        assert!(cache.store_missing(12, 10, 12));
        assert!(cache.flush());
        assert_eq!(
            cache.load(12, 10, 11).unwrap(),
            Some(CachedTile::Present(vec![1, 2, 3]))
        );
        assert_eq!(cache.load(12, 10, 12).unwrap(), Some(CachedTile::Missing));
        let stats = store.stats();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        drop(cache);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn changing_revision_prunes_only_that_provider() {
        let dir = temp_dir("revision");
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 8 * 1024 * 1024).unwrap();
        let old = store.namespace("vectors", "old", "mvt", None).unwrap();
        let terrain = store.namespace("terrain", "v1", "png", None).unwrap();
        assert!(old.store_present(1, 0, 0, vec![1]));
        assert!(terrain.store_present(1, 0, 0, vec![2]));
        assert!(store.flush());
        let new = store.namespace("vectors", "new", "mvt", None).unwrap();
        assert!(store.flush());
        assert_eq!(new.load(1, 0, 0).unwrap(), None);
        assert_eq!(
            terrain.load(1, 0, 0).unwrap(),
            Some(CachedTile::Present(vec![2]))
        );
        drop((old, new, terrain, store));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn budget_evicts_oldest_unpinned_payloads() {
        let dir = temp_dir("budget");
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 4096).unwrap();
        let cache = store.namespace("terrain", "v1", "png", None).unwrap();
        for x in 0..8 {
            assert!(cache.store_present(4, x, 0, vec![x as u8; 1024]));
            assert!(cache.flush());
        }
        let stats = store.stats();
        assert!(stats.logical_bytes <= 4096 * PAYLOAD_BUDGET_PERCENT / 100);
        assert!(stats.evictions > 0);
        drop((cache, store));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn negative_entries_are_charged_to_the_byte_budget() {
        let dir = temp_dir("negative-budget");
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 4096).unwrap();
        let cache = store.namespace("vectors", "v1", "mvt", None).unwrap();
        for x in 0..64 {
            assert!(cache.store_missing(6, x, 0));
        }
        assert!(cache.flush());
        let stats = store.stats();
        assert!(stats.logical_bytes <= 4096 * PAYLOAD_BUDGET_PERCENT / 100);
        assert!(stats.entries < 64);
        assert!(stats.evictions > 0);
        drop((cache, store));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn flush_reports_a_failure_from_an_earlier_writer_batch() {
        let dir = temp_dir("flush-failure");
        // The physical allocation floor is 16 MiB, so this write cannot fit
        // and is processed alone because it exceeds MAX_BATCH_BYTES.
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 1024).unwrap();
        let cache = store.namespace("terrain", "v1", "png", None).unwrap();
        assert!(cache.store_present(1, 0, 0, vec![7; 20 * 1024 * 1024]));
        assert!(!cache.flush());
        assert!(store.stats().write_errors > 0);
        drop((cache, store));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pinned_tiles_survive_budget_eviction() {
        let dir = temp_dir("pinning");
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 4096).unwrap();
        let cache = store.namespace("terrain", "v1", "png", None).unwrap();
        assert!(cache.store_present(4, 0, 0, vec![7; 2048]));
        assert!(cache.flush());
        cache.set_pinned(&[(4, 0, 0)], true);
        assert!(cache.flush());
        for x in 1..8 {
            assert!(cache.store_present(4, x, 0, vec![x as u8; 1024]));
            assert!(cache.flush());
        }
        assert_eq!(
            cache.load(4, 0, 0).unwrap(),
            Some(CachedTile::Present(vec![7; 2048]))
        );
        assert_eq!(store.stats().pinned_entries, 1);
        drop((cache, store));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn migrates_terrarium_files_and_removes_them_after_commit() {
        let dir = temp_dir("terrarium-migration");
        let legacy = dir.join("terrarium");
        let path = legacy.join("3/4/5.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [7, 8, 9]).unwrap();
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 8 * 1024 * 1024).unwrap();
        let cache = store
            .namespace(
                "terrarium",
                "v1",
                "image/png",
                Some(LegacyLayout::Terrarium { root: legacy }),
            )
            .unwrap();
        let report = cache.migrate_legacy();
        assert_eq!(report.imported, 1);
        assert!(!path.exists());
        assert_eq!(
            cache.load(3, 4, 5).unwrap(),
            Some(CachedTile::Present(vec![7, 8, 9]))
        );
        drop((cache, store));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn migration_does_not_overwrite_an_indexed_row() {
        let dir = temp_dir("migration-race");
        let legacy = dir.join("terrarium");
        let path = legacy.join("3/4/5.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [1, 2, 3]).unwrap();
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 8 * 1024 * 1024).unwrap();
        let cache = store
            .namespace(
                "terrarium",
                "v1",
                "image/png",
                Some(LegacyLayout::Terrarium { root: legacy }),
            )
            .unwrap();
        assert!(cache.store_present(3, 4, 5, vec![7, 8, 9]));
        assert_eq!(cache.migrate_legacy().imported, 1);
        assert_eq!(
            cache.load(3, 4, 5).unwrap(),
            Some(CachedTile::Present(vec![7, 8, 9]))
        );
        assert!(!path.exists());
        drop((cache, store));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn conditional_import_does_not_replace_a_pending_upsert() {
        let key = Key {
            namespace_id: 1,
            z: 3,
            x: 4,
            y: 5,
        };
        let mut mutations = HashMap::new();
        coalesce_mutation(
            &mut mutations,
            key,
            Mutation::Upsert(CachedTile::Present(vec![7]).into_write()),
        );
        coalesce_mutation(
            &mut mutations,
            key,
            Mutation::InsertIfAbsent(CachedTile::Present(vec![1]).into_write()),
        );
        let Mutation::Upsert(value) = mutations.remove(&key).unwrap() else {
            panic!("conditional import replaced the pending upsert");
        };
        assert_eq!(value.payload.as_deref(), Some([7].as_slice()));
    }

    #[cfg(unix)]
    #[test]
    fn migration_does_not_follow_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("migration-symlink");
        let legacy = dir.join("terrarium");
        let outside = dir.join("outside");
        let target = outside.join("4/5.png");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(&target, [7, 8, 9]).unwrap();
        symlink(&outside, legacy.join("3")).unwrap();

        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 8 * 1024 * 1024).unwrap();
        let cache = store
            .namespace(
                "terrarium",
                "v1",
                "image/png",
                Some(LegacyLayout::Terrarium { root: legacy }),
            )
            .unwrap();
        assert_eq!(cache.migrate_legacy().imported, 0);
        assert!(target.exists());
        assert_eq!(cache.load(3, 4, 5).unwrap(), None);
        drop((cache, store));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn vector_migration_preserves_negative_entries_and_prunes_old_builds() {
        let dir = temp_dir("vector-migration");
        let root = dir.join("vectors");
        let current = root.join("20260720/4/5_6.mvt");
        let old = root.join("20260719/4/5_6.mvt");
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&current, []).unwrap();
        std::fs::write(&old, [1]).unwrap();
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 8 * 1024 * 1024).unwrap();
        let cache = store
            .namespace(
                "vectors",
                "20260720",
                "mvt",
                Some(LegacyLayout::Vectors {
                    root: root.clone(),
                    build_tag: "20260720".into(),
                }),
            )
            .unwrap();
        let report = cache.migrate_legacy();
        assert_eq!(report.imported, 1);
        assert_eq!(cache.load(4, 5, 6).unwrap(), Some(CachedTile::Missing));
        assert!(!root.join("20260719").exists());
        drop((cache, store));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_readers_observe_committed_tiles_during_writes() {
        let dir = temp_dir("concurrency");
        let store = SourceTileStore::open(dir.join("tiles.sqlite3"), 8 * 1024 * 1024).unwrap();
        let cache = store.namespace("terrain", "v1", "png", None).unwrap();
        for x in 0..32 {
            assert!(cache.store_present(6, x, 0, vec![x as u8; 128]));
        }
        assert!(cache.flush());
        let cache = Arc::new(cache);
        let mut readers = Vec::new();
        for _ in 0..8 {
            let cache = cache.clone();
            readers.push(thread::spawn(move || {
                for _ in 0..64 {
                    for x in 0..32 {
                        assert_eq!(
                            cache.load(6, x, 0).unwrap(),
                            Some(CachedTile::Present(vec![x as u8; 128]))
                        );
                    }
                }
            }));
        }
        for x in 32..64 {
            assert!(cache.store_present(6, x, 0, vec![x as u8; 128]));
        }
        for reader in readers {
            reader.join().unwrap();
        }
        assert!(cache.flush());
        drop(cache);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_database_is_quarantined_and_recreated() {
        let dir = temp_dir("corruption");
        let path = dir.join("tiles.sqlite3");
        std::fs::write(&path, b"not a sqlite database").unwrap();
        let store = SourceTileStore::open(&path, 8 * 1024 * 1024).unwrap();
        assert_eq!(store.stats().corruptions, 1);
        let cache = store.namespace("terrain", "v1", "png", None).unwrap();
        assert!(cache.store_present(1, 0, 0, vec![1, 2, 3]));
        assert!(cache.flush());
        assert_eq!(
            cache.load(1, 0, 0).unwrap(),
            Some(CachedTile::Present(vec![1, 2, 3]))
        );
        assert!(
            std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
        );
        drop((cache, store));
        let _ = std::fs::remove_dir_all(dir);
    }
}
