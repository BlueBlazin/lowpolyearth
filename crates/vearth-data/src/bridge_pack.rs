//! Prepared physical-bridge catalog.
//!
//! Bridge identity and topology come from raw OpenStreetMap objects. Vector
//! tiles are deliberately not used for identity: their feature IDs and line
//! fragments are tile-local rendering data, not stable OSM ways. Preparation
//! may still consult cached vector-tile water polygons to find the navigable
//! water run, but runtime generation only performs an indexed catalog lookup.

use crate::vectors::RoadClass;
use std::collections::{HashMap, HashSet};
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"VBRDG003";
const FORMAT_VERSION: u32 = 3;
const INDEX_ZOOM: u8 = 12;
const WATER_ZOOM: u8 = 14;
const MEAN_RADIUS: f64 = 6_371_008.8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeStructure {
    Beam,
    Arch,
    Truss,
    Suspension,
    CableStayed,
    TiedArch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BridgeMaterial {
    #[default]
    Unknown = 0,
    Stone = 1,
    Brick = 2,
    Concrete = 3,
    Metal = 4,
    Wood = 5,
}

/// A complete physical bridge projected into the local east/north frame of
/// one feature query. Distances along `alignment` remain in metres.
#[derive(Clone, Debug)]
pub struct CanonicalBridge {
    pub id: u64,
    pub class: RoadClass,
    /// One connected traffic alignment, including enough ordinary road at
    /// both ends for the elevated deck to meet the existing road surface.
    pub alignment: Vec<[f32; 2]>,
    /// Actual `man_made=bridge` footprint when OSM supplies one. An empty
    /// outline means preparation had to use a conservative swept deck.
    pub deck_outline: Vec<[f32; 2]>,
    pub bridge_start_m: f32,
    pub bridge_end_m: f32,
    pub water_start_m: f32,
    pub water_end_m: f32,
    pub deck_width_m: f32,
    pub road_width_m: f32,
    pub water_is_ocean: bool,
    pub structure: BridgeStructure,
    pub material: BridgeMaterial,
}

#[derive(Clone, Debug)]
struct StoredBridge {
    id: u64,
    class: RoadClass,
    alignment_e7: Vec<[i32; 2]>, // [lat, lon], degrees * 1e7
    outline_e7: Vec<[i32; 2]>,
    bridge_start_m: f32,
    bridge_end_m: f32,
    water_start_m: f32,
    water_end_m: f32,
    deck_width_m: f32,
    road_width_m: f32,
    water_is_ocean: bool,
    structure: BridgeStructure,
    material: BridgeMaterial,
}

/// In-memory spatial index over a prepared `.vbp` catalog.
pub struct BridgeCatalog {
    bridges: Vec<StoredBridge>,
    index: HashMap<(u32, u32), Vec<usize>>,
    content_hash: u64,
}

impl BridgeCatalog {
    pub fn open(path: &Path) -> io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let content_hash = fnv1a(&bytes);
        let bridges = decode_catalog(&bytes)?;
        let mut catalog = BridgeCatalog {
            bridges,
            index: HashMap::new(),
            content_hash,
        };
        catalog.build_index();
        Ok(catalog)
    }

    pub fn len(&self) -> usize {
        self.bridges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bridges.is_empty()
    }

    pub fn content_hash(&self) -> u64 {
        self.content_hash
    }

    fn build_index(&mut self) {
        for (index, bridge) in self.bridges.iter().enumerate() {
            let Some((lat0, lat1, lon0, lon1)) = stored_bounds(bridge) else {
                continue;
            };
            let (x0, y0) = mercator_tile(lat1, lon0, INDEX_ZOOM);
            let (x1, y1) = mercator_tile(lat0, lon1, INDEX_ZOOM);
            for y in y0.min(y1)..=y0.max(y1) {
                for x in wrapped_xs(x0, x1, INDEX_ZOOM) {
                    self.index.entry((x, y)).or_default().push(index);
                }
            }
        }
    }

    pub fn query(
        &self,
        lat0: f64,
        lon0: f64,
        half_e_m: f64,
        half_n_m: f64,
    ) -> Vec<CanonicalBridge> {
        if self.bridges.is_empty() || lat0.abs() > 1.48 {
            return Vec::new();
        }
        let pad = 160.0;
        let dlat = (half_n_m + pad) / MEAN_RADIUS;
        let dlon = (half_e_m + pad) / (MEAN_RADIUS * lat0.cos().abs().max(0.05));
        let (x0, y0) = mercator_tile(lat0 + dlat, lon0 - dlon, INDEX_ZOOM);
        let (x1, y1) = mercator_tile(lat0 - dlat, lon0 + dlon, INDEX_ZOOM);
        let mut candidates = HashSet::new();
        for y in y0.min(y1)..=y0.max(y1) {
            for x in wrapped_xs(x0, x1, INDEX_ZOOM) {
                if let Some(indices) = self.index.get(&(x, y)) {
                    candidates.extend(indices.iter().copied());
                }
            }
        }

        let mut out = Vec::new();
        for index in candidates {
            let bridge = &self.bridges[index];
            let alignment: Vec<_> = bridge
                .alignment_e7
                .iter()
                .map(|&point| point_e7_to_en(point, lat0, lon0))
                .collect();
            let deck_outline: Vec<_> = bridge
                .outline_e7
                .iter()
                .map(|&point| point_e7_to_en(point, lat0, lon0))
                .collect();
            let radius = bridge.deck_width_m * 0.5 + 3.0;
            let intersects = alignment.iter().chain(&deck_outline).any(|p| {
                p[0] >= -(half_e_m as f32) - radius
                    && p[0] <= half_e_m as f32 + radius
                    && p[1] >= -(half_n_m as f32) - radius
                    && p[1] <= half_n_m as f32 + radius
            }) || polyline_intersects_box(
                &alignment,
                half_e_m as f32 + radius,
                half_n_m as f32 + radius,
            );
            if !intersects {
                continue;
            }
            out.push(CanonicalBridge {
                id: bridge.id,
                class: bridge.class,
                alignment,
                deck_outline,
                bridge_start_m: bridge.bridge_start_m,
                bridge_end_m: bridge.bridge_end_m,
                water_start_m: bridge.water_start_m,
                water_end_m: bridge.water_end_m,
                deck_width_m: bridge.deck_width_m,
                road_width_m: bridge.road_width_m,
                water_is_ocean: bridge.water_is_ocean,
                structure: bridge.structure,
                material: bridge.material,
            });
        }
        out.sort_by_key(|bridge| bridge.id);
        out
    }
}

fn polyline_intersects_box(points: &[[f32; 2]], half_e: f32, half_n: f32) -> bool {
    points.windows(2).any(|segment| {
        let lo = [
            segment[0][0].min(segment[1][0]),
            segment[0][1].min(segment[1][1]),
        ];
        let hi = [
            segment[0][0].max(segment[1][0]),
            segment[0][1].max(segment[1][1]),
        ];
        hi[0] >= -half_e && lo[0] <= half_e && hi[1] >= -half_n && lo[1] <= half_n
    })
}

fn stored_bounds(bridge: &StoredBridge) -> Option<(f64, f64, f64, f64)> {
    let mut lat0 = f64::INFINITY;
    let mut lat1 = f64::NEG_INFINITY;
    let mut lon0 = f64::INFINITY;
    let mut lon1 = f64::NEG_INFINITY;
    for &[lat, lon] in bridge.alignment_e7.iter().chain(&bridge.outline_e7) {
        let lat = (lat as f64 * 1e-7).to_radians();
        let lon = (lon as f64 * 1e-7).to_radians();
        lat0 = lat0.min(lat);
        lat1 = lat1.max(lat);
        lon0 = lon0.min(lon);
        lon1 = lon1.max(lon);
    }
    lat0.is_finite().then_some((lat0, lat1, lon0, lon1))
}

