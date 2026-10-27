//! Curated indexed landmark meshes in their canonical geographic frames.
//! Terrain cleanup and source suppression share each model's placement datum.

use std::sync::{OnceLock, RwLock};
use vearth_core::cell::CellId;
use vearth_core::ellipsoid::{self, MEAN_RADIUS};
use vearth_core::frame::ChunkFrame;

mod assets;
mod city_generated;
mod curated;
mod heritage;
mod heritage_generated;
mod island_generated;
mod models;
mod monument_generated;
mod skyscraper_generated;
mod surface;
pub use assets::{level_path, split_levels};
pub use models::LANDMARKS;
pub use surface::{LandmarkSurfaceData, canonical_mesh, generate};

/// Native landmark-calibration override. Production placement remains the
/// immutable registry below; the calibrator installs one temporary override
/// so generation, suppression, terrain cleanup and LOD locking all see the
/// same edited transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalibrationOverride {
    pub name: &'static str,
    /// Local tangent offset from the registry anchor.
    pub east_m: f64,
    pub north_m: f64,
    /// Added after resolving either a fixed or terrain-backed base.
    pub up_m: f64,
    /// Absolute bearing: degrees clockwise from true north for model +z.
    pub heading_deg: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedPlacement {
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub heading_deg: f64,
    pub base_offset_m: f64,
}

static CALIBRATION_OVERRIDE: OnceLock<RwLock<Option<CalibrationOverride>>> = OnceLock::new();

fn calibration_slot() -> &'static RwLock<Option<CalibrationOverride>> {
    CALIBRATION_OVERRIDE.get_or_init(|| RwLock::new(None))
}

/// Replace the process-local calibration transform. This is intentionally a
/// tiny dev-tool hook rather than persistent product state.
pub fn set_calibration_override(value: Option<CalibrationOverride>) {
    *calibration_slot()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
}

pub fn calibration_override() -> Option<CalibrationOverride> {
    *calibration_slot()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Resolve an editor offset to the exact lat/lon/heading/base values used by
/// generation and emitted by the clipboard payload.
pub fn resolve_calibration(value: CalibrationOverride) -> ResolvedPlacement {
    let lm = LANDMARKS
        .iter()
        .find(|lm| lm.name == value.name)
        .expect("calibration landmark must exist in LANDMARKS");
    let lat = lm.lat_deg.to_radians();
    let lon = lm.lon_deg.to_radians();
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    let anchor = ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
    let east = glam::DVec3::new(-sin_lon, cos_lon, 0.0);
    let north = glam::DVec3::new(-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat);
    let (lat, lon, _) =
        ellipsoid::ecef_to_geodetic(anchor + east * value.east_m + north * value.north_m);
    ResolvedPlacement {
        lat_deg: lat.to_degrees(),
        lon_deg: lon.to_degrees(),
        heading_deg: value.heading_deg.rem_euclid(360.0),
        base_offset_m: lm.base.offset_m() + value.up_m,
    }
}

fn effective_placement(lm: &Landmark) -> ResolvedPlacement {
    placement_with_override(lm, calibration_override())
}

fn placement_with_override(
    lm: &Landmark,
    override_: Option<CalibrationOverride>,
) -> ResolvedPlacement {
    match override_.filter(|value| value.name == lm.name) {
        Some(value) => resolve_calibration(value),
        None => ResolvedPlacement {
            lat_deg: lm.lat_deg,
            lon_deg: lm.lon_deg,
            heading_deg: lm.heading_deg.rem_euclid(360.0),
            base_offset_m: lm.base.offset_m(),
        },
    }
}

/// How the model's y = 0 plane finds its geodetic altitude.
#[derive(Clone, Copy)]
pub enum Base {
    /// Curated absolute altitude (meters above the geoid) — right for
    /// coastal/waterside landmarks where the base is known exactly.
    Fixed(f32),
    /// Sample the terrain at the anchor for hilltop and inland landmarks.
    /// The result is deterministic per level for identical tiles and sampling
    /// hints.
    Terrain,
    /// Sample terrain exactly as above, then lift the model by this many
    /// metres. Used for calibrated plinths whose authored y=0 sits slightly
    /// below or above the corrected architectural grade.
    TerrainOffset(f32),
}

impl Base {
    fn offset_m(self) -> f64 {
        match self {
            Self::TerrainOffset(offset_m) => f64::from(offset_m),
            Self::Fixed(_) | Self::Terrain => 0.0,
        }
    }
}

/// What the landmark's footprint suppresses in the Tier-0 overlay. The OSM
/// rendition of the same structure (extruded blob, deck-on-water) must not
/// z-fight the curated model.
#[derive(Clone, Copy, Default)]
pub struct Suppress {
    pub buildings: bool,
    pub roads: bool,
    pub trees: bool,
}

#[derive(Clone, Copy)]
pub struct Landmark {
    pub name: &'static str,
    pub lat_deg: f64,
    pub lon_deg: f64,
    /// Bearing (degrees, clockwise from true north) that the model's +z
    /// axis points toward.
    pub heading_deg: f64,
    pub base: Base,
    /// Model-space AABB in meters (x east, y up, z north at heading 0).
    pub bbox_min: [f64; 3],
    pub bbox_max: [f64; 3],
    pub suppress: Suppress,
    /// Building suppression footprint override (xz min/max): when the
    /// sampling bbox is deliberately wide (a bridge corridor covering the
    /// full deck for road suppression), whole-feature building kill can
    /// use a narrower band so real blocks beside the approaches survive.
    /// Ordinary landmarks infer this from their terrain-cleanup rectangles
    /// when present, otherwise from the full model bbox.
    pub suppress_bbox_xz: Option<([f64; 2], [f64; 2])>,
}

/// A DEM-cleanup region for a landmark whose structure is present in the
/// elevation raster itself. `half_extent` is the level architectural pad;
/// `feather_m` blends its edge back into untouched terrain, and
/// `sample_clearance_m` keeps the reference samples outside both the model
/// and the contaminated DEM footprint.
#[derive(Clone, Copy, Debug)]
pub struct TerrainFlatten {
    pub center_xz: [f64; 2],
    pub half_extent: [f64; 2],
    pub feather_m: f64,
    pub sample_clearance_m: f64,
    /// Audited bare-earth height when the surrounding DEM is also contaminated.
    pub fixed_base_geo: Option<f32>,
}

impl TerrainFlatten {
    fn sample_offsets(self) -> [[f64; 2]; 8] {
        let hx = self.half_extent[0] + self.feather_m + self.sample_clearance_m;
        let hz = self.half_extent[1] + self.feather_m + self.sample_clearance_m;
        let [cx, cz] = self.center_xz;
        [
            [cx - hx, cz - hz],
            [cx, cz - hz],
            [cx + hx, cz - hz],
            [cx + hx, cz],
            [cx + hx, cz + hz],
            [cx, cz + hz],
            [cx - hx, cz + hz],
            [cx - hx, cz],
        ]
    }
}

const GIZA_TERRAIN_FLATTEN: &[TerrainFlatten] = &[
    TerrainFlatten {
        center_xz: [0.0, 0.0],
        half_extent: [125.0, 125.0],
        feather_m: 40.0,
        sample_clearance_m: 35.0,
        fixed_base_geo: None,
    },
    TerrainFlatten {
        center_xz: [-345.0, -390.0],
        half_extent: [118.0, 118.0],
        feather_m: 38.0,
        sample_clearance_m: 35.0,
        fixed_base_geo: None,
    },
    TerrainFlatten {
        center_xz: [-780.0, -740.0],
        half_extent: [62.0, 62.0],
        feather_m: 30.0,
        sample_clearance_m: 28.0,
        fixed_base_geo: None,
    },
];

const TOKYO_TOWER_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [0.0, 0.0],
    half_extent: [42.0, 42.0],
    feather_m: 24.0,
    sample_clearance_m: 30.0,
    fixed_base_geo: None,
}];

const WESTMINSTER_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [0.0, 0.0],
    half_extent: [61.0, 145.0],
    feather_m: 18.0,
    sample_clearance_m: 28.0,
    fixed_base_geo: None,
}];

const OPERA_HOUSE_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [0.0, 0.0],
    half_extent: [62.0, 93.0],
    feather_m: 20.0,
    sample_clearance_m: 35.0,
    fixed_base_geo: None,
}];

macro_rules! terrain_flatten {
    ($name:ident, $half_x:expr, $half_z:expr, $feather:expr, $clearance:expr) => {
        const $name: &[TerrainFlatten] = &[TerrainFlatten {
            center_xz: [0.0, 0.0],
            half_extent: [$half_x, $half_z],
            feather_m: $feather,
            sample_clearance_m: $clearance,
            fixed_base_geo: None,
        }];
    };
}

