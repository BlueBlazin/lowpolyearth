//! Compact, lazily decoded global terrain bundled with the application.
//!
//! Packs contain independently zlib-compressed 512×512 tiles. Heights are
//! quantized in integer meters and horizontally delta-coded before
//! compression. Independent tile payloads preserve the existing Terrarium
//! cache behavior: only tiles actually sampled by the current view are
//! decoded, and finer zooms continue through the normal disk/network path.

use crate::DataError;
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use std::io::{Read, Write};
use std::ops::Range;
use std::sync::OnceLock;

pub const TILE_SIZE: usize = crate::terrarium::TILE_SIZE;
pub const NATIVE_MAX_ZOOM: u8 = 3;
pub const NATIVE_QUANTIZATION_M: u16 = 1;
pub const NATIVE_BUDGET_BYTES: usize = 50 * 1024 * 1024;

const MAGIC: &[u8; 8] = b"VETRPK01";
const HEADER_LEN: usize = 16;
const ENTRY_LEN: usize = 16;
const RAW_TILE_BYTES: usize = TILE_SIZE * TILE_SIZE * 2;

fn builtin_bytes() -> &'static [u8] {
    vearth_platform::asset_bytes!("../assets/terrain-native.vtp")
}

// A build cannot accidentally ship an oversized pack, even if the offline
// generator's own budget check is bypassed.
#[cfg(not(target_arch = "wasm32"))]
const _: () = assert!(include_bytes!("../assets/terrain-native.vtp").len() <= NATIVE_BUDGET_BYTES);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct TileKey {
    z: u8,
    x: u32,
    y: u32,
}

#[derive(Clone, Debug)]
struct Entry {
    key: TileKey,
    payload: Range<usize>,
}

/// Parsed view of an immutable terrain pack.
pub struct PackedTerrain<'a> {
    bytes: &'a [u8],
    quantization_m: u16,
    max_zoom: u8,
    entries: Vec<Entry>,
}

impl<'a> PackedTerrain<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, DataError> {
        if bytes.len() < HEADER_LEN || &bytes[..MAGIC.len()] != MAGIC {
            return Err(decode_error("bad packed-terrain header"));
        }
        let tile_size = read_u16(bytes, 8)? as usize;
        if tile_size != TILE_SIZE {
            return Err(decode_error(format!(
                "packed terrain tile size {tile_size}, expected {TILE_SIZE}"
            )));
        }
        let quantization_m = read_u16(bytes, 10)?;
        if quantization_m == 0 {
            return Err(decode_error("packed terrain has zero quantization"));
        }
        let count = read_u32(bytes, 12)? as usize;
        let payload_start = HEADER_LEN
            .checked_add(
                count
                    .checked_mul(ENTRY_LEN)
                    .ok_or_else(|| decode_error("packed terrain index overflow"))?,
            )
            .ok_or_else(|| decode_error("packed terrain index overflow"))?;
        if payload_start > bytes.len() {
            return Err(decode_error("truncated packed-terrain index"));
        }

        let mut entries = Vec::with_capacity(count);
        let mut payload_offset = payload_start;
        let mut previous = None;
        let mut max_zoom = 0;
        for index in 0..count {
            let at = HEADER_LEN + index * ENTRY_LEN;
            let z = bytes[at];
            if z > 30 {
                return Err(decode_error(format!("invalid packed terrain zoom {z}")));
            }
            let x = read_u32(bytes, at + 4)?;
            let y = read_u32(bytes, at + 8)?;
            let n = 1u32 << z;
            if x >= n || y >= n {
                return Err(decode_error(format!(
                    "packed terrain tile {z}/{x}/{y} is out of range"
                )));
            }
            let key = TileKey { z, x, y };
            if previous.is_some_and(|prior| prior >= key) {
                return Err(decode_error("packed terrain index is not strictly sorted"));
            }
            previous = Some(key);
            max_zoom = max_zoom.max(z);

            let compressed_len = read_u32(bytes, at + 12)? as usize;
            let payload_end = payload_offset
                .checked_add(compressed_len)
                .ok_or_else(|| decode_error("packed terrain payload overflow"))?;
            if payload_end > bytes.len() {
                return Err(decode_error("truncated packed-terrain payload"));
            }
            entries.push(Entry {
                key,
                payload: payload_offset..payload_end,
            });
            payload_offset = payload_end;
        }
        if payload_offset != bytes.len() {
            return Err(decode_error("packed terrain has trailing bytes"));
        }

        Ok(Self {
            bytes,
            quantization_m,
            max_zoom,
            entries,
        })
    }

    pub fn quantization_m(&self) -> u16 {
        self.quantization_m
    }

    pub fn max_zoom(&self) -> u8 {
        self.max_zoom
    }

    pub fn tile_count(&self) -> usize {
        self.entries.len()
    }

    pub fn contains(&self, z: u8, x: u32, y: u32) -> bool {
        self.entry(z, x, y).is_some()
    }

    /// Decode one tile into row-major meters. `Ok(None)` means the requested
    /// zoom lies above the bundled baseline and should use the normal source.
    pub fn decode_tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Box<[f32]>>, DataError> {
        let Some(entry) = self.entry(z, x, y) else {
            return Ok(None);
        };
        let mut decoder = ZlibDecoder::new(&self.bytes[entry.payload.clone()]);
        let mut raw = Vec::with_capacity(RAW_TILE_BYTES);
        decoder
            .read_to_end(&mut raw)
            .map_err(|error| decode_error(format!("packed terrain inflate: {error}")))?;
        if raw.len() != RAW_TILE_BYTES {
            return Err(decode_error(format!(
                "packed terrain tile decoded to {} bytes, expected {RAW_TILE_BYTES}",
                raw.len()
            )));
        }

        let quantum = self.quantization_m as f32;
        let mut heights = vec![0.0; TILE_SIZE * TILE_SIZE].into_boxed_slice();
        for row in 0..TILE_SIZE {
            let mut value = 0i32;
            for column in 0..TILE_SIZE {
                let sample = row * TILE_SIZE + column;
                let byte = sample * 2;
                value += i16::from_le_bytes([raw[byte], raw[byte + 1]]) as i32;
                heights[sample] = value as f32 * quantum;
            }
        }
        Ok(Some(heights))
    }

    fn entry(&self, z: u8, x: u32, y: u32) -> Option<&Entry> {
        let key = TileKey { z, x, y };
        self.entries
            .binary_search_by_key(&key, |entry| entry.key)
            .ok()
            .map(|index| &self.entries[index])
    }
}

