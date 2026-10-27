//! Compact, lazily decoded global surface appearance bundled with Low Poly Earth.
//!
//! The twelve monthly source mosaics are stored as independently decodable
//! 512-pixel JPEG tiles at two equirectangular levels per month. Coarse globe
//! generation therefore never fans out across the detailed atlas, while a
//! regional view decodes only the handful of tiles it actually samples. The
//! compressed bytes are compiled into the application: this provider performs
//! no network I/O and needs no disk cache.

use crate::{ColorSource, DataError};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const TILE_SIZE: usize = 512;
pub const COARSE_WIDTH: usize = 5_400;
pub const COARSE_HEIGHT: usize = 2_700;
pub const DETAIL_WIDTH: usize = 21_600;
pub const DETAIL_HEIGHT: usize = 10_800;
pub const MONTHS: usize = 12;
pub const LEVELS_PER_MONTH: usize = 2;

const MAGIC: &[u8; 8] = b"VESURF01";
const HEADER_LEN: usize = 16;
const LEVEL_LEN: usize = 16;
const ENTRY_LEN: usize = 4;
const DETAIL_VOXEL_LIMIT_M: f64 = 4_000.0;
const MEMORY_TILES: usize = 64;

/// One month's pack, `index` 0 for January. Separate files let the browser
/// download only the months it shows.
fn builtin_bytes(index: usize) -> &'static [u8] {
    match index {
        0 => vearth_platform::asset_bytes!("../assets/surface-01.vsp"),
        1 => vearth_platform::asset_bytes!("../assets/surface-02.vsp"),
        2 => vearth_platform::asset_bytes!("../assets/surface-03.vsp"),
        3 => vearth_platform::asset_bytes!("../assets/surface-04.vsp"),
        4 => vearth_platform::asset_bytes!("../assets/surface-05.vsp"),
        5 => vearth_platform::asset_bytes!("../assets/surface-06.vsp"),
        6 => vearth_platform::asset_bytes!("../assets/surface-07.vsp"),
        7 => vearth_platform::asset_bytes!("../assets/surface-08.vsp"),
        8 => vearth_platform::asset_bytes!("../assets/surface-09.vsp"),
        9 => vearth_platform::asset_bytes!("../assets/surface-10.vsp"),
        10 => vearth_platform::asset_bytes!("../assets/surface-11.vsp"),
        11 => vearth_platform::asset_bytes!("../assets/surface-12.vsp"),
        _ => panic!("invalid surface month index {index}"),
    }
}

#[derive(Clone, Debug)]
struct Level {
    width: usize,
    height: usize,
    tiles_x: usize,
    tiles_y: usize,
    first_entry: usize,
}

#[derive(Clone, Debug)]
struct Entry {
    payload: Range<usize>,
}

/// Parsed immutable view over a surface pack.
pub struct PackedSurface<'a> {
    bytes: &'a [u8],
    tile_size: usize,
    levels: Vec<Level>,
    entries: Vec<Entry>,
}