const ANGKOR_WAT_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [-25.7, 0.0],
    half_extent: [128.0, 103.0],
    feather_m: 25.0,
    sample_clearance_m: 30.0,
    fixed_base_geo: None,
}];
terrain_flatten!(BUCKINGHAM_TERRAIN_FLATTEN, 67.0, 56.0, 18.0, 25.0);
terrain_flatten!(BURJ_AL_ARAB_TERRAIN_FLATTEN, 34.0, 46.0, 22.0, 30.0);
terrain_flatten!(BURJ_KHALIFA_TERRAIN_FLATTEN, 65.0, 58.0, 25.0, 35.0);
terrain_flatten!(CN_TOWER_TERRAIN_FLATTEN, 30.0, 26.0, 18.0, 25.0);
terrain_flatten!(GOLDEN_TEMPLE_TERRAIN_FLATTEN, 11.0, 11.0, 12.0, 20.0);
const HIMEJI_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [-66.0, -27.5],
    half_extent: [128.0, 72.0],
    feather_m: 25.0,
    sample_clearance_m: 30.0,
    fixed_base_geo: None,
}];
const KREMLIN_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [-54.5, 44.0],
    half_extent: [323.5, 359.0],
    feather_m: 45.0,
    sample_clearance_m: 45.0,
    fixed_base_geo: None,
}];
terrain_flatten!(PISA_TERRAIN_FLATTEN, 10.0, 11.0, 12.0, 20.0);
terrain_flatten!(LOUVRE_TERRAIN_FLATTEN, 19.0, 19.0, 14.0, 22.0);
terrain_flatten!(MARINA_BAY_SANDS_TERRAIN_FLATTEN, 34.0, 171.0, 25.0, 35.0);
terrain_flatten!(NOTRE_DAME_TERRAIN_FLATTEN, 66.0, 26.0, 20.0, 28.0);
terrain_flatten!(NYHAVN_TERRAIN_FLATTEN, 79.0, 12.0, 14.0, 22.0);
terrain_flatten!(PARTHENON_TERRAIN_FLATTEN, 37.0, 18.0, 16.0, 24.0);
terrain_flatten!(ST_BASILS_TERRAIN_FLATTEN, 26.0, 23.0, 18.0, 25.0);
const TAIPEI_101_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    // Cover the complete mapped Taipei 101 building relation, not only the
    // tower shaft, so its podium-side building parts cannot leak through.
    center_xz: [8.0, -3.0],
    half_extent: [78.0, 75.0],
    feather_m: 22.0,
    sample_clearance_m: 35.0,
    fixed_base_geo: None,
}];
terrain_flatten!(WHITE_HOUSE_TERRAIN_FLATTEN, 95.0, 36.0, 25.0, 50.0);
terrain_flatten!(CHRYSLER_TERRAIN_FLATTEN, 34.0, 34.0, 18.0, 25.0);
const FLATIRON_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [3.0, 11.0],
    half_extent: [17.0, 35.0],
    feather_m: 14.0,
    sample_clearance_m: 22.0,
    fixed_base_geo: None,
}];
terrain_flatten!(MERDEKA_118_TERRAIN_FLATTEN, 42.0, 50.0, 20.0, 30.0);
terrain_flatten!(ONE_WTC_TERRAIN_FLATTEN, 34.0, 34.0, 20.0, 30.0);
terrain_flatten!(PETRONAS_TERRAIN_FLATTEN, 27.0, 95.0, 25.0, 35.0);
const STEINWAY_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [-0.4, -2.0],
    half_extent: [17.0, 32.0],
    feather_m: 16.0,
    sample_clearance_m: 22.0,
    fixed_base_geo: None,
}];
terrain_flatten!(GHERKIN_TERRAIN_FLATTEN, 30.0, 30.0, 18.0, 25.0);
const SHARD_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [2.0, 0.4],
    half_extent: [36.0, 29.0],
    feather_m: 18.0,
    sample_clearance_m: 25.0,
    fixed_base_geo: None,
}];
terrain_flatten!(MERLION_TERRAIN_FLATTEN, 2.0, 3.0, 6.0, 10.0);

// The grove's low open ground is 5.28 m in the source DEM; neighboring
// vegetation/building returns reach 32 m, so a perimeter median is not ground.
const SUPERTREES_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [4.5, 4.5],
    half_extent: [86.0, 79.0],
    feather_m: 8.0,
    sample_clearance_m: 0.0,
    fixed_base_geo: Some(5.28),
}];

// Level the pool-and-podium garden at the audited central open-ground datum.
// Raster slope otherwise crosses the authored reflecting pools above their water.
const LOTUS_TERRAIN_FLATTEN: &[TerrainFlatten] = &[TerrainFlatten {
    center_xz: [-1.0, 0.0],
    half_extent: [54.0, 54.5],
    feather_m: 8.0,
    sample_clearance_m: 0.0,
    fixed_base_geo: Some(242.8),
}];

impl Landmark {
    /// DEM regions that contain this landmark's built mass and therefore
    /// cannot be trusted as bare-earth terrain. This remains registry data,
    /// even though only the first affected sites opt in today.
    pub fn terrain_flatten(&self) -> &'static [TerrainFlatten] {
        match self.name {
            "sydney-opera-house" => OPERA_HOUSE_TERRAIN_FLATTEN,
            "giza-necropolis" => GIZA_TERRAIN_FLATTEN,
            "tokyo-tower" => TOKYO_TOWER_TERRAIN_FLATTEN,
            "palace-of-westminster" => WESTMINSTER_TERRAIN_FLATTEN,
            "angkor-wat" => ANGKOR_WAT_TERRAIN_FLATTEN,
            "buckingham-palace" => BUCKINGHAM_TERRAIN_FLATTEN,
            "burj-al-arab" => BURJ_AL_ARAB_TERRAIN_FLATTEN,
            "burj-khalifa" => BURJ_KHALIFA_TERRAIN_FLATTEN,
            "cn-tower" => CN_TOWER_TERRAIN_FLATTEN,
            "golden-temple" => GOLDEN_TEMPLE_TERRAIN_FLATTEN,
            "himeji-castle" => HIMEJI_TERRAIN_FLATTEN,
            "moscow-kremlin" => KREMLIN_TERRAIN_FLATTEN,
            "leaning-tower-of-pisa" => PISA_TERRAIN_FLATTEN,
            "louvre-pyramid" => LOUVRE_TERRAIN_FLATTEN,
            "marina-bay-sands" => MARINA_BAY_SANDS_TERRAIN_FLATTEN,
            "notre-dame" => NOTRE_DAME_TERRAIN_FLATTEN,
            "nyhavn" => NYHAVN_TERRAIN_FLATTEN,
            "parthenon" => PARTHENON_TERRAIN_FLATTEN,
            "st-basils-cathedral" => ST_BASILS_TERRAIN_FLATTEN,
            "taipei-101" => TAIPEI_101_TERRAIN_FLATTEN,
            "white-house" => WHITE_HOUSE_TERRAIN_FLATTEN,
            "chrysler-building" => CHRYSLER_TERRAIN_FLATTEN,
            "flatiron-building" => FLATIRON_TERRAIN_FLATTEN,
            "merdeka-118" => MERDEKA_118_TERRAIN_FLATTEN,
            "one-world-trade-center" => ONE_WTC_TERRAIN_FLATTEN,
            "petronas-towers" => PETRONAS_TERRAIN_FLATTEN,
            "steinway-tower" => STEINWAY_TERRAIN_FLATTEN,
            "the-gherkin" => GHERKIN_TERRAIN_FLATTEN,
            "the-shard" => SHARD_TERRAIN_FLATTEN,
            "singapore-merlion" => MERLION_TERRAIN_FLATTEN,
            "singapore-supertrees" => SUPERTREES_TERRAIN_FLATTEN,
            "lotus-temple" => LOTUS_TERRAIN_FLATTEN,
            _ => {
                let patches = city_generated::terrain_flatten(self.name);
                if patches.is_empty() {
                    curated::terrain_flatten(self.name)
                } else {
                    patches
                }
            }
        }
    }

    fn bbox_radius(&self) -> f64 {
        let (suppress_min, suppress_max) = self.suppress_bbox_xz.unwrap_or((
            [self.bbox_min[0], self.bbox_min[2]],
            [self.bbox_max[0], self.bbox_max[2]],
        ));
        let dx = self.bbox_max[0]
            .max(-self.bbox_min[0])
            .max(suppress_max[0])
            .max(-suppress_min[0]);
        let dz = self.bbox_max[2]
            .max(-self.bbox_min[2])
            .max(suppress_max[1])
            .max(-suppress_min[1]);
        (dx * dx + dz * dz).sqrt()
    }
}

/// A landmark footprint exposed to the streaming controller. Terrain cells
/// intersecting one of these regions use one presentation LOD for the whole
/// model, so long structures do not select different detail levels in adjacent
/// resolutions along their length.
#[derive(Clone, Copy, Debug)]
pub struct LodRegion {
    pub name: &'static str,
    pub lat_rad: f64,
    pub lon_rad: f64,
    pub heading_rad: f64,
    pub bbox_min_xz: [f64; 2],
    pub bbox_max_xz: [f64; 2],
    pub height_m: f64,
    pub radius_m: f64,
    /// Extra local quadtree levels permitted beyond the ordinary world
    /// ladder. Reserved for physically small authored landmarks whose
    /// silhouette cannot read at the standard 0.6 m finest pitch.
    pub detail_level_bias: u8,
}

/// Stable, allocation-free view of every curated landmark footprint.
pub fn lod_regions() -> impl Iterator<Item = LodRegion> {
    LANDMARKS.iter().map(|lm| {
        let placement = effective_placement(lm);
        LodRegion {
            name: lm.name,
            lat_rad: placement.lat_deg.to_radians(),
            lon_rad: placement.lon_deg.to_radians(),
            heading_rad: placement.heading_deg.to_radians(),
            bbox_min_xz: [lm.bbox_min[0], lm.bbox_min[2]],
            bbox_max_xz: [lm.bbox_max[0], lm.bbox_max[2]],
            height_m: lm.bbox_max[1] + placement.base_offset_m.max(0.0),
            radius_m: lm.bbox_radius(),
            detail_level_bias: u8::from(lm.name == "singapore-merlion"),
        }
    })
}

/// Landmark streaming begins when terrain cells reach L13.
pub const LANDMARK_MIN_LEVEL: u8 = 13;

/// Calibration-only marker carried in the otherwise-unused high material
/// bit. Geometry preserves it in `SurfaceVertex::face_ao`; the renderer strips
/// it before atlas lookup and applies the landmark's live transform. Normal
/// production generation never sets this bit.
pub const CALIBRATION_MATERIAL_FLAG: u8 = 0x80;