fn point_e7_to_en(point: [i32; 2], lat0: f64, lon0: f64) -> [f32; 2] {
    let lat = (point[0] as f64 * 1e-7).to_radians();
    let lon = (point[1] as f64 * 1e-7).to_radians();
    let mut dlon = lon - lon0;
    if dlon > std::f64::consts::PI {
        dlon -= std::f64::consts::TAU;
    } else if dlon < -std::f64::consts::PI {
        dlon += std::f64::consts::TAU;
    }
    [
        (dlon * lat0.cos() * MEAN_RADIUS) as f32,
        ((lat - lat0) * MEAN_RADIUS) as f32,
    ]
}

fn mercator_tile(lat: f64, lon: f64, zoom: u8) -> (u32, u32) {
    let n = (1u64 << zoom) as f64;
    let x = ((lon.to_degrees() + 180.0) / 360.0 * n).floor();
    let lat = lat.clamp(-1.484_422, 1.484_422);
    let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) * 0.5 * n).floor();
    let max = (1u64 << zoom) - 1;
    (
        (x as i64).rem_euclid(1i64 << zoom) as u32,
        (y.max(0.0) as u64).min(max) as u32,
    )
}

fn wrapped_xs(start: u32, end: u32, zoom: u8) -> Vec<u32> {
    let n = 1u32 << zoom;
    let count = ((end as u64 + n as u64 - start as u64) % n as u64 + 1) as u32;
    (0..count.min(n)).map(|step| (start + step) % n).collect()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn class_code(class: RoadClass) -> u8 {
    match class {
        RoadClass::Highway => 0,
        RoadClass::Major => 1,
        RoadClass::Medium => 2,
        RoadClass::Minor => 3,
        RoadClass::Path => 4,
        RoadClass::Rail => 5,
        RoadClass::Service => 6,
    }
}

fn decode_class(code: u8) -> io::Result<RoadClass> {
    match code {
        0 => Ok(RoadClass::Highway),
        1 => Ok(RoadClass::Major),
        2 => Ok(RoadClass::Medium),
        3 => Ok(RoadClass::Minor),
        4 => Ok(RoadClass::Path),
        5 => Ok(RoadClass::Rail),
        6 => Ok(RoadClass::Service),
        _ => Err(invalid("bridge catalog road class")),
    }
}

fn structure_code(structure: BridgeStructure) -> u8 {
    match structure {
        BridgeStructure::Beam => 0,
        BridgeStructure::Arch => 1,
        BridgeStructure::Truss => 2,
        BridgeStructure::Suspension => 3,
        BridgeStructure::CableStayed => 4,
        BridgeStructure::TiedArch => 5,
    }
}

fn decode_structure(code: u8) -> io::Result<BridgeStructure> {
    match code {
        0 => Ok(BridgeStructure::Beam),
        1 => Ok(BridgeStructure::Arch),
        2 => Ok(BridgeStructure::Truss),
        3 => Ok(BridgeStructure::Suspension),
        4 => Ok(BridgeStructure::CableStayed),
        5 => Ok(BridgeStructure::TiedArch),
        _ => Err(invalid("bridge catalog structure")),
    }
}

fn decode_material(code: u8) -> io::Result<BridgeMaterial> {
    match code {
        0 => Ok(BridgeMaterial::Unknown),
        1 => Ok(BridgeMaterial::Stone),
        2 => Ok(BridgeMaterial::Brick),
        3 => Ok(BridgeMaterial::Concrete),
        4 => Ok(BridgeMaterial::Metal),
        5 => Ok(BridgeMaterial::Wood),
        _ => Err(invalid("bridge catalog material")),
    }
}

fn decode_catalog(bytes: &[u8]) -> io::Result<Vec<StoredBridge>> {
    let mut input = Cursor::new(bytes);
    let mut magic = [0u8; 8];
    input.read_exact(&mut magic)?;
    if &magic != MAGIC && &magic != b"VBRDG002" {
        return Err(invalid("bridge catalog magic"));
    }
    let version = read_u32(&mut input)?;
    if !((&magic == MAGIC && version == FORMAT_VERSION) || (&magic == b"VBRDG002" && version == 2))
    {
        return Err(invalid(format!("bridge catalog version {version}")));
    }
    let count = read_u32(&mut input)? as usize;
    let mut bridges = Vec::with_capacity(count);
    for _ in 0..count {
        let id = read_u64(&mut input)?;
        let class = decode_class(read_u8(&mut input)?)?;
        let structure = decode_structure(read_u8(&mut input)?)?;
        let water_is_ocean = read_u8(&mut input)? & 1 != 0;
        let material_code = read_u8(&mut input)?;
        let material = if version == 2 {
            BridgeMaterial::Unknown
        } else {
            decode_material(material_code)?
        };
        let deck_width_m = read_f32(&mut input)?;
        let road_width_m = read_f32(&mut input)?;
        let bridge_start_m = read_f32(&mut input)?;
        let bridge_end_m = read_f32(&mut input)?;
        let water_start_m = read_f32(&mut input)?;
        let water_end_m = read_f32(&mut input)?;
        let alignment_e7 = read_points(&mut input, false)?;
        let outline_e7 = read_points(&mut input, true)?;
        bridges.push(StoredBridge {
            id,
            class,
            alignment_e7,
            outline_e7,
            bridge_start_m,
            bridge_end_m,
            water_start_m,
            water_end_m,
            deck_width_m,
            road_width_m,
            water_is_ocean,
            structure,
            material,
        });
    }
    if input.position() != bytes.len() as u64 {
        return Err(invalid("bridge catalog trailing bytes"));
    }
    Ok(bridges)
}

fn read_points(input: &mut Cursor<&[u8]>, allow_empty: bool) -> io::Result<Vec<[i32; 2]>> {
    let count = read_u32(input)? as usize;
    let valid = if allow_empty {
        count == 0 || (3..=32_768).contains(&count)
    } else {
        (2..=32_768).contains(&count)
    };
    if !valid {
        return Err(invalid("bridge catalog point count"));
    }
    let mut points = Vec::with_capacity(count);
    for _ in 0..count {
        points.push([read_i32(input)?, read_i32(input)?]);
    }
    Ok(points)
}

fn encode_catalog(bridges: &[StoredBridge]) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    output.extend_from_slice(&(bridges.len() as u32).to_le_bytes());
    for bridge in bridges {
        output.extend_from_slice(&bridge.id.to_le_bytes());
        output.push(class_code(bridge.class));
        output.push(structure_code(bridge.structure));
        output.push(bridge.water_is_ocean as u8);
        output.push(bridge.material as u8);
        for value in [
            bridge.deck_width_m,
            bridge.road_width_m,
            bridge.bridge_start_m,
            bridge.bridge_end_m,
            bridge.water_start_m,
            bridge.water_end_m,
        ] {
            output.extend_from_slice(&value.to_le_bytes());
        }
        write_points(&mut output, &bridge.alignment_e7);
        write_points(&mut output, &bridge.outline_e7);
    }
    Ok(output)
}

fn write_points(output: &mut Vec<u8>, points: &[[i32; 2]]) {
    output.extend_from_slice(&(points.len() as u32).to_le_bytes());
    for point in points {
        output.extend_from_slice(&point[0].to_le_bytes());
        output.extend_from_slice(&point[1].to_le_bytes());
    }
}

