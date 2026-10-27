//! Bounded-memory offline lookup from geodetic coordinates to IANA timezone.
//!
//! The full-precision source geometry is split into independently compressed
//! polygons. Only the compact name/bounding-box index is retained on the heap;
//! candidate polygons are decoded as streams during a lookup and immediately
//! discarded. This avoids materialising a second full-world geometry atlas,
//! and lets the browser fetch only the blocks holding candidate polygons.

use flate2::read::ZlibDecoder;
use std::io::Read;
use std::sync::OnceLock;
use vearth_platform::assets::Blocks;

pub const TIMEZONE_ATLAS_VERSION: u32 = 1;
pub const TIMEZONE_ATLAS_PROVENANCE: &str =
    "Timezone Boundary Builder 2026c, with oceans, zones equivalent since 1970";

fn packed() -> &'static Blocks {
    static PACKED: OnceLock<Blocks> = OnceLock::new();
    PACKED.get_or_init(|| vearth_platform::asset_blocks!("../assets/timezones-2026c.bin"))
}

/// First read of the index, which precedes the polygons; grown on demand.
const INDEX_PREFIX_BYTES: usize = 64 * 1024;
const TRUNCATED_INDEX: &str = "truncated timezone index";
const MAGIC: &[u8; 4] = b"VTZ1";
const WORLD_DEGREES: i64 = 360;

#[derive(Clone, Debug)]
struct PolygonMeta {
    zone_index: usize,
    bbox: [i32; 4],
    offset: usize,
    compressed_len: usize,
    raw_len: u32,
}

#[derive(Debug)]
struct Atlas {
    scale: i32,
    names: Vec<String>,
    polygons: Vec<PolygonMeta>,
}

/// Handle to the bundled world timezone atlas.
#[derive(Clone, Copy, Debug, Default)]
pub struct TimezoneAtlas;

impl TimezoneAtlas {
    pub const fn bundled() -> Self {
        Self
    }

    /// Resolve an IANA timezone name at geodetic latitude/longitude in radians.
    ///
    /// The source includes fixed-offset ocean zones, so every finite coordinate
    /// on Earth should resolve. `None` indicates invalid input or corrupt data.
    pub fn timezone_at(self, lat_rad: f64, lon_rad: f64) -> Option<String> {
        if !lat_rad.is_finite() || !lon_rad.is_finite() {
            return None;
        }
        let atlas = atlas();
        let latitude = lat_rad.to_degrees().clamp(-90.0, 90.0);
        let longitude = (lon_rad.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
        let point = [
            (longitude * f64::from(atlas.scale)).round() as i32,
            (latitude * f64::from(atlas.scale)).round() as i32,
        ];

        // The pack is ordered from tightest bbox to broadest. Source polygons
        // are non-overlapping away from shared edges; tightest-first makes the
        // exact-edge result deterministic without decoding every candidate.
        for polygon in &atlas.polygons {
            if !bbox_contains(polygon.bbox, point, atlas.scale) {
                continue;
            }
            let end = polygon.offset.checked_add(polygon.compressed_len)?;
            let compressed = match packed().read(polygon.offset..end) {
                Ok(compressed) => compressed,
                Err(error) => {
                    log::error!("timezone polygon read: {error}");
                    return None;
                }
            };
            match polygon_contains(&compressed, polygon.raw_len, point, atlas.scale) {
                Ok(true) => return atlas.names.get(polygon.zone_index).cloned(),
                Ok(false) => {}
                Err(error) => {
                    log::error!("timezone polygon decode: {error}");
                    return None;
                }
            }
        }
        None
    }
}

fn bbox_contains(bbox: [i32; 4], point: [i32; 2], scale: i32) -> bool {
    if point[1] < bbox[1] || point[1] > bbox[3] {
        return false;
    }
    let world = WORLD_DEGREES * i64::from(scale);
    let width = i64::from(bbox[2]) - i64::from(bbox[0]);
    width > world / 2 || (point[0] >= bbox[0] && point[0] <= bbox[2])
}

fn unwrap_longitude(longitude: i32, around: i64, scale: i32) -> i64 {
    let world = WORLD_DEGREES * i64::from(scale);
    let half = world / 2;
    let delta = (i64::from(longitude) - around + half).rem_euclid(world) - half;
    around + delta
}

fn edge_crosses(point_x: i64, point_y: i64, ax: i64, ay: i64, bx: i64, by: i64) -> bool {
    if (ay > point_y) == (by > point_y) {
        return false;
    }
    let intersection_x = ax as f64 + (point_y - ay) as f64 * (bx - ax) as f64 / (by - ay) as f64;
    (point_x as f64) < intersection_x
}

fn polygon_contains(
    compressed: &[u8],
    expected_raw_len: u32,
    point: [i32; 2],
    scale: i32,
) -> Result<bool, String> {
    let mut decoder = ZlibDecoder::new(compressed);
    let ring_count = read_u16(&mut decoder)?;
    let point_x = i64::from(point[0]);
    let point_y = i64::from(point[1]);
    let mut inside = false;

    for _ in 0..ring_count {
        let point_count = read_u32(&mut decoder)?;
        if point_count < 3 {
            return Err("timezone ring has fewer than three points".into());
        }
        let mut raw_x = read_signed_varint(&mut decoder)?;
        let mut raw_y = read_signed_varint(&mut decoder)?;
        let first_x = unwrap_longitude(raw_x, point_x, scale);
        let first_y = i64::from(raw_y);
        let mut ax = first_x;
        let mut ay = first_y;

        for _ in 1..point_count {
            raw_x = raw_x
                .checked_add(read_signed_varint(&mut decoder)?)
                .ok_or_else(|| "timezone longitude overflow".to_string())?;
            raw_y = raw_y
                .checked_add(read_signed_varint(&mut decoder)?)
                .ok_or_else(|| "timezone latitude overflow".to_string())?;
            let bx = unwrap_longitude(raw_x, ax, scale);
            let by = i64::from(raw_y);
            inside ^= edge_crosses(point_x, point_y, ax, ay, bx, by);
            ax = bx;
            ay = by;
        }
        inside ^= edge_crosses(point_x, point_y, ax, ay, first_x, first_y);
    }

    let mut trailing = [0_u8; 1];
    if decoder
        .read(&mut trailing)
        .map_err(|error| format!("timezone zlib: {error}"))?
        != 0
    {
        return Err("trailing timezone polygon bytes".into());
    }
    if decoder.total_out() != u64::from(expected_raw_len) {
        return Err("timezone polygon length mismatch".into());
    }
    Ok(inside)
}

fn read_exact<const N: usize>(reader: &mut impl Read) -> Result<[u8; N], String> {
    let mut bytes = [0_u8; N];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| format!("timezone data: {error}"))?;
    Ok(bytes)
}

