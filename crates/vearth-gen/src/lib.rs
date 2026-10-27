//! Deterministic surface generation from geospatial source data.
//!
//! Generated world content is a pure function of cell address, source data,
//! and `GEN_VERSION`: no wall clock or unseeded randomness affects semantic
//! output. Optional profiling also records elapsed time.

use glam::DVec3;
use std::collections::{BTreeMap, HashMap, HashSet};
use vearth_core::cell::{CellId, ChunkAddr, MAX_LEVEL};
use vearth_core::chunk::CHUNK_DIM;
use vearth_core::frame::ChunkFrame;

pub mod collision;
mod surface_cell;
pub use surface_cell::{ShadowOmission, SurfaceAppearance, SurfaceBatch, SurfaceCell};
mod surface_generation;
pub use surface_generation::{generate_surface_cell, generate_surface_cell_with_features};
pub mod agriculture_catalog;
pub mod architecture;
pub mod architecture_catalog;
pub mod architecture_choices;
pub mod architecture_facade;
pub mod architecture_places;
pub mod architecture_recipes;
pub mod architecture_resolver;
pub mod architecture_styles;
pub mod art_direction;
pub mod bridges;
mod building_composition;
mod building_surface;
pub mod built_surface;
pub mod contracts;
pub mod flora;
#[doc(hidden)]
pub mod flora_test_support;
pub mod flower_destinations;
mod great_wall;
mod hedgerow;
pub mod highrise;
pub mod landmarks;
pub mod observation_towers;
pub mod palette;
pub mod regional_architecture;
pub mod regional_finishes;
mod regional_landscape;
mod road_catalog;
pub mod road_materials;
mod road_surface;
pub mod tall_buildings;
mod terrain_appearance;
mod terrain_detail;
mod terrain_ecology;
mod terrain_pigment;
mod terrain_surface;
mod water_catalog;
mod water_color;
pub use terrain_detail::{TerrainDetailGrid, TerrainDetailRange, TerrainDetailSummary};
pub mod water_surface;
pub use terrain_surface::{TerrainCollision, TerrainCornerShading};
pub mod tier0;
pub mod tree_mesh;
pub mod tree_render;
pub mod vegetation;

use tier0::{BuildingStamp, FacadeStyle, GroundKind, RoadClass};

/// Cache namespace for generated output. Bump when generation changes incompatibly.
pub const GEN_VERSION: u32 = 211;

/// Cache schema can advance for a localized output change without
/// re-rolling unrelated terrain tint, bridge grading, or tree placement.
/// Raise this only when a deliberate planet-wide procedural reseed is part
/// of the art direction.
pub(crate) const PROCEDURAL_SEED_VERSION: u32 = 51;

/// Elevation is sampled at half the terrain-grid spacing (one tile zoom finer) so
/// peaks and ridges survive the corner-grid averaging.
const SAMPLE_ZOOM_FACTOR: f64 = 0.5;
/// At L7 the ordinary terrain hint is already approximately 1 km, making
/// this a source-continuous handoff. Coarser globe cells retain their normal
/// pyramid level and avoid prefetching a regional tile fan-out.
const WATER_BATHYMETRY_MIN_LEVEL: u8 = 7;
/// From regional LOD onward, ocean colour reads one fixed pyramid level
/// independent of terrain detail, so refined quadtree cells cannot jump into
/// rectangular tint bands. The elevation source carries no bathymetry (open
/// sea reads its nominal floor); a ~1 km hint keeps the tint lookup to a
/// handful of coarse source tiles per visible cell.
const WATER_BATHYMETRY_SAMPLE_M: f64 = 1_000.0;

/// Source-roof classification begins at L16; coarser ecology retains mapped
/// footprints. Roof geometry uses its own fixed-resolution source samples.
const SURFACE_MIN_LEVEL: u8 = 16;

pub mod material {
    use crate::vegetation::{
        FOLIAGE_MOTION_CLASS_COUNT, FOLIAGE_SEASON_CLASS_COUNT, FoliageMotionClass,
        FoliageSeasonClass, FoliageTraits,
    };

    pub const AIR: u8 = 0;
    pub const WATER: u8 = 1;
    pub const SAND: u8 = 2;
    pub const GRASS: u8 = 3;
    pub const ROCK: u8 = 4;
    pub const SNOW: u8 = 5;
    pub const SEABED: u8 = 6;
    // Map-derived surface and structure materials.
    pub const ASPHALT: u8 = 7;
    pub const PAVED: u8 = 8;
    pub const RAIL: u8 = 9;
    pub const PATH: u8 = 10;
    pub const WALL: u8 = 11;
    pub const ROOF: u8 = 12;
    pub const TRUNK: u8 = 13;
    pub const LEAVES: u8 = 14;
    // Glazing has an independent material response from masonry.
    pub const WINDOW: u8 = 15;
    // Landmark materials.
    pub const STEEL: u8 = 16;
    pub const MARBLE: u8 = 17;
    // Terminal (L19) representation of a deterministically occupied window
    // in a medium/tall building.
    pub const LIT_WINDOW: u8 = 18;
    // Coverage-quantized emissive facade voxels at L16-L18. They are a
    // spatial mip of the exact same stable room states as LIT_WINDOW, so the
    // city-light handoff stays attached to building surfaces.
    pub const LIT_FACADE_LOW: u8 = 19;
    pub const LIT_FACADE_MEDIUM: u8 = 20;
    pub const LIT_FACADE_HIGH: u8 = 21;

    /// Semantic crown materials.  The low two bits select motion and the
    /// remaining offset selects season; every variant maps back to the shared
    /// leaf art layer in the renderer rather than assuming id == atlas layer.
    pub const FOLIAGE_BASE: u8 = 22;
    pub const FOLIAGE_COUNT: u8 = FOLIAGE_SEASON_CLASS_COUNT * FOLIAGE_MOTION_CLASS_COUNT;
    pub const FOLIAGE_END: u8 = FOLIAGE_BASE + FOLIAGE_COUNT;

    // Persistent surface-response materials. Appearance families (granite,
    // red sandstone, savanna, etc.) remain palette choices and share these
    // ids unless their wetting/snow/texture response actually differs.
    pub const SOIL: u8 = FOLIAGE_END;
    pub const SALT: u8 = FOLIAGE_END + 1;
    pub const ICE: u8 = FOLIAGE_END + 2;
    pub const VOLCANIC: u8 = FOLIAGE_END + 3;
    pub const WETLAND: u8 = FOLIAGE_END + 4;
    /// Red obstruction lamp mounted on the tip of a generic observation
    /// tower. The renderer makes this material emissive only at night.
    pub const AVIATION_BEACON: u8 = FOLIAGE_END + 5;
    // Semantic field surfaces share existing grass/soil atlas art,
    // but distinct ids retain their visual grammar through meshing and let
    // weather/season response evolve without regenerating terrain.
    pub const FIELD_CROP: u8 = FOLIAGE_END + 6;
    pub const FIELD_FLOWER: u8 = FOLIAGE_END + 7;
    pub const FIELD_ORCHARD: u8 = FOLIAGE_END + 8;
    pub const FIELD_VINEYARD: u8 = FOLIAGE_END + 9;
    pub const FIELD_MEADOW: u8 = FOLIAGE_END + 10;
    pub const FIELD_DORMANT: u8 = FOLIAGE_END + 11;
    pub const FIELD_HARVESTED: u8 = FOLIAGE_END + 12;
    /// Steady warm architectural lamp authored onto a curated landmark.
    /// The fixture remains visible by day; the renderer adds emission only
    /// on the night side of the planet.
    pub const LANDMARK_LIGHT: u8 = FOLIAGE_END + 13;
    /// Eiffel-style sparkle fixture. It shares the steady night wash and
    /// receives a multi-colour animated pulse during the first five minutes
    /// of every simulation-clock hour.
    pub const LANDMARK_SPARKLE: u8 = FOLIAGE_END + 14;

    /// Physical construction families for inferred regional buildings.
    pub const BRICK_WALL: u8 = 81;
    pub const STONE_WALL: u8 = 82;
    pub const PLASTER_WALL: u8 = 83;
    pub const TIMBER_WALL: u8 = 84;
    pub const TILE_ROOF: u8 = 85;
    pub const SLATE_ROOF: u8 = 86;
    pub const PANEL_WALL: u8 = 87;
    pub const METAL_ROOF: u8 = 88;
    pub const SIDING_WALL: u8 = 89;
    pub const SHINGLE_ROOF: u8 = 90;
    pub const FLAT_ROOF: u8 = 91;
    pub const THATCH_ROOF: u8 = 92;
    pub const NAMAKO_WALL: u8 = 93;

    /// Steady, authored RGB emission on catalog signs and architectural fixtures.
    /// This is a physical surface, with ordinary depth, fog, and LOD ownership.
    pub const CITY_LIGHT: u8 = 94;

    /// Opaque catalog metal with specular lighting from the shared sky cache.
    /// Persistent surface payloads never own player-mesh texture-layer bits.
    pub const SCULPTURE_METAL: u8 = 95;

    /// Porous ashlar with worn joints, sampled in four-metre wall courses.
    pub const WEATHERED_LIMESTONE: u8 = 119;
    /// Continuous veined stone for carved and polished architectural surfaces.
    pub const VEINED_MARBLE: u8 = 120;
    /// Hand-applied earth plaster without a masonry bond pattern.
    pub const EARTHEN_PLASTER: u8 = 121;
    /// Inferred walking surfaces; physical variants share one batched atlas.
    pub const CONCRETE_PATH: u8 = 122;
    pub const BRICK_PAVING: u8 = 123;
    pub const STONE_PAVING: u8 = 124;
    pub const GRAVEL_PATH: u8 = 125;
    pub const COBBLESTONE: u8 = 126;
    pub const STONE_SLAB: u8 = 127;

    pub const fn is_architecture_wall(material: u8) -> bool {
        super::highrise::is_facade(material)
            || matches!(
                material,
                WALL | BRICK_WALL
                    | STONE_WALL
                    | PLASTER_WALL
                    | TIMBER_WALL
                    | PANEL_WALL
                    | SIDING_WALL
                    | NAMAKO_WALL
                    | WEATHERED_LIMESTONE
                    | VEINED_MARBLE
                    | EARTHEN_PLASTER
            )
    }

    pub const fn is_architecture_roof(material: u8) -> bool {
        matches!(
            material,
            ROOF | TILE_ROOF | SLATE_ROOF | METAL_ROOF | SHINGLE_ROOF | FLAT_ROOF | THATCH_ROOF
        )
    }

    pub const fn is_field_surface(material: u8) -> bool {
        material >= FIELD_CROP && material <= FIELD_HARVESTED
    }

    /// Surface materials whose upward voxel faces follow the low-frequency
    /// terrain slope for lighting while retaining block geometry for depth,
    /// silhouettes, and collision.
    pub const fn uses_macro_terrain_normal(material: u8) -> bool {
        matches!(
            material,
            SAND | GRASS
                | ROCK
                | SNOW
                | SEABED
                | ASPHALT
                | PAVED
                | RAIL
                | PATH
                | CONCRETE_PATH
                | BRICK_PAVING
                | STONE_PAVING
                | GRAVEL_PATH
                | COBBLESTONE
                | STONE_SLAB
                | SOIL
                | SALT
                | ICE
                | VOLCANIC
                | WETLAND
        ) || is_field_surface(material)
    }

    pub const fn foliage(traits: FoliageTraits) -> u8 {
        FOLIAGE_BASE + traits.season as u8 * FOLIAGE_MOTION_CLASS_COUNT + traits.motion as u8
    }

    pub const fn is_foliage(material: u8) -> bool {
        material >= FOLIAGE_BASE && material < FOLIAGE_END
    }

    /// Tree-crown materials that remain visually solid but do not block
    /// vehicles or the chase camera. `LEAVES` covers cached legacy trees;
    /// current trees use the semantic foliage range.
    pub const fn is_tree_foliage(material: u8) -> bool {
        material == LEAVES || is_foliage(material)
    }

    pub const fn foliage_traits(material: u8) -> Option<FoliageTraits> {
        if !is_foliage(material) {
            return None;
        }
        let value = material - FOLIAGE_BASE;
        let season = match value / FOLIAGE_MOTION_CLASS_COUNT {
            0 => FoliageSeasonClass::EvergreenNeedle,
            1 => FoliageSeasonClass::EvergreenBroadleaf,
            2 => FoliageSeasonClass::DeciduousYellow,
            3 => FoliageSeasonClass::DeciduousOrange,
            4 => FoliageSeasonClass::DeciduousRed,
            5 => FoliageSeasonClass::DeciduousConifer,
            6 => FoliageSeasonClass::DroughtDeciduous,
            7 => FoliageSeasonClass::SpringFlowering,
            8 => FoliageSeasonClass::OrchardFlowering,
            9 => FoliageSeasonClass::TropicalBroadleaf,
            10 => FoliageSeasonClass::Palm,
            _ => return None,
        };
        let motion = match value % FOLIAGE_MOTION_CLASS_COUNT {
            0 => FoliageMotionClass::Stiff,
            1 => FoliageMotionClass::Broadleaf,
            2 => FoliageMotionClass::Flexible,
            3 => FoliageMotionClass::Frond,
            _ => return None,
        };
        Some(FoliageTraits { season, motion })
    }
}

pub const NIGHT_LIGHT_MIN_LEVEL: u8 = tier0::BUILDINGS_MIN_LEVEL;
/// An empty proxy range preserves serialized-field compatibility while all
/// building LODs carry light on their exposed geometry.
pub const NIGHT_LIGHT_MAX_PROXY_LEVEL: u8 = NIGHT_LIGHT_MIN_LEVEL - 1;
/// Detail threshold retained by the disabled proxy builder.
pub const NIGHT_LIGHT_DETAIL_PACK_LEVEL: u8 = MAX_LEVEL - 6;
pub const NIGHT_LIGHT_MIN_HEIGHT_M: f32 = vearth_data::vectors::NIGHT_BUILDING_MIN_HEIGHT_M;
pub const NIGHT_LIGHT_BUCKETS: usize = 16;

/// Detached building-light sample retained in column and render formats for
/// compatibility. Current generation leaves this collection empty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NightLightCluster {
    /// Position in the cell's k=0 ChunkFrame, measured in voxel units.
    pub local_pos: [f32; 3],
    /// Abstract luminous surface energy (lit facade square metres). The GPU
    /// converts it to the brightness of one unresolved far-city sample.
    pub energy: f32,
    pub color: [u8; 3],
    /// 0..15, row-major in the cell's 4x4 conservation grid.
    pub bucket: u8,
}

