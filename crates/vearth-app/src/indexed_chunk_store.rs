//! SQLite-backed storage for the native generated-surface cache.
//!
//! The database is only a container and index: values are opaque,
//! compressed surface payloads. Cache keys select the generation and codec
//! namespace; the codec validates content before a record can become resident.
//! Reads use a small lazy connection pool in WAL mode. All mutations go
//! through one bounded queue and one batching thread, keeping SQLite write
//! locks and LRU maintenance off generation and render paths.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use vearth_core::cell::CellId;
use vearth_platform::thread::{self, JoinHandle};
use vearth_platform::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WRITER_QUEUE: usize = 4096;
const MAX_BATCH_COMMANDS: usize = 256;
const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;
const BATCH_DELAY: Duration = Duration::from_millis(12);
const TOUCH_FLUSH_INTERVAL: Duration = Duration::from_secs(30);
const MAX_PENDING_TOUCHES: usize = 8192;
const EVICT_BATCH: usize = 512;
// Leave room for the B-tree, WAL, and SQLite page rounding while interpreting
// the configured limit as a bound on total cache disk use rather than merely
// the sum of payload lengths.
const PAYLOAD_BUDGET_PERCENT: u64 = 88;

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
}

enum Command {
    Write {
        key: Key,
        payload: Arc<[u8]>,
        pin: bool,
    },
    Touch(Key),
    Remove(Key),
    SetPinned(Vec<Key>, bool),
    Flush(mpsc::Sender<()>),
    Shutdown(mpsc::Sender<()>),
}

#[derive(Default)]
struct StoreMetrics {
    writes: AtomicU64,
    evictions: AtomicU64,
    corruptions: AtomicU64,
    write_errors: AtomicU64,
    dropped_writes: AtomicU64,
    logical_bytes: AtomicU64,
    entries: AtomicU64,
    pinned_entries: AtomicU64,
    pinned_bytes: AtomicU64,
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

pub(crate) struct IndexedChunkStore {
    path: PathBuf,
    readers: Vec<Mutex<Option<Connection>>>,
    tx: SyncSender<Command>,
    queued: Arc<AtomicUsize>,
    metrics: Arc<StoreMetrics>,
    budget_bytes: u64,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl IndexedChunkStore {
    pub(crate) fn open(path: PathBuf, budget_bytes: u64) -> rusqlite::Result<Self> {
        open_schema_with_recovery(&path)?;
        let reader_count = vearth_platform::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4)
            .clamp(2, 8);
        let readers = (0..reader_count).map(|_| Mutex::new(None)).collect();
        let (tx, rx) = mpsc::sync_channel(WRITER_QUEUE);
        let queued = Arc::new(AtomicUsize::new(0));
        let metrics = Arc::new(StoreMetrics::default());
        let writer_path = path.clone();
        let writer_metrics = metrics.clone();
        let writer_queued = queued.clone();
        let payload_budget = budget_bytes.saturating_mul(PAYLOAD_BUDGET_PERCENT) / 100;
        let writer = thread::Builder::new()
            .name("chunk-cache-writer".into())
            .spawn(move || {
                if let Err(error) = writer_loop(
                    &writer_path,
                    payload_budget,
                    rx,
                    &writer_queued,
                    &writer_metrics,
                ) {
                    writer_metrics.write_errors.fetch_add(1, Ordering::Relaxed);
                    log::error!("chunk cache: indexed writer stopped: {error}");
                }
            })
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(Self {
            path,
            readers,
            tx,
            queued,
            metrics,
            budget_bytes,
            writer: Mutex::new(Some(writer)),
        })
    }

    pub(crate) fn load(&self, cell: CellId) -> rusqlite::Result<Option<Vec<u8>>> {
        // Keep each worker on a stable connection. This retains its prepared
        // statement and small page cache and avoids round-robin contention.
        let mut hasher = DefaultHasher::new();
        vearth_platform::thread::current().id().hash(&mut hasher);
        let slot = hasher.finish() as usize % self.readers.len();
        let mut reader = self.readers[slot].lock().unwrap();
        if reader.is_none() {
            *reader = Some(open_connection(&self.path, false)?);
        }
        let key = Key::new(cell);
        let sql = "SELECT payload FROM chunks WHERE face=?1 AND level=?2 AND i=?3 AND j=?4";
        let mut statement = reader.as_ref().unwrap().prepare_cached(sql)?;
        let result = statement
            .query_row(params![key.face, key.level, key.i, key.j], |row| row.get(0))
            .optional()?;
        if result.is_some() {
            // This is intentionally a lossy in-memory notification. The
            // writer coalesces touches and persists them at most every 30 s;
            // a hit never performs a SQLite metadata write itself.
            let _ = self.try_send(Command::Touch(key), false);
        }
        Ok(result)
    }

