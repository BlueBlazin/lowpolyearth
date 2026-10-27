//! Mapterhorn terrain tiles (Terrarium encoding) elevation source.
//!
//! Web Mercator z/x/y lossless WebP tiles, 512×512, RGB-encoded heights:
//! `h = (R·256 + G + B/256) − 32768` metres. The planet archive covers every
//! land tile through [`GLOBAL_MAX_ZOOM`] (Copernicus GLO-30 and regional
//! models) and omits all-sea tiles; finer tiles exist only where a national
//! elevation model does. A missing tile through [`GLOBAL_MAX_ZOOM`] is open
//! sea, and a missing finer tile is synthesized from its parent so that
//! sampling it matches sampling the parent.
//!
//! Mapterhorn carries no bathymetry: open sea is exactly 0 m. Decoding lowers
//! it to [`SEA_FLOOR_M`] so negative elevation keeps marking sea.
//!
//! Mercator only covers |lat| ≤ 85.05°; polar sampling clamps to the edge.
//!
//! Cache hierarchy: decoded-tile LRU in memory → indexed source cache →
//! network. Attribution: <https://mapterhorn.com/attribution>

use crate::local_tiles::{LocalTiles, SourceId};
use crate::source_cache::{CachedTile, SourceTileCache};
use crate::{DataError, ElevationSource, TileFetcher};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

pub const TILE_SIZE: usize = 512;
/// Finest zoom requested: 4.8 m pixels at the equator.
pub const MAX_ZOOM: u8 = 14;
/// The planet archive holds every land tile through this zoom.
pub const GLOBAL_MAX_ZOOM: u8 = 12;
const EQUATOR_M: f64 = 40_075_016.686;
/// Decoded tiles are ~1 MB each; 192 ≈ 200 MB ceiling.
const MEMORY_TILES: usize = 192;
/// Nominal floor for open sea, which Mapterhorn stores as exactly 0 m.
/// Water colour treats unknown depth as a coastal shelf, so only the sign
/// matters; the magnitude keeps sea floor clear of the sea-level surface.
pub const SEA_FLOOR_M: f32 = -30.0;

pub const DEFAULT_URL: &str = "https://tiles.mapterhorn.com";
/// Source-cache namespace shared by the app and offline tools. Mapterhorn
/// publishes versioned releases behind one tile endpoint; bump the revision
/// to re-fetch every tile after a release changes existing data.
pub const CACHE_PROVIDER: &str = "mapterhorn";
pub const CACHE_REVISION: &str = "mapterhorn-0.0.13";
pub const CACHE_CONTENT_TYPE: &str = "image/webp";
/// Regional archive index: which z6 regions hold tiles beyond
/// [`GLOBAL_MAX_ZOOM`], and how deep.
pub const COVERAGE_URL: &str = "https://download.mapterhorn.com/download_urls.json";

pub struct Tile {
    pub(crate) heights: Box<[f32]>, // TILE_SIZE² row-major
}

impl Tile {
    /// Row-major decoded heights, exposed for the offline terrain-pack tool.
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }
}

/// Cache slot with in-flight deduplication: concurrent misses on one tile
/// block on the slot while a single fetch runs, instead of each worker
/// downloading the same tile. `None` marks a failed fetch; the initializing
/// thread evicts the slot so a later sample retries.
type TileSlot = std::sync::OnceLock<Option<Arc<Tile>>>;

thread_local! {
    static LOCAL_TILES: std::cell::RefCell<LocalTiles<Tile>> =
        const { std::cell::RefCell::new(LocalTiles::new()) };
}

struct TileMap {
    tiles: HashMap<(u8, u32, u32), (Arc<TileSlot>, u64)>,
    clock: u64,
    /// Failure tombstones: when each failed tile may be retried. Without
    /// the TTL, every corner sample re-fetches a failing tile at the full
    /// blocking timeout — the per-sample retry storm that hung generation
    /// when an endpoint becomes unresponsive.
    failed: HashMap<(u8, u32, u32), vearth_platform::time::Instant>,
}

#[cfg(feature = "async-http")]
struct AsyncTileFlight {
    lock: tokio::sync::Mutex<()>,
    priority: crate::async_http::RequestPriority,
    participants: std::sync::atomic::AtomicUsize,
}

#[cfg(feature = "async-http")]
struct AsyncTileParticipant<'a> {
    source: &'a TerrariumSource,
    key: (u8, u32, u32),
    slot: Arc<TileSlot>,
    flight: Arc<AsyncTileFlight>,
}

#[cfg(feature = "async-http")]
impl Drop for AsyncTileParticipant<'_> {
    fn drop(&mut self) {
        let previous = self
            .flight
            .participants
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        debug_assert!(previous > 0);
        if previous != 1 || self.slot.get().is_some() {
            return;
        }
        // Async participants register under these same two locks before
        // either the slot or flight can escape. Rechecking the counter while
        // holding both closes the last-waiter/new-waiter race.
        let mut map = self.source.map.lock().unwrap_or_else(|e| e.into_inner());
        let mut flights = self
            .source
            .async_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self
            .flight
            .participants
            .load(std::sync::atomic::Ordering::Acquire)
            != 0
            || self.slot.get().is_some()
            || !map
                .tiles
                .get(&self.key)
                .is_some_and(|(slot, _)| Arc::ptr_eq(slot, &self.slot))
        {
            return;
        }
        let is_same_flight = flights
            .get(&self.key)
            .is_some_and(|flight| Arc::ptr_eq(flight, &self.flight));
        if is_same_flight {
            map.tiles.remove(&self.key);
            flights.remove(&self.key);
        }
    }
}

/// A failed tile is not retried more often than this. Async fetches already
/// retry stalls internally, so a failure here has exhausted those attempts.
const FAIL_TTL: vearth_platform::time::Duration = vearth_platform::time::Duration::from_secs(10);
const MAX_FAILURE_TOMBSTONES: usize = MEMORY_TILES;
#[cfg(feature = "async-http")]
/// Concurrent disk/network tile preparations, held through decoding and persistence.
/// The public bucket is latency-bound far from us-east-1 (about a second to
/// first byte, tens of KB/s per connection), so throughput scales with requests
/// in flight rather than bandwidth.
pub const ASYNC_TILE_LIMIT: usize = 96;
/// Terrarium tiles are static S3 objects; see [`crate::async_http::StallRetry`].
#[cfg(feature = "async-http")]
pub const ASYNC_STALL_RETRY: crate::async_http::StallRetry = crate::async_http::StallRetry {
    stall: vearth_platform::time::Duration::from_secs(4),
    attempts: 4,
};

pub struct TerrariumSource {
    /// Blocking transport used only by the legacy pipeline. Async-backed
    /// sources keep this empty so an unexpected CPU-lane cache miss fails
    /// fast and marks the cell incomplete instead of performing network I/O.
    fetcher: Option<Arc<dyn TileFetcher>>,
    #[cfg(feature = "async-http")]
    async_fetcher: Option<Arc<dyn crate::async_http::AsyncTileFetcher>>,
    url_base: String,
    source_cache: Option<SourceTileCache>,
    disk_metrics: crate::metrics::Acquisition,
    packed: Option<&'static crate::packed_terrain::PackedTerrain<'static>>,
    source_id: SourceId,
    map: Mutex<TileMap>,
    fetch_errs: std::sync::atomic::AtomicU64,
    /// Loaded at most once. `None` inside means it could not be read, and
    /// requests beyond the planet archive fall back to 404 handling.
    coverage: std::sync::OnceLock<Option<Arc<Coverage>>>,
    #[cfg(feature = "async-http")]
    async_coverage: tokio::sync::OnceCell<Option<Arc<Coverage>>>,
    #[cfg(feature = "async-http")]
    async_locks: Mutex<HashMap<(u8, u32, u32), Arc<AsyncTileFlight>>>,
    #[cfg(feature = "async-http")]
    async_admission: crate::async_http::PriorityGate,
}

/// Regions holding tiles beyond [`GLOBAL_MAX_ZOOM`]: z6 tile → finest zoom.
pub struct Coverage {
    regions: HashMap<(u32, u32), u8>,
}

impl Coverage {
    pub const REGION_ZOOM: u8 = 6;

    /// Parse Mapterhorn's `download_urls.json`; regional archives are named
    /// `{z}-{x}-{y}.pmtiles` after their z6 tile.
    pub fn parse(json: &[u8]) -> Result<Self, DataError> {
        let value: serde_json::Value =
            serde_json::from_slice(json).map_err(|error| DataError::Decode(error.to_string()))?;
        let items = value
            .get("items")
            .and_then(|items| items.as_array())
            .ok_or_else(|| DataError::Decode("coverage index has no items".into()))?;
        let mut regions = HashMap::new();
        for item in items {
            let Some(name) = item.get("name").and_then(|name| name.as_str()) else {
                continue;
            };
            let Some(stem) = name.strip_suffix(".pmtiles") else {
                continue;
            };
            let parts: Vec<_> = stem.split('-').collect();
            let [z, x, y] = parts.as_slice() else {
                continue;
            };
            let (Ok(z), Ok(x), Ok(y)) = (z.parse::<u8>(), x.parse::<u32>(), y.parse::<u32>())
            else {
                continue;
            };
            let Some(max_zoom) = item.get("max_zoom").and_then(|zoom| zoom.as_u64()) else {
                continue;
            };
            if z == Self::REGION_ZOOM {
                let max_zoom = u8::try_from(max_zoom).unwrap_or(u8::MAX);
                let finest = regions.entry((x, y)).or_insert(GLOBAL_MAX_ZOOM);
                *finest = (*finest).max(max_zoom);
            }
        }
        Ok(Self { regions })
    }

    /// Whether tile `key` can exist. Tiles through [`GLOBAL_MAX_ZOOM`]
    /// may (all-sea tiles are omitted); a finer tile needs a regional
    /// archive that deep.
    pub fn may_exist(&self, (z, x, y): (u8, u32, u32)) -> bool {
        if z <= GLOBAL_MAX_ZOOM {
            return true;
        }
        let shift = z - Self::REGION_ZOOM;
        self.regions
            .get(&(x >> shift, y >> shift))
            .is_some_and(|&finest| z <= finest)
    }
}

