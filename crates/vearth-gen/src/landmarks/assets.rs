//! Bounded indexed landmark source meshes, decoded once per asset.

use glam::{Vec2, Vec3};
use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Mutex, OnceLock},
};
use vearth_core::mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal};

const GRID: usize = 32;
const MAX_RAW_BYTES: usize = 256 * 1024 * 1024;

pub(super) struct Level {
    pub mesh: MeshBuffers,
    pub normals: Vec<[f32; 3]>,
    pub low: Vec3,
    pub high: Vec3,
    bins: Vec<Vec<u32>>,
}
impl Level {
    pub fn candidates(&self, low: Vec2, high: Vec2) -> Vec<u32> {
        let lo = self.low.xz();
        let hi = self.high.xz();
        if low.x > hi.x || low.y > hi.y || high.x < lo.x || high.y < lo.y {
            return Vec::new();
        }
        let scale = Vec2::splat(GRID as f32) / (hi - lo).max(Vec2::splat(0.01));
        let first = ((low - lo) * scale)
            .floor()
            .clamp(Vec2::ZERO, Vec2::splat((GRID - 1) as f32));
        let last = ((high - lo) * scale)
            .floor()
            .clamp(Vec2::ZERO, Vec2::splat((GRID - 1) as f32));
        let mut result = Vec::new();
        for z in first.y as usize..=last.y as usize {
            for x in first.x as usize..=last.x as usize {
                result.extend_from_slice(&self.bins[z * GRID + x]);
            }
        }
        result.sort_unstable();
        result.dedup();
        result
    }
}
use glam::Vec3Swizzles;

/// A landmark's levels of detail, finest first. Native builds decode every
/// level of the embedded asset at once. The browser downloads each level on
/// first use from the per-level files `split_levels` produces, since a view
/// rarely needs the finest level, which holds most of the bytes.
pub(super) struct Asset {
    #[cfg(not(target_arch = "wasm32"))]
    levels: Vec<Level>,
    #[cfg(target_arch = "wasm32")]
    name: String,
    #[cfg(target_arch = "wasm32")]
    levels: Vec<OnceLock<Level>>,
}

impl Asset {
    #[cfg(test)]
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Level `lod`, or the coarsest when the asset has fewer levels.
    pub fn level(&self, lod: usize) -> &Level {
        let lod = lod.min(self.levels.len() - 1);
        #[cfg(not(target_arch = "wasm32"))]
        return &self.levels[lod];
        #[cfg(target_arch = "wasm32")]
        return self.levels[lod].get_or_init(|| {
            let path = level_path(&self.name, lod);
            let bytes = vearth_platform::assets::download_uncached(&path)
                .and_then(|bytes| bytes.ok_or_else(|| "not published".into()))
                .unwrap_or_else(|error| panic!("landmark {path}: {error}"));
            parse_level_payload(&bytes)
                .unwrap_or_else(|error| panic!("invalid landmark {path}: {error}"))
        });
    }
}

/// `[u32 raw length][zlib stream]`, the framing of whole assets and of the
/// per-level payloads.
fn inflate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let expected = bytes.get(..4).ok_or("truncated asset")?;
    let expected = u32::from_le_bytes(expected.try_into().unwrap()) as usize;
    if expected > MAX_RAW_BYTES {
        return Err("landmark decode budget".into());
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&bytes[4..])
        .take(MAX_RAW_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() != expected {
        return Err("landmark decoded size mismatch".into());
    }
    Ok(raw)
}

fn deflate(raw: &[u8]) -> Vec<u8> {
    let mut framed = (raw.len() as u32).to_le_bytes().to_vec();
    let mut encoder = flate2::write::ZlibEncoder::new(&mut framed, flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, raw).expect("in-memory zlib");
    encoder.finish().expect("in-memory zlib");
    framed
}