/// One compressed tile produced by the offline pack generator.
pub struct EncodedTerrainTile {
    key: TileKey,
    payload: Vec<u8>,
}

pub fn encode_tile(
    z: u8,
    x: u32,
    y: u32,
    heights: &[f32],
    quantization_m: u16,
) -> Result<EncodedTerrainTile, DataError> {
    if heights.len() != TILE_SIZE * TILE_SIZE {
        return Err(decode_error(format!(
            "terrain pack input has {} samples, expected {}",
            heights.len(),
            TILE_SIZE * TILE_SIZE
        )));
    }
    if quantization_m == 0 {
        return Err(decode_error("terrain pack quantization must be positive"));
    }
    let n = 1u32
        .checked_shl(z as u32)
        .ok_or_else(|| decode_error(format!("terrain pack zoom {z} is too large")))?;
    if x >= n || y >= n {
        return Err(decode_error(format!(
            "terrain pack input tile {z}/{x}/{y} is out of range"
        )));
    }

    let mut raw = Vec::with_capacity(RAW_TILE_BYTES);
    for row in 0..TILE_SIZE {
        let mut previous = 0i32;
        for column in 0..TILE_SIZE {
            let height = heights[row * TILE_SIZE + column];
            if !height.is_finite() {
                return Err(decode_error(
                    "terrain pack input contains a non-finite height",
                ));
            }
            let mut value = (height / quantization_m as f32).round() as i32;
            // The generator classifies raw DEM ocean as h < 0. Preserve that
            // predicate: naive rounding could turn shallow water into exact
            // sea level and visibly perturb the coarse sea mask. Low positive
            // land may safely round to zero because both remain non-ocean.
            if value == 0 && height < 0.0 {
                value = -1;
            }
            if !(i16::MIN as i32..=i16::MAX as i32).contains(&value) {
                return Err(decode_error(format!(
                    "terrain height {height} m exceeds packed i16 range"
                )));
            }
            let delta = value - previous;
            let delta = i16::try_from(delta)
                .map_err(|_| decode_error(format!("terrain row delta {delta} exceeds i16")))?;
            raw.extend_from_slice(&delta.to_le_bytes());
            previous = value;
        }
    }

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(&raw)
        .map_err(|error| decode_error(format!("packed terrain deflate: {error}")))?;
    let payload = encoder
        .finish()
        .map_err(|error| decode_error(format!("packed terrain deflate: {error}")))?;
    Ok(EncodedTerrainTile {
        key: TileKey { z, x, y },
        payload,
    })
}