    pub(crate) fn store(&self, cell: CellId, payload: Arc<[u8]>, pin: bool) -> bool {
        self.send_durable(Command::Write {
            key: Key::new(cell),
            payload,
            pin,
        })
    }

    pub(crate) fn remove(&self, cell: CellId) {
        let _ = self.send_durable(Command::Remove(Key::new(cell)));
    }

    pub(crate) fn set_pinned(&self, cells: &[CellId], pinned: bool) {
        for batch in cells.chunks(MAX_BATCH_COMMANDS) {
            let keys = batch.iter().copied().map(Key::new).collect();
            let _ = self.send_durable(Command::SetPinned(keys, pinned));
        }
    }

    pub(crate) fn note_corruption(&self) {
        self.metrics.corruptions.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        if self.send_blocking(Command::Flush(tx)) {
            let _ = rx.recv_timeout(Duration::from_secs(10));
        }
    }

    pub(crate) fn stats(&self) -> IndexedStoreStats {
        let file_bytes = |path: &Path| path.metadata().map_or(0, |meta| meta.len());
        let mut disk_bytes = file_bytes(&self.path);
        disk_bytes = disk_bytes.saturating_add(file_bytes(&PathBuf::from(format!(
            "{}-wal",
            self.path.display()
        ))));
        disk_bytes = disk_bytes.saturating_add(file_bytes(&PathBuf::from(format!(
            "{}-shm",
            self.path.display()
        ))));
        IndexedStoreStats {
            writes: self.metrics.writes.load(Ordering::Relaxed),
            evictions: self.metrics.evictions.load(Ordering::Relaxed),
            corruptions: self.metrics.corruptions.load(Ordering::Relaxed),
            write_errors: self.metrics.write_errors.load(Ordering::Relaxed),
            dropped_writes: self.metrics.dropped_writes.load(Ordering::Relaxed),
            logical_bytes: self.metrics.logical_bytes.load(Ordering::Relaxed),
            entries: self.metrics.entries.load(Ordering::Relaxed),
            pinned_entries: self.metrics.pinned_entries.load(Ordering::Relaxed),
            pinned_bytes: self.metrics.pinned_bytes.load(Ordering::Relaxed),
            disk_bytes,
            budget_bytes: self.budget_bytes,
            queued_commands: self.queued.load(Ordering::Relaxed),
        }
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

impl Drop for IndexedChunkStore {
    fn drop(&mut self) {
        let (tx, rx) = mpsc::channel();
        if self.send_blocking(Command::Shutdown(tx)) {
            let _ = rx.recv_timeout(Duration::from_secs(10));
        }
        if let Some(writer) = self.writer.lock().unwrap().take() {
            let _ = writer.join();
        }
    }
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
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    connection.pragma_update(None, "cache_size", if writer { -8192 } else { -2048 })?;
    connection.pragma_update(None, "wal_autocheckpoint", 1000)?;
    Ok(connection)
}

fn initialize_schema(path: &Path) -> rusqlite::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let new_database = !path.exists() || path.metadata().map_or(true, |meta| meta.len() == 0);
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    if new_database {
        // Keep the established page size; the narrow key index can reject
        // misses without visiting the much larger surface payload records.
        connection.pragma_update(None, "page_size", 2048)?;
        connection.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    }
    connection.pragma_update(None, "journal_mode", "WAL")?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let schema_version: i64 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let migrate_chunks = schema_version < 3
        && transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='chunks')",
            [],
            |row| row.get::<_, bool>(0),
        )?;
    if migrate_chunks {
        transaction.execute_batch(
            "DROP TRIGGER IF EXISTS chunks_insert_meta;
             DROP TRIGGER IF EXISTS chunks_delete_meta;
             DROP TRIGGER IF EXISTS chunks_update_meta;
             DROP INDEX IF EXISTS chunks_lru;
             ALTER TABLE chunks RENAME TO chunks_legacy;",
        )?;
    }
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS chunks (
             face          INTEGER NOT NULL,
             level         INTEGER NOT NULL,
             i             INTEGER NOT NULL,
             j             INTEGER NOT NULL,
             payload_bytes INTEGER NOT NULL CHECK(payload_bytes >= 0),
             last_access   INTEGER NOT NULL,
             pinned        INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
             payload       BLOB NOT NULL,
             PRIMARY KEY(face, level, i, j)
         );
         CREATE INDEX IF NOT EXISTS chunks_lru ON chunks(pinned, last_access, face, level, i, j);
         CREATE TABLE IF NOT EXISTS cache_meta (
             id            INTEGER PRIMARY KEY CHECK(id = 0),
             logical_bytes INTEGER NOT NULL,
             entry_count   INTEGER NOT NULL
         );
         INSERT OR IGNORE INTO cache_meta(id, logical_bytes, entry_count) VALUES(0, 0, 0);
         CREATE TRIGGER IF NOT EXISTS chunks_insert_meta AFTER INSERT ON chunks BEGIN
             UPDATE cache_meta SET logical_bytes=logical_bytes+NEW.payload_bytes,
                                   entry_count=entry_count+1 WHERE id=0;
         END;
         CREATE TRIGGER IF NOT EXISTS chunks_delete_meta AFTER DELETE ON chunks BEGIN
             UPDATE cache_meta SET logical_bytes=logical_bytes-OLD.payload_bytes,
                                   entry_count=entry_count-1 WHERE id=0;
         END;
         CREATE TRIGGER IF NOT EXISTS chunks_update_meta AFTER UPDATE OF payload_bytes ON chunks BEGIN
             UPDATE cache_meta SET logical_bytes=logical_bytes+NEW.payload_bytes-OLD.payload_bytes
             WHERE id=0;
         END;",
        )?;
    if migrate_chunks {
        // Ordinary rowid tables keep composite keys in a narrow UNIQUE index.
        // A WITHOUT ROWID B-tree instead reads complete overflow records even
        // while comparing keys for a miss. Copy opaque payloads inside SQLite
        // so migration memory is bounded by its page cache and current row.
        // Metadata precedes the BLOB to avoid following its overflow chain
        // when accounting, evicting, or inspecting pinned entries.
        transaction.execute_batch(
            "INSERT INTO chunks(face,level,i,j,payload,payload_bytes,last_access,pinned)
                 SELECT face,level,i,j,payload,payload_bytes,last_access,pinned
                   FROM chunks_legacy;
             DROP TABLE chunks_legacy;",
        )?;
    }
    if schema_version < 3 {
        transaction.execute_batch(
            "DROP TRIGGER IF EXISTS render_artifacts_insert_meta;
             DROP TRIGGER IF EXISTS render_artifacts_delete_meta;
             DROP TRIGGER IF EXISTS render_artifacts_update_meta;
             DROP INDEX IF EXISTS render_artifacts_lru;
             DROP TABLE IF EXISTS render_artifacts;
             UPDATE cache_meta
                SET logical_bytes=(SELECT COALESCE(SUM(payload_bytes), 0) FROM chunks),
                    entry_count=(SELECT COUNT(*) FROM chunks)
             WHERE id=0;
             PRAGMA user_version=3;",
        )?;
    }
    transaction.commit()?;
    if schema_version < 3 && !new_database {
        // Reclaim the old tree after the atomic migration. Incremental vacuum
        // moves only pages needed to truncate free space, without rebuilding
        // the live payloads a second time. Older imported databases may not
        // have auto-vacuum enabled and still need a one-time full compaction.
        let reclaim = || -> rusqlite::Result<()> {
            connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
            let auto_vacuum: i64 =
                connection.query_row("PRAGMA auto_vacuum", [], |row| row.get(0))?;
            if auto_vacuum == 2 {
                // This pragma yields one row per reclaimed page. Drain it;
                // execute_batch would step only once and leave most pages.
                let mut statement = connection.prepare("PRAGMA incremental_vacuum")?;
                let mut rows = statement.query([])?;
                while rows.next()?.is_some() {}
            } else {
                connection.execute_batch("VACUUM;")?;
            }
            connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        };
        if let Err(error) = reclaim() {
            if sqlite_corruption(&error) {
                return Err(error);
            }
            // The new schema and records already committed. A busy reader or
            // compaction failure must not disable a valid migrated cache.
            log::warn!("chunk cache: migrated-page reclamation failed: {error}");
        }
    }
    Ok(())
}