impl<'a> PackedSurface<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, DataError> {
        if bytes.len() < HEADER_LEN || &bytes[..MAGIC.len()] != MAGIC {
            return Err(decode_error("bad packed-surface header"));
        }
        let tile_size = read_u16(bytes, 8)? as usize;
        if tile_size == 0 {
            return Err(decode_error("packed surface has zero tile size"));
        }
        let level_count = read_u16(bytes, 10)? as usize;
        let entry_count = read_u32(bytes, 12)? as usize;
        if level_count == 0 || entry_count == 0 {
            return Err(decode_error("packed surface is empty"));
        }
        let levels_end = HEADER_LEN
            .checked_add(
                level_count
                    .checked_mul(LEVEL_LEN)
                    .ok_or_else(|| decode_error("packed-surface level index overflow"))?,
            )
            .ok_or_else(|| decode_error("packed-surface level index overflow"))?;
        let entries_end = levels_end
            .checked_add(
                entry_count
                    .checked_mul(ENTRY_LEN)
                    .ok_or_else(|| decode_error("packed-surface tile index overflow"))?,
            )
            .ok_or_else(|| decode_error("packed-surface tile index overflow"))?;
        if entries_end > bytes.len() {
            return Err(decode_error("truncated packed-surface index"));
        }

        let mut levels = Vec::with_capacity(level_count);
        let mut expected_first = 0usize;
        for index in 0..level_count {
            let at = HEADER_LEN + index * LEVEL_LEN;
            let width = read_u32(bytes, at)? as usize;
            let height = read_u32(bytes, at + 4)? as usize;
            let first_entry = read_u32(bytes, at + 8)? as usize;
            let count = read_u32(bytes, at + 12)? as usize;
            if width == 0 || height == 0 || first_entry != expected_first {
                return Err(decode_error("invalid packed-surface level"));
            }
            let tiles_x = width.div_ceil(tile_size);
            let tiles_y = height.div_ceil(tile_size);
            let expected_count = tiles_x
                .checked_mul(tiles_y)
                .ok_or_else(|| decode_error("packed-surface level is too large"))?;
            if count != expected_count {
                return Err(decode_error(format!(
                    "packed-surface level has {count} tiles, expected {expected_count}"
                )));
            }
            expected_first = first_entry
                .checked_add(count)
                .ok_or_else(|| decode_error("packed-surface entry count overflow"))?;
            levels.push(Level {
                width,
                height,
                tiles_x,
                tiles_y,
                first_entry,
            });
        }
        if expected_first != entry_count {
            return Err(decode_error(
                "packed-surface levels do not cover the tile index",
            ));
        }

        let mut entries = Vec::with_capacity(entry_count);
        let mut payload_offset = entries_end;
        for index in 0..entry_count {
            let compressed_len = read_u32(bytes, levels_end + index * ENTRY_LEN)? as usize;
            if compressed_len == 0 {
                return Err(decode_error("packed-surface tile has zero bytes"));
            }
            let payload_end = payload_offset
                .checked_add(compressed_len)
                .ok_or_else(|| decode_error("packed-surface payload overflow"))?;
            if payload_end > bytes.len() {
                return Err(decode_error("truncated packed-surface payload"));
            }
            entries.push(Entry {
                payload: payload_offset..payload_end,
            });
            payload_offset = payload_end;
        }
        if payload_offset != bytes.len() {
            return Err(decode_error("packed surface has trailing bytes"));
        }
        Ok(Self {
            bytes,
            tile_size,
            levels,
            entries,
        })
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    pub fn dimensions(&self, level: usize) -> Option<(usize, usize)> {
        self.levels.get(level).map(|l| (l.width, l.height))
    }

    pub fn tile_count(&self) -> usize {
        self.entries.len()
    }

    fn tile_entry(&self, level: usize, tile_x: usize, tile_y: usize) -> Option<usize> {
        let level = self.levels.get(level)?;
        if tile_x >= level.tiles_x || tile_y >= level.tiles_y {
            return None;
        }
        Some(level.first_entry + tile_y * level.tiles_x + tile_x)
    }

    fn decode_tile(&self, level_index: usize, entry_index: usize) -> Result<RgbTile, DataError> {
        let level = self
            .levels
            .get(level_index)
            .ok_or_else(|| decode_error("packed-surface level is out of range"))?;
        let local = entry_index
            .checked_sub(level.first_entry)
            .ok_or_else(|| decode_error("packed-surface entry is outside its level"))?;
        let tile_x = local % level.tiles_x;
        let tile_y = local / level.tiles_x;
        if tile_y >= level.tiles_y {
            return Err(decode_error("packed-surface entry is outside its level"));
        }
        let expected_width = self.tile_size.min(level.width - tile_x * self.tile_size);
        let expected_height = self.tile_size.min(level.height - tile_y * self.tile_size);
        let entry = self
            .entries
            .get(entry_index)
            .ok_or_else(|| decode_error("packed-surface entry is out of range"))?;
        let (width, height, rgb) = crate::jpeg::decode_rgb(&self.bytes[entry.payload.clone()])?;
        if width != expected_width || height != expected_height || rgb.len() != width * height * 3 {
            return Err(decode_error(format!(
                "packed-surface tile decoded as {width}x{height}/{} bytes; expected {expected_width}x{expected_height} RGB",
                rgb.len()
            )));
        }
        Ok(RgbTile {
            width,
            height,
            rgb: rgb.into_boxed_slice(),
        })
    }
}

