//! Persistent continuous-surface cache. Cache hits restore final geometry and
//! collision directly; workers only pack GPU buffers. The VXS4 schema and
//! surfaces namespace deliberately reject every voxel-era payload.

use crate::indexed_chunk_store::IndexedChunkStore;
use lru::LruCache;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use vearth_core::cell::{CellId, ChunkAddr};
use vearth_core::mesh::{MeshBuffers, SurfaceVertex};
use vearth_gen::built_surface::BuiltDetailBounds;
use vearth_gen::collision::{Capsule, CollisionTriangle, SurfaceCollision};
use vearth_gen::flora::{
    CultivationCalendar, FLORA_COVERAGE_WORDS, FLORA_GROUND_SAMPLES, FloraArchetype, FloraCell,
};
use vearth_gen::tier0::TreeInstance;
use vearth_gen::vegetation::TREE_PROFILES;
use vearth_gen::{
    NIGHT_LIGHT_BUCKETS, NightLightCluster, RoadGuide, SurfaceBatch, SurfaceCell, TerrainCollision,
};

pub const SURFACE_CODEC_VERSION: u32 = 4;
const MAGIC: &[u8; 4] = b"VXS4";
#[cfg(not(target_arch = "wasm32"))]
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
const MAX_DECOMPRESSED_BYTES: usize = 128 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const MAX_RETAINED_CODEC_BYTES: usize = 2 * 1024 * 1024;
const MAX_ROAD_GUIDES_PER_CELL: usize = 2_048;
pub const FLORA_DESCRIPTOR_CACHE_BYTES: usize =
    1 + 1 + 8 + 4 + 1 + 1 + FLORA_COVERAGE_WORDS * 4 + 4 + FLORA_GROUND_SAMPLES * 2 + 1 + 1;
const MAX_FLORA_DESCRIPTORS_PER_CELL: usize = 256;
const MAX_VERTICES: usize = 4_194_304;
const MAX_INDICES: usize = 3 * MAX_VERTICES;
const MAX_BATCHES: usize = 16_384;
use vearth_gen::collision::{MAX_COLLISION_CAPSULES as MAX_CAPSULES, MAX_COLLISION_TRIANGLES};
/// Default budget for the compressed CPU-resident tier.
///
/// Benchmarks may override it without changing persistent cache keys.
const DEFAULT_RAM_BUDGET: usize = 256 * 1024 * 1024;
/// After crossing the RAM budget, batch eviction leaves this much headroom so
/// the next few insertions do not immediately pay eviction overhead again.
const RAM_EVICTION_HEADROOM_DIVISOR: usize = 20;
/// Total on-disk allocation target. The indexed store reserves a small slice
/// for SQLite's B-tree and WAL, so payload eviction begins below this value.
const DEFAULT_DISK_BUDGET: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChunkCacheStats {
    pub ram_hits: u64,
    pub disk_hits: u64,
    pub misses: u64,
    pub resident_entries: usize,
    pub resident_bytes: usize,
    /// Descriptor records represented by the compressed RAM tier.
    pub resident_flora_cells: usize,
    /// Logical uncompressed descriptor bytes; already included (compressed)
    /// in `resident_bytes`, so these values must not be added together.
    pub resident_flora_bytes: usize,
    pub budget_bytes: usize,
    pub indexed_hits: u64,
    pub legacy_hits: u64,
    pub legacy_imports: u64,
    pub writes: u64,
    pub evictions: u64,
    pub corruptions: u64,
    pub write_errors: u64,
    pub dropped_writes: u64,
    pub disk_entries: u64,
    pub logical_disk_bytes: u64,
    pub physical_disk_bytes: u64,
    pub disk_budget_bytes: u64,
    pub pinned_entries: u64,
    pub pinned_bytes: u64,
    pub queued_writes: usize,
}

struct RamEntry {
    encoded: Arc<[u8]>,
    flora_cells: usize,
}

struct RamTier {
    columns: LruCache<CellId, RamEntry>,
    bytes: usize,
    column_bytes: usize,
    flora_cells: usize,
    budget: usize,
}

impl RamTier {
    fn new(budget: usize) -> Self {
        RamTier {
            // Capacity is byte-based rather than entry-based, so eviction is
            // enforced below after each insertion.
            columns: LruCache::unbounded(),
            bytes: 0,
            column_bytes: 0,
            flora_cells: 0,
            budget,
        }
    }

    fn get_column(&mut self, cell: CellId) -> Option<Arc<[u8]>> {
        self.columns.get(&cell).map(|entry| entry.encoded.clone())
    }

    fn remove_column(&mut self, cell: CellId) {
        if let Some(entry) = self.columns.pop(&cell) {
            self.bytes = self.bytes.saturating_sub(entry.encoded.len());
            self.column_bytes = self.column_bytes.saturating_sub(entry.encoded.len());
            self.flora_cells = self.flora_cells.saturating_sub(entry.flora_cells);
        }
    }

    fn insert_column(&mut self, cell: CellId, encoded: Arc<[u8]>, flora_cells: usize) {
        self.remove_column(cell);
        if self.budget == 0 || encoded.len() > self.budget {
            return;
        }
        self.bytes += encoded.len();
        self.column_bytes += encoded.len();
        self.flora_cells += flora_cells;
        self.columns.put(
            cell,
            RamEntry {
                encoded,
                flora_cells,
            },
        );

        self.evict_to_budget();
    }

    fn evict_to_budget(&mut self) {
        if self.bytes <= self.budget {
            return;
        }
        let low_water = self
            .budget
            .saturating_sub(self.budget / RAM_EVICTION_HEADROOM_DIVISOR);
        while self.bytes > low_water && self.columns.len() > 1 {
            let Some((_cell, entry)) = self.columns.pop_lru() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(entry.encoded.len());
            self.column_bytes = self.column_bytes.saturating_sub(entry.encoded.len());
            self.flora_cells = self.flora_cells.saturating_sub(entry.flora_cells);
        }
    }
}

pub struct ChunkCache {
    root: PathBuf,
    dataset_tag: Arc<str>,
    indexed: Option<IndexedChunkStore>,
    ram: Mutex<RamTier>,
    ram_hits: AtomicU64,
    disk_hits: AtomicU64,
    indexed_hits: AtomicU64,
    legacy_hits: AtomicU64,
    legacy_imports: AtomicU64,
    misses: AtomicU64,
}

impl ChunkCache {
    /// `dataset_tag` names physical input data (including the terrain-only
    /// vector contract and default provider profile); GEN_VERSION is baked
    /// into the directory as well. Stale caches are never read again and are
    /// pruned on startup, since every version or dataset bump would otherwise
    /// permanently leak a multi-GB directory.
    pub fn new(cache_root: impl AsRef<Path>, dataset_tag: &str) -> Self {
        Self::with_configured_disk_budget(cache_root, dataset_tag, DEFAULT_DISK_BUDGET)
    }