impl TerrariumSource {
    fn tombstone_failure(&self, key: (u8, u32, u32)) {
        let mut map = self.map.lock().unwrap();
        let mut removed: Vec<_> = map
            .failed
            .iter()
            .filter(|(_, at)| at.elapsed() >= FAIL_TTL)
            .map(|(key, _)| *key)
            .collect();
        for expired in &removed {
            map.failed.remove(expired);
            map.tiles.remove(expired);
        }
        if !map.failed.contains_key(&key)
            && map.failed.len() >= MAX_FAILURE_TOMBSTONES
            && let Some(oldest) = map
                .failed
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(key, _)| *key)
        {
            map.failed.remove(&oldest);
            map.tiles.remove(&oldest);
            removed.push(oldest);
        }
        map.failed
            .insert(key, vearth_platform::time::Instant::now());
        #[cfg(feature = "async-http")]
        if !removed.is_empty() {
            let mut flights = self.async_locks.lock().unwrap();
            for removed in removed {
                flights.remove(&removed);
            }
        }
    }

    pub fn new(
        fetcher: Arc<dyn TileFetcher>,
        url_base: impl Into<String>,
        source_cache: Option<SourceTileCache>,
    ) -> Self {
        TerrariumSource {
            fetcher: Some(fetcher),
            #[cfg(feature = "async-http")]
            async_fetcher: None,
            url_base: url_base.into(),
            source_cache,
            disk_metrics: crate::metrics::acquisition("provider.terrarium.disk"),
            packed: None,
            source_id: SourceId::new(),
            map: Mutex::new(TileMap {
                tiles: HashMap::new(),
                clock: 0,
                failed: HashMap::new(),
            }),
            fetch_errs: std::sync::atomic::AtomicU64::new(0),
            coverage: std::sync::OnceLock::new(),
            #[cfg(feature = "async-http")]
            async_coverage: tokio::sync::OnceCell::new(),
            #[cfg(feature = "async-http")]
            async_locks: Mutex::new(HashMap::new()),
            #[cfg(feature = "async-http")]
            async_admission: crate::async_http::PriorityGate::new(ASYNC_TILE_LIMIT),
        }
    }

    /// Cache-prepared source for the native async pipeline. The synchronous
    /// sampling view is deliberately network-disabled; `prepare` must make
    /// every dependency resident before CPU generation starts.
    #[cfg(feature = "async-http")]
    pub fn new_async(
        fetcher: Arc<dyn crate::async_http::AsyncTileFetcher>,
        url_base: impl Into<String>,
        source_cache: Option<SourceTileCache>,
    ) -> Self {
        TerrariumSource {
            fetcher: None,
            async_fetcher: Some(fetcher),
            url_base: url_base.into(),
            source_cache,
            disk_metrics: crate::metrics::acquisition("provider.terrarium.disk"),
            packed: None,
            source_id: SourceId::new(),
            map: Mutex::new(TileMap {
                tiles: HashMap::new(),
                clock: 0,
                failed: HashMap::new(),
            }),
            fetch_errs: std::sync::atomic::AtomicU64::new(0),
            coverage: std::sync::OnceLock::new(),
            async_coverage: tokio::sync::OnceCell::new(),
            async_locks: Mutex::new(HashMap::new()),
            async_admission: crate::async_http::PriorityGate::new(ASYNC_TILE_LIMIT),
        }
    }

    /// Use a known coverage index instead of fetching [`COVERAGE_URL`].
    pub fn with_coverage(self, coverage: Option<Coverage>) -> Self {
        let coverage = coverage.map(Arc::new);
        let _ = self.coverage.set(coverage.clone());
        #[cfg(feature = "async-http")]
        let _ = self.async_coverage.set(coverage);
        self
    }

    /// Coverage for blocking callers, fetched once through the tile fetcher.
    fn blocking_coverage(&self) -> Option<Arc<Coverage>> {
        self.coverage
            .get_or_init(|| {
                let fetcher = self.fetcher.as_ref()?;
                match fetcher
                    .fetch(COVERAGE_URL)
                    .and_then(|json| Coverage::parse(&json))
                {
                    Ok(coverage) => Some(Arc::new(coverage)),
                    Err(error) => {
                        log::warn!("terrain coverage index: {error}; relying on 404s");
                        None
                    }
                }
            })
            .clone()
    }

    #[cfg(feature = "async-http")]
    async fn async_coverage(&self) -> Option<Arc<Coverage>> {
        self.async_coverage
            .get_or_init(|| async {
                let fetcher = self.async_fetcher.as_ref()?;
                match fetcher
                    .fetch(COVERAGE_URL)
                    .await
                    .and_then(|json| Coverage::parse(&json))
                {
                    Ok(coverage) => Some(Arc::new(coverage)),
                    Err(error) => {
                        log::warn!("terrain coverage index: {error}; relying on 404s");
                        None
                    }
                }
            })
            .await
            .clone()
    }

    /// Zoom whose ground resolution best matches `sample_pitch_m` at `lat`.
    pub fn zoom_for(sample_pitch_m: f64, lat: f64) -> u8 {
        let m_per_px_z0 = EQUATOR_M / TILE_SIZE as f64 * lat.cos().max(0.05);
        let z = (m_per_px_z0 / sample_pitch_m.max(0.01)).log2().ceil();
        (z.max(0.0) as u8).min(MAX_ZOOM)
    }

    /// Use the target-specific terrain pack before consulting disk or the
    /// network. Zooms above the pack's ceiling are unaffected.
    pub fn with_builtin_terrain(mut self) -> Self {
        self.packed = Some(crate::packed_terrain::builtin());
        self
    }

    fn packed_tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Tile>, DataError> {
        let Some(pack) = self.packed else {
            return Ok(None);
        };
        pack.decode_tile(z, x, y)
            .map(|heights| heights.map(|heights| Tile { heights }))
    }

    fn slot_for(&self, key: (u8, u32, u32)) -> Option<Arc<TileSlot>> {
        let mut map = self.map.lock().unwrap();
        // Failure tombstone: answer None instantly until the retry TTL
        // expires (see TileMap::failed).
        if let Some(at) = map.failed.get(&key) {
            if at.elapsed() < FAIL_TTL {
                return None;
            }
            map.failed.remove(&key);
            map.tiles.remove(&key); // drop the memoized-None slot
        }
        map.clock += 1;
        let clock = map.clock;
        if let Some(entry) = map.tiles.get_mut(&key) {
            entry.1 = clock;
            Some(entry.0.clone())
        } else {
            if map.tiles.len() >= MEMORY_TILES {
                // Evict ~1/8 of the least recently used in one sweep.
                let mut by_age: Vec<_> = map
                    .tiles
                    .iter()
                    // In-flight work and acquired CPU dependencies keep their
                    // keyed slots. Only unused completed slots are evictable;
                    // active jobs may temporarily exceed the soft cap.
                    .filter(|(_, (slot, _))| {
                        slot.get()
                            .is_some_and(|tile| tile.is_none() || Arc::strong_count(slot) == 1)
                    })
                    .map(|(k, (_, at))| (*at, *k))
                    .collect();
                by_age.sort_unstable();
                let victims: Vec<_> = by_age
                    .iter()
                    .take(MEMORY_TILES / 8)
                    .map(|(_, k)| *k)
                    .collect();
                for k in victims {
                    map.tiles.remove(&k);
                    #[cfg(feature = "async-http")]
                    self.async_locks.lock().unwrap().remove(&k);
                }
            }
            let slot = Arc::new(TileSlot::new());
            map.tiles.insert(key, (slot.clone(), clock));
            Some(slot)
        }
    }

    #[cfg(feature = "async-http")]
    fn async_participant(&self, key: (u8, u32, u32)) -> Option<AsyncTileParticipant<'_>> {
        let mut map = self.map.lock().unwrap();
        if let Some(at) = map.failed.get(&key) {
            if at.elapsed() < FAIL_TTL {
                return None;
            }
            map.failed.remove(&key);
            map.tiles.remove(&key);
        }
        map.clock += 1;
        let clock = map.clock;
        let mut flights = self.async_locks.lock().unwrap();
        let slot = if let Some(entry) = map.tiles.get_mut(&key) {
            entry.1 = clock;
            entry.0.clone()
        } else {
            if map.tiles.len() >= MEMORY_TILES {
                let mut by_age: Vec<_> = map
                    .tiles
                    .iter()
                    .filter(|(_, (slot, _))| {
                        slot.get()
                            .is_some_and(|tile| tile.is_none() || Arc::strong_count(slot) == 1)
                    })
                    .map(|(key, (_, used_at))| (*used_at, *key))
                    .collect();
                by_age.sort_unstable();
                for (_, victim) in by_age.into_iter().take(MEMORY_TILES / 8) {
                    map.tiles.remove(&victim);
                    flights.remove(&victim);
                }
            }
            let slot = Arc::new(TileSlot::new());
            map.tiles.insert(key, (slot.clone(), clock));
            slot
        };
        let flight = flights
            .entry(key)
            .or_insert_with(|| {
                Arc::new(AsyncTileFlight {
                    lock: tokio::sync::Mutex::new(()),
                    priority: crate::async_http::RequestPriority::shared(),
                    participants: std::sync::atomic::AtomicUsize::new(0),
                })
            })
            .clone();
        flight
            .participants
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        Some(AsyncTileParticipant {
            source: self,
            key,
            slot,
            flight,
        })
    }

    fn with_sampling_tile<R>(
        &self,
        key: (u8, u32, u32),
        read: impl FnOnce(&Tile) -> R,
    ) -> Option<R> {
        let mut read = Some(read);
        if let Some(value) = LOCAL_TILES.with(|local| {
            local
                .borrow_mut()
                .with(self.source_id, key, |tile| read.take().unwrap()(tile))
        }) {
            return Some(value);
        }
        let tile = self.sampling_tile(key)?;
        Some(read.unwrap()(&tile))
    }

    fn sampling_tile(&self, key: (u8, u32, u32)) -> Option<Arc<Tile>> {
        if let Some(tile) = LOCAL_TILES.with(|local| local.borrow_mut().get(self.source_id, key)) {
            return Some(tile);
        }
        let tile = if self.fetcher.is_some() {
            self.tile(key.0, key.1, key.2)
        } else {
            self.cached_tile(key)
        }?;
        let bytes = std::mem::size_of_val(tile.heights.as_ref());
        if !LocalTiles::<Tile>::can_retain(bytes) {
            return Some(tile);
        }
        let map = self.map.lock().unwrap();
        if let Some((slot, _)) = map.tiles.get(&key)
            && slot
                .get()
                .and_then(Option::as_ref)
                .is_some_and(|resident| Arc::ptr_eq(resident, &tile))
        {
            LOCAL_TILES.with(|local| {
                local.borrow_mut().insert(self.source_id, key, slot, bytes);
            });
        }
        Some(tile)
    }

    fn cached_tile(&self, key: (u8, u32, u32)) -> Option<Arc<Tile>> {
        let mut map = self.map.lock().unwrap();
        map.clock += 1;
        let clock = map.clock;
        let entry = map.tiles.get_mut(&key)?;
        entry.1 = clock;
        entry.0.get()?.clone()
    }

    fn tile(&self, z: u8, x: u32, y: u32) -> Option<Arc<Tile>> {
        let key = (z, x, y);
        let slot = self.slot_for(key)?;
        // Fetch + decode outside the lock; concurrent callers for the same
        // tile block here instead of fetching duplicates.
        let mut initialized_here = false;
        let got = slot
            .get_or_init(|| {
                initialized_here = true;
                let decoded = match self.packed_tile(z, x, y) {
                    Ok(Some(tile)) => Ok(tile),
                    Ok(None) => self.source_tile(key),
                    Err(error) => {
                        log::warn!("packed terrain tile {z}/{x}/{y}: {error}; using source");
                        self.source_tile(key)
                    }
                };
                match decoded {
                    Ok(t) => Some(Arc::new(t)),
                    Err(e) => {
                        log::warn!("terrain tile {z}/{x}/{y}: {e}");
                        self.fetch_errs
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        None
                    }
                }
            })
            .clone();
        if initialized_here && got.is_none() {
            // Keep the memoized-None slot and tombstone it; the TTL check
            // above owns the retry (evicting immediately made every later
            // sample refetch a failing tile back-to-back).
            self.tombstone_failure(key);
        }
        got
    }

    /// Decode a tile from disk or the network. A tile the source does not
    /// have is open sea through [`GLOBAL_MAX_ZOOM`] and its parent's
    /// quadrant beyond it.
    fn source_tile(&self, key: (u8, u32, u32)) -> Result<Tile, DataError> {
        let (z, x, y) = key;
        if z > GLOBAL_MAX_ZOOM
            && self
                .blocking_coverage()
                .is_some_and(|coverage| !coverage.may_exist(key))
        {
            return self.synthesized_tile(key);
        }
        match self.tile_bytes(z, x, y) {
            Ok(bytes) => decode_terrarium(&bytes),
            Err(DataError::NotAvailable) if z > GLOBAL_MAX_ZOOM => self.synthesized_tile(key),
            Err(DataError::NotAvailable) => Ok(sea_tile()),
            Err(error) => Err(error),
        }
    }

    fn synthesized_tile(&self, (z, x, y): (u8, u32, u32)) -> Result<Tile, DataError> {
        let parent = self
            .tile(z - 1, x / 2, y / 2)
            .ok_or(DataError::NotAvailable)?;
        Ok(upsample_quadrant(
            &parent,
            (x % 2) as usize,
            (y % 2) as usize,
        ))
    }

    /// Raw tile bytes; `NotAvailable` when the source has no such tile.
    fn tile_bytes(&self, z: u8, x: u32, y: u32) -> Result<Vec<u8>, DataError> {
        if let Some(cache) = &self.source_cache {
            match cache.load(z, x, y) {
                Ok(Some(CachedTile::Present(bytes))) => {
                    self.disk_metrics.cache_hit(bytes.len());
                    return Ok(bytes);
                }
                Ok(Some(CachedTile::Missing)) => {
                    self.disk_metrics.cache_hit(0);
                    return Err(DataError::NotAvailable);
                }
                Ok(None) => self.disk_metrics.cache_miss(),
                Err(error) => {
                    self.disk_metrics.cache_miss();
                    log::warn!("terrain source cache read {z}/{x}/{y}: {error}");
                }
            }
        }
        let fetched = self
            .fetcher
            .as_ref()
            .ok_or(DataError::NotAvailable)?
            .fetch(&format!("{}/{z}/{x}/{y}.webp", self.url_base));
        if let Some(cache) = &self.source_cache {
            match &fetched {
                Ok(bytes) => {
                    let _ = cache.store_present(z, x, y, bytes.clone());
                }
                Err(DataError::NotAvailable) => {
                    let _ = cache.store_missing(z, x, y);
                }
                Err(_) => {}
            }
        }
        fetched
    }

    #[cfg(feature = "async-http")]
    fn planned_tiles(&self, request: &crate::async_prep::PrepareRequest) -> Vec<(u8, u32, u32)> {
        let mut keys: std::collections::BTreeSet<_> =
            request.terrain_tiles.iter().copied().collect();
        let footprint = request.footprint;
        let lat_min = (footprint.center_lat - footprint.radius_rad).max(-1.484_422);
        let lat_max = (footprint.center_lat + footprint.radius_rad).min(1.484_422);
        let mut zoom_samples = vec![lat_min, footprint.center_lat, lat_max];
        if lat_min <= 0.0 && lat_max >= 0.0 {
            zoom_samples.push(0.0);
        }
        let zooms: Vec<_> = zoom_samples
            .into_iter()
            .map(|lat| Self::zoom_for(footprint.sample_hint_m, lat))
            .collect();
        let z_min = *zooms.iter().min().unwrap();
        let z_max = *zooms.iter().max().unwrap();
        let min_cos = lat_min.cos().abs().min(lat_max.cos().abs()).max(0.05);
        // zoom_for varies continuously before rounding, so a latitude span
        // can use every integer zoom between its extrema.
        for z in z_min..=z_max {
            let n = 1u32 << z;
            let pixels_n = TILE_SIZE as f64 * n as f64;
            let (_, py_north) = mercator_px(lat_max, footprint.center_lon, z);
            let (_, py_south) = mercator_px(lat_min, footprint.center_lon, z);
            let y_north = (py_north.min(py_south) / TILE_SIZE as f64).floor() as u32;
            // Bilinear sampling reaches ceil(pixel), not the next whole tile.
            // An integral endpoint has no positive-weight outer tap.
            let y_south = (py_north.max(py_south).ceil().min(pixels_n - 1.0) / TILE_SIZE as f64)
                .floor() as u32;
            let lon_radius = (footprint.radius_rad / min_cos).min(std::f64::consts::PI);
            let xs: Vec<u32> = if lon_radius >= std::f64::consts::PI {
                (0..n).collect()
            } else {
                // Keep the endpoints unwrapped until after pixel/tile rounding.
                // This preserves the full eastward interval even when both
                // endpoints alias to the same tile across the antimeridian.
                let pixel_x = |lon: f64| (lon.to_degrees() + 180.0) / 360.0 * pixels_n;
                let west =
                    (pixel_x(footprint.center_lon - lon_radius) / TILE_SIZE as f64).floor() as i64;
                let east = (pixel_x(footprint.center_lon + lon_radius).ceil() / TILE_SIZE as f64)
                    .floor() as i64;
                let count = (east - west + 1).min(n as i64);
                (0..count)
                    .map(|step| (west + step).rem_euclid(n as i64) as u32)
                    .collect()
            };
            for y in y_north..=y_south {
                for &x in &xs {
                    keys.insert((z, x, y));
                }
            }
        }
        keys.into_iter().collect()
    }

    #[cfg(feature = "async-http")]
    fn prepare_retained_tile(
        &self,
        key: (u8, u32, u32),
        priority: f64,
    ) -> crate::async_prep::RetainedPrepareFuture<'_> {
        // Registration and slot identity are one atomic operation. The future
        // owns this participant even before its first poll, so cancellation of
        // a prior waiter cannot unlink the slot that this acquisition retains.
        let participant = self.async_participant(key);
        Box::pin(async move {
            let mut prepared = crate::async_prep::PreparedSources::default();
            let Some(participant) = participant else {
                prepared
                    .summary
                    .record(crate::async_prep::DependencyOutcome::TransientFailure);
                return prepared;
            };
            prepared
                .summary
                .record(self.prepare_tile(&participant, priority).await);
            if participant.slot.get().is_some_and(Option::is_some) {
                prepared.retain(participant.slot.clone());
            }
            prepared
        })
    }

    #[cfg(feature = "async-http")]
    async fn prepare_tile(
        &self,
        participant: &AsyncTileParticipant<'_>,
        priority: f64,
    ) -> crate::async_prep::DependencyOutcome {
        use crate::async_prep::DependencyOutcome;

        let key = participant.key;
        let slot = participant.slot.clone();
        if let Some(got) = slot.get() {
            return if got.is_some() {
                DependencyOutcome::Ready
            } else {
                DependencyOutcome::TransientFailure
            };
        }

        match self.packed_tile(key.0, key.1, key.2) {
            Ok(Some(tile)) => {
                let _ = slot.set(Some(Arc::new(tile)));
                return DependencyOutcome::Ready;
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!(
                    "packed terrain tile {}/{}/{}: {error}; using source",
                    key.0,
                    key.1,
                    key.2
                );
            }
        }

        let _priority = participant.flight.priority.lease(priority);
        let _leader = participant.flight.lock.lock().await;
        if let Some(got) = slot.get() {
            return if got.is_some() {
                DependencyOutcome::Ready
            } else {
                DependencyOutcome::TransientFailure
            };
        }
        let (z, x, y) = key;
        if z > GLOBAL_MAX_ZOOM
            && self
                .async_coverage()
                .await
                .is_some_and(|coverage| !coverage.may_exist(key))
        {
            return self.fill_synthesized(participant, priority).await;
        }
        let source_permit = self
            .async_admission
            .acquire(participant.flight.priority.clone())
            .await;

        if let Some(cache) = self.source_cache.clone() {
            let read_cache = cache.clone();
            match vearth_platform::rt::task::spawn_blocking(move || read_cache.load(z, x, y)).await
            {
                Ok(Ok(Some(CachedTile::Present(bytes)))) => {
                    self.disk_metrics.cache_hit(bytes.len());
                    let decoded =
                        vearth_platform::rt::task::spawn_blocking(move || decode_terrarium(&bytes))
                            .await;
                    if let Ok(Ok(tile)) = decoded {
                        let _ = slot.set(Some(Arc::new(tile)));
                        return DependencyOutcome::Ready;
                    }
                    let _ = vearth_platform::rt::task::spawn_blocking(move || {
                        cache.remove_corrupt(z, x, y)
                    })
                    .await;
                }
                Ok(Ok(Some(CachedTile::Missing))) => {
                    self.disk_metrics.cache_hit(0);
                    drop(source_permit);
                    return self.fill_absent(participant, priority).await;
                }
                Ok(Ok(None)) => {
                    self.disk_metrics.cache_miss();
                }
                Ok(Err(error)) => {
                    self.disk_metrics.cache_miss();
                    log::warn!("terrain source cache read {z}/{x}/{y}: {error}");
                }
                Err(error) => {
                    self.disk_metrics.cache_miss();
                    log::warn!("terrain source cache task {z}/{x}/{y}: {error}");
                }
            }
        }

        let Some(fetcher) = &self.async_fetcher else {
            return DependencyOutcome::TransientFailure;
        };
        let url = format!("{}/{z}/{x}/{y}.webp", self.url_base);
        let bytes = match fetcher
            .fetch_with_priority(&url, participant.flight.priority.clone())
            .await
        {
            Ok(bytes) => bytes,
            Err(DataError::NotAvailable) => {
                if let Some(cache) = self.source_cache.clone() {
                    let _ = vearth_platform::rt::task::spawn_blocking(move || {
                        cache.store_missing(z, x, y)
                    })
                    .await;
                }
                drop(source_permit);
                return self.fill_absent(participant, priority).await;
            }
            Err(error) => {
                log::warn!("terrain tile {z}/{x}/{y}: {error}");
                self.fetch_errs
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let _ = slot.set(None);
                self.tombstone_failure(key);
                return DependencyOutcome::TransientFailure;
            }
        };
        let persist = bytes.clone();
        let decoded =
            vearth_platform::rt::task::spawn_blocking(move || decode_terrarium(&bytes)).await;
        match decoded {
            Ok(Ok(tile)) => {
                if let Some(cache) = self.source_cache.clone() {
                    let _ = vearth_platform::rt::task::spawn_blocking(move || {
                        cache.store_present(z, x, y, persist)
                    })
                    .await;
                }
                let _ = slot.set(Some(Arc::new(tile)));
                DependencyOutcome::Ready
            }
            Ok(Err(error)) => {
                log::warn!("terrain tile {z}/{x}/{y}: {error}");
                self.fetch_errs
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let _ = slot.set(None);
                self.tombstone_failure(key);
                DependencyOutcome::TransientFailure
            }
            Err(error) => {
                log::warn!("terrain tile {z}/{x}/{y} decode task: {error}");
                self.fetch_errs
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let _ = slot.set(None);
                self.tombstone_failure(key);
                DependencyOutcome::TransientFailure
            }
        }
    }

    /// Fill a tile the source does not have, as [`Self::source_tile`] does.
    #[cfg(feature = "async-http")]
    async fn fill_absent(
        &self,
        participant: &AsyncTileParticipant<'_>,
        priority: f64,
    ) -> crate::async_prep::DependencyOutcome {
        if participant.key.0 > GLOBAL_MAX_ZOOM {
            return self.fill_synthesized(participant, priority).await;
        }
        let _ = participant.slot.set(Some(Arc::new(sea_tile())));
        crate::async_prep::DependencyOutcome::Ready
    }

    /// Fill a tile the source does not have from its prepared parent.
    #[cfg(feature = "async-http")]
    fn fill_synthesized<'a>(
        &'a self,
        participant: &'a AsyncTileParticipant<'a>,
        priority: f64,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = crate::async_prep::DependencyOutcome> + Send + 'a>,
    > {
        use crate::async_prep::DependencyOutcome;
        Box::pin(async move {
            let (z, x, y) = participant.key;
            let parent = self.async_participant((z - 1, x / 2, y / 2));
            let (outcome, parent_tile) = match &parent {
                Some(parent) => {
                    let outcome = self.prepare_tile(parent, priority).await;
                    (outcome, parent.slot.get().cloned().flatten())
                }
                None => (DependencyOutcome::TransientFailure, None),
            };
            drop(parent);
            let tile = match parent_tile {
                Some(parent) => vearth_platform::rt::task::spawn_blocking(move || {
                    upsample_quadrant(&parent, (x % 2) as usize, (y % 2) as usize)
                })
                .await
                .ok(),
                None => None,
            };
            match tile {
                Some(tile) => {
                    let _ = participant.slot.set(Some(Arc::new(tile)));
                    DependencyOutcome::Ready
                }
                None => {
                    let _ = participant.slot.set(None);
                    self.tombstone_failure(participant.key);
                    match outcome {
                        DependencyOutcome::Ready => DependencyOutcome::TransientFailure,
                        failure => failure,
                    }
                }
            }
        })
    }
}

