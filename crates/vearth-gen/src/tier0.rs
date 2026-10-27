//! Vector synthesis: rasterizes buildings,
//! roads, water and landuse polygons into per-column overlays on the cell
//! grid, and places deterministic trees. Everything is a pure function of
//! (cell, feature data, GEN_VERSION) — no unseeded randomness.

use crate::flora::{FieldSurface, select_field_surface};
use crate::tall_buildings::{TallBuildingShape, TallBuildingStyle};
use crate::vegetation::{
    TreeEnvironment, TreeProfileId, natural_tree_density_factor, select_profile, tree_color,
    tree_profile,
};
use crate::{N, PROCEDURAL_SEED_VERSION};
use std::collections::HashSet;
use vearth_core::cell::CellId;
use vearth_core::ellipsoid::MEAN_RADIUS;
use vearth_core::frame::ChunkFrame;
use vearth_core::mesh::MeshBuffers;
use vearth_data::protomaps_semantics::LandcoverKind;
use vearth_data::settlement::{SettlementAtlas, SettlementClass};
pub use vearth_data::vectors::{
    BuildingKind, ExactTree, FeaturePack, FloraRegionKind, GroundKind, ObservationHintKind,
    RoadClass, RoofShape, VectorSource, WaterRegionKind, Watercourse, WatercourseKind,
};
use vearth_data::vectors::{RegionalLandcoverMode, WaterPolygonMode, WatercourseOutputMode};

#[path = "tier0_surface_query.rs"]
mod surface_query;
pub(crate) use surface_query::SourceSurfaceQuery;

/// Level gates: what synthesizes at which LOD. All polygon/line features
/// start together at L11 — the level where vector tiles are fetched at all
/// — so regional views have no rings where a feature class cuts in; the
/// remaining rolloff is the tile scheme's own cartographic generalization
/// (coarse cells read coarse-zoom tiles), which is gradual by design.
/// Individual mapped trees appear from L17; coarser views retain the same
/// source forest-ground identity without adding tree descriptors.
pub const WATER_MIN_LEVEL: u8 = 11; // regional source water coverage
pub const ROADS_MIN_LEVEL: u8 = 11; // source roads from regional views
pub const BUILDINGS_MIN_LEVEL: u8 = 11; // direct mapped building envelopes
pub const GROUND_MIN_LEVEL: u8 = 11; // urban/park carpet from regional views
/// Mapped cliff lines are observed only at detailed source zooms. Keep their
/// conservative surface cue out of coarse cells where one column spans a
/// large mountainside.
pub const CLIFF_SURFACE_MIN_LEVEL: u8 = 14;
pub const EXACT_TREE_CROWN_GUARD_M: f32 = 26.0;
pub const TREES_MIN_LEVEL: u8 = 17; // shared physical tree meshes from detailed source support
/// Detailed rail-pair coverage for ecological masks. Rendered tracks retain
/// their physical gauge independently of this semantic sampling threshold.
pub const RAIL_DETAIL_MIN_LEVEL: u8 = 18;

const TRACK_RAIL_OFFSET_M: f32 = 1.435 * 0.5;
const TRACK_RAIL_HALF_WIDTH_M: f32 = 0.075;

/// Shared water-mask margin for ecological and shoreline-color sampling.
/// Direct water levels and triangles are independent of this semantic grid.
pub const WATER_MARGIN_COLS: usize = 8;
/// Physical width of the inferred shallow-water colour shelf. Keeping this
/// in meters rather than columns makes the tint agree across terrain LODs.
const WATER_DEPTH_RADIUS_M: f32 = 24.0;