fn read_u16(reader: &mut impl Read) -> Result<u16, String> {
    read_exact(reader).map(u16::from_le_bytes)
}

fn read_u32(reader: &mut impl Read) -> Result<u32, String> {
    read_exact(reader).map(u32::from_le_bytes)
}

fn read_signed_varint(reader: &mut impl Read) -> Result<i32, String> {
    let mut unsigned = 0_u32;
    for shift in (0..=28).step_by(7) {
        let byte = read_exact::<1>(reader)?[0];
        if shift == 28 && byte > 0x0f {
            return Err("timezone varint overflow".into());
        }
        unsigned |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(((unsigned >> 1) as i32) ^ -((unsigned & 1) as i32));
        }
    }
    Err("timezone varint is too long".into())
}

fn atlas() -> &'static Atlas {
    static ATLAS: OnceLock<Atlas> = OnceLock::new();
    ATLAS.get_or_init(|| read_index(packed()).expect("bundled timezone atlas is valid"))
}

fn read_index(packed: &Blocks) -> Result<Atlas, String> {
    let mut prefix = INDEX_PREFIX_BYTES.min(packed.len());
    loop {
        match decode_index(&packed.read(0..prefix)?, packed.len()) {
            Err(error) if error == TRUNCATED_INDEX && prefix < packed.len() => {
                prefix = (prefix * 2).min(packed.len());
            }
            result => return result,
        }
    }
}

struct SliceReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> SliceReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let end = self.position.saturating_add(N);
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| TRUNCATED_INDEX.to_string())?;
        self.position = end;
        Ok(bytes.try_into().expect("slice length checked"))
    }

    fn u16(&mut self) -> Result<u16, String> {
        self.take().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.take().map(u32::from_le_bytes)
    }

    fn i32(&mut self) -> Result<i32, String> {
        self.take().map(i32::from_le_bytes)
    }

    fn string(&mut self) -> Result<String, String> {
        let length = usize::from(self.u16()?);
        let end = self.position.saturating_add(length);
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| TRUNCATED_INDEX.to_string())?;
        self.position = end;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| "invalid UTF-8 in timezone index".to_string())
    }
}