#[cfg(feature = "async-http")]
impl crate::async_prep::AsyncSourcePreparer for TerrariumSource {
    fn prepare<'a>(
        &'a self,
        request: &'a crate::async_prep::PrepareRequest,
    ) -> crate::async_prep::PrepareFuture<'a> {
        Box::pin(async move {
            self.prepare_retained_for_coverage(
                request,
                crate::async_prep::PreparationCoverage::Global,
            )
            .await
            .summary
        })
    }

    fn prepare_retained_for_coverage<'a>(
        &'a self,
        request: &'a crate::async_prep::PrepareRequest,
        _coverage: crate::async_prep::PreparationCoverage,
    ) -> crate::async_prep::RetainedPrepareFuture<'a> {
        Box::pin(async move {
            crate::async_prep::join_retained(
                self.planned_tiles(request)
                    .into_iter()
                    .map(|key| self.prepare_retained_tile(key, request.priority))
                    .collect(),
            )
            .await
        })
    }
}

/// Decode a Terrarium tile from lossless WebP or PNG, marking open sea.
pub fn decode_terrarium(bytes: &[u8]) -> Result<Tile, DataError> {
    let decode = |error: &dyn std::fmt::Display| DataError::Decode(error.to_string());
    let (width, height, stride, pixels) =
        if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
            let mut decoder =
                image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|e| decode(&e))?;
            if decoder.is_lossy() {
                return Err(DataError::Decode(
                    "lossy WebP cannot carry Terrarium heights".into(),
                ));
            }
            let (width, height) = decoder.dimensions();
            let stride = if decoder.has_alpha() { 4 } else { 3 };
            let size = decoder
                .output_buffer_size()
                .ok_or_else(|| DataError::Decode("WebP tile too large".into()))?;
            let mut pixels = vec![0u8; size];
            decoder.read_image(&mut pixels).map_err(|e| decode(&e))?;
            (width, height, stride, pixels)
        } else {
            let mut reader = png::Decoder::new(Cursor::new(bytes))
                .read_info()
                .map_err(|e| decode(&e))?;
            let mut pixels = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
            let info = reader.next_frame(&mut pixels).map_err(|e| decode(&e))?;
            let stride = match info.color_type {
                png::ColorType::Rgb => 3,
                png::ColorType::Rgba => 4,
                other => return Err(DataError::Decode(format!("color type {other:?}"))),
            };
            (info.width, info.height, stride, pixels)
        };
    if width as usize != TILE_SIZE || height as usize != TILE_SIZE {
        return Err(DataError::Decode(format!(
            "unexpected tile dims {width}x{height}"
        )));
    }
    let mut heights = vec![0f32; TILE_SIZE * TILE_SIZE].into_boxed_slice();
    for (i, h) in heights.iter_mut().enumerate() {
        let p = i * stride;
        let (r, g, b) = (
            f32::from(pixels[p]),
            f32::from(pixels[p + 1]),
            f32::from(pixels[p + 2]),
        );
        *h = r * 256.0 + g + b / 256.0 - 32768.0;
    }
    mark_sea(&mut heights);
    Ok(Tile { heights })
}