fn open_schema_with_recovery(path: &Path) -> rusqlite::Result<()> {
    match initialize_schema(path) {
        Ok(()) => Ok(()),
        Err(error) if sqlite_corruption(&error) => {
            let suffix = now_epoch_seconds();
            log::error!(
                "chunk cache: quarantining corrupt indexed store {}: {error}",
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
            initialize_schema(path)
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
    payload_budget: u64,
    rx: Receiver<Command>,
    queued: &AtomicUsize,
    metrics: &StoreMetrics,
) -> rusqlite::Result<()> {
    let mut connection = open_connection(path, true)?;
    // Complete an eviction interrupted by a process crash before accepting
    // new writes. This is writer-thread startup work, never render work.
    evict_to_budget(&mut connection, payload_budget, metrics)?;
    refresh_metrics(&connection, metrics)?;
    let mut pending_touches = HashSet::new();
    let mut last_touch_flush = Instant::now();
    let mut running = true;

    while running {
        // Empty stores have no maintenance deadline. A command wakes the
        // writer directly; only outstanding touches require a timed wakeup.
        let next = if pending_touches.is_empty() {
            rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(TOUCH_FLUSH_INTERVAL.saturating_sub(last_touch_flush.elapsed()))
        };
        let first = match next {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !pending_touches.is_empty() && last_touch_flush.elapsed() >= TOUCH_FLUSH_INTERVAL
                {
                    flush_batch(
                        &mut connection,
                        payload_budget,
                        HashMap::new(),
                        Vec::new(),
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

        let mut bytes = match &first {
            Command::Write { payload, .. } => payload.len(),
            _ => 0,
        };
        let mut commands = vec![first];
        let batch_deadline = Instant::now() + BATCH_DELAY;
        while commands.len() < MAX_BATCH_COMMANDS && bytes < MAX_BATCH_BYTES {
            let remaining = batch_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(command) => {
                    if let Command::Write { payload, .. } = &command {
                        bytes = bytes.saturating_add(payload.len());
                    }
                    commands.push(command);
                }
                Err(_) => break,
            }
        }
        queued.fetch_sub(commands.len(), Ordering::Relaxed);

        let mut writes = HashMap::new();
        let mut removes = Vec::new();
        let mut pins = Vec::new();
        let mut acknowledgements = Vec::new();
        let mut force_touch_flush = false;
        let mut explicit_checkpoint = false;
        for command in commands {
            match command {
                Command::Write { key, payload, pin } => {
                    writes.insert(key, (payload, pin));
                }
                Command::Touch(key) => {
                    pending_touches.insert(key);
                }
                Command::Remove(key) => removes.push(key),
                Command::SetPinned(keys, pinned) => pins.push((keys, pinned)),
                Command::Flush(reply) => {
                    force_touch_flush = true;
                    explicit_checkpoint = true;
                    acknowledgements.push(reply);
                }
                Command::Shutdown(reply) => {
                    force_touch_flush = true;
                    explicit_checkpoint = true;
                    running = false;
                    acknowledgements.push(reply);
                }
            }
        }
        let incoming_bytes: u64 = writes
            .values()
            .map(|(payload, _)| payload.len() as u64)
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
        if (!writes.is_empty() || !removes.is_empty() || !pins.is_empty() || !touches.is_empty())
            && let Err(error) = flush_batch(
                &mut connection,
                payload_budget,
                writes,
                removes,
                pins,
                touches,
                metrics,
            )
        {
            metrics.write_errors.fetch_add(1, Ordering::Relaxed);
            log::warn!("chunk cache: indexed write batch failed: {error}");
        }
        if touch_due {
            pending_touches.clear();
            last_touch_flush = Instant::now();
        }
        if explicit_checkpoint {
            // Flush/shutdown are explicit non-render boundaries. Truncating
            // here makes the physical byte metric and disk bound include no
            // stale WAL high-water allocation.
            let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
        for reply in acknowledgements {
            let _ = reply.send(());
        }
    }
    // PASSIVE never waits for readers and bounds normal WAL residue without a
    // render-path checkpoint or VACUUM.
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
    Ok(())
}

fn flush_batch(
    connection: &mut Connection,
    payload_budget: u64,
    writes: HashMap<Key, (Arc<[u8]>, bool)>,
    removes: Vec<Key>,
    pins: Vec<(Vec<Key>, bool)>,
    touches: &mut HashSet<Key>,
    metrics: &StoreMetrics,
) -> rusqlite::Result<()> {
    let now = now_epoch_seconds();
    let write_count = writes.len() as u64;
    let transaction = connection.transaction()?;
    if !writes.is_empty() {
        let mut statement = transaction.prepare_cached(
            "INSERT INTO chunks(face,level,i,j,payload,payload_bytes,last_access,pinned)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(face,level,i,j) DO UPDATE SET
                 payload=excluded.payload,
                 payload_bytes=excluded.payload_bytes,
                 last_access=excluded.last_access,
                 pinned=MAX(chunks.pinned, excluded.pinned)",
        )?;
        for (key, (payload, pin)) in writes {
            statement.execute(params![
                key.face,
                key.level,
                key.i,
                key.j,
                payload.as_ref(),
                payload.len() as i64,
                now,
                pin
            ])?;
        }
    }
    if !removes.is_empty() {
        let mut statement = transaction
            .prepare_cached("DELETE FROM chunks WHERE face=?1 AND level=?2 AND i=?3 AND j=?4")?;
        for key in removes {
            statement.execute(params![key.face, key.level, key.i, key.j])?;
        }
    }
    if !pins.is_empty() {
        let mut statement = transaction.prepare_cached(
            "UPDATE chunks SET pinned=?5 WHERE face=?1 AND level=?2 AND i=?3 AND j=?4",
        )?;
        for (keys, pinned) in pins {
            for key in keys {
                statement.execute(params![key.face, key.level, key.i, key.j, pinned])?;
            }
        }
    }
    if !touches.is_empty() {
        let mut statement = transaction.prepare_cached(
            "UPDATE chunks SET last_access=?5 WHERE face=?1 AND level=?2 AND i=?3 AND j=?4",
        )?;
        for key in touches.drain() {
            statement.execute(params![key.face, key.level, key.i, key.j, now])?;
        }
    }
    transaction.commit()?;
    metrics.writes.fetch_add(write_count, Ordering::Relaxed);
    evict_to_budget(connection, payload_budget, metrics)?;
    refresh_metrics(connection, metrics)
}

fn evict_to_budget(
    connection: &mut Connection,
    payload_budget: u64,
    metrics: &StoreMetrics,
) -> rusqlite::Result<()> {
    let mut logical_bytes = connection.query_row(
        "SELECT logical_bytes FROM cache_meta WHERE id=0",
        [],
        |row| row.get::<_, i64>(0),
    )? as u64;
    while logical_bytes > payload_budget {
        let bytes_needed = logical_bytes - payload_budget;
        let transaction = connection.transaction()?;
        let mut victims = Vec::new();
        let mut reclaimed = 0u64;
        {
            let mut statement = transaction.prepare(
                "SELECT face,level,i,j,payload_bytes FROM chunks
                 WHERE pinned=0 ORDER BY last_access ASC, face, level, i, j LIMIT ?1",
            )?;
            let rows = statement.query_map([EVICT_BATCH as i64], |row| {
                Ok((
                    Key {
                        face: row.get(0)?,
                        level: row.get(1)?,
                        i: row.get(2)?,
                        j: row.get(3)?,
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
        {
            let mut statement = transaction.prepare_cached(
                "DELETE FROM chunks WHERE face=?1 AND level=?2 AND i=?3 AND j=?4",
            )?;
            for key in &victims {
                statement.execute(params![key.face, key.level, key.i, key.j])?;
            }
        }
        let removed = victims.len();
        transaction.commit()?;
        if removed == 0 {
            log::warn!(
                "chunk cache: pinned payload ({logical_bytes} bytes) exceeds disk budget ({payload_budget} bytes)"
            );
            break;
        }
        metrics
            .evictions
            .fetch_add(removed as u64, Ordering::Relaxed);
        logical_bytes = connection.query_row(
            "SELECT logical_bytes FROM cache_meta WHERE id=0",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64;
    }
    Ok(())
}

fn refresh_metrics(connection: &Connection, metrics: &StoreMetrics) -> rusqlite::Result<()> {
    let (logical_bytes, entries): (i64, i64) = connection.query_row(
        "SELECT logical_bytes, entry_count FROM cache_meta WHERE id=0",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let (pinned_entries, pinned_bytes): (i64, i64) = connection.query_row(
        "SELECT COUNT(*), COALESCE(SUM(payload_bytes), 0) FROM chunks WHERE pinned=1",
        [],
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

    #[test]
    fn rowid_migration_preserves_payloads_metadata_and_key_only_lookup() {
        let dir = std::env::temp_dir().join(format!(
            "vearth-indexed-cache-rowid-migration-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chunks.sqlite3");
        let payload: Vec<u8> = (0..524_288).map(|i| (i % 251) as u8).collect();
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "PRAGMA page_size=2048;
                     PRAGMA auto_vacuum=INCREMENTAL;
                     CREATE TABLE chunks (
                         face INTEGER NOT NULL, level INTEGER NOT NULL,
                         i INTEGER NOT NULL, j INTEGER NOT NULL,
                         payload BLOB NOT NULL, payload_bytes INTEGER NOT NULL,
                         last_access INTEGER NOT NULL, pinned INTEGER NOT NULL,
                         PRIMARY KEY(face, level, i, j)
                     ) WITHOUT ROWID;
                     CREATE INDEX chunks_lru ON chunks(pinned,last_access);
                     CREATE TABLE cache_meta (
                         id INTEGER PRIMARY KEY, logical_bytes INTEGER NOT NULL,
                         entry_count INTEGER NOT NULL
                     );
                     INSERT INTO cache_meta VALUES(0, 99, 99);
                     INSERT INTO chunks VALUES(0, 14, 11, 22, X'112233', 3, 123, 0);
                     PRAGMA user_version=2;",
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO chunks VALUES(1, 15, 33, 44, ?1, ?2, 456, 1)",
                    params![payload, payload.len() as i64],
                )
                .unwrap();
        }

        initialize_schema(&path).unwrap();
        initialize_schema(&path).unwrap();

        let connection = Connection::open(&path).unwrap();
        let migrated: (Vec<u8>, i64, i64, bool) = connection
            .query_row(
                "SELECT payload,payload_bytes,last_access,pinned FROM chunks
                 WHERE face=1 AND level=15 AND i=33 AND j=44",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(migrated, (payload.clone(), payload.len() as i64, 456, true));
        let plan: String = connection
            .query_row(
                "EXPLAIN QUERY PLAN SELECT payload FROM chunks
                 WHERE face=1 AND level=15 AND i=33 AND j=45",
                [],
                |row| row.get(3),
            )
            .unwrap();
        assert!(
            plan.contains("USING INDEX sqlite_autoindex_chunks_1"),
            "{plan}"
        );
        let indexed_columns: Vec<String> = connection
            .prepare("PRAGMA index_info(sqlite_autoindex_chunks_1)")
            .unwrap()
            .query_map([], |row| row.get(2))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(indexed_columns, ["face", "level", "i", "j"]);
        let columns: Vec<String> = connection
            .prepare("PRAGMA table_info(chunks)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(columns.last().map(String::as_str), Some("payload"));
        let rowids: i64 = connection
            .query_row("SELECT COUNT(rowid) FROM chunks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rowids, 2);
        drop(connection);

        let store = IndexedChunkStore::open(path.clone(), 8 * 1024 * 1024).unwrap();
        store.flush();
        let stats = store.stats();
        assert_eq!(
            (stats.entries, stats.logical_bytes),
            (2, payload.len() as u64 + 3)
        );
        assert_eq!(
            (stats.pinned_entries, stats.pinned_bytes),
            (1, payload.len() as u64)
        );
        let cell = CellId {
            face: vearth_core::cubesphere::Face(1),
            level: 15,
            i: 33,
            j: 44,
        };
        assert_eq!(store.load(cell).unwrap(), Some(payload));
        assert_eq!(store.load(CellId { j: 45, ..cell }).unwrap(), None);
        assert!(store.store(cell, Arc::from([9u8, 8, 7, 6]), false));
        store.flush();
        let stats = store.stats();
        assert_eq!((stats.entries, stats.logical_bytes), (2, 7));
        assert_eq!((stats.pinned_entries, stats.pinned_bytes), (1, 4));
        store.remove(cell);
        store.flush();
        let stats = store.stats();
        assert_eq!((stats.entries, stats.logical_bytes), (1, 3));
        assert_eq!((stats.pinned_entries, stats.pinned_bytes), (0, 0));
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn equal_age_eviction_uses_cell_order_instead_of_insertion_order() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE chunks (
                     face INTEGER NOT NULL, level INTEGER NOT NULL,
                     i INTEGER NOT NULL, j INTEGER NOT NULL,
                     payload BLOB NOT NULL, payload_bytes INTEGER NOT NULL,
                     last_access INTEGER NOT NULL, pinned INTEGER NOT NULL,
                     PRIMARY KEY(face, level, i, j)
                 );
                 CREATE INDEX chunks_lru ON chunks(pinned,last_access,face,level,i,j);
                 CREATE TABLE cache_meta(id INTEGER PRIMARY KEY,logical_bytes INTEGER,entry_count INTEGER);
                 INSERT INTO cache_meta VALUES(0,6,3);
                 CREATE TRIGGER chunks_delete_meta AFTER DELETE ON chunks BEGIN
                     UPDATE cache_meta SET logical_bytes=logical_bytes-OLD.payload_bytes,
                                           entry_count=entry_count-1 WHERE id=0;
                 END;
                 INSERT INTO chunks VALUES(0,14,3,0,X'1234',2,100,0);
                 INSERT INTO chunks VALUES(0,14,1,0,X'1234',2,100,0);
                 INSERT INTO chunks VALUES(0,14,2,0,X'1234',2,100,0);",
            )
            .unwrap();
        evict_to_budget(&mut connection, 4, &StoreMetrics::default()).unwrap();
        let remaining: Vec<i64> = connection
            .prepare("SELECT i FROM chunks ORDER BY i")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(remaining, [2, 3]);
    }

    #[test]
    fn opening_old_database_removes_render_table_and_repairs_totals() {
        let dir = std::env::temp_dir().join(format!(
            "vearth-indexed-cache-render-table-migration-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chunks.sqlite3");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "PRAGMA page_size=2048;
                     PRAGMA auto_vacuum=INCREMENTAL;
                     CREATE TABLE chunks (
                         face INTEGER NOT NULL, level INTEGER NOT NULL,
                         i INTEGER NOT NULL, j INTEGER NOT NULL,
                         payload BLOB NOT NULL, payload_bytes INTEGER NOT NULL,
                         last_access INTEGER NOT NULL, pinned INTEGER NOT NULL,
                         PRIMARY KEY(face, level, i, j)
                     ) WITHOUT ROWID;
                     CREATE TABLE render_artifacts (
                         face INTEGER NOT NULL, level INTEGER NOT NULL,
                         i INTEGER NOT NULL, j INTEGER NOT NULL,
                         payload BLOB NOT NULL, payload_bytes INTEGER NOT NULL,
                         last_access INTEGER NOT NULL, pinned INTEGER NOT NULL,
                         PRIMARY KEY(face, level, i, j)
                     ) WITHOUT ROWID;
                     CREATE TABLE cache_meta (
                         id INTEGER PRIMARY KEY, logical_bytes INTEGER NOT NULL,
                         entry_count INTEGER NOT NULL
                     );
                     INSERT INTO chunks VALUES(0, 8, 1, 2, X'010203', 3, 1, 0);
                     INSERT INTO render_artifacts
                         VALUES(0, 8, 1, 2, zeroblob(1048576), 1048576, 1, 0);
                     INSERT INTO cache_meta VALUES(0, 1048579, 2);
                     PRAGMA user_version=1;",
                )
                .unwrap();
        }
        let before_bytes = path.metadata().unwrap().len();

        initialize_schema(&path).unwrap();

        let connection = Connection::open(&path).unwrap();
        let render_table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='render_artifacts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let totals: (i64, i64) = connection
            .query_row(
                "SELECT logical_bytes, entry_count FROM cache_meta WHERE id=0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let schema_version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        drop(connection);

        assert_eq!(render_table_count, 0);
        assert_eq!(totals, (3, 1));
        assert_eq!(schema_version, 3);
        assert!(
            path.metadata().unwrap().len() < before_bytes,
            "migration should return obsolete render pages to the filesystem"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