/// A landmark active for the generating cell: the anchor's EN offset from
/// the cell center plus the heading rotation, so columns map into model
/// space with two multiplies.
pub struct ActiveLandmark {
    pub lm: &'static Landmark,
    /// Anchor position in EN meters relative to the cell center.
    anchor_e: f64,
    anchor_n: f64,
    sin_h: f64,
    cos_h: f64,
    /// Canonical ECEF tangent frame at the landmark anchor. Model sampling
    /// uses this shared frame rather than rebuilding an equirectangular EN
    /// approximation around each terrain cell.
    anchor_ecef: glam::DVec3,
    anchor_east: glam::DVec3,
    anchor_north: glam::DVec3,
    anchor_up: glam::DVec3,
    /// Geodetic altitude of model y = 0.
    pub base_geo: f32,
    /// Calibration-only lift applied to the model after terrain cleanup.
    /// Kept separate because flatten targets describe the ground, not the
    /// edited model base.
    calibration_base_offset: f32,
    /// Clean perimeter-derived terrain level for every flatten patch. The
    /// same values drive both the terrain correction and the model base.
    flatten_bases: Vec<f32>,
}

impl ActiveLandmark {
    fn new(
        lm: &'static Landmark,
        placement: ResolvedPlacement,
        anchor_e: f64,
        anchor_n: f64,
        terrain_at: &dyn Fn(f64, f64) -> f32,
    ) -> Self {
        let lat = placement.lat_deg.to_radians();
        let lon = placement.lon_deg.to_radians();
        let h = placement.heading_deg.to_radians();
        let (sin_lat, cos_lat) = lat.sin_cos();
        let (sin_lon, cos_lon) = lon.sin_cos();
        let flatten_bases: Vec<f32> = lm
            .terrain_flatten()
            .iter()
            .map(|patch| {
                patch.fixed_base_geo.unwrap_or_else(|| {
                    median(
                        patch
                            .sample_offsets()
                            .into_iter()
                            .map(|[mx, mz]| {
                                let (a, b) =
                                    model_offset_latlon(placement.heading_deg, lat, lon, mx, mz);
                                terrain_at(a, b)
                            })
                            .collect(),
                    )
                })
            })
            .collect();
        let base_geo = match lm.base {
            Base::Fixed(a) => a,
            Base::Terrain | Base::TerrainOffset(_) if flatten_bases.is_empty() => {
                terrain_at(lat, lon)
            }
            Base::Terrain | Base::TerrainOffset(_) => median(flatten_bases.clone()),
        } + placement.base_offset_m as f32;
        Self {
            lm,
            anchor_e,
            anchor_n,
            sin_h: h.sin(),
            cos_h: h.cos(),
            anchor_ecef: ellipsoid::geodetic_to_ecef(lat, lon, 0.0),
            anchor_east: glam::DVec3::new(-sin_lon, cos_lon, 0.0),
            anchor_north: glam::DVec3::new(-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat),
            anchor_up: glam::DVec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat),
            base_geo,
            calibration_base_offset: placement.base_offset_m as f32,
            flatten_bases,
        }
    }

    /// Exact horizontal replacement footprint in the receiving cell's frame.
    pub(crate) fn road_exclusions(
        &self,
        frame: &vearth_core::frame::ChunkFrame,
    ) -> Vec<Vec<glam::DVec2>> {
        if !self.lm.suppress.roads {
            return Vec::new();
        }
        let lo = self.lm.bbox_min;
        let hi = self.lm.bbox_max;
        let rectangle = [
            [lo[0], lo[2]],
            [hi[0], lo[2]],
            [hi[0], hi[2]],
            [lo[0], hi[2]],
        ];
        let fallback: &[&[[f64; 2]]] = &[&rectangle];
        let rings = city_generated::road_exclusion_polygons(self.lm.name)
            .or_else(|| curated::road_exclusion_polygons(self.lm.name))
            .unwrap_or(fallback);
        rings
            .iter()
            .map(|ring| {
                let mut polygon: Vec<_> = ring
                    .iter()
                    .map(|&[x, z]| {
                        let east = x * self.cos_h + z * self.sin_h;
                        let north = -x * self.sin_h + z * self.cos_h;
                        let p = frame.ecef_to_local(
                            self.anchor_ecef + self.anchor_east * east + self.anchor_north * north,
                        );
                        glam::DVec2::new(p.x, p.z)
                    })
                    .collect();
                if (polygon[1] - polygon[0]).perp_dot(polygon[2] - polygon[0]) < 0.0 {
                    polygon.reverse();
                }
                polygon
            })
            .collect()
    }

    fn flatten_patch(&self, mx: f64, mz: f64) -> Option<(usize, f64)> {
        self.lm
            .terrain_flatten()
            .iter()
            .enumerate()
            .filter_map(|(index, patch)| {
                let dx = (mx - patch.center_xz[0]).abs() - patch.half_extent[0];
                let dz = (mz - patch.center_xz[1]).abs() - patch.half_extent[1];
                let outside = dx.max(0.0).max(dz.max(0.0));
                if outside >= patch.feather_m {
                    return None;
                }
                let t = (1.0 - outside / patch.feather_m).clamp(0.0, 1.0);
                let weight = t * t * (3.0 - 2.0 * t);
                Some((index, weight))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Correct the raw DEM inside an opted-in architectural footprint.
    pub fn flattened_terrain(&self, mx: f64, mz: f64, raw: f32) -> Option<(f32, f64)> {
        // Hongyadong descends a real cliff. Cap rooftop-contaminated DEM under
        // its terraces while preserving the uphill datum and shoreline slope.
        if self.lm.name == "hongyadong" {
            return hongyadong_cliff_cap(mx, mz, raw, self.base_geo);
        }
        let grade = heritage::indexed_grade_height(
            self.lm.name,
            glam::DVec2::new(mx, mz),
            raw,
            self.base_geo,
        );
        let raw = grade.map_or(raw, |(height, _)| height);
        if let Some(cut) = heritage::indexed_height(
            self.lm.name,
            false,
            glam::DVec2::new(mx, mz),
            raw,
            self.base_geo,
        ) {
            return Some((cut.0, cut.1.max(grade.map_or(0.0, |(_, weight)| weight))));
        }
        if grade.is_some() {
            // The catalog forbids mixing independent grades with datum-
            // setting flatten patches, whose boundary would otherwise jump.
            return grade;
        }
        let (index, weight) = self.flatten_patch(mx, mz)?;
        let target = *self.flatten_bases.get(index)?;
        Some((raw + (target - raw) * weight as f32, weight))
    }

    pub(crate) fn corrected_water_height(&self, lat: f64, lon: f64, raw: f64) -> f64 {
        let cuts = curated::water_cuts(self.lm.name);
        if cuts.is_empty() {
            return raw;
        }
        let Some((x, z)) = self.column_at(lat, lon, 0.0) else {
            return raw;
        };
        heritage::indexed_height(
            self.lm.name,
            true,
            glam::DVec2::new(x, z),
            raw as f32,
            self.base_geo,
        )
        .map_or(raw, |(height, _)| raw.min(f64::from(height)))
    }

    /// The surface asset and corrected terrain must use the exact same base.
    pub fn model_base_geo(&self, mx: f64, mz: f64) -> f32 {
        self.flatten_patch(mx, mz)
            .and_then(|(index, _)| self.flatten_bases.get(index).copied())
            .map(|base| base + self.calibration_base_offset)
            .unwrap_or(self.base_geo)
    }

    /// Map a column center (EN meters relative to the cell center) into
    /// model space; None when outside the model's padded footprint.
    pub fn column(&self, e: f64, n: f64, pad: f64) -> Option<(f64, f64)> {
        let (de, dn) = (e - self.anchor_e, n - self.anchor_n);
        // World → model: rotate by -heading.
        let mx = de * self.cos_h - dn * self.sin_h;
        let mz = de * self.sin_h + dn * self.cos_h;
        let b = self.lm;
        (mx >= b.bbox_min[0] - pad
            && mx <= b.bbox_max[0] + pad
            && mz >= b.bbox_min[2] - pad
            && mz <= b.bbox_max[2] + pad)
            .then_some((mx, mz))
    }

    /// Map an absolute geodetic column center into the landmark's canonical
    /// tangent frame. Every terrain cell therefore evaluates the same world
    /// point to the same model coordinate at shared boundaries.
    pub fn column_at(&self, lat: f64, lon: f64, pad: f64) -> Option<(f64, f64)> {
        let point = ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
        self.column_ecef(point, pad).map(|(mx, _, mz)| (mx, mz))
    }

    /// Map a point from the cell's actual rendered tangent grid into the
    /// canonical landmark frame. Generation must use this path: source
    /// samples live on the geodetic column lattice, while mesh vertices live
    /// on `ChunkFrame`'s tangent grid, and those two positions can differ by
    /// several sample intervals within a coarse cube-sphere cell.
    pub fn column_ecef(&self, point: glam::DVec3, pad: f64) -> Option<(f64, f64, f64)> {
        let delta = point - self.anchor_ecef;
        let e = delta.dot(self.anchor_east);
        let n = delta.dot(self.anchor_north);
        let mx = e * self.cos_h - n * self.sin_h;
        let mz = e * self.sin_h + n * self.cos_h;
        let my = delta.dot(self.anchor_up);
        let b = self.lm;
        (mx >= b.bbox_min[0] - pad
            && mx <= b.bbox_max[0] + pad
            && mz >= b.bbox_min[2] - pad
            && mz <= b.bbox_max[2] + pad)
            .then_some((mx, my, mz))
    }

    /// Authored structure ownership can be narrower than a fortress's full
    /// activation bounds; enclosed streets and vegetation remain source-owned.
    pub fn excludes_source_at(&self, point: glam::DVec3, pad: f64) -> bool {
        self.column_ecef(point, pad).is_some_and(|(x, _, z)| {
            heritage::source_footprint(self.lm.name)
                .is_none_or(|footprint| footprint.contains(glam::DVec2::new(x, z), pad))
        })
    }

    pub(crate) fn replaces_source_building(
        &self,
        seed: u64,
        rings: &vearth_data::vectors::Rings,
    ) -> bool {
        // Central Park Tower's cantilever projects over the independently
        // mapped Art Students League building. A planar overlap cannot
        // distinguish its ground-level envelope from the tower parts above.
        let preserve = self.lm.name == "central-park-tower" && seed == 13_506_397_056_615_716_703;
        self.lm.suppress.buildings && !preserve && self.footprint_overlaps(rings)
    }

    /// True when an ordinary building polygon overlaps this landmark's
    /// replacement footprint. Terrain-cleanup rectangles are the canonical
    /// architectural ground footprints for asset-backed landmarks, so reuse
    /// them here (including their feather) instead of maintaining a second,
    /// hand-tuned suppression box. This runs before source-envelope generation
    /// and removes the complete source building.
    ///
    /// Cell-independent: ring geometry and the anchor transform are the same
    /// absolute quantities in every cell that sees the feature, so whole-
    /// feature decisions agree across cell borders.
    pub fn footprint_overlaps(&self, rings: &vearth_data::vectors::Rings) -> bool {
        let (mut x0, mut z0, mut x1, mut z1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in rings.iter().flatten() {
            let (de, dn) = (p[0] as f64 - self.anchor_e, p[1] as f64 - self.anchor_n);
            let mx = de * self.cos_h - dn * self.sin_h;
            let mz = de * self.sin_h + dn * self.cos_h;
            x0 = x0.min(mx);
            z0 = z0.min(mz);
            x1 = x1.max(mx);
            z1 = z1.max(mz);
        }

        // The Fremont roof occupies the street, while casino footprints can
        // overlap its edge. Only the narrow roof may be replaced wholesale.
        if self.lm.name == "fremont-canopy"
            && self
                .lm
                .suppress_bbox_xz
                .is_some_and(|(lo, hi)| x0 < lo[0] || x1 > hi[0])
        {
            return false;
        }

        // A large neighboring source envelope can overlap a small replacement
        // site. Audited catalog limits keep that complete building intact.
        if city_generated::max_source_building_span(self.lm.name)
            .or_else(|| curated::max_source_building_span(self.lm.name))
            .is_some_and(|span| (x1 - x0).max(z1 - z0) > span)
        {
            return false;
        }

        if let Some(footprint) = heritage::source_footprint(self.lm.name) {
            let local: Vec<Vec<glam::DVec2>> = rings
                .iter()
                .map(|ring| {
                    ring.iter()
                        .map(|p| {
                            let de = p[0] as f64 - self.anchor_e;
                            let dn = p[1] as f64 - self.anchor_n;
                            glam::DVec2::new(
                                de * self.cos_h - dn * self.sin_h,
                                de * self.sin_h + dn * self.cos_h,
                            )
                        })
                        .collect()
                })
                .collect();
            return footprint.overlaps(&local);
        }

        let terrain_footprints = self.lm.terrain_flatten();
        if !terrain_footprints.is_empty() {
            return terrain_footprints.iter().any(|patch| {
                let hx = patch.half_extent[0] + patch.feather_m;
                let hz = patch.half_extent[1] + patch.feather_m;
                let mn = [patch.center_xz[0] - hx, patch.center_xz[1] - hz];
                let mx = [patch.center_xz[0] + hx, patch.center_xz[1] + hz];
                x0 <= mx[0] && x1 >= mn[0] && z0 <= mx[1] && z1 >= mn[1]
            });
        }

        let b = self.lm;
        let (mn, mx) = b.suppress_bbox_xz.unwrap_or((
            [b.bbox_min[0], b.bbox_min[2]],
            [b.bbox_max[0], b.bbox_max[2]],
        ));
        x0 <= mx[0] && x1 >= mn[0] && z0 <= mx[1] && z1 >= mn[1]
    }
}

fn hongyadong_cliff_cap(mx: f64, mz: f64, raw: f32, base: f32) -> Option<(f32, f64)> {
    let edge = ((102.0 - mx.abs()) / 7.0)
        .min((mz + 13.0) / 5.0)
        .min((90.0 - mz) / 20.0)
        .clamp(0.0, 1.0);
    if edge == 0.0 {
        return None;
    }
    let weight = edge * edge * (3.0 - 2.0 * edge);
    let grade = base - ((mz + 10.0) * 1.25).clamp(0.0, 50.8) as f32;
    Some((raw + (raw.min(grade) - raw) * weight as f32, weight))
}

fn hongyadong_source() -> &'static Landmark {
    static SOURCE: OnceLock<&Landmark> = OnceLock::new();
    SOURCE.get_or_init(|| LANDMARKS.iter().find(|lm| lm.name == "hongyadong").unwrap())
}

pub(crate) fn bridge_ground_refines_tile(lat: f64, lon: f64, half_extent_m: f64) -> bool {
    let placement = effective_placement(hongyadong_source());
    let anchor_lat = placement.lat_deg.to_radians();
    let anchor_lon = placement.lon_deg.to_radians();
    (lat - anchor_lat).abs() * MEAN_RADIUS < 160.0 + half_extent_m
        && (lon - anchor_lon).abs() * anchor_lat.cos() * MEAN_RADIUS < 160.0 + half_extent_m
}

/// Correct the retained Hongya viaduct's ground datum in absolute geography.
/// Bridge profiles sample beyond their receiving cell, so this deliberately
/// does not depend on that cell's active landmarks or elevation sample pitch.
pub(crate) fn corrected_bridge_ground(lat: f64, lon: f64, raw: f32) -> f32 {
    let lm = hongyadong_source();
    let placement = effective_placement(lm);
    let anchor_lat = placement.lat_deg.to_radians();
    let anchor_lon = placement.lon_deg.to_radians();
    if (lat - anchor_lat).abs() * MEAN_RADIUS > 250.0
        || (lon - anchor_lon).abs() * anchor_lat.cos() * MEAN_RADIUS > 250.0
    {
        return raw;
    }
    let Base::Fixed(base) = lm.base else {
        return raw;
    };
    let (sin_lat, cos_lat) = anchor_lat.sin_cos();
    let (sin_lon, cos_lon) = anchor_lon.sin_cos();
    let offset = ellipsoid::geodetic_to_ecef(lat, lon, 0.0)
        - ellipsoid::geodetic_to_ecef(anchor_lat, anchor_lon, 0.0);
    let east = offset.dot(glam::DVec3::new(-sin_lon, cos_lon, 0.0));
    let north = offset.dot(glam::DVec3::new(
        -sin_lat * cos_lon,
        -sin_lat * sin_lon,
        cos_lat,
    ));
    let (sin_h, cos_h) = placement.heading_deg.to_radians().sin_cos();
    hongyadong_cliff_cap(
        east * cos_h - north * sin_h,
        east * sin_h + north * cos_h,
        raw,
        base + placement.base_offset_m as f32,
    )
    .map_or(raw, |(height, _)| height)
}

/// Visit the landmarks whose footprint can touch this cell, preserving the
/// registry order used for stamping. Keeping the activation predicate in one
/// place also lets source planning account for every `Base::Terrain` lookup
/// without running the lookup itself.
fn for_each_active_anchor(
    cell: CellId,
    frame: &ChunkFrame,
    mut visit: impl FnMut(&'static Landmark, ResolvedPlacement, f64, f64),
) {
    if cell.level < LANDMARK_MIN_LEVEL {
        return;
    }
    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    // Cube-face sample axes are skewed and rotated relative to east/north.
    // A geographic rectangle using their lengths can reject a small gateway
    // inside its own cell. Bound the actual cell corners in ECEF instead.
    let cell_radius = [0.0, 1.0]
        .into_iter()
        .flat_map(|x| {
            [0.0, 1.0].into_iter().map(move |z| {
                let (lat, lon) = frame.column_latlon(x, z);
                ellipsoid::geodetic_to_ecef(lat, lon, 0.0).distance(frame.anchor)
            })
        })
        .fold(0.0_f64, f64::max)
        + 1.0;
    for lm in LANDMARKS {
        let placement = effective_placement(lm);
        let lat = placement.lat_deg.to_radians();
        let lon = placement.lon_deg.to_radians();
        let anchor = ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
        let radius = lm.bbox_radius();
        if anchor.distance(frame.anchor) > cell_radius + radius {
            continue;
        }
        let mut dlon = lon - lon0;
        if dlon > std::f64::consts::PI {
            dlon -= std::f64::consts::TAU;
        } else if dlon < -std::f64::consts::PI {
            dlon += std::f64::consts::TAU;
        }
        let e = dlon * lat0.cos() * MEAN_RADIUS;
        let n = (lat - lat0) * MEAN_RADIUS;
        visit(lm, placement, e, n);
    }
}

/// Every terrain-backed landmark base that generation can sample for this
/// cell. Source dependency planning uses these explicit out-of-cell points in
/// addition to the regular raster footprint.
pub(crate) fn terrain_base_sample_points(cell: CellId, frame: &ChunkFrame) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for_each_active_anchor(cell, frame, |lm, placement, _, _| {
        let lat = placement.lat_deg.to_radians();
        let lon = placement.lon_deg.to_radians();
        if matches!(lm.base, Base::Terrain | Base::TerrainOffset(_)) {
            if lm.terrain_flatten().is_empty() {
                out.push((lat, lon));
            } else {
                for patch in lm.terrain_flatten() {
                    if patch.fixed_base_geo.is_some() {
                        continue;
                    }
                    for [mx, mz] in patch.sample_offsets() {
                        out.push(model_offset_latlon(placement.heading_deg, lat, lon, mx, mz));
                    }
                }
            }
        }
    });
    out
}

fn model_offset_latlon(heading_deg: f64, lat: f64, lon: f64, mx: f64, mz: f64) -> (f64, f64) {
    let h = heading_deg.to_radians();
    let (sin_h, cos_h) = h.sin_cos();
    // Model -> world tangent coordinates (inverse of `column_ecef`).
    let east_m = mx * cos_h + mz * sin_h;
    let north_m = -mx * sin_h + mz * cos_h;
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    let anchor = ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
    let east = glam::DVec3::new(-sin_lon, cos_lon, 0.0);
    let north = glam::DVec3::new(-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat);
    let (sample_lat, sample_lon, _) =
        ellipsoid::ecef_to_geodetic(anchor + east * east_m + north * north_m);
    (sample_lat, sample_lon)
}

fn median(mut values: Vec<f32>) -> f32 {
    values.sort_unstable_by(f32::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        0.5 * (values[middle - 1] + values[middle])
    } else {
        values[middle]
    }
}

/// The landmarks whose footprint (padded by half a cell diagonal) can touch
/// this cell. `terrain_at(lat, lon)` resolves `Base::Terrain` anchors.
pub fn active_for(
    cell: CellId,
    frame: &ChunkFrame,
    terrain_at: &dyn Fn(f64, f64) -> f32,
) -> Vec<ActiveLandmark> {
    let mut out = Vec::new();
    for_each_active_anchor(cell, frame, |lm, placement, e, n| {
        out.push(ActiveLandmark::new(lm, placement, e, n, terrain_at));
    });
    out
}

// Source building datum samples can lie outside their receiving cell. Resolve
// curated terrain in absolute geography so neighboring cells share the same
// foundation, even when only one cell activates the landmark mesh.
pub(crate) fn corrected_source_building_ground(lat: f64, lon: f64, raw: f32) -> f32 {
    static SOURCES: OnceLock<Vec<&'static Landmark>> = OnceLock::new();
    let sources = SOURCES.get_or_init(|| {
        curated::models()
            .filter(|lm| {
                !lm.terrain_flatten().is_empty()
                    || !curated::terrain_cuts(lm.name).is_empty()
                    || !curated::terrain_grades(lm.name).is_empty()
            })
            .collect()
    });
    let override_ = calibration_override();
    let mut ground = raw;
    for &lm in sources {
        let placement = placement_with_override(lm, override_);
        let anchor_lat = placement.lat_deg.to_radians();
        let anchor_lon = placement.lon_deg.to_radians();
        let radius = lm.bbox_min[0]
            .abs()
            .max(lm.bbox_max[0].abs())
            .hypot(lm.bbox_min[2].abs().max(lm.bbox_max[2].abs()))
            * 1.01
            + 2.0;
        let dlon = (lon - anchor_lon + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        if (lat - anchor_lat).abs() * MEAN_RADIUS > radius
            || dlon.abs() * anchor_lat.cos() * MEAN_RADIUS > radius
        {
            continue;
        }
        // Every corrected heritage site supplies its clean datum explicitly;
        // using a receiving cell's raw sample here would reintroduce seams.
        let active = ActiveLandmark::new(lm, placement, 0.0, 0.0, &|_, _| {
            panic!(
                "corrected heritage sites require fixed ground datums: {}",
                lm.name
            )
        });
        if let Some((x, z)) = active.column_at(lat, lon, 0.0)
            && let Some((height, _)) = active.flattened_terrain(x, z, ground)
        {
            ground = height;
        }
    }
    ground
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use vearth_core::cell::{CellId, ChunkAddr};

    #[test]
    fn calibration_resolves_enu_offsets_and_normalizes_heading() {
        let source = LANDMARKS
            .iter()
            .find(|lm| lm.name == "eiffel-tower")
            .unwrap();
        let resolved = resolve_calibration(CalibrationOverride {
            name: source.name,
            east_m: 10.0,
            north_m: 20.0,
            up_m: -1.25,
            heading_deg: -15.0,
        });
        assert!(resolved.lat_deg > source.lat_deg);
        assert!(resolved.lon_deg > source.lon_deg);
        assert_eq!(resolved.heading_deg, 345.0);
        assert_eq!(resolved.base_offset_m, -1.25);
    }

    #[test]
    fn registry_is_sane() {
        assert_eq!(heritage_generated::HERITAGE_MODELS.len(), 52);
        assert_eq!(skyscraper_generated::SKYSCRAPER_MODELS.len(), 59);
        assert_eq!(monument_generated::MONUMENT_MODELS.len(), 30);
        assert_eq!(island_generated::ISLAND_MODELS.len(), 1);
        assert_eq!(LANDMARKS.len(), 231, "unexpected landmark pack size");
        let names: HashSet<_> = LANDMARKS.iter().map(|lm| lm.name).collect();
        assert_eq!(names.len(), LANDMARKS.len(), "duplicate landmark name");
        for lm in LANDMARKS {
            assert!(lm.lat_deg.abs() <= 85.0, "{}", lm.name);
            assert!(lm.lon_deg.abs() <= 180.0, "{}", lm.name);
            for i in 0..3 {
                assert!(lm.bbox_min[i] < lm.bbox_max[i], "{} bbox axis {i}", lm.name);
            }
            assert!(
                lm.bbox_max[1] > 0.0,
                "{} has no above-ground surface",
                lm.name
            );
            for patch in lm.terrain_flatten() {
                assert!(
                    patch.center_xz[0] - patch.half_extent[0] - patch.feather_m >= lm.bbox_min[0]
                        && patch.center_xz[0] + patch.half_extent[0] + patch.feather_m
                            <= lm.bbox_max[0]
                        && patch.center_xz[1] - patch.half_extent[1] - patch.feather_m
                            >= lm.bbox_min[2]
                        && patch.center_xz[1] + patch.half_extent[1] + patch.feather_m
                            <= lm.bbox_max[2],
                    "{} terrain cleanup escapes its activation bbox",
                    lm.name
                );
            }
            // Outdoor overlays without mapped building mass preserve nearby
            // source buildings, including buildings beneath elevated decks.
            if !matches!(
                lm.name,
                "hollywood-sign"
                    | "fremont-canopy"
                    | "angel-of-the-north"
                    | "unisphere"
                    | "dom-luis-i-bridge"
                    | "copacabana-promenade"
            ) {
                assert!(
                    lm.suppress.buildings,
                    "{} keeps competing source mass",
                    lm.name
                );
            }
            let mesh = canonical_mesh(lm.name, 0)
                .unwrap_or_else(|| panic!("{} indexed source missing", lm.name));
            assert!(!mesh.indices.is_empty(), "{} source mesh is empty", lm.name);
        }
    }

    #[test]
    fn replacement_assets_own_their_source_geometry() {
        for (name, minimum_half_width, minimum_length) in [
            ("sydney-harbour-bridge", 26.0, 1_100.0),
            ("golden-gate-bridge", 20.0, 2_700.0),
            ("tower-bridge", 28.0, 312.0),
            ("brooklyn-bridge", 22.0, 1_130.0),
        ] {
            let bridge = LANDMARKS.iter().find(|lm| lm.name == name).unwrap();
            assert!(bridge.suppress.buildings, "{name} keeps source towers");
            assert!(bridge.suppress.roads, "{name} keeps its source road deck");
            assert!(
                bridge.bbox_max[0] - bridge.bbox_min[0] >= minimum_half_width * 2.0,
                "{name} road ownership is too narrow"
            );
            assert!(
                bridge.bbox_max[2] - bridge.bbox_min[2] >= minimum_length,
                "{name} road ownership is too short"
            );
        }
    }

    fn cell_at(lat: f64, lon: f64, level: u8) -> CellId {
        let dir = vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0).normalize();
        CellId::from_dir(dir, level)
    }

    #[test]
    fn small_monuments_activate_in_their_own_rotated_cells() {
        for lm in monument_generated::MONUMENT_MODELS {
            for level in LANDMARK_MIN_LEVEL..=20 {
                let cell = cell_at(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), level);
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                assert!(
                    active_for(cell, &frame, &|_, _| 0.0)
                        .iter()
                        .any(|a| a.lm.name == lm.name),
                    "{} missing at L{level}",
                    lm.name
                );
            }
        }
    }

    #[test]
    fn monument_source_ownership_preserves_surrounding_approaches() {
        for lm in monument_generated::MONUMENT_MODELS {
            assert!(
                heritage::source_footprint(lm.name).is_some(),
                "{} has no indexed ownership",
                lm.name
            );
        }
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "meiji-jingu-second-torii")
            .unwrap();
        let active =
            ActiveLandmark::new(lm, placement_with_override(lm, None), 0.0, 0.0, &|_, _| {
                10.0
            });
        let point = |x: f64, z: f64| {
            active.anchor_ecef
                + active.anchor_east * (x * active.cos_h + z * active.sin_h)
                + active.anchor_north * (-x * active.sin_h + z * active.cos_h)
        };
        assert!(active.excludes_source_at(point(0.0, 0.0), 0.0));
        assert!(active.column_ecef(point(0.0, 0.85), 0.0).is_some());
        assert!(!active.excludes_source_at(point(0.0, 0.85), 0.0));
        let ring = [[-0.1, 1.4], [0.1, 1.4], [0.1, 1.6], [-0.1, 1.6]]
            .into_iter()
            .map(|[x, z]| {
                [
                    (x * active.cos_h + z * active.sin_h) as f32,
                    (-x * active.sin_h + z * active.cos_h) as f32,
                ]
            })
            .collect();
        assert!(!active.footprint_overlaps(&vec![ring]));
    }

    #[test]
    fn stone_circle_interiors_keep_source_vegetation() {
        for (name, x, z) in [("ring-of-brodgar", 0.0, 0.0), ("stonehenge", 0.0, 35.0)] {
            let lm = LANDMARKS.iter().find(|lm| lm.name == name).unwrap();
            let active =
                ActiveLandmark::new(lm, placement_with_override(lm, None), 0.0, 0.0, &|_, _| 0.0);
            let point = active.anchor_ecef
                + active.anchor_east * (x * active.cos_h + z * active.sin_h)
                + active.anchor_north * (-x * active.sin_h + z * active.cos_h);
            assert!(
                !active.excludes_source_at(point, 0.0),
                "{name} clears open ground"
            );
        }
    }

    #[test]
    fn monuments_preserve_natural_terrain() {
        for lm in monument_generated::MONUMENT_MODELS {
            let Base::Fixed(base) = lm.base else {
                panic!("{} must use surveyed geometry grounding", lm.name);
            };
            assert!(
                lm.terrain_flatten().is_empty(),
                "{} flattens terrain",
                lm.name
            );
            assert!(
                curated::terrain_cuts(lm.name).is_empty(),
                "{} cuts terrain",
                lm.name
            );
            assert!(
                curated::terrain_grades(lm.name).is_empty(),
                "{} grades terrain",
                lm.name
            );
            assert!(
                curated::water_cuts(lm.name).is_empty(),
                "{} cuts water",
                lm.name
            );
            let placement = placement_with_override(lm, None);
            let active = ActiveLandmark::new(lm, placement, 0.0, 0.0, &|_, _| {
                panic!("{} must not resample a terrain datum", lm.name)
            });
            assert_eq!(active.base_geo, base);
            assert!(!lm.suppress.roads, "{} removes its approach", lm.name);
            for lod in 0..3 {
                let mesh = canonical_mesh(lm.name, lod).unwrap();
                for v in &mesh.vertices {
                    let [x, _, z] = v.pos.map(f64::from);
                    for ground in [-30.0, 0.0, 37.0, 180.0] {
                        assert!(
                            active.flattened_terrain(x, z, ground).is_none(),
                            "{} changes terrain at {x}, {z}",
                            lm.name
                        );
                    }
                    assert_eq!(active.model_base_geo(x, z), base);
                }
            }
            let (lat, lon) = model_offset_latlon(
                placement.heading_deg,
                placement.lat_deg.to_radians(),
                placement.lon_deg.to_radians(),
                0.0,
                10.0,
            );
            let east =
                (lon - placement.lon_deg.to_radians()) * placement.lat_deg.to_radians().cos();
            let north = lat - placement.lat_deg.to_radians();
            let bearing = east.atan2(north).to_degrees().rem_euclid(360.0);
            let error = (bearing - placement.heading_deg + 180.0).rem_euclid(360.0) - 180.0;
            assert!(error.abs() < 0.2, "{} heading convention reversed", lm.name);
        }
    }

    #[test]
    fn road_replacement_is_explicit_and_uses_continuous_footprints() {
        for landmark in LANDMARKS {
            let cell = cell_at(
                landmark.lat_deg.to_radians(),
                landmark.lon_deg.to_radians(),
                17,
            );
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let active = active_for(cell, &frame, &|_, _| 0.0)
                .into_iter()
                .find(|a| a.lm.name == landmark.name)
                .unwrap_or_else(|| panic!("{} inactive at own anchor", landmark.name));
            let polygons = active.road_exclusions(&frame);
            assert_eq!(
                !polygons.is_empty(),
                landmark.suppress.roads,
                "{}",
                landmark.name
            );
            for polygon in polygons {
                assert!(polygon.len() >= 3);
                assert!(polygon.iter().all(|p| p.is_finite()));
                assert!((polygon[1] - polygon[0]).perp_dot(polygon[2] - polygon[0]) > 0.0);
                assert!(
                    polygon
                        .iter()
                        .any(|p| p.x.fract().abs() > 1e-5 || p.y.fract().abs() > 1e-5)
                );
            }
        }
    }

    #[test]
    fn cliff_cleanup_preserves_model_datum_and_only_removes_rooftop_dem() {
        let landmark = LANDMARKS.iter().find(|lm| lm.name == "hongyadong").unwrap();
        let Base::Fixed(base) = landmark.base else {
            panic!("Hongya's audited cliff datum must not depend on source LOD");
        };
        let cell = cell_at(
            landmark.lat_deg.to_radians(),
            landmark.lon_deg.to_radians(),
            17,
        );
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let active = active_for(cell, &frame, &|_, _| 231.0)
            .into_iter()
            .find(|a| a.lm.name == landmark.name)
            .unwrap();
        let back = active.flattened_terrain(0.0, 0.0, 240.0).unwrap();
        let front = active.flattened_terrain(0.0, 45.0, 210.0).unwrap();
        assert!(back.0 - front.0 > 35.0);
        assert!((front.0 - (base - 50.8)).abs() < 0.001);
        assert_eq!(active.model_base_geo(0.0, 45.0), base);
        assert_eq!(active.flattened_terrain(0.0, 45.0, 175.0).unwrap().0, 175.0);
        assert!(active.flattened_terrain(103.0, 30.0, 250.0).is_none());
        assert!(active.flattened_terrain(0.0, 91.0, 250.0).is_none());
        let feather = active.flattened_terrain(100.0, 30.0, 240.0).unwrap();
        assert!(feather.0 > front.0 && feather.0 < 240.0 && feather.1 > 0.0 && feather.1 < 1.0);
    }

    #[test]
    fn retained_bridge_ground_uses_the_canonical_cliff_across_lods() {
        let lm = LANDMARKS.iter().find(|lm| lm.name == "hongyadong").unwrap();
        let lat = lm.lat_deg.to_radians();
        let lon = lm.lon_deg.to_radians();
        for level in [LANDMARK_MIN_LEVEL, 17, 19] {
            let cell = cell_at(lat, lon, level);
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let active = active_for(cell, &frame, &|_, _| {
                panic!("the audited datum cannot acquire a cell-dependent DEM sample")
            })
            .into_iter()
            .find(|a| a.lm.name == lm.name)
            .unwrap();
            for (x, z) in [(0.0, 45.0), (80.0, 60.0), (100.0, 45.0), (0.0, 80.0)] {
                let (lat, lon) = model_offset_latlon(lm.heading_deg, lat, lon, x, z);
                let (mx, mz) = active.column_at(lat, lon, 0.0).unwrap();
                for raw in [175.0, 210.0, 260.0] {
                    let expected = active.flattened_terrain(mx, mz, raw).unwrap().0;
                    assert!((corrected_bridge_ground(lat, lon, raw) - expected).abs() < 0.0001);
                }
            }
        }
        // The global bridge-profile callback must leave every out-of-footprint
        // station untouched, including profiles evaluated by distant cells.
        for (x, z) in [(103.0, 45.0), (0.0, 91.0), (0.0, -14.0), (1_000.0, 1_000.0)] {
            let (lat, lon) = model_offset_latlon(lm.heading_deg, lat, lon, x, z);
            assert_eq!(corrected_bridge_ground(lat, lon, 260.0), 260.0);
        }
        assert_eq!(
            corrected_bridge_ground(35.145662_f64.to_radians(), 129.128345_f64.to_radians(), 0.0),
            0.0
        );
    }

    #[test]
    fn las_lajas_water_correction_is_canonical_bounded_and_lower_only() {
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "las-lajas-sanctuary")
            .unwrap();
        let lat = lm.lat_deg.to_radians();
        let lon = lm.lon_deg.to_radians();
        let raw = 2600.125;
        let cuts = curated::water_cuts(lm.name);
        assert!(!cuts.is_empty());
        let mut expected = Vec::new();
        let samples: Vec<_> = cuts
            .iter()
            .flat_map(|cut| {
                let center = cut
                    .polygon
                    .iter()
                    .fold(glam::DVec2::ZERO, |p, &v| p + glam::DVec2::from(v))
                    / cut.polygon.len() as f64;
                // A vertex tests feathering; the center tests full correction.
                [center, glam::DVec2::from(cut.polygon[0])]
            })
            .collect();
        for level in [LANDMARK_MIN_LEVEL, 17, 19] {
            let cell = cell_at(lat, lon, level);
            for di in 0..if level == LANDMARK_MIN_LEVEL { 1 } else { 2 } {
                let adjacent = CellId {
                    i: cell.i + di,
                    ..cell
                };
                let frame = ChunkFrame::of(ChunkAddr {
                    cell: adjacent,
                    k: 0,
                });
                let active = active_for(adjacent, &frame, &|_, _| {
                    panic!("water correction requires a fixed, cell-independent datum")
                })
                .into_iter()
                .find(|a| a.lm.name == lm.name)
                .unwrap();
                let actual: Vec<_> = samples
                    .iter()
                    .map(|p| {
                        let (a, b) = model_offset_latlon(lm.heading_deg, lat, lon, p.x, p.y);
                        assert_eq!(active.corrected_water_height(a, b, 2500.0), 2500.0);
                        let height = active.corrected_water_height(a, b, raw);
                        assert!((height - (f64::from(active.base_geo) - 49.0)).abs() < 0.01);
                        height.to_bits()
                    })
                    .collect();
                if expected.is_empty() {
                    expected = actual;
                } else {
                    assert_eq!(actual, expected);
                }
                let (a, b) = model_offset_latlon(lm.heading_deg, lat, lon, 0.0, 150.0);
                assert_eq!(active.corrected_water_height(a, b, raw), raw);
                assert_eq!(active.model_base_geo(0.0, -25.0), active.base_geo);
            }
        }
    }

    #[test]
    fn bled_island_replaces_mapped_buildings_and_preserves_wooded_banks() {
        let source: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/heritage_catalog/references/islands/bled-buildings.json"
        )))
        .unwrap();
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "bled-island")
            .unwrap();
        for level in [LANDMARK_MIN_LEVEL, 17, 19, 20] {
            let cell = cell_at(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), level);
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let active = active_for(cell, &frame, &|_, _| 475.0)
                .into_iter()
                .find(|a| a.lm.name == lm.name)
                .unwrap();
            for building in source["buildings"].as_array().unwrap() {
                let rings = building["rings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|ring| {
                        ring.as_array()
                            .unwrap()
                            .iter()
                            .map(|p| {
                                [
                                    (active.anchor_e + p[0].as_f64().unwrap()) as f32,
                                    (active.anchor_n + p[1].as_f64().unwrap()) as f32,
                                ]
                            })
                            .collect()
                    })
                    .collect();
                let seed = building["seed"].as_str().unwrap().parse().unwrap();
                assert!(
                    active.replaces_source_building(seed, &rings),
                    "source {seed} at L{level}"
                );
            }
            for (x, z) in [(-40.0, -30.0), (25.0, -20.0), (0.0, 55.0)] {
                let point = active.anchor_ecef + active.anchor_east * x + active.anchor_north * z;
                assert!(
                    !active.excludes_source_at(point, 0.0),
                    "wooded bank ({x}, {z})"
                );
            }
            assert!(lm.terrain_flatten().is_empty());
            assert!(curated::terrain_cuts(lm.name).is_empty());
            assert!(curated::water_cuts(lm.name).is_empty());
            for (x, z) in [(-60.0, 0.0), (45.0, 0.0), (0.0, -112.0), (0.0, 78.0)] {
                assert!(
                    active.flattened_terrain(x, z, 475.0).is_none(),
                    "lake ({x}, {z})"
                );
            }
            let summit = active.flattened_terrain(0.0, 0.0, 475.0).unwrap().0;
            assert!((493.0..495.0).contains(&summit));
            assert_eq!(
                active.corrected_water_height(lm.lat_deg, lm.lon_deg, 474.7),
                474.7
            );
        }
    }

    #[test]
    fn central_park_cantilever_preserves_the_independent_building_below() {
        let source: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/skyscraper_catalog/references/extra_us_east/central-park-tower-source-ownership.json"
        )))
        .unwrap();
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "central-park-tower")
            .unwrap();
        for level in [17, 19] {
            let cell = cell_at(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), level);
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let active = active_for(cell, &frame, &|_, _| 20.0)
                .into_iter()
                .find(|a| a.lm.name == lm.name)
                .unwrap();
            for feature in source["features"].as_array().unwrap() {
                let rings = feature["local_rings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|ring| {
                        ring.as_array()
                            .unwrap()
                            .iter()
                            .map(|point| {
                                let x = point[0].as_f64().unwrap();
                                let z = point[1].as_f64().unwrap();
                                [
                                    (active.anchor_e + x * active.cos_h + z * active.sin_h) as f32,
                                    (active.anchor_n - x * active.sin_h + z * active.cos_h) as f32,
                                ]
                            })
                            .collect()
                    })
                    .collect();
                let seed = feature["seed"].as_str().unwrap().parse().unwrap();
                assert!(
                    active.footprint_overlaps(&rings),
                    "expected projected overlap: {seed}"
                );
                assert_eq!(
                    active.replaces_source_building(seed, &rings),
                    feature["replace"].as_bool().unwrap(),
                    "source {seed} at level {level}",
                );
            }
        }
    }

    #[test]
    fn fremont_replaces_street_roof_without_deleting_edge_casinos() {
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "fremont-canopy")
            .unwrap();
        let cell = cell_at(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), 17);
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let active = active_for(cell, &frame, &|_, _| 615.0)
            .into_iter()
            .find(|a| a.lm.name == lm.name)
            .unwrap();
        let ring = |lo: [f64; 2], hi: [f64; 2]| {
            vec![
                vec![
                    [lo[0], lo[1]],
                    [hi[0], lo[1]],
                    [hi[0], hi[1]],
                    [lo[0], hi[1]],
                ]
                .into_iter()
                .map(|[x, z]| {
                    [
                        (active.anchor_e + x * active.cos_h + z * active.sin_h) as f32,
                        (active.anchor_n - x * active.sin_h + z * active.cos_h) as f32,
                    ]
                })
                .collect(),
            ]
        };
        assert!(active.footprint_overlaps(&ring([-16.83, -211.05], [16.9, 210.95])));
        assert!(!active.footprint_overlaps(&ring([-130.0, -60.0], [-15.0, 60.0])));
        assert!(!active.footprint_overlaps(&ring([15.0, -40.0], [150.0, 40.0])));
    }

    #[test]
    fn suzhou_replaces_gate_parts_without_deleting_the_adjacent_mall() {
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "gate-of-the-orient")
            .unwrap();
        for level in [LANDMARK_MIN_LEVEL, 17, 19] {
            let cell = cell_at(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), level);
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let active = active_for(cell, &frame, &|_, _| 5.0)
                .into_iter()
                .find(|a| a.lm.name == lm.name)
                .unwrap();
            let ring = |lo: [f64; 2], hi: [f64; 2]| {
                vec![
                    [
                        [lo[0], lo[1]],
                        [hi[0], lo[1]],
                        [hi[0], hi[1]],
                        [lo[0], hi[1]],
                    ]
                    .into_iter()
                    .map(|[x, z]| {
                        [
                            (active.anchor_e + x * active.cos_h + z * active.sin_h) as f32,
                            (active.anchor_n - x * active.sin_h + z * active.cos_h) as f32,
                        ]
                    })
                    .collect(),
                ]
            };
            // Measured Gate podium and its individual tower envelopes overlap
            // the mall's much larger source envelope at their southern edge.
            assert!(active.footprint_overlaps(&ring([-88.0, -52.0], [104.0, 50.0])));
            assert!(active.footprint_overlaps(&ring([-85.0, -50.0], [-30.0, 50.0])));
            assert!(active.footprint_overlaps(&ring([30.0, -50.0], [85.0, 50.0])));
            assert!(!active.footprint_overlaps(&ring([-251.0, -216.0], [214.0, -18.0])));
        }
    }

    #[test]
    fn santiago_round_podium_preserves_nearby_pond_ground() {
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "santiago-bahai-temple")
            .unwrap();
        let cell = cell_at(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), 17);
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let active = active_for(cell, &frame, &|_, _| 974.6)
            .into_iter()
            .find(|a| a.lm.name == lm.name)
            .unwrap();
        for (x, z) in [(0.0, 0.0), (21.5, 0.0), (0.0, 21.5)] {
            let (ground, weight) = active.flattened_terrain(x, z, 974.6).unwrap();
            assert!((ground - 983.1224).abs() < 0.001);
            assert_eq!(weight, 1.0);
        }
        // This surveyed pond edge is outside the podium. Raising empty corners
        // to the podium datum would deepen an existing source-water cut.
        assert!(active.flattened_terrain(-23.41, 20.41, 974.6).is_none());
        for angle in 0..360 {
            let theta = (angle as f64).to_radians();
            assert!(
                active
                    .flattened_terrain(24.13 * theta.cos(), 24.13 * theta.sin(), 974.6)
                    .is_none()
            );
        }
    }

    #[test]
    fn activation_finds_the_opera_house() {
        // The L15 cell containing the Opera House anchor must activate it.
        let lm = LANDMARKS
            .iter()
            .find(|l| l.name == "sydney-opera-house")
            .unwrap();
        let cell = cell_at(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), 15);
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let active = active_for(cell, &frame, &|_, _| 0.0);
        assert!(active.iter().any(|a| a.lm.name == "sydney-opera-house"));
        // A cell on the other side of the planet must not.
        let far = cell_at(0.9, 0.1, 15);
        let frame_far = ChunkFrame::of(ChunkAddr { cell: far, k: 0 });
        assert!(active_for(far, &frame_far, &|_, _| 0.0).is_empty());
        // Below the gate: nothing.
        let coarse = cell_at(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), 10);
        let frame_c = ChunkFrame::of(ChunkAddr { cell: coarse, k: 0 });
        assert!(active_for(coarse, &frame_c, &|_, _| 0.0).is_empty());
    }

    #[test]
    fn dem_baked_monuments_use_clean_perimeter_ground() {
        for (name, contaminated_radius, clean_ground, baked_top) in
            [("giza-necropolis", 140.0, 61.0, 190.0)]
        {
            let lm = LANDMARKS.iter().find(|lm| lm.name == name).unwrap();
            let lat = lm.lat_deg.to_radians();
            let lon = lm.lon_deg.to_radians();
            let cell = cell_at(lat, lon, 16);
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let anchor = ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
            let terrain = |sample_lat: f64, sample_lon: f64| {
                let sample = ellipsoid::geodetic_to_ecef(sample_lat, sample_lon, 0.0);
                let distance = (sample - anchor).length();
                if distance < contaminated_radius {
                    baked_top
                } else {
                    clean_ground
                }
            };
            assert_eq!(terrain(lat, lon), baked_top, "test setup lost {name} bump");

            let active = active_for(cell, &frame, &terrain)
                .into_iter()
                .find(|active| active.lm.name == name)
                .unwrap();
            assert!(
                (active.base_geo - clean_ground).abs() < 0.01,
                "{name} still used its contaminated anchor: {}",
                active.base_geo
            );
            let (corrected, weight) = active
                .flattened_terrain(0.0, 0.0, baked_top)
                .expect("landmark center must be flattened");
            assert!((corrected - clean_ground).abs() < 0.01);
            assert_eq!(weight, 1.0);
            assert!((active.model_base_geo(0.0, 0.0) - clean_ground).abs() < 0.01);
        }
    }

    #[test]
    fn new_buildings_and_merlion_use_clean_perimeter_ground() {
        for name in [
            "chrysler-building",
            "flatiron-building",
            "merdeka-118",
            "one-world-trade-center",
            "petronas-towers",
            "steinway-tower",
            "the-gherkin",
            "the-shard",
            "singapore-merlion",
        ] {
            let lm = LANDMARKS.iter().find(|lm| lm.name == name).unwrap();
            assert!(!lm.terrain_flatten().is_empty(), "{name} lacks DEM cleanup");
            let lat = lm.lat_deg.to_radians();
            let lon = lm.lon_deg.to_radians();
            let anchor = ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
            let terrain = |sample_lat: f64, sample_lon: f64| {
                let sample = ellipsoid::geodetic_to_ecef(sample_lat, sample_lon, 0.0);
                if (sample - anchor).length() < 5.0 {
                    500.0 // simulated DSM building mass at the registry anchor
                } else {
                    50.0 // clean perimeter grade
                }
            };
            let cell = cell_at(lat, lon, 16);
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let active = active_for(cell, &frame, &terrain)
                .into_iter()
                .find(|active| active.lm.name == name)
                .unwrap();
            assert!(
                (active.base_geo - 50.0).abs() < 0.01,
                "{name} used contaminated anchor terrain: {}",
                active.base_geo
            );
            let (corrected, weight) = active
                .flattened_terrain(
                    lm.terrain_flatten()[0].center_xz[0],
                    lm.terrain_flatten()[0].center_xz[1],
                    500.0,
                )
                .expect("terrain patch center must be corrected");
            assert!((corrected - 50.0).abs() < 0.01, "{name}");
            assert_eq!(weight, 1.0, "{name}");
        }
    }

    #[test]
    fn audited_garden_grade_avoids_contaminated_perimeter_and_feathers_locally() {
        for (name, grade) in [("singapore-supertrees", 5.28), ("lotus-temple", 242.8)] {
            let lm = LANDMARKS.iter().find(|lm| lm.name == name).unwrap();
            let lat = lm.lat_deg.to_radians();
            let lon = lm.lon_deg.to_radians();
            let cell = cell_at(lat, lon, 17);
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let patch = lm.terrain_flatten()[0];
            let points = terrain_base_sample_points(cell, &frame);
            for [x, z] in patch.sample_offsets() {
                let point = model_offset_latlon(lm.heading_deg, lat, lon, x, z);
                assert!(
                    !points.contains(&point),
                    "{name} requested contaminated perimeter"
                );
            }
            let active = active_for(cell, &frame, &|_, _| 999.0)
                .into_iter()
                .find(|a| a.lm.name == name)
                .unwrap();
            assert_eq!(active.base_geo, grade);
            let [x, z] = patch.center_xz;
            let (corrected, weight) = active.flattened_terrain(x, z, 999.0).unwrap();
            assert!((corrected - grade).abs() < 0.0001);
            assert_eq!(weight, 1.0);
            assert_eq!(active.model_base_geo(x, z), grade);
            let edge = x + patch.half_extent[0];
            let feathered = active
                .flattened_terrain(edge + patch.feather_m * 0.5, z, 999.0)
                .unwrap();
            assert!(feathered.0 > grade && feathered.0 < 999.0);
            assert!(
                active
                    .flattened_terrain(edge + patch.feather_m + 1.0, z, 999.0)
                    .is_none()
            );
        }
        // A model's absolute datum does not opt it into DEM modification.
        let fixed = LANDMARKS
            .iter()
            .find(|lm| lm.name == "louvre-abu-dhabi")
            .unwrap();
        assert!(matches!(fixed.base, Base::Fixed(4.0)));
        assert!(fixed.terrain_flatten().is_empty());
    }

    #[test]
    fn corrected_heritage_sites_resolve_without_cell_terrain_samples() {
        for lm in curated::models() {
            if !curated::terrain_grades(lm.name).is_empty() {
                assert!(matches!(lm.base, Base::Fixed(_)));
                assert!(
                    lm.terrain_flatten().is_empty(),
                    "{} mixes independent grades with model-datum patches",
                    lm.name
                );
            }
            if lm.terrain_flatten().is_empty()
                && curated::terrain_cuts(lm.name).is_empty()
                && curated::terrain_grades(lm.name).is_empty()
            {
                continue;
            }
            let active = ActiveLandmark::new(lm, effective_placement(lm), 0.0, 0.0, &|_, _| {
                panic!("{} needs an explicit clean datum", lm.name)
            });
            assert!(active.base_geo.is_finite());
        }
    }

    #[test]
    fn zayed_approach_grade_preserves_stair_datum_across_cells() {
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "sheikh-zayed-grand-mosque")
            .unwrap();
        let Base::Fixed(base) = lm.base else {
            panic!("the approach needs a fixed architectural datum");
        };
        let (lat, lon) = model_offset_latlon(
            lm.heading_deg,
            lm.lat_deg.to_radians(),
            lm.lon_deg.to_radians(),
            235.0,
            18.0,
        );
        for level in [15, 17, 19] {
            let cell = cell_at(lat, lon, level);
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let active = active_for(cell, &frame, &|_, _| -6.0)
                .into_iter()
                .find(|a| a.lm.name == lm.name)
                .unwrap();
            assert_eq!(active.model_base_geo(235.0, 18.0), base);
            for raw in [-8.0, -6.0, 4.3, 18.0] {
                let ground = active.flattened_terrain(235.0, 18.0, raw).unwrap().0;
                assert!((ground - (base - 9.15)).abs() < 0.001);
                let lateral_ground = active.flattened_terrain(260.0, 70.0, raw).unwrap().0;
                assert!((lateral_ground - (base - 9.15)).abs() < 0.001);
                assert_eq!(active.model_base_geo(260.0, 70.0), base);
                assert!((corrected_source_building_ground(lat, lon, raw) - ground).abs() < 0.001);
                assert!(active.flattened_terrain(440.0, 180.0, raw).is_none());
            }
        }
    }

    #[test]
    fn retained_osaka_buildings_follow_corrected_plaza_across_cells() {
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "osaka-castle")
            .unwrap();
        let (lat, lon) = model_offset_latlon(
            lm.heading_deg,
            lm.lat_deg.to_radians(),
            lm.lon_deg.to_radians(),
            40.0,
            -30.0,
        );
        let corrected = corrected_source_building_ground(lat, lon, 55.0);
        assert!(
            (corrected - 31.25).abs() < 0.001,
            "plaza ground {corrected}"
        );
        // A native building's shared source datum is queried beyond the
        // receiving cell at every LOD, independently of mesh activation.
        for level in [15, 17, 19] {
            let cell = cell_at(lat, lon, level);
            for i in [cell.i.saturating_sub(1), cell.i, cell.i + 1] {
                let cell = CellId { i, ..cell };
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                for active in active_for(cell, &frame, &|_, _| 55.0)
                    .into_iter()
                    .filter(|a| a.lm.name == lm.name)
                {
                    let (x, z) = active.column_at(lat, lon, 0.0).unwrap();
                    assert_eq!(active.flattened_terrain(x, z, 55.0).unwrap().0, corrected);
                }
            }
        }
        assert_eq!(corrected_source_building_ground(lat, lon, 20.0), 20.0);
        let (outside_lat, outside_lon) = model_offset_latlon(
            lm.heading_deg,
            lm.lat_deg.to_radians(),
            lm.lon_deg.to_radians(),
            180.0,
            -30.0,
        );
        assert_eq!(
            corrected_source_building_ground(outside_lat, outside_lon, 55.0),
            55.0
        );
    }

    #[test]
    fn milan_forecourt_clears_roof_contamination_without_lowering_the_cathedral() {
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "milan-cathedral")
            .unwrap();
        let active = ActiveLandmark::new(lm, effective_placement(lm), 0.0, 0.0, &|_, _| 148.4);
        let (ground, weight) = active.flattened_terrain(-20.0, -140.0, 148.4).unwrap();
        assert_eq!(weight, 1.0);
        assert!((ground - 128.5).abs() < 0.001);
        assert_eq!(active.model_base_geo(0.0, -100.0), 128.5);
        assert_eq!(
            active.flattened_terrain(0.0, -210.0, 128.0).unwrap().0,
            128.0
        );
        assert!(active.flattened_terrain(-100.0, -140.0, 140.0).is_none());
    }

    #[test]
    fn excavated_heritage_ground_does_not_lower_the_building_datum() {
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "biete-giyorgis")
            .unwrap();
        let Base::Fixed(base) = lm.base else {
            panic!("the rock-cut church needs its surveyed rim datum");
        };
        let lat = lm.lat_deg.to_radians();
        let lon = lm.lon_deg.to_radians();
        let cell = cell_at(lat, lon, 16);
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let active = active_for(cell, &frame, &|_, _| base + 35.0)
            .into_iter()
            .find(|active| active.lm.name == lm.name)
            .unwrap();
        let (floor, weight) = active.flattened_terrain(0.0, 0.0, base + 35.0).unwrap();
        assert!((floor - (base - 12.6)).abs() < 0.001);
        assert_eq!(weight, 1.0);
        assert_eq!(active.model_base_geo(0.0, 0.0), base);
        let below_floor = base - 20.0;
        assert_eq!(
            active.flattened_terrain(0.0, 0.0, below_floor).unwrap().0,
            below_floor
        );
        assert!(active.flattened_terrain(40.0, 40.0, base + 35.0).is_none());
    }

    #[test]
    fn opera_concourse_meets_corrected_terrain_across_the_full_podium() {
        let lm = LANDMARKS
            .iter()
            .find(|lm| lm.name == "sydney-opera-house")
            .unwrap();
        let lat = lm.lat_deg.to_radians();
        let lon = lm.lon_deg.to_radians();
        let anchor = ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
        let terrain = |sample_lat, sample_lon| {
            let sample = ellipsoid::geodetic_to_ecef(sample_lat, sample_lon, 0.0);
            if (sample - anchor).length() < 90.0 {
                60.0
            } else {
                3.1
            }
        };
        let cell = cell_at(lat, lon, 16);
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let active = active_for(cell, &frame, &terrain)
            .into_iter()
            .find(|active| active.lm.name == lm.name)
            .unwrap();
        for x in [-60.0, 0.0, 60.0] {
            let (corrected, weight) = active.flattened_terrain(x, 0.0, 60.0).unwrap();
            assert_eq!(weight, 1.0);
            assert!((corrected - 3.1).abs() < 0.01);
            assert!((active.model_base_geo(x, 0.0) + 3.1 - corrected).abs() < 0.01);
        }
    }

    #[test]
    fn long_landmark_coordinates_are_cell_independent() {
        let lm = LANDMARKS
            .iter()
            .find(|landmark| landmark.name == "golden-gate-bridge")
            .unwrap();
        let lat = lm.lat_deg.to_radians();
        let lon = lm.lon_deg.to_radians();
        let anchor_cell = cell_at(lat, lon, 15);
        let mut samples = Vec::new();
        for di in -1i64..=1 {
            for dj in -1i64..=1 {
                let i = anchor_cell.i as i64 + di;
                let j = anchor_cell.j as i64 + dj;
                if i < 0 || j < 0 {
                    continue;
                }
                let cell = CellId {
                    i: i as u32,
                    j: j as u32,
                    ..anchor_cell
                };
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                if let Some(active) = active_for(cell, &frame, &|_, _| 0.0)
                    .into_iter()
                    .find(|active| active.lm.name == lm.name)
                {
                    samples.push(active.column_at(lat, lon, 0.0).unwrap());
                }
            }
        }
        assert!(samples.len() >= 2, "bridge must span neighboring cells");
        for (mx, mz) in samples {
            assert!(
                mx.abs() < 1.0e-6 && mz.abs() < 1.0e-6,
                "anchor mapped to {mx},{mz}"
            );
        }
    }
}