    /// Construct with a user-selected on-disk budget. The environment
    /// override remains authoritative for benchmark and diagnostic runs.
    pub fn with_configured_disk_budget(
        cache_root: impl AsRef<Path>,
        dataset_tag: &str,
        configured_disk_budget: u64,
    ) -> Self {
        let ram_budget = vearth_platform::env::var("VEARTH_CHUNK_RAM_MB")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .map(|mb| mb.saturating_mul(1024 * 1024))
            .unwrap_or(DEFAULT_RAM_BUDGET);
        let disk_budget = vearth_platform::env::var("VEARTH_CHUNK_DISK_GB")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|gb| (gb * 1024.0 * 1024.0 * 1024.0) as u64)
            .unwrap_or(configured_disk_budget);
        Self::with_budgets(cache_root, dataset_tag, ram_budget, disk_budget)
    }

    /// Construct with an explicit compressed-RAM budget. Public primarily so
    /// deterministic tests and trace sweeps can exercise very small budgets.
    pub fn with_ram_budget(
        cache_root: impl AsRef<Path>,
        dataset_tag: &str,
        ram_budget: usize,
    ) -> Self {
        Self::with_budgets(cache_root, dataset_tag, ram_budget, DEFAULT_DISK_BUDGET)
    }

    /// Construct with explicit compressed-RAM and total-disk budgets.
    pub fn with_budgets(
        cache_root: impl AsRef<Path>,
        dataset_tag: &str,
        ram_budget: usize,
        disk_budget: u64,
    ) -> Self {
        let tag = format!(
            "g{}-s{}-{dataset_tag}",
            vearth_gen::GEN_VERSION,
            SURFACE_CODEC_VERSION
        );
        let chunks_dir = cache_root.as_ref().join("surfaces");
        prune_stale(&chunks_dir, &tag);
        let root = chunks_dir.join(tag);
        let indexed_path = root.join("surfaces.sqlite3");
        let use_indexed =
            vearth_platform::env::var("VEARTH_CHUNK_STORE").map_or(true, |value| value != "files");
        let indexed = match use_indexed
            .then(|| IndexedChunkStore::open(indexed_path, disk_budget))
            .transpose()
        {
            Ok(store) => store,
            Err(error) => {
                // Keep the surface-file cache operational if SQLite cannot be
                // opened (read-only disk, unsupported filesystem, etc.).
                log::error!("chunk cache: indexed store unavailable: {error}");
                None
            }
        };
        ChunkCache {
            root,
            dataset_tag: Arc::from(dataset_tag),
            indexed,
            ram: Mutex::new(RamTier::new(ram_budget)),
            ram_hits: AtomicU64::new(0),
            disk_hits: AtomicU64::new(0),
            indexed_hits: AtomicU64::new(0),
            legacy_hits: AtomicU64::new(0),
            legacy_imports: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub fn dataset_tag(&self) -> &str {
        &self.dataset_tag
    }

    fn path(&self, cell: CellId) -> PathBuf {
        self.root
            .join(format!("f{}", cell.face.0))
            .join(format!("l{}", cell.level))
            .join(format!("{}_{}.vxs", cell.i, cell.j))
    }

    pub fn load(&self, cell: CellId) -> Option<SurfaceCell> {
        let ram_bytes = self.ram.lock().unwrap().get_column(cell);
        if let Some(bytes) = ram_bytes {
            if let Some(column) = decode(cell, &bytes) {
                self.ram_hits.fetch_add(1, Ordering::Relaxed);
                return Some(column);
            }
            // Defensive: a corrupt memory entry must not mask a valid disk
            // entry. This should be unreachable because only encode() inserts.
            self.ram.lock().unwrap().remove_column(cell);
        }

        if let Some(indexed) = &self.indexed {
            match indexed.load(cell) {
                Ok(Some(bytes)) => match decode(cell, &bytes) {
                    Some(column) => {
                        self.disk_hits.fetch_add(1, Ordering::Relaxed);
                        self.indexed_hits.fetch_add(1, Ordering::Relaxed);
                        let flora_cells = column.flora.len();
                        let bytes: Arc<[u8]> = Arc::from(bytes);
                        self.ram
                            .lock()
                            .unwrap()
                            .insert_column(cell, bytes, flora_cells);
                        return Some(column);
                    }
                    None => {
                        log::warn!(
                            "chunk cache: corrupt indexed entry for {cell:?}; trying legacy fallback"
                        );
                        indexed.note_corruption();
                        indexed.remove(cell);
                    }
                },
                Ok(None) => {}
                Err(error) => {
                    // A transient busy/I/O failure must degrade to the old
                    // file and then a cold miss, never stall generation.
                    log::warn!("chunk cache: indexed read failed for {cell:?}: {error}");
                }
            }
        }

        let bytes = match std::fs::read(self.path(cell)) {
            Ok(bytes) => bytes,
            Err(_) => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        match decode(cell, &bytes) {
            Some(col) => {
                self.disk_hits.fetch_add(1, Ordering::Relaxed);
                self.legacy_hits.fetch_add(1, Ordering::Relaxed);
                let flora_cells = col.flora.len();
                let bytes: Arc<[u8]> = Arc::from(bytes);
                self.ram
                    .lock()
                    .unwrap()
                    .insert_column(cell, bytes.clone(), flora_cells);
                if let Some(indexed) = &self.indexed
                    && indexed.store(cell, bytes, false)
                {
                    self.legacy_imports.fetch_add(1, Ordering::Relaxed);
                }
                Some(col)
            }
            None => {
                log::warn!("chunk cache: corrupt entry for {cell:?}; regenerating");
                let _ = std::fs::remove_file(self.path(cell));
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    pub fn store(&self, col: &SurfaceCell) {
        self.store_with_pin(col, false);
    }

    /// Store and protect a generated column for an offline area. Pinned bytes
    /// may exceed the ordinary budget; they are never selected by LRU.
    pub fn store_pinned(&self, col: &SurfaceCell) {
        self.store_with_pin(col, true);
    }

    fn store_with_pin(&self, col: &SurfaceCell, pinned: bool) {
        if !col.complete {
            return;
        }
        let Some(bytes) = encode(col) else {
            // Runtime visual admission need not fit a persistent codec page.
            // Never write an entry that the bounded decoder would discard.
            return;
        };
        let bytes: Arc<[u8]> = Arc::from(bytes);
        self.ram
            .lock()
            .unwrap()
            .insert_column(col.cell, bytes.clone(), col.flora.len());
        if let Some(indexed) = &self.indexed {
            if !indexed.store(col.cell, bytes.clone(), pinned) {
                self.write_legacy(col.cell, &bytes);
            }
        } else {
            self.write_legacy(col.cell, &bytes);
        }
    }

    /// Change eviction protection for already-cached cells. Offline download
    /// paths should use `store_pinned` for new cells and this method when an
    /// area is removed or promoted after download.
    pub fn set_pinned(&self, cells: &[CellId], pinned: bool) {
        if let Some(indexed) = &self.indexed {
            indexed.set_pinned(cells, pinned);
        }
    }

    fn write_legacy(&self, cell: CellId, bytes: &[u8]) {
        let path = self.path(cell);
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        // Atomic-ish: write then rename, so readers never see partials.
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    pub fn stats(&self) -> ChunkCacheStats {
        let disk = self
            .indexed
            .as_ref()
            .map(IndexedChunkStore::stats)
            .unwrap_or_default();
        let ram = self.ram.lock().unwrap();
        ChunkCacheStats {
            ram_hits: self.ram_hits.load(Ordering::Relaxed),
            disk_hits: self.disk_hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            resident_entries: ram.columns.len(),
            resident_bytes: ram.column_bytes,
            resident_flora_cells: ram.flora_cells,
            resident_flora_bytes: ram.flora_cells * FLORA_DESCRIPTOR_CACHE_BYTES,
            budget_bytes: ram.budget,
            indexed_hits: self.indexed_hits.load(Ordering::Relaxed),
            legacy_hits: self.legacy_hits.load(Ordering::Relaxed),
            legacy_imports: self.legacy_imports.load(Ordering::Relaxed),
            writes: disk.writes,
            evictions: disk.evictions,
            corruptions: disk.corruptions,
            write_errors: disk.write_errors,
            dropped_writes: disk.dropped_writes,
            disk_entries: disk.entries,
            logical_disk_bytes: disk.logical_bytes,
            physical_disk_bytes: disk.disk_bytes,
            disk_budget_bytes: disk.budget_bytes,
            pinned_entries: disk.pinned_entries,
            pinned_bytes: disk.pinned_bytes,
            queued_writes: disk.queued_commands,
        }
    }

    /// Wait until queued writes and coalesced access touches are durable.
    /// Normal rendering never calls this; it is intended for shutdown-like
    /// offline download boundaries and deterministic benchmarks.
    pub fn flush(&self) {
        if let Some(indexed) = &self.indexed {
            indexed.flush();
        }
    }
}

/// Remove sibling cache directories left by other GEN_VERSIONs or dataset
/// tags: cache keys embed both, so those entries can never be read again.
/// Deletes only names matching the cache's own `g<digits>-<tag>` pattern —
/// anything else in the directory is not ours to touch. A concurrently
/// running instance is safe: it uses the same current tag, which is never
/// pruned.
fn prune_stale(chunks_dir: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(chunks_dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let ours = name
            .strip_prefix('g')
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_ascii_digit())
            && name.contains('-');
        if ours && name != keep && e.path().is_dir() {
            match std::fs::remove_dir_all(e.path()) {
                Ok(()) => log::info!("chunk cache: pruned stale {name}"),
                Err(err) => log::warn!("chunk cache: could not prune {name}: {err}"),
            }
        }
    }
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn encoded_size(col: &SurfaceCell) -> Option<usize> {
    if !col.terrain_relief_m.is_finite() || col.terrain_relief_m < 0.0 {
        return None;
    }
    let counts = [
        (col.road_guides.len(), MAX_ROAD_GUIDES_PER_CELL, 40),
        (col.night_lights.len(), NIGHT_LIGHT_BUCKETS, 20),
        (col.tree_instances.len(), 1024, 45),
        (
            col.flora.len(),
            MAX_FLORA_DESCRIPTORS_PER_CELL,
            FLORA_DESCRIPTOR_CACHE_BYTES,
        ),
        (
            col.solid_collision.triangles().len(),
            MAX_COLLISION_TRIANGLES,
            36,
        ),
        (col.solid_collision.capsules().len(), MAX_CAPSULES, 32),
        (col.water.len(), MAX_COLLISION_TRIANGLES, 41),
    ];
    if col.batches.len() > MAX_BATCHES {
        return None;
    }
    // Fixed header, record counts and the two terrain-presence bytes.
    let mut bytes = 55_usize;
    if let Some(detail) = &col.terrain_detail {
        detail.validate().ok()?;
        bytes = bytes.checked_add((33 * 33 + 32) * 4)?;
    }
    for (count, limit, stride) in counts {
        if count > limit {
            return None;
        }
        bytes = bytes.checked_add(count.checked_mul(stride)?)?;
    }
    let (mut vertices, mut indices) = (0_usize, 0_usize);
    for batch in &col.batches {
        if !batch.validate_shadow_omissions() {
            return None;
        }
        vertices = vertices.checked_add(batch.mesh.vertices.len())?;
        indices = indices.checked_add(batch.mesh.indices.len())?;
        bytes = bytes.checked_add(17 + usize::from(batch.detail.is_some()) * 20)?;
        bytes = bytes.checked_add(batch.shadow_omissions.len().checked_mul(12)?)?;
    }
    if vertices > MAX_VERTICES || indices > MAX_INDICES {
        return None;
    }
    bytes = bytes.checked_add(vertices.checked_mul(20)?)?;
    bytes = bytes.checked_add(indices.checked_mul(4)?)?;
    if col.terrain_collision.is_some() {
        bytes = bytes.checked_add((33 * 33 + 32) * 4)?;
    }
    (bytes <= MAX_DECOMPRESSED_BYTES).then_some(bytes)
}

fn encode(col: &SurfaceCell) -> Option<Vec<u8>> {
    let size = encoded_size(col)?;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(MAGIC);
    out.push(u8::from(col.complete));
    push_f32(&mut out, col.height_range.0);
    push_f32(&mut out, col.height_range.1);
    push_f32(&mut out, col.terrain_step_m);
    push_f32(&mut out, col.terrain_relief_m);
    push_u32(&mut out, col.road_guides.len() as u32);
    for guide in &col.road_guides {
        for value in [
            guide.a_local[0],
            guide.a_local[1],
            guide.b_local[0],
            guide.b_local[1],
            guide.a_exit[0],
            guide.a_exit[1],
            guide.b_exit[0],
            guide.b_exit[1],
            guide.width_m,
            guide.assist,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    push_u32(&mut out, col.night_lights.len() as u32);
    for light in &col.night_lights {
        for value in light.local_pos {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&light.energy.to_le_bytes());
        out.extend_from_slice(&light.color);
        out.push(light.bucket);
    }
    push_u32(&mut out, col.tree_instances.len() as u32);
    for tree in &col.tree_instances {
        out.push(tree.profile as u8);
        out.extend_from_slice(&tree.seed.to_le_bytes());
        out.extend_from_slice(&tree.lat_rad.to_le_bytes());
        out.extend_from_slice(&tree.lon_rad.to_le_bytes());
        for value in [
            tree.elevation_m,
            tree.height_m,
            tree.yaw_rad,
            tree.cell_x,
            tree.cell_z,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    push_u32(&mut out, col.flora.len() as u32);
    for flora in &col.flora {
        out.push(flora.archetype as u8);
        out.push(flora.calendar as u8);
        out.extend_from_slice(&flora.field_seed.to_le_bytes());
        out.extend_from_slice(&flora.row_heading_rad.to_le_bytes());
        out.push(flora.palette);
        out.push(flora.base_density);
        for word in flora.coverage {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&flora.ground_base_cm.to_le_bytes());
        for offset in flora.ground_cm {
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.push(flora.phenology_offset);
        out.push(flora.curated_style);
    }

    push_u32(&mut out, col.batches.len() as u32);
    for batch in &col.batches {
        push_u32(&mut out, batch.addr.k as u32);
        out.push(u8::from(batch.detail.is_some()));
        if let Some(detail) = batch.detail {
            for value in detail
                .center
                .into_iter()
                .chain([detail.radius_m, detail.min_screen_px])
            {
                push_f32(&mut out, value);
            }
        }
        encode_mesh(&mut out, &batch.mesh);
        push_u32(&mut out, batch.shadow_omissions.len() as u32);
        for span in &batch.shadow_omissions {
            push_u32(&mut out, span.first_index);
            push_u32(&mut out, span.index_count);
            push_f32(&mut out, span.max_deviation_m);
        }
    }
    out.push(u8::from(col.terrain_collision.is_some()));
    if let Some(terrain) = &col.terrain_collision {
        for &height in terrain.heights_local.iter() {
            push_f32(&mut out, height);
        }
        for mask in terrain.source_columns {
            push_u32(&mut out, mask);
        }
    }
    push_u32(&mut out, col.solid_collision.triangles().len() as u32);
    for triangle in col.solid_collision.triangles() {
        for value in triangle.vertices.into_iter().flatten() {
            push_f32(&mut out, value);
        }
    }
    push_u32(&mut out, col.solid_collision.capsules().len() as u32);
    for capsule in col.solid_collision.capsules() {
        for value in capsule
            .a
            .into_iter()
            .chain(capsule.b)
            .chain([capsule.radius_a, capsule.radius_b])
        {
            push_f32(&mut out, value);
        }
    }
    push_u32(&mut out, col.water.len() as u32);
    for triangle in &col.water {
        for value in triangle.vertices.into_iter().flatten() {
            push_f32(&mut out, value);
        }
        out.push(triangle.kind as u8);
        push_f32(&mut out, triangle.depth_m);
    }
    out.push(u8::from(col.terrain_detail.is_some()));
    if let Some(detail) = &col.terrain_detail {
        for &height in detail.heights_m.iter() {
            push_f32(&mut out, height);
        }
        for mask in detail.source_columns {
            push_u32(&mut out, mask);
        }
    }
    debug_assert_eq!(out.len(), size);
    #[cfg(not(target_arch = "wasm32"))]
    return encode_zstd(&out);
    // The browser has no C zstd; gzip frames are part of the same codec.
    #[cfg(target_arch = "wasm32")]
    return encode_gzip(&out);
}

#[cfg(target_arch = "wasm32")]
fn encode_gzip(input: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write as _;
    let mut encoder = flate2::write::GzEncoder::new(
        Vec::with_capacity(input.len() / 3),
        flate2::Compression::fast(),
    );
    encoder.write_all(input).ok()?;
    encoder.finish().ok()
}

fn encode_mesh(out: &mut Vec<u8>, mesh: &MeshBuffers) {
    push_u32(out, mesh.vertices.len() as u32);
    push_u32(out, mesh.indices.len() as u32);
    for vertex in &mesh.vertices {
        for value in vertex.pos {
            out.extend_from_slice(&value.to_le_bytes());
        }
        push_u32(out, vertex.face_ao);
        out.extend_from_slice(&vertex.rgba);
    }
    for &index in &mesh.indices {
        push_u32(out, index);
    }
}

#[cfg(test)]
fn encode_stable_gzip(input: &[u8]) -> Vec<u8> {
    let deflate = miniz_oxide::deflate::compress_to_vec(input, 1);
    let mut gzip = Vec::with_capacity(10 + deflate.len() + 8);
    // RFC 1952 header: deflate, no optional fields or timestamp, fastest
    // compression marker, and reproducible unknown-OS byte.
    gzip.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 4, 255]);
    gzip.extend_from_slice(&deflate);
    gzip.extend_from_slice(&crc32fast::hash(input).to_le_bytes());
    gzip.extend_from_slice(&(input.len() as u32).to_le_bytes());
    gzip
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    // Workers own independent frames and contexts. Output buffers stay with
    // their caller; only bounded native codec workspace survives between jobs.
    static SURFACE_COMPRESSOR: std::cell::RefCell<Option<zstd::bulk::Compressor<'static>>> = const { std::cell::RefCell::new(None) };
    static SURFACE_DECOMPRESSOR: std::cell::RefCell<Option<zstd::zstd_safe::DCtx<'static>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
fn new_surface_compressor() -> Option<zstd::bulk::Compressor<'static>> {
    let mut compressor = zstd::bulk::Compressor::new(1).ok()?;
    compressor.include_checksum(true).ok()?;
    compressor.include_contentsize(true).ok()?;
    Some(compressor)
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_zstd(input: &[u8]) -> Option<Vec<u8>> {
    SURFACE_COMPRESSOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(new_surface_compressor()?);
        }
        let compressor = slot.as_mut()?;
        // compress2 resets each frame; this private context never receives a
        // dictionary or parameters from another codec user.
        let result = compressor.compress(input).ok();
        if result.is_none() || compressor.context_mut().sizeof() > MAX_RETAINED_CODEC_BYTES {
            slot.take();
        }
        result
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn decode_zstd(bytes: &[u8], size: usize) -> Option<Vec<u8>> {
    SURFACE_DECOMPRESSOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(zstd::zstd_safe::DCtx::try_create()?);
        }
        let decompressor = slot.as_mut()?;
        let mut output = Vec::with_capacity(size);
        let result = decompressor.decompress(&mut output, bytes);
        if result.is_err() || decompressor.sizeof() > MAX_RETAINED_CODEC_BYTES {
            slot.take();
        }
        result.ok()?;
        Some(output)
    })
}

fn decode_payload(bytes: &[u8], max_bytes: usize) -> Option<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    if bytes.starts_with(&ZSTD_MAGIC) {
        let descriptor = *bytes.get(4)?;
        // A cache record owns one checksummed frame with a bounded, known
        // output size. Check the window too before the decoder can allocate.
        if descriptor & 0x04 == 0 {
            return None;
        }
        let size = usize::try_from(zstd::zstd_safe::get_frame_content_size(bytes).ok()??).ok()?;
        if size > max_bytes {
            return None;
        }
        if descriptor & 0x20 == 0 {
            // Zstandard's Window_Descriptor: 5 exponent and 3 mantissa bits.
            // Single-segment frames instead use the validated content size.
            let window = *bytes.get(5)?;
            let base = 1u64 << (10 + (window >> 3));
            let window_size = base + (base / 8) * u64::from(window & 7);
            if window_size > max_bytes as u64 {
                return None;
            }
        }
        if zstd::zstd_safe::find_frame_compressed_size(bytes).ok()? != bytes.len() {
            return None;
        }
        let output = decode_zstd(bytes, size)?;
        return (output.len() == size).then_some(output);
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        // Compression framing is independent of the versioned surface schema.
        // BufRead preserves the exact end so concatenated/trailing data
        // cannot hide behind the first valid frame.
        let mut decoder = flate2::bufread::GzDecoder::new(bytes);
        let mut output = Vec::new();
        decoder
            .by_ref()
            .take(max_bytes.checked_add(1)? as u64)
            .read_to_end(&mut output)
            .ok()?;
        if output.len() > max_bytes || !decoder.into_inner().is_empty() {
            return None;
        }
        Some(output)
    } else {
        None
    }
}

fn decode(cell: CellId, bytes: &[u8]) -> Option<SurfaceCell> {
    let b = decode_payload(bytes, MAX_DECOMPRESSED_BYTES)?;
    if b.get(..4)? != MAGIC {
        return None;
    }
    let complete = match *b.get(4)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let mut pos = 5;
    let u32at = |pos: &mut usize| -> Option<u32> {
        let value = u32::from_le_bytes(b.get(*pos..*pos + 4)?.try_into().ok()?);
        *pos += 4;
        Some(value)
    };
    let f32at = |pos: &mut usize| -> Option<f32> {
        let value = f32::from_bits(u32at(pos)?);
        value.is_finite().then_some(value)
    };
    let h0 = f32at(&mut pos)?;
    let h1 = f32at(&mut pos)?;
    let terrain_step_m = f32at(&mut pos)?;
    let terrain_relief_m = f32at(&mut pos)?;
    if h0 > h1 || terrain_step_m < 0.0 || terrain_relief_m < 0.0 {
        return None;
    }
    let n_road_guides = u32at(&mut pos)? as usize;
    if n_road_guides > MAX_ROAD_GUIDES_PER_CELL {
        return None;
    }
    let mut road_guides = Vec::with_capacity(n_road_guides);
    for _ in 0..n_road_guides {
        let guide = RoadGuide {
            a_local: [f32at(&mut pos)?, f32at(&mut pos)?],
            b_local: [f32at(&mut pos)?, f32at(&mut pos)?],
            a_exit: [f32at(&mut pos)?, f32at(&mut pos)?],
            b_exit: [f32at(&mut pos)?, f32at(&mut pos)?],
            width_m: f32at(&mut pos)?,
            assist: f32at(&mut pos)?,
        };
        if !guide
            .a_local
            .into_iter()
            .chain(guide.b_local)
            .chain(guide.a_exit)
            .chain(guide.b_exit)
            .chain([guide.width_m, guide.assist])
            .all(f32::is_finite)
            || [guide.a_exit, guide.b_exit].into_iter().any(|exit| {
                let length_sq = exit[0] * exit[0] + exit[1] * exit[1];
                length_sq > 1.001 * 1.001
            })
            || guide.width_m <= 0.0
            || !(0.0..=1.0).contains(&guide.assist)
        {
            return None;
        }
        road_guides.push(guide);
    }
    let n_lights = u32at(&mut pos)? as usize;
    if n_lights > NIGHT_LIGHT_BUCKETS {
        return None;
    }
    let mut night_lights = Vec::with_capacity(n_lights);
    for _ in 0..n_lights {
        let local_pos = [f32at(&mut pos)?, f32at(&mut pos)?, f32at(&mut pos)?];
        let energy = f32at(&mut pos)?;
        let packed = b.get(pos..pos + 4)?;
        pos += 4;
        if !local_pos.iter().all(|value| value.is_finite())
            || !energy.is_finite()
            || energy <= 0.0
            || packed[3] as usize >= NIGHT_LIGHT_BUCKETS
        {
            return None;
        }
        night_lights.push(NightLightCluster {
            local_pos,
            energy,
            color: [packed[0], packed[1], packed[2]],
            bucket: packed[3],
        });
    }
    let n_trees = u32at(&mut pos)? as usize;
    if n_trees > 1024 {
        return None;
    }
    let mut tree_instances = Vec::with_capacity(n_trees);
    for _ in 0..n_trees {
        let profile = TREE_PROFILES.get(*b.get(pos)? as usize)?.id;
        pos += 1;
        let seed = u64::from_le_bytes(b.get(pos..pos + 8)?.try_into().ok()?);
        pos += 8;
        let lat_rad = f64::from_le_bytes(b.get(pos..pos + 8)?.try_into().ok()?);
        pos += 8;
        let lon_rad = f64::from_le_bytes(b.get(pos..pos + 8)?.try_into().ok()?);
        pos += 8;
        let elevation_m = f32at(&mut pos)?;
        let height_m = f32at(&mut pos)?;
        let yaw_rad = f32at(&mut pos)?;
        let cell_x = f32at(&mut pos)?;
        let cell_z = f32at(&mut pos)?;
        if !lat_rad.is_finite()
            || !lon_rad.is_finite()
            || ![elevation_m, height_m, yaw_rad, cell_x, cell_z]
                .into_iter()
                .all(f32::is_finite)
            || height_m <= 0.0
        {
            return None;
        }
        tree_instances.push(TreeInstance {
            profile,
            seed,
            lat_rad,
            lon_rad,
            elevation_m,
            height_m,
            yaw_rad,
            cell_x,
            cell_z,
        });
    }
    let n_flora = u32at(&mut pos)? as usize;
    if n_flora > MAX_FLORA_DESCRIPTORS_PER_CELL {
        return None;
    }
    let mut flora = Vec::with_capacity(n_flora);
    for _ in 0..n_flora {
        let archetype = FloraArchetype::from_u8(*b.get(pos)?)?;
        pos += 1;
        let calendar = CultivationCalendar::from_u8(*b.get(pos)?)?;
        pos += 1;
        let field_seed = u64::from_le_bytes(b.get(pos..pos + 8)?.try_into().ok()?);
        pos += 8;
        let row_heading_rad = f32at(&mut pos)?;
        let palette = *b.get(pos)?;
        pos += 1;
        let base_density = *b.get(pos)?;
        pos += 1;
        let mut coverage = [0u32; FLORA_COVERAGE_WORDS];
        for word in &mut coverage {
            *word = u32at(&mut pos)?;
        }
        let ground_base_cm = i32::from_le_bytes(b.get(pos..pos + 4)?.try_into().ok()?);
        pos += 4;
        let mut ground_cm = [0i16; FLORA_GROUND_SAMPLES];
        for offset in &mut ground_cm {
            *offset = i16::from_le_bytes(b.get(pos..pos + 2)?.try_into().ok()?);
            pos += 2;
        }
        let phenology_offset = *b.get(pos)?;
        let curated_style = *b.get(pos + 1)?;
        pos += 2;
        if field_seed == 0
            || !row_heading_rad.is_finite()
            || !(0.0..std::f32::consts::TAU).contains(&row_heading_rad)
            || (palette >= 4
                && !vearth_gen::agriculture_catalog::treatment_for_palette(palette)
                    .is_some_and(|treatment| treatment.archetype == archetype))
            || base_density == 0
            || coverage.iter().all(|word| *word == 0)
        {
            return None;
        }
        flora.push(FloraCell {
            archetype,
            calendar,
            field_seed,
            row_heading_rad,
            palette,
            base_density,
            coverage,
            ground_base_cm,
            ground_cm,
            phenology_offset,
            curated_style,
        });
    }

    let mut reader = Reader {
        bytes: &b,
        pos,
        vertices_left: MAX_VERTICES,
        indices_left: MAX_INDICES,
    };
    let count = reader.count(MAX_BATCHES, 17)?;
    let mut batches = Vec::with_capacity(count);
    for _ in 0..count {
        let k = i32::from_le_bytes(reader.take()?);
        if k.checked_mul(32).is_none() {
            return None;
        }
        let detail = match reader.take::<1>()?[0] {
            0 => None,
            1 => {
                let detail = BuiltDetailBounds {
                    center: reader.point()?,
                    radius_m: reader.float()?,
                    min_screen_px: reader.float()?,
                };
                if detail.radius_m <= 0.0 || detail.min_screen_px <= 0.0 {
                    return None;
                }
                Some(detail)
            }
            _ => return None,
        };
        let mesh = reader.mesh()?;
        let span_count = reader.count(mesh.indices.len() / 3, 12)?;
        let mut shadow_omissions = Vec::with_capacity(span_count);
        for _ in 0..span_count {
            shadow_omissions.push(vearth_gen::ShadowOmission {
                first_index: u32::from_le_bytes(reader.take()?),
                index_count: u32::from_le_bytes(reader.take()?),
                max_deviation_m: reader.float()?,
            });
        }
        let batch = SurfaceBatch {
            addr: ChunkAddr { cell, k },
            detail,
            mesh,
            shadow_omissions,
        };
        if !batch.validate_shadow_omissions() {
            return None;
        }
        batches.push(batch);
    }
    let terrain_collision = match reader.take::<1>()?[0] {
        0 => None,
        1 => {
            let mut heights_local = Box::new([0.0; 33 * 33]);
            for height in heights_local.iter_mut() {
                *height = reader.float()?;
            }
            let mut source_columns = [0; 32];
            for mask in &mut source_columns {
                *mask = u32::from_le_bytes(reader.take()?);
            }
            let terrain = TerrainCollision {
                heights_local,
                source_columns,
            };
            terrain.validate().ok()?;
            Some(terrain)
        }
        _ => return None,
    };
    let count = reader.count(MAX_COLLISION_TRIANGLES, 36)?;
    let mut triangles = Vec::with_capacity(count);
    for _ in 0..count {
        triangles.push(CollisionTriangle {
            vertices: [reader.point()?, reader.point()?, reader.point()?],
        });
    }
    let count = reader.count(MAX_CAPSULES, 32)?;
    let mut capsules = Vec::with_capacity(count);
    for _ in 0..count {
        capsules.push(Capsule {
            a: reader.point()?,
            b: reader.point()?,
            radius_a: reader.float()?,
            radius_b: reader.float()?,
        });
    }
    let count = reader.count(MAX_COLLISION_TRIANGLES, 41)?;
    let mut water = Vec::with_capacity(count);
    use vearth_data::vectors::WaterRegionKind::*;
    let kinds = [
        Ocean,
        InlandWater,
        Lake,
        Reservoir,
        River,
        Canal,
        Stream,
        Drainage,
        Basin,
        Dock,
        Decorative,
        Playa,
        Reef,
        Other,
        CompatibilityUnknown,
    ];
    for _ in 0..count {
        let vertices = [reader.point()?, reader.point()?, reader.point()?];
        let kind = *kinds.get(reader.take::<1>()?[0] as usize)?;
        let depth_m = reader.float()?;
        if depth_m < 0.0 {
            return None;
        }
        water.push(vearth_gen::water_surface::WaterTriangle {
            vertices,
            kind,
            depth_m,
        });
    }
    let terrain_detail = match reader.take::<1>()?[0] {
        0 => None,
        1 => {
            let mut heights_m = Box::new([0.0; 33 * 33]);
            for height in heights_m.iter_mut() {
                *height = reader.float()?;
            }
            let mut source_columns = [0; 32];
            for mask in &mut source_columns {
                *mask = u32::from_le_bytes(reader.take::<4>()?);
            }
            let detail = vearth_gen::TerrainDetailGrid {
                heights_m,
                source_columns,
            };
            detail.validate().ok()?;
            Some(detail)
        }
        _ => return None,
    };
    if reader.pos != b.len() {
        return None;
    }
    let solid_collision = SurfaceCollision::from_parts(triangles, capsules);
    solid_collision.validate().ok()?;
    Some(SurfaceCell {
        cell,
        batches,
        height_range: (h0, h1),
        terrain_step_m,
        terrain_relief_m,
        terrain_detail,
        terrain_collision,
        solid_collision,
        water,
        road_guides,
        night_lights,
        tree_instances,
        flora,
        complete,
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    vertices_left: usize,
    indices_left: usize,
}
impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let end = self.pos.checked_add(N)?;
        let result = self.bytes.get(self.pos..end)?.try_into().ok()?;
        self.pos = end;
        Some(result)
    }

    fn count(&mut self, limit: usize, minimum_bytes: usize) -> Option<usize> {
        let count = u32::from_le_bytes(self.take()?) as usize;
        (count <= limit && count <= self.bytes.len().checked_sub(self.pos)? / minimum_bytes)
            .then_some(count)
    }

    fn float(&mut self) -> Option<f32> {
        let value = f32::from_le_bytes(self.take()?);
        value.is_finite().then_some(value)
    }

    fn point(&mut self) -> Option<[f32; 3]> {
        Some([self.float()?, self.float()?, self.float()?])
    }

    fn mesh(&mut self) -> Option<MeshBuffers> {
        let vertex_count = self.count(self.vertices_left, 20)?;
        let index_count = self.count(self.indices_left, 4)?;
        if (vertex_count == 0) != (index_count == 0) || index_count % 3 != 0 {
            return None;
        }
        let required = vertex_count
            .checked_mul(20)?
            .checked_add(index_count.checked_mul(4)?)?;
        self.bytes.get(self.pos..self.pos.checked_add(required)?)?;
        self.vertices_left -= vertex_count;
        self.indices_left -= index_count;
        let mut vertices = Vec::with_capacity(vertex_count);
        for _ in 0..vertex_count {
            let pos = self.point()?;
            let face_ao = u32::from_le_bytes(self.take()?);
            let material = ((face_ao >> 5) & 255)
                & !u32::from(vearth_gen::landmarks::CALIBRATION_MATERIAL_FLAG);
            if !(1..=u32::from(vearth_gen::material::LANDMARK_SPARKLE)).contains(&material)
                && !matches!(
                    material as u8,
                    vearth_gen::material::CITY_LIGHT
                        | vearth_gen::material::SCULPTURE_METAL
                        | vearth_gen::material::CONCRETE_PATH
                        | vearth_gen::material::BRICK_PAVING
                        | vearth_gen::material::STONE_PAVING
                        | vearth_gen::material::GRAVEL_PATH
                        | vearth_gen::material::COBBLESTONE
                        | vearth_gen::material::STONE_SLAB
                )
                && !vearth_gen::material::is_architecture_wall(material as u8)
                && !vearth_gen::material::is_architecture_roof(material as u8)
            {
                return None;
            }
            vertices.push(SurfaceVertex {
                pos,
                face_ao,
                rgba: self.take()?,
            });
        }
        let mut indices = Vec::with_capacity(index_count);
        for _ in 0..index_count {
            let index = u32::from_le_bytes(self.take()?);
            if index as usize >= vertex_count {
                return None;
            }
            indices.push(index);
        }
        Some(MeshBuffers { vertices, indices })
    }
}

/// Bounded codec-only timing for the generated-surface cache. This bypasses
/// RAM/disk tiers so encode and decode remain independently observable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SurfaceCodecProfile {
    pub encoded_bytes: usize,
    pub encode_ns: Vec<u64>,
    pub decode_ns: Vec<u64>,
}

pub fn profile_surface_codec(column: &SurfaceCell, samples: usize) -> Option<SurfaceCodecProfile> {
    let samples = samples.clamp(1, 10_000);
    let mut profile = SurfaceCodecProfile {
        encode_ns: Vec::with_capacity(samples),
        decode_ns: Vec::with_capacity(samples),
        ..Default::default()
    };
    for _ in 0..samples {
        let started = vearth_platform::time::Instant::now();
        let bytes = encode(column)?;
        profile.encode_ns.push(started.elapsed().as_nanos() as u64);
        profile.encoded_bytes = bytes.len();
        let started = vearth_platform::time::Instant::now();
        let decoded = decode(column.cell, &bytes)?;
        profile.decode_ns.push(started.elapsed().as_nanos() as u64);
        std::hint::black_box(decoded);
    }
    Some(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepared_render::{pack_surface_cell, test_surface};
    use vearth_core::cubesphere::Face;
    fn cell() -> CellId {
        CellId {
            face: Face(0),
            level: 17,
            i: 65536,
            j: 65536,
        }
    }
    fn raw(bytes: &[u8]) -> Vec<u8> {
        decode_payload(bytes, MAX_DECOMPRESSED_BYTES).unwrap()
    }

    #[test]
    fn reused_surface_codecs_preserve_frames_and_recover_after_corruption() {
        let mut state = 0x13d6b189u32;
        for size in [0, 1, 257, 65_536, 2_097_152, 19, 257] {
            let input: Vec<_> = (0..size)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state >> 24) as u8
                })
                .collect();
            let expected = new_surface_compressor().unwrap().compress(&input).unwrap();
            let frame = encode_zstd(&input).unwrap();
            assert_eq!(frame, expected, "frame of {size} bytes");
            assert_eq!(frame[4] & 0x03, 0, "frames never use a dictionary");
            assert_eq!(
                decode_payload(&frame, MAX_DECOMPRESSED_BYTES),
                Some(input.clone())
            );

            let mut corrupt = frame.clone();
            *corrupt.last_mut().unwrap() ^= 1;
            assert!(decode_payload(&corrupt, MAX_DECOMPRESSED_BYTES).is_none());
            SURFACE_DECOMPRESSOR.with(|slot| assert!(slot.borrow().is_none()));
            assert_eq!(decode_payload(&frame, MAX_DECOMPRESSED_BYTES), Some(input));
            SURFACE_COMPRESSOR.with(|slot| {
                assert!(slot.borrow_mut().as_mut().is_none_or(|context| {
                    context.context_mut().sizeof() <= MAX_RETAINED_CODEC_BYTES
                }));
            });
            SURFACE_DECOMPRESSOR.with(|slot| {
                assert!(
                    slot.borrow()
                        .as_ref()
                        .is_none_or(|context| { context.sizeof() <= MAX_RETAINED_CODEC_BYTES })
                );
            });
        }
    }

    #[test]
    #[ignore = "manual release-mode codec context benchmark"]
    fn benchmark_surface_codec_context_reuse() {
        use vearth_platform::time::Instant;

        for size in [16_384, 262_144, 4_194_304] {
            let input: Vec<_> = (0..size).map(|i| ((i * 31 ^ i / 32) % 251) as u8).collect();
            let encoded = new_surface_compressor().unwrap().compress(&input).unwrap();
            let mut samples = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
            for iteration in 0..42 {
                let order = if iteration % 2 == 0 {
                    [0, 1, 2, 3]
                } else {
                    [3, 2, 1, 0]
                };
                for index in order {
                    let start = Instant::now();
                    let result = match index {
                        0 => new_surface_compressor().unwrap().compress(&input).unwrap(),
                        1 => encode_zstd(&input).unwrap(),
                        2 => zstd::bulk::decompress(&encoded, input.len()).unwrap(),
                        3 => decode_zstd(&encoded, input.len()).unwrap(),
                        _ => unreachable!(),
                    };
                    std::hint::black_box(result);
                    if iteration >= 2 {
                        samples[index].push(start.elapsed().as_secs_f64() * 1_000_000.0);
                    }
                }
            }
            for values in &mut samples {
                values.sort_by(f64::total_cmp);
            }
            eprintln!(
                "codec size={size}: encode fresh={:.1}us reused={:.1}us; decode fresh={:.1}us reused={:.1}us",
                samples[0][20], samples[1][20], samples[2][20], samples[3][20],
            );
        }
    }

    #[test]
    fn zstd_and_legacy_gzip_restore_identical_surface_products() {
        let source = test_surface(cell());
        let encoded = encode(&source).unwrap();
        assert!(encoded.starts_with(&ZSTD_MAGIC));
        assert_ne!(encoded[4] & 0x04, 0);
        let uncompressed = raw(&encoded);
        assert_eq!(&uncompressed[..4], MAGIC);
        assert_eq!(
            zstd::zstd_safe::get_frame_content_size(&encoded).unwrap(),
            Some(uncompressed.len() as u64)
        );
        let legacy = encode_stable_gzip(&uncompressed);
        assert_eq!(raw(&legacy), uncompressed);
        let expected = pack_surface_cell(source).unwrap();
        for bytes in [&encoded, &legacy] {
            let restored = decode(cell(), bytes).unwrap();
            assert_eq!(pack_surface_cell(restored).unwrap(), expected);
        }
        for _ in 0..4 {
            assert_eq!(encode_zstd(&uncompressed).unwrap(), encoded);
        }
    }

    #[test]
    fn compressed_surface_records_reject_truncation_corruption_and_extra_frames() {
        let uncompressed = raw(&encode(&test_surface(cell())).unwrap());
        for encoded in [
            encode_zstd(&uncompressed).unwrap(),
            encode_stable_gzip(&uncompressed),
        ] {
            for end in 0..encoded.len() {
                assert!(decode(cell(), &encoded[..end]).is_none(), "prefix {end}");
            }
            for offset in [encoded.len() / 2, encoded.len() - 4, encoded.len() - 1] {
                let mut corrupt = encoded.clone();
                corrupt[offset] ^= 0x80;
                assert!(decode(cell(), &corrupt).is_none(), "corruption {offset}");
            }
            for tail in [b"trailing bytes".as_slice(), encoded.as_slice()] {
                let mut trailing = encoded.clone();
                trailing.extend_from_slice(tail);
                assert!(decode(cell(), &trailing).is_none());
            }
        }
        let mut skippable = vec![0x50, 0x2a, 0x4d, 0x18, 0, 0, 0, 0];
        skippable.extend_from_slice(&encode_zstd(&uncompressed).unwrap());
        assert!(decode(cell(), &skippable).is_none());
    }

    #[test]
    fn compressed_payload_limits_apply_to_legacy_and_current_frames() {
        let bytes: Vec<_> = (0..1024).map(|n| ((n * 37 + n / 7) % 256) as u8).collect();
        for encoded in [encode_zstd(&bytes).unwrap(), encode_stable_gzip(&bytes)] {
            assert_eq!(decode_payload(&encoded, bytes.len()).unwrap(), bytes);
            assert!(decode_payload(&encoded, bytes.len() - 1).is_none());
        }
        let empty = encode_zstd(&[]).unwrap();
        assert_eq!(decode_payload(&empty, 0).unwrap(), Vec::<u8>::new());
        assert!(decode(cell(), &empty).is_none());
    }

    #[test]
    fn zstd_rejects_missing_checksum_unknown_size_and_excessive_window() {
        let bytes = vec![42u8; 1024];
        let unchecked = zstd::bulk::compress(&bytes, 1).unwrap();
        assert_eq!(unchecked[4] & 4, 0);
        assert!(decode_payload(&unchecked, MAX_DECOMPRESSED_BYTES).is_none());
        let mut compressor = zstd::bulk::Compressor::new(1).unwrap();
        compressor.include_checksum(true).unwrap();
        compressor.include_contentsize(false).unwrap();
        let unknown_size = compressor.compress(&bytes).unwrap();
        assert_eq!(
            zstd::zstd_safe::get_frame_content_size(&unknown_size).unwrap(),
            None
        );
        assert!(decode_payload(&unknown_size, MAX_DECOMPRESSED_BYTES).is_none());

        // Rewrite a valid single-segment frame to advertise a separate window;
        // the compressed data/checksum remain valid for the bounded window.
        let mut separate = encode_zstd(&bytes).unwrap();
        assert_ne!(separate[4] & 0x20, 0);
        assert_ne!(separate[4] >> 6, 0);
        separate[4] &= !0x20;
        separate.insert(5, 0); // 1 KiB window.
        assert_eq!(decode_payload(&separate, 1024).unwrap(), bytes);
        separate[5] = 1; // 1 KiB + 128 B; content size itself is still only 1 KiB.
        assert!(decode_payload(&separate, 1024).is_none());
        separate[5] = 0xff;
        assert!(decode_payload(&separate, MAX_DECOMPRESSED_BYTES).is_none());
    }

    #[test]
    fn zstd_rejects_oversized_advertised_content_before_output_allocation() {
        for size in [MAX_DECOMPRESSED_BYTES as u64 + 1, u64::MAX] {
            // Four-byte magic, 8-byte content size, single segment, checksum.
            let mut header = ZSTD_MAGIC.to_vec();
            header.push(0xe4);
            header.extend_from_slice(&size.to_le_bytes());
            header.extend_from_slice(&[1, 0, 0]); // Empty last raw block.
            header.extend_from_slice(&[0; 4]);
            assert!(decode_payload(&header, MAX_DECOMPRESSED_BYTES).is_none());
        }
    }
    #[test]
    fn surface_cache_roundtrip_preserves_gpu_and_collision_products() {
        let source = test_surface(cell());
        let bytes = encode(&source).unwrap();
        assert_eq!(bytes, encode(&source).unwrap());
        let restored = decode(cell(), &bytes).unwrap();
        assert_eq!(restored.solid_collision, source.solid_collision);
        assert_eq!(restored.water, source.water);
        let expected = pack_surface_cell(source).unwrap();
        let actual = pack_surface_cell(restored).unwrap();
        assert_eq!(
            actual.compute_semantic_fingerprint(),
            expected.compute_semantic_fingerprint()
        );
        assert!(actual.validate().is_ok());
    }

    #[test]
    fn regional_crop_palettes_survive_cache_and_reject_mismatched_forms() {
        let mut source = test_surface(cell());
        for treatment in vearth_gen::agriculture_catalog::CROP_TREATMENTS {
            source.flora = vec![FloraCell {
                archetype: treatment.archetype,
                calendar: CultivationCalendar::TemperateSummer,
                field_seed: 42,
                row_heading_rad: 0.5,
                palette: treatment.kind.palette(),
                base_density: 240,
                coverage: [u32::MAX; FLORA_COVERAGE_WORDS],
                ground_base_cm: 0,
                ground_cm: [0; FLORA_GROUND_SAMPLES],
                phenology_offset: 128,
                curated_style: 0,
            }];
            let restored = decode(cell(), &encode(&source).unwrap()).unwrap();
            assert_eq!(restored.flora, source.flora);
            source.flora[0].archetype = FloraArchetype::Vineyard;
            assert!(decode(cell(), &encode(&source).unwrap()).is_none());
        }
    }

    #[test]
    fn terrain_relief_roundtrip_is_independent_of_geometry_and_content_heights() {
        let mut source = test_surface(cell());
        source.height_range = (-77_000.0, 20.0);
        for relief in [0.0_f32, -0.0, 12.25, f32::MAX] {
            source.terrain_relief_m = relief;
            let encoded = encode(&source).unwrap();
            assert_eq!(encoded_size(&source), Some(raw(&encoded).len()));
            for bytes in [encoded.clone(), encode_stable_gzip(&raw(&encoded))] {
                let restored = decode(cell(), &bytes).unwrap();
                assert_eq!(restored.terrain_relief_m.to_bits(), relief.to_bits());
                assert_eq!(restored.height_range, source.height_range);
                assert_eq!(restored.terrain_step_m, source.terrain_step_m);
                assert_eq!(restored.batches.len(), source.batches.len());
                for (restored, source) in restored.batches.iter().zip(&source.batches) {
                    assert_eq!(restored.addr, source.addr);
                    assert_eq!(restored.mesh.vertices, source.mesh.vertices);
                    assert_eq!(restored.mesh.indices, source.mesh.indices);
                    assert_eq!(restored.shadow_omissions, source.shadow_omissions);
                }
                assert_eq!(restored.solid_collision, source.solid_collision);
                assert_eq!(
                    pack_surface_cell(restored).unwrap(),
                    pack_surface_cell(source.clone()).unwrap()
                );
            }
        }
    }

    #[test]
    fn terrain_relief_invalid_records_and_previous_surface_schema_are_rejected() {
        let source = test_surface(cell());
        let good = raw(&encode(&source).unwrap());
        let relief_offset = 4 + 1 + 3 * 4;
        for relief in [-0.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut invalid = source.clone();
            invalid.terrain_relief_m = relief;
            assert!(encode(&invalid).is_none());
            let mut bytes = good.clone();
            bytes[relief_offset..relief_offset + 4].copy_from_slice(&relief.to_le_bytes());
            for encoded in [encode_zstd(&bytes).unwrap(), encode_stable_gzip(&bytes)] {
                assert!(decode(cell(), &encoded).is_none());
            }
        }
        let mut previous = good;
        previous[..4].copy_from_slice(b"VXS2");
        previous.drain(relief_offset..relief_offset + 4);
        for encoded in [
            encode_zstd(&previous).unwrap(),
            encode_stable_gzip(&previous),
        ] {
            assert!(decode(cell(), &encoded).is_none());
        }
    }

    #[test]
    fn terrain_detail_cache_preserves_physical_grid_without_changing_geometry() {
        let mut source = test_surface(cell());
        let grid = source.terrain_detail.as_mut().unwrap();
        grid.source_columns = [0; 32];
        grid.source_columns[0] = 1;
        grid.source_columns[31] = 1 << 31;
        grid.heights_m[0] = -17.25;
        grid.heights_m[1] = -0.0;
        grid.heights_m[33 * 33 - 1] = 1_234.5;
        let encoded = encode(&source).unwrap();
        assert_eq!(encoded_size(&source), Some(raw(&encoded).len()));
        for bytes in [encoded.clone(), encode_stable_gzip(&raw(&encoded))] {
            let restored = decode(cell(), &bytes).unwrap();
            let original_grid = source.terrain_detail.as_ref().unwrap();
            let restored_grid = restored.terrain_detail.as_ref().unwrap();
            assert_eq!(restored_grid.source_columns, original_grid.source_columns);
            assert!(
                restored_grid
                    .heights_m
                    .iter()
                    .zip(original_grid.heights_m.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
            assert_eq!(
                pack_surface_cell(restored).unwrap(),
                pack_surface_cell(source.clone()).unwrap()
            );
        }
        source.terrain_detail = None;
        let restored = decode(cell(), &encode(&source).unwrap()).unwrap();
        assert!(restored.terrain_detail.is_none());
    }

    #[test]
    fn terrain_detail_cache_rejects_invalid_grids_and_previous_schema() {
        let source = test_surface(cell());
        let good = raw(&encode(&source).unwrap());
        let offset = good.len() - 1 - (33 * 33 + 32) * 4;
        assert_eq!(good[offset], 1);
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut invalid = source.clone();
            invalid.terrain_detail.as_mut().unwrap().heights_m[0] = value;
            assert!(encode(&invalid).is_none());
            let mut bytes = good.clone();
            bytes[offset + 1..offset + 5].copy_from_slice(&value.to_le_bytes());
            assert!(decode(cell(), &encode_zstd(&bytes).unwrap()).is_none());
        }
        let mut bad = good.clone();
        bad[offset] = 2;
        assert!(decode(cell(), &encode_zstd(&bad).unwrap()).is_none());
        let mut previous = good[..offset].to_vec();
        previous[..4].copy_from_slice(b"VXS3");
        assert!(decode(cell(), &encode_zstd(&previous).unwrap()).is_none());
    }

    #[test]
    fn bounded_shadow_spans_roundtrip_and_reject_invalid_records() {
        let mut source = test_surface(cell());
        let original = source.batches[0].mesh.indices.clone();
        source.batches[0].mesh.indices.extend_from_slice(&original);
        source.batches[0].mesh.indices.extend_from_slice(&original);
        source.batches[0]
            .shadow_omissions
            .push(vearth_gen::ShadowOmission {
                first_index: 3,
                index_count: 3,
                max_deviation_m: 0.02,
            });
        let encoded = encode(&source).unwrap();
        let restored = decode(cell(), &encoded).unwrap();
        assert_eq!(
            restored.batches[0].shadow_omissions,
            source.batches[0].shadow_omissions
        );
        assert_eq!(
            restored.batches[0].mesh.indices,
            source.batches[0].mesh.indices
        );
        assert_eq!(
            vearth_gen::contracts::surface_cell_fingerprint(&restored),
            vearth_gen::contracts::surface_cell_fingerprint(&source)
        );
        assert_eq!(
            pack_surface_cell(restored).unwrap(),
            pack_surface_cell(source.clone()).unwrap()
        );
        let good = raw(&encoded);
        assert_eq!(encoded_size(&source), Some(good.len()));
        let mut record = Vec::new();
        push_u32(&mut record, 3);
        push_u32(&mut record, 3);
        push_f32(&mut record, 0.02);
        let offsets: Vec<_> = good
            .windows(record.len())
            .enumerate()
            .filter_map(|(i, v)| (v == record).then_some(i))
            .collect();
        assert_eq!(offsets.len(), 1);
        let offset = offsets[0];
        for (field, value) in [
            (0, 1),
            (0, u32::MAX),
            (4, 0),
            (4, 9),
            (8, f32::NAN.to_bits()),
            (8, (-0.01f32).to_bits()),
        ] {
            let mut bad = good.clone();
            bad[offset + field..offset + field + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode(cell(), &encode_stable_gzip(&bad)).is_none());
        }
        let mut excessive = good.clone();
        excessive[offset - 4..offset].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(cell(), &encode_stable_gzip(&excessive)).is_none());
        source.batches[0].shadow_omissions[0].first_index = 1;
        assert!(encode(&source).is_none());
        let mut old = good;
        old[..4].copy_from_slice(b"VXS1");
        assert!(decode(cell(), &encode_stable_gzip(&old)).is_none());
    }

    #[test]
    fn cache_encoder_obeys_decoder_allocation_limits() {
        let mut source = test_surface(cell());
        let bytes = encode(&source).unwrap();
        assert_eq!(encoded_size(&source), Some(raw(&bytes).len()));
        let capsule = source.solid_collision.capsules()[0];
        source.solid_collision =
            SurfaceCollision::from_parts(Vec::new(), vec![capsule; MAX_CAPSULES + 1]);
        assert!(encode(&source).is_none());
        source
            .solid_collision
            .limit_complexity(&mut source.terrain_collision);
        let restored = decode(cell(), &encode(&source).unwrap()).unwrap();
        assert!(restored.solid_collision.is_empty());
        assert_eq!(restored.terrain_collision, source.terrain_collision);
        source.batches = vec![source.batches[0].clone(); MAX_BATCHES + 1];
        assert!(encode(&source).is_none());
    }
    #[test]
    fn generated_water_cache_preserves_visible_top_contact_and_typed_metadata() {
        use crate::prepared_render::{test_generated_water, test_water_contact_top};
        use vearth_data::vectors::WaterRegionKind;
        for kind in [WaterRegionKind::Ocean, WaterRegionKind::Lake] {
            let source = test_generated_water(kind);
            let top = test_water_contact_top(&source);
            let restored = decode(source.cell, &encode(&source).unwrap()).unwrap();
            assert_eq!(restored.water, source.water);
            assert_eq!(restored.solid_collision, source.solid_collision);
            assert!(
                (restored.solid_collision.top(0., 0.).unwrap() - top).abs() < 1e-4,
                "{kind:?} cached contact differs from visible top"
            );
            assert!(restored.solid_collision.contains(0., top - 0.02, 0.));
            let distance = restored
                .solid_collision
                .raycast([0., top + 5., 0.], [0., -1., 0.], 10.)
                .unwrap();
            assert!(
                (distance - 5.).abs() < 1e-4,
                "{kind:?} cached camera ray crossed water"
            );
            assert_eq!(
                pack_surface_cell(restored)
                    .unwrap()
                    .compute_semantic_fingerprint(),
                pack_surface_cell(source)
                    .unwrap()
                    .compute_semantic_fingerprint()
            );
        }
    }

    #[test]
    fn voxel_payloads_and_malformed_surface_records_are_rejected() {
        for magic in [
            b"VXC9", b"VXA1", b"VXA2", b"VXA3", b"VXA4", b"VXA5", b"VXA6", b"VXA7",
        ] {
            assert!(decode(cell(), &encode_stable_gzip(magic)).is_none());
        }
        let good = raw(&encode(&test_surface(cell())).unwrap());
        for end in [0, 3, 4, 5, 17, 21, good.len() / 2, good.len() - 1] {
            assert!(decode(cell(), &encode_stable_gzip(&good[..end])).is_none());
        }
        for (offset, value) in [
            (5, f32::NAN.to_bits()),
            (37, u32::MAX),
            (54, f32::INFINITY.to_bits()),
            (114, u32::MAX),
        ] {
            let mut bad = good.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(
                decode(cell(), &encode_stable_gzip(&bad)).is_none(),
                "bad field at {offset}"
            );
        }
        let mut bad = good.clone();
        let end = bad.len();
        let detail_bytes = 1 + (33 * 33 + 32) * 4;
        bad[end - detail_bytes - 5] = 255;
        assert!(decode(cell(), &encode_stable_gzip(&bad)).is_none());
        let mut bad = good;
        bad.push(0);
        assert!(decode(cell(), &encode_stable_gzip(&bad)).is_none());
    }
    #[test]
    fn calibration_payload_bits_survive_surface_cache() {
        let mut source = test_surface(cell());
        source.batches[0].mesh.vertices[0].face_ao |=
            u32::from(vearth_gen::landmarks::CALIBRATION_MATERIAL_FLAG) << 5;
        let restored = decode(cell(), &encode(&source).unwrap()).unwrap();
        assert_eq!(
            restored.batches[0].mesh.vertices,
            source.batches[0].mesh.vertices
        );
    }

    #[test]
    fn construction_materials_roundtrip_surface_cache() {
        use vearth_gen::material::*;
        for material in [
            BRICK_WALL,
            STONE_WALL,
            PLASTER_WALL,
            TIMBER_WALL,
            TILE_ROOF,
            SLATE_ROOF,
            PANEL_WALL,
            METAL_ROOF,
            SIDING_WALL,
            SHINGLE_ROOF,
            FLAT_ROOF,
            THATCH_ROOF,
            NAMAKO_WALL,
            CITY_LIGHT,
            SCULPTURE_METAL,
            WEATHERED_LIMESTONE,
            VEINED_MARBLE,
            EARTHEN_PLASTER,
            CONCRETE_PATH,
            BRICK_PAVING,
            STONE_PAVING,
            GRAVEL_PATH,
            COBBLESTONE,
            STONE_SLAB,
        ]
        .into_iter()
        .chain(103..=118)
        {
            let mut source = test_surface(cell());
            for vertex in &mut source.batches[0].mesh.vertices {
                vertex.face_ao = (vertex.face_ao & !(255 << 5)) | (u32::from(material) << 5);
            }
            let restored =
                decode(cell(), &encode(&source).unwrap()).expect("valid construction material");
            assert_eq!(
                restored.batches[0].mesh.vertices,
                source.batches[0].mesh.vertices
            );
        }
    }

    #[test]
    fn reserved_materials_are_rejected_by_surface_cache() {
        for material in [0_u32, 96, 102] {
            let mut source = test_surface(cell());
            source.batches[0].mesh.vertices[0].face_ao = material << 5;
            assert!(decode(cell(), &encode(&source).unwrap()).is_none());
        }
    }
    #[test]
    fn surface_namespace_roundtrips_disk_and_never_persists_incomplete_cells() {
        let path = std::env::temp_dir().join(format!(
            "vearth-surface-cache-{}-{:?}",
            std::process::id(),
            vearth_platform::thread::current().id()
        ));
        let cache = ChunkCache::with_budgets(&path, "surface-test", 0, 16 * 1024 * 1024);
        let mut source = test_surface(cell());
        source.complete = false;
        cache.store(&source);
        assert!(cache.load(cell()).is_none());
        source.complete = true;
        cache.store(&source);
        cache.flush();
        assert!(cache.root.starts_with(path.join("surfaces")));
        let restored = cache.load(cell()).unwrap();
        assert_eq!(
            pack_surface_cell(restored)
                .unwrap()
                .compute_semantic_fingerprint(),
            pack_surface_cell(source)
                .unwrap()
                .compute_semantic_fingerprint()
        );
        drop(cache);
        let _ = std::fs::remove_dir_all(path);
    }
}