/// One encoded level produced by the offline packer. Tiles are row-major.
pub struct EncodedSurfaceLevel {
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<Vec<u8>>,
}

pub fn encode_pack(tile_size: u16, levels: Vec<EncodedSurfaceLevel>) -> Result<Vec<u8>, DataError> {
    if tile_size == 0 || levels.is_empty() {
        return Err(decode_error("surface pack needs a tile size and levels"));
    }
    let tile_size_usize = tile_size as usize;
    let mut entry_count = 0usize;
    for level in &levels {
        if level.width == 0 || level.height == 0 {
            return Err(decode_error("surface pack level is empty"));
        }
        let expected = (level.width as usize).div_ceil(tile_size_usize)
            * (level.height as usize).div_ceil(tile_size_usize);
        if level.tiles.len() != expected || level.tiles.iter().any(Vec::is_empty) {
            return Err(decode_error(format!(
                "surface pack level has {} tiles, expected {expected}",
                level.tiles.len()
            )));
        }
        entry_count = entry_count
            .checked_add(expected)
            .ok_or_else(|| decode_error("surface pack has too many tiles"))?;
    }
    let level_count = u16::try_from(levels.len())
        .map_err(|_| decode_error("surface pack has too many levels"))?;
    let entry_count_u32 =
        u32::try_from(entry_count).map_err(|_| decode_error("surface pack has too many tiles"))?;
    let payload_bytes =
        levels
            .iter()
            .flat_map(|level| &level.tiles)
            .try_fold(0usize, |sum, tile| {
                sum.checked_add(tile.len())
                    .ok_or_else(|| decode_error("surface pack payload size overflow"))
            })?;
    let capacity = HEADER_LEN + levels.len() * LEVEL_LEN + entry_count * ENTRY_LEN + payload_bytes;
    let mut output = Vec::with_capacity(capacity);
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&tile_size.to_le_bytes());
    output.extend_from_slice(&level_count.to_le_bytes());
    output.extend_from_slice(&entry_count_u32.to_le_bytes());
    let mut first_entry = 0u32;
    for level in &levels {
        output.extend_from_slice(&level.width.to_le_bytes());
        output.extend_from_slice(&level.height.to_le_bytes());
        output.extend_from_slice(&first_entry.to_le_bytes());
        let count = u32::try_from(level.tiles.len())
            .map_err(|_| decode_error("surface pack level has too many tiles"))?;
        output.extend_from_slice(&count.to_le_bytes());
        first_entry += count;
    }
    for level in &levels {
        for tile in &level.tiles {
            let len =
                u32::try_from(tile.len()).map_err(|_| decode_error("surface tile exceeds u32"))?;
            output.extend_from_slice(&len.to_le_bytes());
        }
    }
    for level in levels {
        for tile in level.tiles {
            output.extend_from_slice(&tile);
        }
    }
    debug_assert_eq!(output.len(), capacity);
    Ok(output)
}

struct RgbTile {
    width: usize,
    height: usize,
    rgb: Box<[u8]>,
}

type TileKey = (u8, usize, usize);
type TileSlot = OnceLock<Arc<RgbTile>>;

thread_local! {
    // Built-in packs are immutable across provider instances. Four strong
    // references retain at most 4 * 512 * 512 * 3 RGB bytes per sampling thread,
    // including tiles evicted from a provider's shared cache.
    static LOCAL_TILES: RefCell<[Option<(TileKey, Arc<RgbTile>)>; 4]> =
        const { RefCell::new([None, None, None, None]) };
}

struct TileMap {
    entries: HashMap<TileKey, (Arc<TileSlot>, u64)>,
    clock: u64,
}

/// Network-free global appearance source.
pub struct BundledSurface {
    cache: Mutex<TileMap>,
    month: AtomicU8,
}

impl Default for BundledSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl BundledSurface {
    /// Construct the historical August view. Runtime callers should prefer
    /// [`Self::for_month`] so the atlas follows their simulated calendar.
    pub fn new() -> Self {
        Self::for_month(8)
    }