/// Decodes the index from a prefix of the `total_len`-byte atlas.
fn decode_index(bytes: &[u8], total_len: usize) -> Result<Atlas, String> {
    let mut reader = SliceReader::new(bytes);
    if &reader.take::<4>()? != MAGIC {
        return Err("invalid timezone atlas magic".into());
    }
    let version = reader.u32()?;
    if version != TIMEZONE_ATLAS_VERSION {
        return Err(format!("unsupported timezone atlas version {version}"));
    }
    let scale = i32::try_from(reader.u32()?).map_err(|_| "invalid timezone scale")?;
    if scale <= 0 {
        return Err("invalid timezone scale".into());
    }
    let name_count = usize::try_from(reader.u32()?).map_err(|_| "invalid timezone count")?;
    let polygon_count =
        usize::try_from(reader.u32()?).map_err(|_| "invalid timezone polygon count")?;
    let mut names = Vec::with_capacity(name_count);
    for _ in 0..name_count {
        names.push(reader.string()?);
    }
    let mut polygons = Vec::with_capacity(polygon_count);
    for _ in 0..polygon_count {
        let zone_index = usize::from(reader.u16()?);
        if zone_index >= names.len() {
            return Err("timezone polygon has invalid name index".into());
        }
        let bbox = [reader.i32()?, reader.i32()?, reader.i32()?, reader.i32()?];
        let offset = usize::try_from(reader.u32()?).map_err(|_| "invalid polygon offset")?;
        let compressed_len =
            usize::try_from(reader.u32()?).map_err(|_| "invalid polygon length")?;
        let raw_len = reader.u32()?;
        if reader.u32()? != 0 {
            return Err("timezone polygon reserved field is nonzero".into());
        }
        if offset
            .checked_add(compressed_len)
            .is_none_or(|end| end > total_len)
        {
            return Err("timezone polygon block is out of bounds".into());
        }
        polygons.push(PolygonMeta {
            zone_index,
            bbox,
            offset,
            compressed_len,
            raw_len,
        });
    }
    if polygons
        .first()
        .is_some_and(|polygon| polygon.offset < reader.position)
    {
        return Err("timezone polygon overlaps index".into());
    }
    Ok(Atlas {
        scale,
        names,
        polygons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(lat: f64, lon: f64) -> String {
        TimezoneAtlas::bundled()
            .timezone_at(lat.to_radians(), lon.to_radians())
            .unwrap_or_else(|| panic!("no timezone at {lat}, {lon}"))
    }

    #[test]
    fn full_precision_atlas_covers_civil_and_fractional_offset_zones() {
        assert_eq!(zone(-33.8688, 151.2093), "Australia/Sydney");
        assert_eq!(zone(27.7172, 85.3240), "Asia/Kathmandu");
        assert_eq!(zone(-31.6764, 128.8833), "Australia/Eucla");
        assert_eq!(zone(-43.95, -176.55), "Pacific/Chatham");
        assert_eq!(zone(33.4484, -112.0740), "America/Phoenix");
        assert_eq!(zone(31.2304, 121.4737), "Asia/Shanghai");
    }

    #[test]
    fn ocean_and_antimeridian_are_covered() {
        assert!(!zone(0.0, -140.0).is_empty());
        assert!(!zone(0.0, 179.9).is_empty());
        assert!(!zone(0.0, -179.9).is_empty());
    }

    #[test]
    fn index_reads_grow_until_the_index_fits() {
        let bundled: &[u8] = include_bytes!("../assets/timezones-2026c.bin");
        assert_eq!(
            decode_index(&bundled[..1024], bundled.len()).unwrap_err(),
            TRUNCATED_INDEX
        );
        let atlas = read_index(packed()).unwrap();
        assert_eq!(atlas.polygons.len(), self::atlas().polygons.len());
    }

    #[test]
    fn resident_index_is_small_and_geometry_stays_packed() {
        let atlas = atlas();
        let heap_bytes = atlas.names.iter().map(String::capacity).sum::<usize>()
            + atlas.polygons.capacity() * std::mem::size_of::<PolygonMeta>();
        assert!(
            heap_bytes < 128 * 1024,
            "timezone index uses {heap_bytes} bytes"
        );
        assert!(packed().len() < 24 * 1024 * 1024);
    }
}