fn read_u8(input: &mut Cursor<&[u8]>) -> io::Result<u8> {
    let mut byte = [0];
    input.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn read_u32(input: &mut Cursor<&[u8]>) -> io::Result<u32> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_i32(input: &mut Cursor<&[u8]>) -> io::Result<i32> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

fn read_u64(input: &mut Cursor<&[u8]>) -> io::Result<u64> {
    let mut bytes = [0; 8];
    input.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_f32(input: &mut Cursor<&[u8]>) -> io::Result<f32> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

// Native raw-OSM preparation.

#[derive(Clone, Copy, Debug)]
pub struct BuildSummary {
    pub inputs: usize,
    pub bridges: usize,
    pub bytes: usize,
}

#[derive(Clone)]
struct OsmWay {
    id: i64,
    nodes: Vec<i64>,
    tags: HashMap<String, String>,
}

#[derive(Clone)]
struct OsmMember {
    kind: String,
    reference: i64,
    role: String,
}

#[derive(Clone)]
struct OsmRelation {
    id: i64,
    members: Vec<OsmMember>,
    tags: HashMap<String, String>,
}

#[derive(Default)]
struct OsmData {
    nodes: HashMap<i64, [f64; 2]>, // radians [lat, lon]
    ways: HashMap<i64, OsmWay>,
    relations: HashMap<i64, OsmRelation>,
}

#[derive(Clone)]
struct TrafficWay {
    id: i64,
    nodes: Vec<i64>,
    points: Vec<[f64; 2]>,
    class: RoadClass,
    width_m: f64,
    structure: BridgeStructure,
    material: BridgeMaterial,
}

#[derive(Clone)]
struct Outline {
    id: u64,
    points: Vec<[f64; 2]>,
    structure: Option<BridgeStructure>,
    material: BridgeMaterial,
}

#[derive(Clone)]
struct RawWater {
    ring: Vec<[f64; 2]>,
    is_ocean: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pont_neuf() -> Vec<StoredBridge> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/pont-neuf-bridges.osm");
        let mut osm = OsmData::default();
        parse_osm_xml(&path, &mut osm).unwrap();
        canonicalize_with_water(&osm, &|_, _, start, end| Ok(Some((start, end, false)))).unwrap()
    }

    #[test]
    fn bridge_catalog_distinct_pont_neuf_outlines_keep_their_own_alignment() {
        let bridges = pont_neuf();
        for outline_id in [53_574_149, 53_574_164] {
            let bridge = bridges
                .iter()
                .find(|bridge| bridge.id == osm_way_id(outline_id) ^ 0x4000_0000_0000_0000)
                .unwrap();
            assert_eq!(bridge.structure, BridgeStructure::Arch);
            assert_eq!(bridge.material, BridgeMaterial::Unknown);
            let points: Vec<_> = bridge
                .alignment_e7
                .iter()
                .map(|p| {
                    [
                        f64::from(p[0]).to_radians() / 1.0e7,
                        f64::from(p[1]).to_radians() / 1.0e7,
                    ]
                })
                .collect();
            let ring: Vec<_> = bridge
                .outline_e7
                .iter()
                .map(|p| {
                    [
                        f64::from(p[0]).to_radians() / 1.0e7,
                        f64::from(p[1]).to_radians() / 1.0e7,
                    ]
                })
                .collect();
            let frame = to_local(&ring);
            let polygon: Vec<_> = ring
                .iter()
                .map(|p| project(*p, frame.lat0, frame.lon0))
                .collect();
            let local = to_local(&points);
            let midpoint = point_at_s(
                &local.points,
                (bridge.bridge_start_m + bridge.bridge_end_m) as f64 * 0.5,
            );
            let geographic = [
                local.lat0 + midpoint[1] / MEAN_RADIUS,
                local.lon0 + midpoint[0] / (MEAN_RADIUS * local.lat0.cos()),
            ];
            assert!(
                point_in_ring(&polygon, project(geographic, frame.lat0, frame.lon0)),
                "bridge {outline_id} borrowed the other channel's footprint"
            );
        }
    }

    #[test]
    fn bridge_catalog_material_roundtrip_and_legacy_reserved_byte() {
        let mut bridge = pont_neuf().remove(0);
        bridge.material = BridgeMaterial::Stone;
        bridge.structure = BridgeStructure::TiedArch;
        let mut encoded = encode_catalog(&[bridge]).unwrap();
        let decoded = decode_catalog(&encoded).unwrap();
        assert_eq!(decoded[0].material, BridgeMaterial::Stone);
        assert_eq!(decoded[0].structure, BridgeStructure::TiedArch);
        encoded[..8].copy_from_slice(b"VBRDG002");
        encoded[8..12].copy_from_slice(&2u32.to_le_bytes());
        encoded[25] = structure_code(BridgeStructure::Arch);
        encoded[27] = 255;
        let legacy = decode_catalog(&encoded).unwrap();
        assert_eq!(legacy[0].material, BridgeMaterial::Unknown);
        assert_eq!(legacy[0].structure, BridgeStructure::Arch);
        encoded[..8].copy_from_slice(MAGIC);
        encoded[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        assert!(decode_catalog(&encoded).is_err());
    }

    #[test]
    fn bridge_catalog_structure_and_material_require_source_tags() {
        let mut tags = HashMap::from([("bridge".into(), "yes".into())]);
        assert_eq!(tagged_structure(&tags), Some(BridgeStructure::Beam));
        assert_eq!(tagged_material(&tags), BridgeMaterial::Unknown);
        tags.insert("bridge:structure".into(), "arch".into());
        assert_eq!(tagged_structure(&tags), Some(BridgeStructure::Arch));
        tags.insert("bridge:structure".into(), "tied_arch".into());
        assert_eq!(tagged_structure(&tags), Some(BridgeStructure::TiedArch));
        tags.insert("material".into(), "steel".into());
        assert_eq!(tagged_material(&tags), BridgeMaterial::Metal);
        tags.insert("bridge:material".into(), "stone".into());
        assert_eq!(tagged_material(&tags), BridgeMaterial::Stone);
    }
}

/// Build a catalog from one or more raw OSM XML extracts. OSM IDs and shared
/// node references define topology. Cached MVT data is read only for water
/// intersection, never for bridge identity, grouping, width, or approaches.
pub fn build_osm_catalog(
    tile_root: &Path,
    osm_inputs: &[PathBuf],
    output: &Path,
) -> io::Result<BuildSummary> {
    if osm_inputs.is_empty() {
        return Err(invalid("at least one --osm input is required"));
    }
    let mut osm = OsmData::default();
    for input in osm_inputs {
        if input
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pbf"))
        {
            parse_osm_pbf(input, &mut osm)?;
        } else {
            parse_osm_xml(input, &mut osm)?;
        }
    }
    let mut bridges = canonicalize_osm(&osm, tile_root)?;
    bridges.sort_by_key(|bridge| bridge.id);
    bridges.dedup_by_key(|bridge| bridge.id);
    let bytes = encode_catalog(&bridges)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = output.with_extension("vbp.tmp");
    std::fs::write(&temporary, &bytes)?;
    std::fs::rename(&temporary, output)?;
    Ok(BuildSummary {
        inputs: osm_inputs.len(),
        bridges: bridges.len(),
        bytes: bytes.len(),
    })
}

#[cfg(target_arch = "wasm32")]
fn parse_osm_pbf(_path: &Path, _data: &mut OsmData) -> io::Result<()> {
    Err(invalid("OSM PBF input requires a native build"))
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_osm_pbf(path: &Path, data: &mut OsmData) -> io::Result<()> {
    use osmpbfreader::{OsmId, OsmObj, OsmPbfReader};

    fn tags(source: &osmpbfreader::Tags) -> HashMap<String, String> {
        source
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }
    fn pbf_error(error: impl std::fmt::Display) -> io::Error {
        invalid(format!("OSM PBF: {error}"))
    }

    let mut bridge_nodes = HashSet::new();
    let mut relation_ways = HashSet::new();
    let file = std::fs::File::open(path)?;
    let mut reader = OsmPbfReader::new(file);
    for object in reader.iter() {
        match object.map_err(pbf_error)? {
            OsmObj::Way(way) => {
                let way_tags = tags(&way.tags);
                if is_bridge(&way_tags)
                    || way_tags
                        .get("man_made")
                        .is_some_and(|value| value == "bridge")
                {
                    let nodes: Vec<_> = way.nodes.into_iter().map(|node| node.0).collect();
                    if is_bridge(&way_tags) && road_class(&way_tags).is_some() {
                        bridge_nodes.extend(nodes.iter().copied());
                    }
                    data.ways.insert(
                        way.id.0,
                        OsmWay {
                            id: way.id.0,
                            nodes,
                            tags: way_tags,
                        },
                    );
                }
            }
            OsmObj::Relation(relation) => {
                let relation_tags = tags(&relation.tags);
                let relevant = relation_tags
                    .get("man_made")
                    .is_some_and(|value| value == "bridge")
                    || relation_tags
                        .get("type")
                        .is_some_and(|value| value == "bridge");
                if relevant {
                    let members: Vec<_> = relation
                        .refs
                        .into_iter()
                        .map(|member| {
                            let (kind, reference) = match member.member {
                                OsmId::Node(id) => ("node", id.0),
                                OsmId::Way(id) => {
                                    relation_ways.insert(id.0);
                                    ("way", id.0)
                                }
                                OsmId::Relation(id) => ("relation", id.0),
                            };
                            OsmMember {
                                kind: kind.to_string(),
                                reference,
                                role: member.role.to_string(),
                            }
                        })
                        .collect();
                    data.relations.insert(
                        relation.id.0,
                        OsmRelation {
                            id: relation.id.0,
                            members,
                            tags: relation_tags,
                        },
                    );
                }
            }
            OsmObj::Node(_) => {}
        }
    }

    // A second pass retains relation-member outline pieces and only the
    // ordinary road/rail ways touching a bridge node. Those are sufficient
    // to construct exact approach joins without loading a planet road graph.
    let file = std::fs::File::open(path)?;
    let mut reader = OsmPbfReader::new(file);
    for object in reader.iter() {
        let OsmObj::Way(way) = object.map_err(pbf_error)? else {
            continue;
        };
        let touches_bridge = way.nodes.iter().any(|node| bridge_nodes.contains(&node.0));
        if !relation_ways.contains(&way.id.0) && !touches_bridge {
            continue;
        }
        let way_tags = tags(&way.tags);
        if !relation_ways.contains(&way.id.0) && road_class(&way_tags).is_none() {
            continue;
        }
        data.ways.insert(
            way.id.0,
            OsmWay {
                id: way.id.0,
                nodes: way.nodes.into_iter().map(|node| node.0).collect(),
                tags: way_tags,
            },
        );
    }

    let needed_nodes: HashSet<_> = data
        .ways
        .values()
        .flat_map(|way| way.nodes.iter().copied())
        .collect();
    let file = std::fs::File::open(path)?;
    let mut reader = OsmPbfReader::new(file);
    for object in reader.iter() {
        let OsmObj::Node(node) = object.map_err(pbf_error)? else {
            continue;
        };
        if needed_nodes.contains(&node.id.0) {
            data.nodes.insert(
                node.id.0,
                [node.lat().to_radians(), node.lon().to_radians()],
            );
        }
    }
    Ok(())
}

fn parse_osm_xml(path: &Path, data: &mut OsmData) -> io::Result<()> {
    use quick_xml::Reader;
    use quick_xml::events::{BytesStart, Event};
    use std::io::BufReader;

    fn attr(element: &BytesStart<'_>, key: &[u8]) -> io::Result<Option<String>> {
        for attribute in element.attributes().with_checks(false) {
            let attribute = attribute.map_err(|error| invalid(error.to_string()))?;
            if attribute.key.as_ref() == key {
                return Ok(Some(
                    String::from_utf8_lossy(attribute.value.as_ref()).into_owned(),
                ));
            }
        }
        Ok(None)
    }
    fn number<T: std::str::FromStr>(
        element: &BytesStart<'_>,
        key: &[u8],
        label: &str,
    ) -> io::Result<T> {
        attr(element, key)?
            .ok_or_else(|| invalid(format!("OSM {label} missing")))?
            .parse()
            .map_err(|_| invalid(format!("OSM {label}")))
    }
    fn apply_empty(
        element: &BytesStart<'_>,
        data: &mut OsmData,
        way: &mut Option<OsmWay>,
        relation: &mut Option<OsmRelation>,
    ) -> io::Result<()> {
        match element.name().as_ref() {
            b"node" => {
                let id = number(element, b"id", "node id")?;
                let lat: f64 = number(element, b"lat", "node latitude")?;
                let lon: f64 = number(element, b"lon", "node longitude")?;
                data.nodes.insert(id, [lat.to_radians(), lon.to_radians()]);
            }
            b"nd" => {
                if let Some(way) = way {
                    way.nodes.push(number(element, b"ref", "node reference")?);
                }
            }
            b"tag" => {
                let Some(key) = attr(element, b"k")? else {
                    return Ok(());
                };
                let value = attr(element, b"v")?.unwrap_or_default();
                if let Some(way) = way {
                    way.tags.insert(key, value);
                } else if let Some(relation) = relation {
                    relation.tags.insert(key, value);
                }
            }
            b"member" => {
                if let Some(relation) = relation {
                    relation.members.push(OsmMember {
                        kind: attr(element, b"type")?.unwrap_or_default(),
                        reference: number(element, b"ref", "member reference")?,
                        role: attr(element, b"role")?.unwrap_or_default(),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    let file = std::fs::File::open(path)?;
    let mut reader = Reader::from_reader(BufReader::new(file));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut current_way: Option<OsmWay> = None;
    let mut current_relation: Option<OsmRelation> = None;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) => match element.name().as_ref() {
                b"way" => {
                    current_way = Some(OsmWay {
                        id: number(&element, b"id", "way id")?,
                        nodes: Vec::new(),
                        tags: HashMap::new(),
                    });
                }
                b"relation" => {
                    current_relation = Some(OsmRelation {
                        id: number(&element, b"id", "relation id")?,
                        members: Vec::new(),
                        tags: HashMap::new(),
                    });
                }
                _ => apply_empty(&element, data, &mut current_way, &mut current_relation)?,
            },
            Ok(Event::Empty(element)) => {
                apply_empty(&element, data, &mut current_way, &mut current_relation)?
            }
            Ok(Event::End(element)) => match element.name().as_ref() {
                b"way" => {
                    if let Some(way) = current_way.take() {
                        // Overpass unions can emit a selected way with its
                        // full tags and later repeat it as a dependency
                        // skeleton. Never let the tagless copy erase the
                        // authoritative bridge/highway metadata.
                        data.ways
                            .entry(way.id)
                            .and_modify(|existing| {
                                if way.nodes.len() > existing.nodes.len() {
                                    existing.nodes = way.nodes.clone();
                                }
                                existing.tags.extend(way.tags.clone());
                            })
                            .or_insert(way);
                    }
                }
                b"relation" => {
                    if let Some(relation) = current_relation.take() {
                        data.relations
                            .entry(relation.id)
                            .and_modify(|existing| {
                                if relation.members.len() > existing.members.len() {
                                    existing.members = relation.members.clone();
                                }
                                existing.tags.extend(relation.tags.clone());
                            })
                            .or_insert(relation);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(invalid(format!(
                    "{} at byte {}: {error}",
                    path.display(),
                    reader.error_position()
                )));
            }
            _ => {}
        }
        buffer.clear();
    }
    Ok(())
}

fn canonicalize_osm(osm: &OsmData, tile_root: &Path) -> io::Result<Vec<StoredBridge>> {
    canonicalize_with_water(osm, &|alignment, local, start, end| {
        water_run(tile_root, alignment, local, start, end)
    })
}

fn canonicalize_with_water(
    osm: &OsmData,
    water: &dyn Fn(&[[f64; 2]], &LocalFrame, f64, f64) -> io::Result<Option<(f64, f64, bool)>>,
) -> io::Result<Vec<StoredBridge>> {
    let mut bridge_ways: Vec<_> = osm
        .ways
        .values()
        .filter(|way| is_bridge(&way.tags))
        .filter_map(|way| traffic_way(way, osm))
        .collect();
    bridge_ways.sort_by_key(|way| way.id);
    if bridge_ways.is_empty() {
        return Ok(Vec::new());
    }
    let mut approaches: Vec<_> = osm
        .ways
        .values()
        .filter(|way| !is_bridge(&way.tags))
        .filter_map(|way| traffic_way(way, osm))
        .collect();
    approaches.sort_by_key(|way| way.id);
    let outlines = bridge_outlines(osm);

    let mut dsu = Dsu::new(bridge_ways.len());
    let way_indices: HashMap<_, _> = bridge_ways
        .iter()
        .enumerate()
        .map(|(index, way)| (way.id, index))
        .collect();
    let mut by_node: HashMap<i64, Vec<usize>> = HashMap::new();
    for (index, way) in bridge_ways.iter().enumerate() {
        for &node in &way.nodes {
            by_node.entry(node).or_default().push(index);
        }
    }
    for indices in by_node.values() {
        for &index in &indices[1..] {
            dsu.union(indices[0], index);
        }
    }
    // Explicit bridge relations and actual bridge outlines are authoritative
    // grouping boundaries. They collapse carriageways and attached paths into
    // one physical object without guessing from tile fragments.
    for relation in osm.relations.values().filter(|relation| {
        relation
            .tags
            .get("type")
            .is_some_and(|value| value == "bridge")
            || relation
                .tags
                .get("man_made")
                .is_some_and(|value| value == "bridge")
    }) {
        let members: Vec<_> = relation
            .members
            .iter()
            .filter(|member| member.kind == "way")
            .filter_map(|member| way_indices.get(&member.reference).copied())
            .collect();
        if let Some((&first, rest)) = members.split_first() {
            for &index in rest {
                dsu.union(first, index);
            }
        }
    }
    const PREP_INDEX_ZOOM: u8 = 17;
    let mut outline_index: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, outline) in outlines.iter().enumerate() {
        let Some((lat0, lat1, lon0, lon1)) = geographic_bounds(&outline.points) else {
            continue;
        };
        let (x0, y0) = mercator_tile(lat1, lon0, PREP_INDEX_ZOOM);
        let (x1, y1) = mercator_tile(lat0, lon1, PREP_INDEX_ZOOM);
        for y in y0.min(y1)..=y0.max(y1) {
            for x in wrapped_xs(x0, x1, PREP_INDEX_ZOOM) {
                outline_index.entry((x, y)).or_default().push(index);
            }
        }
    }
    let mut outline_members: Vec<Vec<usize>> = vec![Vec::new(); outlines.len()];
    let mut assigned_outline = vec![None; bridge_ways.len()];
    for (index, way) in bridge_ways.iter().enumerate() {
        let mut candidates = HashSet::new();
        for &point in &way.points {
            let tile = mercator_tile(point[0], point[1], PREP_INDEX_ZOOM);
            if let Some(indices) = outline_index.get(&tile) {
                candidates.extend(indices.iter().copied());
            }
        }
        let mut matches: Vec<_> = candidates
            .into_iter()
            .filter(|&outline_index| way_in_outline(way, &outlines[outline_index]))
            .map(|outline_index| {
                (
                    outline_index,
                    polygon_area_m2(&outlines[outline_index].points),
                )
            })
            .collect();
        matches.sort_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((outline_index, _)) = matches.first() {
            outline_members[*outline_index].push(index);
            assigned_outline[index] = Some(*outline_index);
        }
    }
    for members in &outline_members {
        if let Some((&first, rest)) = members.split_first() {
            for &index in rest {
                dsu.union(first, index);
            }
        }
    }

    // Raw OSM occasionally has divided carriageways but no bridge outline or
    // relation. Join only locally parallel, longitudinally overlapping ways;
    // stable IDs still keep unrelated crossings separate.
    let mut nearby: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for a in 0..bridge_ways.len() {
        let midpoint = bridge_ways[a].points[bridge_ways[a].points.len() / 2];
        let (x, y) = mercator_tile(midpoint[0], midpoint[1], PREP_INDEX_ZOOM);
        let n = 1u32 << PREP_INDEX_ZOOM;
        for dy in -1i64..=1 {
            let ny = y as i64 + dy;
            if !(0..n as i64).contains(&ny) {
                continue;
            }
            for dx in -1i64..=1 {
                let nx = (x as i64 + dx).rem_euclid(n as i64) as u32;
                if let Some(indices) = nearby.get(&(nx, ny as u32)) {
                    for &b in indices {
                        if same_unmapped_structure(&bridge_ways[a], &bridge_ways[b]) {
                            dsu.union(a, b);
                        }
                    }
                }
            }
        }
        nearby.entry((x, y)).or_default().push(a);
    }

    // A shared road junction/relation can connect distinct physical spans
    // across an island. An authored outline must never be borrowed from a
    // different span simply because that polygon has the smallest area.
    let mut groups: HashMap<(usize, Option<usize>), Vec<usize>> = HashMap::new();
    for index in 0..bridge_ways.len() {
        groups
            .entry((dsu.find(index), assigned_outline[index]))
            .or_default()
            .push(index);
    }
    let mut output = Vec::new();
    for members in groups.into_values() {
        let representative = *members
            .iter()
            .max_by(|&&a, &&b| {
                class_rank(bridge_ways[a].class)
                    .cmp(&class_rank(bridge_ways[b].class))
                    .then_with(|| {
                        geographic_len(&bridge_ways[a].points)
                            .total_cmp(&geographic_len(&bridge_ways[b].points))
                    })
            })
            .expect("non-empty bridge group");
        let traffic = merge_group_alignment(&bridge_ways, &members, representative);
        let outline = members
            .iter()
            .filter_map(|&member| assigned_outline[member])
            .min_by(|&a, &b| {
                polygon_area_m2(&outlines[a].points)
                    .total_cmp(&polygon_area_m2(&outlines[b].points))
            })
            .map(|index| &outlines[index]);
        let (alignment, bridge_start_m) = extend_with_approaches(&traffic, &approaches);
        let bridge_end_m = bridge_start_m + geographic_len(&traffic.points);
        let local = to_local(&alignment);
        let Some((water_start, water_end, water_is_ocean)) =
            water(&alignment, &local, bridge_start_m, bridge_end_m)?
        else {
            continue;
        };
        if water_end - water_start < 8.0 {
            continue;
        }
        let road_width_m = traffic.width_m.clamp(2.0, 30.0);
        let deck_width_m = if let Some(outline) = outline {
            outline_width(outline, &traffic).clamp(road_width_m, 80.0)
        } else {
            fallback_group_width(&bridge_ways, &members, &traffic).clamp(road_width_m, 36.0)
        };
        let tagged_structure = outline.and_then(|outline| outline.structure).or_else(|| {
            members
                .iter()
                .map(|&index| bridge_ways[index].structure)
                .find(|structure| *structure != BridgeStructure::Beam)
        });
        let structure = tagged_structure.unwrap_or(BridgeStructure::Beam);
        let material = outline
            .map(|o| o.material)
            .filter(|m| *m != BridgeMaterial::Unknown)
            .unwrap_or(traffic.material);
        let outline_points = outline
            .map(|outline| outline.points.as_slice())
            .unwrap_or(&[]);
        let id = outline.map(|outline| outline.id).unwrap_or_else(|| {
            osm_way_id(
                members
                    .iter()
                    .map(|&index| bridge_ways[index].id)
                    .min()
                    .unwrap(),
            )
        });
        output.push(StoredBridge {
            id,
            class: traffic.class,
            alignment_e7: to_e7(&alignment),
            outline_e7: to_e7(outline_points),
            bridge_start_m: bridge_start_m as f32,
            bridge_end_m: bridge_end_m as f32,
            water_start_m: water_start as f32,
            water_end_m: water_end as f32,
            deck_width_m: deck_width_m as f32,
            road_width_m: road_width_m as f32,
            water_is_ocean,
            structure,
            material,
        });
    }
    Ok(output)
}

fn traffic_way(way: &OsmWay, osm: &OsmData) -> Option<TrafficWay> {
    let class = road_class(&way.tags)?;
    let points: Vec<_> = way
        .nodes
        .iter()
        .filter_map(|node| osm.nodes.get(node).copied())
        .collect();
    if points.len() != way.nodes.len() || points.len() < 2 {
        return None;
    }
    Some(TrafficWay {
        id: way.id,
        nodes: way.nodes.clone(),
        points,
        class,
        width_m: tagged_width(&way.tags, class),
        structure: tagged_structure(&way.tags).unwrap_or(BridgeStructure::Beam),
        material: tagged_material(&way.tags),
    })
}

fn is_bridge(tags: &HashMap<String, String>) -> bool {
    tags.get("bridge").is_some_and(|value| {
        !matches!(
            value.as_str(),
            "no" | "false" | "0" | "proposed" | "construction"
        )
    })
}

fn road_class(tags: &HashMap<String, String>) -> Option<RoadClass> {
    if tags.contains_key("railway") {
        return Some(RoadClass::Rail);
    }
    Some(match tags.get("highway")?.as_str() {
        "motorway" | "motorway_link" | "trunk" | "trunk_link" => RoadClass::Highway,
        "primary" | "primary_link" | "secondary" | "secondary_link" => RoadClass::Major,
        "tertiary" | "tertiary_link" => RoadClass::Medium,
        "footway" | "path" | "cycleway" | "bridleway" | "steps" | "pedestrian" => RoadClass::Path,
        _ => RoadClass::Minor,
    })
}

fn tagged_width(tags: &HashMap<String, String>, class: RoadClass) -> f64 {
    if let Some(width) = tags.get("width").and_then(|value| parse_measure(value)) {
        return width;
    }
    let lanes = tags
        .get("lanes")
        .and_then(|value| value.split(';').next())
        .and_then(|value| value.parse::<f64>().ok());
    lanes
        .map(|lanes| lanes.clamp(1.0, 12.0) * 3.2 + 1.2)
        .unwrap_or(class.width_m() as f64)
}

fn parse_measure(value: &str) -> Option<f64> {
    let token: String = value
        .chars()
        .take_while(|character| character.is_ascii_digit() || matches!(character, '.' | ','))
        .collect();
    token.replace(',', ".").parse().ok()
}

fn tagged_structure(tags: &HashMap<String, String>) -> Option<BridgeStructure> {
    let value = tags
        .get("bridge:structure")
        .or_else(|| tags.get("bridge"))?
        .to_ascii_lowercase();
    if value.contains("suspension") {
        Some(BridgeStructure::Suspension)
    } else if value.contains("cable") {
        Some(BridgeStructure::CableStayed)
    } else if ["tied_arch", "through_arch", "bowstring"].contains(&value.as_str()) {
        Some(BridgeStructure::TiedArch)
    } else if value.contains("arch") {
        Some(BridgeStructure::Arch)
    } else if value.contains("truss") {
        Some(BridgeStructure::Truss)
    } else if value.contains("beam") || value == "yes" || value == "viaduct" {
        Some(BridgeStructure::Beam)
    } else {
        None
    }
}

fn tagged_material(tags: &HashMap<String, String>) -> BridgeMaterial {
    match tags
        .get("bridge:material")
        .or_else(|| tags.get("material"))
        .map(|v| v.to_ascii_lowercase())
        .as_deref()
    {
        Some("stone" | "masonry" | "sandstone" | "limestone" | "granite") => BridgeMaterial::Stone,
        Some("brick") => BridgeMaterial::Brick,
        Some("concrete" | "reinforced_concrete") => BridgeMaterial::Concrete,
        Some("steel" | "iron" | "metal") => BridgeMaterial::Metal,
        Some("wood" | "timber") => BridgeMaterial::Wood,
        _ => BridgeMaterial::Unknown,
    }
}

fn bridge_outlines(osm: &OsmData) -> Vec<Outline> {
    let mut outlines = Vec::new();
    for way in osm.ways.values().filter(|way| {
        way.tags
            .get("man_made")
            .is_some_and(|value| value == "bridge")
    }) {
        if way.nodes.len() < 4 || way.nodes.first() != way.nodes.last() {
            continue;
        }
        let points: Vec<_> = way
            .nodes
            .iter()
            .filter_map(|node| osm.nodes.get(node).copied())
            .collect();
        if points.len() == way.nodes.len() {
            outlines.push(Outline {
                id: osm_way_id(way.id) ^ 0x4000_0000_0000_0000,
                points,
                structure: tagged_structure(&way.tags),
                material: tagged_material(&way.tags),
            });
        }
    }
    for relation in osm.relations.values().filter(|relation| {
        relation
            .tags
            .get("man_made")
            .is_some_and(|value| value == "bridge")
    }) {
        let chains: Vec<_> = relation
            .members
            .iter()
            .filter(|member| member.kind == "way" && member.role != "inner")
            .filter_map(|member| osm.ways.get(&member.reference).map(|way| way.nodes.clone()))
            .collect();
        let Some(nodes) = join_closed_chain(chains) else {
            continue;
        };
        let points: Vec<_> = nodes
            .iter()
            .filter_map(|node| osm.nodes.get(node).copied())
            .collect();
        if points.len() == nodes.len() {
            outlines.push(Outline {
                id: osm_relation_id(relation.id),
                points,
                structure: tagged_structure(&relation.tags),
                material: tagged_material(&relation.tags),
            });
        }
    }
    outlines.sort_by_key(|outline| outline.id);
    outlines
}

fn join_closed_chain(mut chains: Vec<Vec<i64>>) -> Option<Vec<i64>> {
    chains.retain(|chain| chain.len() >= 2);
    let mut result = chains.pop()?;
    while result.first() != result.last() {
        let end = *result.last()?;
        let index = chains
            .iter()
            .position(|chain| chain.first() == Some(&end) || chain.last() == Some(&end))?;
        let mut next = chains.swap_remove(index);
        if next.last() == Some(&end) {
            next.reverse();
        }
        result.extend(next.into_iter().skip(1));
    }
    Some(result)
}

fn way_in_outline(way: &TrafficWay, outline: &Outline) -> bool {
    let frame = to_local(&outline.points);
    let ring: Vec<_> = outline
        .points
        .iter()
        .map(|&point| project(point, frame.lat0, frame.lon0))
        .collect();
    way.points
        .iter()
        .any(|&point| point_in_ring(&ring, project(point, frame.lat0, frame.lon0)))
        || way.points.windows(2).any(|segment| {
            let midpoint = [
                (segment[0][0] + segment[1][0]) * 0.5,
                (segment[0][1] + segment[1][1]) * 0.5,
            ];
            point_in_ring(&ring, project(midpoint, frame.lat0, frame.lon0))
        })
}

fn same_unmapped_structure(a: &TrafficWay, b: &TrafficWay) -> bool {
    if (a.class == RoadClass::Rail) != (b.class == RoadClass::Rail) {
        return false;
    }
    let frame = to_local(&a.points);
    let ap: Vec<_> = a
        .points
        .iter()
        .map(|&point| project(point, frame.lat0, frame.lon0))
        .collect();
    let bp: Vec<_> = b
        .points
        .iter()
        .map(|&point| project(point, frame.lat0, frame.lon0))
        .collect();
    let ad = unit(sub(*ap.last().unwrap(), ap[0]));
    let bd = unit(sub(*bp.last().unwrap(), bp[0]));
    if dot(ad, bd).abs() < 0.96 {
        return false;
    }
    let axis = if dot(ad, bd) < 0.0 {
        [-ad[0], -ad[1]]
    } else {
        ad
    };
    let a_range = projected_range(&ap, axis);
    let b_range = projected_range(&bp, axis);
    let overlap = a_range.1.min(b_range.1) - a_range.0.max(b_range.0);
    let shorter = (a_range.1 - a_range.0).min(b_range.1 - b_range.0);
    if overlap < shorter * 0.65 || shorter < 8.0 {
        return false;
    }
    let normal = [-axis[1], axis[0]];
    let a_lateral = ap.iter().map(|&point| dot(point, normal)).sum::<f64>() / ap.len() as f64;
    let b_lateral = bp.iter().map(|&point| dot(point, normal)).sum::<f64>() / bp.len() as f64;
    (a_lateral - b_lateral).abs() <= (a.width_m + b.width_m).max(12.0)
}

fn merge_group_alignment(ways: &[TrafficWay], members: &[usize], seed: usize) -> TrafficWay {
    let rail = ways[seed].class == RoadClass::Rail;
    let mut used = HashSet::new();
    used.insert(seed);
    let mut nodes = ways[seed].nodes.clone();
    let mut points = ways[seed].points.clone();
    let mut class = ways[seed].class;
    let mut width_m = ways[seed].width_m;
    let mut structure = ways[seed].structure;
    let mut material = ways[seed].material;

    for grow_front in [false, true] {
        loop {
            let join_node = if grow_front {
                nodes[0]
            } else {
                *nodes.last().unwrap()
            };
            let desired = if grow_front {
                local_direction(points[1], points[0])
            } else {
                local_direction(points[points.len() - 2], *points.last().unwrap())
            };
            let candidate = members
                .iter()
                .copied()
                .filter(|index| !used.contains(index))
                .filter(|&index| (ways[index].class == RoadClass::Rail) == rail)
                .filter_map(|index| {
                    let way = &ways[index];
                    let at_first = way.nodes.first() == Some(&join_node);
                    let at_last = way.nodes.last() == Some(&join_node);
                    if !at_first && !at_last {
                        return None;
                    }
                    let outward = if at_first {
                        local_direction(way.points[0], way.points[1])
                    } else {
                        local_direction(
                            *way.points.last().unwrap(),
                            way.points[way.points.len() - 2],
                        )
                    };
                    let straightness = dot(desired, outward);
                    (straightness >= 0.25).then_some((index, straightness, at_first))
                })
                .max_by(|a, b| {
                    a.1.total_cmp(&b.1)
                        .then_with(|| class_rank(ways[a.0].class).cmp(&class_rank(ways[b.0].class)))
                });
            let Some((index, _, at_first)) = candidate else {
                break;
            };
            used.insert(index);
            let way = &ways[index];
            let mut next_nodes = way.nodes.clone();
            let mut next_points = way.points.clone();
            if grow_front {
                if at_first {
                    next_nodes.reverse();
                    next_points.reverse();
                }
                next_nodes.pop();
                next_points.pop();
                next_nodes.extend(nodes);
                next_points.extend(points);
                nodes = next_nodes;
                points = next_points;
            } else {
                if !at_first {
                    next_nodes.reverse();
                    next_points.reverse();
                }
                nodes.extend(next_nodes.into_iter().skip(1));
                points.extend(next_points.into_iter().skip(1));
            }
            if class_rank(way.class) > class_rank(class) {
                class = way.class;
            }
            width_m = width_m.max(way.width_m);
            if structure == BridgeStructure::Beam && way.structure != BridgeStructure::Beam {
                structure = way.structure;
            }
            if material == BridgeMaterial::Unknown {
                material = way.material;
            }
        }
    }
    TrafficWay {
        id: members.iter().map(|&index| ways[index].id).min().unwrap(),
        nodes,
        points,
        class,
        width_m,
        structure,
        material,
    }
}

fn extend_with_approaches(bridge: &TrafficWay, approaches: &[TrafficWay]) -> (Vec<[f64; 2]>, f64) {
    const MAX_APPROACH: f64 = 140.0;
    let start_node = bridge.nodes[0];
    let end_node = *bridge.nodes.last().unwrap();
    let start_direction = local_direction(bridge.points[1], bridge.points[0]);
    let end_direction = local_direction(
        bridge.points[bridge.points.len() - 2],
        *bridge.points.last().unwrap(),
    );
    let start = best_approach(
        start_node,
        start_direction,
        bridge.class,
        approaches,
        MAX_APPROACH,
    );
    let end = best_approach(
        end_node,
        end_direction,
        bridge.class,
        approaches,
        MAX_APPROACH,
    );
    let mut alignment = Vec::new();
    if let Some(mut points) = start {
        points.reverse();
        alignment.extend(points);
        alignment.pop();
    }
    let bridge_start = if let Some(&last) = alignment.last() {
        geographic_len(&alignment) + geographic_len(&[last, bridge.points[0]])
    } else {
        0.0
    };
    alignment.extend(bridge.points.iter().copied());
    if let Some(points) = end {
        alignment.extend(points.into_iter().skip(1));
    }
    dedup_points(&mut alignment);
    (alignment, bridge_start)
}

fn best_approach(
    node: i64,
    desired: [f64; 2],
    class: RoadClass,
    approaches: &[TrafficWay],
    limit: f64,
) -> Option<Vec<[f64; 2]>> {
    let mut best: Option<(f64, Vec<[f64; 2]>)> = None;
    for way in approaches.iter().filter(|way| {
        (way.class == RoadClass::Rail) == (class == RoadClass::Rail)
            && (way.nodes.first() == Some(&node) || way.nodes.last() == Some(&node))
    }) {
        let mut points = way.points.clone();
        if way.nodes.last() == Some(&node) {
            points.reverse();
        }
        let direction = local_direction(points[0], points[1]);
        let score = dot(direction, desired);
        if score < 0.45 {
            continue;
        }
        clip_geographic_path(&mut points, limit);
        if best.as_ref().is_none_or(|current| score > current.0) {
            best = Some((score, points));
        }
    }
    best.map(|(_, points)| points)
}

fn clip_geographic_path(points: &mut Vec<[f64; 2]>, limit: f64) {
    if points.len() < 2 {
        return;
    }
    let mut output = vec![points[0]];
    let frame = to_local(points);
    let projected: Vec<_> = points
        .iter()
        .map(|&point| project(point, frame.lat0, frame.lon0))
        .collect();
    let mut used = 0.0;
    for index in 1..points.len() {
        let length = distance(projected[index - 1], projected[index]);
        if used + length <= limit {
            output.push(points[index]);
            used += length;
        } else {
            let t = ((limit - used) / length).clamp(0.0, 1.0);
            output.push([
                points[index - 1][0] + (points[index][0] - points[index - 1][0]) * t,
                points[index - 1][1] + (points[index][1] - points[index - 1][1]) * t,
            ]);
            break;
        }
    }
    *points = output;
}

fn water_run(
    tile_root: &Path,
    alignment: &[[f64; 2]],
    frame: &LocalFrame,
    bridge_start: f64,
    bridge_end: f64,
) -> io::Result<Option<(f64, f64, bool)>> {
    let source_dir = tile_root.join(WATER_ZOOM.to_string());
    let mut tiles = HashSet::new();
    for &point in alignment {
        let (x, y) = mercator_tile(point[0], point[1], WATER_ZOOM);
        for dy in -1i64..=1 {
            let ty = y as i64 + dy;
            if !(0..(1i64 << WATER_ZOOM)).contains(&ty) {
                continue;
            }
            for dx in -1i64..=1 {
                let tx = (x as i64 + dx).rem_euclid(1i64 << WATER_ZOOM) as u32;
                tiles.insert((tx, ty as u32));
            }
        }
    }
    let mut water = Vec::new();
    for (x, y) in tiles {
        let path = source_dir.join(format!("{x}_{y}.mvt"));
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let tile = crate::mvt::decode_tile(&bytes)
            .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        let Some(layer) = tile.layer("water") else {
            continue;
        };
        for feature in &layer.features {
            if feature.geom_type != crate::mvt::GeomType::Polygon {
                continue;
            }
            let is_ocean = matches!(
                feature
                    .get(layer, "kind")
                    .and_then(crate::mvt::Value::as_str),
                Some("ocean")
            );
            for ring in &feature.paths {
                if ring.len() >= 3 {
                    water.push(RawWater {
                        ring: ring
                            .iter()
                            .map(|&point| {
                                tile_point_en(
                                    WATER_ZOOM,
                                    x,
                                    y,
                                    layer.extent,
                                    point,
                                    frame.lat0,
                                    frame.lon0,
                                )
                            })
                            .collect(),
                        is_ocean,
                    });
                }
            }
        }
    }
    longest_water_run(&frame.points, bridge_start, bridge_end, &water)
}

fn longest_water_run(
    points: &[[f64; 2]],
    bridge_start: f64,
    bridge_end: f64,
    water: &[RawWater],
) -> io::Result<Option<(f64, f64, bool)>> {
    if water.is_empty() || bridge_end - bridge_start < 8.0 {
        return Ok(None);
    }
    let span = bridge_end - bridge_start;
    let count = ((span / 2.0).ceil() as usize).clamp(4, 8192);
    let mut runs = Vec::new();
    let mut current: Option<(f64, f64, bool)> = None;
    for index in 0..=count {
        let s = bridge_start + span * index as f64 / count as f64;
        let point = point_at_s(points, s);
        let mut wet = false;
        let mut ocean = false;
        for polygon in water {
            if point_in_ring(&polygon.ring, point) {
                wet = true;
                ocean |= polygon.is_ocean;
            }
        }
        if wet {
            if let Some(run) = &mut current {
                run.1 = s;
                run.2 |= ocean;
            } else {
                current = Some((s, s, ocean));
            }
        } else if let Some(run) = current.take()
            && run.1 - run.0 >= 6.0
        {
            runs.push(run);
        }
    }
    if let Some(run) = current
        && run.1 - run.0 >= 6.0
    {
        runs.push(run);
    }
    Ok(runs
        .into_iter()
        .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0))))
}

fn tile_point_en(
    zoom: u8,
    x: u32,
    y: u32,
    extent: u32,
    point: [i32; 2],
    lat0: f64,
    lon0: f64,
) -> [f64; 2] {
    let n = (1u64 << zoom) as f64;
    let mx = (x as f64 + point[0] as f64 / extent as f64) / n;
    let my = (y as f64 + point[1] as f64 / extent as f64) / n;
    let lon = (mx * 360.0 - 180.0).to_radians();
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * my)).sinh().atan();
    project([lat, lon], lat0, lon0)
}

fn fallback_group_width(ways: &[TrafficWay], members: &[usize], alignment: &TrafficWay) -> f64 {
    let frame = to_local(&alignment.points);
    let rep: Vec<_> = alignment
        .points
        .iter()
        .map(|&point| project(point, frame.lat0, frame.lon0))
        .collect();
    let axis = unit(sub(*rep.last().unwrap(), rep[0]));
    let normal = [-axis[1], axis[0]];
    let center = rep.iter().map(|&point| dot(point, normal)).sum::<f64>() / rep.len() as f64;
    let mut radius = alignment.width_m * 0.5;
    for &member in members {
        for &point in &ways[member].points {
            let local = project(point, frame.lat0, frame.lon0);
            radius = radius.max((dot(local, normal) - center).abs() + ways[member].width_m * 0.5);
        }
    }
    radius * 2.0 + 1.0
}

fn outline_width(outline: &Outline, traffic: &TrafficWay) -> f64 {
    let frame = to_local(&traffic.points);
    let traffic_points: Vec<_> = traffic
        .points
        .iter()
        .map(|&point| project(point, frame.lat0, frame.lon0))
        .collect();
    let axis = unit(sub(*traffic_points.last().unwrap(), traffic_points[0]));
    let normal = [-axis[1], axis[0]];
    let range = projected_range(
        &outline
            .points
            .iter()
            .map(|&point| project(point, frame.lat0, frame.lon0))
            .collect::<Vec<_>>(),
        normal,
    );
    range.1 - range.0
}

fn polygon_area_m2(points: &[[f64; 2]]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let frame = to_local(points);
    let projected: Vec<_> = points
        .iter()
        .map(|&point| project(point, frame.lat0, frame.lon0))
        .collect();
    projected
        .windows(2)
        .map(|edge| edge[0][0] * edge[1][1] - edge[1][0] * edge[0][1])
        .sum::<f64>()
        .abs()
        * 0.5
}

fn geographic_bounds(points: &[[f64; 2]]) -> Option<(f64, f64, f64, f64)> {
    let mut lat0 = f64::INFINITY;
    let mut lat1 = f64::NEG_INFINITY;
    let mut lon0 = f64::INFINITY;
    let mut lon1 = f64::NEG_INFINITY;
    for point in points {
        lat0 = lat0.min(point[0]);
        lat1 = lat1.max(point[0]);
        lon0 = lon0.min(point[1]);
        lon1 = lon1.max(point[1]);
    }
    lat0.is_finite().then_some((lat0, lat1, lon0, lon1))
}

fn class_rank(class: RoadClass) -> u8 {
    match class {
        RoadClass::Highway => 6,
        RoadClass::Major => 5,
        RoadClass::Medium => 4,
        RoadClass::Minor => 3,
        RoadClass::Service => 3,
        RoadClass::Rail => 2,
        RoadClass::Path => 1,
    }
}

fn osm_way_id(id: i64) -> u64 {
    id.unsigned_abs()
}

fn osm_relation_id(id: i64) -> u64 {
    id.unsigned_abs() | 0x8000_0000_0000_0000
}

fn to_e7(points: &[[f64; 2]]) -> Vec<[i32; 2]> {
    points
        .iter()
        .map(|point| {
            [
                (point[0].to_degrees() * 1e7).round() as i32,
                (point[1].to_degrees() * 1e7).round() as i32,
            ]
        })
        .collect()
}

struct LocalFrame {
    lat0: f64,
    lon0: f64,
    points: Vec<[f64; 2]>,
}

fn to_local(points: &[[f64; 2]]) -> LocalFrame {
    let lat0 = points.iter().map(|point| point[0]).sum::<f64>() / points.len().max(1) as f64;
    let lon0 = points.first().map_or(0.0, |point| point[1]);
    LocalFrame {
        lat0,
        lon0,
        points: points
            .iter()
            .map(|&point| project(point, lat0, lon0))
            .collect(),
    }
}

fn project(point: [f64; 2], lat0: f64, lon0: f64) -> [f64; 2] {
    let mut dlon = point[1] - lon0;
    if dlon > std::f64::consts::PI {
        dlon -= std::f64::consts::TAU;
    } else if dlon < -std::f64::consts::PI {
        dlon += std::f64::consts::TAU;
    }
    [
        dlon * lat0.cos() * MEAN_RADIUS,
        (point[0] - lat0) * MEAN_RADIUS,
    ]
}

fn geographic_len(points: &[[f64; 2]]) -> f64 {
    to_local(points)
        .points
        .windows(2)
        .map(|edge| distance(edge[0], edge[1]))
        .sum()
}

fn local_direction(from: [f64; 2], to: [f64; 2]) -> [f64; 2] {
    let frame = to_local(&[from, to]);
    unit(sub(frame.points[1], frame.points[0]))
}

fn dedup_points(points: &mut Vec<[f64; 2]>) {
    points.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-11 && (a[1] - b[1]).abs() < 1e-11);
}

fn point_at_s(points: &[[f64; 2]], s: f64) -> [f64; 2] {
    if points.is_empty() {
        return [0.0; 2];
    }
    let mut remaining = s.max(0.0);
    for edge in points.windows(2) {
        let length = distance(edge[0], edge[1]);
        if remaining <= length && length > 1e-9 {
            let t = remaining / length;
            return [
                edge[0][0] + (edge[1][0] - edge[0][0]) * t,
                edge[0][1] + (edge[1][1] - edge[0][1]) * t,
            ];
        }
        remaining -= length;
    }
    *points.last().unwrap()
}

fn point_in_ring(ring: &[[f64; 2]], point: [f64; 2]) -> bool {
    if ring.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = *ring.last().unwrap();
    for &current in ring {
        if ((current[1] > point[1]) != (previous[1] > point[1]))
            && point[0]
                < (previous[0] - current[0]) * (point[1] - current[1]) / (previous[1] - current[1])
                    + current[0]
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn projected_range(points: &[[f64; 2]], axis: [f64; 2]) -> (f64, f64) {
    points.iter().fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(minimum, maximum), &point| {
            let value = dot(point, axis);
            (minimum.min(value), maximum.max(value))
        },
    )
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn unit(vector: [f64; 2]) -> [f64; 2] {
    let length = vector[0].hypot(vector[1]);
    if length < 1e-9 {
        [1.0, 0.0]
    } else {
        [vector[0] / length, vector[1] / length]
    }
}

struct Dsu {
    parent: Vec<usize>,
}

impl Dsu {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
        }
    }

    fn find(&mut self, index: usize) -> usize {
        if self.parent[index] != index {
            self.parent[index] = self.find(self.parent[index]);
        }
        self.parent[index]
    }

    fn union(&mut self, a: usize, b: usize) {
        let a = self.find(a);
        let b = self.find(b);
        if a != b {
            self.parent[b] = a;
        }
    }
}