/// Compact cell-local road centerline segment retained for optional gameplay
/// assistance. Endpoints are expressed in the k=0 chunk's horizontal voxel
/// coordinates, so the app can reconstruct them without keeping source MVT
/// geometry or issuing data-provider queries from the frame loop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadGuide {
    pub a_local: [f32; 2],
    pub b_local: [f32; 2],
    /// Unit direction continuing away from endpoint A along the same authored
    /// polyline, or zero when A terminates the source line.
    pub a_exit: [f32; 2],
    /// Unit direction continuing away from endpoint B along the same authored
    /// polyline, or zero when B terminates the source line.
    pub b_exit: [f32; 2],
    pub width_m: f32,
    /// Relative magnetic-assist strength. All rideable surfaces receive the
    /// same baseline; rails and bridges are omitted entirely.
    pub assist: f32,
}

/// Ground's geodetic corner lattice, including mapped road grades and shoulders.
/// Values are meters above the reference ellipsoid, without a cell-frame offset.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainSurfaceData {
    pub heights_m: Box<[f32; (N + 1) * (N + 1)]>,
    /// Shared-corner source normals and appearance. Synthetic fixtures may
    /// omit these and retain the mesh's local geometric fallback.
    pub shading: Option<Box<[TerrainCornerShading; (N + 1) * (N + 1)]>>,
    /// Active surface domain: one bit per horizontal quad in each row.
    pub source_columns: [u32; N],
}

fn include_source_height_range(
    mut range: (f32, f32),
    frame: &ChunkFrame,
    source: &TerrainSurfaceData,
) -> (f32, f32) {
    for z in 0..=N {
        for x in 0..=N {
            let incident = (z.saturating_sub(1)..=z.min(N - 1)).any(|cz| {
                (x.saturating_sub(1)..=x.min(N - 1))
                    .any(|cx| source.source_columns[cz] & (1 << cx) != 0)
            });
            if !incident {
                continue;
            }
            let source_m = source.heights_m[z * (N + 1) + x];
            let (_, offset_m) = frame.column_surface(x as f64 / N as f64, z as f64 / N as f64);
            let local_y = ((f64::from(source_m) + offset_m) / frame.grid_scale.y) as f32;
            let rendered_m = f64::from(local_y) * frame.grid_scale.y - offset_m;
            if !rendered_m.is_finite() {
                continue;
            }
            let rounded = rendered_m as f32;
            let lower = if f64::from(rounded) > rendered_m {
                rounded.next_down()
            } else {
                rounded
            };
            let upper = if f64::from(rounded) < rendered_m {
                rounded.next_up()
            } else {
                rounded
            };
            range.0 = range.0.min(source_m).min(lower);
            range.1 = range.1.max(source_m).max(upper);
        }
    }
    range
}

/// Re-exported source traits so the app depends on gen's view of data.
pub use vearth_data::vectors::VectorSource;

fn elevation_margin_for_pack(
    pack: &vearth_data::vectors::FeaturePack,
    frame: &ChunkFrame,
    level: u8,
) -> usize {
    let has_inland_polygon = pack.water.iter().any(|water| {
        !water.is_ocean()
            && (pack.water_polygon_mode
                == vearth_data::vectors::WaterPolygonMode::LegacyUndifferentiated
                || water.creates_surface_water())
    });
    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    let max_step_m = tier0::GridProj::of(frame, lat0, lon0).max_step_m as f32;
    let has_admitted_watercourse = pack.watercourse_output_mode
        == vearth_data::vectors::WatercourseOutputMode::RiversAndCanals
        && pack.watercourses.iter().any(|watercourse| {
            matches!(
                watercourse.kind,
                vearth_data::vectors::WatercourseKind::River
                    | vearth_data::vectors::WatercourseKind::Canal
            ) && !watercourse.intermittent
                && !watercourse.is_bridge
                && watercourse.source_tile.0 == watercourse.authority_zoom
                && watercourse.visible_at_terrain_level(level)
                && max_step_m <= watercourse.width_m * 2.0
        });
    let mut margin = if has_inland_polygon || has_admitted_watercourse {
        tier0::WATER_MARGIN_COLS
    } else {
        0
    };
    if level >= tier0::CLIFF_SURFACE_MIN_LEVEL && !pack.cliffs.is_empty() {
        margin = margin.max(1);
    }
    if !pack.exact_trees.is_empty() {
        let projection = tier0::GridProj::of(frame, lat0, lon0);
        let guard = (tier0::EXACT_TREE_CROWN_GUARD_M / projection.step_m as f32).ceil() as i32 + 1;
        for tree in &pack.exact_trees {
            let [cx, cz] = projection.to_cols(tree.point);
            if !cx.is_finite() || !cz.is_finite() {
                continue;
            }
            let (x, z) = (cx.floor() as i32, cz.floor() as i32);
            if x < -guard || z < -guard || x >= N as i32 + guard || z >= N as i32 + guard {
                continue;
            }
            let required = (-x).max(-z).max(x - (N as i32 - 1)).max(z - (N as i32 - 1));
            margin = margin.max(required.max(0) as usize);
        }
    }
    margin
}
pub use vearth_data::{ColorSource, ElevationSource};

/// The data surface generation draws from — normally the provider registry
/// (`vearth_data::registry::Registry` implements all three traits).
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    pub elevation: &'a dyn ElevationSource,
    /// LiDAR surface-model (DSM) channel: first-return height
    /// — ground plus built/vegetation mass — where covered. Drives nDSM
    /// building massing; None falls back to footprint extrusion.
    pub surface: Option<&'a dyn ElevationSource>,
    pub imagery: Option<&'a dyn ColorSource>,
    pub vectors: Option<&'a dyn VectorSource>,
}

impl<'a> Sources<'a> {
    pub fn terrain_only(elevation: &'a dyn ElevationSource) -> Self {
        Sources {
            elevation,
            surface: None,
            imagery: None,
            vectors: None,
        }
    }
}

/// Conservative geodetic footprint of every raster-like source query a cell
/// can make during generation.
///
/// The cap covers the interior terrain/imagery lattice, the maximum inland-
/// water margin, the full vector query window used by road conditioning, the
/// DSM roughness ring, and every active terrain-backed landmark anchor. It is
/// deliberately provider-agnostic: data providers can translate the public
/// cap into their own tile scheme without introducing a data -> gen
/// dependency. Angles are geodetic radians on the unit sphere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceFootprint {
    /// Geodetic latitude of the cap center, in radians.
    pub center_lat: f64,
    /// Geodetic longitude of the cap center, in radians.
    pub center_lon: f64,
    /// Great-circle angular radius of the cap, in radians (`0..=PI`).
    pub radius_rad: f64,
    /// Sampling-resolution hint passed to elevation, surface, and high-
    /// resolution imagery providers, in meters.
    pub sample_hint_m: f64,
}

impl SourceFootprint {
    /// Whether a geodetic source sample lies inside this conservative cap.
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        angular_distance(self.center_lat, self.center_lon, lat, lon)
            <= self.radius_rad + 32.0 * f64::EPSILON
    }
}

/// Elevation pitch for LOD-independent ground datums (water levels, building
/// bases, bridge ends), metres.
///
/// Eight metres selects the 9.6 m-at-the-equator terrain level (z13 of the
/// 512 px pyramid) through the mid-latitudes. Web Mercator pixels widen toward
/// the equator, so below about 33° the same pitch would select the next finer
/// level and quadruple the tiles behind every datum; this level's pixel is
/// used there instead, widened 1% so one prepared cap resolves to one zoom.
pub(crate) fn datum_pitch_m(lat: f64) -> f64 {
    const DATUM_EQUATOR_PIXEL_M: f64 = 40_075_016.686 / (512.0 * 8_192.0);
    (DATUM_EQUATOR_PIXEL_M * lat.cos().abs() * 1.01).max(8.0)
}

/// Fixed-detail source coverage near the features that sample it.
///
/// Water and building datums read elevation at a fixed pitch regardless of
/// cell level, so a single cap around a coarse cell would prepare full-detail
/// tiles across the whole cell for one lake or farmhouse. Geodetic rectangles
/// are snapped instead to a bucket grid centred on the cell, and each touched
/// bucket becomes one small cap. Longitudes are offsets from the cell centre,
/// so the grid never splits at the antimeridian.
pub(crate) struct DatumCover {
    center_lat: f64,
    center_lon: f64,
    step_lat: f64,
    step_lon: f64,
    sample_hint_m: f64,
    buckets: std::collections::BTreeSet<(i64, i64)>,
    /// Rectangles too wide for the bucket grid (near the poles), each as one cap.
    wide: Vec<SourceFootprint>,
}

impl DatumCover {
    /// Bucket count above which a rectangle is covered by a single cap.
    const MAX_BUCKETS: i64 = 4096;

    pub(crate) fn new(frame: &ChunkFrame, sample_hint_m: f64) -> Self {
        let (center_lat, center_lon) = frame.column_latlon(0.5, 0.5);
        let cell_m = frame.grid_scale.x.max(frame.grid_scale.z) * N as f64;
        let step_lat = (cell_m / 16.0).max(256.0) / vearth_core::ellipsoid::MEAN_RADIUS;
        Self {
            center_lat,
            center_lon,
            step_lat,
            step_lon: step_lat / center_lat.cos().abs().max(0.05),
            sample_hint_m,
            buckets: Default::default(),
            wide: Vec::new(),
        }
    }