/// Open sea: the planet archive omits tiles with no land.
pub fn sea_tile() -> Tile {
    Tile {
        heights: vec![SEA_FLOOR_M; TILE_SIZE * TILE_SIZE].into_boxed_slice(),
    }
}

/// Lower Mapterhorn's exactly-zero open sea to [`SEA_FLOOR_M`]. A zero
/// pixel counts as sea only beside another zero pixel in the tile, so an
/// isolated land sample at 0 m stays land.
fn mark_sea(heights: &mut [f32]) {
    let n = TILE_SIZE;
    let sea: Vec<bool> = (0..heights.len())
        .map(|i| {
            let (x, y) = (i % n, i / n);
            heights[i] == 0.0
                && ((x > 0 && heights[i - 1] == 0.0)
                    || (x + 1 < n && heights[i + 1] == 0.0)
                    || (y > 0 && heights[i - n] == 0.0)
                    || (y + 1 < n && heights[i + n] == 0.0))
        })
        .collect();
    for (height, sea) in heights.iter_mut().zip(sea) {
        if sea {
            *height = SEA_FLOOR_M;
        }
    }
}

/// Child tile covering quadrant `(qx, qy)` of `parent`, one zoom finer.
///
/// Heights sit at pixel corners and sampling is bilinear, so the child's
/// samples at half-parent-pixel steps reproduce the parent's surface exactly
/// within each parent cell. The last half pixel of the parent's east and
/// south edges holds the edge value, as sampling does without a neighbour.
pub(crate) fn upsample_quadrant(parent: &Tile, qx: usize, qy: usize) -> Tile {
    let half = TILE_SIZE / 2;
    let axis = |q: usize, i: usize| {
        let position = q * half * 2 + i;
        let low = position / 2;
        let high = (low + 1).min(TILE_SIZE - 1);
        let fraction = if position % 2 == 1 && high != low {
            0.5
        } else {
            0.0
        };
        (low, high, fraction)
    };
    let mut heights = vec![0f32; TILE_SIZE * TILE_SIZE].into_boxed_slice();
    for row in 0..TILE_SIZE {
        let (y0, y1, fy) = axis(qy, row);
        for column in 0..TILE_SIZE {
            let (x0, x1, fx) = axis(qx, column);
            let at = |x: usize, y: usize| parent.heights[y * TILE_SIZE + x];
            let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
            let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
            heights[row * TILE_SIZE + column] = top + (bottom - top) * fy;
        }
    }
    Tile { heights }
}

/// The tile containing lat/lon (radians) at zoom z, with the same wrap and
/// clamp semantics as sampling — the contract for prefetch prediction.
pub fn tile_at(lat: f64, lon: f64, z: u8) -> (u32, u32) {
    let (px, py) = mercator_px(lat, lon, z);
    let tx = (px / TILE_SIZE as f64).floor() as u32;
    let ty = (py / TILE_SIZE as f64).floor() as u32;
    (tx, ty)
}

/// lat/lon (radians) → continuous Web Mercator pixel coordinates at zoom z.
fn mercator_px(lat: f64, lon: f64, z: u8) -> (f64, f64) {
    let n = (TILE_SIZE as f64) * (1u64 << z) as f64;
    let x = ((lon.to_degrees() + 180.0) / 360.0 * n).rem_euclid(n);
    // Clamp to the Mercator limit; poles read the edge row.
    let lat_c = lat.clamp(-1.484_422, 1.484_422); // ±85.05113°
    let y = (1.0 - ((lat_c.tan() + 1.0 / lat_c.cos()).ln()) / std::f64::consts::PI) / 2.0 * n;
    // rem_euclid can round a tiny negative input up to n; n and zero are
    // the same longitude, but only zero belongs to a valid pixel/tile.
    (if x < n { x } else { 0.0 }, y.clamp(0.0, n - 1.0))
}

struct BilinearFootprint {
    tile_x: [u32; 2],
    tile_y: [u32; 2],
    pixel_x: [usize; 2],
    pixel_y: [usize; 2],
    fraction: [f32; 2],
}

impl BilinearFootprint {
    fn at(lat: f64, lon: f64, z: u8) -> Self {
        let (px, py) = mercator_px(lat, lon, z);
        let pixels_n = TILE_SIZE as u64 * (1u64 << z);
        let (x0, y0) = (px.floor() as u64, py.floor() as u64);
        let x = [x0, (x0 + 1) % pixels_n];
        let y = [y0, (y0 + 1).min(pixels_n - 1)];
        Self {
            tile_x: x.map(|x| (x / TILE_SIZE as u64) as u32),
            tile_y: y.map(|y| (y / TILE_SIZE as u64) as u32),
            pixel_x: x.map(|x| (x % TILE_SIZE as u64) as usize),
            pixel_y: y.map(|y| (y % TILE_SIZE as u64) as usize),
            fraction: [(px - x0 as f64) as f32, (py - y0 as f64) as f32],
        }
    }

    fn interpolate(&self, height: impl Fn(usize, usize) -> f32) -> f32 {
        let [ax, ay] = self.fraction;
        height(0, 0) * (1.0 - ax) * (1.0 - ay)
            + height(1, 0) * ax * (1.0 - ay)
            + height(0, 1) * (1.0 - ax) * ay
            + height(1, 1) * ax * ay
    }
}