    pub fn for_month(month: u8) -> Self {
        assert!((1..=MONTHS as u8).contains(&month), "invalid surface month");
        Self {
            cache: Mutex::new(TileMap {
                entries: HashMap::new(),
                clock: 0,
            }),
            month: AtomicU8::new(month),
        }
    }

    pub fn month(&self) -> u8 {
        self.month.load(Ordering::Acquire)
    }

    pub fn set_month(&self, month: u8) -> bool {
        if !(1..=MONTHS as u8).contains(&month) {
            return false;
        }
        self.month.swap(month, Ordering::AcqRel) != month
    }

    fn slots_for(&self, keys: &[TileKey]) -> Vec<(TileKey, Arc<TileSlot>)> {
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        cache.clock += 1;
        let clock = cache.clock;
        let mut unique = Vec::with_capacity(keys.len());
        for &key in keys {
            if unique.iter().any(|(existing, _)| *existing == key) {
                continue;
            }
            let slot = cache
                .entries
                .entry(key)
                .or_insert_with(|| (Arc::new(OnceLock::new()), clock));
            slot.1 = clock;
            unique.push((key, slot.0.clone()));
        }
        if cache.entries.len() > MEMORY_TILES {
            let mut completed: Vec<_> = cache
                .entries
                .iter()
                .filter(|(_, (slot, _))| slot.get().is_some())
                .map(|(key, (_, age))| (*age, *key))
                .collect();
            completed.sort_unstable();
            let remove = cache
                .entries
                .len()
                .saturating_sub(MEMORY_TILES - MEMORY_TILES / 8);
            for (_, key) in completed.into_iter().take(remove) {
                if !keys.contains(&key) {
                    cache.entries.remove(&key);
                }
            }
        }
        unique
    }

    fn sample_level(&self, level_index: usize, lat: f64, lon: f64) -> Option<[u8; 3]> {
        let month = self.month();
        let pack = builtin_for_month(month);
        let level = pack.levels.get(level_index)?;
        let fx = (lon.to_degrees() + 180.0) / 360.0 * level.width as f64 - 0.5;
        let fy = (90.0 - lat.to_degrees()) / 180.0 * level.height as f64 - 0.5;
        let x0 = fx.floor() as i64;
        let y0 = fy.floor() as i64;
        let tx = (fx - x0 as f64) as f32;
        let ty = (fy - y0 as f64) as f32;
        let points = [(x0, y0), (x0 + 1, y0), (x0, y0 + 1), (x0 + 1, y0 + 1)];
        let pixels = points.map(|(x, y)| {
            (
                x.rem_euclid(level.width as i64) as usize,
                y.clamp(0, level.height as i64 - 1) as usize,
            )
        });
        let keys = pixels.map(|(x, y)| {
            let tile_x = x / pack.tile_size;
            let tile_y = y / pack.tile_size;
            let entry = pack
                .tile_entry(level_index, tile_x, tile_y)
                .expect("surface sample must resolve a tile");
            (month, level_index, entry)
        });
        let samples: [[u8; 3]; 4] =
            LOCAL_TILES.with(|local| {
                let mut local = local.borrow_mut();
                let mut missing = [keys[0]; 4];
                let mut missing_count = 0;
                for key in keys {
                    if !local.iter().flatten().any(|(cached, _)| *cached == key)
                        && !missing[..missing_count].contains(&key)
                    {
                        missing[missing_count] = key;
                        missing_count += 1;
                    }
                }
                if missing_count != 0 {
                    for (key, slot) in self.slots_for(&missing[..missing_count]) {
                        let position = local
                            .iter()
                            .position(Option::is_none)
                            .or_else(|| {
                                local.iter().position(|entry| {
                                    entry.as_ref().is_some_and(|(key, _)| !keys.contains(key))
                                })
                            })
                            .expect("four tiles cover every bilinear tap");
                        // Release before decoding; spare tiles survive so alternating
                        // coarse and detail samples can both hit the local cache.
                        local[position] = None;
                        let tile =
                            slot.get_or_init(|| {
                                Arc::new(pack.decode_tile(level_index, key.2).unwrap_or_else(
                                    |error| panic!("invalid bundled surface tile: {error}"),
                                ))
                            });
                        local[position] = Some((key, Arc::clone(tile)));
                    }
                }
                std::array::from_fn(|corner| {
                    let tile = &local
                        .iter()
                        .flatten()
                        .find(|(key, _)| *key == keys[corner])
                        .expect("surface tile must be locally retained")
                        .1;
                    let (x, y) = pixels[corner];
                    let local_x = x % pack.tile_size;
                    let local_y = y % pack.tile_size;
                    debug_assert!(local_x < tile.width && local_y < tile.height);
                    let offset = (local_y * tile.width + local_x) * 3;
                    tile.rgb[offset..offset + 3].try_into().unwrap()
                })
            });
        let mut out = [0u8; 3];
        for (channel, value) in out.iter_mut().enumerate() {
            let [a, b, c, d] = samples.map(|sample| f32::from(sample[channel]));
            let north = a + (b - a) * tx;
            let south = c + (d - c) * tx;
            *value = (north + (south - north) * ty).round().clamp(0.0, 255.0) as u8;
        }
        Some(out)
    }
}