    fn lon_offset(&self, lon: f64) -> f64 {
        (lon - self.center_lon + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI
    }

    /// Cover every point within `margin_m` of the geodetic rectangle spanned
    /// by `corners` (radians). Returns false, adding nothing, for non-finite input.
    pub(crate) fn add(&mut self, corners: &[(f64, f64)], margin_m: f64) -> bool {
        use std::f64::consts::FRAC_PI_2;
        let radius = vearth_core::ellipsoid::MEAN_RADIUS;
        let (mut lat_lo, mut lat_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut lon_lo, mut lon_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &(lat, lon) in corners {
            if !lat.is_finite() || !lon.is_finite() {
                return false;
            }
            let offset = self.lon_offset(lon);
            lat_lo = lat_lo.min(lat);
            lat_hi = lat_hi.max(lat);
            lon_lo = lon_lo.min(offset);
            lon_hi = lon_hi.max(offset);
        }
        if lat_lo > lat_hi {
            return false;
        }
        let lat_lo = (lat_lo - margin_m / radius).max(-FRAC_PI_2);
        let lat_hi = (lat_hi + margin_m / radius).min(FRAC_PI_2);
        let cos = lat_lo.cos().abs().min(lat_hi.cos().abs()).max(0.01);
        let lon_margin = margin_m / (radius * cos);
        let y0 = self.row(lat_lo);
        let y1 = self.row(lat_hi);
        let x0 = self.column(lon_lo - lon_margin);
        let x1 = self.column(lon_hi + lon_margin);
        if (y1 - y0 + 1).saturating_mul(x1 - x0 + 1) > Self::MAX_BUCKETS {
            self.wide.push(self.cap(
                lat_lo,
                lat_hi,
                self.center_lon + lon_lo - lon_margin,
                self.center_lon + lon_hi + lon_margin,
            ));
            return true;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.buckets.insert((y, x));
            }
        }
        true
    }

    fn row(&self, lat: f64) -> i64 {
        ((lat - self.center_lat) / self.step_lat + 0.5).floor() as i64
    }

    fn column(&self, lon_offset: f64) -> i64 {
        (lon_offset / self.step_lon + 0.5).floor() as i64
    }

    pub(crate) fn contains(&self, lat: f64, lon: f64) -> bool {
        self.buckets
            .contains(&(self.row(lat), self.column(self.lon_offset(lon))))
            || self.wide.iter().any(|cap| cap.contains(lat, lon))
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.buckets.is_empty() && self.wide.is_empty()
    }

    /// Cap containing a geodetic rectangle; longitudes may be unwrapped.
    fn cap(&self, lat0: f64, lat1: f64, lon0: f64, lon1: f64) -> SourceFootprint {
        use std::f64::consts::{PI, TAU};
        const SEGMENTS: usize = 16;
        let center_lat = (lat0 + lat1) * 0.5;
        let center_lon = ((lon0 + lon1) * 0.5 + PI).rem_euclid(TAU) - PI;
        // Meridian edges are great-circle arcs, so the cap holds them once it
        // holds their endpoints. Parallel edges bulge poleward of the chord
        // between samples by at most cos(lat) * (1 - cos(step / 2)).
        let step = (lon1 - lon0) / SEGMENTS as f64;
        let radius_rad = [lat0, lat1]
            .into_iter()
            .flat_map(|lat| (0..=SEGMENTS).map(move |i| (lat, lon0 + step * i as f64)))
            .map(|(lat, lon)| angular_distance(center_lat, center_lon, lat, lon))
            .fold(0.0, f64::max);
        let bulge = lat0.cos().abs().max(lat1.cos().abs()) * (1.0 - (step * 0.5).cos());
        let slack = 1.0 / vearth_core::ellipsoid::MEAN_RADIUS;
        SourceFootprint {
            center_lat,
            center_lon,
            radius_rad: (radius_rad + bulge + slack).min(PI),
            sample_hint_m: self.sample_hint_m,
        }
    }

    pub(crate) fn footprints(&self) -> Vec<SourceFootprint> {
        use std::f64::consts::FRAC_PI_2;
        self.buckets
            .iter()
            .map(|&(y, x)| {
                let lat0 = self.center_lat + (y as f64 - 0.5) * self.step_lat;
                let lon0 = self.center_lon + (x as f64 - 0.5) * self.step_lon;
                self.cap(
                    lat0.max(-FRAC_PI_2),
                    (lat0 + self.step_lat).min(FRAC_PI_2),
                    lon0,
                    lon0 + self.step_lon,
                )
            })
            .chain(self.wide.iter().copied())
            .collect()
    }
}

fn angular_distance(lat0: f64, lon0: f64, lat: f64, lon: f64) -> f64 {
    let cosine = lat0.sin() * lat.sin() + lat0.cos() * lat.cos() * (lon - lon0).cos();
    cosine.clamp(-1.0, 1.0).acos()
}

/// Plan the complete raster-source footprint for `cell` without consulting
/// source data or changing generation behavior.
///
/// The regular footprint starts from `vector_query_bounds`, whose margin is
/// wider than the maximum water ring and DSM neighborhood. The latitude and
/// longitude path lengths are added rather than combined diagonally: by the
/// triangle inequality this bounds every point in that local EN window, including
/// near cube-face seams. Exact expanded-cell boundary points cover the
/// cube-sphere mapping itself, and terrain-backed landmark anchors extend the
/// cap when a model reaches into the cell from outside the regular window.
pub fn source_query_footprint(cell: CellId) -> SourceFootprint {
    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let (center_lat, center_lon, half_e_m, half_n_m) = vector_query_bounds(cell);
    let radius = vearth_core::ellipsoid::MEAN_RADIUS;

    // Road-conditioning samples use lat0 + north/R, lon0 + east/(R*cos(lat0)).
    // Follow the center-latitude parallel east, then a meridian north: the
    // parallel's cosine cancels the longitude conversion. Sum both path
    // lengths to retain the road-profile margin beyond the window corners.
    // At a pole the only useful safe cap is the whole sphere.
    let north_rad = half_n_m / radius;
    let lon_rad = half_e_m / (radius * center_lat.cos().abs());
    let east_rad = if center_lat.cos().abs() <= 1.0e-12 {
        std::f64::consts::PI
    } else {
        half_e_m / radius
    };
    let mut radius_rad = (north_rad + east_rad).min(std::f64::consts::PI);

    let mut include = |lat: f64, lon: f64| {
        radius_rad = radius_rad.max(angular_distance(center_lat, center_lon, lat, lon));
    };

    // Inland water adds WATER_MARGIN_COLS on every side. Sampling its boundary
    // is a cheap exact guard for cube-sphere curvature and x/z scale
    // differences; the wider rectangular vector window above already bounds
    // exact-tree roots and the one-column cliff stencil.
    let margin = tier0::WATER_MARGIN_COLS as i64;
    let edge = N as i64 + margin;
    for q in -margin..=edge {
        let t = q as f64 / N as f64;
        for (fx, fz) in [
            (-margin as f64 / N as f64, t),
            (edge as f64 / N as f64, t),
            (t, -margin as f64 / N as f64),
            (t, edge as f64 / N as f64),
        ] {
            let (lat, lon) = frame.column_latlon(fx, fz);
            include(lat, lon);
        }
    }

    for (lat, lon) in landmarks::terrain_base_sample_points(cell, &frame) {
        include(lat, lon);
    }

    let wall = great_wall::Plan::for_frame(&frame);
    for (lat, lon) in wall.sample_points() {
        include(lat, lon);
    }

    // Raster planners clamp Mercator latitude and floor their longitude
    // cosine. Keep the established polar bound when that conversion can no
    // longer represent this cap; the sampler still uses the original longitude.
    if center_lat.abs() + radius_rad >= 1.484_422 {
        let polar_east = if center_lat.cos().abs() <= 1.0e-12 {
            std::f64::consts::PI
        } else {
            lon_rad
        };
        radius_rad = radius_rad.max((north_rad + polar_east).min(std::f64::consts::PI));
    }

    SourceFootprint {
        center_lat,
        center_lon,
        // Millimeter-scale numerical guard at Earth radius.
        radius_rad: (radius_rad + 1.0e-10).min(std::f64::consts::PI),
        sample_hint_m: if wall.is_empty() {
            frame.grid_scale.y * SAMPLE_ZOOM_FACTOR
        } else {
            (frame.grid_scale.y * SAMPLE_ZOOM_FACTOR).min(great_wall::SAMPLE_M)
        },
    }
}

/// Exactly the terrarium tiles `generate_surface_cell` will sample for this
/// cell. Callers can prefetch these into a `MemoryTiles` store before
/// generation. Deterministic and duplicate-free.
pub fn required_tiles(cell: CellId) -> Vec<(u8, u32, u32)> {
    use vearth_data::terrarium::{TerrariumSource, sampling_tiles};
    let frame0 = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let vertical_pitch_m = frame0.grid_scale.y * SAMPLE_ZOOM_FACTOR;
    let mut set = std::collections::BTreeSet::new();
    for gz in -1..=N as i64 + 1 {
        for gx in -1..=N as i64 + 1 {
            let (fx, fz) = (gx as f64 / N as f64, gz as f64 / N as f64);
            let (lat, lon) = frame0.column_latlon(fx, fz);
            let z = TerrariumSource::zoom_for(vertical_pitch_m, lat);
            set.extend(sampling_tiles(lat, lon, z));
        }
    }
    if cell.level >= WATER_BATHYMETRY_MIN_LEVEL {
        // Water tint samples column centres from a fixed regional bathymetry
        // level. Include those exact dependencies so async generation cannot
        // turn a cold cache miss into a zero-depth rectangular patch.
        for z in 0..N {
            for x in 0..N {
                let (fx, fz) = ((x as f64 + 0.5) / N as f64, (z as f64 + 0.5) / N as f64);
                let (lat, lon) = frame0.column_latlon(fx, fz);
                let zoom = TerrariumSource::zoom_for(WATER_BATHYMETRY_SAMPLE_M, lat);
                set.extend(sampling_tiles(lat, lon, zoom));
            }
        }
    }
    for (lat, lon) in landmarks::terrain_base_sample_points(cell, &frame0) {
        let z = TerrariumSource::zoom_for(vertical_pitch_m, lat);
        set.extend(sampling_tiles(lat, lon, z));
    }
    for (lat, lon) in great_wall::Plan::for_frame(&frame0).sample_points() {
        let z = TerrariumSource::zoom_for(great_wall::SAMPLE_M, lat);
        set.extend(sampling_tiles(lat, lon, z));
    }
    set.into_iter().collect()
}

const N: usize = CHUNK_DIM;

/// The geodetic window `generate_surface_cell` queries for vector features:
/// (lat0, lon0, half_e_m, half_n_m). Public so worker pools can prefetch
/// vector tiles concurrently with elevation tiles — the same contract idea
/// as `required_tiles`.
pub fn vector_query_bounds(cell: CellId) -> (f64, f64, f64, f64) {
    let frame0 = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let voxel = frame0.grid_scale;
    let (lat0, lon0) = frame0.column_latlon(0.5, 0.5);
    // Margin: canopy overhang (tree ring ~19 m + radius), road half-widths
    // reaching in from outside, and the water flatten window
    // (`WATER_MARGIN_COLS` columns).
    let margin =
        tier0::EXACT_TREE_CROWN_GUARD_M as f64 + voxel.x * (2.0 + tier0::WATER_MARGIN_COLS as f64);
    let half_e = voxel.x * (N as f64) * 0.5 + margin;
    let half_n = voxel.z * (N as f64) * 0.5 + margin;
    (lat0, lon0, half_e, half_n)
}

pub const ROAD_GUIDE_MIN_LEVEL: u8 = 17;
const MAX_ROAD_GUIDES_PER_CELL: usize = 2_048;

fn build_road_guides(
    cell: CellId,
    frame: &ChunkFrame,
    pack: &vearth_data::vectors::FeaturePack,
) -> Vec<RoadGuide> {
    use vearth_data::vectors::RoadClass;

    if cell.level < ROAD_GUIDE_MIN_LEVEL {
        return Vec::new();
    }
    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    let projection = tier0::GridProj::of(frame, lat0, lon0);
    // Include centerlines just outside the cell so the player's capture
    // envelope cannot blink while crossing a streamed-cell boundary.
    let reach_cols = (16.0 / projection.step_m) as f32 + 1.0;
    let mut guides = Vec::new();
    for road in &pack.roads {
        if road.is_bridge || road.class == RoadClass::Rail {
            continue;
        }
        let assist = match road.class {
            RoadClass::Highway => 0.9,
            RoadClass::Major | RoadClass::Medium => 1.0,
            RoadClass::Minor | RoadClass::Service => 0.9,
            RoadClass::Path => 0.9,
            RoadClass::Rail => unreachable!(),
        };
        for (segment_index, segment) in road.pts.windows(2).enumerate() {
            let dx = segment[1][0] - segment[0][0];
            let dz = segment[1][1] - segment[0][1];
            if dx * dx + dz * dz < 0.25 {
                continue;
            }
            let mut a_local = projection.to_cols(segment[0]);
            let mut b_local = projection.to_cols(segment[1]);
            let previous_local = segment_index
                .checked_sub(1)
                .map(|index| projection.to_cols(road.pts[index]));
            let next_local = road
                .pts
                .get(segment_index + 2)
                .map(|point| projection.to_cols(*point));
            if a_local[0].max(b_local[0]) < -reach_cols
                || a_local[0].min(b_local[0]) > N as f32 + reach_cols
                || a_local[1].max(b_local[1]) < -reach_cols
                || a_local[1].min(b_local[1]) > N as f32 + reach_cols
            {
                continue;
            }
            // Canonical endpoint order makes exact duplicates from adjacent
            // vector tiles cheap to remove deterministically.
            let swapped = (b_local[0], b_local[1]) < (a_local[0], a_local[1]);
            let (mut a_exit, mut b_exit) = if swapped {
                (
                    next_local.map_or([0.0; 2], |next| {
                        [next[0] - b_local[0], next[1] - b_local[1]]
                    }),
                    previous_local.map_or([0.0; 2], |previous| {
                        [previous[0] - a_local[0], previous[1] - a_local[1]]
                    }),
                )
            } else {
                (
                    previous_local.map_or([0.0; 2], |previous| {
                        [previous[0] - a_local[0], previous[1] - a_local[1]]
                    }),
                    next_local.map_or([0.0; 2], |next| {
                        [next[0] - b_local[0], next[1] - b_local[1]]
                    }),
                )
            };
            for exit in [&mut a_exit, &mut b_exit] {
                let length = (exit[0] * exit[0] + exit[1] * exit[1]).sqrt();
                if length > 1.0e-4 {
                    exit[0] /= length;
                    exit[1] /= length;
                } else {
                    *exit = [0.0; 2];
                }
            }
            if swapped {
                std::mem::swap(&mut a_local, &mut b_local);
            }
            guides.push(RoadGuide {
                a_local,
                b_local,
                a_exit,
                b_exit,
                width_m: road.width_m(),
                assist,
            });
        }
    }
    guides.sort_by(|a, b| {
        [a.a_local[0], a.a_local[1], a.b_local[0], a.b_local[1]]
            .into_iter()
            .zip([b.a_local[0], b.a_local[1], b.b_local[0], b.b_local[1]])
            .find_map(|(a, b)| {
                (a.total_cmp(&b) != std::cmp::Ordering::Equal).then(|| a.total_cmp(&b))
            })
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    guides.dedup();
    guides.truncate(MAX_ROAD_GUIDES_PER_CELL);
    guides
}

fn cell_may_intersect_flower_destination(cell: CellId, frame: &ChunkFrame) -> bool {
    if cell.level < tier0::GROUND_MIN_LEVEL {
        return false;
    }
    let (lat, lon) = frame.column_latlon(0.5, 0.5);
    let cell_radius_m = cell.size_m() * std::f64::consts::FRAC_1_SQRT_2 * 1.08;
    crate::flower_destinations::FLOWER_DESTINATIONS
        .iter()
        .any(|destination| {
            let (destination_lat, destination_lon, radius_m) =
                destination.influence.bounding_circle();
            geodetic_distance_m(lat, lon, destination_lat, destination_lon)
                <= cell_radius_m + radius_m
        })
}

fn geodetic_distance_m(lat_a: f64, lon_a: f64, lat_b: f64, lon_b: f64) -> f64 {
    let delta_lat = lat_b - lat_a;
    let delta_lon = (lon_b - lon_a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI;
    let haversine = (delta_lat * 0.5).sin().powi(2)
        + lat_a.cos() * lat_b.cos() * (delta_lon * 0.5).sin().powi(2);
    2.0 * vearth_core::ellipsoid::MEAN_RADIUS * haversine.sqrt().asin()
}

fn apply_curated_flower_destinations(
    cell: CellId,
    frame: &ChunkFrame,
    overlay: &mut tier0::CellOverlay,
    cols: &[[Col; N]; N],
) {
    use crate::flower_destinations::{DestinationMode, SurfaceMask};
    use vearth_data::vectors::FloraRegionKind;

    for z in 0..N {
        for x in 0..N {
            let (lat, lon) =
                frame.column_latlon((x as f64 + 0.5) / N as f64, (z as f64 + 0.5) / N as f64);
            let Some(destination) = crate::flower_destinations::query_flower_destination(lat, lon)
            else {
                continue;
            };
            let semantic = overlay.cols[z][x];
            if semantic.water
                || semantic.ocean
                || semantic.road.is_some()
                || semantic.building.is_some()
                || matches!(
                    semantic.ground,
                    Some(GroundKind::Paved | GroundKind::Urban | GroundKind::Sand)
                )
            {
                continue;
            }
            let mapped_surface = semantic.flora.and_then(|stamp| match stamp.kind {
                FloraRegionKind::Farmland => Some(SurfaceMask::FARMLAND),
                FloraRegionKind::Garden => Some(SurfaceMask::GARDEN),
                FloraRegionKind::Meadow => Some(SurfaceMask::MEADOW),
                FloraRegionKind::Orchard | FloraRegionKind::Vineyard => None,
            });
            let natural_surface = semantic.flora.is_none()
                && matches!(
                    semantic.ground,
                    None | Some(GroundKind::Green | GroundKind::NaturalGrass)
                );
            let eligible = match destination.mode {
                DestinationMode::ExactSite => mapped_surface.map_or(natural_surface, |surface| {
                    destination.allowed_surfaces.contains(surface)
                }),
                DestinationMode::MappedFieldOverride => mapped_surface
                    .is_some_and(|surface| destination.allowed_surfaces.contains(surface)),
                DestinationMode::EcologicalBoost => mapped_surface
                    .map_or(natural_surface, |surface| {
                        destination.allowed_surfaces.contains(surface)
                    }),
            };
            if !eligible {
                continue;
            }
            if destination.mode == DestinationMode::EcologicalBoost {
                // Ecological destinations are a permission to boost suitable
                // patches, not a regional paint operation. Evaluate the same
                // stable 16×16 patch address used by the descriptor before
                // changing the terrain surface, keeping ground colour and
                // procedural flower coverage in exact agreement.
                let mask_x = x * flora::FLORA_COVERAGE_EDGE / N;
                let mask_z = z * flora::FLORA_COVERAGE_EDGE / N;
                let sample_x = mask_x * (N / flora::FLORA_COVERAGE_EDGE)
                    + (N / flora::FLORA_COVERAGE_EDGE) / 2;
                let sample_z = mask_z * (N / flora::FLORA_COVERAGE_EDGE)
                    + (N / flora::FLORA_COVERAGE_EDGE) / 2;
                let environment =
                    wildflower_environment_at(Some(overlay), cols, sample_x, sample_z);
                let suitability = flora::curated_ecological_suitability(destination, environment);
                let fx = (mask_x as f64 + 0.5) / flora::FLORA_COVERAGE_EDGE as f64;
                let fz = (mask_z as f64 + 0.5) / flora::FLORA_COVERAGE_EDGE as f64;
                if !flora::wildflower_patch_exists_at(
                    flora::curated_field_seed(destination.id),
                    cell,
                    fx,
                    fz,
                    flora::WildflowerRoute::NaturalGrass,
                    suitability,
                ) {
                    continue;
                }
            }
            let kind = flora::curated_region_kind(destination);
            let seed = flora::curated_field_seed(destination.id);
            let col = &mut overlay.cols[z][x];
            col.flora = Some(tier0::FloraStamp {
                location_rad: None,
                kind,
                seed,
                curated_style: destination.id as u8,
                surface: flora::curated_field_surface(destination),
            });
            col.ground = Some(GroundKind::Green);
        }
    }
}

/// Resolve cultivation once per source identity so terrain and plant records
/// share the same regional recipe, including fields spanning many columns.
fn resolve_crop_catalog(
    overlay: &mut tier0::CellOverlay,
    frame: &ChunkFrame,
    cols: &[[Col; N]; N],
) {
    let mut fields =
        BTreeMap::<u64, (tier0::FloraStamp, [u32; flora::FLORA_COVERAGE_WORDS])>::new();
    for (z, row) in overlay.cols.iter().enumerate() {
        for (x, col) in row.iter().enumerate() {
            let Some(stamp) = col.flora.filter(|stamp| {
                stamp.curated_style == 0
                    && stamp.kind == vearth_data::vectors::FloraRegionKind::Farmland
                    && stamp.surface.kind == flora::FieldSurfaceKind::Crop
            }) else {
                continue;
            };
            let bit = (z * flora::FLORA_COVERAGE_EDGE / N) * flora::FLORA_COVERAGE_EDGE
                + x * flora::FLORA_COVERAGE_EDGE / N;
            fields
                .entry(stamp.seed)
                .or_insert((stamp, [0; flora::FLORA_COVERAGE_WORDS]))
                .1[bit / 32] |= 1 << (bit % 32);
        }
    }
    let recipes: BTreeMap<_, _> = fields
        .into_iter()
        .map(|(seed, (stamp, coverage))| {
            let environment = field_crop_environment(stamp, frame, overlay, cols, coverage);
            let mut resolved = stamp;
            if agriculture_catalog::infer_vineyard(seed, environment) {
                resolved.kind = vearth_data::vectors::FloraRegionKind::Vineyard;
                resolved.surface = flora::select_field_surface(resolved.kind, seed);
            } else {
                resolved.surface.palette = agriculture_catalog::crop_recipe(seed, environment)
                    .kind
                    .palette();
            }
            (seed, resolved)
        })
        .collect();
    for col in overlay.cols.iter_mut().flatten() {
        if let Some(stamp) = &mut col.flora
            && stamp.curated_style == 0
            && stamp.kind == vearth_data::vectors::FloraRegionKind::Farmland
            && stamp.surface.kind == flora::FieldSurfaceKind::Crop
            && let Some(&resolved) = recipes.get(&stamp.seed)
        {
            *stamp = resolved;
        }
    }
}

fn build_flora_descriptors(
    cell: CellId,
    frame: &ChunkFrame,
    overlay: Option<&tier0::CellOverlay>,
    cols: &[[Col; N]; N],
    landmark_hits: &[Vec<&'static landmarks::Landmark>],
    terrain: Option<&TerrainCollision>,
) -> Vec<flora::FloraCell> {
    const SOURCE_BLOCK: usize = N / flora::FLORA_COVERAGE_EDGE;
    debug_assert_eq!(SOURCE_BLOCK * flora::FLORA_COVERAGE_EDGE, N);

    let mut fields: BTreeMap<(u64, u8, u8), (tier0::FloraStamp, [u32; 8])> = BTreeMap::new();
    if let Some(overlay) = overlay {
        for mask_z in 0..flora::FLORA_COVERAGE_EDGE {
            for mask_x in 0..flora::FLORA_COVERAGE_EDGE {
                let mut winner = None;
                let mut eligible = true;
                for z in mask_z * SOURCE_BLOCK..(mask_z + 1) * SOURCE_BLOCK {
                    for x in mask_x * SOURCE_BLOCK..(mask_x + 1) * SOURCE_BLOCK {
                        if !landmark_hits.is_empty() && !landmark_hits[z * N + x].is_empty() {
                            eligible = false;
                            break;
                        }
                        let Some(stamp) = overlay.cols[z][x].flora else {
                            eligible = false;
                            break;
                        };
                        if winner.is_some_and(|current| current != stamp) {
                            eligible = false;
                            break;
                        }
                        winner = Some(stamp);
                    }
                    if !eligible {
                        break;
                    }
                }
                let Some(stamp) = winner.filter(|_| eligible) else {
                    continue;
                };
                let key = (stamp.seed, stamp.kind as u8, stamp.curated_style);
                let (_, coverage) = fields.entry(key).or_insert((stamp, [0; 8]));
                let bit = mask_z * flora::FLORA_COVERAGE_EDGE + mask_x;
                coverage[bit / 32] |= 1 << (bit % 32);
            }
        }
    }

    let sample_indices = [0, N / 4, N / 2, N * 3 / 4, N];
    let ground_m = std::array::from_fn(|index| {
        let x = sample_indices[index % flora::FLORA_GROUND_EDGE];
        let z = sample_indices[index / flora::FLORA_GROUND_EDGE];
        let height = terrain.and_then(|terrain| terrain.sample_local(x as f64, z as f64));
        height.map_or_else(
            || {
                let top = f64::from(cols[z.min(N - 1)][x.min(N - 1)].top);
                top
            },
            |height| height * frame.grid_scale.y,
        ) as f32
    });
    let (ground_base_cm, ground_cm) = flora::quantize_ground_grid(ground_m);
    let mut descriptors: Vec<_> = fields
        .into_values()
        .filter_map(|(stamp, mut coverage)| {
            let curated =
                crate::flower_destinations::FlowerDestinationId::from_u8(stamp.curated_style)
                    .map(crate::flower_destinations::flower_destination);
            if curated.is_none()
                && matches!(
                    stamp.kind,
                    vearth_data::vectors::FloraRegionKind::Garden
                        | vearth_data::vectors::FloraRegionKind::Meadow
                )
            {
                for mask_z in 0..flora::FLORA_COVERAGE_EDGE {
                    for mask_x in 0..flora::FLORA_COVERAGE_EDGE {
                        let bit = mask_z * flora::FLORA_COVERAGE_EDGE + mask_x;
                        if coverage[bit / 32] & (1 << (bit % 32)) == 0 {
                            continue;
                        }
                        let fx = (mask_x as f64 + 0.5) / flora::FLORA_COVERAGE_EDGE as f64;
                        let fz = (mask_z as f64 + 0.5) / flora::FLORA_COVERAGE_EDGE as f64;
                        let keep = if stamp.kind == vearth_data::vectors::FloraRegionKind::Garden {
                            flora::garden_patch_at(stamp.seed, cell, fx, fz)
                                == flora::GardenPatch::FlowerBed
                        } else {
                            let x = mask_x * SOURCE_BLOCK + SOURCE_BLOCK / 2;
                            let z = mask_z * SOURCE_BLOCK + SOURCE_BLOCK / 2;
                            let environment = wildflower_environment_at(overlay, cols, x, z);
                            let suitability = flora::wildflower_suitability(
                                flora::WildflowerRoute::MappedMeadow,
                                environment,
                            );
                            flora::wildflower_patch_exists_at(
                                stamp.seed,
                                cell,
                                fx,
                                fz,
                                flora::WildflowerRoute::MappedMeadow,
                                suitability,
                            )
                        };
                        if !keep {
                            coverage[bit / 32] &= !(1 << (bit % 32));
                        }
                    }
                }
            }
            if coverage.iter().all(|word| *word == 0) {
                return None;
            }
            let environment = field_crop_environment(
                stamp,
                frame,
                overlay.expect("mapped flora"),
                cols,
                coverage,
            );
            if let Some(destination) = curated
                && destination.mode == crate::flower_destinations::DestinationMode::EcologicalBoost
            {
                for mask_z in 0..flora::FLORA_COVERAGE_EDGE {
                    for mask_x in 0..flora::FLORA_COVERAGE_EDGE {
                        let bit = mask_z * flora::FLORA_COVERAGE_EDGE + mask_x;
                        if coverage[bit / 32] & (1 << (bit % 32)) == 0 {
                            continue;
                        }
                        let x = mask_x * SOURCE_BLOCK + SOURCE_BLOCK / 2;
                        let z = mask_z * SOURCE_BLOCK + SOURCE_BLOCK / 2;
                        let wild_environment = wildflower_environment_at(overlay, cols, x, z);
                        let suitability =
                            flora::curated_ecological_suitability(destination, wild_environment);
                        let fx = (mask_x as f64 + 0.5) / flora::FLORA_COVERAGE_EDGE as f64;
                        let fz = (mask_z as f64 + 0.5) / flora::FLORA_COVERAGE_EDGE as f64;
                        if !flora::wildflower_patch_exists_at(
                            stamp.seed,
                            cell,
                            fx,
                            fz,
                            flora::WildflowerRoute::NaturalGrass,
                            suitability,
                        ) {
                            coverage[bit / 32] &= !(1 << (bit % 32));
                        }
                    }
                }
                if coverage.iter().all(|word| *word == 0) {
                    return None;
                }
            }
            let archetype = curated.map_or_else(
                || {
                    agriculture_catalog::treatment_for_palette(stamp.surface.palette)
                        .filter(|_| stamp.surface.kind == flora::FieldSurfaceKind::Crop)
                        .map_or_else(
                            || {
                                flora::descriptor_archetype(
                                    stamp.kind,
                                    stamp.surface.kind,
                                    stamp.seed,
                                    environment,
                                )
                            },
                            |treatment| treatment.archetype,
                        )
                },
                flora::curated_archetype,
            );
            let calendar = if agriculture_catalog::treatment_for_palette(stamp.surface.palette)
                .is_some_and(|treatment| treatment.kind == agriculture_catalog::CropKind::Tea)
            {
                flora::CultivationCalendar::PerennialOrchard
            } else {
                flora::select_cultivation_calendar(archetype, environment, stamp.curated_style)
            };
            Some(flora::FloraCell {
                archetype,
                calendar,
                field_seed: stamp.seed,
                row_heading_rad: stamp.surface.heading_rad() as f32,
                palette: stamp.surface.palette,
                base_density: curated.map_or_else(
                    || flora::descriptor_density(archetype),
                    |entry| entry.density,
                ),
                coverage,
                ground_base_cm,
                ground_cm,
                phenology_offset: curated.map_or_else(
                    || flora::descriptor_phenology_offset(stamp.seed),
                    flora::curated_phenology_offset,
                ),
                curated_style: stamp.curated_style,
            })
        })
        .collect();

    // Natural-grass inference requires a loaded vector overlay, even when the
    // pack has no explicit meadow, because the overlay supplies the mandatory
    // forest, cultivation, road, building, water, and urban exclusions.
    if cell.level >= flora::NATURAL_WILDFLOWER_MIN_LEVEL
        && let Some(natural_overlay) = overlay
    {
        let seed = flora::ecological_region_seed(cell);
        let mut coverage = [0u32; flora::FLORA_COVERAGE_WORDS];
        for mask_z in 0..flora::FLORA_COVERAGE_EDGE {
            for mask_x in 0..flora::FLORA_COVERAGE_EDGE {
                let eligible = (mask_z * SOURCE_BLOCK..(mask_z + 1) * SOURCE_BLOCK).all(|z| {
                    (mask_x * SOURCE_BLOCK..(mask_x + 1) * SOURCE_BLOCK).all(|x| {
                        natural_wildflower_surface(Some(natural_overlay), cols, landmark_hits, x, z)
                    })
                });
                if !eligible {
                    continue;
                }
                let x = mask_x * SOURCE_BLOCK + SOURCE_BLOCK / 2;
                let z = mask_z * SOURCE_BLOCK + SOURCE_BLOCK / 2;
                let environment = wildflower_environment_at(Some(natural_overlay), cols, x, z);
                let suitability = flora::wildflower_suitability(
                    flora::WildflowerRoute::NaturalGrass,
                    environment,
                );
                let fx = (mask_x as f64 + 0.5) / flora::FLORA_COVERAGE_EDGE as f64;
                let fz = (mask_z as f64 + 0.5) / flora::FLORA_COVERAGE_EDGE as f64;
                if flora::wildflower_patch_exists_at(
                    seed,
                    cell,
                    fx,
                    fz,
                    flora::WildflowerRoute::NaturalGrass,
                    suitability,
                ) {
                    let bit = mask_z * flora::FLORA_COVERAGE_EDGE + mask_x;
                    coverage[bit / 32] |= 1 << (bit % 32);
                }
            }
        }
        if coverage.iter().any(|word| *word != 0) {
            let environment = crop_environment(frame, natural_overlay, cols, coverage);
            descriptors.push(flora::FloraCell {
                archetype: flora::FloraArchetype::MeadowWildflowers,
                calendar: flora::select_cultivation_calendar(
                    flora::FloraArchetype::MeadowWildflowers,
                    environment,
                    0,
                ),
                field_seed: seed,
                row_heading_rad: 0.0,
                palette: flora::select_field_surface(
                    vearth_data::vectors::FloraRegionKind::Meadow,
                    seed,
                )
                .palette,
                base_density: 72,
                coverage,
                ground_base_cm,
                ground_cm,
                phenology_offset: flora::descriptor_phenology_offset(seed),
                curated_style: 0,
            });
        }
    }
    descriptors
}

fn nearby_water(overlay: Option<&tier0::CellOverlay>, x: usize, z: usize) -> bool {
    let Some(overlay) = overlay else {
        return false;
    };
    let radius = 4usize;
    (z.saturating_sub(radius)..=(z + radius).min(N - 1)).any(|near_z| {
        (x.saturating_sub(radius)..=(x + radius).min(N - 1)).any(|near_x| {
            overlay.cols[near_z][near_x].water_ecology || overlay.cols[near_z][near_x].ocean
        })
    })
}

fn wildflower_environment_at(
    overlay: Option<&tier0::CellOverlay>,
    cols: &[[Col; N]; N],
    x: usize,
    z: usize,
) -> flora::WildflowerEnvironment {
    let col = cols[z][x];
    let snow_exposure = match classify_surface(&col) {
        SurfaceFamily::Ice | SurfaceFamily::Snow => 1.0,
        _ => ((1.0 - col.climate.temp_c) / 7.0).clamp(0.0, 1.0),
    };
    flora::WildflowerEnvironment {
        temp_c: col.climate.temp_c,
        elevation_m: col.elev,
        slope_rad: col.slope,
        relief_m: col.relief,
        aridity: col.climate.arid,
        precipitation_mm_day: col.climate.precipitation_mm_day,
        humidity: col.climate.humidity,
        near_water: nearby_water(overlay, x, z),
        snow_exposure,
    }
}

fn natural_wildflower_surface(
    overlay: Option<&tier0::CellOverlay>,
    cols: &[[Col; N]; N],
    landmark_hits: &[Vec<&'static landmarks::Landmark>],
    x: usize,
    z: usize,
) -> bool {
    if !landmark_hits.is_empty() && !landmark_hits[z * N + x].is_empty() {
        return false;
    }
    if let Some(overlay) = overlay {
        let semantic = overlay.cols[z][x];
        if semantic.flora.is_some()
            || semantic.water
            || semantic.ocean
            || semantic.road.is_some()
            || semantic.building.is_some()
            || !matches!(
                semantic.ground,
                None | Some(GroundKind::Green | GroundKind::NaturalGrass)
            )
        {
            return false;
        }
    }
    let col = cols[z][x];
    matches!(
        classify_surface(&col),
        SurfaceFamily::Temperate
            | SurfaceFamily::Steppe
            | SurfaceFamily::Chaparral
            | SurfaceFamily::Tundra
    ) || (classify_surface(&col) == SurfaceFamily::Boreal && col.elev >= 1_200.0)
}

fn field_crop_environment(
    stamp: tier0::FloraStamp,
    frame: &ChunkFrame,
    overlay: &tier0::CellOverlay,
    cols: &[[Col; N]; N],
    coverage: [u32; flora::FLORA_COVERAGE_WORDS],
) -> flora::CropEnvironment {
    let Some(location) = stamp
        .location_rad
        .filter(|location| location.iter().all(|v| v.is_finite()))
    else {
        return crop_environment(frame, overlay, cols, coverage);
    };
    let [lat, lon] = flora::field_context_location(location);
    // Annual source normals already contain broad orography. Local clipped
    // masks, terrain relief and nearby-water samples must not reselect the
    // crop or calendar as the same field crosses a cell/LOD boundary.
    let climate = climate_at(lat, lon, 0.0);
    flora::CropEnvironment {
        lat_deg: lat.to_degrees() as f32,
        lon_deg: lon.to_degrees() as f32,
        temp_c: climate.temp_c,
        elevation_m: 0.0,
        slope_rad: 0.0,
        relief_m: 0.0,
        aridity: climate.arid,
        precipitation_mm_day: climate.precipitation_mm_day,
        humidity: climate.humidity,
        near_water: false,
    }
}

fn crop_environment(
    frame: &ChunkFrame,
    overlay: &tier0::CellOverlay,
    cols: &[[Col; N]; N],
    coverage: [u32; flora::FLORA_COVERAGE_WORDS],
) -> flora::CropEnvironment {
    const SOURCE_BLOCK: usize = N / flora::FLORA_COVERAGE_EDGE;
    let covered = |mask_x: usize, mask_z: usize| {
        let bit = mask_z * flora::FLORA_COVERAGE_EDGE + mask_x;
        coverage[bit / 32] & (1 << (bit % 32)) != 0
    };
    let mut sample_count = 0.0f32;
    let mut center_x = 0.0f64;
    let mut center_z = 0.0f64;
    let mut elevation_m = 0.0f32;
    let mut slope_rad = 0.0f32;
    let mut relief_m = 0.0f32;
    let mut temp_c = 0.0f32;
    let mut aridity = 0.0f32;
    let mut precipitation_mm_day = 0.0f32;
    let mut humidity = 0.0f32;
    let mut near_water = false;
    for mask_z in 0..flora::FLORA_COVERAGE_EDGE {
        for mask_x in 0..flora::FLORA_COVERAGE_EDGE {
            if !covered(mask_x, mask_z) {
                continue;
            }
            let x = mask_x * SOURCE_BLOCK + SOURCE_BLOCK / 2;
            let z = mask_z * SOURCE_BLOCK + SOURCE_BLOCK / 2;
            let col = cols[z][x];
            sample_count += 1.0;
            center_x += x as f64 + 0.5;
            center_z += z as f64 + 0.5;
            elevation_m += col.elev;
            slope_rad += col.slope;
            relief_m += col.relief;
            temp_c += col.climate.temp_c;
            aridity += col.climate.arid;
            precipitation_mm_day += col.climate.precipitation_mm_day;
            humidity += col.climate.humidity;
            let radius = 4usize;
            for near_z in z.saturating_sub(radius)..=(z + radius).min(N - 1) {
                for near_x in x.saturating_sub(radius)..=(x + radius).min(N - 1) {
                    near_water |= overlay.cols[near_z][near_x].water_ecology
                        || overlay.cols[near_z][near_x].ocean;
                }
            }
        }
    }
    let divisor = sample_count.max(1.0);
    let (lat, lon) = frame.column_latlon(
        center_x / f64::from(divisor) / N as f64,
        center_z / f64::from(divisor) / N as f64,
    );
    flora::CropEnvironment {
        lat_deg: lat.to_degrees() as f32,
        lon_deg: lon.to_degrees() as f32,
        temp_c: temp_c / divisor,
        elevation_m: elevation_m / divisor,
        slope_rad: slope_rad / divisor,
        relief_m: relief_m / divisor,
        aridity: aridity / divisor,
        precipitation_mm_day: precipitation_mm_day / divisor,
        humidity: humidity / divisor,
        near_water,
    }
}

/// Everything precomputed per voxel column.
#[derive(Clone, Copy)]
struct Col {
    top: f32,
    sea: f32,
    elev: f32,
    /// Positive ocean depth from the regional colour source level (or the
    /// ordinary globe pyramid before its L7 handoff). This is deliberately
    /// separate from `elev`, whose fine resolution follows terrain LOD and
    /// therefore must not drive local water colour.
    bathymetry: f32,
    /// Geographic color fields shared across cells and LODs.
    landscape_variation: [f32; 2],
    slope: f32,
    /// Peak-to-trough elevation in a compact neighbourhood around this
    /// column. Unlike slope, this detects broad mesas, canyon benches and
    /// mountain massifs without adding another data source.
    relief: f32,
    climate: Climate,
    regional: regional_landscape::Palette,
    /// Real surface color (Blue Marble et al.), when a source covers here.
    img: Option<[u8; 3]>,
    /// Bundled regional colour (~2 km/px), available without HTTP at local
    /// and regional views but deliberately distinct from roof-scale ortho.
    img_detail: Option<[u8; 3]>,
    /// Regional ortho color: an aerial pixel at street LODs.
    /// Unlike `img` this is trusted nearly outright — 8 cm pixels are not
    /// mush at 0.6 m voxels.
    img_hr: Option<[u8; 3]>,
}

impl Col {
    const EMPTY: Col = Col {
        top: 0.0,
        sea: f32::NEG_INFINITY,
        elev: 0.0,
        bathymetry: 0.0,
        landscape_variation: [0.0; 2],
        regional: regional_landscape::Palette::EMPTY,
        slope: 0.0,
        relief: 0.0,
        climate: Climate {
            temp_c: 15.0,
            arid: 0.0,
            precipitation_mm_day: 1.5,
            humidity: 0.55,
        },
        img: None,
        img_detail: None,
        img_hr: None,
    };
}

/// Disabled detached-proxy builder retained while the serialized column and
/// render formats still carry compatibility fields. The empty level range
/// returns before any source data is inspected.
/// Time-neutral observed climate normals. The monthly atlas is already
/// bundled for weather; averaging it here avoids a second payload and
/// distinguishes wet and dry regions at equal latitudes.
#[derive(Clone, Copy)]
struct Climate {
    temp_c: f32,
    arid: f32,
    precipitation_mm_day: f32,
    humidity: f32,
}

fn climate_at(lat: f64, lon: f64, elevation_m: f32) -> Climate {
    let observed = vearth_data::climate::ClimateAtlas::bundled().annual_surface(lat, lon);
    let precipitation_dryness =
        (1.0 - (observed.precipitation_mm_day - 0.20) / (3.0 - 0.20)).clamp(0.0, 1.0);
    let humidity_dryness = (1.0 - observed.relative_humidity).clamp(0.0, 1.0);
    Climate {
        // The atlas already contains broad orography, so use a deliberately
        // weak lapse correction: enough for locally resolved summits above a
        // coarse climate cell, without double-cooling whole mountain ranges.
        temp_c: observed.mean_temperature_c - 2.4 * (elevation_m.max(0.0) / 1000.0),
        arid: (precipitation_dryness * 0.72 + humidity_dryness * 0.28).clamp(0.0, 1.0),
        precipitation_mm_day: observed.precipitation_mm_day,
        humidity: observed.relative_humidity,
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn tint(base: [f32; 3], shade: f32) -> [u8; 3] {
    [
        (base[0] * shade).clamp(0.0, 255.0) as u8,
        (base[1] * shade).clamp(0.0, 255.0) as u8,
        (base[2] * shade).clamp(0.0, 255.0) as u8,
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SurfaceFamily {
    Seabed,
    Ice,
    Snow,
    Volcanic,
    Slate,
    Granite,
    Limestone,
    RedSedimentary,
    TanSedimentary,
    Scree,
    Salt,
    PaleSand,
    GoldSand,
    RedSand,
    Clay,
    Loess,
    Alluvium,
    Tundra,
    Boreal,
    Temperate,
    Tropical,
    Steppe,
    Savanna,
    Chaparral,
    Wetland,
    Peat,
}

const ROCK_SEMANTIC_SLOPE_RAD: f32 = 0.55;
const ROCK_BLEND_START_RAD: f32 = 0.42;
const ROCK_BLEND_END_RAD: f32 = 0.70;
const ROCK_RELIEF_START_M: f32 = 50.0;
const ROCK_RELIEF_END_M: f32 = 180.0;
const ROCK_MATERIAL_EXPOSURE: f32 = 0.75;

fn source_rgb(col: &Col) -> Option<[f32; 3]> {
    // Material identity uses a stable regional footprint. Roof-scale pixels
    // can contain a shadow, crop row or canopy and vary with the active LOD.
    // They still supply appearance below, and serve as identity fallback when
    // the provider has no regional imagery at all.
    col.img_detail.or(col.img).or(col.img_hr).map(|rgb| {
        [
            rgb[0] as f32 / 255.0,
            rgb[1] as f32 / 255.0,
            rgb[2] as f32 / 255.0,
        ]
    })
}

/// Classify persistent physical/appearance families from complementary cues.
/// Imagery supplies hue; observed climate says whether green/brown is likely
/// vegetation or bare soil; slope and local relief keep cliffs and mesas out
/// of the lawn material. All thresholds overlap broad continuous palette
/// blends below, so the result is stable rather than random colour noise.
fn classify_surface(col: &Col) -> SurfaceFamily {
    classify_surface_with_steep_rock(col, col.slope > ROCK_SEMANTIC_SLOPE_RAD)
}

/// Evaluate the ordinary classifier with an explicit steep-rock decision.
/// Semantic consumers retain the existing hard threshold, while terrain
/// colour evaluates both endpoints and blends between them continuously.
fn classify_surface_with_steep_rock(col: &Col, steep_rock: bool) -> SurfaceFamily {
    if col.elev < 0.0 && col.sea > f32::NEG_INFINITY {
        return SurfaceFamily::Seabed;
    }
    let temp = col.climate.temp_c;
    let observed = source_rgb(col);
    let snow_evidence = observed.map_or_else(
        || temp < -8.0 && col.climate.precipitation_mm_day > 0.70 && col.climate.humidity > 0.50,
        |rgb| {
            let low = rgb[0].min(rgb[1]).min(rgb[2]);
            let high = rgb[0].max(rgb[1]).max(rgb[2]);
            low > 0.64 && high - low < 0.16 && rgb[0] - rgb[2] < 0.10
        },
    );
    // Elevation and a cold annual mean alone cannot establish persistent
    // snow: dry plateaus and polar deserts can remain exposed. Explicit
    // mapped glaciers are resolved separately and retain their authority.
    if temp < -12.0 && snow_evidence {
        return SurfaceFamily::Ice;
    }
    if temp < 1.0 && snow_evidence {
        return SurfaceFamily::Snow;
    }

    let rgb = observed.unwrap_or([0.44, 0.42, 0.36]);
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    let min = rgb[0].min(rgb[1]).min(rgb[2]);
    let chroma = max - min;
    let value = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
    let warmth = rgb[0] - rgb[2];
    let red_excess = rgb[0] - rgb[1];
    let redness = rgb[0] - (rgb[1] + rgb[2]) * 0.5;
    let green_excess = rgb[1] - (rgb[0] + rgb[2]) * 0.5;
    let arid = col.climate.arid;
    let rugged = col.slope > 0.28 || col.relief > 90.0;

    if arid > 0.55 && value > 0.72 && chroma < 0.17 && col.slope < 0.20 {
        return SurfaceFamily::Salt;
    }
    // Regional forest imagery is often olive: R ~= G with low B. Warmth
    // alone therefore cannot establish a sedimentary rock appearance.
    if arid > 0.40 && red_excess > 0.055 && warmth > 0.08 {
        return if rugged {
            SurfaceFamily::RedSedimentary
        } else if redness > 0.11 {
            SurfaceFamily::RedSand
        } else {
            SurfaceFamily::Clay
        };
    }
    if arid > 0.34 && red_excess > 0.03 && warmth > 0.055 && rugged {
        return SurfaceFamily::TanSedimentary;
    }
    if value < 0.31 && chroma < 0.12 && warmth < 0.08 && green_excess < 0.025 {
        return SurfaceFamily::Volcanic;
    }
    if steep_rock {
        return if value > 0.62 && chroma < 0.18 {
            SurfaceFamily::Limestone
        } else if warmth > 0.045 {
            SurfaceFamily::Granite
        } else if value < 0.38 {
            SurfaceFamily::Slate
        } else {
            SurfaceFamily::Scree
        };
    }
    if arid > 0.68 && temp > 8.0 {
        return if redness > 0.035 {
            SurfaceFamily::RedSand
        } else if value > 0.66 {
            SurfaceFamily::PaleSand
        } else {
            SurfaceFamily::GoldSand
        };
    }
    if source_rgb(col).is_some()
        && value < 0.38
        && green_excess < 0.06
        && col.climate.precipitation_mm_day > 3.5
        && col.climate.humidity > 0.70
        && col.relief < 30.0
    {
        return if temp < 8.0 {
            SurfaceFamily::Peat
        } else {
            SurfaceFamily::Wetland
        };
    }
    if arid > 0.50 {
        return if temp > 20.0 {
            SurfaceFamily::Savanna
        } else if warmth > 0.04 {
            SurfaceFamily::Loess
        } else {
            SurfaceFamily::Steppe
        };
    }
    if green_excess < -0.025 && warmth > 0.04 {
        return SurfaceFamily::Alluvium;
    }
    if temp < 1.5 {
        SurfaceFamily::Tundra
    } else if temp < 9.0 {
        SurfaceFamily::Boreal
    } else if arid > 0.35 {
        SurfaceFamily::Chaparral
    } else if temp < 19.0 {
        SurfaceFamily::Temperate
    } else {
        SurfaceFamily::Tropical
    }
}

fn rock_exposure(col: &Col) -> f32 {
    let slope = smoothstep01(
        (col.slope - ROCK_BLEND_START_RAD) / (ROCK_BLEND_END_RAD - ROCK_BLEND_START_RAD),
    );
    let relief = smoothstep01(
        (col.relief - ROCK_RELIEF_START_M) / (ROCK_RELIEF_END_M - ROCK_RELIEF_START_M),
    );
    // Relief is sampled over a fixed number of columns, so its world-space
    // support shrinks during refinement. It may support a moderate slope,
    // but cannot keep a resolved cliff grassy at the finest LOD.
    let relief_support = 0.65 + 0.35 * relief;
    slope * (relief_support + (1.0 - relief_support) * slope)
}

fn surface_style(family: SurfaceFamily, col: &Col, sample_alt: f32) -> (u8, [f32; 3]) {
    use palette::terrain as p;
    let temp = col.climate.temp_c;
    let arid = col.climate.arid;
    let (mat, base) = match family {
        SurfaceFamily::Seabed => (material::SEABED, p::SEABED),
        SurfaceFamily::Ice => (
            material::ICE,
            lerp3(
                p::GLACIER_ICE,
                p::DEEP_SNOW,
                ((-12.0 - temp) / 20.0).clamp(0.0, 1.0),
            ),
        ),
        SurfaceFamily::Snow => (material::SNOW, lerp3(p::OLD_SNOW, p::SNOW, 0.70)),
        SurfaceFamily::Volcanic => (material::VOLCANIC, lerp3(p::BASALT, p::VOLCANIC_ASH, 0.28)),
        SurfaceFamily::Slate => (material::ROCK, p::SLATE),
        SurfaceFamily::Granite => (material::ROCK, lerp3(p::GRANITE, p::PALE_GRANITE, 0.25)),
        SurfaceFamily::Limestone => (material::ROCK, p::LIMESTONE),
        SurfaceFamily::RedSedimentary => {
            // Broad, continuous strata make canyon walls legible without
            // pretending that this 2 km imagery knows exact formations.
            let phase = 0.5 + 0.5 * (sample_alt * std::f32::consts::TAU / 185.0).sin();
            let pale = (0.5 + 0.5 * (sample_alt * std::f32::consts::TAU / 410.0).cos()).powf(7.0);
            let layered = lerp3(p::RED_SANDSTONE, p::TAN_SANDSTONE, phase * 0.48);
            (
                material::ROCK,
                lerp3(layered, p::WARM_LIMESTONE, pale * 0.32),
            )
        }
        SurfaceFamily::TanSedimentary => (
            material::ROCK,
            lerp3(p::TAN_SANDSTONE, p::WARM_LIMESTONE, 0.28),
        ),
        SurfaceFamily::Scree => (material::ROCK, p::SCREE),
        SurfaceFamily::Salt => (material::SALT, lerp3(p::SALT, p::GYPSUM, 0.35)),
        SurfaceFamily::PaleSand => (material::SAND, p::PALE_SAND),
        SurfaceFamily::GoldSand => (material::SAND, lerp3(p::DUNE_SAND, p::GOLD_DUNE, arid)),
        SurfaceFamily::RedSand => (material::SAND, lerp3(p::IRON_SAND, p::RED_DUNE, arid)),
        SurfaceFamily::Clay => (material::SOIL, lerp3(p::DRY_CLAY, p::RED_CLAY, 0.55)),
        SurfaceFamily::Loess => (material::SOIL, p::LOESS),
        SurfaceFamily::Alluvium => (material::SOIL, p::ALLUVIUM),
        SurfaceFamily::Tundra
        | SurfaceFamily::Boreal
        | SurfaceFamily::Temperate
        | SurfaceFamily::Tropical
        | SurfaceFamily::Steppe
        | SurfaceFamily::Savanna
        | SurfaceFamily::Chaparral => (
            material::GRASS,
            ecological_palette(terrain_ecology::Cover::Inferred, col),
        ),
        SurfaceFamily::Wetland | SurfaceFamily::Peat => (
            material::WETLAND,
            lerp3(p::PEAT, p::WETLAND, smoothstep01((temp - 2.0) / 10.0)),
        ),
    };
    (mat, col.regional.substrate(mat, base))
}

fn ecological_palette(cover: terrain_ecology::Cover, col: &Col) -> [f32; 3] {
    col.regional.vegetation(
        cover,
        terrain_ecology::palette(cover, col.climate, col.elev),
        col.img_hr.is_some(),
    )
}

fn terrain_appearance(
    col: &Col,
    sample_alt: f32,
    _jitter: f32,
    jitter_scale: f32,
) -> SurfaceAppearance {
    terrain_appearance_with_exposure(col, sample_alt, jitter_scale, rock_exposure(col))
}

fn terrain_appearance_with_exposure(
    col: &Col,
    sample_alt: f32,
    jitter_scale: f32,
    exposure: f32,
) -> SurfaceAppearance {
    let ground_family = classify_surface_with_steep_rock(col, false);
    let rock_family = classify_surface_with_steep_rock(col, true);
    let (ground_mat, ground_procedural) = surface_style(ground_family, col, sample_alt);
    let (rock_mat, rock_procedural) = surface_style(rock_family, col, sample_alt);
    let exposure = if ground_family == rock_family {
        0.0
    } else {
        exposure
    };
    let procedural = lerp3(ground_procedural, rock_procedural, exposure);
    let mat = if exposure > ROCK_MATERIAL_EXPOSURE {
        rock_mat
    } else {
        ground_mat
    };

    // Orbital imagery retains its measured color. At local scales the
    // source is interpreted through its regional material palette so baked
    // image shadows do not compete with runtime lighting.
    let detail = smoothstep01((jitter_scale - 1.0 / 3.0) * 1.5);
    let source = col.img_hr.or(col.img_detail).or(col.img);
    let base = if col.elev < 0.0 {
        procedural
    } else if let Some(source) = source {
        art_direction::imagery_surface(mat, source.map(f32::from), procedural, detail)
    } else {
        procedural
    };

    let base = art_direction::landscape_surface(mat, base, col.landscape_variation, detail);
    SurfaceAppearance {
        material: mat,
        tint: tint(base, 1.0),
    }
}

fn natural_cover_exposure(slope_rad: f32) -> f32 {
    // Mapped vegetation is stronger evidence than an inferred rocky hillside.
    // Its estimated DEM slope exposes geology continuously over about 39–60°;
    // unlike generic terrain, shrinking column relief cannot change this blend.
    smoothstep01((slope_rad - 0.68) / (1.05 - 0.68))
}

fn natural_cover_base(col: &Col, vegetation: [f32; 3], weight: f32, detail: f32) -> [f32; 3] {
    let exposure = natural_cover_exposure(col.slope);
    if exposure >= 1.0 {
        return terrain_appearance_with_exposure(col, col.elev, detail, 1.0)
            .tint
            .map(f32::from);
    }
    let ground = terrain_appearance_with_exposure(col, col.elev, detail, 0.0);
    let covered = lerp3(ground.tint.map(f32::from), vegetation, weight);
    if exposure == 0.0 {
        return covered;
    }
    let rock = terrain_appearance_with_exposure(col, col.elev, detail, 1.0);
    lerp3(covered, rock.tint.map(f32::from), exposure)
}

fn road_appearance(class: RoadClass, _jitter: f32, _jitter_scale: f32) -> SurfaceAppearance {
    let (mat, base): (u8, [f32; 3]) = match class {
        RoadClass::Highway => (material::ASPHALT, palette::infrastructure::HIGHWAY),
        RoadClass::Major => (material::ASPHALT, palette::infrastructure::MAJOR_ROAD),
        RoadClass::Medium => (material::ASPHALT, palette::infrastructure::MEDIUM_ROAD),
        RoadClass::Minor => (material::ASPHALT, palette::infrastructure::MINOR_ROAD),
        RoadClass::Service => (material::ASPHALT, palette::infrastructure::MINOR_ROAD),
        RoadClass::Path => (material::PATH, palette::infrastructure::PATH),
        RoadClass::Rail => (material::STEEL, palette::infrastructure::RAIL_STEEL),
    };
    SurfaceAppearance {
        material: mat,
        tint: tint(base, 1.0),
    }
}

fn bridge_structure_appearance(
    kind: bridges::BridgeKind,
    jitter: f32,
    jitter_scale: f32,
) -> SurfaceAppearance {
    let base: [f32; 3] = match kind {
        bridges::BridgeKind::Beam => [150.0, 148.0, 142.0],
        bridges::BridgeKind::Arch => [96.0, 102.0, 110.0],
        bridges::BridgeKind::Truss => [104.0, 78.0, 62.0],
        bridges::BridgeKind::Suspension => [148.0, 72.0, 56.0],
    };
    let shade = 1.0 + (jitter - 0.5) * 0.08 * jitter_scale;
    SurfaceAppearance {
        material: material::STEEL,
        tint: tint(base, shade),
    }
}

/// Concrete piers, pylons and deck slabs.
fn bridge_support_appearance(jitter: f32, jitter_scale: f32) -> SurfaceAppearance {
    let shade = 1.0 + (jitter - 0.5) * 0.08 * jitter_scale;
    SurfaceAppearance {
        material: material::PAVED,
        tint: tint([139.0, 136.0, 129.0], shade),
    }
}

fn ground_appearance(
    kind: GroundKind,
    col: &Col,
    jitter: f32,
    jitter_scale: f32,
) -> SurfaceAppearance {
    let mut owned = *col;
    owned.regional = owned
        .regional
        .scaled(smoothstep01((jitter_scale - 1.0 / 3.0) * 1.5));
    if !matches!(
        kind,
        GroundKind::Forest
            | GroundKind::Scrub
            | GroundKind::NaturalGrass
            | GroundKind::BareRock
            | GroundKind::RegionalBarren
            | GroundKind::RegionalForest
            | GroundKind::RegionalGrassland
            | GroundKind::RegionalScrub
    ) {
        owned.regional = regional_landscape::Palette::EMPTY;
    }
    let col = &owned;
    if kind == GroundKind::Forest {
        let forest = ecological_palette(terrain_ecology::Cover::Forest, col);
        return SurfaceAppearance {
            material: material::GRASS,
            tint: tint(natural_cover_base(col, forest, 0.36, jitter_scale), 1.0),
        };
    }
    if kind.is_natural_semantic() {
        return semantic_ground_appearance(kind, col, jitter, jitter_scale);
    }
    if kind.is_managed_semantic() {
        return managed_ground_appearance(kind, col, jitter, jitter_scale);
    }
    if kind.is_regional_semantic() {
        return regional_ground_appearance(kind, col, jitter, jitter_scale);
    }
    let terrain = terrain_appearance(col, col.elev, jitter, jitter_scale);
    let t = [
        terrain.tint[0] as f32,
        terrain.tint[1] as f32,
        terrain.tint[2] as f32,
    ];
    let (mat, base): (u8, [f32; 3]) = match kind {
        GroundKind::Green => (material::GRASS, lerp3(t, palette::ground::GREEN, 0.65)),
        GroundKind::Forest => unreachable!("natural forest returns above"),
        GroundKind::Paved => (material::PAVED, palette::ground::PAVED),
        GroundKind::Sand => (material::SAND, palette::ground::SAND),
        // Warm concrete blended over the local terrain color, so cities
        // stay subtly climate-tinted rather than uniformly gray.
        GroundKind::Urban => (material::PAVED, lerp3(t, palette::ground::URBAN, 0.72)),
        GroundKind::BareRock
        | GroundKind::Wetland
        | GroundKind::Scrub
        | GroundKind::NaturalGrass
        | GroundKind::Glacier
        | GroundKind::Farmyard
        | GroundKind::RailwayArea
        | GroundKind::AirportArea
        | GroundKind::RunwayArea
        | GroundKind::RegionalBarren
        | GroundKind::RegionalForest
        | GroundKind::RegionalGlacier
        | GroundKind::RegionalGrassland
        | GroundKind::RegionalScrub
        | GroundKind::RegionalUrban => unreachable!("semantic ground returns above"),
    };
    // The ground carpet takes the regional aerial pixel
    // strongly — sidewalks, tram tracks and courtyard pavings all read —
    // while the carpet color keeps enough weight to stay recognizably
    // "urban block" vs "park" when the ortho is in shadow.
    let base = match col.img_hr {
        Some(hr) => art_direction::imagery_surface(mat, hr.map(f32::from), base, 1.0),
        None => base,
    };
    SurfaceAppearance {
        material: mat,
        tint: tint(base, 1.0),
    }
}

fn semantic_ground_appearance(
    kind: GroundKind,
    col: &Col,
    jitter: f32,
    jitter_scale: f32,
) -> SurfaceAppearance {
    if matches!(kind, GroundKind::Scrub | GroundKind::NaturalGrass) {
        let (cover, weight) = if kind == GroundKind::Scrub {
            (terrain_ecology::Cover::Scrub, 0.24)
        } else {
            (terrain_ecology::Cover::Grass, 0.20)
        };
        let target = ecological_palette(cover, col);
        return SurfaceAppearance {
            material: material::GRASS,
            tint: tint(natural_cover_base(col, target, weight, jitter_scale), 1.0),
        };
    }
    let family = match kind {
        GroundKind::BareRock => mapped_rock_family(col),
        GroundKind::Wetland => {
            if col.climate.temp_c < 8.0 {
                SurfaceFamily::Peat
            } else {
                SurfaceFamily::Wetland
            }
        }
        GroundKind::Scrub | GroundKind::NaturalGrass => {
            unreachable!("mapped vegetation returns above")
        }
        GroundKind::Glacier => SurfaceFamily::Ice,
        GroundKind::Green
        | GroundKind::Paved
        | GroundKind::Sand
        | GroundKind::Urban
        | GroundKind::Forest
        | GroundKind::Farmyard
        | GroundKind::RailwayArea
        | GroundKind::AirportArea
        | GroundKind::RunwayArea
        | GroundKind::RegionalBarren
        | GroundKind::RegionalForest
        | GroundKind::RegionalGlacier
        | GroundKind::RegionalGrassland
        | GroundKind::RegionalScrub
        | GroundKind::RegionalUrban => {
            unreachable!("non-natural ground handled by ground_appearance")
        }
    };
    let (material, procedural) = surface_style(family, col, col.elev);
    let weight = match kind {
        GroundKind::BareRock => 0.34,
        GroundKind::Wetland => 0.30,
        GroundKind::Glacier => 0.55,
        _ => unreachable!(),
    };
    let observed = terrain_appearance(col, col.elev, jitter, jitter_scale);
    let base = lerp3(observed.tint.map(f32::from), procedural, weight);
    SurfaceAppearance {
        material,
        tint: tint(base, 1.0),
    }
}

fn managed_ground_appearance(
    kind: GroundKind,
    col: &Col,
    jitter: f32,
    jitter_scale: f32,
) -> SurfaceAppearance {
    let observed = terrain_appearance(col, col.elev, jitter, jitter_scale);
    let observed_tint = [
        observed.tint[0] as f32,
        observed.tint[1] as f32,
        observed.tint[2] as f32,
    ];
    let (material, target, weight) = match kind {
        GroundKind::Farmyard => {
            let family = if col.climate.arid > 0.52 {
                SurfaceFamily::Clay
            } else if col.climate.humidity > 0.68 {
                SurfaceFamily::Alluvium
            } else {
                SurfaceFamily::Loess
            };
            let (_, compacted_soil) = surface_style(family, col, col.elev);
            (material::SOIL, compacted_soil, 0.34)
        }
        GroundKind::RailwayArea => (
            material::PAVED,
            lerp3(palette::terrain::SCREE, palette::ground::URBAN, 0.55),
            0.30,
        ),
        GroundKind::AirportArea => (
            observed.material,
            lerp3(palette::ground::URBAN, palette::ground::PAVED, 0.42),
            0.18,
        ),
        GroundKind::RunwayArea => (material::PAVED, palette::ground::PAVED, 0.36),
        GroundKind::Green
        | GroundKind::Paved
        | GroundKind::Sand
        | GroundKind::Urban
        | GroundKind::Forest
        | GroundKind::BareRock
        | GroundKind::Wetland
        | GroundKind::Scrub
        | GroundKind::NaturalGrass
        | GroundKind::Glacier
        | GroundKind::RegionalBarren
        | GroundKind::RegionalForest
        | GroundKind::RegionalGlacier
        | GroundKind::RegionalGrassland
        | GroundKind::RegionalScrub
        | GroundKind::RegionalUrban => unreachable!("non-managed ground handled elsewhere"),
    };
    let base = lerp3(observed_tint, target, weight);
    SurfaceAppearance {
        material,
        tint: tint(base, 1.0),
    }
}

fn regional_ground_appearance(
    kind: GroundKind,
    col: &Col,
    jitter: f32,
    jitter_scale: f32,
) -> SurfaceAppearance {
    let observed = terrain_appearance(col, col.elev, jitter, jitter_scale);
    let observed_tint = [
        observed.tint[0] as f32,
        observed.tint[1] as f32,
        observed.tint[2] as f32,
    ];
    let (material, target, weight) = match kind {
        GroundKind::RegionalBarren => {
            // Regional barren includes dunes, salt flats and bare soil. It
            // establishes sparse cover, not detailed exposed-rock geology.
            let family = classify_surface(col);
            let (material, target) = surface_style(family, col, col.elev);
            let (material, target) = if matches!(material, material::GRASS | material::WETLAND) {
                surface_style(mapped_rock_family(col), col, col.elev)
            } else {
                (material, target)
            };
            (material, target, 0.14)
        }
        GroundKind::RegionalForest => (
            material::GRASS,
            ecological_palette(terrain_ecology::Cover::Forest, col),
            0.14,
        ),
        GroundKind::RegionalGlacier => {
            let (material, target) = surface_style(SurfaceFamily::Ice, col, col.elev);
            (material, target, 0.34)
        }
        GroundKind::RegionalGrassland => {
            let target = ecological_palette(terrain_ecology::Cover::Grass, col);
            (material::GRASS, target, 0.10)
        }
        GroundKind::RegionalScrub => {
            let target = ecological_palette(terrain_ecology::Cover::Scrub, col);
            (material::GRASS, target, 0.10)
        }
        GroundKind::RegionalUrban => (observed.material, palette::ground::URBAN, 0.12),
        GroundKind::Green
        | GroundKind::Paved
        | GroundKind::Sand
        | GroundKind::Urban
        | GroundKind::Forest
        | GroundKind::BareRock
        | GroundKind::Wetland
        | GroundKind::Scrub
        | GroundKind::NaturalGrass
        | GroundKind::Glacier
        | GroundKind::Farmyard
        | GroundKind::RailwayArea
        | GroundKind::AirportArea
        | GroundKind::RunwayArea => unreachable!("non-regional ground handled elsewhere"),
    };
    let base = if matches!(
        kind,
        GroundKind::RegionalForest | GroundKind::RegionalGrassland | GroundKind::RegionalScrub
    ) {
        natural_cover_base(col, target, weight, jitter_scale)
    } else {
        lerp3(observed_tint, target, weight)
    };
    SurfaceAppearance {
        material,
        tint: tint(base, 1.0),
    }
}

fn mapped_rock_family(col: &Col) -> SurfaceFamily {
    let inferred = classify_surface_with_steep_rock(col, true);
    if matches!(
        inferred,
        SurfaceFamily::Volcanic
            | SurfaceFamily::Slate
            | SurfaceFamily::Granite
            | SurfaceFamily::Limestone
            | SurfaceFamily::RedSedimentary
            | SurfaceFamily::TanSedimentary
            | SurfaceFamily::Scree
    ) {
        return inferred;
    }
    let rgb = source_rgb(col).unwrap_or([0.44, 0.42, 0.36]);
    let value = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
    let warmth = rgb[0] - rgb[2];
    if col.climate.arid > 0.38 && warmth > 0.06 {
        SurfaceFamily::RedSedimentary
    } else if value > 0.62 {
        SurfaceFamily::Limestone
    } else if value < 0.36 {
        SurfaceFamily::Slate
    } else {
        SurfaceFamily::Granite
    }
}

// Flower-head colour belongs to seasonal geometry, not immutable terrain.
// FIELD_FLOWER material semantics remain available to the renderer, but the
// generated tint is the underlying observed/procedural substrate.
// Global crop-ground tuning map.
const FIELD_FLOWER_SEMANTIC_WEIGHT: f32 = 0.0;
const FIELD_CULTIVATED_SEMANTIC_WEIGHT: f32 = 0.18;
const FIELD_MEADOW_SEMANTIC_WEIGHT: f32 = 0.14;

fn field_semantic_weight(kind: crate::flora::FieldSurfaceKind) -> f32 {
    use crate::flora::FieldSurfaceKind as Kind;
    match kind {
        Kind::FlowerField => FIELD_FLOWER_SEMANTIC_WEIGHT,
        Kind::Meadow => FIELD_MEADOW_SEMANTIC_WEIGHT,
        Kind::Crop | Kind::Orchard | Kind::Vineyard | Kind::Dormant | Kind::Harvested => {
            FIELD_CULTIVATED_SEMANTIC_WEIGHT
        }
    }
}

fn field_surface_appearance(
    stamp: tier0::FloraStamp,
    col: &Col,
    cell: CellId,
    x: usize,
    z: usize,
) -> SurfaceAppearance {
    use crate::flora::FieldSurfaceKind as Kind;
    use palette::field as p;

    let mut owned = *col;
    owned.regional = regional_landscape::Palette::EMPTY;
    let col = &owned;
    let sample = crate::flora::sample_field_column(stamp.surface, stamp.seed, cell, x, z);
    let terrain = terrain_appearance(col, col.elev, 0.5, 0.0);
    // Broad ecological destinations authorize procedural blooms; they are not
    // land-cover replacements. Their descriptor-driven flowers and overview
    // aggregates provide all seasonal colour, so an out-of-season region
    // cannot leave a grid of permanent painted patches on ordinary terrain.
    if crate::flower_destinations::FlowerDestinationId::from_u8(stamp.curated_style)
        .map(crate::flower_destinations::flower_destination)
        .is_some_and(|destination| {
            destination.mode == crate::flower_destinations::DestinationMode::EcologicalBoost
        })
    {
        return terrain;
    }
    let substrate_rgb = col
        .img_hr
        .map_or_else(|| terrain.tint.map(f32::from), |hr| hr.map(f32::from));
    if stamp.curated_style == 0 && stamp.kind == vearth_data::vectors::FloraRegionKind::Garden {
        let patch = crate::flora::garden_patch_at(
            stamp.seed,
            cell,
            (x as f64 + 0.5) / N as f64,
            (z as f64 + 0.5) / N as f64,
        );
        if patch != crate::flora::GardenPatch::FlowerBed {
            if patch == crate::flora::GardenPatch::Gap {
                return SurfaceAppearance {
                    material: material::FIELD_MEADOW,
                    tint: terrain.tint,
                };
            }
            let (low, lush) = p::MEADOW[sample.palette as usize % p::MEADOW.len()];
            let maintained = match patch {
                crate::flora::GardenPatch::MaintainedGreen => lerp3(low, lush, 0.62),
                crate::flora::GardenPatch::ShrubProxy => lerp3(low, lush, 0.28),
                _ => unreachable!(),
            };
            return SurfaceAppearance {
                material: material::FIELD_MEADOW,
                tint: tint(
                    lerp3(substrate_rgb, maintained, FIELD_MEADOW_SEMANTIC_WEIGHT),
                    1.0,
                ),
            };
        }
    }
    let (mat, semantic) = match sample.kind {
        Kind::Crop => {
            let (base, rows) = agriculture_catalog::treatment_for_palette(sample.palette)
                .map_or_else(
                    || p::CROP[sample.palette as usize % p::CROP.len()],
                    |treatment| {
                        let soil = treatment.soil.map(f32::from);
                        (soil, lerp3(soil, treatment.green.map(f32::from), 0.46))
                    },
                );
            (
                material::FIELD_CROP,
                lerp3(base, rows, 0.22 + sample.row * 0.66),
            )
        }
        Kind::FlowerField => {
            let (green, _, _) = p::FLOWER[sample.palette as usize % p::FLOWER.len()];
            let strip = crate::flora::generic_flower_palette_linear(sample.palette)
                [usize::from(sample.flower_variant)]
            .map(|value| value * 255.0);
            (
                material::FIELD_FLOWER,
                lerp3(green, strip, 0.24 + sample.row * 0.68),
            )
        }
        Kind::Orchard => {
            let (floor, rows) = p::ORCHARD[sample.palette as usize % p::ORCHARD.len()];
            (
                material::FIELD_ORCHARD,
                lerp3(floor, rows, 0.12 + sample.row * 0.84),
            )
        }
        Kind::Vineyard => {
            let (floor, rows) = p::VINEYARD[sample.palette as usize % p::VINEYARD.len()];
            (
                material::FIELD_VINEYARD,
                lerp3(floor, rows, 0.10 + sample.row * 0.88),
            )
        }
        Kind::Meadow => {
            let (low, lush) = p::MEADOW[sample.palette as usize % p::MEADOW.len()];
            (
                material::FIELD_MEADOW,
                lerp3(low, lush, 0.20 + sample.variation * 0.58),
            )
        }
        Kind::Dormant => {
            let (straw, rows) = p::DORMANT[sample.palette as usize % p::DORMANT.len()];
            (
                material::FIELD_DORMANT,
                lerp3(straw, rows, 0.18 + sample.row * 0.58),
            )
        }
        Kind::Harvested => {
            let (stubble, soil) = p::HARVESTED[sample.palette as usize % p::HARVESTED.len()];
            (
                material::FIELD_HARVESTED,
                lerp3(stubble, soil, 0.12 + sample.row * 0.72),
            )
        }
    };

    // A field remains a restrained substrate at every terrain LOD. Row and
    // strip detail already attenuate analytically with sampling footprint;
    // changing color opacity by quadtree level created bright polygonal seams.
    let base = if sample.kind == Kind::Crop
        && agriculture_catalog::treatment_for_palette(sample.palette).is_some()
    {
        // Historical aerial crop color cannot dictate today's planting state.
        // The neutral cultivated floor remains visible between seasonal plants.
        lerp3(terrain.tint.map(f32::from), semantic, 0.84)
    } else if stamp.curated_style == 0
        && stamp.kind == vearth_data::vectors::FloraRegionKind::Meadow
        && sample.kind == Kind::Meadow
    {
        let detail = (100.0 / cell.sample_pitch_m()).clamp(0.05, 1.0) as f32;
        natural_cover_base(col, semantic, FIELD_MEADOW_SEMANTIC_WEIGHT, detail)
    } else {
        lerp3(substrate_rgb, semantic, field_semantic_weight(sample.kind))
    };
    let authored_variation = (sample.variation - 0.5) * 0.035;
    SurfaceAppearance {
        material: mat,
        tint: tint(base, 1.0 + authored_variation),
    }
}

#[derive(Clone, Copy)]
struct FacadeGrammar {
    floor_m: f64,
    bay_m: f32,
    middle_x: (f32, f32),
    base_x: (f32, f32),
    middle_y: (f64, f64),
    base_y: (f64, f64),
    crown_y: (f64, f64),
    open_threshold: u8,
}

fn facade_grammar(stamp: &BuildingStamp) -> FacadeGrammar {
    if let Some(system) = stamp.design.facade {
        let profile = system.profile();
        let half_pier = (profile.pier_m / profile.bay_m).clamp(0.03, 0.30) as f32;
        return FacadeGrammar {
            floor_m: profile.floor_m,
            bay_m: profile.bay_m as f32,
            middle_x: (half_pier, 1.0 - half_pier),
            base_x: (half_pier, 1.0 - half_pier),
            middle_y: profile.glazing,
            base_y: profile.glazing,
            crown_y: profile.glazing,
            open_threshold: 255,
        };
    }
    use architecture_styles::{GroundStorey, StyleProfile};
    let profile = stamp.design.style.profile();
    let geometric = stamp.design.expression >= 96;
    let style_profile = if geometric {
        profile
    } else {
        // Local finish survives modern infill; historic opening proportions do
        // not turn a large contemporary block into a stretched old townhouse.
        architecture_styles::ArchitectureStyle::Contemporary.profile()
    };
    let StyleProfile {
        bay_width_m,
        floor_height_m,
        window_width_ratio,
        window_height_ratio,
        sill_ratio,
        ..
    } = style_profile;
    if stamp.design.construction == regional_architecture::ConstructionSystem::Industrial {
        return FacadeGrammar {
            floor_m: 4.5,
            bay_m: 5.5,
            middle_x: (0.30, 0.70),
            base_x: (0.30, 0.70),
            middle_y: (0.58, 0.78),
            base_y: (0.60, 0.80),
            crown_y: (0.58, 0.76),
            open_threshold: 235,
        };
    }
    let width = window_width_ratio.clamp(0.25, 0.85);
    let x = ((1.0 - width) * 0.5, (1.0 + width) * 0.5);
    let y0 = f64::from(sill_ratio.clamp(0.08, 0.42));
    let y1 = (y0 + f64::from(window_height_ratio)).min(0.93);
    let shop = geometric
        && matches!(
            profile.ground_storey,
            GroundStorey::Shopfront | GroundStorey::Arcade
        )
        && stamp.design.use_kind != vearth_data::vectors::BuildingUse::Residential;
    let base_x = if shop { (0.12, 0.88) } else { x };
    let base_y = if shop { (0.06, 0.89) } else { (y0, y1) };
    FacadeGrammar {
        floor_m: f64::from(floor_height_m),
        bay_m: bay_width_m,
        middle_x: x,
        base_x,
        middle_y: (y0, y1),
        base_y,
        crown_y: (y0, y1),
        // A repeated architectural bay is a coherent opening rhythm. Small
        // finish/light changes vary windows without random holes in the rhythm.
        open_threshold: 255,
    }
}

fn facade_unit_hash(seed: u64, edge: u16, bay: u32, floor: u32) -> u64 {
    let mut s = seed ^ (edge as u64) << 48 ^ (bay as u64) << 24 ^ floor as u64;
    s = s.wrapping_add(0x9E3779B97F4A7C15);
    s = (s ^ (s >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    s = (s ^ (s >> 27)).wrapping_mul(0x94D049BB133111EB);
    s ^ (s >> 31)
}

fn facade_wall_color(stamp: &BuildingStamp) -> [f32; 3] {
    art_direction::architecture_surface(stamp.design.wall_material, stamp.design.wall)
        .map(f32::from)
}

fn facade_window_color(stamp: &BuildingStamp, state: u64) -> [u8; 3] {
    let base = stamp.design.glass.map(f32::from);
    let shade = 0.94 + ((state >> 24) & 15) as f32 / 125.0;
    tint(base, shade)
}

fn facade_floor_bands(
    grammar: FacadeGrammar,
    floor: u32,
    floor_count: u32,
) -> ((f32, f32), (f64, f64)) {
    let base = floor == 0;
    let crown = floor + 1 == floor_count;
    let x = if base {
        grammar.base_x
    } else {
        grammar.middle_x
    };
    let y = if base {
        grammar.base_y
    } else if crown {
        grammar.crown_y
    } else {
        grammar.middle_y
    };
    (x, y)
}

fn facade_night_threshold(style: FacadeStyle) -> u8 {
    match style {
        FacadeStyle::LowRise => 62,
        FacadeStyle::Masonry => 94,
        FacadeStyle::Townhouse => 88,
        FacadeStyle::Continental => 86,
        FacadeStyle::Mediterranean => 78,
        FacadeStyle::Nordic => 100,
        FacadeStyle::Alpine => 74,
        FacadeStyle::Office => 138,
        FacadeStyle::Tower => 164,
        FacadeStyle::Industrial => 38,
    }
}

fn facade_unit_is_lit(style: FacadeStyle, grammar: FacadeGrammar, state: u64) -> bool {
    ((state >> 40) & 0xff) < grammar.open_threshold as u64
        && ((state >> 8) & 0xff) < facade_night_threshold(style) as u64
}

/// Deterministic facade grammar: each footprint edge is divided into
/// fitted structural bays with solid corner piers. Floors align vertically;
/// randomness only selects a building family and the state of a complete
/// opening. The roof band keeps its own palette.
fn building_appearance(
    alt: f64,
    lo: f64,
    flat_top: f64,
    stamp: &BuildingStamp,
    img_hr: Option<[u8; 3]>,
    surface_epsilon_m: f64,
) -> SurfaceAppearance {
    let wall_material = stamp.design.wall_material;
    let roof_material = stamp.design.roof_material;
    let seed = stamp.seed;
    // The roof takes the regional aerial pixel — this is what makes a
    // slate roof differ from its terracotta neighbor — with the
    // palette keeping a trace so shadowed orthos stay roof-colored.
    let roof_only = stamp.building_kind == tier0::BuildingKind::RoofOnly;
    if roof_only || alt > flat_top - surface_epsilon_m || alt > flat_top {
        let authored = stamp.design.roof;
        let base = art_direction::architecture_surface(material::ROOF, authored).map(f32::from);
        let base = match img_hr {
            Some(hr) => {
                art_direction::imagery_surface(material::ROOF, hr.map(f32::from), base, 1.0)
            }
            None => base,
        };
        let shade = 0.95 + ((seed >> 11) & 0xf) as f32 / 160.0;
        return SurfaceAppearance {
            material: roof_material,
            tint: tint(base, shade),
        };
    }
    let wall = facade_wall_color(stamp);
    // A roof's aerial pigment cannot repaint the selected facade contract.
    let facade = stamp.facade;
    if stamp.facade_openings && facade.boundary && !facade.corner && facade.length_m > 1.0 {
        let grammar = facade_grammar(stamp);
        // Floor identity depends on the source building, independently of terrain LOD.
        let usable_h = (flat_top - lo).max(0.1);
        let floor_count = (usable_h / grammar.floor_m).round().clamp(1.0, 192.0) as u32;
        let floor_h = usable_h / floor_count as f64;
        let above = (alt - lo).clamp(0.0, usable_h - f64::EPSILON);
        let floor = (above / floor_h)
            .floor()
            .clamp(0.0, floor_count as f64 - 1.0) as u32;
        let floor_frac = (above - floor as f64 * floor_h) / floor_h;

        let (x_band, y_band) = facade_floor_bands(grammar, floor, floor_count);

        let tall_profile = stamp
            .tall_style
            .map(crate::tall_buildings::tall_building_profile);
        let mechanical_floor = tall_profile.is_some_and(|profile| {
            profile.mechanical_every > 0
                && floor > 1
                && floor + 2 < floor_count
                && (floor + 1).is_multiple_of(u32::from(profile.mechanical_every))
        });
        if mechanical_floor {
            return SurfaceAppearance {
                material: wall_material,
                tint: tint(
                    art_direction::architecture_surface(
                        stamp.design.wall_material,
                        stamp.design.wall,
                    )
                    .map(f32::from),
                    0.96,
                ),
            };
        }

        let bay_count = (facade.length_m / grammar.bay_m).round().clamp(1.0, 128.0) as u32;
        let bay_w = facade.length_m / bay_count as f32;
        let along = facade
            .along_m
            .clamp(0.0, (facade.length_m - f32::EPSILON).max(0.0));
        let bay = (along / bay_w).floor().min(bay_count as f32 - 1.0) as u32;
        let bay_frac = (along - bay as f32 * bay_w) / bay_w;

        let state = facade_unit_hash(seed, facade.edge, bay, floor);
        if let Some(profile) = tall_profile {
            use crate::tall_buildings::TallBuildingStyle as Tall;
            let structural = match profile.id {
                Tall::Diagrid => {
                    let u = (bay as f64 + bay_frac as f64) / 8.0;
                    let v = (floor as f64 + floor_frac) / 16.0;
                    let a = (u - v).rem_euclid(1.0);
                    let b = (u + v).rem_euclid(1.0);
                    a.min(1.0 - a).min(b.min(1.0 - b)) < 0.06
                }
                Tall::FramedTube | Tall::BundledTube => {
                    floor_frac < 0.10 || !(0.12..=0.88).contains(&bay_frac)
                }
                Tall::ResidentialLoggia
                | Tall::CircularResidential
                | Tall::SCurveSlab
                | Tall::RippledBalcony => floor_frac < 0.09,
                _ => false,
            };
            if structural {
                let material = if profile.id == Tall::Diagrid {
                    material::STEEL
                } else {
                    material::WALL
                };
                return SurfaceAppearance {
                    material,
                    tint: tint(
                        art_direction::architecture_surface(material, stamp.design.joinery)
                            .map(f32::from),
                        0.96,
                    ),
                };
            }
        }
        let opening_present = ((state >> 40) & 0xff) < grammar.open_threshold as u64;
        if opening_present
            && (x_band.0..x_band.1).contains(&bay_frac)
            && (y_band.0..y_band.1).contains(&floor_frac)
        {
            return SurfaceAppearance {
                material: if stamp.night_eligible
                    && facade_unit_is_lit(stamp.facade_style, grammar, state)
                {
                    material::LIT_WINDOW
                } else {
                    material::WINDOW
                },
                tint: facade_window_color(stamp, state),
            };
        }
    }
    let height_frac = ((alt - lo) / (flat_top - lo).max(surface_epsilon_m)).clamp(0.0, 1.0);
    let zone_shade = if height_frac < 0.13 {
        0.88 // visually weight the base
    } else if height_frac > 0.88 {
        0.96 // restrained crown/parapet band
    } else {
        1.0
    };
    let wear = f32::from(stamp.design.weathering) / 255.0;
    let shelter = if height_frac < 0.13 {
        0.16
    } else if height_frac > 0.88 {
        0.08
    } else {
        0.035
    };
    let shade = (0.97 + ((seed >> 23) & 0x1f) as f32 / 620.0) * zone_shade * (1.0 - wear * shelter);
    SurfaceAppearance {
        material: wall_material,
        tint: tint(wall, shade),
    }
}

fn smoothstep01(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

/// Inland DEMs normally contain the water surface, not bathymetry. Preserve
/// a genuinely lower sampled bed when one exists, otherwise infer a modest
/// optical depth from distance to shore. Narrow rivers stay shallow while
/// broad lakes and harbours gain a readable centre without pretending to
/// know their full physical depth.
fn inland_water_depth(terrain_depth_m: f32, shore: f32) -> f32 {
    let inferred = 2.5 + 13.5 * smoothstep01(shore);
    terrain_depth_m.max(inferred)
}

/// Near a mapped coast, coarse global bathymetry can put a deep sample on the
/// beach or report DEM land inside a lagoon. Replace only that nearshore band
/// with a shoreline-derived shelf, then blend completely back to real
/// bathymetry at the capped edge so open ocean remains data-driven.
fn ocean_water_depth(bathymetry_m: f32, shore: f32) -> f32 {
    let shore = shore.clamp(0.0, 1.0);
    // A mapped boundary alone does not establish a sunlit sand bank. Keep
    // unmeasured coastal water optically substantial; observed offshore
    // shoals can still become pale without outlining every cliff in cyan.
    let coastal_shelf = 12.0 + 18.35 * smoothstep01(shore);
    // OSM coastline authority can carve water over a positive/noisy DEM
    // sample. Zero here means "no bathymetry", not a physically zero-depth
    // rectangular patch. Fade that uncertainty over the first metre so
    // crossing a sample threshold cannot abruptly recolor a shallow lagoon.
    let bathymetry_confidence = smoothstep01(bathymetry_m);
    let bathymetry_weight = smoothstep01((shore - 0.55) / 0.45) * bathymetry_confidence;
    coastal_shelf + (bathymetry_m - coastal_shelf) * bathymetry_weight
}

fn water_appearance(depth_m: f32, col: &Col, inland: bool) -> SurfaceAppearance {
    // These are restrained climate priors, not measured water chemistry.
    // Smooth shared weights keep both depth endpoints continuous when climate
    // or the local relief estimate straddles a classification boundary.
    let (glacial, sediment) = if inland {
        (
            smoothstep01((7.0 - col.climate.temp_c) / 6.0)
                * smoothstep01((col.relief - 30.0) / 60.0),
            smoothstep01((col.climate.arid - 0.48) / 0.20)
                * smoothstep01((0.55 - col.climate.humidity) / 0.20),
        )
    } else {
        (0.0, 0.0)
    };
    // Climate alone cannot distinguish a humid freshwater lake from a tidal
    // harbour. Clear blue remains the base; more specific chemistry needs data.
    let tropical = smoothstep01((col.climate.temp_c - 18.0) / 9.0)
        * smoothstep01((col.climate.humidity - 0.45) / 0.35);
    let bottom = lerp3(
        lerp3(
            lerp3(palette::water::SHALLOW, palette::water::LAGOON, tropical),
            palette::water::SEDIMENT,
            sediment,
        ),
        palette::water::GLACIAL,
        glacial,
    );
    let body = lerp3(
        lerp3(
            lerp3(palette::water::MID, palette::water::TROPICAL_MID, tropical),
            palette::water::SEDIMENT_MID,
            sediment,
        ),
        palette::water::GLACIAL_MID,
        glacial,
    );
    // Extinction is in reciprocal metres. Red disappears first in clear
    // water; suspended mineral/sediment families have shorter, greener paths.
    let extinction = lerp3(
        lerp3(
            lerp3([0.40, 0.12, 0.075], [0.36, 0.075, 0.043], tropical),
            [0.20, 0.16, 0.25],
            sediment,
        ),
        [0.21, 0.09, 0.095],
        glacial,
    );
    SurfaceAppearance {
        material: material::WATER,
        tint: water_color::column_color(
            depth_m,
            bottom,
            body,
            extinction,
            col.landscape_variation,
            (0.35 + 0.65 * tropical) * (1.0 - sediment) * (1.0 - glacial),
        ),
    }
}

/// Deterministic per-column jitter in [0,1): splitmix64 over the address.
fn hash01(cell: CellId, x: u32, z: u32) -> f32 {
    let mut s = (cell.face.0 as u64) << 58
        ^ (cell.level as u64) << 52
        ^ (cell.i as u64) << 26
        ^ (cell.j as u64)
        ^ (x as u64) << 47
        ^ (z as u64) << 37
        ^ PROCEDURAL_SEED_VERSION as u64;
    s = s.wrapping_add(0x9E3779B97F4A7C15);
    let mut z64 = s;
    z64 = (z64 ^ (z64 >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z64 = (z64 ^ (z64 >> 27)).wrapping_mul(0x94D049BB133111EB);
    z64 ^= z64 >> 31;
    (z64 >> 40) as f32 / (1u64 << 24) as f32
}

#[cfg(test)]
mod generation_tests;