pub fn encode_pack(
    quantization_m: u16,
    mut tiles: Vec<EncodedTerrainTile>,
) -> Result<Vec<u8>, DataError> {
    if quantization_m == 0 {
        return Err(decode_error("terrain pack quantization must be positive"));
    }
    tiles.sort_unstable_by_key(|tile| tile.key);
    if tiles.windows(2).any(|pair| pair[0].key == pair[1].key) {
        return Err(decode_error("terrain pack contains duplicate tiles"));
    }
    let count = u32::try_from(tiles.len())
        .map_err(|_| decode_error("terrain pack contains too many tiles"))?;
    let payload_bytes = tiles.iter().try_fold(0usize, |total, tile| {
        total
            .checked_add(tile.payload.len())
            .ok_or_else(|| decode_error("terrain pack payload size overflow"))
    })?;
    let index_bytes = tiles
        .len()
        .checked_mul(ENTRY_LEN)
        .ok_or_else(|| decode_error("terrain pack index size overflow"))?;
    let capacity = HEADER_LEN
        .checked_add(index_bytes)
        .and_then(|size| size.checked_add(payload_bytes))
        .ok_or_else(|| decode_error("terrain pack size overflow"))?;

    let mut output = Vec::with_capacity(capacity);
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&(TILE_SIZE as u16).to_le_bytes());
    output.extend_from_slice(&quantization_m.to_le_bytes());
    output.extend_from_slice(&count.to_le_bytes());
    for tile in &tiles {
        output.push(tile.key.z);
        output.extend_from_slice(&[0; 3]);
        output.extend_from_slice(&tile.key.x.to_le_bytes());
        output.extend_from_slice(&tile.key.y.to_le_bytes());
        let len = u32::try_from(tile.payload.len())
            .map_err(|_| decode_error("compressed terrain tile exceeds u32"))?;
        output.extend_from_slice(&len.to_le_bytes());
    }
    for tile in tiles {
        output.extend_from_slice(&tile.payload);
    }
    debug_assert_eq!(output.len(), capacity);
    Ok(output)
}

/// Target-specific pack compiled into the application.
pub fn builtin() -> &'static PackedTerrain<'static> {
    static PACK: OnceLock<PackedTerrain<'static>> = OnceLock::new();
    PACK.get_or_init(|| {
        PackedTerrain::parse(builtin_bytes())
            .unwrap_or_else(|error| panic!("invalid built-in terrain pack: {error}"))
    })
}

/// Embedded size; native only, since the browser would have to download it.
#[cfg(not(target_arch = "wasm32"))]
pub fn builtin_len() -> usize {
    builtin_bytes().len()
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DataError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| decode_error("truncated packed terrain"))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DataError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| decode_error("truncated packed terrain"))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn decode_error(message: impl Into<String>) -> DataError {
    DataError::Decode(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_round_trip_preserves_sign_and_quantization_bound() {
        let heights: Vec<f32> = (0..TILE_SIZE * TILE_SIZE)
            .map(|sample| match sample % 7 {
                0 => -1.0,
                1 => 1.0,
                2 => -9_000.25,
                3 => 8_848.75,
                4 => 0.0,
                5 => -17.9,
                _ => 31.1,
            })
            .collect();
        let bytes = encode_pack(4, vec![encode_tile(0, 0, 0, &heights, 4).unwrap()]).unwrap();
        let pack = PackedTerrain::parse(&bytes).unwrap();
        let decoded = pack.decode_tile(0, 0, 0).unwrap().unwrap();
        for (&before, &after) in heights.iter().zip(decoded.iter()) {
            assert_eq!(before < 0.0, after < 0.0);
            assert!((before - after).abs() <= 4.0, "{before} -> {after}");
        }
    }

    #[test]
    fn missing_tile_falls_through() {
        let heights = vec![100.0; TILE_SIZE * TILE_SIZE];
        let bytes = encode_pack(1, vec![encode_tile(0, 0, 0, &heights, 1).unwrap()]).unwrap();
        let pack = PackedTerrain::parse(&bytes).unwrap();
        assert!(pack.contains(0, 0, 0));
        assert!(!pack.contains(1, 0, 0));
        assert!(pack.decode_tile(1, 0, 0).unwrap().is_none());
    }

    #[test]
    fn builtin_pack_matches_target_policy() {
        let pack = builtin();
        assert_eq!(pack.max_zoom(), NATIVE_MAX_ZOOM);
        assert_eq!(pack.quantization_m(), NATIVE_QUANTIZATION_M);
        assert!(builtin_len() <= NATIVE_BUDGET_BYTES);
        for entry in &pack.entries {
            let decoded = pack
                .decode_tile(entry.key.z, entry.key.x, entry.key.y)
                .unwrap()
                .unwrap();
            assert_eq!(decoded.len(), TILE_SIZE * TILE_SIZE);
            assert!(decoded.iter().all(|height| height.is_finite()));
        }
    }
}