impl ColorSource for BundledSurface {
    fn month(&self) -> Option<u8> {
        Some(BundledSurface::month(self))
    }

    fn color(&self, lat: f64, lon: f64) -> Option<[u8; 3]> {
        self.sample_level(0, lat, lon)
    }

    fn set_month(&self, month: u8) -> bool {
        BundledSurface::set_month(self, month)
    }

    fn color_detail(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> Option<[u8; 3]> {
        (sample_pitch_m <= DETAIL_VOXEL_LIMIT_M)
            .then(|| self.sample_level(1, lat, lon))
            .flatten()
    }
}

pub fn builtin() -> &'static PackedSurface<'static> {
    builtin_for_month(8)
}

pub fn builtin_for_month(month: u8) -> &'static PackedSurface<'static> {
    assert!((1..=MONTHS as u8).contains(&month), "invalid surface month");
    static PACKS: [OnceLock<PackedSurface<'static>>; MONTHS] = [const { OnceLock::new() }; MONTHS];
    let index = month as usize - 1;
    PACKS[index].get_or_init(|| {
        PackedSurface::parse(builtin_bytes(index))
            .unwrap_or_else(|error| panic!("invalid built-in surface pack: {error}"))
    })
}

/// Embedded size of all months; native only, since the browser would have to
/// download every pack.
#[cfg(not(target_arch = "wasm32"))]
pub fn builtin_len() -> usize {
    (0..MONTHS).map(|index| builtin_bytes(index).len()).sum()
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DataError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| decode_error("truncated packed surface"))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DataError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| decode_error("truncated packed surface"))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn decode_error(message: impl Into<String>) -> DataError {
    DataError::Decode(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear_local_tiles() {
        LOCAL_TILES.with(|local| *local.borrow_mut() = [None, None, None, None]);
    }

    fn pixel_location(level: &Level, x: f64, y: f64) -> (f64, f64) {
        (
            (90.0 - (y + 0.5) * 180.0 / level.height as f64).to_radians(),
            ((x + 0.5) * 360.0 / level.width as f64 - 180.0).to_radians(),
        )
    }

    // Independent raw JPEG reads bypass both decoded-tile caches.
    fn direct_sample(month: u8, level_index: usize, lat: f64, lon: f64) -> [u8; 3] {
        let pack = builtin_for_month(month);
        let level = &pack.levels[level_index];
        let fx = (lon.to_degrees() + 180.0) / 360.0 * level.width as f64 - 0.5;
        let fy = (90.0 - lat.to_degrees()) / 180.0 * level.height as f64 - 0.5;
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let taps = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| {
            let x = (x0 + dx).rem_euclid(level.width as i64) as usize;
            let y = (y0 + dy).clamp(0, level.height as i64 - 1) as usize;
            let entry = pack
                .tile_entry(level_index, x / pack.tile_size, y / pack.tile_size)
                .unwrap();
            let tile = pack.decode_tile(level_index, entry).unwrap();
            let offset = ((y % pack.tile_size) * tile.width + x % pack.tile_size) * 3;
            <[u8; 3]>::try_from(&tile.rgb[offset..offset + 3]).unwrap()
        });
        std::array::from_fn(|channel| {
            let [a, b, c, d] = taps.map(|tap| f32::from(tap[channel]));
            let north = a + (b - a) * tx;
            let south = c + (d - c) * tx;
            (north + (south - north) * ty).round().clamp(0.0, 255.0) as u8
        })
    }

    #[test]
    fn bundled_pack_contains_two_levels_for_every_month() {
        for month in 1..=MONTHS as u8 {
            let pack = builtin_for_month(month);
            assert_eq!(pack.level_count(), LEVELS_PER_MONTH);
            assert_eq!(pack.dimensions(0), Some((COARSE_WIDTH, COARSE_HEIGHT)));
            assert_eq!(pack.dimensions(1), Some((DETAIL_WIDTH, DETAIL_HEIGHT)));
            assert!(pack.tile_count() > 900);
        }
    }

    #[test]
    fn grand_canyon_detail_is_warm_bare_ground() {
        let source = BundledSurface::new();
        let rgb = source
            .color_detail(36.1001_f64.to_radians(), -112.0956_f64.to_radians(), 20.0)
            .unwrap();
        assert!(rgb[0] > rgb[1] + 15, "{rgb:?}");
        assert!(rgb[1] > rgb[2] + 10, "{rgb:?}");
    }

    #[test]
    fn detail_gate_and_longitude_wrap_are_stable() {
        let source = BundledSurface::new();
        assert!(source.color_detail(0.2, 0.3, 10_000.0).is_none());
        let west = source.color(0.2, -std::f64::consts::PI + 1e-8).unwrap();
        let east = source.color(0.2, std::f64::consts::PI + 1e-8).unwrap();
        for channel in 0..3 {
            assert!(
                west[channel].abs_diff(east[channel]) <= 2,
                "{west:?} {east:?}"
            );
        }
    }

    #[test]
    fn month_selection_changes_the_sampled_atlas() {
        let source = BundledSurface::for_month(1);
        let january = source.color(50.0_f64.to_radians(), 10.0_f64.to_radians());
        assert!(source.set_month(7));
        let july = source.color(50.0_f64.to_radians(), 10.0_f64.to_radians());
        assert_ne!(january, july);
        assert!(!source.set_month(7));
        assert!(!source.set_month(0));
    }

    #[test]
    fn local_tiles_reuse_both_levels_across_instances_and_month_changes() {
        clear_local_tiles();
        let source = BundledSurface::for_month(1);
        let (lat, lon) = (50.0_f64.to_radians(), 10.0_f64.to_radians());
        let january = std::array::from_fn::<_, LEVELS_PER_MONTH, _>(|level| {
            let expected = direct_sample(1, level, lat, lon);
            assert_eq!(source.sample_level(level, lat, lon), Some(expected));
            expected
        });
        let clock = source.cache.lock().unwrap().clock;
        let other = BundledSurface::for_month(1);
        for _ in 0..32 {
            for (level, expected) in january.into_iter().enumerate() {
                assert_eq!(source.sample_level(level, lat, lon), Some(expected));
                assert_eq!(other.sample_level(level, lat, lon), Some(expected));
            }
        }
        assert_eq!(source.cache.lock().unwrap().clock, clock);
        assert!(other.cache.lock().unwrap().entries.is_empty());
        assert!(source.set_month(7));
        for level in 0..LEVELS_PER_MONTH {
            assert_eq!(
                source.sample_level(level, lat, lon),
                Some(direct_sample(7, level, lat, lon))
            );
        }
        assert!(source.set_month(1));
        for (level, expected) in january.into_iter().enumerate() {
            assert_eq!(source.sample_level(level, lat, lon), Some(expected));
        }
    }

    #[test]
    fn local_tiles_survive_shared_eviction_and_release_replaced_tiles() {
        clear_local_tiles();
        let source = BundledSurface::for_month(8);
        let pack = builtin_for_month(8);
        let (lat, lon) = pixel_location(&pack.levels[1], 511.75, 511.75);
        let expected = direct_sample(8, 1, lat, lon);
        assert_eq!(source.sample_level(1, lat, lon), Some(expected));
        let retained = LOCAL_TILES.with(|local| {
            local
                .borrow()
                .iter()
                .flatten()
                .map(|(key, tile)| (*key, Arc::downgrade(tile)))
                .collect::<Vec<_>>()
        });
        assert_eq!(retained.len(), 4);
        // Exercise shared eviction without changing this thread's current taps.
        for entry in pack.entries.len() - MEMORY_TILES - 8..pack.entries.len() {
            let slots = source.slots_for(&[(8, 1, entry)]);
            slots[0]
                .1
                .get_or_init(|| Arc::new(pack.decode_tile(1, entry).unwrap()));
        }
        let clock = {
            let cache = source.cache.lock().unwrap();
            assert!(cache.entries.len() <= MEMORY_TILES);
            assert!(
                retained
                    .iter()
                    .all(|(key, _)| !cache.entries.contains_key(key))
            );
            cache.clock
        };
        assert_eq!(source.sample_level(1, lat, lon), Some(expected));
        assert_eq!(source.cache.lock().unwrap().clock, clock);
        let (far_lat, far_lon) = pixel_location(&pack.levels[1], 4095.75, 4095.75);
        assert_eq!(
            source.sample_level(1, far_lat, far_lon),
            Some(direct_sample(8, 1, far_lat, far_lon))
        );
        assert!(retained.iter().all(|(_, tile)| tile.upgrade().is_none()));
        LOCAL_TILES.with(|local| {
            let local = local.borrow();
            assert_eq!(local.iter().flatten().count(), 4);
            assert!(
                local
                    .iter()
                    .flatten()
                    .map(|(_, tile)| tile.rgb.len())
                    .sum::<usize>()
                    <= 4 * TILE_SIZE * TILE_SIZE * 3
            );
        });
        assert_eq!(source.sample_level(1, lat, lon), Some(expected));
    }

    #[test]
    fn local_tiles_match_direct_monthly_border_samples_on_multiple_threads() {
        let sources = [1, 7, 12].map(|month| Arc::new(BundledSurface::for_month(month)));
        let mut cases = Vec::new();
        for (source_index, source) in sources.iter().enumerate() {
            let pack = builtin_for_month(source.month());
            for level_index in 0..LEVELS_PER_MONTH {
                for (x, y) in [(511.75, 511.75), (-0.25, -0.25)] {
                    let (lat, lon) = pixel_location(&pack.levels[level_index], x, y);
                    cases.push((
                        source_index,
                        level_index,
                        lat,
                        lon,
                        direct_sample(source.month(), level_index, lat, lon),
                    ));
                }
            }
        }
        vearth_platform::thread::scope(|scope| {
            for offset in 0..4 {
                let sources = &sources;
                let cases = &cases;
                scope.spawn(move || {
                    for index in 0..cases.len() * 2 {
                        let &(source, level, lat, lon, expected) =
                            &cases[(index + offset) % cases.len()];
                        assert_eq!(
                            sources[source].sample_level(level, lat, lon),
                            Some(expected)
                        );
                    }
                });
            }
        });
    }

    #[test]
    fn cached_rgb_taps_match_direct_atlas_samples_at_seams_and_poles() {
        for month in [1, 4, 7, 10, 12] {
            let source = BundledSurface::for_month(month);
            let pack = builtin_for_month(month);
            for level_index in 0..LEVELS_PER_MONTH {
                let level = &pack.levels[level_index];
                for (x, y) in [
                    (-0.25, -0.25),
                    (0.0, 0.0),
                    (511.75, 511.75),
                    (512.0, 512.0),
                    (512.25, 512.75),
                    (level.width as f64 - 0.25, level.height as f64 - 0.25),
                    (level.width as f64 + 0.25, level.height as f64 + 0.25),
                ] {
                    let (lat, lon) = pixel_location(level, x, y);
                    let expected = direct_sample(month, level_index, lat, lon);
                    assert_eq!(
                        source.sample_level(level_index, lat, lon),
                        Some(expected),
                        "month {month}, level {level_index}, pixel {x}/{y}"
                    );
                }
            }
        }
    }
}