/// Distinct tiles used by bilinear sampling at lat/lon (radians), including
/// the containing tile for fallback. Zero-weight outer taps are omitted;
/// prediction and sampling share longitude wrapping and polar edge clamping.
pub fn sampling_tiles(lat: f64, lon: f64, z: u8) -> Vec<(u8, u32, u32)> {
    let footprint = BilinearFootprint::at(lat, lon, z);
    let mut keys = Vec::with_capacity(4);
    for iy in 0..=usize::from(footprint.fraction[1] > 0.0) {
        for ix in 0..=usize::from(footprint.fraction[0] > 0.0) {
            let key = (z, footprint.tile_x[ix], footprint.tile_y[iy]);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// Bilinear terrarium sampling over any tile lookup, including
/// fetch-through-cache and pre-populated memory sources. Missing neighbour
/// taps clamp to the containing tile's edge; a missing containing tile is None.
pub fn sample_elevation<F>(lat: f64, lon: f64, sample_pitch_m: f64, lookup: F) -> Option<f32>
where
    F: Fn(u8, u32, u32) -> Option<Arc<Tile>>,
{
    let z = TerrariumSource::zoom_for(sample_pitch_m, lat);
    let footprint = BilinearFootprint::at(lat, lon, z);
    let [tx, east] = footprint.tile_x;
    let [ty, south] = footprint.tile_y;
    let [ax, ay] = footprint.fraction;
    let tile = lookup(z, tx, ty)?;
    let east_tile = (east != tx && ax > 0.0)
        .then(|| lookup(z, east, ty))
        .flatten();
    let south_tile = (south != ty && ay > 0.0)
        .then(|| lookup(z, tx, south))
        .flatten();
    let southeast_tile = (east != tx && south != ty && ax > 0.0 && ay > 0.0)
        .then(|| lookup(z, east, south))
        .flatten();
    let h = |ix: usize, iy: usize| {
        let crosses_x = footprint.tile_x[ix] != tx;
        let crosses_y = footprint.tile_y[iy] != ty;
        let source = match (crosses_x, crosses_y) {
            (false, false) => Some(&tile),
            (true, false) => east_tile.as_ref(),
            (false, true) => south_tile.as_ref(),
            (true, true) => southeast_tile.as_ref(),
        };
        if let Some(source) = source {
            source.heights[footprint.pixel_y[iy] * TILE_SIZE + footprint.pixel_x[ix]]
        } else {
            let x = if crosses_x {
                TILE_SIZE - 1
            } else {
                footprint.pixel_x[ix]
            };
            let y = if crosses_y {
                TILE_SIZE - 1
            } else {
                footprint.pixel_y[iy]
            };
            tile.heights[y * TILE_SIZE + x]
        }
    };
    Some(footprint.interpolate(h))
}

impl ElevationSource for TerrariumSource {
    fn elevation_m(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> f32 {
        self.sample_elevation(lat, lon, sample_pitch_m)
            .value
            .unwrap_or(0.0)
    }

    fn sample_elevation(
        &self,
        lat: f64,
        lon: f64,
        sample_pitch_m: f64,
    ) -> crate::SourceSample<f32> {
        let complete = std::cell::Cell::new(true);
        let z = Self::zoom_for(sample_pitch_m, lat);
        let footprint = BilinearFootprint::at(lat, lon, z);
        let value = if footprint.tile_x[0] == footprint.tile_x[1]
            && footprint.tile_y[0] == footprint.tile_y[1]
        {
            let value =
                self.with_sampling_tile((z, footprint.tile_x[0], footprint.tile_y[0]), |tile| {
                    footprint.interpolate(|ix, iy| {
                        tile.heights[footprint.pixel_y[iy] * TILE_SIZE + footprint.pixel_x[ix]]
                    })
                });
            complete.set(value.is_some());
            value
        } else {
            sample_elevation(lat, lon, sample_pitch_m, |z, x, y| {
                let tile = self.sampling_tile((z, x, y));
                if tile.is_none() {
                    complete.set(false);
                }
                tile
            })
        };
        if !complete.get() && self.fetcher.is_none() {
            self.fetch_errs
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        crate::SourceSample {
            value,
            complete: complete.get(),
        }
    }

    fn fetch_errors(&self) -> u64 {
        self.fetch_errs.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Fetch all missing tiles concurrently. Tiles that are cached or already
    /// being fetched by another worker spawn nothing.
    fn prefetch(&self, tiles: &[(u8, u32, u32)]) {
        {
            let missing: Vec<_> = {
                let map = self.map.lock().unwrap();
                tiles
                    .iter()
                    .copied()
                    .filter(|k| !map.tiles.contains_key(k))
                    .collect()
            };
            if missing.is_empty() {
                return;
            }
            vearth_platform::thread::scope(|s| {
                for &(z, x, y) in &missing {
                    s.spawn(move || {
                        let _ = self.tile(z, x, y);
                    });
                }
            });
        }
    }
}

/// Pre-populated tile store for callers that prepare raw PNGs before generation.
/// Missing tiles sample as 0 m rather than blocking.
pub struct MemoryTiles {
    map: std::sync::RwLock<HashMap<(u8, u32, u32), Arc<Tile>>>,
    packed: Option<&'static crate::packed_terrain::PackedTerrain<'static>>,
}

impl Default for MemoryTiles {
    fn default() -> Self {
        Self {
            map: std::sync::RwLock::new(HashMap::new()),
            packed: None,
        }
    }
}

impl MemoryTiles {
    pub fn with_builtin_terrain() -> Self {
        Self {
            map: std::sync::RwLock::new(HashMap::new()),
            packed: Some(crate::packed_terrain::builtin()),
        }
    }

    pub fn insert_png(&self, z: u8, x: u32, y: u32, png_bytes: &[u8]) -> Result<(), DataError> {
        let tile = Arc::new(decode_terrarium(png_bytes)?);
        self.map.write().unwrap().insert((z, x, y), tile);
        Ok(())
    }

    pub fn contains(&self, z: u8, x: u32, y: u32) -> bool {
        self.map.read().unwrap().contains_key(&(z, x, y))
            || self.packed.is_some_and(|pack| pack.contains(z, x, y))
    }

    pub fn len(&self) -> usize {
        self.map.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn tile(&self, z: u8, x: u32, y: u32) -> Option<Arc<Tile>> {
        let key = (z, x, y);
        if let Some(tile) = self.map.read().unwrap().get(&key).cloned() {
            return Some(tile);
        }
        let packed = self.packed?;
        let heights = match packed.decode_tile(z, x, y) {
            Ok(Some(heights)) => heights,
            Ok(None) => return None,
            Err(error) => {
                log::warn!("packed terrain tile {z}/{x}/{y}: {error}");
                return None;
            }
        };
        let tile = Arc::new(Tile { heights });
        let mut map = self.map.write().unwrap();
        Some(map.entry(key).or_insert_with(|| tile.clone()).clone())
    }
}

impl ElevationSource for MemoryTiles {
    fn elevation_m(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> f32 {
        self.sample_elevation(lat, lon, sample_pitch_m)
            .value
            .unwrap_or(0.0)
    }

    fn sample_elevation(
        &self,
        lat: f64,
        lon: f64,
        sample_pitch_m: f64,
    ) -> crate::SourceSample<f32> {
        let complete = std::cell::Cell::new(true);
        let value = sample_elevation(lat, lon, sample_pitch_m, |z, x, y| {
            let tile = self.tile(z, x, y);
            if tile.is_none() {
                complete.set(false);
            }
            tile
        });
        crate::SourceSample {
            value,
            complete: complete.get(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "async-http")]
    #[test]
    fn retained_raster_owns_the_registered_slot_across_last_waiter_cancellation() {
        let source = TerrariumSource::new_async(
            Arc::new(AsyncFakeFetcher {
                bytes: encode_tile(|_, _| 4321.5),
                calls: Default::default(),
            }),
            "http://unused",
            None,
        );
        let key = (4, 8, 8);
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let old = source.async_participant(key).unwrap();
        let slot = Arc::downgrade(&old.slot);
        let retained_future = source.prepare_retained_tile(key, 1.0);
        assert_eq!(
            old.flight
                .participants
                .load(std::sync::atomic::Ordering::Acquire),
            2
        );
        // The prior request cancels after the new future captures its slot but
        // before that future polls. Its registered ownership must keep the
        // exact keyed slot alive through fetching and the eventual CPU lease.
        drop(old);
        assert!(Arc::ptr_eq(
            &source.map.lock().unwrap().tiles[&key].0,
            &slot.upgrade().unwrap(),
        ));
        let retained = runtime.block_on(retained_future);
        assert!(retained.summary.complete());
        let tile = slot
            .upgrade()
            .unwrap()
            .get()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        let mut next_key = 0;
        let mut pressure = || {
            let mut map = source.map.lock().unwrap();
            if let Some(entry) = map.tiles.get_mut(&key) {
                entry.1 = 0;
            }
            while map.tiles.len() < MEMORY_TILES {
                let unused = Arc::new(TileSlot::new());
                assert!(unused.set(Some(tile.clone())).is_ok());
                map.tiles.insert((25, next_key, 0), (unused, u64::MAX));
                next_key += 1;
            }
            drop(map);
            source.slot_for((25, next_key, 0));
            next_key += 1;
        };
        pressure();
        assert!(Arc::ptr_eq(
            &source.map.lock().unwrap().tiles[&key].0,
            &slot.upgrade().unwrap(),
        ));
        drop(retained);
        pressure();
        assert!(
            slot.upgrade().is_none(),
            "finished CPU leases must release the cache slot"
        );

        let canceled_key = (24, 0, 0);
        let unpolled = source.prepare_retained_tile(canceled_key, 1.0);
        let canceled_slot = Arc::downgrade(&source.map.lock().unwrap().tiles[&canceled_key].0);
        drop(unpolled);
        assert!(!source.map.lock().unwrap().tiles.contains_key(&canceled_key));
        assert!(
            !source
                .async_locks
                .lock()
                .unwrap()
                .contains_key(&canceled_key)
        );
        assert!(canceled_slot.upgrade().is_none());
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn retained_raster_dependencies_survive_lru_pressure_until_generation_finishes() {
        use crate::async_prep::AsyncSourcePreparer as _;
        let fetcher = Arc::new(AsyncFakeFetcher {
            bytes: encode_tile(|_, _| 4321.5),
            calls: std::sync::atomic::AtomicU64::new(0),
        });
        let source = TerrariumSource::new_async(fetcher.clone(), "http://unused", None);
        let request = prepare_request();
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let retained = runtime.block_on(source.prepare_retained_for_coverage(
            &request,
            crate::async_prep::PreparationCoverage::Global,
        ));
        assert!(retained.summary.complete());
        let expected = source.sample_elevation(0.0, 0.0, 1_000_000.0);
        assert!(expected.complete && expected.value.is_some());
        let requests = fetcher.calls.load(std::sync::atomic::Ordering::Relaxed);
        let protected: Vec<_> = source.map.lock().unwrap().tiles.keys().copied().collect();
        assert!(!protected.is_empty());
        let tile = source.map.lock().unwrap().tiles[&protected[0]]
            .0
            .get()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        let mut next_key = 0;
        let mut apply_pressure = || {
            let mut map = source.map.lock().unwrap();
            for key in &protected {
                if let Some(entry) = map.tiles.get_mut(key) {
                    entry.1 = 0;
                }
            }
            while map.tiles.len() < MEMORY_TILES {
                let slot = Arc::new(TileSlot::new());
                assert!(slot.set(Some(tile.clone())).is_ok());
                map.tiles.insert((25, next_key, 0), (slot, u64::MAX));
                next_key += 1;
            }
            drop(map);
            source.slot_for((25, next_key, 0));
            next_key += 1;
        };
        apply_pressure();
        assert_eq!(source.sample_elevation(0.0, 0.0, 1_000_000.0), expected);
        assert!(
            protected
                .iter()
                .all(|key| source.map.lock().unwrap().tiles.contains_key(key))
        );
        drop(retained);
        apply_pressure();
        let missing = source.sample_elevation(0.0, 0.0, 1_000_000.0);
        assert!(!missing.complete && missing.value.is_none());
        assert_eq!(
            fetcher.calls.load(std::sync::atomic::Ordering::Relaxed),
            requests,
            "CPU sampling must never reacquire a missing async raster"
        );
    }

    fn test_source_cache(label: &str) -> (SourceTileCache, std::path::PathBuf) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "vearth-terrarium-cache-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let store = crate::source_cache::SourceTileStore::open(
            root.join("source_tiles.sqlite3"),
            8 * 1024 * 1024,
        )
        .unwrap();
        let cache = store
            .namespace("terrarium", "test-v1", "image/png", None)
            .unwrap();
        (cache, root)
    }

    /// Fetcher serving one synthetic terrarium tile from memory.
    struct FakeFetcher {
        png: Vec<u8>,
    }

    impl TileFetcher for FakeFetcher {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>, DataError> {
            Ok(self.png.clone())
        }
    }

    struct CountingFetcher {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl TileFetcher for CountingFetcher {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>, DataError> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(DataError::NotAvailable)
        }
    }

    fn encode_tile(height_fn: impl Fn(usize, usize) -> f64) -> Vec<u8> {
        let rgb = terrarium_rgb(height_fn);
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, TILE_SIZE as u32, TILE_SIZE as u32);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&rgb).unwrap();
        }
        out
    }

    fn encode_webp(height_fn: impl Fn(usize, usize) -> f64) -> Vec<u8> {
        let mut out = Vec::new();
        image_webp::WebPEncoder::new(&mut out)
            .encode(
                &terrarium_rgb(height_fn),
                TILE_SIZE as u32,
                TILE_SIZE as u32,
                image_webp::ColorType::Rgb8,
            )
            .unwrap();
        out
    }

    fn terrarium_rgb(height_fn: impl Fn(usize, usize) -> f64) -> Vec<u8> {
        let mut rgb = vec![0u8; TILE_SIZE * TILE_SIZE * 3];
        for y in 0..TILE_SIZE {
            for x in 0..TILE_SIZE {
                let v = (height_fn(x, y) + 32768.0).clamp(0.0, 65535.99);
                let r = (v / 256.0) as u32 as u8;
                let g = (v as u32 % 256) as u8;
                let b = ((v - v.floor()) * 256.0) as u8;
                let p = (y * TILE_SIZE + x) * 3;
                rgb[p] = r;
                rgb[p + 1] = g;
                rgb[p + 2] = b;
            }
        }
        rgb
    }

    #[test]
    fn webp_and_png_decode_the_same_heights() {
        let height = |x: usize, y: usize| x as f64 * 1.5 - y as f64 * 0.75 + 12.25;
        let webp = decode_terrarium(&encode_webp(height)).unwrap();
        let png = decode_terrarium(&encode_tile(height)).unwrap();
        assert_eq!(webp.heights, png.heights);
        assert!((webp.heights[3 * TILE_SIZE + 7] - 20.5).abs() < 0.01);
    }

    #[test]
    fn exact_zero_regions_decode_as_sea_but_isolated_zeros_stay_land() {
        let tile = decode_terrarium(&encode_webp(|x, y| {
            if x < 20 {
                0.0
            } else if (x, y) == (300, 300) {
                0.0
            } else {
                4.0
            }
        }))
        .unwrap();
        assert_eq!(tile.heights[10 * TILE_SIZE + 5], SEA_FLOOR_M);
        assert_eq!(tile.heights[300 * TILE_SIZE + 300], 0.0);
        assert_eq!(tile.heights[300 * TILE_SIZE + 301], 4.0);
        assert!(sea_tile().heights.iter().all(|&h| h == SEA_FLOOR_M));
    }

    #[test]
    fn synthesized_children_sample_exactly_like_their_parent() {
        let z = 9;
        let parent = Arc::new(
            decode_terrarium(&encode_tile(|x, y| {
                100.0 + (x as f64 * 0.37).sin() * 40.0 + (y * y) as f64 * 0.002
            }))
            .unwrap(),
        );
        let (px, py) = (300, 170);
        let children: HashMap<_, _> = (0..4)
            .map(|i| {
                let (qx, qy) = (i % 2, i / 2);
                let key = (z + 1, px * 2 + qx as u32, py * 2 + qy as u32);
                (key, Arc::new(upsample_quadrant(&parent, qx, qy)))
            })
            .collect();
        let size = TILE_SIZE as f64;
        // Stay off the parent's last half pixel, which has no neighbour here.
        for (fx, fy) in [
            (0.1, 0.2),
            (0.49, 0.51),
            (0.5, 0.5),
            (0.73, 0.02),
            (0.998, 0.4),
        ] {
            let parent_px = (f64::from(px) + fx * (size - 1.0) / size) * size;
            let parent_py = (f64::from(py) + fy * (size - 1.0) / size) * size;
            let (lat, lon, pitch) = sample_location(parent_px, parent_py, z);
            let coarse = sample_elevation(lat, lon, pitch, |_, _, _| Some(parent.clone())).unwrap();
            let fine = sample_elevation(lat, lon, pitch / 2.0, |zoom, x, y| {
                assert_eq!(zoom, z + 1);
                children.get(&(zoom, x, y)).cloned()
            })
            .unwrap();
            assert!(
                (coarse - fine).abs() < 1.0e-3,
                "({fx}, {fy}): {coarse} vs {fine}"
            );
        }
    }

    #[test]
    fn coverage_index_names_regions_and_their_finest_zoom() {
        let coverage = Coverage::parse(
            br#"{"version":"0.0.13","items":[
                {"name":"planet.pmtiles","max_zoom":12},
                {"name":"6-33-22.pmtiles","max_zoom":16},
                {"name":"6-40-27.pmtiles","max_zoom":13}]}"#,
        )
        .unwrap();
        assert!(coverage.may_exist((GLOBAL_MAX_ZOOM, 0, 0)));
        assert!(coverage.may_exist((16, 33 << 10, 22 << 10)));
        assert!(!coverage.may_exist((17, 33 << 11, 22 << 11)));
        assert!(coverage.may_exist((13, (40 << 7) + 5, (27 << 7) + 9)));
        assert!(!coverage.may_exist((14, 40 << 8, 27 << 8)));
        assert!(!coverage.may_exist((13, 0, 0)));
        assert!(Coverage::parse(b"{}").is_err());
    }

    #[test]
    fn decode_round_trips_encoding() {
        let png = encode_tile(|x, y| x as f64 * 10.0 - y as f64 * 3.0 + 0.25);
        let tile = decode_terrarium(&png).unwrap();
        for (x, y) in [(0, 0), (10, 20), (TILE_SIZE - 1, TILE_SIZE - 1), (128, 7)] {
            let want = x as f32 * 10.0 - y as f32 * 3.0 + 0.25;
            let got = tile.heights[y * TILE_SIZE + x];
            assert!((got - want).abs() < 0.01, "({x},{y}): {got} vs {want}");
        }
    }

    #[test]
    fn mercator_projection_known_points() {
        // Null island at z=0 is the exact tile center.
        let center = TILE_SIZE as f64 / 2.0;
        let (x, y) = mercator_px(0.0, 0.0, 0);
        assert!((x - center).abs() < 1e-9 && (y - center).abs() < 1e-9);
        // Positive latitude is up (smaller y).
        let (_, y_n) = mercator_px(0.5, 0.0, 0);
        assert!(y_n < center);
        // Polar clamp stays in range at high zoom.
        let (_, y_pole) = mercator_px(1.57, 0.1, 10);
        assert!(y_pole >= 0.0);
    }

    #[test]
    fn zoom_selection() {
        // ~620 m voxels at the equator → z7 (611.5 m/px); street level clamps.
        assert_eq!(TerrariumSource::zoom_for(620.0, 0.0), 7);
        assert_eq!(TerrariumSource::zoom_for(0.6, 0.0), MAX_ZOOM);
        // High latitude needs less zoom for the same ground size.
        assert!(TerrariumSource::zoom_for(611.0, 1.2) < 7);
    }

    fn sample_location(px: f64, py: f64, z: u8) -> (f64, f64, f64) {
        let pixels_n = TILE_SIZE as f64 * (1u64 << z) as f64;
        let lon = (px / pixels_n - 0.5) * std::f64::consts::TAU;
        let lat = (std::f64::consts::PI * (1.0 - 2.0 * py / pixels_n))
            .sinh()
            .atan();
        let sample_pitch_m = pitch_for_zoom(z, lat);
        assert_eq!(TerrariumSource::zoom_for(sample_pitch_m, lat), z);
        (lat, lon, sample_pitch_m)
    }

    fn pitch_for_zoom(z: u8, lat: f64) -> f64 {
        EQUATOR_M / TILE_SIZE as f64 * lat.cos().max(0.05) / (1u64 << z) as f64 * 1.05
    }

    struct TileMapFetcher {
        tiles: HashMap<(u8, u32, u32), Vec<u8>>,
    }

    impl TileFetcher for TileMapFetcher {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, DataError> {
            let mut parts = url.trim_end_matches(".webp").rsplit('/');
            let y = parts.next().unwrap().parse().unwrap();
            let x = parts.next().unwrap().parse().unwrap();
            let z = parts.next().unwrap().parse().unwrap();
            self.tiles
                .get(&(z, x, y))
                .cloned()
                .ok_or(DataError::NotAvailable)
        }
    }

    #[test]
    fn bilinear_gradient_is_continuous_across_tile_edges_and_corners() {
        let z = 2;
        let mut tiles = HashMap::new();
        let memory = MemoryTiles::default();
        for ty in 1..=2 {
            for tx in 1..=2 {
                let png = encode_tile(|x, y| {
                    100.0
                        + (tx as usize * TILE_SIZE + x) as f64 * 0.5
                        + (ty as usize * TILE_SIZE + y) as f64
                });
                memory.insert_png(z, tx, ty, &png).unwrap();
                tiles.insert((z, tx, ty), png);
            }
        }
        let source =
            TerrariumSource::new(Arc::new(TileMapFetcher { tiles }), "http://unused", None);
        // Tiles 1..=2 meet at pixel `seam` on both axes.
        let seam = 2.0 * TILE_SIZE as f64;
        for (px, py) in [
            (seam - 0.75, seam - 111.25),
            (seam - 111.75, seam - 0.25),
            (seam - 0.75, seam - 0.25),
            (seam - 0.001, seam - 0.001),
            (seam + 0.001, seam + 0.001),
            (seam + 0.25, seam + 0.25),
        ] {
            let (lat, lon, sample_pitch_m) = sample_location(px, py, z);
            let expected = (100.0 + px * 0.5 + py) as f32;
            let sync = source.elevation_m(lat, lon, sample_pitch_m);
            let prepared = memory.elevation_m(lat, lon, sample_pitch_m);
            assert!(
                (sync - expected).abs() < 0.0002,
                "({px}, {py}): {sync} vs {expected}"
            );
            assert_eq!(
                sync, prepared,
                "fetch-through and prepared sources must agree"
            );
        }
        assert_eq!(source.fetch_errors(), 0);
    }

    #[test]
    fn missing_neighbours_retain_owner_edge_heights() {
        let z = 2;
        let owner = Arc::new(decode_terrarium(&encode_tile(|_, _| 100.0)).unwrap());
        let east = Arc::new(decode_terrarium(&encode_tile(|_, _| 140.0)).unwrap());
        let south = Arc::new(decode_terrarium(&encode_tile(|_, _| 200.0)).unwrap());
        let diagonal = Arc::new(decode_terrarium(&encode_tile(|_, _| 280.0)).unwrap());
        let seam = 2.0 * TILE_SIZE as f64;
        let (lat, lon, sample_pitch_m) = sample_location(seam - 0.75, seam - 0.5, z);
        for available in 0..8 {
            let got = sample_elevation(lat, lon, sample_pitch_m, |_, x, y| match (x, y) {
                (1, 1) => Some(owner.clone()),
                (2, 1) if available & 1 != 0 => Some(east.clone()),
                (1, 2) if available & 2 != 0 => Some(south.clone()),
                (2, 2) if available & 4 != 0 => Some(diagonal.clone()),
                _ => None,
            })
            .unwrap();
            let expected = 100.0 * 0.375
                + if available & 1 != 0 { 140.0 } else { 100.0 } * 0.125
                + if available & 2 != 0 { 200.0 } else { 100.0 } * 0.375
                + if available & 4 != 0 { 280.0 } else { 100.0 } * 0.125;
            assert!(
                (got - expected).abs() < 0.0001,
                "neighbours {available}: {got}"
            );
        }
        let calls = std::cell::Cell::new(0);
        assert!(
            sample_elevation(lat, lon, sample_pitch_m, |_, _, _| {
                calls.set(calls.get() + 1);
                None
            })
            .is_none()
        );
        assert_eq!(
            calls.get(),
            1,
            "a missing owner retains the no-data contract"
        );

        let owner =
            Arc::new(decode_terrarium(&encode_tile(|x, y| x as f64 + 2.0 * y as f64)).unwrap());
        let (lat, lon, sample_pitch_m) = sample_location(seam - 0.75, TILE_SIZE as f64 + 37.5, z);
        let got = sample_elevation(lat, lon, sample_pitch_m, |_, x, y| {
            (x == 1 && y == 1).then(|| owner.clone())
        })
        .unwrap();
        let expected = (TILE_SIZE - 1) as f32 + 75.0;
        assert!(
            (got - expected).abs() < 0.0001,
            "fallback preserves along-edge interpolation"
        );
    }

    #[test]
    fn bilinear_sampling_wraps_longitudes_and_the_antimeridian_taps() {
        // Heights stay off exactly 0 m, which decodes as open sea.
        let last = (TILE_SIZE - 1) as f64;
        let tiles = MemoryTiles::default();
        tiles
            .insert_png(2, 3, 2, &encode_tile(|x, _| x as f64 - last + 0.5))
            .unwrap();
        tiles
            .insert_png(2, 0, 2, &encode_tile(|x, _| x as f64 + 1.5))
            .unwrap();
        let size = TILE_SIZE as f64;
        let (lat, lon, sample_pitch_m) = sample_location(4.0 * size - 0.25, 2.0 * size + 88.25, 2);
        for turns in -2..=2 {
            let wrapped = lon + turns as f64 * std::f64::consts::TAU;
            let got = tiles.elevation_m(lat, wrapped, sample_pitch_m);
            assert!(
                (got - 1.25).abs() < 0.0001,
                "wrapped longitude {wrapped}: {got}"
            );
            assert_eq!(sampling_tiles(lat, wrapped, 2), vec![(2, 3, 2), (2, 0, 2)]);
        }

        let tiles = MemoryTiles::default();
        tiles
            .insert_png(
                0,
                0,
                0,
                &encode_tile(|x, _| if x == TILE_SIZE - 1 { 11.0 } else { 1.0 }),
            )
            .unwrap();
        let (lat, lon, sample_pitch_m) = sample_location(size - 0.5, size / 2.0, 0);
        assert!((tiles.elevation_m(lat, lon, sample_pitch_m) - 6.0).abs() < 0.0001);
        assert_eq!(sampling_tiles(lat, lon, 0), vec![(0, 0, 0)]);
    }

    #[test]
    fn sampling_clamps_polar_rows_and_avoids_zero_weight_neighbour_fetches() {
        let tile = Arc::new(decode_terrarium(&encode_tile(|_, y| 100.0 + y as f64)).unwrap());
        for (lat, expected_y, expected_height) in [
            (std::f64::consts::FRAC_PI_2, 0, 100.0),
            (
                -std::f64::consts::FRAC_PI_2,
                3,
                100.0 + (TILE_SIZE - 1) as f32,
            ),
        ] {
            let got = sample_elevation(lat, 0.0, pitch_for_zoom(2, lat), |z, x, y| {
                assert_eq!((z, x, y), (2, 2, expected_y));
                Some(tile.clone())
            })
            .unwrap();
            assert!(
                (got - expected_height).abs() < 0.001,
                "polar edge height {got}"
            );
        }
        let seam = 2.0 * TILE_SIZE as f64;
        let (lat, lon, sample_pitch_m) = sample_location(seam - 1.0, seam, 2);
        let calls = std::cell::Cell::new(0);
        sample_elevation(lat, lon, sample_pitch_m, |z, x, y| {
            calls.set(calls.get() + 1);
            assert_eq!((z, x, y), (2, 1, 2));
            Some(tile.clone())
        })
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(sampling_tiles(lat, lon, 2), vec![(2, 1, 2)]);
    }

    #[test]
    fn tile_prediction_wraps_just_west_of_antimeridian() {
        let z = 11;
        let n = 1u32 << z;
        assert_eq!(tile_at(0.0, -std::f64::consts::PI - 1.0e-9, z).0, n - 1);
        assert_eq!(tile_at(0.0, std::f64::consts::PI - 1.0e-9, z).0, n - 1);
    }

    #[test]
    fn source_samples_through_cache() {
        let png = encode_tile(|_, _| 1234.0);
        let src = TerrariumSource::new(Arc::new(FakeFetcher { png }), "http://unused", None);
        let h = src.elevation_m(0.5, 0.5, 100_000.0);
        assert!((h - 1234.0).abs() < 0.02, "{h}");
        // Second sample hits the memory cache (same value, no panic).
        let h2 = src.elevation_m(0.50001, 0.5, 100_000.0);
        assert!((h2 - 1234.0).abs() < 0.02);
    }

    #[test]
    fn local_sampling_preserves_values_without_pinning_shared_slots() {
        let source = TerrariumSource::new(
            Arc::new(FakeFetcher {
                png: encode_tile(|x, y| x as f64 + y as f64 * 0.25),
            }),
            "http://unused",
            None,
        );
        let inside = TILE_SIZE as f64 + 144.25;
        let (lat, lon, pitch) = sample_location(inside, inside + 0.25, 2);
        let expected = source.sample_elevation(lat, lon, pitch);
        let clock = source.map.lock().unwrap().clock;
        for _ in 0..64 {
            assert_eq!(source.sample_elevation(lat, lon, pitch), expected);
        }
        assert_eq!(source.map.lock().unwrap().clock, clock);
        let (weak_slot, weak_tile) = {
            let mut map = source.map.lock().unwrap();
            let slot = &map.tiles[&(2, 1, 1)].0;
            assert_eq!(Arc::strong_count(slot), 1);
            let weak_slot = Arc::downgrade(slot);
            let weak_tile = Arc::downgrade(slot.get().unwrap().as_ref().unwrap());
            map.tiles.clear();
            (weak_slot, weak_tile)
        };
        assert!(weak_slot.upgrade().is_none());
        assert_eq!(source.sample_elevation(lat, lon, pitch), expected);
        assert!(weak_tile.upgrade().is_none());
        assert!(source.map.lock().unwrap().clock > clock);
    }

    #[test]
    fn borrowed_sampling_matches_reference_at_interiors_borders_and_poles() {
        let source = TerrariumSource::new(
            Arc::new(FakeFetcher {
                png: encode_tile(|x, y| -512.0 + x as f64 * 2.125 + y as f64 * 0.25),
            }),
            "http://unused",
            None,
        );
        for z in [0, 2, 8] {
            let side = (TILE_SIZE << z) as f64;
            for pixel in [0.0, 0.125, 127.75, 255.0, 255.875, side - 0.125] {
                for offset in 0..32 {
                    let (lat, lon, pitch) =
                        sample_location(pixel, (pixel + offset as f64 * 0.125).min(side - 1.0), z);
                    let expected =
                        sample_elevation(lat, lon, pitch, |z, x, y| source.tile(z, x, y));
                    assert_eq!(
                        source.sample_elevation(lat, lon, pitch),
                        crate::SourceSample::ready(expected)
                    );
                }
            }
        }
    }

    #[test]
    fn source_reopens_raw_tile_from_indexed_cache_without_network() {
        let (cache, root) = test_source_cache("reopen");
        let png = encode_tile(|_, _| 1234.0);
        let first = TerrariumSource::new(
            Arc::new(FakeFetcher { png }),
            "http://unused",
            Some(cache.clone()),
        );
        assert!((first.elevation_m(0.5, 0.5, 100_000.0) - 1234.0).abs() < 0.02);
        assert!(cache.flush());
        drop(first);

        let offline = Arc::new(CountingFetcher {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let reopened = TerrariumSource::new(offline.clone(), "http://unused", Some(cache.clone()));
        assert!((reopened.elevation_m(0.5, 0.5, 100_000.0) - 1234.0).abs() < 0.02);
        assert_eq!(offline.calls.load(std::sync::atomic::Ordering::Relaxed), 0);
        drop((reopened, cache));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn builtin_pack_serves_coarse_tiles_and_absent_finer_tiles_complete_from_parents() {
        let fetcher = Arc::new(CountingFetcher {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let src =
            TerrariumSource::new(fetcher.clone(), "http://unused", None).with_builtin_terrain();

        let coarse = src.elevation_m(0.0, 0.0, 1_000_000.0);
        assert!(coarse.is_finite());
        assert_eq!(fetcher.calls.load(std::sync::atomic::Ordering::Relaxed), 0);

        // No coverage index and no tiles at z14, z13 or z12: the finest tile
        // is completed from its parents, and an absent planet tile is sea.
        let fine = src.elevation_m(0.0, 0.0, 1.0);
        assert_eq!(fine, SEA_FLOOR_M);
        assert_eq!(fetcher.calls.load(std::sync::atomic::Ordering::Relaxed), 4);
    }

    #[test]
    fn memory_tiles_recognize_builtin_pack_without_eager_decode() {
        let tiles = MemoryTiles::with_builtin_terrain();
        let max_zoom = crate::packed_terrain::builtin().max_zoom();
        let last = (1u32 << max_zoom) - 1;
        assert!(tiles.contains(0, 0, 0));
        assert!(tiles.contains(max_zoom, last, last));
        assert!(!tiles.contains(max_zoom + 1, 0, 0));
        assert!(tiles.is_empty());
    }

    #[cfg(feature = "async-http")]
    struct AsyncFakeFetcher {
        bytes: Vec<u8>,
        calls: std::sync::atomic::AtomicU64,
    }

    #[cfg(feature = "async-http")]
    impl crate::async_http::AsyncTileFetcher for AsyncFakeFetcher {
        fn fetch<'a>(&'a self, _url: &'a str) -> crate::async_http::FetchFuture<'a> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let bytes = self.bytes.clone();
            Box::pin(std::future::ready(Ok(bytes)))
        }
    }

    #[cfg(feature = "async-http")]
    struct PendingAsyncFetcher {
        calls: std::sync::atomic::AtomicU64,
    }

    #[cfg(feature = "async-http")]
    impl crate::async_http::AsyncTileFetcher for PendingAsyncFetcher {
        fn fetch<'a>(&'a self, _url: &'a str) -> crate::async_http::FetchFuture<'a> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Box::pin(std::future::pending())
        }
    }

    /// Serves the tiles it holds and 404s every other URL, recording requests.
    #[cfg(feature = "async-http")]
    struct SparseAsyncFetcher {
        tiles: HashMap<(u8, u32, u32), Vec<u8>>,
        requested: Mutex<Vec<String>>,
    }

    #[cfg(feature = "async-http")]
    impl crate::async_http::AsyncTileFetcher for SparseAsyncFetcher {
        fn fetch<'a>(&'a self, url: &'a str) -> crate::async_http::FetchFuture<'a> {
            self.requested.lock().unwrap().push(url.to_owned());
            let mut parts = url.trim_end_matches(".webp").rsplit('/');
            let key = (|| {
                let y = parts.next()?.parse().ok()?;
                let x = parts.next()?.parse().ok()?;
                let z = parts.next()?.parse().ok()?;
                Some((z, x, y))
            })();
            let found = key.and_then(|key| self.tiles.get(&key).cloned());
            Box::pin(std::future::ready(found.ok_or(DataError::NotAvailable)))
        }
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_prepare_completes_absent_tiles_from_parents_and_sea() {
        let parent = (GLOBAL_MAX_ZOOM, 1000, 1000);
        let fetcher = Arc::new(SparseAsyncFetcher {
            tiles: HashMap::from([(parent, encode_webp(|x, _| 50.0 + x as f64))]),
            requested: Mutex::new(Vec::new()),
        });
        let (cache, root) = test_source_cache("absent-tiles");
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let prepare = |source: &TerrariumSource, key| {
            runtime.block_on(async {
                let participant = source.async_participant(key).unwrap();
                let outcome = source.prepare_tile(&participant, 1.0).await;
                (outcome, participant.slot.get().cloned().flatten())
            })
        };
        let source =
            TerrariumSource::new_async(fetcher.clone(), "http://unused", Some(cache.clone()))
                .with_coverage(None);
        // A 404 beyond the planet archive is completed from the parent.
        let child = (GLOBAL_MAX_ZOOM + 1, 2001, 2000);
        let (outcome, tile) = prepare(&source, child);
        assert_eq!(outcome, crate::async_prep::DependencyOutcome::Ready);
        let expected = upsample_quadrant(&decode_terrarium(&fetcher.tiles[&parent]).unwrap(), 1, 0);
        assert_eq!(tile.unwrap().heights, expected.heights);
        // An absent planet tile is open sea.
        let (outcome, tile) = prepare(&source, (GLOBAL_MAX_ZOOM, 7, 7));
        assert_eq!(outcome, crate::async_prep::DependencyOutcome::Ready);
        assert!(tile.unwrap().heights.iter().all(|&h| h == SEA_FLOOR_M));
        let first = fetcher.requested.lock().unwrap().len();
        assert_eq!(first, 3, "child 404, parent, and the sea tile's 404");

        // A new session reads the recorded 404s from disk; a coverage index
        // that rules a region out skips the request entirely.
        let coverage = Coverage::parse(br#"{"items":[]}"#).unwrap();
        let source = TerrariumSource::new_async(fetcher.clone(), "http://unused", Some(cache))
            .with_coverage(Some(coverage));
        assert!(prepare(&source, child).1.is_some());
        assert!(prepare(&source, (GLOBAL_MAX_ZOOM, 7, 7)).1.is_some());
        assert!(
            prepare(&source, (GLOBAL_MAX_ZOOM + 2, 4000, 4000))
                .1
                .is_some()
        );
        let requested = fetcher.requested.lock().unwrap().clone();
        assert!(
            requested[first..]
                .iter()
                .all(|url| url.contains(&format!("/{GLOBAL_MAX_ZOOM}/"))),
            "only planet tiles reach the network: {:?}",
            &requested[first..]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(feature = "async-http")]
    fn prepare_request() -> crate::async_prep::PrepareRequest {
        crate::async_prep::PrepareRequest {
            terrain_tiles: vec![(0, 0, 0)],
            footprint: crate::async_prep::SourceFootprint {
                center_lat: 0.0,
                center_lon: 0.0,
                radius_rad: 0.0,
                sample_hint_m: 1_000_000.0,
            },
            vector_bounds: None,
            night_building_bounds: None,
            priority: 10.0,
        }
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_plan_covers_all_zoom_and_wrapped_tiles_in_footprint_envelope() {
        let fetcher = Arc::new(AsyncFakeFetcher {
            bytes: Vec::new(),
            calls: std::sync::atomic::AtomicU64::new(0),
        });
        let source = TerrariumSource::new_async(fetcher, "http://unused", None);
        let footprint = crate::async_prep::SourceFootprint {
            center_lat: 0.04,
            center_lon: std::f64::consts::PI - 0.002,
            radius_rad: 0.08,
            sample_hint_m: 600.0,
        };
        let request = crate::async_prep::PrepareRequest {
            terrain_tiles: Vec::new(),
            footprint,
            vector_bounds: None,
            night_building_bounds: None,
            priority: 1.0,
        };
        let planned: std::collections::HashSet<_> =
            source.planned_tiles(&request).into_iter().collect();
        let min_cos = (footprint.center_lat - footprint.radius_rad)
            .cos()
            .abs()
            .min((footprint.center_lat + footprint.radius_rad).cos().abs())
            .max(0.05);
        let lon_radius = footprint.radius_rad / min_cos;
        for lat_step in 0..=32 {
            let lat = footprint.center_lat - footprint.radius_rad
                + 2.0 * footprint.radius_rad * lat_step as f64 / 32.0;
            for lon_step in 0..=32 {
                let lon =
                    footprint.center_lon - lon_radius + 2.0 * lon_radius * lon_step as f64 / 32.0;
                let z = TerrariumSource::zoom_for(footprint.sample_hint_m, lat);
                for key in sampling_tiles(lat, lon, z) {
                    assert!(
                        planned.contains(&key),
                        "plan missed sampled key {key:?} at {lat}/{lon}"
                    );
                }
            }
        }
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_plan_covers_bilinear_envelopes_across_latitudes_and_zooms() {
        let source = TerrariumSource::new_async(
            Arc::new(AsyncFakeFetcher {
                bytes: Vec::new(),
                calls: Default::default(),
            }),
            "http://unused",
            None,
        );
        let mut footprints = Vec::new();
        for z in 0..=MAX_ZOOM {
            for lat in [-1.5_f64, -1.1, -0.01, 0.0, 0.7, 1.054, 1.5] {
                for lon in [-3.15, -3.14, 0.0, 3.14, 3.15] {
                    footprints.push(crate::async_prep::SourceFootprint {
                        center_lat: lat,
                        center_lon: lon,
                        radius_rad: (0.3 * std::f64::consts::TAU * lat.cos().abs()
                            / (1u32 << z) as f64)
                            .min(0.08),
                        sample_hint_m: pitch_for_zoom(z, lat),
                    });
                }
            }
        }
        // Nearly global longitude intervals can have endpoints in the same
        // coarse tile. The whole eastward arc must remain prepared.
        footprints.push(crate::async_prep::SourceFootprint {
            center_lat: 1.4,
            center_lon: std::f64::consts::PI - 0.01,
            radius_rad: 0.25,
            sample_hint_m: 20_000.0,
        });
        for footprint in footprints {
            let request = crate::async_prep::PrepareRequest {
                terrain_tiles: Vec::new(),
                footprint,
                ..prepare_request()
            };
            let planned: std::collections::HashSet<_> =
                source.planned_tiles(&request).into_iter().collect();
            let lat_min = (footprint.center_lat - footprint.radius_rad).max(-1.484_422);
            let lat_max = (footprint.center_lat + footprint.radius_rad).min(1.484_422);
            let min_cos = lat_min.cos().abs().min(lat_max.cos().abs()).max(0.05);
            let lon_radius = (footprint.radius_rad / min_cos).min(std::f64::consts::PI);
            for lat_step in 0..=16 {
                let lat = lat_min + (lat_max - lat_min) * lat_step as f64 / 16.0;
                for lon_step in 0..=16 {
                    let lon = footprint.center_lon - lon_radius
                        + 2.0 * lon_radius * lon_step as f64 / 16.0;
                    let z = TerrariumSource::zoom_for(footprint.sample_hint_m, lat);
                    for key in sampling_tiles(lat, lon, z) {
                        assert!(
                            planned.contains(&key),
                            "plan missed {key:?} at {lat}/{lon}: {footprint:?}"
                        );
                    }
                }
            }
        }
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_plan_keeps_exact_legacy_tiles_without_padding_them() {
        let source = TerrariumSource::new_async(
            Arc::new(AsyncFakeFetcher {
                bytes: Vec::new(),
                calls: Default::default(),
            }),
            "http://unused",
            None,
        );
        let mut request = prepare_request();
        request.terrain_tiles = vec![(15, 123, 456), (2, 0, 0), (15, 123, 456)];
        assert_eq!(
            source.planned_tiles(&request),
            vec![(0, 0, 0), (2, 0, 0), (15, 123, 456)]
        );
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_prepare_includes_bilinear_neighbours_before_cache_only_sampling() {
        use crate::async_prep::AsyncSourcePreparer as _;

        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        for (px, py) in [
            (400.25, 400.25),
            (511.0, 511.0),
            (511.25, 400.25),
            (400.25, 511.25),
            (511.75, 511.25),
            (1023.0, 511.0),
            (1023.5, 511.25),
            (1023.5, 1023.0),
        ] {
            let (lat, lon, sample_pitch_m) = sample_location(px, py, 2);
            let fetcher = Arc::new(AsyncFakeFetcher {
                bytes: encode_tile(|_, _| 4321.5),
                calls: std::sync::atomic::AtomicU64::new(0),
            });
            let source = TerrariumSource::new_async(fetcher.clone(), "http://unused", None);
            let request = crate::async_prep::PrepareRequest {
                terrain_tiles: Vec::new(),
                footprint: crate::async_prep::SourceFootprint {
                    center_lat: lat,
                    center_lon: lon,
                    radius_rad: 0.0,
                    sample_hint_m: sample_pitch_m,
                },
                ..prepare_request()
            };
            let expected: std::collections::BTreeSet<_> =
                sampling_tiles(lat, lon, 2).into_iter().collect();
            let planned: std::collections::BTreeSet<_> =
                source.planned_tiles(&request).into_iter().collect();
            assert_eq!(planned, expected);
            let summary = runtime.block_on(source.prepare(&request));
            assert!(summary.complete());
            assert_eq!(summary.ready as usize, expected.len());
            let got = source.elevation_m(lat, lon, sample_pitch_m);
            assert!((got - 4321.5).abs() < 0.001);
            assert_eq!(
                source.fetch_errors(),
                0,
                "all interpolation taps must be resident"
            );
            assert_eq!(
                fetcher.calls.load(std::sync::atomic::Ordering::Relaxed) as usize,
                expected.len()
            );
        }
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_prepare_populates_the_synchronous_cache_once() {
        use crate::async_prep::AsyncSourcePreparer as _;

        let fetcher = Arc::new(AsyncFakeFetcher {
            bytes: encode_tile(|_, _| 4321.5),
            calls: std::sync::atomic::AtomicU64::new(0),
        });
        let src = TerrariumSource::new_async(fetcher.clone(), "http://unused", None);
        let request = prepare_request();
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let summaries = runtime.block_on(crate::async_prep::join_summaries(vec![
            src.prepare(&request),
            src.prepare(&request),
        ]));
        assert!(summaries.complete());
        assert_eq!(summaries.ready, 2);
        assert_eq!(fetcher.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        let got = src.elevation_m(0.0, 0.0, 1_000_000.0);
        assert!((got - 4321.5).abs() < 0.02, "{got}");
        assert_eq!(fetcher.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_prepare_uses_builtin_pack_without_network() {
        use crate::async_prep::AsyncSourcePreparer as _;

        let fetcher = Arc::new(AsyncFakeFetcher {
            bytes: b"network must not be used".to_vec(),
            calls: std::sync::atomic::AtomicU64::new(0),
        });
        let src = TerrariumSource::new_async(fetcher.clone(), "http://unused", None)
            .with_builtin_terrain();
        let request = prepare_request();
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let summary = runtime.block_on(src.prepare(&request));
        assert!(summary.complete());
        assert_eq!(summary.ready, 1);
        assert_eq!(fetcher.calls.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(src.elevation_m(0.0, 0.0, 1_000_000.0).is_finite());
        assert_eq!(fetcher.calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_prepare_failure_is_tombstoned_and_typed_incomplete() {
        use crate::async_prep::AsyncSourcePreparer as _;

        let fetcher = Arc::new(AsyncFakeFetcher {
            bytes: b"not a png".to_vec(),
            calls: std::sync::atomic::AtomicU64::new(0),
        });
        let src = TerrariumSource::new_async(fetcher.clone(), "http://unused", None);
        let request = prepare_request();
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let first = runtime.block_on(src.prepare(&request));
        let second = runtime.block_on(src.prepare(&request));
        assert!(!first.complete());
        assert!(!second.complete());
        assert_eq!(first.transient_failures, 1);
        assert_eq!(second.transient_failures, 1);
        assert_eq!(fetcher.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(src.fetch_errors(), 1);
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn source_gate_bounds_requests_and_cancellation_reclaims_unfinished_slots() {
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let fetcher = Arc::new(PendingAsyncFetcher {
                calls: std::sync::atomic::AtomicU64::new(0),
            });
            let source = Arc::new(TerrariumSource::new_async(
                fetcher.clone(),
                "http://unused",
                None,
            ));
            let planned = ASYNC_TILE_LIMIT + 8;
            let tasks: Vec<_> = (0..planned as u32)
                .map(|x| {
                    let source = source.clone();
                    vearth_platform::rt::spawn(async move {
                        let participant =
                            source.async_participant((GLOBAL_MAX_ZOOM, x, 0)).unwrap();
                        source.prepare_tile(&participant, 1.0).await
                    })
                })
                .collect();
            while source.map.lock().unwrap().tiles.len() < planned {
                vearth_platform::rt::task::yield_now().await;
            }
            assert_eq!(
                fetcher.calls.load(std::sync::atomic::Ordering::Relaxed),
                ASYNC_TILE_LIMIT as u64
            );
            for task in &tasks {
                task.abort();
            }
            for task in tasks {
                let _ = task.await;
            }
            assert!(source.map.lock().unwrap().tiles.is_empty());
            assert!(source.async_locks.lock().unwrap().is_empty());
        });
    }

    /// Live network test — run explicitly with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_everest_is_high_and_pacific_is_sea() {
        let src = TerrariumSource::new(Arc::new(crate::HttpFetcher::default()), DEFAULT_URL, None);
        // Everest (27.9881°N, 86.9250°E) at ~500 m voxels.
        let h = src.elevation_m(27.9881f64.to_radians(), 86.925f64.to_radians(), 500.0);
        assert!(h > 8000.0, "Everest sampled at {h} m");
        // Mid-Pacific is open sea (no bathymetry).
        let d = src.elevation_m(0.0, (-150.0f64).to_radians(), 500.0);
        assert_eq!(d, SEA_FLOOR_M, "Pacific sampled at {d} m");
        // Netherlands is near sea level.
        let nl = src.elevation_m(52.37f64.to_radians(), 4.9f64.to_radians(), 500.0);
        assert!((-10.0..50.0).contains(&nl), "Amsterdam sampled at {nl} m");
    }
}