/// The level sections of a whole asset, with its header checked.
fn level_sections(raw: &[u8]) -> Result<Vec<&[u8]>, String> {
    let mut cursor = Cursor {
        bytes: raw,
        offset: 0,
    };
    if cursor.take(4)? != b"LMS1" {
        return Err("landmark format".into());
    }
    let count = cursor.u32()? as usize;
    if count == 0 || count > 4 {
        return Err("landmark LOD count".into());
    }
    let mut sections = Vec::with_capacity(count);
    for _ in 0..count {
        let start = cursor.offset;
        let vertex_count = cursor.u32()? as usize;
        let index_count = cursor.u32()? as usize;
        if vertex_count > 4_000_000 || index_count > 12_000_000 || index_count % 3 != 0 {
            return Err("landmark geometry count".into());
        }
        cursor.take(vertex_count * 28 + index_count * 4)?;
        sections.push(&raw[start..cursor.offset]);
    }
    if cursor.offset != raw.len() {
        return Err("trailing landmark data".into());
    }
    Ok(sections)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse(bytes: &[u8]) -> Result<Asset, String> {
    let raw = inflate(bytes)?;
    let levels = level_sections(&raw)?
        .into_iter()
        .map(parse_level)
        .collect::<Result<_, _>>()?;
    Ok(Asset { levels })
}

/// Splits a whole asset into standalone per-level payloads, finest first,
/// for publishing to the browser.
pub fn split_levels(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let raw = inflate(bytes)?;
    Ok(level_sections(&raw)?.into_iter().map(deflate).collect())
}

#[cfg(any(target_arch = "wasm32", test))]
fn parse_level_payload(bytes: &[u8]) -> Result<Level, String> {
    parse_level(&inflate(bytes)?)
}

fn parse_level(section: &[u8]) -> Result<Level, String> {
    let mut cursor = Cursor {
        bytes: section,
        offset: 0,
    };
    let vertex_count = cursor.u32()? as usize;
    let index_count = cursor.u32()? as usize;
    if vertex_count > 4_000_000 || index_count > 12_000_000 || index_count % 3 != 0 {
        return Err("landmark geometry count".into());
    }
    let mut mesh = MeshBuffers::default();
    let mut normals = Vec::with_capacity(vertex_count);
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for _ in 0..vertex_count {
        let pos = [cursor.f32()?, cursor.f32()?, cursor.f32()?];
        let normal = [cursor.f32()?, cursor.f32()?, cursor.f32()?];
        let color: [u8; 4] = cursor.take(4)?.try_into().unwrap();
        let point = Vec3::from_array(pos);
        let n = Vec3::from_array(normal);
        if !point.is_finite()
            || !n.is_finite()
            || n.length_squared() < 0.5
            || n.length_squared() > 1.5
            || color[3] == 0
        {
            return Err("invalid landmark vertex".into());
        }
        low = low.min(point);
        high = high.max(point);
        mesh.vertices.push(SurfaceVertex {
            pos,
            face_ao: encode_surface_normal(2 | (3 << 3) | (u32::from(color[3]) << 5), normal),
            rgba: [color[0], color[1], color[2], 255],
        });
        normals.push(normal);
    }
    for _ in 0..index_count {
        let index = cursor.u32()?;
        if index as usize >= vertex_count {
            return Err("landmark index".into());
        }
        mesh.indices.push(index);
    }
    if mesh.indices.is_empty() {
        return Err("empty landmark".into());
    }
    if cursor.offset != section.len() {
        return Err("trailing landmark data".into());
    }
    let mut bins = vec![Vec::new(); GRID * GRID];
    let scale = Vec2::splat(GRID as f32) / (high - low).xz().max(Vec2::splat(0.01));
    for (index, ids) in mesh.indices.chunks_exact(3).enumerate() {
        let p = ids
            .iter()
            .map(|&i| Vec3::from_array(mesh.vertices[i as usize].pos).xz())
            .collect::<Vec<_>>();
        let a = p.iter().copied().reduce(Vec2::min).unwrap();
        let b = p.iter().copied().reduce(Vec2::max).unwrap();
        let first = ((a - low.xz()) * scale)
            .floor()
            .clamp(Vec2::ZERO, Vec2::splat((GRID - 1) as f32));
        let last = ((b - low.xz()) * scale)
            .floor()
            .clamp(Vec2::ZERO, Vec2::splat((GRID - 1) as f32));
        for z in first.y as usize..=last.y as usize {
            for x in first.x as usize..=last.x as usize {
                bins[z * GRID + x].push(index as u32);
            }
        }
    }
    Ok(Level {
        mesh,
        normals,
        low,
        high,
        bins,
    })
}

/// Published path of one level of a landmark in the browser build.
pub fn level_path(name: &str, lod: usize) -> String {
    format!("generated/landmarks/{name}/{lod}.bin")
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.offset.checked_add(n).ok_or("landmark overflow")?;
        let data = self
            .bytes
            .get(self.offset..end)
            .ok_or("truncated landmark")?;
        self.offset = end;
        Ok(data)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn get(name: &str) -> Option<Arc<Asset>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<Asset>>>>> = OnceLock::new();
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(name.to_owned())
        .or_insert_with(|| {
            let count = (0..4)
                .take_while(|&lod| vearth_platform::assets::is_published(&level_path(name, lod)))
                .count();
            (count > 0).then(|| {
                Arc::new(Asset {
                    name: name.to_owned(),
                    levels: (0..count).map(|_| OnceLock::new()).collect(),
                })
            })
        })
        .clone()
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn get(name: &str) -> Option<Arc<Asset>> {
    type Entry = Arc<OnceLock<Arc<Asset>>>;
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Entry>>> = OnceLock::new();
    let (name, bytes) = bytes(name)?;
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let entry = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(name)
        .or_insert_with(|| Arc::new(OnceLock::new()))
        .clone();
    Some(
        entry
            .get_or_init(|| {
                Arc::new(
                    parse(bytes).unwrap_or_else(|e| panic!("invalid indexed landmark {name}: {e}")),
                )
            })
            .clone(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn bytes(name: &str) -> Option<(&'static str, &'static [u8])> {
    if let Some(asset) = super::island_generated::bytes(name) {
        return Some(asset);
    }
    if let Some(asset) = super::monument_generated::bytes(name) {
        return Some(asset);
    }
    if let Some(asset) = super::skyscraper_generated::bytes(name) {
        return Some(asset);
    }
    if let Some(asset) = super::heritage_generated::bytes(name) {
        return Some(asset);
    }
    if let Some(asset) = super::city_generated::bytes(name) {
        return Some(asset);
    }
    macro_rules! asset {
        ($name:literal) => {
            (
                $name,
                vearth_platform::asset_bytes!(concat!(
                    "../../assets/landmarks/surfaces/",
                    $name,
                    ".lms"
                )),
            )
        };
    }
    Some(match name {
        "angkor-wat" => asset!("angkor-wat"),
        "arc-de-triomphe" => asset!("arc-de-triomphe"),
        "brandenburg-gate" => asset!("brandenburg-gate"),
        "brooklyn-bridge" => asset!("brooklyn-bridge"),
        "buckingham-palace" => asset!("buckingham-palace"),
        "burj-al-arab" => asset!("burj-al-arab"),
        "burj-khalifa" => asset!("burj-khalifa"),
        "christ-the-redeemer" => asset!("christ-the-redeemer"),
        "chrysler-building" => asset!("chrysler-building"),
        "cn-tower" => asset!("cn-tower"),
        "colosseum" => asset!("colosseum"),
        "eiffel-tower" => asset!("eiffel-tower"),
        "empire-state-building" => asset!("empire-state-building"),
        "flatiron-building" => asset!("flatiron-building"),
        "gateway-arch" => asset!("gateway-arch"),
        "giza-necropolis" => asset!("giza-necropolis"),
        "golden-gate-bridge" => asset!("golden-gate-bridge"),
        "golden-temple" => asset!("golden-temple"),
        "himeji-castle" => asset!("himeji-castle"),
        "leaning-tower-of-pisa" => asset!("leaning-tower-of-pisa"),
        "london-eye" => asset!("london-eye"),
        "louvre-pyramid" => asset!("louvre-pyramid"),
        "marina-bay-sands" => asset!("marina-bay-sands"),
        "merdeka-118" => asset!("merdeka-118"),
        "moscow-kremlin" => asset!("moscow-kremlin"),
        "notre-dame" => asset!("notre-dame"),
        "nyhavn" => asset!("nyhavn"),
        "one-world-trade-center" => asset!("one-world-trade-center"),
        "palace-of-westminster" => asset!("palace-of-westminster"),
        "parthenon" => asset!("parthenon"),
        "petronas-towers" => asset!("petronas-towers"),
        "singapore-merlion" => asset!("singapore-merlion"),
        "space-needle" => asset!("space-needle"),
        "st-basils-cathedral" => asset!("st-basils-cathedral"),
        "statue-of-liberty" => asset!("statue-of-liberty"),
        "steinway-tower" => asset!("steinway-tower"),
        "sydney-harbour-bridge" => asset!("sydney-harbour-bridge"),
        "sydney-opera-house" => asset!("sydney-opera-house"),
        "taipei-101" => asset!("taipei-101"),
        "the-gherkin" => asset!("the-gherkin"),
        "the-shard" => asset!("the-shard"),
        "tokyo-tower" => asset!("tokyo-tower"),
        "tower-bridge" => asset!("tower-bridge"),
        "washington-monument" => asset!("washington-monument"),
        "white-house" => asset!("white-house"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_levels_decode_like_the_whole_asset() {
        for name in ["arc-de-triomphe", "bled-island", "nyhavn"] {
            let (_, bytes) = bytes(name).unwrap();
            let whole = get(name).unwrap();
            let split = split_levels(bytes).unwrap();
            assert_eq!(split.len(), whole.level_count());
            for (lod, payload) in split.iter().enumerate() {
                let level = parse_level_payload(payload).unwrap();
                let expected = whole.level(lod);
                assert_eq!(level.mesh.vertices.len(), expected.mesh.vertices.len());
                assert_eq!(level.mesh.indices, expected.mesh.indices);
                assert!(
                    level
                        .mesh
                        .vertices
                        .iter()
                        .zip(&expected.mesh.vertices)
                        .all(|(a, b)| a.pos == b.pos && a.face_ao == b.face_ao && a.rgba == b.rgba)
                );
                assert_eq!(level.bins, expected.bins);
            }
        }
    }

    #[test]
    fn indexed_assets_are_finite_and_preserve_materials_at_each_lod() {
        for name in [
            "washington-monument",
            "gateway-arch",
            "sydney-opera-house",
            "nyhavn",
        ] {
            let a = get(name).unwrap();
            assert_eq!(a.level_count(), 3);
            for level in (0..a.level_count()).map(|lod| a.level(lod)) {
                assert!(!level.mesh.indices.is_empty());
                let all = level.candidates(level.low.xz(), level.high.xz());
                assert_eq!(all.len(), level.mesh.indices.len() / 3);
            }
        }
    }
    #[test]
    fn baked_catalog_lods_fit_activation_bounds_and_spatial_index() {
        for landmark in super::super::city_generated::CITY_MODELS
            .iter()
            .chain(super::super::heritage_generated::HERITAGE_MODELS)
            .chain(super::super::skyscraper_generated::SKYSCRAPER_MODELS)
            .chain(super::super::monument_generated::MONUMENT_MODELS)
            .chain(super::super::island_generated::ISLAND_MODELS)
        {
            let asset = get(landmark.name).unwrap();
            assert_eq!(asset.levels.len(), 3);
            let mut previous = usize::MAX;
            for level in &asset.levels {
                assert!(
                    level.mesh.indices.len() <= previous,
                    "{} LOD grows",
                    landmark.name
                );
                previous = level.mesh.indices.len();
                for axis in 0..3 {
                    assert!(
                        f64::from(level.low[axis]) >= landmark.bbox_min[axis],
                        "{} low bounds",
                        landmark.name
                    );
                    assert!(
                        f64::from(level.high[axis]) <= landmark.bbox_max[axis],
                        "{} high bounds",
                        landmark.name
                    );
                }
                let candidates = level.candidates(level.low.xz(), level.high.xz());
                assert_eq!(
                    candidates.len() * 3,
                    level.mesh.indices.len(),
                    "{} lost triangles in spatial index",
                    landmark.name
                );
                assert!(
                    level
                        .mesh
                        .vertices
                        .iter()
                        .all(|v| ((v.face_ao >> 5) & 255) < 128)
                );
            }
        }
    }

    #[test]
    fn malformed_asset_cannot_allocate_an_unbounded_decode() {
        assert!(parse(&u32::MAX.to_le_bytes()).is_err());
        assert!(parse(b"bad").is_err());
    }
}