/// Source ownership and ecology at one terrain sample square.
#[derive(Clone, Copy, Default)]
pub struct ColOverlay {
    pub road: Option<RoadStamp>,
    pub water: bool,
    /// Winning typed inland-water family. This remains transient generation
    /// state; names and context never enter generated chunks.
    pub water_kind: Option<WaterRegionKind>,
    /// Whether this water column may influence broad vegetation/ecology
    /// proximity. Decorative and drainage water still renders physically.
    pub water_ecology: bool,
    /// OSM coastline authority: an ocean polygon covers this column —
    /// sea at geoid 0, regardless of what the DEM bathymetry says.
    pub ocean: bool,
    /// Normalized distance from the nearest shoreline: 0 at the water edge,
    /// 1 once the edge is at least WATER_DEPTH_RADIUS_M away. Inland DEMs
    /// normally encode the water surface rather than the bed, so this gives
    /// the colour model a stable apparent-depth signal without extra source
    /// data. The fixed physical radius makes the value agree across LODs.
    pub water_shore: f32,
    pub building: Option<BuildingStamp>,
    pub ground: Option<GroundKind>,
    /// Winning source-preserved agricultural/floral semantic. This is visual
    /// metadata only; direct surfaces and tree roots use it as source context.
    pub flora: Option<FloraStamp>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloraStamp {
    /// Source anchor for field-wide climate and cultivation identity.
    pub location_rad: Option<[f64; 2]>,
    pub kind: FloraRegionKind,
    pub seed: u64,
    /// Zero for inferred flora; otherwise the 1-based bundled registry id.
    pub curated_style: u8,
    /// Immutable field-wide surface identity. Keeping this on the winning
    /// semantic stamp means overlap resolution, generation and the rendered
    /// terrain material all use exactly one heading and palette.
    pub surface: FieldSurface,
}

#[derive(Clone, Copy)]
struct RegionalLandcoverStamp {
    location_rad: Option<[f64; 2]>,
    kind: LandcoverKind,
    stable_identity: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WaterRegionStamp {
    kind: WaterRegionKind,
    identity: vearth_data::protomaps_semantics::SourceFeatureIdentity,
}

const fn water_identity_key(
    identity: vearth_data::protomaps_semantics::SourceFeatureIdentity,
) -> (u8, u64) {
    match identity {
        vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(value) => (0, value),
        vearth_data::protomaps_semantics::SourceFeatureIdentity::DeterministicFallback(value) => {
            (1, value)
        }
    }
}

fn water_region_wins(candidate: WaterRegionStamp, current: WaterRegionStamp) -> bool {
    candidate.kind.precedence() > current.kind.precedence()
        || (candidate.kind.precedence() == current.kind.precedence()
            && water_identity_key(candidate.identity) < water_identity_key(current.identity))
}

fn water_creates_surface(water: &vearth_data::vectors::Water, mode: WaterPolygonMode) -> bool {
    mode == WaterPolygonMode::LegacyUndifferentiated || water.creates_surface_water()
}

fn water_contributes_ecology(kind: WaterRegionKind, mode: WaterPolygonMode) -> bool {
    mode == WaterPolygonMode::LegacyUndifferentiated || kind.contributes_ecological_proximity()
}

fn watercourse_admitted(
    watercourse: &Watercourse,
    mode: WatercourseOutputMode,
    level: u8,
    max_step_m: f32,
) -> bool {
    mode == WatercourseOutputMode::RiversAndCanals
        && matches!(
            watercourse.kind,
            WatercourseKind::River | WatercourseKind::Canal
        )
        && !watercourse.intermittent
        && !watercourse.is_bridge
        && watercourse.source_tile.0 == watercourse.authority_zoom
        && level >= watercourse.kind.minimum_terrain_level()
        && max_step_m <= watercourse.width_m * 2.0
}

const fn watercourse_region_kind(kind: WatercourseKind) -> WaterRegionKind {
    match kind {
        WatercourseKind::River => WaterRegionKind::River,
        WatercourseKind::Canal => WaterRegionKind::Canal,
        WatercourseKind::Stream => WaterRegionKind::Stream,
        WatercourseKind::Drain | WatercourseKind::Ditch => WaterRegionKind::Drainage,
    }
}

const fn regional_landcover_precedence(kind: LandcoverKind) -> u8 {
    match kind {
        LandcoverKind::Grassland => 0,
        LandcoverKind::Scrub => 1,
        LandcoverKind::Farmland => 2,
        LandcoverKind::Forest => 3,
        LandcoverKind::UrbanArea => 4,
        LandcoverKind::Barren => 5,
        LandcoverKind::Glacier => 6,
        LandcoverKind::Unknown => 0,
    }
}

fn regional_landcover_wins(
    candidate: RegionalLandcoverStamp,
    current: RegionalLandcoverStamp,
) -> bool {
    regional_landcover_precedence(candidate.kind)
        .cmp(&regional_landcover_precedence(current.kind))
        .then_with(|| current.stable_identity.cmp(&candidate.stable_identity))
        .is_gt()
}

const fn regional_ground_kind(kind: LandcoverKind) -> Option<GroundKind> {
    Some(match kind {
        LandcoverKind::Barren => GroundKind::RegionalBarren,
        LandcoverKind::Forest => GroundKind::RegionalForest,
        LandcoverKind::Glacier => GroundKind::RegionalGlacier,
        LandcoverKind::Grassland => GroundKind::RegionalGrassland,
        LandcoverKind::Scrub => GroundKind::RegionalScrub,
        LandcoverKind::UrbanArea => GroundKind::RegionalUrban,
        LandcoverKind::Farmland | LandcoverKind::Unknown => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailPart {
    Rail,
}

#[derive(Clone, Copy, PartialEq)]
pub struct RoadStamp {
    pub class: RoadClass,
    pub is_bridge: bool,
    /// Coverage of the conservative semantic strip. Direct road geometry
    /// and collision use the metric source width.
    pub surface_fraction: f32,
    /// Rail-pair identity where the semantic grid resolves it. `None` retains
    /// coarse ecological coverage without changing the direct rendered track.
    pub rail_part: Option<RailPart>,
}

#[derive(Clone, Copy)]
pub struct BuildingStamp {
    // Admission identifies a fragment in this job's immutable FeaturePack.
    // Stable seeds identify art/night behavior and can repeat across tiles.
    pub(crate) source_index: Option<usize>,
    pub(crate) parent_index: Option<usize>,
    /// Whether this volume can carry occupied-room windows and facade details.
    pub facade_openings: bool,
    /// Source-reported lower height above the building datum, metres.
    pub min_h: f32,
    /// Resolved upper height, including conservative outline-sliver correction.
    pub height: f32,
    /// A source-anchored surface-model verdict admits measured roof geometry
    /// instead of an inferred roof profile.
    pub lidar: bool,
    /// Absolute geodetic roof datum when `lidar` is true. NaN leaves source
    /// relative heights authoritative.
    pub top_geo: f32,
    pub seed: u64,
    /// Whole-building outlines remain a fallback, but part geometry is
    /// authoritative wherever the two overlap so stepped massing survives.
    pub building_kind: BuildingKind,
    /// Source-reported total used for overlap precedence and facade classification.
    pub source_total_height: f32,
    /// Night facade eligibility from the same source object as its geometry.
    pub night_eligible: bool,
    /// Deterministic architectural family inferred once from footprint and
    /// height; direct boundary geometry fits its bays and floors.
    pub facade_style: FacadeStyle,
    /// Non-regional high-rise system. This changes fallback facade rhythm,
    /// structure and material treatment but never source massing.
    pub tall_style: Option<TallBuildingStyle>,
    /// Production panel system, independent of the legacy catalog examples.
    pub highrise: Option<crate::highrise::System>,
    /// Provider-free visual fallback selected from geographic context.
    pub design: crate::regional_architecture::BuildingDesign,
    pub roof_shape: Option<RoofShape>,
    pub roof_height_m: Option<f32>,
    /// Source-edge coordinates supplied while emitting a facade. Semantic
    /// grid copies leave this unset.
    pub facade: FacadeStamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacadeStyle {
    LowRise,
    Masonry,
    Townhouse,
    Continental,
    Mediterranean,
    Nordic,
    Alpine,
    Office,
    Tower,
    Industrial,
}

#[derive(Clone, Copy, Debug)]
pub struct FacadeStamp {
    pub boundary: bool,
    pub corner: bool,
    /// Stable segment ordinal within the building ring set.
    pub edge: u16,
    /// Distance from the segment start and full segment length, in metres.
    pub along_m: f32,
    pub length_m: f32,
    pub edge_distance_m: f32,
}

/// Source-anchored, canopy-gated surface sampling for building roof authority.
pub struct SurfaceSampler<'a> {
    pub at: &'a dyn Fn(f64, f64) -> Option<f32>,
}

/// Geographic instance identity for the shared tree renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeInstance {
    pub profile: TreeProfileId,
    pub seed: u64,
    pub lat_rad: f64,
    pub lon_rad: f64,
    pub elevation_m: f32,
    pub height_m: f32,
    pub yaw_rad: f32,
    pub cell_x: f32,
    pub cell_z: f32,
}

/// Nominal detail pitch used by the canonical near-tree preview.
pub const NEAR_TREE_VOXEL_M: f32 = 0.15;

/// Canonical color seed for one species. Geometry variation comes from the
/// profile; world instances retain individual height, heading and placement.
pub const fn near_tree_seed(profile: TreeProfileId) -> u64 {
    0x5354_4147_455f_5452 ^ profile as u64
}

/// Linear-light colour compensation for a shared near-field species mesh.
///
/// All surface LODs use the same world-seed variation around the canonical
/// color seed, without generating unique per-instance geometry.
pub fn near_tree_color_scale(profile: TreeProfileId, world_seed: u64) -> f32 {
    fn linear(value: u8) -> f32 {
        let value = f32::from(value) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    let representative_luminance = |seed: u64| {
        // Crown grammars mix the per-tree shade with cluster variation using
        // `varied_tree_shade`. Its expected cluster contribution is 0.5.
        let shade = hash01(seed ^ 5) * 0.58 + 0.5 * 0.42;
        let color = tree_color(profile, false, shade, 0.5, 0.55);
        0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
    };

    let canonical = representative_luminance(near_tree_seed(profile)).max(1.0e-5);
    (representative_luminance(world_seed) / canonical).clamp(0.72, 1.38)
}

pub struct CellOverlay {
    pub cols: Box<[[ColOverlay; N]; N]>,
    /// Whole source objects; independent of whether a cell centre hits a footprint.
    pub(crate) source_buildings: Vec<BuildingStamp>,
    pub(crate) source_towers: Vec<ObservationTowerSpec>,
    pub tree_instances: Vec<TreeInstance>,
}

impl CellOverlay {
    pub fn empty() -> Self {
        CellOverlay {
            cols: Box::new([[ColOverlay::default(); N]; N]),
            source_buildings: Vec::new(),
            source_towers: Vec::new(),
            tree_instances: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.source_buildings.is_empty()
            && self.source_towers.is_empty()
            && self.tree_instances.is_empty()
            && self.cols.iter().flatten().all(|c| {
                c.road.is_none()
                    && !c.water
                    && !c.ocean
                    && c.building.is_none()
                    && c.ground.is_none()
                    && c.flora.is_none()
            })
    }
}

/// Column-grid transform: local EN meters (about the cell anchor) → column
/// coordinates (0..32 across the cell), via an affine fit of the cell's uv
/// lattice. The fit's error is second-order in cell size — sub-voxel at
/// every level where vectors are enabled.
pub(crate) struct GridProj {
    origin_e: f64,
    origin_n: f64,
    /// Inverse of the 2×2 [d(EN)/d(fx) | d(EN)/d(fz)] matrix.
    inv: [f64; 4],
    /// Ground meters per column step (for width→columns conversions).
    pub(crate) step_m: f64,
    /// Longer EN axis step. Narrow water lines use this conservative scale so
    /// cube-face/polar distortion cannot turn their footprint into a broad band.
    pub(crate) max_step_m: f64,
}

impl GridProj {
    /// Conservative column extents of a metric circle, including skewed EN axes.
    pub(crate) fn radius_cols(&self, radius_m: f64) -> [f32; 2] {
        [
            self.inv[0].hypot(self.inv[1]),
            self.inv[2].hypot(self.inv[3]),
        ]
        .map(|norm| (radius_m * norm * N as f64) as f32)
    }

    pub(crate) fn of(frame: &ChunkFrame, lat0: f64, lon0: f64) -> GridProj {
        let en = |fx: f64, fz: f64| -> (f64, f64) {
            let (lat, lon) = frame.column_latlon(fx, fz);
            let mut dlon = lon - lon0;
            if dlon > std::f64::consts::PI {
                dlon -= std::f64::consts::TAU;
            } else if dlon < -std::f64::consts::PI {
                dlon += std::f64::consts::TAU;
            }
            (
                (dlon * lat0.cos() * MEAN_RADIUS),
                (lat - lat0) * MEAN_RADIUS,
            )
        };
        let (e00, n00) = en(0.0, 0.0);
        let (e10, n10) = en(1.0, 0.0);
        let (e01, n01) = en(0.0, 1.0);
        let (a, b) = (e10 - e00, e01 - e00); // d(E)/d(fx), d(E)/d(fz)
        let (c, d) = (n10 - n00, n01 - n00);
        let det = a * d - b * c;
        let inv = if det.abs() < 1e-9 {
            [0.0; 4]
        } else {
            [d / det, -b / det, -c / det, a / det]
        };
        let x_step_m = (a * a + c * c).sqrt() / N as f64;
        let z_step_m = (b * b + d * d).sqrt() / N as f64;
        let step_m = x_step_m.min(z_step_m).max(1e-6);
        let max_step_m = x_step_m.max(z_step_m).max(1e-6);
        GridProj {
            origin_e: e00,
            origin_n: n00,
            inv,
            step_m,
            max_step_m,
        }
    }

    /// EN meters → column coordinates (column x covers [x, x+1)).
    pub(crate) fn to_cols(&self, en: [f32; 2]) -> [f32; 2] {
        let (de, dn) = (en[0] as f64 - self.origin_e, en[1] as f64 - self.origin_n);
        let fx = self.inv[0] * de + self.inv[1] * dn;
        let fz = self.inv[2] * de + self.inv[3] * dn;
        [(fx * N as f64) as f32, (fz * N as f64) as f32]
    }

    fn to_en(&self, columns: [f32; 2]) -> [f32; 2] {
        let fx = f64::from(columns[0]) / N as f64;
        let fz = f64::from(columns[1]) / N as f64;
        let determinant = self.inv[0] * self.inv[3] - self.inv[1] * self.inv[2];
        [
            (self.origin_e + (self.inv[3] * fx - self.inv[1] * fz) / determinant) as f32,
            (self.origin_n + (-self.inv[2] * fx + self.inv[0] * fz) / determinant) as f32,
        ]
    }
}

fn splitmix(mut s: u64) -> u64 {
    s = s.wrapping_add(0x9E3779B97F4A7C15);
    s = (s ^ (s >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    s = (s ^ (s >> 27)).wrapping_mul(0x94D049BB133111EB);
    s ^ (s >> 31)
}

fn hash01(s: u64) -> f32 {
    (splitmix(s) >> 40) as f32 / (1u64 << 24) as f32
}

fn ring_area_m2(ring: &[[f32; 2]]) -> f32 {
    if ring.len() < 3 {
        return 0.0;
    }
    let origin = ring[0].map(f64::from);
    let twice = (0..ring.len())
        .map(|i| {
            let p = ring[i].map(f64::from);
            let q = ring[(i + 1) % ring.len()].map(f64::from);
            (p[0] - origin[0]) * (q[1] - origin[1]) - (q[0] - origin[0]) * (p[1] - origin[1])
        })
        .sum::<f64>();
    (twice.abs() * 0.5) as f32
}

pub(crate) fn building_metrics(building: &vearth_data::vectors::Building) -> TallBuildingShape {
    let outer_area = building
        .rings
        .first()
        .map_or(0.0, |ring| ring_area_m2(ring));
    let holes = building
        .rings
        .iter()
        .skip(1)
        .map(|ring| ring_area_m2(ring))
        .sum::<f32>();
    let area = (outer_area - holes).max(0.0);
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    for p in building.rings.iter().flatten() {
        lo[0] = lo[0].min(p[0]);
        lo[1] = lo[1].min(p[1]);
        hi[0] = hi[0].max(p[0]);
        hi[1] = hi[1].max(p[1]);
    }
    let a = (hi[0] - lo[0]).max(0.1);
    let b = (hi[1] - lo[1]).max(0.1);
    let outer = building
        .rings
        .first()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let perimeter = (0..outer.len())
        .map(|index| {
            let point = outer[index];
            let next = outer[(index + 1) % outer.len()];
            (next[0] - point[0]).hypot(next[1] - point[1])
        })
        .sum::<f32>();
    TallBuildingShape {
        area_m2: area,
        aspect: a.max(b) / a.min(b),
        compactness: if perimeter > 0.1 {
            (4.0 * std::f32::consts::PI * outer_area / perimeter.powi(2)).clamp(0.0, 1.0)
        } else {
            0.0
        },
        vertex_count: outer.len().min(u16::MAX as usize) as u16,
        // Courtyard holes change usable area, not the outer silhouette. Only
        // the outer ring may justify a concave/curved tower family.
        fill_ratio: (outer_area / (a * b)).clamp(0.0, 1.0),
    }
}

fn point_in_rings(point: [f32; 2], rings: &[Vec<[f32; 2]>]) -> bool {
    let mut inside = false;
    for ring in rings {
        for index in 0..ring.len() {
            let a = ring[index];
            let b = ring[(index + 1) % ring.len()];
            if (a[1] > point[1]) != (b[1] > point[1]) {
                let t = (point[1] - a[1]) / (b[1] - a[1]);
                if point[0] < a[0] + t * (b[0] - a[0]) {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

fn ring_bbox_centre(rings: &vearth_data::vectors::Rings) -> Option<[f32; 2]> {
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for point in rings.first()? {
        low[0] = low[0].min(point[0]);
        low[1] = low[1].min(point[1]);
        high[0] = high[0].max(point[0]);
        high[1] = high[1].max(point[1]);
    }
    low.into_iter()
        .chain(high)
        .all(f32::is_finite)
        .then_some([(low[0] + high[0]) * 0.5, (low[1] + high[1]) * 0.5])
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ObservationTowerSpec {
    // Pack-local geometry identity: disconnected source components may share
    // their authored feature's art seed.
    source_index: usize,
    pub centre_en: [f32; 2],
    pub height_m: f32,
    pub seed: u64,
}

fn observation_tower_specs(
    pack: &FeaturePack,
    lm_suppress: Option<&dyn Fn(&vearth_data::vectors::Rings) -> bool>,
) -> Vec<ObservationTowerSpec> {
    let mut specs: Vec<ObservationTowerSpec> = Vec::new();
    for hint in &pack.observation_hints {
        let area_limit = match hint.kind {
            ObservationHintKind::Viewpoint => 900.0,
            // `tourism=attraction` is broader than a viewpoint. Keep this
            // path deliberately tighter so a famous office skyscraper does
            // not turn into a needle tower.
            ObservationHintKind::Attraction => 650.0,
        };
        let height_floor = match hint.kind {
            ObservationHintKind::Viewpoint => 45.0,
            ObservationHintKind::Attraction => 80.0,
        };
        let anchor = pack
            .buildings
            .iter()
            .enumerate()
            .filter(|(_, building)| building.kind == BuildingKind::Building)
            .filter(|(_, building)| point_in_rings(hint.point, &building.rings))
            .filter(|(_, building)| building.height_m >= height_floor)
            .filter(|(_, building)| {
                let shape = building_metrics(building);
                shape.area_m2 <= area_limit && shape.aspect <= 1.8 && shape.compactness >= 0.42
            })
            .max_by(|(_, left), (_, right)| left.height_m.total_cmp(&right.height_m));
        let Some((source_index, anchor)) = anchor else {
            continue;
        };
        // Curated assets own both classification and massing wherever their
        // source buildings overlap. This keeps CN Tower, Space Needle, Tokyo
        // Tower and other authored towers completely untouched.
        if lm_suppress.is_some_and(|suppresses| suppresses(&anchor.rings)) {
            continue;
        }
        let Some(centre_en) = ring_bbox_centre(&anchor.rings) else {
            continue;
        };
        // Tourist/viewpoint hints do not identify a tower's construction.
        // Explicit source parts already describe its massing and must not
        // be erased by the generic needle-and-pod fallback.
        if pack.buildings.iter().any(|part| {
            part.kind.is_part()
                && part.height_m > part.min_height_m
                && (point_in_rings(centre_en, &part.rings)
                    || ring_bbox_centre(&part.rings)
                        .is_some_and(|point| point_in_rings(point, &anchor.rings)))
        }) {
            continue;
        }
        let height_m = anchor.height_m.clamp(45.0, 420.0);
        let duplicate_radius =
            crate::observation_towers::observation_tower_envelope_radius(height_m) * 1.5;
        if specs.iter().any(|existing| {
            (existing.centre_en[0] - centre_en[0]).hypot(existing.centre_en[1] - centre_en[1])
                <= duplicate_radius
        }) {
            continue;
        }
        specs.push(ObservationTowerSpec {
            source_index,
            centre_en,
            height_m,
            seed: anchor.seed,
        });
    }
    specs
}

fn observation_tower_replaces(
    spec: ObservationTowerSpec,
    source_index: usize,
    building: &vearth_data::vectors::Building,
) -> bool {
    source_index == spec.source_index || point_in_rings(spec.centre_en, &building.rings)
}

/// Even-odd scanline fill of one polygon (ring set) over the column grid;
/// calls `mark(x, z)` for every covered column center.
fn fill_polygon(rings: &[Vec<[f32; 2]>], mut mark: impl FnMut(usize, usize)) {
    fill_polygon_grid(rings, N, 0, &mut |x, z| mark(x, z));
}

fn flora_stamp_wins(candidate: FloraStamp, current: FloraStamp) -> bool {
    (candidate.kind.precedence(), candidate.seed, candidate.kind)
        > (current.kind.precedence(), current.seed, current.kind)
}

/// Grid-parameterized fill: ring coordinates stay in cell-column space, but
/// marking covers an `n`×`n` grid whose origin sits `off` columns before the
/// cell's (used for the water mask margin). `mark` receives grid coords.
fn fill_polygon_grid(
    rings: &[Vec<[f32; 2]>],
    n: usize,
    off: i64,
    mark: &mut dyn FnMut(usize, usize),
) {
    let mut xs: Vec<f32> = Vec::with_capacity(16);
    for zg in 0..n {
        let zc = (zg as i64 - off) as f32 + 0.5;
        xs.clear();
        for ring in rings {
            let m = ring.len();
            if m < 3 {
                continue;
            }
            for i in 0..m {
                let p = ring[i];
                let q = ring[(i + 1) % m];
                if (p[1] <= zc) != (q[1] <= zc) {
                    xs.push(p[0] + (zc - p[1]) * (q[0] - p[0]) / (q[1] - p[1]));
                }
            }
        }
        xs.sort_unstable_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            let x0 = ((pair[0] - 0.5).ceil() as i64 + off).max(0);
            let x1 = ((pair[1] - 0.5).floor() as i64 + off).min(n as i64 - 1);
            for x in x0..=x1 {
                mark(x as usize, zg);
            }
        }
    }
}

/// Stamp a polyline of the given half-width; marks covered columns.
pub(crate) fn stamp_polyline(
    pts: &[[f32; 2]],
    radius_cols: f32,
    mut mark: impl FnMut(usize, usize),
) {
    let r = radius_cols.max(0.51); // a road is never thinner than a column
    let mut visit = |cx: f32, cz: f32| {
        let x0 = (cx - r).floor().max(0.0) as i64;
        let x1 = ((cx + r).floor()).min(N as f32 - 1.0) as i64;
        let z0 = (cz - r).floor().max(0.0) as i64;
        let z1 = ((cz + r).floor()).min(N as f32 - 1.0) as i64;
        for z in z0..=z1 {
            for x in x0..=x1 {
                let (dx, dz) = (x as f32 + 0.5 - cx, z as f32 + 0.5 - cz);
                if dx * dx + dz * dz <= r * r {
                    mark(x as usize, z as usize);
                }
            }
        }
    };
    let pad = r + 1.0;
    for seg in pts.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        // Quick reject: segment bbox entirely off-grid.
        if a[0].max(b[0]) < -pad
            || a[0].min(b[0]) > N as f32 + pad
            || a[1].max(b[1]) < -pad
            || a[1].min(b[1]) > N as f32 + pad
        {
            continue;
        }
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let steps = (len / 0.5).ceil().clamp(1.0, 4096.0) as usize;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            visit(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
        }
    }
}

fn clip_segment_to_square(
    a: [f32; 2],
    b: [f32; 2],
    min: f32,
    max: f32,
) -> Option<([f32; 2], [f32; 2])> {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let mut enter = 0.0f32;
    let mut exit = 1.0f32;
    for (p, q) in [
        (-delta[0], a[0] - min),
        (delta[0], max - a[0]),
        (-delta[1], a[1] - min),
        (delta[1], max - a[1]),
    ] {
        if p.abs() <= f32::EPSILON {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            enter = enter.max(t);
        } else {
            exit = exit.min(t);
        }
        if enter > exit {
            return None;
        }
    }
    Some((
        [a[0] + delta[0] * enter, a[1] + delta[1] * enter],
        [a[0] + delta[0] * exit, a[1] + delta[1] * exit],
    ))
}

/// Margin-grid watercourse stroke. Unlike the legacy road helper this never
/// forces a minimum physical radius. Column centers inside the metric stroke
/// are selected, while one deterministic primary/bridge path keeps a
/// sub-column centerline 4-connected without inflating it to a road-like band.
fn stamp_watercourse_grid(
    pts: &[[f32; 2]],
    radius_cols: f32,
    n: usize,
    off: i64,
    mark: &mut dyn FnMut(usize, usize),
) {
    let radius = radius_cols.max(0.0);
    let coverage = radius;
    let mut previous_primary: Option<(i64, i64)> = None;
    let mut visit = |cx: f32, cz: f32| {
        let (gx, gz) = (cx + off as f32, cz + off as f32);
        let x0 = (gx - coverage).floor() as i64;
        let x1 = (gx + coverage).floor() as i64;
        let z0 = (gz - coverage).floor() as i64;
        let z1 = (gz + coverage).floor() as i64;
        for z in z0.max(0)..=z1.min(n as i64 - 1) {
            for x in x0.max(0)..=x1.min(n as i64 - 1) {
                let (dx, dz) = (x as f32 + 0.5 - gx, z as f32 + 0.5 - gz);
                if dx * dx + dz * dz <= coverage * coverage {
                    mark(x as usize, z as usize);
                }
            }
        }

        let primary = (gx.floor() as i64, gz.floor() as i64);
        if let Some(previous) = previous_primary
            && previous.0 != primary.0
            && previous.1 != primary.1
        {
            let bridge = (primary.0, previous.1).min((previous.0, primary.1));
            if bridge.0 >= 0 && bridge.1 >= 0 && bridge.0 < n as i64 && bridge.1 < n as i64 {
                mark(bridge.0 as usize, bridge.1 as usize);
            }
        }
        if primary.0 >= 0 && primary.1 >= 0 && primary.0 < n as i64 && primary.1 < n as i64 {
            mark(primary.0 as usize, primary.1 as usize);
        }
        previous_primary = Some(primary);
    };

    let pad = coverage + 1.0;
    for segment in pts.windows(2) {
        let min = -off as f32 - pad;
        let max = n as f32 - off as f32 + pad;
        let Some((a, b)) = clip_segment_to_square(segment[0], segment[1], min, max) else {
            continue;
        };
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        let steps = (length / 0.25).ceil().clamp(1.0, 8192.0) as usize;
        for index in 0..=steps {
            let t = index as f32 / steps as f32;
            visit(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
        }
    }
}

pub(crate) fn rail_part_at(lateral_m: f32, step_m: f32, level: u8) -> Option<RailPart> {
    if level < RAIL_DETAIL_MIN_LEVEL {
        return None;
    }

    // Treat a sub-voxel steel rail as covering a column when the physical
    // centerline intersects its footprint. This keeps both rails continuous
    // through the L18→L19 handoff without inflating the real-world gauge.
    let rail_tolerance = TRACK_RAIL_HALF_WIDTH_M + step_m * 0.5;
    if (lateral_m - TRACK_RAIL_OFFSET_M).abs() <= rail_tolerance {
        return Some(RailPart::Rail);
    }
    None
}

/// Rasterize only the standard-gauge steel pair. Columns between and around
/// the rails remain untouched so their existing terrain/ground material shows
/// through; no synthesized track bed or sleepers are introduced.
pub(crate) fn stamp_railway(
    pts: &[[f32; 2]],
    step_m: f32,
    level: u8,
    mut mark: impl FnMut(usize, usize, RailPart),
) {
    let radius_cols = (TRACK_RAIL_OFFSET_M + TRACK_RAIL_HALF_WIDTH_M) / step_m + 0.5;
    for segment in pts.windows(2) {
        let (a, b) = (segment[0], segment[1]);
        let direction = [b[0] - a[0], b[1] - a[1]];
        let length2 = direction[0] * direction[0] + direction[1] * direction[1];
        if length2 < 1.0e-8 {
            continue;
        }
        let x0 = (a[0].min(b[0]) - radius_cols).floor().max(0.0) as i64;
        let x1 = (a[0].max(b[0]) + radius_cols).floor().min(N as f32 - 1.0) as i64;
        let z0 = (a[1].min(b[1]) - radius_cols).floor().max(0.0) as i64;
        let z1 = (a[1].max(b[1]) + radius_cols).floor().min(N as f32 - 1.0) as i64;
        if x0 <= x1 && z0 <= z1 {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let column = [x as f32 + 0.5, z as f32 + 0.5];
                    let offset = [column[0] - a[0], column[1] - a[1]];
                    let t = ((offset[0] * direction[0] + offset[1] * direction[1]) / length2)
                        .clamp(0.0, 1.0);
                    let closest = [a[0] + direction[0] * t, a[1] + direction[1] * t];
                    let lateral_m = (column[0] - closest[0]).hypot(column[1] - closest[1]) * step_m;
                    if let Some(part) = rail_part_at(lateral_m, step_m, level) {
                        mark(x as usize, z as usize, part);
                    }
                }
            }
        }
    }
}

fn rail_part_priority(part: Option<RailPart>) -> u8 {
    match part {
        None => 0,
        Some(RailPart::Rail) => 1,
    }
}

fn road_priority(c: RoadClass) -> u8 {
    match c {
        RoadClass::Highway => 6,
        RoadClass::Major => 5,
        RoadClass::Medium => 4,
        RoadClass::Minor => 3,
        RoadClass::Service => 2,
        RoadClass::Rail => 1,
        RoadClass::Path => 0,
    }
}

fn road_stamp_wins(candidate: RoadStamp, current: RoadStamp) -> bool {
    let candidate_priority = road_priority(candidate.class);
    let current_priority = road_priority(current.class);
    candidate_priority > current_priority
        || (candidate_priority == current_priority
            && candidate.class == RoadClass::Rail
            && rail_part_priority(candidate.rail_part) > rail_part_priority(current.rail_part))
}

/// Road terrain conditioning gate: below 9.6 m voxels a road is a
/// one-column map line and grading would be invisible.
pub const ROAD_GRADE_MIN_LEVEL: u8 = 15;

/// Along-way DEM smoothing window and endpoint taper, meters of arclength.
/// The taper blends clipped way ends into raw terrain, which also bounds the
/// profile mismatch where MVT tiles clip the same way differently.
const GRADE_TAPER_M: f64 = 45.0;

/// Per-column road grading: (blend weight 0..1, target geodetic elevation).
/// Weight 0 = column untouched.
pub type RoadGrades = Box<[[(f32, f32); N]; N]>;

pub(crate) struct RoadSurfaceGrades {
    pub columns: RoadGrades,
    pub corners: Box<[[(f32, f32); N + 1]; N + 1]>,
}

/// Terrain conditioning along roads: sample the DEM along each
/// non-bridge way, box-smooth the profile by arclength, and stamp the
/// smoothed profile onto nearby columns — full weight across the road
/// width, falling off over a shoulder. Combination keeps the max-weight
/// target (min target on ties), so the duplicate way copies that MVT tile
/// clipping produces stamp idempotently, and junctions resolve
/// deterministically regardless of iteration order. Everything is a
/// function of way geometry + geographic DEM samples, so adjacent cells
/// grade their shared border identically.
pub fn road_grades(
    pack: &FeaturePack,
    frame: &ChunkFrame,
    level: u8,
    elev_en: &dyn Fn(f64, f64) -> f32,
) -> Option<RoadGrades> {
    road_surface_grades(pack, frame, level, elev_en).map(|grades| grades.columns)
}

pub(crate) fn road_surface_grades(
    pack: &FeaturePack,
    frame: &ChunkFrame,
    level: u8,
    elev_en: &dyn Fn(f64, f64) -> f32,
) -> Option<RoadSurfaceGrades> {
    if level < ROAD_GRADE_MIN_LEVEL || pack.roads.is_empty() {
        return None;
    }
    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    let proj = GridProj::of(frame, lat0, lon0);
    let step = proj.step_m;
    let mut grid: RoadGrades = Box::new([[(0f32, 0f32); N]; N]);
    let mut corners = Box::new([[(0f32, 0f32); N + 1]; N + 1]);
    let smooth = |t: f64| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };

    for road in &pack.roads {
        if road.is_bridge || road.pts.len() < 2 {
            continue;
        }
        let w_m = road.width_m() as f64;
        let (smooth_m, grade_strength, shoulder_m) = road.profile.detail.grading(road.class);
        let r_in = w_m * 0.5;
        // Shoulder: a couple of columns, at least a fraction of the width.
        let r_out = r_in
            + if grade_strength < 1.0 {
                shoulder_m
            } else {
                (step * 1.8).max(shoulder_m)
            };
        // Stations resolve the along-way profile; interpolation between them
        // avoids flat plateaus from overlapping constant-height station discs.
        let ds = (r_in * 0.9).clamp(2.0, 12.0);

        // Quick reject: way bbox vs grid bbox (in columns, with reach).
        let reach = (r_out / step) as f32 + 1.0;
        let cols_pts: Vec<[f32; 2]> = road.pts.iter().map(|&p| proj.to_cols(p)).collect();
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for c in &cols_pts {
            for a in 0..2 {
                lo[a] = lo[a].min(c[a]);
                hi[a] = hi[a].max(c[a]);
            }
        }
        if hi[0] < -reach || lo[0] > N as f32 + reach || hi[1] < -reach || lo[1] > N as f32 + reach
        {
            continue;
        }

        // Resample the way to uniform arclength stations.
        let mut seg_len: Vec<f64> = Vec::with_capacity(road.pts.len() - 1);
        let mut total = 0f64;
        for seg in road.pts.windows(2) {
            let (dx, dy) = (
                (seg[1][0] - seg[0][0]) as f64,
                (seg[1][1] - seg[0][1]) as f64,
            );
            let l = (dx * dx + dy * dy).sqrt();
            seg_len.push(l);
            total += l;
        }
        if total < 1e-3 {
            continue;
        }
        let n_st = ((total / ds).ceil() as usize).clamp(1, 4096);
        let mut stations: Vec<([f64; 2], f64)> = Vec::with_capacity(n_st + 1); // (EN, s)
        {
            let (mut seg, mut s_seg) = (0usize, 0f64);
            for i in 0..=n_st {
                let s = total * i as f64 / n_st as f64;
                while seg + 1 < seg_len.len() && s_seg + seg_len[seg] < s {
                    s_seg += seg_len[seg];
                    seg += 1;
                }
                let t = if seg_len[seg] > 1e-9 {
                    (s - s_seg) / seg_len[seg]
                } else {
                    0.0
                };
                let (a, b) = (road.pts[seg], road.pts[seg + 1]);
                let en = [
                    a[0] as f64 + (b[0] as f64 - a[0] as f64) * t,
                    a[1] as f64 + (b[1] as f64 - a[1] as f64) * t,
                ];
                stations.push((en, s));
            }
        }

        // Only segments whose shoulder can touch this cell need their two
        // endpoint profiles. Their smoothing windows also serve the corner
        // lattice, so continuous rendering adds no second DEM sampling pass.
        let r_out_cols = (r_out / step) as f32;
        let k = ((smooth_m * n_st as f64 / total).round() as usize).min(n_st);
        let station_cols: Vec<_> = stations
            .iter()
            .map(|(en, _)| proj.to_cols([en[0] as f32, en[1] as f32]))
            .collect();
        let mut active = vec![false; n_st];
        let mut profile_needed = vec![false; stations.len()];
        let mut needed = vec![false; stations.len()];
        for i in 0..n_st {
            let (a, b) = (station_cols[i], station_cols[i + 1]);
            let intersects = a[0].max(b[0]) + r_out_cols >= 0.0
                && a[0].min(b[0]) - r_out_cols <= N as f32
                && a[1].max(b[1]) + r_out_cols >= 0.0
                && a[1].min(b[1]) - r_out_cols <= N as f32;
            if !intersects {
                continue;
            }
            active[i] = true;
            profile_needed[i..=i + 1].fill(true);
            needed[i.saturating_sub(k)..=(i + 1 + k).min(n_st)].fill(true);
        }
        if !active.iter().any(|active| *active) {
            continue;
        }
        let mut h_raw = vec![0.0f32; stations.len()];
        for (i, (en, _)) in stations.iter().enumerate() {
            if needed[i] {
                h_raw[i] = elev_en(en[0], en[1]);
            }
        }

        let mut targets = vec![0.0_f32; stations.len()];
        for i in 0..stations.len() {
            if profile_needed[i] {
                let (a, b) = (i.saturating_sub(k), (i + k).min(n_st));
                targets[i] = (h_raw[a..=b]
                    .iter()
                    .map(|height| *height as f64)
                    .sum::<f64>()
                    / (b + 1 - a) as f64) as f32;
            }
        }
        let mut road_columns = Box::new([[(0.0, 0.0, f32::INFINITY); N]; N]);
        let mut road_corners = Box::new([[(0.0, 0.0, f32::INFINITY); N + 1]; N + 1]);
        // Find the nearest segment before combining separate ways. Full-width
        // road weights tie; selecting the lowest endpoint of every overlapping
        // segment would recreate the station plateaus even with interpolation.
        for i in 0..n_st {
            if !active[i] {
                continue;
            }
            let (a, b) = (station_cols[i], station_cols[i + 1]);
            let dx = b[0] - a[0];
            let dz = b[1] - a[1];
            let length_sq = dx * dx + dz * dz;
            if length_sq <= 1.0e-12 {
                continue;
            }
            let x0 = (a[0].min(b[0]) - r_out_cols).floor().max(0.0) as i64;
            let x1 = (a[0].max(b[0]) + r_out_cols).floor().min(N as f32) as i64;
            let z0 = (a[1].min(b[1]) - r_out_cols).floor().max(0.0) as i64;
            let z1 = (a[1].max(b[1]) + r_out_cols).floor().min(N as f32) as i64;
            if x1 < x0 || z1 < z0 {
                continue;
            }
            let sample = |x: f32, z: f32| {
                let t = (((x - a[0]) * dx + (z - a[1]) * dz) / length_sq).clamp(0.0, 1.0);
                let d_m = (x - a[0] - dx * t).hypot(z - a[1] - dz * t) as f64 * step;
                let s = stations[i].1 + (stations[i + 1].1 - stations[i].1) * f64::from(t);
                let taper = smooth(s.min(total - s) / GRADE_TAPER_M);
                let w =
                    (smooth(1.0 - (d_m - r_in) / (r_out - r_in)) * taper) as f32 * grade_strength;
                (
                    w,
                    targets[i] + (targets[i + 1] - targets[i]) * t,
                    d_m as f32,
                )
            };
            let nearest = |slot: &mut (f32, f32, f32), sample: (f32, f32, f32)| {
                if sample.0 > 0.0
                    && (sample.2 < slot.2 || (sample.2 == slot.2 && sample.1 < slot.1))
                {
                    *slot = sample;
                }
            };
            for z in z0..=z1 {
                for x in x0..=x1 {
                    if x < N as i64 && z < N as i64 {
                        nearest(
                            &mut road_columns[z as usize][x as usize],
                            sample(x as f32 + 0.5, z as f32 + 0.5),
                        );
                    }
                    nearest(
                        &mut road_corners[z as usize][x as usize],
                        sample(x as f32, z as f32),
                    );
                }
            }
        }
        let merge = |slot: &mut (f32, f32), sample: &(f32, f32, f32)| {
            if sample.0 > slot.0 || (sample.0 > 0.0 && sample.0 == slot.0 && sample.1 < slot.1) {
                *slot = (sample.0, sample.1);
            }
        };
        for (slot, sample) in grid.iter_mut().flatten().zip(road_columns.iter().flatten()) {
            merge(slot, sample);
        }
        for (slot, sample) in corners
            .iter_mut()
            .flatten()
            .zip(road_corners.iter().flatten())
        {
            merge(slot, sample);
        }
    }
    Some(RoadSurfaceGrades {
        columns: grid,
        corners,
    })
}

/// Build the cell overlay from a feature pack. `elev_geo(x, z)` is the
/// per-column *geodetic* terrain elevation (meters, no exaggeration), valid
/// on `[-margin, N+margin)²` of the shared column lattice — used for the
/// water flatten levels. `margin` is the extra sampled ring in columns
/// (`WATER_MARGIN_COLS` when the pack has water, else 0).
pub fn build_overlay(
    cell: CellId,
    frame: &ChunkFrame,
    pack: &FeaturePack,
    elev_geo: &dyn Fn(i32, i32) -> f32,
    margin: usize,
    surf: Option<&SurfaceSampler<'_>>,
    lm_suppress: Option<&dyn Fn(&vearth_data::vectors::Rings) -> bool>,
    night_eligible: Option<&dyn Fn(u64) -> bool>,
) -> CellOverlay {
    build_overlay_with_tree_ground(
        cell,
        frame,
        pack,
        elev_geo,
        margin,
        surf,
        lm_suppress,
        night_eligible,
        None,
    )
}

pub(crate) fn build_overlay_with_tree_ground(
    cell: CellId,
    frame: &ChunkFrame,
    pack: &FeaturePack,
    elev_geo: &dyn Fn(i32, i32) -> f32,
    margin: usize,
    surf: Option<&SurfaceSampler<'_>>,
    lm_suppress: Option<&dyn Fn(&vearth_data::vectors::Rings) -> bool>,
    night_eligible: Option<&dyn Fn(u64) -> bool>,
    tree_ground: Option<&dyn Fn(f64, f64) -> Option<f32>>,
) -> CellOverlay {
    let mut ov = CellOverlay::empty();
    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    let proj = GridProj::of(frame, lat0, lon0);
    let level = cell.level;

    let to_cols = |rings: &vearth_data::vectors::Rings| -> Vec<Vec<[f32; 2]>> {
        rings
            .iter()
            .map(|r| r.iter().map(|&p| proj.to_cols(p)).collect())
            .collect()
    };

    // Ground tint, beneath all other features.
    if level >= GROUND_MIN_LEVEL {
        // Resolve the coarse z7 context into one stable winner per column
        // before any detailed output. The provenance mask lets
        // every detailed phase replace regional farmland regardless of its
        // field seed while retaining the existing detailed-flora tie policy.
        let mut regional_winners = Box::new([[None; N]; N]);
        let mut regional_flora = Box::new([[false; N]; N]);
        for region in &pack.regional_landcover {
            if pack.regional_landcover_mode == RegionalLandcoverMode::ExistingFarmlandOnly
                && region.kind != LandcoverKind::Farmland
            {
                continue;
            }
            let candidate = RegionalLandcoverStamp {
                location_rad: region.location_rad,
                kind: region.kind,
                stable_identity: region.stable_identity,
            };
            fill_polygon(&to_cols(&region.rings), |x, z| {
                if regional_winners[z][x]
                    .is_none_or(|current| regional_landcover_wins(candidate, current))
                {
                    regional_winners[z][x] = Some(candidate);
                }
            });
        }
        for z in 0..N {
            for x in 0..N {
                let Some(stamp) = regional_winners[z][x] else {
                    continue;
                };
                let col = &mut ov.cols[z][x];
                if stamp.kind == LandcoverKind::Farmland {
                    // Generalized z7 landcover can enclose entire settlements.
                    // Density evidence only vetoes this inferred planting;
                    // detailed mapped fields below retain their authority.
                    let (lat, lon) = frame
                        .column_latlon((x as f64 + 0.5) / N as f64, (z as f64 + 0.5) / N as f64);
                    if matches!(
                        SettlementAtlas::bundled().class_at(lat, lon),
                        SettlementClass::City | SettlementClass::Town | SettlementClass::Suburban
                    ) {
                        continue;
                    }
                    col.ground = Some(GroundKind::Green);
                    col.flora = Some(FloraStamp {
                        location_rad: stamp.location_rad.map(crate::flora::field_context_location),
                        kind: FloraRegionKind::Farmland,
                        seed: stamp.stable_identity,
                        curated_style: 0,
                        surface: select_field_surface(
                            FloraRegionKind::Farmland,
                            stamp.stable_identity,
                        ),
                    });
                    regional_flora[z][x] = true;
                } else if let Some(kind) = regional_ground_kind(stamp.kind) {
                    col.ground = Some(kind);
                }
            }
        }
        // Keep one forest-ground identity through the tree gate. At regional
        // LODs this reads as a canopy carpet; once tree geometry appears it
        // becomes the shaded floor visible between trunks and crowns. Explicit
        // ground kinds below still win where source polygons overlap.
        for rings in &pack.forest {
            fill_polygon(&to_cols(rings), |x, z| {
                let col = &mut ov.cols[z][x];
                if regional_flora[z][x] {
                    col.flora = None;
                    regional_flora[z][x] = false;
                }
                col.ground = Some(GroundKind::Forest);
            });
        }
        // New natural semantics are resolved as a class before legacy ground
        // and existing mapped forest/flora. This makes target-kind overlaps
        // independent of source feature order while retaining the stronger
        // established higher-authority outputs.
        for (kind, rings) in pack
            .ground
            .iter()
            .filter(|(kind, _)| kind.is_natural_semantic())
        {
            fill_polygon(&to_cols(rings), |x, z| {
                let current = ov.cols[z][x].ground;
                if current == Some(GroundKind::Forest)
                    || current.is_some_and(|current| {
                        current.is_natural_semantic()
                            && current.natural_precedence() >= kind.natural_precedence()
                    })
                {
                    return;
                }
                let col = &mut ov.cols[z][x];
                if regional_flora[z][x] {
                    col.flora = None;
                    regional_flora[z][x] = false;
                }
                col.ground = Some(*kind);
            });
        }
        // Managed substrates are similarly resolved by class rather than
        // feature iteration order. They supersede general natural cover, but
        // not established mapped forest, legacy detailed ground, or the flora
        // phase below.
        for (kind, rings) in pack
            .ground
            .iter()
            .filter(|(kind, _)| kind.is_managed_semantic())
        {
            fill_polygon(&to_cols(rings), |x, z| {
                let current = ov.cols[z][x].ground;
                if current == Some(GroundKind::Forest)
                    || current.is_some_and(|current| {
                        current.is_managed_semantic()
                            && current.managed_precedence() >= kind.managed_precedence()
                    })
                {
                    return;
                }
                let col = &mut ov.cols[z][x];
                if regional_flora[z][x] {
                    col.flora = None;
                    regional_flora[z][x] = false;
                }
                col.ground = Some(*kind);
            });
        }
        for (kind, rings) in pack
            .ground
            .iter()
            .filter(|(kind, _)| !kind.is_natural_semantic() && !kind.is_managed_semantic())
        {
            fill_polygon(&to_cols(rings), |x, z| {
                let col = &mut ov.cols[z][x];
                if regional_flora[z][x] {
                    col.flora = None;
                    regional_flora[z][x] = false;
                }
                col.ground = Some(*kind);
            });
        }
        for region in &pack.flora {
            let candidate = FloraStamp {
                location_rad: region
                    .location_rad
                    .map(crate::flora::field_context_location),
                kind: region.kind,
                seed: region.seed,
                curated_style: 0,
                surface: select_field_surface(region.kind, region.seed),
            };
            fill_polygon(&to_cols(&region.rings), |x, z| {
                let col = &mut ov.cols[z][x];
                // Paving, urban fabric and sand are physical/surface
                // exclusions. Green/forest overlaps remain eligible; the
                // semantic field material replaces their generic top-surface
                // tint later without changing occupancy.
                if matches!(
                    col.ground,
                    Some(GroundKind::Paved | GroundKind::Urban | GroundKind::Sand)
                ) {
                    return;
                }
                if regional_flora[z][x]
                    || col
                        .flora
                        .is_none_or(|current| flora_stamp_wins(candidate, current))
                {
                    col.flora = Some(candidate);
                    col.ground = Some(GroundKind::Green);
                    regional_flora[z][x] = false;
                }
            });
        }
    }

    // Water polygons and admitted watercourses.
    let admitted_watercourses = pack
        .watercourses
        .iter()
        .filter(|watercourse| {
            watercourse_admitted(
                watercourse,
                pack.watercourse_output_mode,
                level,
                proj.max_step_m as f32,
            )
        })
        .collect::<Vec<_>>();
    if level >= WATER_MIN_LEVEL && (!pack.water.is_empty() || !admitted_watercourses.is_empty()) {
        // Ocean polygons are the coastline authority: sea at geoid 0.
        // Rasterize a margin ring even though ocean needs no elevation
        // flattening; it supplies a seam-stable nearshore depth cue.
        let shore_margin = ((WATER_DEPTH_RADIUS_M / proj.step_m as f32).ceil() as usize).max(1);
        let shore_ne = N + 2 * shore_margin;
        let mut ocean_mask = vec![false; shore_ne * shore_ne];
        let mut inland_shore_mask = vec![false; shore_ne * shore_ne];
        let mut inland_winners = vec![None; shore_ne * shore_ne];
        for w in pack.water.iter().filter(|w| w.is_ocean()) {
            fill_polygon_grid(
                &to_cols(&w.rings),
                shore_ne,
                shore_margin as i64,
                &mut |x, z| ocean_mask[z * shore_ne + x] = true,
            );
        }
        for w in pack
            .water
            .iter()
            .filter(|w| !w.is_ocean() && water_creates_surface(w, pack.water_polygon_mode))
        {
            fill_polygon_grid(
                &to_cols(&w.rings),
                shore_ne,
                shore_margin as i64,
                &mut |x, z| {
                    let index = z * shore_ne + x;
                    let candidate = WaterRegionStamp {
                        kind: w.kind,
                        identity: w.identity,
                    };
                    if inland_winners[index]
                        .is_none_or(|current| water_region_wins(candidate, current))
                    {
                        inland_winners[index] = Some(candidate);
                    }
                    inland_shore_mask[index] = true;
                },
            );
        }
        let polygon_inland_shore_mask = inland_shore_mask.clone();
        for watercourse in &admitted_watercourses {
            let candidate = WaterRegionStamp {
                kind: watercourse_region_kind(watercourse.kind),
                identity: watercourse.identity,
            };
            let radius_cols = watercourse.width_m * 0.5 / proj.max_step_m as f32;
            for path in &watercourse.paths {
                let points = path
                    .iter()
                    .map(|&point| proj.to_cols(point))
                    .collect::<Vec<_>>();
                stamp_watercourse_grid(
                    &points,
                    radius_cols,
                    shore_ne,
                    shore_margin as i64,
                    &mut |x, z| {
                        let index = z * shore_ne + x;
                        if ocean_mask[index] || polygon_inland_shore_mask[index] {
                            return;
                        }
                        if inland_winners[index]
                            .is_none_or(|current| water_region_wins(candidate, current))
                        {
                            inland_winners[index] = Some(candidate);
                        }
                        inland_shore_mask[index] = true;
                    },
                );
            }
        }
        // A partial vector pack can expose an MVT tile edge as an apparent
        // coastline. Disable the edge cue until coverage is complete rather
        // than painting rectangular shallow-water patches during retries.
        let shore_signal = |mask: &[bool]| {
            if pack.complete {
                water_shore_factors(
                    mask,
                    shore_ne,
                    shore_margin,
                    proj.step_m as f32,
                    WATER_DEPTH_RADIUS_M,
                )
            } else {
                mask.iter()
                    .map(|&water| if water { 1.0 } else { 0.0 })
                    .collect()
            }
        };
        let ocean_shore = shore_signal(&ocean_mask);
        let inland_shore = shore_signal(&inland_shore_mask);
        for z in 0..N {
            for x in 0..N {
                let i = (z + shore_margin) * shore_ne + x + shore_margin;
                if ocean_mask[i] {
                    ov.cols[z][x].ocean = true;
                    ov.cols[z][x].water_kind = Some(WaterRegionKind::Ocean);
                    ov.cols[z][x].water_ecology = true;
                    ov.cols[z][x].water_shore = ocean_shore[i];
                }
            }
        }
        let has_polygon_inland = pack
            .water
            .iter()
            .any(|w| !w.is_ocean() && water_creates_surface(w, pack.water_polygon_mode));
        if has_polygon_inland || !admitted_watercourses.is_empty() {
            // Inland water mask over the margin-expanded grid: border
            // columns must see their full flatten window or levels would
            // step at cell borders.
            let ne = N + 2 * margin;
            let mut mask = vec![false; ne * ne];
            let mut ocean_level_mask = vec![false; ne * ne];
            for water in pack.water.iter().filter(|water| water.is_ocean()) {
                fill_polygon_grid(&to_cols(&water.rings), ne, margin as i64, &mut |x, z| {
                    ocean_level_mask[z * ne + x] = true
                });
            }
            for w in pack
                .water
                .iter()
                .filter(|w| !w.is_ocean() && water_creates_surface(w, pack.water_polygon_mode))
            {
                fill_polygon_grid(&to_cols(&w.rings), ne, margin as i64, &mut |x, z| {
                    mask[z * ne + x] = true
                });
            }
            let polygon_level_mask = mask.clone();
            for watercourse in &admitted_watercourses {
                let radius_cols = watercourse.width_m * 0.5 / proj.max_step_m as f32;
                for path in &watercourse.paths {
                    let points = path
                        .iter()
                        .map(|&point| proj.to_cols(point))
                        .collect::<Vec<_>>();
                    stamp_watercourse_grid(&points, radius_cols, ne, margin as i64, &mut |x, z| {
                        let index = z * ne + x;
                        if !ocean_level_mask[index] && !polygon_level_mask[index] {
                            mask[index] = true;
                        }
                    });
                }
            }
            for z in 0..N {
                for x in 0..N {
                    let i = (z + margin) * ne + x + margin;
                    if mask[i] && !ov.cols[z][x].ocean {
                        ov.cols[z][x].water = true;
                        if let Some(winner) =
                            inland_winners[(z + shore_margin) * shore_ne + x + shore_margin]
                        {
                            ov.cols[z][x].water_kind = Some(winner.kind);
                            ov.cols[z][x].water_ecology =
                                water_contributes_ecology(winner.kind, pack.water_polygon_mode);
                        }
                    }
                }
            }
            for z in 0..N {
                for x in 0..N {
                    let i = (z + shore_margin) * shore_ne + x + shore_margin;
                    if ov.cols[z][x].water && !ov.cols[z][x].ocean {
                        ov.cols[z][x].water_shore = inland_shore[i];
                    }
                }
            }
        }
        beach_sand(&mut ov, elev_geo, level, proj.step_m);
    }

    // Roads.
    if level >= ROADS_MIN_LEVEL {
        for road in &pack.roads {
            if level >= crate::bridges::BRIDGE_MIN_LEVEL && road.is_bridge {
                // Bridge collision and visible decks share the continuous
                // profile, with plain source-way decks outside catalog coverage.
                continue;
            }
            let pts: Vec<[f32; 2]> = road.pts.iter().map(|&p| proj.to_cols(p)).collect();
            if road.class == RoadClass::Rail && level >= RAIL_DETAIL_MIN_LEVEL {
                stamp_railway(&pts, proj.step_m as f32, level, |x, z, rail_part| {
                    let stamp = RoadStamp {
                        class: road.class,
                        is_bridge: road.is_bridge,
                        surface_fraction: 1.0,
                        rail_part: Some(rail_part),
                    };
                    let slot = &mut ov.cols[z][x].road;
                    if slot.is_none_or(|current| road_stamp_wins(stamp, current)) {
                        *slot = Some(stamp);
                    }
                });
            } else {
                let stamp = RoadStamp {
                    class: road.class,
                    is_bridge: road.is_bridge,
                    surface_fraction: if road.is_bridge || road.class == RoadClass::Rail {
                        1.0
                    } else {
                        let width = road.width_m();
                        width / width.max(1.02 * proj.step_m as f32)
                    },
                    rail_part: None,
                };
                let r_cols = (road.width_m() as f64 * 0.5 / proj.step_m) as f32;
                stamp_polyline(&pts, r_cols, |x, z| {
                    let slot = &mut ov.cols[z][x].road;
                    if slot.is_none_or(|current| road_stamp_wins(stamp, current)) {
                        *slot = Some(stamp);
                    }
                });
            }
        }
    }

    // Buildings.
    if level >= BUILDINGS_MIN_LEVEL {
        let observation_towers = observation_tower_specs(pack, lm_suppress);
        let composition = crate::building_composition::resolve(&pack.buildings);
        let mut design_cache = vec![None; pack.buildings.len()];
        for (source_index, b) in pack.buildings.iter().enumerate() {
            // Whole-feature landmark suppression: a footprint that
            // overlaps a curated model's bbox never stamps at all — same
            // verdict in every cell that sees the polygon.
            if lm_suppress.is_some_and(|f| f(&b.rings)) {
                continue;
            }
            if observation_towers
                .iter()
                .any(|tower| observation_tower_replaces(*tower, source_index, b))
            {
                continue;
            }
            let rings = to_cols(&b.rings);
            let mut source_bounds = [
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ];
            for point in rings.iter().flatten() {
                source_bounds[0] = source_bounds[0].min(point[0]);
                source_bounds[1] = source_bounds[1].max(point[0]);
                source_bounds[2] = source_bounds[2].min(point[1]);
                source_bounds[3] = source_bounds[3].max(point[1]);
            }
            // Keep a catalog source in every cell reached by its attachment.
            // The source footprint itself still owns raster columns and the
            // final geometric clip owns the sign triangles at the seam.
            let padding = proj.radius_cols(crate::building_surface::city_attachment_radius_m(
                b.seed, level,
            ));
            if !source_bounds.iter().all(|bound| bound.is_finite())
                || source_bounds[1] <= -padding[0]
                || source_bounds[0] >= N as f32 + padding[0]
                || source_bounds[3] <= -padding[1]
                || source_bounds[2] >= N as f32 + padding[1]
                || !b.height_m.is_finite()
                || !b.min_height_m.is_finite()
                || b.height_m <= b.min_height_m
            {
                continue;
            }
            let mut footprint = Vec::new();
            fill_polygon(&rings, |x, z| footprint.push((x, z)));
            let metrics = building_metrics(b);
            let composed = &composition[source_index];
            let design = composed.design(b, &pack.buildings, (lat0, lon0), &mut design_cache);
            let classification_height = if b.kind.is_part() {
                b.height_m
            } else {
                b.height_m - b.min_height_m
            };
            let tall_style = composed
                .facade_openings
                .then(|| {
                    crate::regional_architecture::tall_style(
                        design,
                        classification_height,
                        metrics,
                        composed.style_seed,
                    )
                })
                .flatten();
            let style = match design.construction {
                crate::regional_architecture::ConstructionSystem::Industrial => {
                    FacadeStyle::Industrial
                }
                _ if design.facade.is_some_and(|system| !system.residential()) => {
                    FacadeStyle::Office
                }
                _ if tall_style.is_some() => FacadeStyle::Tower,
                _ if design.detail == crate::regional_architecture::DetailFamily::Classical => {
                    FacadeStyle::Townhouse
                }
                _ if classification_height > 14.0 => FacadeStyle::Masonry,
                _ => FacadeStyle::LowRise,
            };
            let (roof_shape, roof_profile_height) =
                crate::regional_architecture::roof_profile(b, design);
            let measured = surf.and_then(|grid| lidar_verdict(b, frame, grid.at));
            let source_stamp = BuildingStamp {
                source_index: Some(source_index),
                parent_index: composed.parent,
                facade_openings: composed.facade_openings,
                min_h: b.min_height_m,
                height: if measured.is_some() {
                    b.height_m
                } else {
                    composed.height
                },
                lidar: measured.is_some(),
                top_geo: measured.unwrap_or(f32::NAN),
                seed: b.seed,
                building_kind: b.kind,
                source_total_height: b.height_m,
                night_eligible: composed.facade_openings
                    && b.kind != BuildingKind::RoofOnly
                    && night_eligible.is_some_and(|eligible| eligible(b.seed)),
                facade_style: style,
                tall_style,
                highrise: design.facade,
                design,
                roof_shape,
                roof_height_m: roof_profile_height,
                facade: FacadeStamp {
                    boundary: false,
                    corner: false,
                    edge: 0,
                    along_m: 0.0,
                    length_m: 0.0,
                    edge_distance_m: f32::INFINITY,
                },
            };
            ov.source_buildings.push(source_stamp);
            for (x, z) in footprint {
                let slot = &mut ov.cols[z][x].building;
                let candidate = source_stamp;
                *slot = Some(match *slot {
                    // A part wins over a whole outline even when it is lower;
                    // otherwise the outline would flatten stepped roofs and
                    // setbacks. Keep the winning part's complete vertical
                    // span: borrowing the outline's ground-level min height
                    // stretches rooftop crowns and elevated parts to the
                    // street. Records of the same kind remain tallest-wins.
                    Some(cur) => match (
                        candidate.building_kind.is_part(),
                        cur.building_kind.is_part(),
                    ) {
                        (true, false) => candidate,
                        (false, true) => cur,
                        _ if candidate.source_total_height > cur.source_total_height => candidate,
                        _ => cur,
                    },
                    None => candidate,
                });
            }
        }
        for tower in &observation_towers {
            let centre = proj.to_cols(tower.centre_en);
            let envelope_m =
                crate::observation_towers::observation_tower_envelope_radius(tower.height_m);
            let envelope_cols = (f64::from(envelope_m) / proj.step_m) as f32;
            let x0 = (centre[0] - envelope_cols).floor().max(0.0) as usize;
            let x1 = (centre[0] + envelope_cols).ceil().min(N as f32 - 1.0) as usize;
            let z0 = (centre[1] - envelope_cols).floor().max(0.0) as usize;
            let z1 = (centre[1] + envelope_cols).ceil().min(N as f32 - 1.0) as usize;
            if x0 > x1 || z0 > z1 {
                continue;
            }
            ov.source_towers.push(*tower);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    // Preserve unrelated neighbouring buildings beneath an
                    // overhanging pod. Source tower parts were removed above,
                    // so every column belonging to the tower itself is free.
                    if ov.cols[z][x].building.is_some() {
                        continue;
                    }
                    let local_x_m = ((x as f32 + 0.5) - centre[0]) * proj.step_m as f32;
                    let local_z_m = ((z as f32 + 0.5) - centre[1]) * proj.step_m as f32;
                    if local_x_m.hypot(local_z_m) > envelope_m + proj.step_m as f32 * 0.5 {
                        continue;
                    }
                    ov.cols[z][x].building = Some(BuildingStamp {
                        source_index: None,
                        parent_index: None,
                        facade_openings: false,
                        design: Default::default(),
                        min_h: 0.0,
                        height: tower.height_m,
                        lidar: false,
                        top_geo: f32::NAN,
                        seed: tower.seed,
                        building_kind: BuildingKind::Building,
                        source_total_height: tower.height_m,
                        night_eligible: false,
                        facade_style: FacadeStyle::Tower,
                        tall_style: None,
                        highrise: None,
                        roof_shape: Some(RoofShape::Flat),
                        roof_height_m: None,
                        facade: FacadeStamp {
                            boundary: false,
                            corner: false,
                            edge: 0,
                            along_m: 0.0,
                            length_m: 1.0,
                            edge_distance_m: envelope_m,
                        },
                    });
                }
            }
        }
    }

    // A mapped line strengthens only an existing DEM break. It never creates
    // height, guesses a high side, or overrides stronger physical owners.
    if level >= CLIFF_SURFACE_MIN_LEVEL {
        apply_cliff_surface(&mut ov, pack, &proj, elev_geo);
    }

    // Physical overlays always win over visual flora. Keeping the clearing
    // centralized makes the verdict independent of source feature order.
    for col in ov.cols.iter_mut().flatten() {
        if col.water
            || col.ocean
            || col.road.is_some()
            || col.building.is_some()
            || matches!(
                col.ground,
                Some(GroundKind::Paved | GroundKind::Urban | GroundKind::Sand)
            )
        {
            col.flora = None;
        }
    }

    // Trees: exact POIs plus deterministic physical-lattice placement.
    if level >= TREES_MIN_LEVEL
        && !(pack.forest.is_empty() && pack.ground.is_empty() && pack.exact_trees.is_empty())
    {
        let mut forest = Box::new([[false; N]; N]);
        for rings in &pack.forest {
            fill_polygon(&to_cols(rings), |x, z| forest[z][x] = true);
        }
        let exact_tree_cols = place_exact_trees(
            frame,
            &mut ov,
            pack,
            &proj,
            elev_geo,
            tree_ground,
            proj.step_m as f32,
            margin,
        );
        place_trees(
            cell,
            frame,
            &mut ov,
            &forest,
            pack,
            &proj,
            &exact_tree_cols,
            elev_geo,
            tree_ground,
            proj.step_m as f32,
        );
    }

    ov
}

/// Per-building LiDAR verdict: sample the gated DSM on a lattice
/// over the building's own footprint; when at least ~55% of the in-polygon
/// points answer (and at least 3 do), the building is massed from LiDAR and
/// the median rooftop height is returned for hole-filling. The lattice
/// lives in the building's EN bbox — ring coordinates invert exactly to
/// absolute lat/lon (same formula `vectors.rs` used to produce them), so
/// every same-level cell that sees this building computes the same verdict.
fn lidar_verdict(
    b: &vearth_data::vectors::Building,
    frame: &ChunkFrame,
    surf_at: &dyn Fn(f64, f64) -> Option<f32>,
) -> Option<f32> {
    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    let (mut e0, mut n0, mut e1, mut n1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in b.rings.iter().flatten() {
        e0 = e0.min(p[0]);
        n0 = n0.min(p[1]);
        e1 = e1.max(p[0]);
        n1 = n1.max(p[1]);
    }
    if e1 <= e0 || n1 <= n0 {
        return None;
    }
    // Even-odd point-in-polygon over the ring set, in EN meters.
    let inside = |e: f32, n: f32| -> bool {
        let mut c = false;
        for ring in b.rings.iter() {
            let m = ring.len();
            for i in 0..m {
                let (a, q) = (ring[i], ring[(i + 1) % m]);
                if (a[1] > n) != (q[1] > n) {
                    let t = (n - a[1]) / (q[1] - a[1]);
                    if e < a[0] + t * (q[0] - a[0]) {
                        c = !c;
                    }
                }
            }
        }
        c
    };
    const K: usize = 6;
    let (mut total, mut tops): (usize, Vec<f32>) = (0, Vec::new());
    for j in 0..K {
        for i in 0..K {
            let e = e0 + (e1 - e0) * (i as f32 + 0.5) / K as f32;
            let n = n0 + (n1 - n0) * (j as f32 + 0.5) / K as f32;
            if !inside(e, n) {
                continue;
            }
            total += 1;
            let lat = lat0 + n as f64 / MEAN_RADIUS;
            let lon = lon0 + e as f64 / (MEAN_RADIUS * lat0.cos().max(1e-6));
            if let Some(s) = surf_at(lat, lon) {
                tops.push(s);
            }
        }
    }
    if total == 0 || tops.len() < 3 || tops.len() * 100 < total * 55 {
        return None;
    }
    tops.sort_by(f32::total_cmp);
    Some(tops[tops.len() / 2])
}

/// Beach gate: a sand band one to a few columns wide only reads as a beach
/// once columns are ~20 m or finer.
const BEACH_MIN_LEVEL: u8 = 14;

/// Sand banding along the ocean edge: low-lying land columns within
/// a few meters of an ocean column read as beach, under any explicit
/// landuse (which wins — a quay stays paved).
fn beach_sand(ov: &mut CellOverlay, elev_geo: &dyn Fn(i32, i32) -> f32, level: u8, step_m: f64) {
    if level < BEACH_MIN_LEVEL {
        return;
    }
    let d = ((8.0 / step_m).round() as i64).clamp(1, 3);
    for z in 0..N as i64 {
        for x in 0..N as i64 {
            let c = &ov.cols[z as usize][x as usize];
            if c.ocean || c.water || c.ground.is_some() || elev_geo(x as i32, z as i32) > 5.0 {
                continue;
            }
            let mut near_ocean = false;
            'scan: for dz in -d..=d {
                for dx in -d..=d {
                    let (nx, nz) = (x + dx, z + dz);
                    if (0..N as i64).contains(&nx)
                        && (0..N as i64).contains(&nz)
                        && ov.cols[nz as usize][nx as usize].ocean
                    {
                        near_ocean = true;
                        break 'scan;
                    }
                }
            }
            if near_ocean {
                ov.cols[z as usize][x as usize].ground = Some(GroundKind::Sand);
            }
        }
    }
}

/// Approximate Euclidean distance to dry land on a small raster mask.
///
/// Distances saturate at a fixed physical radius. A shoreline farther away
/// therefore becomes uniformly "deep" instead of exposing the edge of this
/// local calculation as either a cell seam or an LOD transition.
fn water_shore_factors(
    mask: &[bool],
    ne: usize,
    margin: usize,
    step_m: f32,
    radius_m: f32,
) -> Vec<f32> {
    debug_assert_eq!(mask.len(), ne * ne);
    debug_assert!(margin > 0);
    debug_assert!(step_m > 0.0 && radius_m > 0.0);
    let cap = (radius_m / step_m + 0.5).min(margin as f32 + 0.5);
    let diagonal = std::f32::consts::SQRT_2;
    let mut distance = mask
        .iter()
        .map(|&water| if water { cap } else { 0.0 })
        .collect::<Vec<_>>();

    for z in 0..ne {
        for x in 0..ne {
            let i = z * ne + x;
            if !mask[i] {
                continue;
            }
            let mut d = distance[i];
            if x > 0 {
                d = d.min(distance[i - 1] + 1.0);
            }
            if z > 0 {
                d = d.min(distance[i - ne] + 1.0);
                if x > 0 {
                    d = d.min(distance[i - ne - 1] + diagonal);
                }
                if x + 1 < ne {
                    d = d.min(distance[i - ne + 1] + diagonal);
                }
            }
            distance[i] = d.min(cap);
        }
    }
    for z in (0..ne).rev() {
        for x in (0..ne).rev() {
            let i = z * ne + x;
            if !mask[i] {
                continue;
            }
            let mut d = distance[i];
            if x + 1 < ne {
                d = d.min(distance[i + 1] + 1.0);
            }
            if z + 1 < ne {
                d = d.min(distance[i + ne] + 1.0);
                if x > 0 {
                    d = d.min(distance[i + ne - 1] + diagonal);
                }
                if x + 1 < ne {
                    d = d.min(distance[i + ne + 1] + diagonal);
                }
            }
            distance[i] = d.min(cap);
        }
    }

    distance
        .into_iter()
        .zip(mask)
        .map(|(d, &water)| {
            if water {
                ((d - 0.5) * step_m / radius_m).clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
        .collect()
}

/// Inferred regional roof profiles with a conservative chamfer fallback for
/// small low-rise buildings.

const CLIFF_INFLUENCE_RADIUS_M: f32 = 6.0;
const CLIFF_SUPPORT_SLOPE_RAD: f32 = 0.42;

fn apply_cliff_surface(
    overlay: &mut CellOverlay,
    pack: &FeaturePack,
    projection: &GridProj,
    elevation: &dyn Fn(i32, i32) -> f32,
) {
    let radius_cols = CLIFF_INFLUENCE_RADIUS_M / projection.step_m as f32;
    let mut influenced = [[false; N]; N];
    for cliff in &pack.cliffs {
        let points: Vec<_> = cliff
            .pts
            .iter()
            .map(|&point| projection.to_cols(point))
            .collect();
        stamp_polyline(&points, radius_cols, |x, z| influenced[z][x] = true);
    }
    for z in 0..N {
        for x in 0..N {
            if !influenced[z][x] {
                continue;
            }
            let column = &overlay.cols[z][x];
            if column.water
                || column.ocean
                || column.road.is_some()
                || column.building.is_some()
                || column.flora.is_some()
                || !cliff_ground_eligible(column.ground)
            {
                continue;
            }
            if cliff_supported(elevation, x as i32, z as i32, projection.step_m as f32) {
                overlay.cols[z][x].ground = Some(GroundKind::BareRock);
            }
        }
    }
}

fn cliff_ground_eligible(ground: Option<GroundKind>) -> bool {
    matches!(
        ground,
        None | Some(
            GroundKind::Green
                | GroundKind::Forest
                | GroundKind::BareRock
                | GroundKind::Scrub
                | GroundKind::NaturalGrass
                | GroundKind::RegionalBarren
                | GroundKind::RegionalForest
                | GroundKind::RegionalGrassland
                | GroundKind::RegionalScrub
        )
    )
}

fn cliff_supported(elevation: &dyn Fn(i32, i32) -> f32, x: i32, z: i32, step_m: f32) -> bool {
    let centre = elevation(x, z);
    let neighbours = [
        elevation(x - 1, z),
        elevation(x + 1, z),
        elevation(x, z - 1),
        elevation(x, z + 1),
    ];
    centre.is_finite()
        && neighbours.iter().all(|sample| sample.is_finite())
        && (neighbours
            .iter()
            .map(|sample| (sample - centre).abs())
            .fold(0.0f32, f32::max)
            / step_m)
            .atan()
            >= CLIFF_SUPPORT_SLOPE_RAD
}

/// Point-query counterpart of the ground overlay's authority order. It lets
/// off-cell roots and shared terrain corners retain their source-ground verdict.
pub(crate) fn tree_source_surface(
    pack: &FeaturePack,
    point: [f32; 2],
) -> (Option<GroundKind>, bool) {
    source_surface_with(pack, None, |_, _, rings| point_in_rings(point, rings))
}

#[derive(Clone, Copy)]
enum SurfaceLayer {
    Regional,
    Forest,
    Ground,
    Flora,
}

fn source_surface_with(
    pack: &FeaturePack,
    candidates: Option<&[Vec<usize>; 4]>,
    mut contains: impl FnMut(SurfaceLayer, usize, &vearth_data::vectors::Rings) -> bool,
) -> (Option<GroundKind>, bool) {
    // Candidate lists contain ascending source ordinals. The same evaluator
    // also serves unindexed point queries, retaining every authority tie-break.
    let indices = |layer: usize, len: usize| {
        let values = candidates.map(|layers| layers[layer].as_slice());
        (0..values.map_or(len, <[usize]>::len))
            .map(move |index| values.map_or(index, |values| values[index]))
    };
    let mut regional = None;
    for index in indices(0, pack.regional_landcover.len()) {
        let region = &pack.regional_landcover[index];
        if (pack.regional_landcover_mode == RegionalLandcoverMode::ExistingFarmlandOnly
            && region.kind != LandcoverKind::Farmland)
            || !contains(SurfaceLayer::Regional, index, &region.rings)
        {
            continue;
        }
        let candidate = RegionalLandcoverStamp {
            location_rad: region.location_rad,
            kind: region.kind,
            stable_identity: region.stable_identity,
        };
        if regional.is_none_or(|current| regional_landcover_wins(candidate, current)) {
            regional = Some(candidate);
        }
    }
    let mut cultivated = regional.is_some_and(|region| region.kind == LandcoverKind::Farmland);
    let mut ground = if cultivated {
        Some(GroundKind::Green)
    } else {
        regional.and_then(|region| regional_ground_kind(region.kind))
    };
    if indices(1, pack.forest.len())
        .any(|index| contains(SurfaceLayer::Forest, index, &pack.forest[index]))
    {
        ground = Some(GroundKind::Forest);
        cultivated = false;
    }
    for class in 0..3 {
        for index in indices(2, pack.ground.len()) {
            let (kind, rings) = &pack.ground[index];
            let kind_class = if kind.is_natural_semantic() {
                0
            } else if kind.is_managed_semantic() {
                1
            } else {
                2
            };
            if kind_class != class || !contains(SurfaceLayer::Ground, index, rings) {
                continue;
            }
            if class < 2
                && (ground == Some(GroundKind::Forest)
                    || ground.is_some_and(|current| {
                        if class == 0 {
                            current.is_natural_semantic()
                                && current.natural_precedence() >= kind.natural_precedence()
                        } else {
                            current.is_managed_semantic()
                                && current.managed_precedence() >= kind.managed_precedence()
                        }
                    }))
            {
                continue;
            }
            ground = Some(*kind);
            cultivated = false;
        }
    }
    if !matches!(
        ground,
        Some(GroundKind::Paved | GroundKind::Urban | GroundKind::Sand)
    ) && indices(3, pack.flora.len())
        .any(|index| contains(SurfaceLayer::Flora, index, &pack.flora[index].rings))
    {
        ground = Some(GroundKind::Green);
        cultivated = true;
    }
    (ground, cultivated)
}

/// Point counterpart of mapped terrain ownership, including DEM-supported
/// cliff lines. Corner samples use the same level gate, ground precedence,
/// slope support and minimum resolved influence as column rasterization.
pub(crate) fn terrain_corner_surface(
    surfaces: &SourceSurfaceQuery<'_>,
    projection: &GridProj,
    level: u8,
    point: [f32; 2],
    corner: [i32; 2],
    elevation: &dyn Fn(i32, i32) -> f32,
) -> (Option<GroundKind>, bool) {
    let pack = surfaces.pack();
    let (ground, cultivated) = surfaces.at(point);
    if pack.cliffs.is_empty()
        || level < CLIFF_SURFACE_MIN_LEVEL
        || cultivated
        || !cliff_ground_eligible(ground)
        || !cliff_supported(elevation, corner[0], corner[1], projection.step_m as f32)
    {
        return (ground, cultivated);
    }
    let columns = projection.to_cols(point);
    let line_contains = |points: &[[f32; 2]], radius: f32| {
        points.windows(2).any(|segment| {
            point_segment_distance_squared(
                columns,
                projection.to_cols(segment[0]),
                projection.to_cols(segment[1]),
            ) <= radius * radius
        })
    };
    if !pack.cliffs.iter().any(|cliff| {
        line_contains(
            &cliff.pts,
            (CLIFF_INFLUENCE_RADIUS_M / projection.step_m as f32).max(0.51),
        )
    }) || pack
        .water
        .iter()
        .any(|water| point_in_rings(point, &water.rings))
        || pack
            .buildings
            .iter()
            .any(|building| point_in_rings(point, &building.rings))
        || pack.roads.iter().any(|road| {
            line_contains(
                &road.pts,
                (road.width_m() * 0.5 / projection.step_m as f32).max(0.51),
            )
        })
        || pack.watercourses.iter().any(|watercourse| {
            watercourse_admitted(
                watercourse,
                pack.watercourse_output_mode,
                level,
                projection.max_step_m as f32,
            ) && watercourse.paths.iter().any(|path| {
                line_contains(
                    path,
                    (watercourse.width_m * 0.5 / projection.max_step_m as f32).max(0.51),
                )
            })
        })
    {
        return (ground, cultivated);
    }
    (Some(GroundKind::BareRock), false)
}

/// Rejected neighboring exact roots must not suppress visible inferred trees.
/// Their physical exclusions therefore use the same checks as owned roots.
fn tree_root_terrain_blocked(
    pack: &FeaturePack,
    projection: &GridProj,
    level: u8,
    columns: [f32; 2],
    elevation: &dyn Fn(i32, i32) -> f32,
) -> bool {
    let x = columns[0].floor() as i32;
    let z = columns[1].floor() as i32;
    let centre = [x as f32 + 0.5, z as f32 + 0.5];
    let line_contains = |points: &[[f32; 2]], radius: f32| {
        points.windows(2).any(|segment| {
            point_segment_distance_squared(
                centre,
                projection.to_cols(segment[0]),
                projection.to_cols(segment[1]),
            ) <= radius * radius
        })
    };
    if pack.watercourses.iter().any(|watercourse| {
        watercourse_admitted(
            watercourse,
            pack.watercourse_output_mode,
            level,
            projection.max_step_m as f32,
        ) && watercourse.paths.iter().any(|path| {
            // Include the raster stroke's sub-column primary and bridge cells.
            // The same one-column buffer applies to owner and margin roots.
            line_contains(
                path,
                watercourse.width_m * 0.5 / projection.max_step_m as f32 + 1.0,
            )
        })
    }) {
        return true;
    }
    if level < CLIFF_SURFACE_MIN_LEVEL
        || !pack.cliffs.iter().any(|cliff| {
            line_contains(
                &cliff.pts,
                (CLIFF_INFLUENCE_RADIUS_M / projection.step_m as f32).max(0.51),
            )
        })
    {
        return false;
    }
    let point = projection.to_en(centre);
    // Detailed physical/managed surfaces retain the same precedence as the
    // cliff overlay. An exact street tree is not rejected by a nearby cliff
    // line when its paved substrate does not admit cliff treatment.
    let (ground, cultivated) = tree_source_surface(pack, point);
    if !cliff_ground_eligible(ground) || cultivated {
        return false;
    }
    cliff_supported(elevation, x, z, projection.step_m as f32)
}

// Root admission is geographic and identical for owned and neighboring trees.
// Terrain datums are irrelevant to horizontal landmark ownership.
struct TreeRootAdmission {
    level: u8,
    owners:
        std::collections::HashMap<vearth_core::cell::CellId, Vec<crate::landmarks::ActiveLandmark>>,
}
impl TreeRootAdmission {
    fn new(level: u8) -> Self {
        Self {
            level,
            owners: std::collections::HashMap::new(),
        }
    }
    fn allows(&mut self, lat: f64, lon: f64) -> bool {
        if crate::great_wall::excludes_tree(lat, lon) {
            return false;
        }
        let root = vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
        if !root.is_finite() {
            return false;
        }
        let owner = vearth_core::cell::CellId::from_dir(root, self.level);
        let active = self.owners.entry(owner).or_insert_with(|| {
            let frame = ChunkFrame::of(vearth_core::cell::ChunkAddr { cell: owner, k: 0 });
            crate::landmarks::active_for(owner, &frame, &|_, _| 0.0)
        });
        !active
            .iter()
            .any(|landmark| landmark.lm.suppress.trees && landmark.excludes_source_at(root, 0.0))
    }
}

/// Stable physical placement on an L20 lattice, independent of the rendered
/// terrain level. Forest candidates are denser than managed green spaces;
/// source exclusions and ecological limits still determine admission.
fn place_trees(
    cell: CellId,
    frame: &ChunkFrame,
    ov: &mut CellOverlay,
    forest: &[[bool; N]; N],
    pack: &FeaturePack,
    projection: &GridProj,
    exact_tree_cols: &[[f32; 2]],
    elev_geo: &dyn Fn(i32, i32) -> f32,
    tree_ground: Option<&dyn Fn(f64, f64) -> Option<f32>>,
    col_step_m: f32,
) {
    // Optional landmark pads own their source vegetation. They do not add
    // new candidates to the ordinary world placement lattice.
    if cell.level > vearth_core::cell::MAX_LEVEL {
        return;
    }
    let mut admission = TreeRootAdmission::new(cell.level);
    let shift = (20 - cell.level) as u32;
    let n_sub = 1i64 << shift; // placement cells per terrain-cell edge
    let cols_per_sub = N as f32 / n_sub as f32; // ≥ 4 at L17
    let i0 = (cell.i as i64) << shift;
    let j0 = (cell.j as i64) << shift;
    for sj in 0..n_sub {
        for si in 0..n_sub {
            let (gi, gj) = (i0 + si, j0 + sj);
            if gi < 0 || gj < 0 || gi >= (1i64 << 20) || gj >= (1i64 << 20) {
                continue;
            }
            let seed = splitmix(
                (cell.face.0 as u64) << 58
                    ^ (gi as u64) << 29
                    ^ (gj as u64)
                    ^ (PROCEDURAL_SEED_VERSION as u64) << 47,
            );
            // Position: uniform offset within the L20 cell, expressed on the
            // current cell's column grid (grid z runs along −v).
            let cx = (si as f32 + hash01(seed ^ 1)) * cols_per_sub;
            let cz = ((n_sub - 1 - sj) as f32 + (1.0 - hash01(seed ^ 2))) * cols_per_sub;
            let (xc, zc) = (
                (cx.floor() as i64).clamp(0, N as i64 - 1) as usize,
                (cz.floor() as i64).clamp(0, N as i64 - 1) as usize,
            );
            let point = projection.to_en([cx, cz]);
            if tree_root_terrain_blocked(pack, projection, cell.level, [cx, cz], elev_geo) {
                continue;
            }
            let col = &ov.cols[zc][xc];
            let cultivated = col.flora.is_some();
            let forest_root = forest[zc][xc];
            let ground = col.ground;
            let natural = forest_root && !cultivated && ground != Some(GroundKind::BareRock);
            let managed = !natural && !cultivated && ground == Some(GroundKind::Green);
            let scrub = !natural && !cultivated && ground == Some(GroundKind::Scrub);
            let density = if natural {
                0.82
            } else if scrub {
                0.04
            } else if managed {
                0.025
            } else {
                0.0
            };
            if hash01(seed) >= density
                || col.water
                || col.ocean
                || col.building.is_some()
                || col.road.is_some()
                || exact_tree_cols
                    .iter()
                    .any(|point| (point[0] - cx).hypot(point[1] - cz) * col_step_m < 6.0)
            {
                continue;
            }

            let (root_lat, root_lon) = {
                let root_cell = CellId {
                    face: cell.face,
                    level: 20,
                    i: gi as u32,
                    j: gj as u32,
                };
                let (u, v) =
                    root_cell.uv_at(f64::from(hash01(seed ^ 1)), f64::from(hash01(seed ^ 2)));
                let direction = vearth_core::cubesphere::face_uv_to_dir(cell.face, u, v);
                let (lat, lon, _) = vearth_core::ellipsoid::ecef_to_geodetic(
                    vearth_core::ellipsoid::surface_point_along(direction),
                );
                (lat, lon)
            };
            let (lat, lon) = (root_lat, root_lon);
            let root_x = cx.floor() as i32;
            let root_z = cz.floor() as i32;
            let elevation_m = elev_geo(root_x, root_z);
            if !elevation_m.is_finite() {
                continue;
            }
            // Quantization suppresses harmless parent/child projection ulps
            // before a categorical vegetation profile is selected.
            // The source atlas is one-degree data. Quarter-degree snapping is
            // well below its information content and guarantees that the same
            // canonical L20 tree candidate receives the same categorical
            // profile when visited through a parent or any child cell.
            let climate_lat = ((lat.to_degrees() * 4.0).round() / 4.0).to_radians();
            let climate_lon = ((lon.to_degrees() * 4.0).round() / 4.0).to_radians();
            let climate = crate::climate_at(climate_lat, climate_lon, elevation_m);
            let env = TreeEnvironment {
                lat_deg: lat.to_degrees() as f32,
                lon_deg: lon.to_degrees() as f32,
                temp_c: climate.temp_c,
                arid: climate.arid,
                elevation_m,
                near_water: tree_near_water(pack, point),
            };
            if natural && hash01(seed ^ 0x4543_4f4c_4f47_595f) >= natural_tree_density_factor(env) {
                continue;
            }
            // Managed groves share a local identity. Natural woodland uses
            // canonical root coordinates and a continuous geographic habitat
            // field, independent of this grid and the rendered terrain level.
            let stand_seed = splitmix(
                (cell.face.0 as u64) << 58
                    ^ ((gi as u64) >> 5) << 29
                    ^ ((gj as u64) >> 5)
                    ^ 0x5354_414e_4453_4545,
            );
            let profile = select_profile(env, managed, seed, stand_seed);
            let height_m = tree_height(profile, env, managed, seed);
            if !admission.allows(root_lat, root_lon) {
                continue;
            }
            let tree = TreeInstance {
                profile,
                seed,
                lat_rad: root_lat,
                lon_rad: root_lon,
                elevation_m: source_tree_elevation(frame, pack, tree_ground, root_lat, root_lon)
                    .unwrap_or(elevation_m),
                height_m,
                yaw_rad: hash01(seed ^ 0x5941_575f_5452_4545) * std::f32::consts::TAU,
                cell_x: cx,
                cell_z: cz,
            };
            ov.tree_instances.push(tree);
        }
    }
}

fn place_exact_trees(
    frame: &ChunkFrame,
    ov: &mut CellOverlay,
    pack: &FeaturePack,
    projection: &GridProj,
    elev_geo: &dyn Fn(i32, i32) -> f32,
    tree_ground: Option<&dyn Fn(f64, f64) -> Option<f32>>,
    _col_step_m: f32,
    elevation_margin: usize,
) -> Vec<[f32; 2]> {
    let mut admission = TreeRootAdmission::new(frame.addr.cell.level);
    let mut seen = HashSet::new();
    let mut admitted = Vec::with_capacity(pack.exact_trees.len());
    for tree in &pack.exact_trees {
        if !seen.insert(tree.identity) {
            continue;
        }
        let [cx, cz] = projection.to_cols(tree.point);
        if !cx.is_finite() || !cz.is_finite() {
            continue;
        }
        let interior = cx >= 0.0 && cz >= 0.0 && cx < N as f32 && cz < N as f32;
        // Neighboring mapped roots suppress inferred trees within six metres.
        // Resolve the same admission checks as their owner before adding that
        // exclusion. Paved and urban ground remain eligible for street trees.
        if exact_tree_root_blocked(pack, tree.point)
            || tree_root_terrain_blocked(
                pack,
                projection,
                frame.addr.cell.level,
                [cx, cz],
                elev_geo,
            )
        {
            continue;
        }
        if interior {
            let col = &ov.cols[cz.floor() as usize][cx.floor() as usize];
            if col.water
                || col.ocean
                || col.building.is_some()
                || col.road.is_some()
                || col.ground.is_some_and(exact_tree_ground_excluded)
            {
                continue;
            }
        }
        let root_x = cx.floor() as i32;
        let root_z = cz.floor() as i32;
        let margin = elevation_margin as i32;
        if root_x < -margin
            || root_z < -margin
            || root_x >= N as i32 + margin
            || root_z >= N as i32 + margin
        {
            continue;
        }
        let elevation_m = elev_geo(root_x, root_z);
        if !elevation_m.is_finite() {
            continue;
        }
        // Source EN is relative to the query centre. Inverting that source
        // projection avoids owner/neighbor drift through a second tangent grid.
        let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
        let lat = lat0 + f64::from(tree.point[1]) / MEAN_RADIUS;
        let lon =
            (lon0 + f64::from(tree.point[0]) / (MEAN_RADIUS * lat0.cos()) + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
        if !admission.allows(lat, lon) {
            continue;
        }
        admitted.push([cx, cz]);
        if !interior {
            continue;
        }
        let climate_lat = ((lat.to_degrees() * 4.0).round() / 4.0).to_radians();
        let climate_lon = ((lon.to_degrees() * 4.0).round() / 4.0).to_radians();
        let climate = crate::climate_at(climate_lat, climate_lon, elevation_m);
        let env = TreeEnvironment {
            lat_deg: lat.to_degrees() as f32,
            lon_deg: lon.to_degrees() as f32,
            temp_c: climate.temp_c,
            arid: climate.arid,
            elevation_m,
            near_water: false,
        };
        let profile = select_profile(env, true, tree.seed, tree.seed);
        let height_m = tree_height(profile, env, true, tree.seed);
        let instance = TreeInstance {
            profile,
            seed: tree.seed,
            lat_rad: lat,
            lon_rad: lon,
            elevation_m: source_tree_elevation(frame, pack, tree_ground, lat, lon)
                .unwrap_or(elevation_m),
            height_m,
            yaw_rad: hash01(tree.seed ^ 0x5941_575f_5452_4545) * std::f32::consts::TAU,
            cell_x: cx,
            cell_z: cz,
        };
        ov.tree_instances.push(instance);
    }
    admitted
}

fn source_tree_elevation(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    sample: Option<&dyn Fn(f64, f64) -> Option<f32>>,
    lat: f64,
    lon: f64,
) -> Option<f32> {
    let sample = sample?;
    let point = vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
    let owner = CellId::from_dir(point, frame.addr.cell.level);
    let owner_frame = ChunkFrame::of(vearth_core::cell::ChunkAddr { cell: owner, k: 0 });
    let local = owner_frame.ecef_to_local(point);
    let centre = [
        local.x.floor().clamp(0.0, N as f64 - 1.0) as f32 + 0.5,
        local.z.floor().clamp(0.0, N as f64 - 1.0) as f32 + 0.5,
    ];
    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    let projection = GridProj::of(&owner_frame, lat0, lon0);
    let centre_en = projection.to_en(centre);
    let (ground, _) = tree_source_surface(pack, centre_en);
    if ground.is_some_and(|ground| {
        matches!(
            ground,
            GroundKind::Paved
                | GroundKind::Urban
                | GroundKind::RailwayArea
                | GroundKind::RunwayArea
                | GroundKind::RegionalUrban
        )
    }) || pack
        .water
        .iter()
        .any(|water| point_in_rings(centre_en, &water.rings))
        || pack
            .buildings
            .iter()
            .any(|building| point_in_rings(centre_en, &building.rings))
    {
        return None;
    }
    // Use the canonical owner's column guard across projections, covering
    // the actual grade shoulder plus one column for raster boundary drift.
    if pack.roads.iter().any(|road| {
        let width = f64::from(road.width_m());
        let reach = width * 0.5 + (projection.step_m * 1.8).max(width * 0.4);
        let reach_cols = (reach / projection.step_m + 1.0) as f32;
        road.pts.windows(2).any(|segment| {
            point_segment_distance_squared(
                centre,
                projection.to_cols(segment[0]),
                projection.to_cols(segment[1]),
            ) <= reach_cols * reach_cols
        })
    }) {
        return None;
    }
    sample(lat, lon).filter(|height| height.is_finite())
}

fn exact_tree_root_blocked(pack: &FeaturePack, point: [f32; 2]) -> bool {
    if pack
        .water
        .iter()
        .any(|water| point_in_rings(point, &water.rings))
        || pack
            .buildings
            .iter()
            .any(|building| point_in_rings(point, &building.rings))
    {
        return true;
    }
    if pack
        .ground
        .iter()
        .any(|(kind, rings)| exact_tree_ground_excluded(*kind) && point_in_rings(point, rings))
        || (pack.regional_landcover_mode == RegionalLandcoverMode::SelectiveRegionalTerrain
            && pack.regional_landcover.iter().any(|region| {
                matches!(region.kind, LandcoverKind::Barren | LandcoverKind::Glacier)
                    && point_in_rings(point, &region.rings)
            }))
    {
        return true;
    }
    pack.roads.iter().any(|road| {
        let radius2 = (road.width_m() * 0.5).powi(2);
        road.pts
            .windows(2)
            .any(|segment| point_segment_distance_squared(point, segment[0], segment[1]) <= radius2)
    })
}

const fn exact_tree_ground_excluded(kind: GroundKind) -> bool {
    matches!(
        kind,
        GroundKind::BareRock
            | GroundKind::Glacier
            | GroundKind::RunwayArea
            | GroundKind::RegionalBarren
            | GroundKind::RegionalGlacier
    )
}

fn point_segment_distance_squared(point: [f32; 2], start: [f32; 2], end: [f32; 2]) -> f32 {
    let delta = [end[0] - start[0], end[1] - start[1]];
    let length2 = delta[0] * delta[0] + delta[1] * delta[1];
    if length2 <= f32::EPSILON {
        return (point[0] - start[0]).powi(2) + (point[1] - start[1]).powi(2);
    }
    let t = (((point[0] - start[0]) * delta[0] + (point[1] - start[1]) * delta[1]) / length2)
        .clamp(0.0, 1.0);
    let nearest = [start[0] + delta[0] * t, start[1] + delta[1] * t];
    (point[0] - nearest[0]).powi(2) + (point[1] - nearest[1]).powi(2)
}

fn tree_near_water(pack: &FeaturePack, point: [f32; 2]) -> bool {
    // Physical source context stays independent of the terrain raster, which
    // can hide a thin river at coarser levels.
    pack.water.iter().any(|water| {
        (water.is_ocean() || water_contributes_ecology(water.kind, pack.water_polygon_mode))
            && (point_in_rings(point, &water.rings)
                || water.rings.iter().any(|ring| {
                    ring.iter()
                        .zip(ring.iter().cycle().skip(1))
                        .any(|(&a, &b)| point_segment_distance_squared(point, a, b) <= 36.0)
                }))
    }) || (pack.watercourse_output_mode == WatercourseOutputMode::RiversAndCanals
        && pack.watercourses.iter().any(|watercourse| {
            matches!(
                watercourse.kind,
                WatercourseKind::River | WatercourseKind::Canal
            ) && !watercourse.intermittent
                && !watercourse.is_bridge
                && watercourse.paths.iter().any(|path| {
                    path.windows(2).any(|segment| {
                        point_segment_distance_squared(point, segment[0], segment[1])
                            <= (6.0 + watercourse.width_m * 0.5).powi(2)
                    })
                })
        }))
}

#[allow(clippy::too_many_arguments)]
fn tree_height(profile_id: TreeProfileId, env: TreeEnvironment, managed: bool, seed: u64) -> f32 {
    let profile = tree_profile(profile_id);
    let size_t = hash01(seed ^ 3);
    let stress = (1.0 - env.arid * 0.16).clamp(0.76, 1.0);
    let managed_scale = if managed { 0.88 } else { 1.0 };
    (profile.height_m[0] + (profile.height_m[1] - profile.height_m[0]) * size_t)
        * stress
        * managed_scale
}

/// One reusable, object-local near-field tree mesh. Vertex positions are in
/// metres around a ground-centred origin, ready for species-wide sharing and
/// cheap per-tree model transforms in the renderer.
pub struct NearTreeMesh {
    pub mesh: MeshBuffers,
    pub height_m: f32,
    pub radius_m: f32,
}

/// Build the production surface tree for a nominal ground-detail pitch.
/// The pitch selects a geometric LOD; it does not voxelize the tree. Staging
/// and production share the same canonical crown and branch layout.
pub fn build_tree_preview_mesh(profile: TreeProfileId, sample_pitch_m: f32) -> NearTreeMesh {
    let lod = if sample_pitch_m <= 0.25 {
        0
    } else if sample_pitch_m <= 1.0 {
        1
    } else {
        2
    };
    crate::tree_mesh::build_tree_mesh(profile, lod)
}

/// Build the canonical high-detail mesh for a species. This is intentionally
/// separate from terrain meshing: callers cache one result per visible
/// species and instance it for every nearby tree.
pub fn build_near_tree_mesh(profile: TreeProfileId) -> NearTreeMesh {
    crate::tree_mesh::build_tree_mesh(profile, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::cell::ChunkAddr;
    use vearth_core::cubesphere::Face;
    use vearth_data::vectors::{
        Building, CliffLine, FloraRegion, RegionalLandcoverRegion, Road, Water, Watercourse,
    };

    #[test]
    fn procedural_tree_sampling_visits_only_owned_roots() {
        let (cell_a, frame_a) = test_cell(19);
        let cell_b = CellId {
            i: cell_a.i + 1,
            ..cell_a
        };
        let frame_b = ChunkFrame::of(ChunkAddr { cell: cell_b, k: 0 });
        let mut pack = FeaturePack::empty();
        pack.forest.push(square(400.0));
        let seen_a = std::cell::RefCell::new(HashSet::new());
        let seen_b = std::cell::RefCell::new(HashSet::new());
        let sample_a = |lat: f64, lon: f64| {
            seen_a.borrow_mut().insert((lat.to_bits(), lon.to_bits()));
            Some(100.0)
        };
        let sample_b = |lat: f64, lon: f64| {
            seen_b.borrow_mut().insert((lat.to_bits(), lon.to_bits()));
            Some(100.0)
        };
        let a = build_overlay_with_tree_ground(
            cell_a,
            &frame_a,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
            Some(&sample_a),
        );
        let b = build_overlay_with_tree_ground(
            cell_b,
            &frame_b,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
            Some(&sample_b),
        );
        assert!(!a.tree_instances.is_empty() && !b.tree_instances.is_empty());
        assert_eq!(seen_a.borrow().len(), a.tree_instances.len());
        assert_eq!(seen_b.borrow().len(), b.tree_instances.len());
        assert!(seen_a.borrow().is_disjoint(&seen_b.borrow()));
        assert!(a.tree_instances.iter().all(|tree| {
            seen_a
                .borrow()
                .contains(&(tree.lat_rad.to_bits(), tree.lon_rad.to_bits()))
        }));
        assert!(b.tree_instances.iter().all(|tree| {
            seen_b
                .borrow()
                .contains(&(tree.lat_rad.to_bits(), tree.lon_rad.to_bits()))
        }));
        assert!(
            a.tree_instances.iter().all(|tree_a| {
                b.tree_instances
                    .iter()
                    .all(|tree_b| tree_a.seed != tree_b.seed)
            }),
            "adjacent cells duplicated descriptor ownership"
        );
    }

    #[test]
    fn source_tree_ground_guard_protects_the_same_road_shoulder_across_projections() {
        let (cell, owner) = test_cell(19);
        let neighbor = ChunkFrame::of(ChunkAddr {
            cell: CellId {
                i: cell.i + 1,
                ..cell
            },
            k: 0,
        });
        let (lat, lon) = owner.column_latlon(31.75 / N as f64, 14.2 / N as f64);
        let called = std::cell::Cell::new(0);
        let sample = |_, _| {
            called.set(called.get() + 1);
            Some(999.0)
        };
        for visitor in [owner, neighbor] {
            let (lat0, lon0) = visitor.column_latlon(0.5, 0.5);
            let projection = GridProj::of(&owner, lat0, lon0);
            let mut pack = FeaturePack::empty();
            pack.roads.push(Road {
                profile: Default::default(),
                pts: vec![
                    projection.to_en([31.0, 0.0]),
                    projection.to_en([31.0, 32.0]),
                ],
                class: RoadClass::Minor,
                is_bridge: false,
            });
            assert!(source_tree_elevation(&visitor, &pack, Some(&sample), lat, lon).is_none());
            pack.roads.clear();
            assert_eq!(
                source_tree_elevation(&visitor, &pack, Some(&sample), lat, lon),
                Some(999.0)
            );
        }
        assert_eq!(
            called.get(),
            2,
            "protected roots queried the raw terrain sampler"
        );
    }

    #[test]
    fn production_tree_preview_ladder_has_bounded_surface_geometry() {
        let near = build_near_tree_mesh(TreeProfileId::EnglishOak);
        let middle = build_tree_preview_mesh(TreeProfileId::EnglishOak, 0.6);
        let distant = build_tree_preview_mesh(TreeProfileId::EnglishOak, 2.4);
        assert!(near.mesh.indices.len() > middle.mesh.indices.len());
        assert!(middle.mesh.indices.len() > distant.mesh.indices.len());
        for (tree, budget) in [(&near, 3000), (&middle, 1000), (&distant, 300)] {
            assert!(tree.mesh.indices.len() / 3 <= budget);
            assert!((tree.height_m / near.height_m - 1.0).abs() < 0.12);
            assert!((tree.radius_m / near.radius_m - 1.0).abs() < 0.17);
            assert!(tree.mesh.vertices.iter().all(|vertex| {
                vearth_core::mesh::decode_surface_normal(vertex.face_ao).is_some()
                    && vertex.pos.iter().all(|value| value.is_finite())
            }));
        }
    }

    fn test_cell(level: u8) -> (CellId, ChunkFrame) {
        let n = 1u32 << level;
        let cell = CellId {
            face: Face(0),
            level,
            i: n / 2,
            j: n / 2,
        };
        (cell, ChunkFrame::of(ChunkAddr { cell, k: 0 }))
    }

    /// A square in EN meters centered on the cell anchor.
    fn square(half_m: f32) -> vearth_data::vectors::Rings {
        vec![vec![
            [-half_m, -half_m],
            [half_m, -half_m],
            [half_m, half_m],
            [-half_m, half_m],
        ]]
    }

    fn typed_water(kind: WaterRegionKind, identity: u64, half_m: f32) -> Water {
        Water {
            rings: square(half_m),
            kind,
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(identity),
            intermittent: false,
            alkaline: false,
        }
    }

    fn en_from_cols(frame: &ChunkFrame, columns: [f32; 2]) -> [f32; 2] {
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = GridProj::of(frame, lat, lon);
        let fx = f64::from(columns[0]) / N as f64;
        let fz = f64::from(columns[1]) / N as f64;
        let determinant =
            projection.inv[0] * projection.inv[3] - projection.inv[1] * projection.inv[2];
        let de = (projection.inv[3] * fx - projection.inv[1] * fz) / determinant;
        let dn = (-projection.inv[2] * fx + projection.inv[0] * fz) / determinant;
        [
            (projection.origin_e + de) as f32,
            (projection.origin_n + dn) as f32,
        ]
    }

    fn en_from_latlon(frame: &ChunkFrame, lat: f64, lon: f64) -> [f32; 2] {
        let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
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

    fn typed_watercourse(
        frame: &ChunkFrame,
        kind: WatercourseKind,
        identity: u64,
        columns: &[[f32; 2]],
    ) -> Watercourse {
        Watercourse {
            paths: vec![
                columns
                    .iter()
                    .map(|&point| en_from_cols(frame, point))
                    .collect(),
            ],
            kind,
            width_m: kind.fallback_width_m(),
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(identity),
            style_identity: identity,
            source_tile: (15, 0, 0),
            authority_zoom: 15,
            source_feature_index: 0,
            source_min_zoom: None,
            fragment_count: 1,
            intermittent: false,
            is_bridge: false,
        }
    }

    fn geodetic_watercourse(
        frame: &ChunkFrame,
        kind: WatercourseKind,
        identity: u64,
        points: &[(f64, f64)],
    ) -> Watercourse {
        let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
        let paths = vec![
            points
                .iter()
                .map(|&(lat, lon)| {
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
                })
                .collect(),
        ];
        Watercourse {
            paths,
            kind,
            width_m: kind.fallback_width_m(),
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(identity),
            style_identity: identity,
            source_tile: (15, 0, 0),
            authority_zoom: 15,
            source_feature_index: 0,
            source_min_zoom: None,
            fragment_count: 1,
            intermittent: false,
            is_bridge: false,
        }
    }

    fn watercourse_overlay_from_geodetic(
        cell: CellId,
        frame: &ChunkFrame,
        kind: WatercourseKind,
        points: &[(f64, f64)],
    ) -> CellOverlay {
        let mut pack = FeaturePack::empty();
        pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
        pack.watercourses
            .push(geodetic_watercourse(frame, kind, 1, points));
        build_overlay(
            cell,
            frame,
            &pack,
            &|_, _| 0.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        )
    }

    #[test]
    fn building_rasterizes_centered() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        // 6×6 m building at the anchor → ~10×10 columns at 0.6 m voxels.
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(3.0),
            height_m: 21.0,
            min_height_m: 0.0,
            seed: 42,
            kind: BuildingKind::Building,
        });
        let ov = build_overlay(cell, &frame, &pack, &|_, _| 100.0, 0, None, None, None);
        let covered = ov
            .cols
            .iter()
            .flatten()
            .filter(|c| c.building.is_some())
            .count();
        // ~5 columns of 0.6 m per 3 m half-width → footprint ~10² = 100.
        assert!((60..160).contains(&covered), "covered {covered}");
        // Center column definitely covered, with the right height.
        let mid = ov.cols[N / 2][N / 2].building.expect("center covered");
        assert_eq!(mid.source_total_height, 21.0);
        assert!(mid.height <= 21.01);
        assert_eq!(mid.facade_style, FacadeStyle::Masonry);
        assert!(!mid.facade.boundary, "footprint center is not a facade");
        assert_eq!(ov.source_buildings.len(), 1);
        assert!(
            ov.cols
                .iter()
                .flatten()
                .filter_map(|column| column.building)
                .all(|building| !building.facade.boundary)
        );
    }

    #[test]
    fn building_admission_preserves_column_coverage_and_courtyards() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        let building = |rings, seed| Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings,
            height_m: 21.0,
            min_height_m: 0.0,
            seed,
            kind: BuildingKind::Building,
        };
        let far_rings = square(3.0)
            .into_iter()
            .map(|ring| ring.into_iter().map(|[e, n]| [e + 1000.0, n]).collect())
            .collect();
        pack.buildings.push(building(far_rings, 1));
        // Bounds overlap, but the even-odd hole contains every cell column.
        let mut courtyard = square(100.0);
        courtyard.extend(square(60.0));
        pack.buildings.push(building(courtyard, 2));
        let overlay = |pack: &FeaturePack| {
            build_overlay(cell, &frame, pack, &|_, _| 100.0, 0, None, None, None)
        };
        let irrelevant = overlay(&pack);
        assert!(
            irrelevant
                .cols
                .iter()
                .flatten()
                .all(|c| c.building.is_none())
        );

        pack.buildings.push(building(square(3.0), 3));
        let relevant = overlay(&pack);
        let stamps: Vec<_> = relevant
            .cols
            .iter()
            .flatten()
            .filter_map(|c| c.building)
            .collect();
        assert!(!stamps.is_empty());
        assert!(stamps.iter().all(|stamp| stamp.seed == 3));
        assert!(
            stamps
                .iter()
                .all(|stamp| stamp.top_geo.is_nan() && stamp.source_total_height == 21.0)
        );
    }

    #[test]
    fn observation_poi_requires_compact_tall_geometry_and_yields_special_stamp() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(5.0),
            height_m: 180.0,
            min_height_m: 0.0,
            seed: 77,
            kind: BuildingKind::Building,
        });
        pack.observation_hints
            .push(vearth_data::vectors::ObservationHint {
                point: [0.0, 0.0],
                kind: ObservationHintKind::Viewpoint,
                seed: 77,
            });
        let specs = observation_tower_specs(&pack, None);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].height_m, 180.0);

        let ov = build_overlay(cell, &frame, &pack, &|_, _| 10.0, 0, None, None, None);
        assert_eq!(ov.source_towers.len(), 1);
        assert_eq!(ov.source_towers[0].height_m, 180.0);
        let tower_columns: Vec<_> = ov
            .cols
            .iter()
            .flatten()
            .filter_map(|column| column.building)
            .filter(|building| building.seed == ov.source_towers[0].seed)
            .collect();
        assert!(!tower_columns.is_empty());
        assert!(
            tower_columns
                .iter()
                .all(|building| building.height == 180.0)
        );

        let suppressed = |_: &vearth_data::vectors::Rings| true;
        assert!(observation_tower_specs(&pack, Some(&suppressed)).is_empty());

        pack.buildings[0].rings = square(20.0);
        assert!(observation_tower_specs(&pack, None).is_empty());
    }

    #[test]
    fn observation_tower_shared_art_seed_keeps_remote_source_components() {
        let (cell, frame) = test_cell(14);
        let source = |east: f32, height_m| Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(5.0)
                .into_iter()
                .map(|ring| ring.into_iter().map(|[e, n]| [e + east, n]).collect())
                .collect(),
            height_m,
            min_height_m: 0.0,
            seed: 77,
            kind: BuildingKind::Building,
        };
        for reverse in [false, true] {
            for hint_seed in [77, 999] {
                let mut pack = FeaturePack::empty();
                pack.buildings = vec![source(0.0, 180.0), source(100.0, 220.0)];
                if reverse {
                    pack.buildings.reverse();
                }
                let local_index = usize::from(reverse);
                let remote_index = 1 - local_index;
                pack.observation_hints
                    .push(vearth_data::vectors::ObservationHint {
                        point: [0.0, 0.0],
                        kind: ObservationHintKind::Viewpoint,
                        seed: hint_seed,
                    });
                let specs = observation_tower_specs(&pack, None);
                assert_eq!(specs.len(), 1);
                assert_eq!(specs[0].source_index, local_index);
                assert_eq!(specs[0].height_m, 180.0);
                assert!(!observation_tower_replaces(
                    specs[0],
                    remote_index,
                    &pack.buildings[remote_index]
                ));
                let overlay = build_overlay(cell, &frame, &pack, &|_, _| 10.0, 0, None, None, None);
                assert_eq!(overlay.source_towers.len(), 1);
                assert_eq!(overlay.source_towers[0].centre_en, [0.0, 0.0]);
                assert_eq!(overlay.source_buildings.len(), 1);
                assert_eq!(overlay.source_buildings[0].source_index, Some(remote_index));
                assert_eq!(overlay.source_buildings[0].height, 220.0);

                pack.observation_hints
                    .push(vearth_data::vectors::ObservationHint {
                        point: [100.0, 0.0],
                        kind: ObservationHintKind::Viewpoint,
                        seed: hint_seed,
                    });
                let both = observation_tower_specs(&pack, None);
                assert_eq!(both.len(), 2);
                assert_ne!(both[0].source_index, both[1].source_index);
            }
        }
    }

    #[test]
    fn observation_tower_replacement_keeps_exact_anchor_and_geometric_parent() {
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            // A narrow notch excludes the bounds center but the hint remains
            // inside the selected footprint.
            rings: vec![vec![
                [-5.0, -5.0],
                [5.0, -5.0],
                [5.0, 5.0],
                [1.0, 5.0],
                [1.0, -0.5],
                [-1.0, -0.5],
                [-1.0, 5.0],
                [-5.0, 5.0],
            ]],
            height_m: 180.0,
            min_height_m: 0.0,
            seed: 77,
            kind: BuildingKind::Building,
        });
        pack.observation_hints
            .push(vearth_data::vectors::ObservationHint {
                point: [-3.0, 0.0],
                kind: ObservationHintKind::Viewpoint,
                seed: 77,
            });
        let specs = observation_tower_specs(&pack, None);
        assert_eq!(specs.len(), 1);
        let spec = specs[0];
        assert!(!point_in_rings(spec.centre_en, &pack.buildings[0].rings));
        assert!(observation_tower_replaces(spec, 0, &pack.buildings[0]));

        let parent = Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(8.0),
            height_m: 20.0,
            min_height_m: 0.0,
            seed: 999,
            kind: BuildingKind::Building,
        };
        assert!(observation_tower_replaces(spec, 1, &parent));

        // Identity alone does not classify an unrelated same-seed footprint.
        pack.observation_hints[0].point = [100.0, 0.0];
        assert!(observation_tower_specs(&pack, None).is_empty());
    }

    #[test]
    fn building_parts_override_taller_outline_only_where_they_overlap() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(8.0),
            height_m: 60.0,
            min_height_m: 0.0,
            seed: 100,
            kind: BuildingKind::Building,
        });
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(3.0),
            height_m: 52.0,
            min_height_m: 40.0,
            seed: 101,
            kind: BuildingKind::Part,
        });

        let overlay = build_overlay(cell, &frame, &pack, &|_, _| 0.0, 0, None, None, None);
        let center = overlay.cols[N / 2][N / 2]
            .building
            .expect("part covers center");
        assert_eq!(center.building_kind, BuildingKind::Part);
        assert_eq!(center.min_h, 40.0);
        assert_eq!(center.source_total_height, 52.0);
        assert!(center.height <= 52.01);
        assert!(
            overlay
                .cols
                .iter()
                .flatten()
                .filter_map(|c| c.building)
                .any(|building| building.building_kind == BuildingKind::Building
                    && building.source_total_height == 60.0)
        );
    }

    #[test]
    fn source_tower_parts_override_tourist_needle_inference() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        // Campanile's consumed outline is approximately 159 m² and 98.6 m
        // high. A viewpoint and an attraction both satisfy the old heuristic.
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(6.3),
            height_m: 98.6,
            min_height_m: 0.0,
            seed: 35_184_624_726_525,
            kind: BuildingKind::Building,
        });
        for kind in [
            ObservationHintKind::Attraction,
            ObservationHintKind::Viewpoint,
        ] {
            pack.observation_hints
                .push(vearth_data::vectors::ObservationHint {
                    point: [0.0, 0.0],
                    kind,
                    seed: 35_184_624_726_525,
                });
        }
        assert_eq!(observation_tower_specs(&pack, None).len(), 1);
        let distant = square(2.0)
            .into_iter()
            .map(|ring| ring.into_iter().map(|[e, n]| [e + 30.0, n]).collect())
            .collect();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: distant,
            height_m: 15.0,
            min_height_m: 10.0,
            seed: 2,
            kind: BuildingKind::Part,
        });
        assert_eq!(
            observation_tower_specs(&pack, None).len(),
            1,
            "an unrelated neighbouring part does not disable the fallback"
        );
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(4.0),
            height_m: 98.6,
            min_height_m: 70.0,
            seed: 3,
            kind: BuildingKind::Part,
        });
        assert!(observation_tower_specs(&pack, None).is_empty());
        let overlay = build_overlay(cell, &frame, &pack, &|_, _| 0.0, 0, None, None, None);
        let center = overlay.cols[N / 2][N / 2].building.unwrap();
        assert_eq!(center.seed, 3);
        assert_eq!(center.min_h, 70.0);
        assert_eq!(center.source_total_height, 98.6);
        assert!(overlay.source_towers.is_empty());
    }

    #[test]
    fn complete_roof_only_parts_keep_their_elevated_span_without_night_facades() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(3.0),
            height_m: 11.0,
            min_height_m: 10.0,
            seed: 202,
            kind: BuildingKind::RoofOnly,
        });

        let overlay = build_overlay(
            cell,
            &frame,
            &pack,
            &|_, _| 0.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        let center = overlay.cols[N / 2][N / 2]
            .building
            .expect("roof covers center");
        assert_eq!(center.building_kind, BuildingKind::RoofOnly);
        assert_eq!(center.min_h, 10.0);
        assert_eq!(center.source_total_height, 11.0);
        assert_eq!(center.roof_shape, Some(RoofShape::Flat));
        assert_eq!(center.roof_height_m, None);
        assert!(!center.night_eligible);
    }

    #[test]
    fn regional_roof_profile_stays_within_reported_total_height() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: Some(RoofShape::Gabled),
            use_kind: Default::default(),
            rings: square(6.0),
            height_m: 12.0,
            min_height_m: 0.0,
            seed: 55,
            kind: BuildingKind::Building,
        });
        let overlay = build_overlay(cell, &frame, &pack, &|_, _| 0.0, 0, None, None, None);
        let roofs: Vec<_> = overlay
            .cols
            .iter()
            .flatten()
            .filter_map(|column| column.building)
            .collect();
        assert!(
            roofs
                .iter()
                .any(|building| building.roof_height_m.is_some_and(|rise| rise > 0.5))
        );
        assert!(roofs.iter().all(|building| building.height <= 12.01));
    }

    #[test]
    fn courtyard_wings_keep_physical_roof_pitch_across_visiting_cells_and_lods() {
        let lat = 45.435_f64.to_radians();
        let lon = 12.335_f64.to_radians();
        let mut source = square(50.0);
        source.extend(square(44.0));
        let seed = 52_776_560_900_236;
        for level in [18, 19] {
            for [east, north] in [[47.0, 0.0], [-47.0, 0.0], [0.0, 47.0]] {
                let target_lat = lat + north / MEAN_RADIUS;
                let target_lon = lon + east / (MEAN_RADIUS * lat.cos());
                let cell = CellId::from_dir(
                    vearth_core::ellipsoid::geodetic_to_ecef(target_lat, target_lon, 0.0),
                    level,
                );
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                let mut pack = FeaturePack::empty();
                let rings = source
                    .iter()
                    .map(|ring| {
                        ring.iter()
                            .map(|&[e, n]| {
                                en_from_latlon(
                                    &frame,
                                    lat + f64::from(n) / MEAN_RADIUS,
                                    lon + f64::from(e) / (MEAN_RADIUS * lat.cos()),
                                )
                            })
                            .collect()
                    })
                    .collect();
                pack.buildings.push(Building {
                    location_rad: None,
                    roof_shape: Some(RoofShape::Gabled),
                    use_kind: Default::default(),
                    rings,
                    height_m: 13.0,
                    min_height_m: 0.0,
                    seed,
                    kind: BuildingKind::Building,
                });
                let overlay = build_overlay(cell, &frame, &pack, &|_, _| 0.0, 0, None, None, None);
                let roofs: Vec<_> = overlay
                    .cols
                    .iter()
                    .flatten()
                    .filter_map(|c| c.building)
                    .collect();
                assert!(
                    overlay
                        .source_buildings
                        .iter()
                        .any(|b| b.roof_height_m.is_some_and(|rise| rise > 1.0)),
                    "courtyard roof source profile missing at L{level}"
                );
                assert!(
                    roofs
                        .iter()
                        .all(|b| b.roof_shape == Some(RoofShape::Gabled) && b.height <= 13.001)
                );
                for z in 0..N {
                    for x in 0..N {
                        let (p_lat, p_lon) = frame.column_latlon(
                            (x as f64 + 0.5) / N as f64,
                            (z as f64 + 0.5) / N as f64,
                        );
                        let e = (p_lon - lon) * MEAN_RADIUS * lat.cos();
                        let n = (p_lat - lat) * MEAN_RADIUS;
                        if e.abs() < 43.5 && n.abs() < 43.5 {
                            assert!(
                                overlay.cols[z][x].building.is_none(),
                                "courtyard was filled"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn elevated_roof_profile_preserves_source_bottom_and_top_at_each_lod() {
        for level in [17, 18, 19] {
            let (cell, frame) = test_cell(level);
            let mut pack = FeaturePack::empty();
            pack.buildings.push(Building {
                location_rad: None,
                roof_shape: None,
                use_kind: Default::default(),
                rings: square(6.0),
                height_m: 13.0,
                min_height_m: 10.0,
                seed: 202,
                kind: BuildingKind::RoofOnly,
            });
            let overlay = build_overlay(
                cell,
                &frame,
                &pack,
                &|_, _| 0.0,
                0,
                None,
                None,
                Some(&|_| true),
            );
            let roofs: Vec<_> = overlay
                .cols
                .iter()
                .flatten()
                .filter_map(|c| c.building)
                .collect();
            assert!(!roofs.is_empty());
            assert!(roofs.iter().all(|b| b.min_h == 10.0
                && b.source_total_height == 13.0
                && b.height <= 13.001
                && !b.night_eligible
                && b.building_kind == BuildingKind::RoofOnly));
            assert!(roofs.iter().all(|b| b.height == 13.0));
            assert_eq!(overlay.source_buildings.len(), 1);
            assert_eq!(overlay.source_buildings[0].min_h, 10.0);
        }
        let tower_cap = Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(4.0),
            height_m: 54.0,
            min_height_m: 51.0,
            seed: 3,
            kind: BuildingKind::RoofOnly,
        };
        assert_eq!(
            crate::regional_architecture::roof_profile(&tower_cap, Default::default()),
            (Some(RoofShape::Flat), None)
        );
    }

    #[test]
    fn ndsm_source_verdict_is_independent_of_the_ecology_grid() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(6.0),
            height_m: 8.0, // OSM says a flat 8 m box…
            min_height_m: 0.0,
            seed: 42,
            kind: BuildingKind::Building,
        });
        // Roof authority is sampled at source geometry, without a whole-cell
        // surface raster or a requirement for a roof to cover a grid centre.
        let at = |_: f64, _: f64| Some(118.0f32);
        let surf = SurfaceSampler { at: &at };
        let ov = build_overlay(
            cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            Some(&surf),
            None,
            None,
        );
        let source = &ov.source_buildings[0];
        assert!(source.lidar);
        assert_eq!(source.top_geo, 118.0);
        assert_eq!(source.source_total_height, 8.0);
        for column in ov.cols.iter().flatten().filter_map(|c| c.building) {
            assert_eq!(column.top_geo, source.top_geo);
            assert_eq!(column.height, source.height);
        }
    }

    #[test]
    fn canopy_covered_building_keeps_osm_extrusion() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(6.0),
            height_m: 8.0,
            min_height_m: 0.0,
            seed: 7,
            kind: BuildingKind::Building,
        });
        // The gated sampler rejects everything (building under crowns):
        // the whole building must stay a clean OSM extrusion.
        let at = |_: f64, _: f64| None;
        let surf = SurfaceSampler { at: &at };
        let ov = build_overlay(
            cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            Some(&surf),
            None,
            None,
        );
        let mid = ov.cols[N / 2][N / 2].building.expect("center covered");
        assert!(!mid.lidar, "no verdict → no per-column lidar mixing");
        assert_eq!(mid.source_total_height, 8.0);
    }

    #[test]
    fn road_stamps_a_line() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-20.0, 0.0], [20.0, 0.0]],
            class: RoadClass::Minor,
            is_bridge: false,
        });
        let ov = build_overlay(cell, &frame, &pack, &|_, _| 0.0, 0, None, None, None);
        let road_cols = ov
            .cols
            .iter()
            .flatten()
            .filter(|c| c.road.is_some())
            .count();
        // 6.5 m wide crossing a 19 m cell: ≥ 5 columns wide × 32 long.
        assert!(road_cols > 100, "road cols {road_cols}");
        assert!(
            ov.cols[N / 2][2].road.is_some(),
            "row through center covered"
        );
    }

    #[test]
    fn subcolumn_path_coverage_preserves_the_physical_road_footprint() {
        let mut fractions = Vec::new();
        for level in [11, 14, 17, 20] {
            let (cell, frame) = test_cell(level);
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = GridProj::of(&frame, lat, lon);
            let reach = (projection.step_m * N as f64) as f32;
            let mut pack = FeaturePack::empty();
            pack.roads.push(Road {
                profile: Default::default(),
                pts: vec![[-reach, 0.0], [reach, 0.0]],
                class: RoadClass::Path,
                is_bridge: false,
            });
            let render = |pack: &FeaturePack| {
                build_overlay(cell, &frame, pack, &|_, _| 0.0, 0, None, None, None)
            };
            let overlay = render(&pack);
            let points: Vec<_> = pack.roads[0]
                .pts
                .iter()
                .map(|&point| projection.to_cols(point))
                .collect();
            let width = RoadClass::Path.width_m();
            let mut original_mask = [[false; N]; N];
            stamp_polyline(
                &points,
                (f64::from(width) * 0.5 / projection.step_m) as f32,
                |x, z| original_mask[z][x] = true,
            );
            let minimum_strip_m = 1.02 * projection.step_m as f32;
            let stamps: Vec<_> = overlay
                .cols
                .iter()
                .flatten()
                .zip(original_mask.iter().flatten())
                .filter_map(|(column, &covered)| {
                    assert_eq!(column.road.is_some(), covered);
                    column.road
                })
                .collect();
            assert!(!stamps.is_empty());
            let fraction = stamps[0].surface_fraction;
            assert!(fraction > 0.0 && fraction <= 1.0);
            assert!(
                stamps
                    .iter()
                    .all(|stamp| stamp.surface_fraction == fraction)
            );
            if width < minimum_strip_m {
                assert!((fraction * minimum_strip_m - width).abs() < 1.0e-5);
            } else {
                assert_eq!(fraction, 1.0);
            }
            fractions.push(fraction);

            pack.roads[0].is_bridge = true;
            let bridge = render(&pack);
            assert!(
                bridge
                    .cols
                    .iter()
                    .flatten()
                    .all(|column| column.road.is_none()),
                "mapped bridges have their own continuous deck and must not also become ground-road coverage"
            );
        }
        assert!(fractions.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(fractions[0] < 0.02);
        assert_eq!(fractions[3], 1.0);
    }

    #[test]
    fn fine_railway_rasterizes_only_twin_rails() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-20.0, 0.0], [20.0, 0.0]],
            class: RoadClass::Rail,
            is_bridge: false,
        });
        let ov = build_overlay(cell, &frame, &pack, &|_, _| 0.0, 0, None, None, None);

        let mut rails = 0;
        let mut rail_rows = std::collections::BTreeSet::new();
        for (z, row) in ov.cols.iter().enumerate() {
            for column in row {
                let Some(stamp) = column.road else {
                    continue;
                };
                assert_eq!(stamp.class, RoadClass::Rail);
                assert_eq!(stamp.surface_fraction, 1.0);
                assert_eq!(
                    stamp.rail_part,
                    Some(RailPart::Rail),
                    "fine railways must stamp only steel rails"
                );
                rails += 1;
                rail_rows.insert(z);
            }
        }
        assert!(rails > 0, "railway lost its steel pair");
        assert!(
            rail_rows.len() >= 2,
            "standard-gauge pair collapsed to one row: {rail_rows:?}"
        );
        assert!(
            rails <= N * 4,
            "fine railway stamped a replacement surface instead of only rails: {rails}"
        );
    }

    #[test]
    fn coarse_railway_stays_a_continuous_cartographic_strip() {
        let (cell, frame) = test_cell(RAIL_DETAIL_MIN_LEVEL - 1);
        let mut pack = FeaturePack::empty();
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-100.0, 0.0], [100.0, 0.0]],
            class: RoadClass::Rail,
            is_bridge: false,
        });
        let ov = build_overlay(cell, &frame, &pack, &|_, _| 0.0, 0, None, None, None);
        let stamps: Vec<_> = ov
            .cols
            .iter()
            .flatten()
            .filter_map(|column| column.road)
            .collect();
        assert!(!stamps.is_empty());
        assert!(stamps.iter().all(|stamp| stamp.class == RoadClass::Rail
            && stamp.rail_part.is_none()
            && stamp.surface_fraction == 1.0));
    }

    #[test]
    fn watercourse_stroke_is_direction_independent_margin_aware_and_four_connected() {
        let n = N + 2 * WATER_MARGIN_COLS;
        let points = [[-6.25, -3.75], [N as f32 + 5.75, N as f32 + 3.25]];
        let raster = |points: &[[f32; 2]]| {
            let mut mask = vec![false; n * n];
            stamp_watercourse_grid(points, 0.25, n, WATER_MARGIN_COLS as i64, &mut |x, z| {
                mask[z * n + x] = true
            });
            mask
        };
        let forward = raster(&points);
        let reversed = raster(&[points[1], points[0]]);
        assert_eq!(forward, reversed);
        assert!(forward.iter().any(|marked| *marked));
        assert!((0..WATER_MARGIN_COLS).any(|x| (0..n).any(|z| forward[z * n + x])));
        assert!(((N + WATER_MARGIN_COLS)..n).any(|x| (0..n).any(|z| forward[z * n + x])));

        let start = forward.iter().position(|marked| *marked).unwrap();
        let mut seen = vec![false; n * n];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(index) = stack.pop() {
            let (x, z) = (index % n, index / n);
            for (dx, dz) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x as i64 + dx, z as i64 + dz);
                if nx < 0 || nz < 0 || nx >= n as i64 || nz >= n as i64 {
                    continue;
                }
                let next = nz as usize * n + nx as usize;
                if forward[next] && !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        assert_eq!(
            seen.iter().filter(|marked| **marked).count(),
            forward.iter().filter(|marked| **marked).count()
        );
    }

    #[test]
    fn watercourse_stroke_clips_all_octants_and_degenerate_source_segments() {
        let n = N + 2 * WATER_MARGIN_COLS;
        let center = [N as f32 * 0.5, N as f32 * 0.5];
        for far in [
            [-1_000_000.0, center[1]],
            [1_000_000.0, center[1]],
            [center[0], -1_000_000.0],
            [center[0], 1_000_000.0],
            [-1_000_000.0, -1_000_000.0],
            [1_000_000.0, -1_000_000.0],
            [-1_000_000.0, 1_000_000.0],
            [1_000_000.0, 1_000_000.0],
        ] {
            let raster = |points: &[[f32; 2]]| {
                let mut mask = vec![false; n * n];
                stamp_watercourse_grid(points, 0.25, n, WATER_MARGIN_COLS as i64, &mut |x, z| {
                    mask[z * n + x] = true
                });
                mask
            };
            let forward = raster(&[far, center]);
            let reversed = raster(&[center, far]);
            assert_eq!(forward, reversed, "direction changed clipped ray {far:?}");
            assert!(forward.iter().any(|marked| *marked));
            assert!(
                forward.iter().filter(|marked| **marked).count() <= 2 * n + 2,
                "clipped ray expanded beyond its 4-connected footprint"
            );
        }

        let mut degenerate = vec![false; n * n];
        stamp_watercourse_grid(
            &[center, center],
            0.25,
            n,
            WATER_MARGIN_COLS as i64,
            &mut |x, z| degenerate[z * n + x] = true,
        );
        assert_eq!(
            degenerate.iter().filter(|marked| **marked).count(),
            1,
            "a repeated source point should remain a bounded deterministic mark"
        );
    }

    #[test]
    fn watercourse_stroke_connects_across_adjacent_cell_interiors() {
        let grid = N + 2 * WATER_MARGIN_COLS;
        let global = [[-4.0, 3.25], [(2 * N) as f32 + 4.0, 19.25]];
        let raster = |cell_x: f32| {
            let local = [
                [global[0][0] - cell_x, global[0][1]],
                [global[1][0] - cell_x, global[1][1]],
            ];
            let mut mask = vec![false; grid * grid];
            stamp_watercourse_grid(&local, 0.25, grid, WATER_MARGIN_COLS as i64, &mut |x, z| {
                mask[z * grid + x] = true
            });
            mask
        };
        let left = raster(0.0);
        let right = raster(N as f32);
        let interior = |mask: &[bool], x: usize, z: usize| {
            mask[(z + WATER_MARGIN_COLS) * grid + x + WATER_MARGIN_COLS]
        };
        assert!(
            (0..N).any(|z| interior(&left, N - 1, z) && interior(&right, 0, z)),
            "independently rasterized neighbor cells lost their shared-edge connection"
        );

        let mut joined = vec![false; N * N * 2];
        for z in 0..N {
            for x in 0..N {
                joined[z * (2 * N) + x] = interior(&left, x, z);
                joined[z * (2 * N) + N + x] = interior(&right, x, z);
            }
        }
        let start = joined.iter().position(|marked| *marked).unwrap();
        let mut seen = vec![false; joined.len()];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(index) = stack.pop() {
            let (x, z) = (index % (2 * N), index / (2 * N));
            for (dx, dz) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x as i64 + dx, z as i64 + dz);
                if nx < 0 || nz < 0 || nx >= (2 * N) as i64 || nz >= N as i64 {
                    continue;
                }
                let next = nz as usize * (2 * N) + nx as usize;
                if joined[next] && !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        assert_eq!(
            seen.iter().filter(|marked| **marked).count(),
            joined.iter().filter(|marked| **marked).count()
        );
    }

    #[test]
    fn common_geodetic_watercourse_survives_cell_and_admitted_lod_reprojection() {
        let (left, left_frame) = test_cell(19);
        let right = CellId {
            i: left.i + 1,
            ..left
        };
        let right_frame = ChunkFrame::of(ChunkAddr { cell: right, k: 0 });
        let adjacent_line = [
            left_frame.column_latlon(-0.25, 0.5),
            right_frame.column_latlon(1.25, 0.5),
        ];
        let left_overlay = watercourse_overlay_from_geodetic(
            left,
            &left_frame,
            WatercourseKind::River,
            &adjacent_line,
        );
        let right_overlay = watercourse_overlay_from_geodetic(
            right,
            &right_frame,
            WatercourseKind::River,
            &adjacent_line,
        );
        let shared_rows = (0..N)
            .filter(|&z| left_overlay.cols[z][N - 1].water && right_overlay.cols[z][0].water)
            .collect::<Vec<_>>();
        assert!(
            !shared_rows.is_empty(),
            "one source line lost wet occupancy after adjacent query reprojection"
        );
        for z in shared_rows {
            assert_eq!(
                left_overlay.cols[z][N - 1].water_kind,
                right_overlay.cols[z][0].water_kind
            );
        }

        for (kind, parent_level) in [(WatercourseKind::River, 14), (WatercourseKind::Canal, 15)] {
            let (parent, parent_frame) = test_cell(parent_level);
            let midpoint = parent_frame.column_latlon(0.75, 0.25);
            let child = CellId::from_dir(
                vearth_core::ellipsoid::geodetic_to_ecef(midpoint.0, midpoint.1, 0.0).normalize(),
                parent.level + 1,
            );
            assert_eq!(child.parent(), Some(parent));
            let child_frame = ChunkFrame::of(ChunkAddr { cell: child, k: 0 });
            let line = [
                parent_frame.column_latlon(-0.25, 0.25),
                parent_frame.column_latlon(1.25, 0.25),
            ];
            let parent_overlay =
                watercourse_overlay_from_geodetic(parent, &parent_frame, kind, &line);
            let child_overlay = watercourse_overlay_from_geodetic(child, &child_frame, kind, &line);
            assert!(
                parent_overlay
                    .cols
                    .iter()
                    .flatten()
                    .any(|column| column.water),
                "admitted parent lost {kind:?}"
            );
            assert!(
                child_overlay
                    .cols
                    .iter()
                    .flatten()
                    .any(|column| column.water),
                "admitted child lost {kind:?}"
            );

            let projected = |frame: &ChunkFrame| {
                let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
                let mut dlon = midpoint.1 - lon0;
                if dlon > std::f64::consts::PI {
                    dlon -= std::f64::consts::TAU;
                } else if dlon < -std::f64::consts::PI {
                    dlon += std::f64::consts::TAU;
                }
                GridProj::of(frame, lat0, lon0).to_cols([
                    (dlon * lat0.cos() * MEAN_RADIUS) as f32,
                    ((midpoint.0 - lat0) * MEAN_RADIUS) as f32,
                ])
            };
            let parent_point = projected(&parent_frame);
            let child_point = projected(&child_frame);
            let cell_at = |overlay: &CellOverlay, point: [f32; 2]| {
                let x = point[0].floor().clamp(0.0, N as f32 - 1.0) as usize;
                let z = point[1].floor().clamp(0.0, N as f32 - 1.0) as usize;
                (overlay.cols[z][x].water, overlay.cols[z][x].water_kind)
            };
            let (parent_water, parent_kind) = cell_at(&parent_overlay, parent_point);
            let (child_water, child_kind) = cell_at(&child_overlay, child_point);
            assert!(parent_water && child_water);
            assert_eq!(parent_kind, child_kind);
        }
    }

    #[test]
    fn watercourse_lod_admission_never_turns_a_narrow_line_into_a_coarse_band() {
        let overlay = |level, kind, mode| {
            let (cell, frame) = test_cell(level);
            let mut pack = FeaturePack::empty();
            pack.watercourse_output_mode = mode;
            pack.watercourses.push(typed_watercourse(
                &frame,
                kind,
                1,
                &[[-4.0, 16.25], [N as f32 + 4.0, 16.25]],
            ));
            build_overlay(
                cell,
                &frame,
                &pack,
                &|_, _| 0.0,
                WATER_MARGIN_COLS,
                None,
                None,
                None,
            )
        };

        assert!(
            !overlay(
                13,
                WatercourseKind::River,
                WatercourseOutputMode::RiversAndCanals
            )
            .cols
            .iter()
            .flatten()
            .any(|column| column.water)
        );
        let river = overlay(
            14,
            WatercourseKind::River,
            WatercourseOutputMode::RiversAndCanals,
        );
        assert!((0..N).all(|x| river.cols[16][x].water));
        assert!(
            river
                .cols
                .iter()
                .flatten()
                .filter(|column| column.water)
                .count()
                <= N * 2
        );
        let refined_river = overlay(
            15,
            WatercourseKind::River,
            WatercourseOutputMode::RiversAndCanals,
        );
        assert!((0..N).all(|x| refined_river.cols[16][x].water));
        assert!(
            refined_river
                .cols
                .iter()
                .flatten()
                .filter(|column| column.water)
                .count()
                <= N * 2,
            "an admitted parent/child handoff widened the refined river"
        );
        assert!(
            !overlay(
                14,
                WatercourseKind::Canal,
                WatercourseOutputMode::RiversAndCanals
            )
            .cols
            .iter()
            .flatten()
            .any(|column| column.water)
        );
        assert!(
            overlay(
                15,
                WatercourseKind::Canal,
                WatercourseOutputMode::RiversAndCanals
            )
            .cols
            .iter()
            .flatten()
            .any(|column| column.water)
        );
        assert!(
            !overlay(
                19,
                WatercourseKind::River,
                WatercourseOutputMode::DisabledG96
            )
            .cols
            .iter()
            .flatten()
            .any(|column| column.water)
        );

        let (cell, frame) = test_cell(19);
        for (intermittent, is_bridge) in [(true, false), (false, true)] {
            let mut watercourse = typed_watercourse(
                &frame,
                WatercourseKind::River,
                2,
                &[[-4.0, 16.25], [N as f32 + 4.0, 16.25]],
            );
            watercourse.intermittent = intermittent;
            watercourse.is_bridge = is_bridge;
            let mut pack = FeaturePack::empty();
            pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
            pack.watercourses.push(watercourse);
            let excluded = build_overlay(
                cell,
                &frame,
                &pack,
                &|_, _| 0.0,
                WATER_MARGIN_COLS,
                None,
                None,
                None,
            );
            assert!(
                !excluded.cols.iter().flatten().any(|column| column.water),
                "unreviewed intermittent/bridge line became ordinary wet water"
            );
        }

        let mut simplified = typed_watercourse(
            &frame,
            WatercourseKind::River,
            3,
            &[[-4.0, 16.25], [N as f32 + 4.0, 16.25]],
        );
        simplified.source_tile.0 = simplified.authority_zoom - 1;
        let mut pack = FeaturePack::empty();
        pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
        pack.watercourses.push(simplified);
        let excluded = build_overlay(
            cell,
            &frame,
            &pack,
            &|_, _| 0.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        );
        assert!(
            !excluded.cols.iter().flatten().any(|column| column.water),
            "a simplified non-authority source zoom entered physical output"
        );
    }

    #[test]
    fn watercourse_width_projection_stays_within_reviewed_two_x_bound() {
        for level in [14u8, 15, 19] {
            let edge = (1u32 << level) - 1;
            for face in 0..6 {
                for (i, j) in [(0, 0), (0, edge / 2), (edge / 2, edge / 2), (edge, edge)] {
                    let cell = CellId {
                        face: Face(face),
                        level,
                        i,
                        j,
                    };
                    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                    let (lat, lon) = frame.column_latlon(0.5, 0.5);
                    let projection = GridProj::of(&frame, lat, lon);
                    let determinant = projection.inv[0] * projection.inv[3]
                        - projection.inv[1] * projection.inv[2];
                    let a = projection.inv[3] / determinant;
                    let b = -projection.inv[1] / determinant;
                    let c = -projection.inv[2] / determinant;
                    let d = projection.inv[0] / determinant;
                    let x_axis = a.hypot(c) / N as f64;
                    let z_axis = b.hypot(d) / N as f64;
                    let min_axis = x_axis.min(z_axis);
                    let max_axis = x_axis.max(z_axis);
                    assert!(
                        (projection.step_m - min_axis).abs() / min_axis < 1e-9,
                        "width scale diverged from the conservative axis at {cell:?}"
                    );
                    assert!(
                        (projection.max_step_m - max_axis).abs() / max_axis < 1e-9,
                        "conservative width scale diverged from the long axis at {cell:?}"
                    );
                    for kind in [WatercourseKind::River, WatercourseKind::Canal] {
                        let watercourse =
                            typed_watercourse(&frame, kind, 1, &[[0.0, 16.0], [N as f32, 16.0]]);
                        let admitted = watercourse_admitted(
                            &watercourse,
                            WatercourseOutputMode::RiversAndCanals,
                            level,
                            projection.max_step_m as f32,
                        );
                        assert_eq!(
                            admitted,
                            level >= kind.minimum_terrain_level()
                                && projection.max_step_m as f32 <= kind.fallback_width_m() * 2.0,
                            "distortion-aware admission changed at {cell:?} for {kind:?}"
                        );
                        let radius_cols =
                            kind.fallback_width_m() * 0.5 / projection.max_step_m as f32;
                        assert!(
                            (radius_cols * projection.max_step_m as f32
                                - kind.fallback_width_m() * 0.5)
                                .abs()
                                < 1e-5,
                            "long-axis footprint widened at {cell:?} for {kind:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn watercourse_union_preserves_ocean_and_typed_polygon_authority() {
        let (cell, frame) = test_cell(19);
        let line = typed_watercourse(
            &frame,
            WatercourseKind::River,
            30,
            &[[-4.0, 16.0], [N as f32 + 4.0, 16.0]],
        );
        let mut inland = FeaturePack::empty();
        inland.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
        inland.watercourses.push(line.clone());
        inland
            .water
            .push(typed_water(WaterRegionKind::Lake, 10, 4.0));
        let mut polygon_only = FeaturePack::empty();
        polygon_only
            .water
            .push(typed_water(WaterRegionKind::Lake, 10, 4.0));
        let polygon_overlay = build_overlay(
            cell,
            &frame,
            &polygon_only,
            &|_, _| 0.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        );
        let inland_overlay = build_overlay(
            cell,
            &frame,
            &inland,
            &|_, _| 0.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        );
        assert_eq!(
            inland_overlay.cols[N / 2][N / 2].water_kind,
            Some(WaterRegionKind::Lake)
        );
        assert!(inland_overlay.cols[N / 2][2].water);
        assert_eq!(
            inland_overlay.cols[N / 2][2].water_kind,
            Some(WaterRegionKind::River)
        );
        assert!((0..N).all(|x| inland_overlay.cols[N / 2][x].water));
        for z in 0..N {
            for x in 0..N {
                if polygon_overlay.cols[z][x].water {
                    assert!(
                        inland_overlay.cols[z][x].water_shore + f32::EPSILON
                            >= polygon_overlay.cols[z][x].water_shore,
                        "overlapping line introduced an internal polygon shore"
                    );
                    assert_eq!(
                        inland_overlay.cols[z][x].water_kind, polygon_overlay.cols[z][x].water_kind,
                        "overlapping line changed the polygon semantic owner"
                    );
                }
            }
        }

        let ocean_rings = vec![vec![
            en_from_cols(&frame, [0.0, 0.0]),
            en_from_cols(&frame, [16.0, 0.0]),
            en_from_cols(&frame, [16.0, N as f32]),
            en_from_cols(&frame, [0.0, N as f32]),
        ]];
        let mut coast = FeaturePack::empty();
        coast.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
        coast.watercourses.push(line);
        coast.water.push(Water {
            rings: ocean_rings,
            kind: WaterRegionKind::Ocean,
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(1),
            intermittent: false,
            alkaline: false,
        });
        let coast_overlay = build_overlay(
            cell,
            &frame,
            &coast,
            &|_, _| 0.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        );
        assert!(coast_overlay.cols[N / 2][8].ocean);
        assert!(!coast_overlay.cols[N / 2][8].water);
        assert!(coast_overlay.cols[N / 2][24].water);
        assert_eq!(
            coast_overlay.cols[N / 2][24].water_kind,
            Some(WaterRegionKind::River)
        );
        assert!(
            (0..N).all(|x| {
                coast_overlay.cols[N / 2][x].ocean || coast_overlay.cols[N / 2][x].water
            }),
            "ocean outlet contains a dry transition cell"
        );
    }

    #[test]
    fn exact_tree_respects_incompatible_ground_and_supported_cliffs() {
        let (cell, frame) = test_cell(19);
        let point = en_from_cols(&frame, [N as f32 * 0.5, N as f32 * 0.5]);
        let exact_tree = || ExactTree {
            point,
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(99),
            seed: 0x7068_7973_6963_616c,
        };

        for kind in [
            GroundKind::BareRock,
            GroundKind::Glacier,
            GroundKind::RunwayArea,
            GroundKind::RegionalBarren,
            GroundKind::RegionalGlacier,
        ] {
            let mut pack = FeaturePack::empty();
            pack.ground.push((kind, square(80.0)));
            pack.exact_trees.push(exact_tree());
            let overlay = build_overlay(cell, &frame, &pack, &|_, _| 42.0, 0, None, None, None);
            assert!(
                overlay
                    .tree_instances
                    .iter()
                    .all(|tree| tree.seed != 0x7068_7973_6963_616c),
                "{kind:?} must exclude an exact tree"
            );
        }

        for kind in [GroundKind::Paved, GroundKind::Urban, GroundKind::Green] {
            let mut pack = FeaturePack::empty();
            pack.ground.push((kind, square(80.0)));
            pack.exact_trees.push(exact_tree());
            let overlay = build_overlay(cell, &frame, &pack, &|_, _| 42.0, 0, None, None, None);
            assert!(
                overlay
                    .tree_instances
                    .iter()
                    .any(|tree| tree.seed == 0x7068_7973_6963_616c),
                "mapped street trees remain eligible on {kind:?}"
            );
        }

        let mut cliff_pack = FeaturePack::empty();
        cliff_pack.ground.push((GroundKind::Green, square(80.0)));
        cliff_pack.exact_trees.push(exact_tree());
        cliff_pack.cliffs.push(CliffLine {
            pts: vec![
                en_from_cols(&frame, [4.0, N as f32 * 0.5]),
                en_from_cols(&frame, [N as f32 - 4.0, N as f32 * 0.5]),
            ],
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(100),
        });
        let flat = build_overlay(cell, &frame, &cliff_pack, &|_, _| 42.0, 1, None, None, None);
        assert!(
            flat.tree_instances
                .iter()
                .any(|tree| tree.seed == 0x7068_7973_6963_616c),
            "an unsupported cliff line alone must not suppress the tree"
        );
        let steep = build_overlay(
            cell,
            &frame,
            &cliff_pack,
            &|x, _| if x < N as i32 / 2 { 36.0 } else { 42.0 },
            1,
            None,
            None,
            None,
        );
        assert!(
            steep
                .tree_instances
                .iter()
                .all(|tree| tree.seed != 0x7068_7973_6963_616c),
            "the resolved DEM-supported cliff ground must suppress the tree"
        );
    }

    #[test]
    fn cliff_surface_requires_a_dem_break_and_respects_stronger_owners() {
        let (cell, frame) = test_cell(19);
        let endpoints = [
            en_from_cols(&frame, [4.0, N as f32 * 0.5]),
            en_from_cols(&frame, [N as f32 - 4.0, N as f32 * 0.5]),
        ];
        let cliff = CliffLine {
            pts: endpoints.to_vec(),
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(901),
        };
        let mut pack = FeaturePack::empty();
        pack.cliffs.push(cliff.clone());
        let reads = std::cell::Cell::new(0usize);
        let steep = build_overlay(
            cell,
            &frame,
            &pack,
            &|x, _| {
                reads.set(reads.get() + 1);
                x as f32 * 2.0
            },
            0,
            None,
            None,
            None,
        );
        assert!(reads.get() <= 5 * N * N);
        assert!(
            steep.cols[N / 2]
                .iter()
                .any(|column| { column.ground == Some(GroundKind::BareRock) })
        );

        let flat = build_overlay(cell, &frame, &pack, &|_, _| 42.0, 0, None, None, None);
        assert!(
            flat.cols
                .iter()
                .flatten()
                .all(|column| { column.ground != Some(GroundKind::BareRock) })
        );

        pack.cliffs[0].pts.reverse();
        let reversed = build_overlay(
            cell,
            &frame,
            &pack,
            &|x, _| x as f32 * 2.0,
            0,
            None,
            None,
            None,
        );
        for z in 0..N {
            for x in 0..N {
                assert_eq!(steep.cols[z][x].ground, reversed.cols[z][x].ground);
            }
        }

        pack.water.push(Water::inland(square(80.0)));
        let water = build_overlay(
            cell,
            &frame,
            &pack,
            &|x, _| x as f32 * 2.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        );
        assert!(water.cols[N / 2][N / 2].water);
        assert_ne!(water.cols[N / 2][N / 2].ground, Some(GroundKind::BareRock));

        let steep = |x: i32, _: i32| x as f32 * 2.0;
        let mut road_pack = FeaturePack::empty();
        road_pack.cliffs.push(cliff.clone());
        road_pack.roads.push(Road {
            profile: Default::default(),
            pts: endpoints.to_vec(),
            class: RoadClass::Minor,
            is_bridge: false,
        });
        let road = build_overlay(cell, &frame, &road_pack, &steep, 0, None, None, None);
        assert!(road.cols[N / 2][N / 2].road.is_some());
        assert_ne!(road.cols[N / 2][N / 2].ground, Some(GroundKind::BareRock));

        let mut building_pack = FeaturePack::empty();
        building_pack.cliffs.push(cliff.clone());
        building_pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(80.0),
            height_m: 12.0,
            min_height_m: 0.0,
            seed: 7,
            kind: BuildingKind::Building,
        });
        let building = build_overlay(cell, &frame, &building_pack, &steep, 0, None, None, None);
        assert!(building.cols[N / 2][N / 2].building.is_some());
        assert_ne!(
            building.cols[N / 2][N / 2].ground,
            Some(GroundKind::BareRock)
        );

        let mut flora_pack = FeaturePack::empty();
        flora_pack.cliffs.push(cliff.clone());
        flora_pack.flora.push(FloraRegion {
            location_rad: None,
            rings: square(80.0),
            kind: FloraRegionKind::Farmland,
            seed: 8,
        });
        let flora = build_overlay(cell, &frame, &flora_pack, &steep, 0, None, None, None);
        assert!(flora.cols[N / 2][N / 2].flora.is_some());
        assert_ne!(flora.cols[N / 2][N / 2].ground, Some(GroundKind::BareRock));

        for protected in [
            GroundKind::Glacier,
            GroundKind::Wetland,
            GroundKind::Paved,
            GroundKind::Urban,
            GroundKind::Sand,
            GroundKind::Farmyard,
            GroundKind::RailwayArea,
            GroundKind::AirportArea,
            GroundKind::RunwayArea,
            GroundKind::RegionalGlacier,
            GroundKind::RegionalUrban,
        ] {
            let mut protected_pack = FeaturePack::empty();
            protected_pack.cliffs.push(cliff.clone());
            protected_pack.ground.push((protected, square(80.0)));
            let protected_overlay =
                build_overlay(cell, &frame, &protected_pack, &steep, 0, None, None, None);
            assert_eq!(
                protected_overlay.cols[N / 2][N / 2].ground,
                Some(protected),
                "cliff replaced {protected:?}"
            );
        }
    }

    #[test]
    fn cliff_support_uses_physical_slope_across_its_lod_gate() {
        let resolved_center = |level: u8| {
            let (cell, frame) = test_cell(level);
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = GridProj::of(&frame, lat, lon);
            let mut pack = FeaturePack::empty();
            pack.cliffs.push(CliffLine {
                pts: vec![
                    en_from_cols(&frame, [4.0, N as f32 * 0.5]),
                    en_from_cols(&frame, [N as f32 - 4.0, N as f32 * 0.5]),
                ],
                identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(902),
            });
            let rise_per_column = 0.5f32.tan() * projection.step_m as f32;
            let overlay = build_overlay(
                cell,
                &frame,
                &pack,
                &|x, _| x as f32 * rise_per_column,
                usize::from(level >= CLIFF_SURFACE_MIN_LEVEL),
                None,
                None,
                None,
            );
            overlay.cols[N / 2][N / 2].ground == Some(GroundKind::BareRock)
        };

        assert!(!resolved_center(CLIFF_SURFACE_MIN_LEVEL - 1));
        assert!(resolved_center(CLIFF_SURFACE_MIN_LEVEL));
        assert!(resolved_center(19));
    }

    #[test]
    fn cliff_surface_is_continuous_across_adjacent_cells() {
        let level = 19;
        let n = 1u32 << level;
        let left_cell = CellId {
            face: Face(0),
            level,
            i: n / 2,
            j: n / 2,
        };
        let right_cell = CellId {
            i: left_cell.i + 1,
            ..left_cell
        };
        let left_frame = ChunkFrame::of(ChunkAddr {
            cell: left_cell,
            k: 0,
        });
        let right_frame = ChunkFrame::of(ChunkAddr {
            cell: right_cell,
            k: 0,
        });
        let start = left_frame.column_latlon(0.75, 0.5);
        let end = right_frame.column_latlon(0.25, 0.5);
        let pack_for = |frame: &ChunkFrame| {
            let mut pack = FeaturePack::empty();
            pack.cliffs.push(CliffLine {
                pts: vec![
                    en_from_latlon(frame, start.0, start.1),
                    en_from_latlon(frame, end.0, end.1),
                ],
                identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(903),
            });
            pack
        };
        let left_projection = GridProj::of(&left_frame, start.0, start.1);
        let right_projection = GridProj::of(&right_frame, end.0, end.1);
        let left_rise = 0.5f32.tan() * left_projection.step_m as f32;
        let right_rise = 0.5f32.tan() * right_projection.step_m as f32;
        let left = build_overlay(
            left_cell,
            &left_frame,
            &pack_for(&left_frame),
            &|x, _| x as f32 * left_rise,
            1,
            None,
            None,
            None,
        );
        let right = build_overlay(
            right_cell,
            &right_frame,
            &pack_for(&right_frame),
            &|x, _| x as f32 * right_rise,
            1,
            None,
            None,
            None,
        );

        assert_eq!(left.cols[N / 2][N - 1].ground, Some(GroundKind::BareRock));
        assert_eq!(right.cols[N / 2][0].ground, Some(GroundKind::BareRock));
    }

    #[test]
    fn water_shore_signal_runs_from_edge_to_deeper_center() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.water.push(Water::inland(square(6.0)));
        let ov = build_overlay(
            cell,
            &frame,
            &pack,
            &|_, _| 0.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        );
        let row = &ov.cols[N / 2];
        let edge = row.iter().position(|column| column.water).unwrap();
        let edge_shore = row[edge].water_shore;
        let center_shore = row[N / 2].water_shore;
        assert!(edge_shore < 0.2, "edge signal {edge_shore}");
        assert!(
            center_shore > edge_shore + 0.18,
            "edge {edge_shore}, center {center_shore}"
        );
    }

    #[test]
    fn typed_water_mapping_and_ecology_are_explicit_for_every_polygon_role() {
        let (cell, frame) = test_cell(19);
        for kind in [
            WaterRegionKind::Ocean,
            WaterRegionKind::InlandWater,
            WaterRegionKind::Lake,
            WaterRegionKind::Reservoir,
            WaterRegionKind::River,
            WaterRegionKind::Canal,
            WaterRegionKind::Stream,
            WaterRegionKind::Drainage,
            WaterRegionKind::Basin,
            WaterRegionKind::Dock,
            WaterRegionKind::Decorative,
            WaterRegionKind::Playa,
            WaterRegionKind::Reef,
            WaterRegionKind::Other,
            WaterRegionKind::CompatibilityUnknown,
        ] {
            let mut pack = FeaturePack::empty();
            pack.water.push(typed_water(kind, 1, 6.0));
            let ov = build_overlay(cell, &frame, &pack, &|_, _| 20.0, 0, None, None, None);
            let center = ov.cols[N / 2][N / 2];
            assert_eq!(center.ocean, kind == WaterRegionKind::Ocean, "{kind:?}");
            assert_eq!(
                center.water,
                kind.creates_surface_water() && kind != WaterRegionKind::Ocean,
                "{kind:?}"
            );
            assert_eq!(
                center.water_ecology,
                kind.creates_surface_water() && kind.contributes_ecological_proximity(),
                "{kind:?}"
            );
            if kind.creates_surface_water() {
                assert_eq!(center.water_kind, Some(kind), "{kind:?}");
            } else {
                assert_eq!(center.water_kind, None, "{kind:?}");
            }
        }

        let mut legacy = FeaturePack::empty();
        legacy.water_polygon_mode = WaterPolygonMode::LegacyUndifferentiated;
        legacy
            .water
            .push(typed_water(WaterRegionKind::Playa, 1, 6.0));
        let legacy = build_overlay(cell, &frame, &legacy, &|_, _| 20.0, 0, None, None, None);
        let center = legacy.cols[N / 2][N / 2];
        assert!(center.water && center.water_ecology);
        assert_eq!(center.water_kind, Some(WaterRegionKind::Playa));
    }

    #[test]
    fn typed_water_overlap_is_order_independent_and_uses_lower_identity_ties() {
        let (cell, frame) = test_cell(19);
        let build = |water: Vec<Water>| {
            let mut pack = FeaturePack::empty();
            pack.water = water;
            build_overlay(cell, &frame, &pack, &|_, _| 20.0, 0, None, None, None).cols[N / 2][N / 2]
        };
        let broad = || typed_water(WaterRegionKind::InlandWater, 50, 6.0);
        let decorative = || typed_water(WaterRegionKind::Decorative, 40, 6.0);
        let lake_high_id = || typed_water(WaterRegionKind::Lake, 30, 6.0);
        let lake_low_id = || typed_water(WaterRegionKind::Lake, 20, 6.0);

        let forward = build(vec![broad(), decorative(), lake_high_id(), lake_low_id()]);
        let reverse = build(vec![lake_low_id(), lake_high_id(), decorative(), broad()]);
        for result in [forward, reverse] {
            assert!(result.water);
            assert_eq!(result.water_kind, Some(WaterRegionKind::Lake));
            assert!(result.water_ecology);
        }
        assert!(water_region_wins(
            WaterRegionStamp {
                kind: WaterRegionKind::Lake,
                identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(20),
            },
            WaterRegionStamp {
                kind: WaterRegionKind::Lake,
                identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(30),
            }
        ));
    }

    #[test]
    fn decorative_and_drainage_water_do_not_create_riparian_tree_context() {
        let (cell, frame) = test_cell(19);
        let pack = |kind| {
            let mut pack = FeaturePack::empty();
            pack.water.push(typed_water(kind, 1, 3.0));
            pack
        };
        for kind in [WaterRegionKind::Decorative, WaterRegionKind::Drainage] {
            let pack = pack(kind);
            let ov = build_overlay(cell, &frame, &pack, &|_, _| 20.0, 0, None, None, None);
            assert!(ov.cols[N / 2][N / 2].water);
            assert!(!tree_near_water(&pack, [5.0, 0.0]));
        }
        let lake = pack(WaterRegionKind::Lake);
        assert!(tree_near_water(&lake, [5.0, 0.0]));
        assert!(!tree_near_water(&lake, [10.0, 0.0]));
    }

    #[test]
    fn typed_ocean_preserves_explicit_land_over_the_automatic_beach_band() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        let explicit_ground = square(6.0);
        pack.ground
            .push((GroundKind::Green, explicit_ground.clone()));
        pack.water.push(Water {
            rings: vec![vec![[-6.0, -6.0], [0.0, -6.0], [0.0, 6.0], [-6.0, 6.0]]],
            kind: WaterRegionKind::Ocean,
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(1),
            intermittent: false,
            alkaline: false,
        });
        let ov = build_overlay(cell, &frame, &pack, &|_, _| 0.0, 0, None, None, None);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let proj = GridProj::of(&frame, lat, lon);
        let projected = explicit_ground
            .iter()
            .map(|ring| ring.iter().map(|&point| proj.to_cols(point)).collect())
            .collect::<Vec<Vec<[f32; 2]>>>();
        let mut explicit_mask = [[false; N]; N];
        fill_polygon(&projected, |x, z| explicit_mask[z][x] = true);

        assert!(ov.cols.iter().flatten().any(|column| column.ocean));
        let mut dry_explicit = 0;
        for z in 0..N {
            for x in 0..N {
                if explicit_mask[z][x] && !ov.cols[z][x].ocean {
                    dry_explicit += 1;
                    assert_eq!(ov.cols[z][x].ground, Some(GroundKind::Green));
                }
            }
        }
        assert!(dry_explicit > 0);
    }

    #[test]
    fn incomplete_water_pack_does_not_treat_tile_edges_as_shore() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.complete = false;
        pack.water.push(Water::inland(square(6.0)));
        let ov = build_overlay(
            cell,
            &frame,
            &pack,
            &|_, _| 0.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        );
        assert!(
            ov.cols
                .iter()
                .flatten()
                .filter(|column| column.water)
                .all(|column| column.water_shore == 1.0)
        );
    }

    #[test]
    fn road_surface_profile_has_no_station_plateaus_and_shares_corner_heights() {
        let (_, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-100.0, 0.0], [100.0, 0.0]],
            class: RoadClass::Minor,
            is_bridge: false,
        });
        let elevation = |e: f64, _: f64| (40.0 + e * 0.5) as f32;
        let grades = road_surface_grades(&pack, &frame, 19, &elevation).unwrap();
        let row = N / 2;
        let deltas: Vec<_> = grades.corners[row]
            .windows(2)
            .map(|pair| pair[1].1 - pair[0].1)
            .collect();
        assert!(deltas.iter().all(|delta| *delta > 0.1));
        assert!(
            deltas
                .iter()
                .all(|delta| (*delta - deltas[0]).abs() < 0.0001)
        );
        for x in 0..N {
            assert!(grades.columns[row][x].0 > 0.99);
            let midpoint = (grades.corners[row][x].1 + grades.corners[row][x + 1].1) * 0.5;
            assert!((grades.columns[row][x].1 - midpoint).abs() < 0.0001);
        }
        pack.roads[0].pts.reverse();
        let reversed = road_surface_grades(&pack, &frame, 19, &elevation).unwrap();
        for (a, b) in grades
            .corners
            .iter()
            .flatten()
            .zip(reversed.corners.iter().flatten())
        {
            assert!((a.0 - b.0).abs() < 0.0001 && (a.1 - b.1).abs() < 0.0001);
        }
        pack.roads.push(Road {
            profile: Default::default(),
            pts: pack.roads[0].pts.clone(),
            class: pack.roads[0].class,
            is_bridge: pack.roads[0].is_bridge,
        });
        let duplicated = road_surface_grades(&pack, &frame, 19, &elevation).unwrap();
        assert_eq!(reversed.corners, duplicated.corners);
        assert_eq!(reversed.columns, duplicated.columns);
    }

    #[test]
    fn trail_grading_preserves_more_of_the_original_hillside() {
        use vearth_data::road_profile::RoadDetail;
        let (_, frame) = test_cell(19);
        let elevation = |east: f64, _: f64| (15.0 * (-(east / 12.0).powi(2)).exp()) as f32;
        let mut pack = FeaturePack::empty();
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-200.0, 0.0], [200.0, 0.0]],
            class: RoadClass::Minor,
            is_bridge: false,
        });
        let street = road_surface_grades(&pack, &frame, 19, &elevation).unwrap();
        pack.roads[0].class = RoadClass::Path;
        pack.roads[0].profile.detail = RoadDetail::Footway;
        let trail = road_surface_grades(&pack, &frame, 19, &elevation).unwrap();
        let (sw, sh) = street.corners[N / 2][N / 2];
        let (tw, th) = trail.corners[N / 2][N / 2];
        assert!(sw > 0.99 && tw <= 0.23);
        assert!((15.0 - (15.0 + (th - 15.0) * tw)).abs() < (15.0 - sh).abs());
        let footprint =
            |g: &RoadSurfaceGrades| g.corners.iter().flatten().filter(|(w, _)| *w > 0.0).count();
        assert!(footprint(&trail) < footprint(&street));
    }

    #[test]
    fn road_grades_follow_smoothed_profile() {
        let (_cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        // 200 m street through the cell: the cell interior sits far from
        // both way ends, so the endpoint taper is 1 there.
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-100.0, 0.0], [100.0, 0.0]],
            class: RoadClass::Minor,
            is_bridge: false,
        });
        // DEM: gentle eastward slope, 0.5 m per meter.
        let elev_en = |e: f64, _: f64| (e * 0.5) as f32;
        let grades = road_grades(&pack, &frame, 19, &elev_en).expect("grades");
        // On-axis mid-cell column: full weight, target near the local DEM
        // (linear slope → box mean = center value).
        let (w, target) = grades[N / 2][N / 2];
        assert!(w > 0.99, "on-axis weight {w}");
        assert!(target.abs() < 1.5, "target {target}");
        // Target grades along the way.
        let (_, t_e) = grades[N / 2][N - 2];
        assert!(t_e > target + 1.0, "east target {t_e} vs {target}");
        // Columns far from the road stay untouched.
        assert_eq!(grades[1][N / 2].0, 0.0);
        // Bridges never grade terrain.
        pack.roads[0].is_bridge = true;
        assert!(road_grades(&pack, &frame, 19, &elev_en).unwrap()[N / 2][N / 2].0 == 0.0);
        // Below the gate: no grading at all.
        pack.roads[0].is_bridge = false;
        assert!(road_grades(&pack, &frame, 14, &elev_en).is_none());
    }

    #[test]
    fn procedural_tree_placement_never_samples_unowned_roots() {
        let (cell, frame) = test_cell(19);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = GridProj::of(&frame, lat, lon);
        let mut pack = FeaturePack::empty();
        pack.forest.push(vec![vec![
            projection.to_en([0.0, 0.0]),
            projection.to_en([32.0, 0.0]),
            projection.to_en([32.0, 32.0]),
            projection.to_en([0.0, 32.0]),
        ]]);
        let outside_reads = std::cell::Cell::new(0);
        let elevation = |x: i32, z: i32| {
            if !(0..32).contains(&x) || !(0..32).contains(&z) {
                outside_reads.set(outside_reads.get() + 1);
            }
            50.0
        };
        let mut overlay = CellOverlay::empty();
        place_trees(
            cell,
            &frame,
            &mut overlay,
            &[[true; N]; N],
            &pack,
            &projection,
            &[],
            &elevation,
            None,
            projection.step_m as f32,
        );
        assert!(!overlay.tree_instances.is_empty());
        assert_eq!(
            outside_reads.get(),
            0,
            "unsupported off-cell forest candidates were synthesized"
        );
    }

    #[test]
    fn flora_overlap_precedence_is_stable_and_physical_surfaces_win() {
        let regions = [
            (FloraRegionKind::Meadow, 90),
            (FloraRegionKind::Farmland, 80),
            (FloraRegionKind::Garden, 70),
            (FloraRegionKind::Vineyard, 60),
            (FloraRegionKind::Orchard, 100),
        ];
        let (cell, frame) = test_cell(17);
        let generate = |reverse: bool, paved: bool| {
            let mut pack = FeaturePack::empty();
            let mut ordered = regions.to_vec();
            if reverse {
                ordered.reverse();
            }
            for (kind, seed) in ordered {
                pack.flora.push(vearth_data::vectors::FloraRegion {
                    location_rad: None,
                    rings: square(3_000.0),
                    kind,
                    seed,
                });
            }
            // A duplicate fragment is deliberately idempotent.
            pack.flora.push(vearth_data::vectors::FloraRegion {
                location_rad: None,
                rings: square(3_000.0),
                kind: FloraRegionKind::Orchard,
                seed: 100,
            });
            if paved {
                pack.ground.push((GroundKind::Paved, square(3_000.0)));
            }
            build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None)
        };

        let expected = Some(FloraStamp {
            location_rad: None,
            kind: FloraRegionKind::Orchard,
            seed: 100,
            curated_style: 0,
            surface: select_field_surface(FloraRegionKind::Orchard, 100),
        });
        assert_eq!(generate(false, false).cols[N / 2][N / 2].flora, expected);
        assert_eq!(generate(true, false).cols[N / 2][N / 2].flora, expected);
        let paved = generate(false, true);
        assert_eq!(paved.cols[N / 2][N / 2].ground, Some(GroundKind::Paved));
        assert_eq!(paved.cols[N / 2][N / 2].flora, None);

        let flora_pack = || {
            let mut pack = FeaturePack::empty();
            pack.flora.push(vearth_data::vectors::FloraRegion {
                location_rad: None,
                rings: square(3_000.0),
                kind: FloraRegionKind::Farmland,
                seed: 7,
            });
            pack
        };

        let mut road = flora_pack();
        road.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-3_000.0, 0.0], [3_000.0, 0.0]],
            class: RoadClass::Major,
            is_bridge: false,
        });
        assert_eq!(
            build_overlay(cell, &frame, &road, &|_, _| 50.0, 0, None, None, None).cols[N / 2]
                [N / 2]
                .flora,
            None
        );

        let mut water = flora_pack();
        water.water.push(Water::inland(square(3_000.0)));
        assert_eq!(
            build_overlay(
                cell,
                &frame,
                &water,
                &|_, _| 50.0,
                WATER_MARGIN_COLS,
                None,
                None,
                None,
            )
            .cols[N / 2][N / 2]
                .flora,
            None
        );

        let mut building = flora_pack();
        building.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(3_000.0),
            height_m: 12.0,
            min_height_m: 0.0,
            seed: 3,
            kind: BuildingKind::Building,
        });
        assert_eq!(
            build_overlay(cell, &frame, &building, &|_, _| 50.0, 0, None, None, None,).cols[N / 2]
                [N / 2]
                .flora,
            None
        );
    }

    #[test]
    fn natural_land_precedence_is_order_independent_and_existing_outputs_win() {
        let kinds = [
            GroundKind::NaturalGrass,
            GroundKind::Scrub,
            GroundKind::Wetland,
            GroundKind::BareRock,
            GroundKind::Glacier,
        ];
        let (cell, frame) = test_cell(17);
        let generate = |reverse: bool, existing: Option<GroundKind>, flora: bool| {
            let mut pack = FeaturePack::empty();
            let mut ordered = kinds.to_vec();
            if reverse {
                ordered.reverse();
            }
            if !reverse {
                pack.ground
                    .push((GroundKind::NaturalGrass, square(3_000.0)));
            }
            for kind in ordered {
                pack.ground.push((kind, square(3_000.0)));
            }
            if reverse {
                pack.ground
                    .push((GroundKind::NaturalGrass, square(3_000.0)));
            }
            if existing == Some(GroundKind::Forest) {
                pack.forest.push(square(3_000.0));
            } else if let Some(kind) = existing {
                pack.ground.push((kind, square(3_000.0)));
            }
            if flora {
                pack.flora.push(vearth_data::vectors::FloraRegion {
                    location_rad: None,
                    rings: square(3_000.0),
                    kind: FloraRegionKind::Farmland,
                    seed: 77,
                });
            }
            build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None)
        };

        assert_eq!(
            generate(false, None, false).cols[N / 2][N / 2].ground,
            Some(GroundKind::Glacier)
        );
        assert_eq!(
            generate(true, None, false).cols[N / 2][N / 2].ground,
            Some(GroundKind::Glacier)
        );
        assert_eq!(
            generate(false, Some(GroundKind::Forest), false).cols[N / 2][N / 2].ground,
            Some(GroundKind::Forest)
        );
        assert_eq!(
            generate(false, Some(GroundKind::Paved), false).cols[N / 2][N / 2].ground,
            Some(GroundKind::Paved)
        );
        let flora = generate(false, None, true);
        assert_eq!(flora.cols[N / 2][N / 2].ground, Some(GroundKind::Green));
        assert!(flora.cols[N / 2][N / 2].flora.is_some());
    }

    #[test]
    fn managed_land_precedence_is_order_independent_and_existing_outputs_win() {
        let kinds = [
            GroundKind::Farmyard,
            GroundKind::AirportArea,
            GroundKind::RailwayArea,
            GroundKind::RunwayArea,
        ];
        let (cell, frame) = test_cell(17);
        let generate = |reverse: bool, existing: Option<GroundKind>, flora: bool| {
            let mut pack = FeaturePack::empty();
            let mut ordered = kinds.to_vec();
            if reverse {
                ordered.reverse();
            }
            for kind in ordered {
                pack.ground.push((kind, square(3_000.0)));
            }
            if existing == Some(GroundKind::Forest) {
                pack.forest.push(square(3_000.0));
            } else if let Some(kind) = existing {
                pack.ground.push((kind, square(3_000.0)));
            }
            if flora {
                pack.flora.push(vearth_data::vectors::FloraRegion {
                    location_rad: None,
                    rings: square(3_000.0),
                    kind: FloraRegionKind::Farmland,
                    seed: 81,
                });
            }
            build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None)
        };

        assert_eq!(
            generate(false, None, false).cols[N / 2][N / 2].ground,
            Some(GroundKind::RunwayArea)
        );
        assert_eq!(
            generate(true, None, false).cols[N / 2][N / 2].ground,
            Some(GroundKind::RunwayArea)
        );
        assert_eq!(
            generate(false, Some(GroundKind::Forest), false).cols[N / 2][N / 2].ground,
            Some(GroundKind::Forest)
        );
        assert_eq!(
            generate(false, Some(GroundKind::Paved), false).cols[N / 2][N / 2].ground,
            Some(GroundKind::Paved)
        );
        let flora = generate(false, None, true);
        assert_eq!(flora.cols[N / 2][N / 2].ground, Some(GroundKind::Green));
        assert!(flora.cols[N / 2][N / 2].flora.is_some());
        assert!(flora.tree_instances.is_empty());

        for level in [GROUND_MIN_LEVEL, 15, 19] {
            let (cell, frame) = test_cell(level);
            let mut pack = FeaturePack::empty();
            pack.ground.push((GroundKind::AirportArea, square(3_000.0)));
            let first = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
            let second = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
            assert_eq!(
                first.cols[N / 2][N / 2].ground,
                Some(GroundKind::AirportArea),
                "airport substrate disappeared at L{level}"
            );
            assert_eq!(
                first.cols[N / 2][N / 2].ground,
                second.cols[N / 2][N / 2].ground,
                "airport substrate was not deterministic at L{level}"
            );
        }
    }

    fn regional_pack(
        kind: LandcoverKind,
        stable_identity: u64,
        mode: RegionalLandcoverMode,
    ) -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.regional_landcover_mode = mode;
        pack.regional_landcover.push(RegionalLandcoverRegion {
            location_rad: None,
            rings: square(3_000.0),
            kind,
            source_id: Some(stable_identity),
            stable_identity,
        });
        pack
    }

    #[test]
    fn regional_landcover_modes_preserve_farmland_and_gate_only_new_fallbacks() {
        let (cell, frame) = test_cell(17);
        let render = |kind, mode| {
            let pack = regional_pack(kind, 41, mode);
            build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None)
        };

        for mode in [
            RegionalLandcoverMode::ExistingFarmlandOnly,
            RegionalLandcoverMode::SelectiveRegionalTerrain,
        ] {
            let farmland = render(LandcoverKind::Farmland, mode);
            let center = farmland.cols[N / 2][N / 2];
            assert_eq!(center.ground, Some(GroundKind::Green));
            assert_eq!(center.flora.expect("farmland flora").seed, 41);
        }

        assert_eq!(
            render(
                LandcoverKind::Forest,
                RegionalLandcoverMode::ExistingFarmlandOnly,
            )
            .cols[N / 2][N / 2]
                .ground,
            None
        );
        assert_eq!(
            render(
                LandcoverKind::Forest,
                RegionalLandcoverMode::SelectiveRegionalTerrain,
            )
            .cols[N / 2][N / 2]
                .ground,
            Some(GroundKind::RegionalForest)
        );
    }

    #[test]
    fn settlement_context_vetoes_only_inferred_regional_farmland() {
        for (lat, lon, urban) in [
            (18.5207_f64, 73.8524_f64, true),
            (35.6762, 139.6503, true),
            (-13.8333, -171.7667, true),
            (48.5, 11.0, false),
            (42.0, -93.0, false),
        ] {
            let cell = CellId::from_dir(
                vearth_core::ellipsoid::geodetic_to_ecef(lat.to_radians(), lon.to_radians(), 0.0)
                    .normalize(),
                17,
            );
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let center = frame.column_latlon(0.5, 0.5);
            assert_eq!(
                matches!(
                    SettlementAtlas::bundled().class_at(center.0, center.1),
                    SettlementClass::City | SettlementClass::Town | SettlementClass::Suburban
                ),
                urban,
                "fixture must exercise intended settlement class at {lat}, {lon}"
            );
            for mode in [
                RegionalLandcoverMode::ExistingFarmlandOnly,
                RegionalLandcoverMode::SelectiveRegionalTerrain,
            ] {
                let mut pack = regional_pack(LandcoverKind::Farmland, 41, mode);
                let inferred =
                    build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
                assert_eq!(inferred.cols[N / 2][N / 2].flora.is_none(), urban);
                if urban {
                    assert_eq!(inferred.cols[N / 2][N / 2].ground, None);
                }
                pack.flora.push(vearth_data::vectors::FloraRegion {
                    location_rad: None,
                    rings: square(3_000.0),
                    kind: FloraRegionKind::Farmland,
                    seed: 88,
                });
                let mapped = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
                let stamp = mapped.cols[N / 2][N / 2]
                    .flora
                    .expect("mapped field is authoritative");
                assert_eq!(stamp.seed, 88);
                assert_eq!(stamp.kind, FloraRegionKind::Farmland);
            }
        }
    }

    #[test]
    fn regional_landcover_mapping_overlap_and_identity_are_deterministic() {
        let mappings = [
            (LandcoverKind::Barren, GroundKind::RegionalBarren),
            (LandcoverKind::Forest, GroundKind::RegionalForest),
            (LandcoverKind::Glacier, GroundKind::RegionalGlacier),
            (LandcoverKind::Grassland, GroundKind::RegionalGrassland),
            (LandcoverKind::Scrub, GroundKind::RegionalScrub),
            (LandcoverKind::UrbanArea, GroundKind::RegionalUrban),
        ];
        let (cell, frame) = test_cell(17);
        for (source, ground) in mappings {
            let pack = regional_pack(source, 12, RegionalLandcoverMode::SelectiveRegionalTerrain);
            let overlay = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
            assert_eq!(overlay.cols[N / 2][N / 2].ground, Some(ground));
        }

        let all = [
            LandcoverKind::Grassland,
            LandcoverKind::Scrub,
            LandcoverKind::Farmland,
            LandcoverKind::Forest,
            LandcoverKind::UrbanArea,
            LandcoverKind::Barren,
            LandcoverKind::Glacier,
        ];
        let render = |reverse: bool| {
            let mut pack = FeaturePack::empty();
            let mut ordered = all.to_vec();
            if reverse {
                ordered.reverse();
            }
            for (index, kind) in ordered.into_iter().enumerate() {
                pack.regional_landcover.push(RegionalLandcoverRegion {
                    location_rad: None,
                    rings: square(3_000.0),
                    kind,
                    source_id: Some(index as u64 + 1),
                    stable_identity: index as u64 + 1,
                });
            }
            build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None)
        };
        assert_eq!(
            render(false).cols[N / 2][N / 2].ground,
            Some(GroundKind::RegionalGlacier)
        );
        assert_eq!(
            render(true).cols[N / 2][N / 2].ground,
            Some(GroundKind::RegionalGlacier)
        );

        let mut tied = regional_pack(
            LandcoverKind::Farmland,
            9,
            RegionalLandcoverMode::SelectiveRegionalTerrain,
        );
        tied.regional_landcover.push(RegionalLandcoverRegion {
            location_rad: None,
            rings: square(3_000.0),
            kind: LandcoverKind::Farmland,
            source_id: Some(2),
            stable_identity: 2,
        });
        let tied = build_overlay(cell, &frame, &tied, &|_, _| 50.0, 0, None, None, None);
        assert_eq!(tied.cols[N / 2][N / 2].flora.unwrap().seed, 2);
    }

    #[test]
    fn every_detailed_land_authority_replaces_regional_ground_and_farmland() {
        let (cell, frame) = test_cell(17);
        for regional in [
            LandcoverKind::Barren,
            LandcoverKind::Farmland,
            LandcoverKind::Forest,
            LandcoverKind::Glacier,
            LandcoverKind::Grassland,
            LandcoverKind::Scrub,
            LandcoverKind::UrbanArea,
        ] {
            for detailed in [
                GroundKind::BareRock,
                GroundKind::RunwayArea,
                GroundKind::Paved,
            ] {
                let mut pack = regional_pack(
                    regional,
                    17,
                    RegionalLandcoverMode::SelectiveRegionalTerrain,
                );
                pack.ground.push((detailed, square(3_000.0)));
                let overlay = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
                assert_eq!(overlay.cols[N / 2][N / 2].ground, Some(detailed));
                assert_eq!(overlay.cols[N / 2][N / 2].flora, None);
            }

            let mut forest = regional_pack(
                regional,
                17,
                RegionalLandcoverMode::SelectiveRegionalTerrain,
            );
            forest.forest.push(square(3_000.0));
            let overlay = build_overlay(cell, &frame, &forest, &|_, _| 50.0, 0, None, None, None);
            assert_eq!(overlay.cols[N / 2][N / 2].ground, Some(GroundKind::Forest));
            assert_eq!(overlay.cols[N / 2][N / 2].flora, None);

            let mut flora = regional_pack(
                regional,
                17,
                RegionalLandcoverMode::SelectiveRegionalTerrain,
            );
            flora.flora.push(vearth_data::vectors::FloraRegion {
                location_rad: None,
                rings: square(3_000.0),
                kind: FloraRegionKind::Garden,
                seed: 88,
            });
            let overlay = build_overlay(cell, &frame, &flora, &|_, _| 50.0, 0, None, None, None);
            let stamp = overlay.cols[N / 2][N / 2].flora.expect("detailed flora");
            assert_eq!(stamp.kind, FloraRegionKind::Garden);
            assert_eq!(stamp.seed, 88);
        }
    }

    #[test]
    fn near_tree_colour_scale_preserves_canonical_identity_and_world_variation() {
        for profile in [
            TreeProfileId::SugarMaple,
            TreeProfileId::EnglishOak,
            TreeProfileId::NorwaySpruce,
        ] {
            assert!(
                (near_tree_color_scale(profile, near_tree_seed(profile)) - 1.0).abs()
                    < f32::EPSILON
            );
            let scales = (0..64)
                .map(|seed| near_tree_color_scale(profile, splitmix(seed)))
                .collect::<Vec<_>>();
            assert!(scales.iter().all(|scale| (0.72..=1.38).contains(scale)));
            assert!(scales.iter().any(|scale| *scale < 0.98));
            assert!(scales.iter().any(|scale| *scale > 1.02));
        }
    }

    #[test]
    fn tree_placement_is_lattice_stable_across_levels() {
        // The same region generated at L18 and L19 must agree on tree
        // positions: collect trunk world columns at both levels.
        let n19 = 1u32 << 19;
        let base = CellId {
            face: Face(0),
            level: 19,
            i: n19 / 2,
            j: n19 / 2,
        };
        let parent = base.parent().unwrap();
        let mut pack = FeaturePack::empty();
        pack.forest.push(square(400.0));

        let tree_ids = |cell: CellId| -> std::collections::HashSet<_> {
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let ov = build_overlay(cell, &frame, &pack, &|_, _| 10.0, 0, None, None, None);
            // Use the stable ownership record rather than clipped render
            // spans: wide tropical crowns legitimately overhang a cell by
            // different amounts at parent and child resolutions.
            ov.tree_instances
                .iter()
                .map(|tree| (tree.profile, tree.seed))
                .collect::<std::collections::HashSet<_>>()
        };
        // Parent covers four children. Their owned candidate identities,
        // including selected profiles, must partition the parent exactly.
        let parent_ids = tree_ids(parent);
        let child_ids: std::collections::HashSet<_> = parent
            .children()
            .iter()
            .flat_map(|&child| tree_ids(child))
            .collect();
        assert_eq!(parent_ids, child_ids);
    }

    #[test]
    fn fractional_tree_ground_changes_root_without_reselecting_the_tree() {
        let (cell, frame) = test_cell(19);
        let mut pack = FeaturePack::empty();
        pack.exact_trees.push(ExactTree {
            point: [0.0, 0.0],
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(91),
            seed: 73,
        });
        let original = build_overlay(cell, &frame, &pack, &|_, _| 100.0, 0, None, None, None);
        let adjusted = build_overlay_with_tree_ground(
            cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
            Some(&|_, _| Some(106.5)),
        );
        assert_eq!(original.tree_instances.len(), 1);
        assert_eq!(adjusted.tree_instances.len(), 1);
        let mut same_tree = adjusted.tree_instances[0].clone();
        assert_eq!(same_tree.elevation_m, 106.5);
        same_tree.elevation_m = original.tree_instances[0].elevation_m;
        assert_eq!(same_tree, original.tree_instances[0]);
        for sample in [None, Some(f32::NAN)] {
            let fallback = build_overlay_with_tree_ground(
                cell,
                &frame,
                &pack,
                &|_, _| 100.0,
                0,
                None,
                None,
                None,
                Some(&|_, _| sample),
            );
            assert_eq!(fallback.tree_instances, original.tree_instances);
        }
    }

    #[test]
    fn exact_tree_poi_uses_existing_tree_path_and_suppresses_nearby_inference() {
        let (cell, frame) = test_cell(19);
        let point = en_from_cols(&frame, [N as f32 * 0.5, N as f32 * 0.5]);
        let mut pack = FeaturePack::empty();
        pack.ground.push((GroundKind::Green, square(80.0)));
        pack.exact_trees.push(ExactTree {
            point,
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(77),
            seed: 0x7472_6565,
        });
        // Buffered copies with the same source identity must not duplicate the
        // physical tree or its near-field instance.
        pack.exact_trees.push(ExactTree {
            point: [point[0] + 0.01, point[1]],
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(77),
            seed: 0x7472_6565,
        });

        let overlay = build_overlay(cell, &frame, &pack, &|_, _| 42.0, 0, None, None, None);
        let exact = overlay
            .tree_instances
            .iter()
            .find(|tree| tree.seed == 0x7472_6565)
            .expect("exact tree instance");
        assert_eq!(exact.elevation_m, 42.0);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let step_m = GridProj::of(&frame, lat, lon).step_m as f32;
        assert!(overlay.tree_instances.iter().all(|tree| {
            tree.seed == exact.seed
                || (tree.cell_x - exact.cell_x).hypot(tree.cell_z - exact.cell_z) * step_m >= 6.0
        }));

        pack.water.push(Water::inland(square(4.0)));
        let blocked = build_overlay(cell, &frame, &pack, &|_, _| 42.0, 0, None, None, None);
        assert!(
            blocked
                .tree_instances
                .iter()
                .all(|tree| tree.seed != 0x7472_6565),
            "existing water authority must suppress an exact tree"
        );

        let off_cell_point = en_from_cols(&frame, [-1.0, N as f32 * 0.5]);
        let translated_square = |centre: [f32; 2], half: f32| {
            vec![vec![
                [centre[0] - half, centre[1] - half],
                [centre[0] + half, centre[1] - half],
                [centre[0] + half, centre[1] + half],
                [centre[0] - half, centre[1] + half],
            ]]
        };
        let mut margin_pack = FeaturePack::empty();
        margin_pack.exact_trees.push(ExactTree {
            point: off_cell_point,
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(88),
            seed: 0x6d61_7267_696e,
        });
        margin_pack
            .water
            .push(Water::inland(translated_square(off_cell_point, 2.0)));
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = GridProj::of(&frame, lat, lon);
        let mut margin_overlay = CellOverlay::empty();
        let admitted = place_exact_trees(
            &frame,
            &mut margin_overlay,
            &margin_pack,
            &projection,
            &|_, _| 42.0,
            None,
            projection.step_m as f32,
            0,
        );
        assert!(admitted.is_empty());
        assert!(margin_overlay.tree_instances.is_empty());

        margin_pack.water.clear();
        margin_pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: translated_square(off_cell_point, 2.0),
            height_m: 8.0,
            min_height_m: 0.0,
            seed: 1,
            kind: vearth_data::vectors::BuildingKind::Building,
        });
        assert!(exact_tree_root_blocked(&margin_pack, off_cell_point));
        margin_pack.buildings.clear();
        margin_pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![
                [off_cell_point[0] - 4.0, off_cell_point[1]],
                [off_cell_point[0] + 4.0, off_cell_point[1]],
            ],
            class: RoadClass::Path,
            is_bridge: false,
        });
        assert!(exact_tree_root_blocked(&margin_pack, off_cell_point));
    }

    #[test]
    fn exact_neighbor_root_suppresses_inferred_trees_without_becoming_owned() {
        let (cell, frame) = test_cell(17);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = GridProj::of(&frame, lat, lon);
        let mut pack = FeaturePack::empty();
        pack.forest.push(square(400.0));
        let elevation = |x, z| {
            assert!((0..N as i32).contains(&x) && (0..N as i32).contains(&z));
            50.0
        };
        let original = build_overlay(cell, &frame, &pack, &elevation, 0, None, None, None);
        let (tree, edge) = original
            .tree_instances
            .iter()
            .flat_map(|tree| {
                [
                    tree.cell_x,
                    tree.cell_z,
                    N as f32 - tree.cell_x,
                    N as f32 - tree.cell_z,
                ]
                .into_iter()
                .enumerate()
                .map(move |(edge, distance)| (tree, edge, distance))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(tree, edge, _)| (tree, edge))
            .expect("forest fixture must have a boundary tree");
        let outside = -0.05 / projection.step_m as f32;
        let neighbor = match edge {
            0 => [outside, tree.cell_z],
            1 => [tree.cell_x, outside],
            2 => [N as f32 - outside, tree.cell_z],
            _ => [tree.cell_x, N as f32 - outside],
        };
        let point = projection.to_en(neighbor);
        let neighbor = projection.to_cols(point);
        let expected = original
            .tree_instances
            .iter()
            .filter(|tree| {
                (tree.cell_x - neighbor[0]).hypot(tree.cell_z - neighbor[1])
                    * projection.step_m as f32
                    >= 6.0
            })
            .cloned()
            .collect::<Vec<_>>();
        assert!(expected.len() < original.tree_instances.len());
        pack.exact_trees.push(ExactTree {
            point,
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(8144),
            seed: 0x6e65_6967_6862_6f72,
        });
        let adjusted = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 2, None, None, None);
        assert_eq!(adjusted.tree_instances, expected);
    }

    #[test]
    fn exact_tree_is_lod_stable_and_missing_elevation_falls_back() {
        let (parent, parent_frame) = test_cell(18);
        let target = parent_frame.column_latlon(0.7, 0.3);
        let child = CellId::from_dir(
            vearth_core::ellipsoid::geodetic_to_ecef(target.0, target.1, 0.0).normalize(),
            parent.level + 1,
        );
        assert_eq!(child.parent(), Some(parent));
        let child_frame = ChunkFrame::of(ChunkAddr { cell: child, k: 0 });
        let exact_seed = 0x6c6f_645f_7472_6565;

        let exact_pack = |frame: &ChunkFrame| {
            let mut pack = FeaturePack::empty();
            pack.exact_trees.push(ExactTree {
                point: en_from_latlon(frame, target.0, target.1),
                identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(555),
                seed: exact_seed,
            });
            pack
        };
        let exact_instance = |cell: CellId, frame: &ChunkFrame| {
            let pack = exact_pack(frame);
            let overlay = build_overlay(cell, frame, &pack, &|_, _| 42.0, 0, None, None, None);
            overlay
                .tree_instances
                .into_iter()
                .find(|tree| tree.seed == exact_seed)
                .expect("exact tree owned by containing cell")
        };
        let parent_tree = exact_instance(parent, &parent_frame);
        let child_tree = exact_instance(child, &child_frame);
        assert_eq!(parent_tree.seed, child_tree.seed);
        assert_eq!(parent_tree.profile, child_tree.profile);
        let parent_position =
            vearth_core::ellipsoid::geodetic_to_ecef(parent_tree.lat_rad, parent_tree.lon_rad, 0.0);
        let child_position =
            vearth_core::ellipsoid::geodetic_to_ecef(child_tree.lat_rad, child_tree.lon_rad, 0.0);
        assert!(
            parent_position.distance(child_position) < 0.1,
            "same mapped point drifted across parent/child generation"
        );

        let missing_pack = exact_pack(&child_frame);
        let missing = build_overlay(
            child,
            &child_frame,
            &missing_pack,
            &|_, _| f32::NAN,
            0,
            None,
            None,
            None,
        );
        assert!(missing.tree_instances.is_empty());
    }

    #[test]
    fn trees_grow_in_forest_and_respect_water() {
        let (cell, frame) = test_cell(18);
        let mut pack = FeaturePack::empty();
        pack.forest.push(square(60.0));
        let ov = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
        assert!(
            !ov.tree_instances.is_empty(),
            "forest cell should have trees"
        );
        assert!(
            ov.tree_instances
                .iter()
                .all(|tree| tree.height_m.is_finite() && tree.height_m > 0.0)
        );
        // Same cell fully under water: no trees.
        pack.water.push(Water::inland(square(300.0)));
        let ov2 = build_overlay(
            cell,
            &frame,
            &pack,
            &|_, _| 50.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        );
        assert!(ov2.tree_instances.is_empty(), "no trees in water");
    }

    #[test]
    fn exact_margin_tree_retains_exclusion_without_duplicate_ownership() {
        let (_, frame) = test_cell(19);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = GridProj::of(&frame, lat, lon);
        let mut pack = FeaturePack::empty();
        pack.exact_trees.push(ExactTree {
            point: projection.to_en([-0.2, 16.0]),
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(812),
            seed: 71,
        });
        let mut overlay = CellOverlay::empty();
        let ground_calls = std::cell::Cell::new(0);
        let ground = |_, _| {
            ground_calls.set(ground_calls.get() + 1);
            Some(42.0)
        };
        let admitted = place_exact_trees(
            &frame,
            &mut overlay,
            &pack,
            &projection,
            &|_, _| 42.0,
            Some(&ground),
            projection.step_m as f32,
            2,
        );
        assert_eq!(admitted.len(), 1);
        assert!(overlay.tree_instances.is_empty());
        assert_eq!(
            ground_calls.get(),
            0,
            "neighbor roots need no rendered datum"
        );
    }

    #[test]
    fn rejected_river_and_cliff_roots_leave_no_tree_or_neighbor_exclusion() {
        use vearth_data::protomaps_semantics::SourceFeatureIdentity;
        use vearth_data::vectors::CliffLine;

        let (left, left_frame) = test_cell(19);
        let right = CellId {
            i: left.i + 1,
            ..left
        };
        let right_frame = ChunkFrame::of(ChunkAddr { cell: right, k: 0 });
        let root = left_frame.column_latlon(31.8 / 32.0, 16.5 / 32.0);
        let line = [
            left_frame.column_latlon(31.8 / 32.0, -0.25),
            left_frame.column_latlon(31.8 / 32.0, 1.25),
        ];
        for (cell, frame, offset) in [(left, left_frame, 0), (right, right_frame, 32)] {
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = GridProj::of(&frame, lat, lon);
            let pack_with_root = || {
                let mut pack = FeaturePack::empty();
                pack.exact_trees.push(ExactTree {
                    point: en_from_latlon(&frame, root.0, root.1),
                    identity: SourceFeatureIdentity::Provided(813),
                    seed: 71,
                });
                pack
            };
            let render = |pack: &FeaturePack, slope: f32| {
                let elevation =
                    |x, _| 42.0 + (x + offset - 31) as f32 * projection.step_m as f32 * slope;
                let overlay = build_overlay(
                    cell,
                    &frame,
                    pack,
                    &elevation,
                    WATER_MARGIN_COLS,
                    None,
                    None,
                    None,
                );
                let admitted = place_exact_trees(
                    &frame,
                    &mut CellOverlay::empty(),
                    pack,
                    &projection,
                    &elevation,
                    None,
                    projection.step_m as f32,
                    WATER_MARGIN_COLS,
                );
                (overlay, admitted)
            };
            let (clear, admitted) = render(&pack_with_root(), 0.0);
            assert_eq!(clear.tree_instances.len(), usize::from(offset == 0));
            assert_eq!(admitted.len(), 1);

            let mut river = pack_with_root();
            river.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
            river.watercourses.push(geodetic_watercourse(
                &frame,
                WatercourseKind::River,
                814,
                &line,
            ));
            let (flooded, admitted) = render(&river, 0.0);
            assert!(flooded.tree_instances.is_empty());
            assert!(
                admitted.is_empty(),
                "river-rejected root suppressed nearby trees"
            );
            river.watercourses[0].intermittent = true;
            let (intermittent, admitted) = render(&river, 0.0);
            assert_eq!(intermittent.tree_instances.len(), usize::from(offset == 0));
            assert_eq!(admitted.len(), 1, "nonphysical watercourse blocked a tree");

            let mut cliff = pack_with_root();
            cliff.cliffs.push(CliffLine {
                pts: line
                    .iter()
                    .map(|&(lat, lon)| en_from_latlon(&frame, lat, lon))
                    .collect(),
                identity: SourceFeatureIdentity::Provided(815),
            });
            let (supported, admitted) = render(&cliff, 0.75);
            assert!(supported.tree_instances.is_empty());
            assert!(
                admitted.is_empty(),
                "cliff-rejected root suppressed nearby trees"
            );
            let (unsupported, admitted) = render(&cliff, 0.0);
            assert_eq!(unsupported.tree_instances.len(), usize::from(offset == 0));
            assert_eq!(admitted.len(), 1, "unsupported cliff line blocked a tree");
        }
    }

    #[test]
    fn ordinary_green_remains_sparse_and_deterministic() {
        let (cell, frame) = test_cell(17);
        let mut pack = FeaturePack::empty();
        pack.ground.push((GroundKind::Green, square(3_000.0)));
        let ov = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
        assert_eq!(
            ov.tree_instances.len(),
            2,
            "managed green became a dense forest"
        );
        assert!(
            ov.tree_instances
                .iter()
                .all(|tree| tree.height_m.is_finite() && tree.height_m > 0.0)
        );
        let repeat = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
        assert_eq!(ov.tree_instances, repeat.tree_instances);
        pack.forest.push(square(3_000.0));
        let forest = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
        assert!(forest.tree_instances.len() > ov.tree_instances.len() * 10);
    }

    #[test]
    fn flora_regions_keep_green_surface_but_never_place_managed_trees() {
        let kinds = [
            FloraRegionKind::Farmland,
            FloraRegionKind::Meadow,
            FloraRegionKind::Garden,
            FloraRegionKind::Orchard,
            FloraRegionKind::Vineyard,
        ];
        let (cell, frame) = test_cell(17);
        for (index, kind) in kinds.into_iter().enumerate() {
            let mut pack = FeaturePack::empty();
            pack.flora.push(vearth_data::vectors::FloraRegion {
                location_rad: None,
                rings: square(3_000.0),
                kind,
                seed: index as u64 + 1,
            });
            let ov = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
            assert_eq!(ov.cols[N / 2][N / 2].ground, Some(GroundKind::Green));
            assert_eq!(
                ov.cols[N / 2][N / 2].flora,
                Some(FloraStamp {
                    location_rad: None,
                    kind,
                    seed: index as u64 + 1,
                    curated_style: 0,
                    surface: select_field_surface(kind, index as u64 + 1),
                })
            );
            assert!(
                ov.tree_instances.is_empty(),
                "{kind:?} received ordinary managed trees"
            );
        }
    }

    #[test]
    fn regional_farmland_matches_the_pre_l3_flora_path_and_regional_wilds_add_no_trees() {
        for level in [GROUND_MIN_LEVEL, 12, 15, 17, 19] {
            let (cell, frame) = test_cell(level);
            let regional = regional_pack(
                LandcoverKind::Farmland,
                73,
                RegionalLandcoverMode::SelectiveRegionalTerrain,
            );
            let mut legacy = FeaturePack::empty();
            legacy.flora.push(vearth_data::vectors::FloraRegion {
                location_rad: None,
                rings: square(3_000.0),
                kind: FloraRegionKind::Farmland,
                seed: 73,
            });
            let regional =
                build_overlay(cell, &frame, &regional, &|_, _| 50.0, 0, None, None, None);
            let legacy = build_overlay(cell, &frame, &legacy, &|_, _| 50.0, 0, None, None, None);
            for (regional, legacy) in regional
                .cols
                .iter()
                .flatten()
                .zip(legacy.cols.iter().flatten())
            {
                assert_eq!(regional.ground, legacy.ground);
                assert_eq!(regional.flora, legacy.flora);
            }

            for kind in [
                LandcoverKind::Forest,
                LandcoverKind::Grassland,
                LandcoverKind::Scrub,
            ] {
                let pack = regional_pack(kind, 73, RegionalLandcoverMode::SelectiveRegionalTerrain);
                let first = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
                let second = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
                assert!(first.tree_instances.is_empty());
                assert_eq!(
                    first.cols[N / 2][N / 2].ground,
                    second.cols[N / 2][N / 2].ground
                );
            }
        }
    }

    #[test]
    fn forest_ground_identity_persists_when_tree_geometry_appears() {
        let mut pack = FeaturePack::empty();
        pack.forest.push(square(3000.0));

        for level in 16..=19 {
            let (cell, frame) = test_cell(level);
            let ov = build_overlay(cell, &frame, &pack, &|_, _| 50.0, 0, None, None, None);
            assert_eq!(
                ov.cols[N / 2][N / 2].ground,
                Some(GroundKind::Forest),
                "L{level} changed the forest ground identity"
            );
            assert_eq!(
                ov.tree_instances.is_empty(),
                level < TREES_MIN_LEVEL,
                "L{level} tree gate disagrees with the ground-continuity fixture"
            );
        }
    }

    #[test]
    fn levels_gate_features() {
        let (cell, frame) = test_cell(12);
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: square(2000.0),
            height_m: 30.0,
            min_height_m: 0.0,
            seed: 1,
            kind: BuildingKind::Building,
        });
        pack.water.push(Water::inland(square(400.0)));
        pack.forest.push(square(3000.0));
        let ov = build_overlay(
            cell,
            &frame,
            &pack,
            &|_, _| 5.0,
            WATER_MARGIN_COLS,
            None,
            None,
            None,
        );
        // L12: every polygon feature is live (no transition rings between
        // the carpet and buildings/roads)...
        assert!(ov.cols[N / 2][N / 2].water);
        assert!(ov.cols[N / 4][N / 4].building.is_some());
        // ...but trees stay gated: forest renders as ground tint instead.
        assert!(ov.tree_instances.is_empty());
        assert_eq!(ov.cols[2][2].ground, Some(GroundKind::Forest));
        // At the tree gate actual trees join the same persistent forest
        // ground treatment (fresh pack: the water square above would flood a
        // whole 76 m L17 cell).
        let mut forest_pack = FeaturePack::empty();
        forest_pack.forest.push(square(3000.0));
        let (cell17, frame17) = test_cell(17);
        let ov17 = build_overlay(
            cell17,
            &frame17,
            &forest_pack,
            &|_, _| 5.0,
            0,
            None,
            None,
            None,
        );
        assert!(!ov17.tree_instances.is_empty());
        assert_eq!(ov17.cols[2][2].ground, Some(GroundKind::Forest));
    }
    #[test]
    fn landmark_root_suppression_precedes_tree_ownership_and_neighbor_exclusion() {
        let landmark = crate::landmarks::LANDMARKS
            .iter()
            .find(|lm| lm.name == "eiffel-tower")
            .unwrap();
        let (lat, lon) = (landmark.lat_deg.to_radians(), landmark.lon_deg.to_radians());
        let owner = CellId::from_dir(vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0), 19);
        for cell in [
            owner,
            CellId {
                i: owner.i + 1,
                ..owner
            },
        ] {
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let (a, b) = frame.column_latlon(0.5, 0.5);
            let projection = GridProj::of(&frame, a, b);
            let mut pack = FeaturePack::empty();
            pack.exact_trees.push(ExactTree {
                point: en_from_latlon(&frame, lat, lon),
                identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(9021),
                seed: 9021,
            });
            let mut overlay = CellOverlay::empty();
            let admitted = place_exact_trees(
                &frame,
                &mut overlay,
                &pack,
                &projection,
                &|_, _| 40.0,
                None,
                projection.step_m as f32,
                128,
            );
            assert!(admitted.is_empty());
            assert!(overlay.tree_instances.is_empty());
        }
    }
}
