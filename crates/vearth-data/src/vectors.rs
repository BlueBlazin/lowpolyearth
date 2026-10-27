//! Vector features for the voxelizer:
//! OSM-derived buildings, roads, water and landuse from a Protomaps basemap
//! PMTiles archive (planetiler build, ODbL — attribution required).
//!
//! The generator asks for everything intersecting a cell-sized bbox and gets
//! features projected to **local east/north meters** about the bbox center —
//! geometry math downstream is planar and frame-agnostic. Raw MVT remains the
//! lossless disk authority; each successful in-memory fill normalizes once to
//! `SemanticTile`, and every production terrain/location consumer reads typed
//! facts while borrowing source geometry. Query planning, semantic telemetry,
//! and terrain cache identity are source-scoped at this boundary.

use crate::DataError;
use crate::admin_regions::{AdminRegion, AdminRegionAtlas};
use crate::bridge_pack::{BridgeCatalog, CanonicalBridge};
#[cfg(test)]
use crate::mvt::polygon::{polygon_components, raw_component_contains};
use crate::mvt::{self, GeomType, Value};
use crate::pmtiles::PmTiles;
use crate::protomaps::{Compatibility, SourceDescriptor};
use crate::protomaps_observability::{
    SemanticConsumerReport, SemanticDiscardReason, SemanticLayerFamily, SemanticTelemetry,
    SemanticTileOutcome, WaterSemanticOutcome,
};
use crate::protomaps_semantics::{
    LandcoverKind, LanduseKind, LayerFacts, SemanticFeature, SemanticLayer, SemanticTile,
    SourceFact, SourceFeatureIdentity, WaterKind, WaterKindDetail, WaterSurfaceDisposition,
};
use crate::road_profile::{RoadDetail, RoadProfile};
use crate::source_cache::{CachedTile, LegacyLayout, SourceTileCache};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

const EQUATOR_M: f64 = 40_075_016.686;
const MEAN_RADIUS: f64 = 6_371_008.8;
/// Async generation can have 32–64 prepared cell plans awaiting the bounded
/// CPU lane. Each fine plan normally pins two authority tiles; a 96-entry cap
/// evicted successfully prepared inputs before their consumer ran. Count is a
/// fragmentation/metadata guard; the measured 384 MiB byte bound remains the
/// authoritative resident-memory ceiling.
const TILE_CACHE_MAX: usize = 256;
const TILE_CACHE_MAX_BYTES: usize = 384 * 1024 * 1024;
/// A bbox never needs more than a few tiles; hard cap for safety.
const MAX_TILES_PER_QUERY: usize = 9;
/// Transient projected geometry owned by one terrain FeaturePack. This is
/// deliberately separate from the decoded semantic-tile LRU byte budget.
pub const MAX_REGIONAL_LANDCOVER_PROJECTED_BYTES: usize = 8 * 1024 * 1024;
/// Projected centerline geometry is duplicated into a per-cell `FeaturePack`.
/// Reject the whole watercourse line channel when a dense query exceeds this bound;
/// partial waterways must never be persisted as complete terrain input.
pub const MAX_WATERCOURSE_PROJECTED_BYTES: usize = 8 * 1024 * 1024;
const MAX_CLIFF_LINES_PER_PACK: usize = 8_192;
const MAX_CLIFF_POINTS_PER_PACK: usize = 65_536;
const MAX_CLIFF_PROJECTED_BYTES: usize = 8 * 1024 * 1024;
const MAX_EXACT_TREES_PER_PACK: usize = 4_096;
/// Fixed-detail query support retained for detached night-light proxy callers.
/// z14 retains mapped medium/tall buildings while bounding the query to a 5x5
/// tile window.
const NIGHT_BUILDING_EQUATOR_ZOOM: f64 = 14.0;
const MAX_NIGHT_TILES_PER_QUERY: usize = 25;
pub const NIGHT_BUILDING_MIN_HEIGHT_M: f32 = 18.0;
const TERRAIN_DETAIL_LAYERS: &[&str] = &["buildings", "earth", "landuse", "pois", "roads", "water"];
const TERRAIN_LANDCOVER_LAYERS: &[&str] = &["landcover"];
const NIGHT_BUILDING_LAYERS: &[&str] = &["buildings"];
const LOCATION_CONTEXT_LAYERS: &[&str] = &[
    "landuse",
    "natural",
    "physical_point",
    "places",
    "pois",
    "water",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoadClass {
    Highway,
    Major,
    Medium,
    Minor,
    Service,
    Path,
    Rail,
}

impl RoadClass {
    /// Stamped road width. OSM lacks reliable per-way widths; class defaults
    /// remain legible at 0.6 m voxels.
    pub fn width_m(self) -> f32 {
        match self {
            RoadClass::Highway => 19.0,
            RoadClass::Major => 12.0,
            RoadClass::Medium => 9.0,
            RoadClass::Minor => 6.5,
            RoadClass::Service => 4.2,
            RoadClass::Path => 2.4,
            RoadClass::Rail => 3.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundKind {
    /// Lawn-like managed cover: parks, pitches, cemeteries.
    Green,
    /// Paved plazas, pedestrian streets, rail platforms.
    Paved,
    /// Beaches and dunes.
    Sand,
    /// Built-up fabric (residential/commercial/industrial): the gray city
    /// carpet that makes urban areas read as urban from altitude.
    Urban,
    /// Forest floor: canopy polygons tint the ground dark green at levels
    /// too coarse for individual trees, so forests exist at regional LODs
    /// and the tree gate cuts in over an already-green carpet.
    Forest,
    /// Detailed mapped exposed-rock authority.
    BareRock,
    /// Detailed mapped wet substrate.
    Wetland,
    /// Detailed mapped shrub-dominant natural cover.
    Scrub,
    /// Detailed mapped natural grass, distinct from managed green ground.
    NaturalGrass,
    /// Detailed mapped glacier/ice authority.
    Glacier,
    /// Detailed agricultural working ground. This is a compacted substrate,
    /// not a cultivated-field or generic managed-lawn identity.
    Farmyard,
    /// Polygonal railway premises/working areas. Rail geometry remains owned
    /// by the dedicated transport path.
    RailwayArea,
    /// Broad aerodrome and airfield operational ground. The observed local
    /// material remains authoritative rather than painting the envelope as
    /// pavement.
    AirportArea,
    /// Runway and taxiway pavement substrate without markings, shoulders,
    /// lights, guidance, or navigation semantics.
    RunwayArea,
    /// Coarse z7 barren fallback, distinct from detailed bare-rock authority.
    RegionalBarren,
    /// Coarse z7 forest ground cue. It does not create parcel-level trees.
    RegionalForest,
    /// Coarse z7 glacier fallback.
    RegionalGlacier,
    /// Coarse z7 grassland cue, distinct from detailed natural grass.
    RegionalGrassland,
    /// Coarse z7 scrub cue. It does not create parcel-level shrubs or trees.
    RegionalScrub,
    /// Coarse z7 urban-area cue that retains the observed local material.
    RegionalUrban,
}

impl GroundKind {
    pub const fn is_natural_semantic(self) -> bool {
        matches!(
            self,
            Self::BareRock | Self::Wetland | Self::Scrub | Self::NaturalGrass | Self::Glacier
        )
    }

    pub const fn natural_precedence(self) -> u8 {
        match self {
            Self::NaturalGrass => 0,
            Self::Scrub => 1,
            Self::Wetland => 2,
            Self::BareRock => 3,
            Self::Glacier => 4,
            Self::Green
            | Self::Paved
            | Self::Sand
            | Self::Urban
            | Self::Forest
            | Self::Farmyard
            | Self::RailwayArea
            | Self::AirportArea
            | Self::RunwayArea
            | Self::RegionalBarren
            | Self::RegionalForest
            | Self::RegionalGlacier
            | Self::RegionalGrassland
            | Self::RegionalScrub
            | Self::RegionalUrban => 0,
        }
    }

    pub const fn is_managed_semantic(self) -> bool {
        matches!(
            self,
            Self::Farmyard | Self::RailwayArea | Self::AirportArea | Self::RunwayArea
        )
    }

    /// Stable winner for overlapping substrate polygons. Narrower
    /// transport premises outrank the broader airport envelope, while legacy
    /// detailed surfaces and existing mapped flora/forest remain stronger.
    pub const fn managed_precedence(self) -> u8 {
        match self {
            Self::AirportArea => 0,
            Self::Farmyard => 1,
            Self::RailwayArea => 2,
            Self::RunwayArea => 3,
            Self::Green
            | Self::Paved
            | Self::Sand
            | Self::Urban
            | Self::Forest
            | Self::BareRock
            | Self::Wetland
            | Self::Scrub
            | Self::NaturalGrass
            | Self::Glacier
            | Self::RegionalBarren
            | Self::RegionalForest
            | Self::RegionalGlacier
            | Self::RegionalGrassland
            | Self::RegionalScrub
            | Self::RegionalUrban => 0,
        }
    }

    pub const fn is_regional_semantic(self) -> bool {
        matches!(
            self,
            Self::RegionalBarren
                | Self::RegionalForest
                | Self::RegionalGlacier
                | Self::RegionalGrassland
                | Self::RegionalScrub
                | Self::RegionalUrban
        )
    }
}

/// Source-preserved agricultural and floral polygon semantics.
///
/// These regions intentionally do not enter [`GroundKind::Green`]. The source
/// kind and stable identity avoid re-decoding or inference from a generic lawn
/// surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FloraRegionKind {
    Farmland,
    Meadow,
    Garden,
    Orchard,
    Vineyard,
}

impl FloraRegionKind {
    /// Stable source-overlap priority.
    /// Orchard and vineyard share a tier and use `FloraRegion::seed` as the
    /// deterministic tie-break.
    pub const fn precedence(self) -> u8 {
        match self {
            FloraRegionKind::Meadow => 0,
            FloraRegionKind::Farmland => 1,
            FloraRegionKind::Garden => 2,
            FloraRegionKind::Orchard | FloraRegionKind::Vineyard => 3,
        }
    }

    const fn seed_tag(self) -> u64 {
        match self {
            FloraRegionKind::Farmland => 0x6661_726d_6c61_6e64,
            FloraRegionKind::Meadow => 0x6d65_6164_6f77_0000,
            FloraRegionKind::Garden => 0x6761_7264_656e_0000,
            FloraRegionKind::Orchard => 0x6f72_6368_6172_6400,
            FloraRegionKind::Vineyard => 0x7669_6e65_7961_7264,
        }
    }
}

/// Polygon as even-odd ring set, local EN meters.
pub type Rings = Vec<Vec<[f32; 2]>>;

pub struct FloraRegion {
    /// Source polygon anchor in geodetic radians, captured before local projection.
    pub location_rad: Option<[f64; 2]>,
    pub rings: Rings,
    pub kind: FloraRegionKind,
    /// Stable field identity. MVT ids are authoritative; fallbacks exclude
    /// source tile coordinates from the resulting hash.
    pub seed: u64,
}

/// Source-preserved z0–7 regional context. The generator may use this only as
/// a weaker fallback beneath every detailed land output.
pub struct RegionalLandcoverRegion {
    /// Source polygon anchor in geodetic radians, captured before local projection.
    pub location_rad: Option<[f64; 2]>,
    pub rings: Rings,
    pub kind: LandcoverKind,
    pub source_id: Option<u64>,
    pub stable_identity: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionalLandcoverMode {
    /// Preserve all regional facts, but materialize only farmland fallback.
    ExistingFarmlandOnly,
    /// Admit the reviewed non-farmland regional ground families as weaker
    /// terrain fallback beneath every detailed output.
    SelectiveRegionalTerrain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterPolygonMode {
    /// Every non-empty polygon becomes surface water and
    /// contributes the same broad ecological proximity.
    LegacyUndifferentiated,
    /// Typed dry/underwater polygons remain contextual and
    /// decorative/drainage water has limited ecological authority.
    Typed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatercourseOutputMode {
    /// Retain and reconcile typed centerlines, but do not
    /// admit them to generated terrain.
    DisabledG96,
    /// Admit only rivers and canals that pass raster and continuity gates.
    RiversAndCanals,
}

/// Whether a footprint represents a whole building or one of its volumetric
/// parts. Whole outlines remain useful fallbacks while parts carry the height
/// and roof variation that makes an enriched silhouette recognizable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BuildingKind {
    #[default]
    Building,
    Part,
    /// Pinned-archive `building_part + kind_detail=roof` with an explicit
    /// top and minimum height. Incomplete roof records remain ordinary parts.
    RoofOnly,
}

impl BuildingKind {
    pub const fn is_part(self) -> bool {
        matches!(self, Self::Part | Self::RoofOnly)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoofShape {
    Flat,
    Gabled,
    Hipped,
    Pyramidal,
    Mansard,
    Gambrel,
    Skillion,
    Sawtooth,
    Dome,
    Onion,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BuildingUse {
    #[default]
    Unknown,
    Residential,
    Commercial,
    Industrial,
    /// Non-occupied service structures with solid envelopes.
    Utility,
    Civic,
}

fn building_use(detail: Option<&str>) -> BuildingUse {
    match detail {
        Some(
            "house" | "residential" | "apartments" | "detached" | "semidetached_house" | "terrace"
            | "bungalow" | "dormitory",
        ) => BuildingUse::Residential,
        Some("commercial" | "retail" | "office" | "hotel" | "supermarket") => {
            BuildingUse::Commercial
        }
        Some("industrial" | "warehouse" | "hangar" | "factory") => BuildingUse::Industrial,
        Some(
            "chimney" | "smokestack" | "silo" | "storage_tank" | "water_tower" | "cooling_tower"
            | "spire" | "steeple",
        ) => BuildingUse::Utility,
        Some(
            "school" | "university" | "hospital" | "civic" | "public" | "government" | "church"
            | "mosque" | "temple" | "cathedral" | "chapel" | "synagogue" | "shrine" | "college"
            | "kindergarten" | "museum" | "library" | "fire_station" | "police" | "train_station"
            | "transportation" | "stadium" | "sports_hall",
        ) => BuildingUse::Civic,
        _ => BuildingUse::Unknown,
    }
}

fn building_roof(shape: Option<&str>) -> Option<RoofShape> {
    Some(match shape? {
        "flat" => RoofShape::Flat,
        "gabled" => RoofShape::Gabled,
        "hipped" => RoofShape::Hipped,
        "pyramidal" => RoofShape::Pyramidal,
        "mansard" => RoofShape::Mansard,
        "gambrel" => RoofShape::Gambrel,
        "skillion" => RoofShape::Skillion,
        "sawtooth" => RoofShape::Sawtooth,
        "dome" => RoofShape::Dome,
        "onion" => RoofShape::Onion,
        _ => return None,
    })
}

pub struct Building {
    pub rings: Rings,
    /// Latitude/longitude in radians from the source component bounds, before local
    /// f32 projection. The same source geometry retains its anchor across cells.
    pub location_rad: Option<[f64; 2]>,
    pub height_m: f32,
    pub min_height_m: f32,
    /// Stable per-feature seed (from geometry) for facade/roof variation.
    pub seed: u64,
    pub kind: BuildingKind,
    /// Optional source facts. Missing values never establish a heritage style
    /// or a particular use merely from the country containing the footprint.
    pub roof_shape: Option<RoofShape>,
    pub use_kind: BuildingUse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationHintKind {
    Viewpoint,
    Attraction,
}

/// A basemap POI that may identify an observation tower when it can be
/// conservatively joined to tall, compact mapped building geometry. A POI by
/// itself is never enough: natural viewpoints and ordinary attractions are
/// intentionally retained as hints rather than classified structures.
#[derive(Debug, Clone, Copy)]
pub struct ObservationHint {
    pub point: [f32; 2],
    pub kind: ObservationHintKind,
    pub seed: u64,
}

/// One authored POI tree admitted into the existing terrain/tree instance
/// path.  This intentionally carries only physical placement and stable
/// identity; names and general POI context remain outside terrain output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExactTree {
    pub point: [f32; 2],
    pub identity: SourceFeatureIdentity,
    pub seed: u64,
}

/// One optional, archive-observed `earth.kind=cliff` line. Direction has no
/// physical meaning: the pinned source exposes no verified high/low-side fact.
#[derive(Debug, Clone, PartialEq)]
pub struct CliffLine {
    pub pts: Vec<[f32; 2]>,
    pub identity: SourceFeatureIdentity,
}

pub struct Road {
    pub profile: RoadProfile,
    pub pts: Vec<[f32; 2]>,
    pub class: RoadClass,
    pub is_bridge: bool,
}

impl Road {
    pub fn width_m(&self) -> f32 {
        self.profile.detail.width_m(self.class)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WaterRegionKind {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterGameplayRole {
    PotentiallyNavigable,
    Limited,
    NonNavigable,
    DrySeasonal,
    UnderwaterContext,
    CompatibilityUnknown,
}

/// Line-water families with an accepted physical footprint contract. Broad
/// bay/strait/generic-water lines remain contextual until a source-honest
/// centerline meaning can be proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WatercourseKind {
    River,
    Canal,
    Stream,
    Drain,
    Ditch,
}

impl WatercourseKind {
    pub fn from_semantic(facts: &crate::protomaps_semantics::WaterFacts) -> Option<Self> {
        match facts.class {
            Some(WaterKind::River) => Some(Self::River),
            Some(WaterKind::Canal) => Some(Self::Canal),
            Some(WaterKind::Stream) => Some(Self::Stream),
            Some(WaterKind::Drain) => Some(Self::Drain),
            Some(WaterKind::Ditch) => Some(Self::Ditch),
            Some(WaterKind::Water) => match facts.detail_class {
                Some(WaterKindDetail::River | WaterKindDetail::Riverbank) => Some(Self::River),
                Some(WaterKindDetail::Canal) => Some(Self::Canal),
                Some(WaterKindDetail::Stream) => Some(Self::Stream),
                Some(WaterKindDetail::Drain) => Some(Self::Drain),
                Some(WaterKindDetail::Ditch) => Some(Self::Ditch),
                _ => None,
            },
            _ => None,
        }
    }

    /// Protomaps v4 has no water-line width field, and the pinned archive has
    /// none either. These are conservative full-width footprints, not claims
    /// about real channel or bank geometry.
    pub const fn fallback_width_m(self) -> f32 {
        match self {
            Self::River => 12.0,
            Self::Canal => 6.0,
            Self::Stream => 2.5,
            Self::Drain => 1.5,
            Self::Ditch => 1.0,
        }
    }

    /// Earliest terrain level at which a column is no wider than twice this
    /// class footprint. The width remains metric; admission never widens a
    /// narrow feature merely to make it visible.
    pub const fn minimum_terrain_level(self) -> u8 {
        match self {
            Self::River => 14,
            Self::Canal => 15,
            Self::Stream => 16,
            Self::Drain => 17,
            Self::Ditch => 18,
        }
    }

    const fn style_tag(self) -> u64 {
        match self {
            Self::River => 0x7269_7665_7200_0000,
            Self::Canal => 0x6361_6e61_6c00_0000,
            Self::Stream => 0x7374_7265_616d_0000,
            Self::Drain => 0x6472_6169_6e00_0000,
            Self::Ditch => 0x6469_7463_6800_0000,
        }
    }

    const fn semantic_kind(self) -> WaterKind {
        match self {
            Self::River => WaterKind::River,
            Self::Canal => WaterKind::Canal,
            Self::Stream => WaterKind::Stream,
            Self::Drain => WaterKind::Drain,
            Self::Ditch => WaterKind::Ditch,
        }
    }
}

impl WaterRegionKind {
    pub fn from_semantic(facts: &crate::protomaps_semantics::WaterFacts, reservoir: bool) -> Self {
        if reservoir || facts.detail_class == Some(WaterKindDetail::Reservoir) {
            return Self::Reservoir;
        }
        match facts.class {
            Some(WaterKind::Ocean) => Self::Ocean,
            Some(WaterKind::Lake) => Self::Lake,
            Some(WaterKind::Playa) => Self::Playa,
            Some(WaterKind::Basin) => Self::Basin,
            Some(WaterKind::Canal) => Self::Canal,
            Some(WaterKind::Ditch | WaterKind::Drain) => Self::Drainage,
            Some(WaterKind::Dock) => Self::Dock,
            Some(WaterKind::Fountain | WaterKind::SwimmingPool) => Self::Decorative,
            Some(WaterKind::Reef) => Self::Reef,
            Some(WaterKind::River) => Self::River,
            Some(WaterKind::Stream) => Self::Stream,
            Some(WaterKind::Water) => match facts.detail_class {
                Some(WaterKindDetail::Lake) => Self::Lake,
                Some(WaterKindDetail::River | WaterKindDetail::Riverbank) => Self::River,
                Some(WaterKindDetail::Canal) => Self::Canal,
                Some(WaterKindDetail::Stream) => Self::Stream,
                Some(WaterKindDetail::Ditch | WaterKindDetail::Drain) => Self::Drainage,
                Some(WaterKindDetail::Basin) => Self::Basin,
                Some(WaterKindDetail::Dock) => Self::Dock,
                Some(
                    WaterKindDetail::Reservoir
                    | WaterKindDetail::Rock
                    | WaterKindDetail::Sand
                    | WaterKindDetail::Unknown,
                )
                | None => Self::InlandWater,
            },
            Some(WaterKind::Bay | WaterKind::Sea | WaterKind::Strait) => Self::InlandWater,
            Some(WaterKind::Other) => Self::Other,
            Some(WaterKind::Unknown) | None => Self::CompatibilityUnknown,
        }
    }

    pub const fn is_ocean(self) -> bool {
        matches!(self, Self::Ocean)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Ocean => "ocean",
            Self::InlandWater => "inland_water",
            Self::Lake => "lake",
            Self::Reservoir => "reservoir",
            Self::River => "river",
            Self::Canal => "canal",
            Self::Stream => "stream",
            Self::Drainage => "drainage",
            Self::Basin => "basin",
            Self::Dock => "dock",
            Self::Decorative => "decorative",
            Self::Playa => "playa",
            Self::Reef => "reef",
            Self::Other => "other",
            Self::CompatibilityUnknown => "compatibility_unknown",
        }
    }

    /// Dry playa and underwater reef polygons remain outside the wet surface mask.
    pub const fn creates_surface_water(self) -> bool {
        !matches!(self, Self::Playa | Self::Reef)
    }

    /// Broad ecological proximity remains limited to natural/open water.
    /// Decorative pools/fountains and drainage polygons still render, but do
    /// not create a regional riparian signal.
    pub const fn contributes_ecological_proximity(self) -> bool {
        !matches!(
            self,
            Self::Drainage | Self::Dock | Self::Decorative | Self::Playa | Self::Reef
        )
    }

    pub const fn gameplay_role(self) -> WaterGameplayRole {
        match self {
            Self::Ocean | Self::InlandWater | Self::Lake | Self::Reservoir | Self::River => {
                WaterGameplayRole::PotentiallyNavigable
            }
            Self::Canal | Self::Basin | Self::Dock | Self::Other => WaterGameplayRole::Limited,
            Self::Stream | Self::Drainage | Self::Decorative => WaterGameplayRole::NonNavigable,
            Self::Playa => WaterGameplayRole::DrySeasonal,
            Self::Reef => WaterGameplayRole::UnderwaterContext,
            Self::CompatibilityUnknown => WaterGameplayRole::CompatibilityUnknown,
        }
    }

    /// Stable specificity winner for overlapping inland polygons. Ocean is a
    /// separate coastline authority and always wins before this comparison.
    pub const fn precedence(self) -> u8 {
        match self {
            Self::CompatibilityUnknown => 0,
            Self::Other => 1,
            Self::InlandWater => 2,
            Self::Decorative => 3,
            Self::Dock => 4,
            Self::Basin => 5,
            Self::Drainage => 6,
            Self::Stream => 7,
            Self::Canal => 8,
            Self::River => 9,
            Self::Lake => 10,
            Self::Reservoir => 11,
            Self::Ocean => 12,
            Self::Playa | Self::Reef => 0,
        }
    }
}

pub struct Water {
    pub rings: Rings,
    pub kind: WaterRegionKind,
    pub identity: SourceFeatureIdentity,
    pub intermittent: bool,
    pub alkaline: bool,
}

/// One source watercourse after same-identity fragments at the winning source
/// zoom have been reconciled. The paths remain centerlines.
#[derive(Debug, Clone, PartialEq)]
pub struct Watercourse {
    pub paths: Vec<Vec<[f32; 2]>>,
    pub kind: WatercourseKind,
    pub width_m: f32,
    pub identity: SourceFeatureIdentity,
    /// Stable width/appearance identity. An id-less source falls back to a
    /// class-global value so clipped fragments agree visually without being
    /// incorrectly joined into one physical feature.
    pub style_identity: u64,
    pub source_tile: TileKey,
    /// Authored water-layer maximum zoom. Physical rollout admits only
    /// centerlines extracted at this authority so adjacent LOD queries cannot
    /// alternate between differently simplified source geometries.
    pub authority_zoom: u8,
    pub source_feature_index: usize,
    pub source_min_zoom: Option<f32>,
    pub fragment_count: usize,
    pub intermittent: bool,
    pub is_bridge: bool,
}

impl Watercourse {
    pub const fn visible_at_terrain_level(&self, level: u8) -> bool {
        level >= self.kind.minimum_terrain_level()
    }
}

impl Water {
    pub fn inland(rings: Rings) -> Self {
        Self {
            rings,
            kind: WaterRegionKind::InlandWater,
            identity: SourceFeatureIdentity::DeterministicFallback(0),
            intermittent: false,
            alkaline: false,
        }
    }

    pub fn ocean(rings: Rings) -> Self {
        Self {
            rings,
            kind: WaterRegionKind::Ocean,
            identity: SourceFeatureIdentity::DeterministicFallback(0),
            intermittent: false,
            alkaline: false,
        }
    }

    pub const fn is_ocean(&self) -> bool {
        self.kind.is_ocean()
    }

    pub const fn creates_surface_water(&self) -> bool {
        self.kind.creates_surface_water()
    }
}

pub struct FeaturePack {
    pub buildings: Vec<Building>,
    pub observation_hints: Vec<ObservationHint>,
    pub exact_trees: Vec<ExactTree>,
    pub cliffs: Vec<CliffLine>,
    pub roads: Vec<Road>,
    pub water: Vec<Water>,
    pub watercourses: Vec<Watercourse>,
    pub watercourse_output_mode: WatercourseOutputMode,
    pub water_polygon_mode: WaterPolygonMode,
    /// Tree-bearing polygons (forests, woods).
    pub forest: Vec<Rings>,
    /// Ground-tint polygons.
    pub ground: Vec<(GroundKind, Rings)>,
    /// Source-preserved agricultural and floral polygons.
    pub flora: Vec<FloraRegion>,
    /// Coarse official z0–7 landcover, kept separate from detailed landuse so
    /// consumer authority remains explicit.
    pub regional_landcover: Vec<RegionalLandcoverRegion>,
    pub regional_landcover_mode: RegionalLandcoverMode,
    /// Canonical physical bridges from the prepared bridge catalog. Unlike
    /// `roads`, these are complete, source-zoom-independent objects rather
    /// than geometry clipped to this terrain cell's vector-tile window.
    pub bridges: Vec<CanonicalBridge>,
    /// True when `bridges` came from an authoritative catalog query. When
    /// set, the renderer must not also draw tile-local `is_bridge` ways.
    pub bridge_authority: bool,
    /// False when any tile fetch failed: the pack may be missing features.
    /// Consumers must not persist chunks generated from an incomplete pack
    /// (a transient network blip would otherwise be baked into the cache).
    pub complete: bool,
}

impl FeaturePack {
    pub fn empty() -> Self {
        FeaturePack {
            buildings: Vec::new(),
            observation_hints: Vec::new(),
            exact_trees: Vec::new(),
            cliffs: Vec::new(),
            roads: Vec::new(),
            water: Vec::new(),
            watercourses: Vec::new(),
            watercourse_output_mode: WatercourseOutputMode::DisabledG96,
            water_polygon_mode: WaterPolygonMode::Typed,
            forest: Vec::new(),
            ground: Vec::new(),
            flora: Vec::new(),
            regional_landcover: Vec::new(),
            regional_landcover_mode: RegionalLandcoverMode::SelectiveRegionalTerrain,
            bridges: Vec::new(),
            bridge_authority: false,
            complete: true,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buildings.is_empty()
            && self.observation_hints.is_empty()
            && self.exact_trees.is_empty()
            && self.cliffs.is_empty()
            && self.roads.is_empty()
            && self.water.is_empty()
            && self.watercourses.is_empty()
            && self.forest.is_empty()
            && self.ground.is_empty()
            && self.flora.is_empty()
            && self.regional_landcover.is_empty()
            && self.bridges.is_empty()
    }

    pub fn regional_landcover_projected_bytes(&self) -> usize {
        let mut bytes =
            self.regional_landcover.capacity() * std::mem::size_of::<RegionalLandcoverRegion>();
        for region in &self.regional_landcover {
            bytes += region.rings.capacity() * std::mem::size_of::<Vec<[f32; 2]>>();
            bytes += region
                .rings
                .iter()
                .map(|ring| ring.capacity() * std::mem::size_of::<[f32; 2]>())
                .sum::<usize>();
        }
        bytes
    }

    pub fn watercourse_projected_bytes(&self) -> usize {
        self.watercourses.capacity() * std::mem::size_of::<Watercourse>()
            + self
                .watercourses
                .iter()
                .map(|watercourse| {
                    watercourse.paths.capacity() * std::mem::size_of::<Vec<[f32; 2]>>()
                        + watercourse
                            .paths
                            .iter()
                            .map(|path| path.capacity() * std::mem::size_of::<[f32; 2]>())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}

/// User-facing geographic identity suitable for an arrival toast. The name is
/// already composed with first-level region context when useful. Countries
/// are never part of this model or its display string.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedLocation {
    pub id: u64,
    pub name: String,
    pub primary_name: Option<String>,
    pub region_name: Option<String>,
    /// Source names for the selected primary only; an empty language is `name`.
    /// These are ordinary Unicode `name:*` values, never provider-shaped glyphs.
    pub localized_names: Vec<(String, String)>,
    pub distance_m: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct GeographicCandidate {
    id: u64,
    name: String,
    localized_names: Vec<(String, String)>,
    score: f64,
    distance_m: f64,
    containing: bool,
    populated_place: bool,
}

/// The generator's window onto Tier-0 vector data. `lat0`/`lon0` in radians;
/// features come back in local east/north meters about that point, covering
/// at least the ±half_e × ±half_n box.
pub trait VectorSource: Send + Sync {
    fn features(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) -> FeaturePack;

    /// Optional stable-detail building set for detached night-light proxies.
    /// Providers with a vector pyramid use one fixed source zoom; the default
    /// delegates to the ordinary feature query. Production cell plans do not
    /// currently request this data.
    fn night_buildings(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) -> FeaturePack {
        self.features(lat0, lon0, half_e_m, half_n_m)
    }

    /// Warm caches for the given window with all fetches in flight
    /// concurrently; a later `features` call for the same window then hits
    /// memory. Default: no-op.
    fn prefetch_vectors(&self, _lat0: f64, _lon0: f64, _half_e_m: f64, _half_n_m: f64) {}

    fn prefetch_night_buildings(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) {
        self.prefetch_vectors(lat0, lon0, half_e_m, half_n_m);
    }

    /// Data-source attribution line.
    fn attribution(&self) -> String {
        String::new()
    }
}

/// z/x/y of the Web-Mercator tile containing (lat, lon) at zoom z.
fn mercator_tile(lat: f64, lon: f64, z: u8) -> (u32, u32) {
    let n = (1u64 << z) as f64;
    let x = ((lon.to_degrees() + 180.0) / 360.0 * n).floor();
    let lat_c = lat.clamp(-1.484_422, 1.484_422);
    let y =
        ((1.0 - ((lat_c.tan() + 1.0 / lat_c.cos()).ln()) / std::f64::consts::PI) / 2.0 * n).floor();
    let max = (1u64 << z) - 1;
    (
        (x as i64).rem_euclid(1i64 << z) as u32,
        (y.max(0.0) as u64).min(max) as u32,
    )
}

/// West-to-east modular x span, inclusive. A wrapped bbox near +180° must
/// enumerate `n-1, 0`, not the enormous numeric min..max interval between
/// them. `full_world` disambiguates windows at least one circumference wide.
fn mercator_x_span(x_west: u32, x_east: u32, n: u32, full_world: bool) -> (u32, u32) {
    let count = if full_world {
        n
    } else {
        (((x_east as u64 + n as u64 - x_west as u64) % n as u64) + 1) as u32
    };
    (x_west, count)
}

/// A decoded-tile cache slot. `OnceLock` gives in-flight deduplication for
/// free: when several workers miss on the same tile simultaneously, exactly
/// one fetches while the rest block on the slot, preventing duplicate range
/// requests. A typed error marks a failed
/// fetch; the initializing thread evicts the slot so a later query retries.
type TileSlot = std::sync::OnceLock<Result<Arc<Option<SemanticTile>>, DataError>>;
pub type SourceTileKey = (u8, u32, u32);
type TileKey = SourceTileKey;
type DecodedTileMap = HashMap<TileKey, (Arc<TileSlot>, u64)>;
type SharedDecodedTiles = Arc<Mutex<DecodedTileMap>>;
#[cfg(feature = "async-http")]
#[derive(Clone)]
enum AsyncVectorTileAcquisition {
    Tile(Arc<Option<SemanticTile>>),
    TransientFailure,
    DecodeFailure,
}

#[cfg(feature = "async-http")]
type AsyncTileMap = HashMap<
    TileKey,
    (
        Arc<crate::pmtiles::AsyncSlot<AsyncVectorTileAcquisition>>,
        crate::async_http::RequestPriority,
    ),
>;
#[cfg(feature = "async-http")]
type SharedAsyncTiles = Arc<Mutex<AsyncTileMap>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VectorQueryKind {
    TerrainDetail,
    TerrainLandcover,
    NightBuildings,
    LocationContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryAuthority {
    AdaptiveWindow,
    AuthoredLayerMaximum,
    FixedGroundDetail,
    ContextZoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryRequirement {
    PointLinePolygon,
    LinePolygon,
    Polygon,
    BuildingPolygon,
    ContextGeometry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryCompleteness {
    Required,
    Optional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryRanking {
    PhysicalClassification,
    SourceOrder,
    MinimumHeightThenSource,
    ExistingLocationScore,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedVectorQuery {
    pub kind: VectorQueryKind,
    pub layers: &'static [&'static str],
    pub authority: QueryAuthority,
    pub geometry: GeometryRequirement,
    pub ranking: QueryRanking,
    pub completeness: QueryCompleteness,
    pub bounds: (f64, f64, f64, f64),
    /// Additional margin beyond the caller's already-conservative bounds.
    pub extra_margin_m: f64,
    pub zoom: u8,
    pub tiles: Vec<SourceTileKey>,
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VectorQueryPlan {
    pub queries: Vec<PlannedVectorQuery>,
}

impl VectorQueryPlan {
    fn observe_required_caps(self) -> Self {
        use crate::metrics::RequiredQueryClass;
        for query in &self.queries {
            if !query.capped || query.completeness != QueryCompleteness::Required {
                continue;
            }
            let kind = match query.kind {
                VectorQueryKind::TerrainDetail => RequiredQueryClass::TerrainDetail,
                VectorQueryKind::TerrainLandcover => RequiredQueryClass::TerrainLandcover,
                VectorQueryKind::NightBuildings => RequiredQueryClass::NightBuildings,
                VectorQueryKind::LocationContext => continue,
            };
            crate::metrics::record_required_query_cap(kind, query.zoom);
        }
        self
    }

    pub fn unique_tile_keys(&self) -> Vec<SourceTileKey> {
        let mut keys: Vec<_> = self
            .queries
            .iter()
            .flat_map(|query| query.tiles.iter().copied())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    pub fn required_complete(&self) -> bool {
        self.queries
            .iter()
            .all(|query| query.completeness == QueryCompleteness::Optional || !query.capped)
    }

    fn query(&self, kind: VectorQueryKind) -> Option<&PlannedVectorQuery> {
        self.queries.iter().find(|query| query.kind == kind)
    }
}

/// Concurrent archive range reads. Each cell at the vector level waits on a
/// directory lookup and tile reads of about a second each from the public
/// build, so throughput follows requests in flight.
#[cfg(feature = "async-http")]
pub const ASYNC_RANGE_LIMIT: usize = 64;
/// Archive byte ranges are immutable; see [`crate::async_http::StallRetry`].
#[cfg(feature = "async-http")]
pub const ASYNC_STALL_RETRY: crate::async_http::StallRetry = crate::async_http::StallRetry {
    stall: vearth_platform::time::Duration::from_secs(4),
    attempts: 4,
};

pub struct ProtomapsSource {
    archive: Arc<PmTiles>,
    /// Cache tag identifying the dataset version (e.g. the build date).
    pub build_tag: String,
    descriptor: SourceDescriptor,
    source_cache: Option<SourceTileCache>,
    disk_metrics: crate::metrics::Acquisition,
    decoded: SharedDecodedTiles,
    clock: Arc<std::sync::atomic::AtomicU64>,
    normalizations: Arc<std::sync::atomic::AtomicU64>,
    semantic_telemetry: Arc<SemanticTelemetry>,
    bridge_catalog: Option<Arc<BridgeCatalog>>,
    regional_landcover_mode: RegionalLandcoverMode,
    water_polygon_mode: WaterPolygonMode,
    watercourse_output_mode: WatercourseOutputMode,
    #[cfg(feature = "async-http")]
    async_prepared: bool,
    #[cfg(feature = "async-http")]
    async_inflight: SharedAsyncTiles,
}

impl ProtomapsSource {
    /// Enforce the soft cap without ever evicting a slot that a blocking
    /// caller is still initializing. If every candidate is in flight, the
    /// map may exceed the cap briefly and is pruned after a fill completes.
    fn prune_decoded_completed(cache: &mut DecodedTileMap) {
        Self::prune_decoded_completed_with_limits(cache, TILE_CACHE_MAX, TILE_CACHE_MAX_BYTES);
    }

    fn prune_decoded_completed_with_limits(
        cache: &mut DecodedTileMap,
        max_entries: usize,
        max_bytes: usize,
    ) {
        let completed_bytes = |cache: &DecodedTileMap| {
            cache
                .values()
                .filter_map(|(slot, _)| slot.get())
                .filter_map(|result| result.as_ref().ok())
                .filter_map(|tile| tile.as_ref().as_ref())
                .map(SemanticTile::estimated_heap_bytes)
                .sum::<usize>()
        };
        while cache.len() > max_entries || completed_bytes(cache) > max_bytes {
            let victim = cache
                .iter()
                .filter(|(_, (slot, _))| match slot.get() {
                    Some(Ok(tile)) => Arc::strong_count(tile) == 1,
                    Some(Err(_)) => true,
                    None => false,
                })
                .min_by_key(|(_, (_, used_at))| *used_at)
                .map(|(key, _)| *key);
            let Some(victim) = victim else { break };
            cache.remove(&victim);
        }
    }

    pub fn new(
        archive: PmTiles,
        build_tag: impl Into<String>,
        source_cache: Option<SourceTileCache>,
    ) -> Result<Self, DataError> {
        let build_tag = build_tag.into();
        let metadata_json = archive.metadata_json().map_err(|error| {
            DataError::Decode(format!(
                "protomaps source {build_tag}: metadata unavailable: {error}"
            ))
        })?;
        Self::from_metadata(archive, build_tag, source_cache, &metadata_json)
    }

    fn from_metadata(
        archive: PmTiles,
        build_tag: String,
        source_cache: Option<SourceTileCache>,
        metadata_json: &str,
    ) -> Result<Self, DataError> {
        let descriptor =
            SourceDescriptor::from_metadata(&build_tag, &archive.header, metadata_json)?;
        if descriptor.compatibility == Compatibility::Incompatible {
            return Err(DataError::Decode(format!(
                "incompatible Protomaps archive: {}",
                descriptor.diagnostic_summary()
            )));
        }
        if let Some(cache) = &source_cache {
            let legacy_mismatch = match cache.legacy_layout() {
                None => false,
                Some(LegacyLayout::Vectors {
                    build_tag: legacy, ..
                }) => legacy != &build_tag,
                Some(_) => true,
            };
            if cache.provider() != "vectors" || cache.revision() != build_tag || legacy_mismatch {
                return Err(DataError::Decode(format!(
                    "protomaps source {build_tag}: source cache belongs to {}/{}",
                    cache.provider(),
                    cache.revision()
                )));
            }
        }
        let bridge_catalog = source_cache
            .as_ref()
            .and_then(|cache| match cache.legacy_layout() {
                Some(LegacyLayout::Vectors { root, build_tag }) => {
                    Some(root.join(build_tag).join("bridges.vbp"))
                }
                _ => None,
            })
            .and_then(|path| match BridgeCatalog::open(&path) {
                Ok(catalog) => {
                    log::info!(
                        "bridges: prepared catalog {} ({} physical bridges)",
                        path.display(),
                        catalog.len()
                    );
                    Some(Arc::new(catalog))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    log::warn!("bridge catalog {}: {error}", path.display());
                    None
                }
            });
        let disk_metrics =
            crate::metrics::acquisition(format!("provider.vectors.{build_tag}.disk"));
        let semantic_telemetry = Arc::new(SemanticTelemetry::new(
            &build_tag,
            descriptor.metadata_fingerprint,
        ));
        let source = ProtomapsSource {
            archive: Arc::new(archive),
            build_tag,
            descriptor,
            source_cache,
            disk_metrics,
            decoded: Arc::new(Mutex::new(HashMap::new())),
            clock: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            normalizations: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            semantic_telemetry,
            bridge_catalog,
            regional_landcover_mode: RegionalLandcoverMode::SelectiveRegionalTerrain,
            water_polygon_mode: WaterPolygonMode::Typed,
            watercourse_output_mode: WatercourseOutputMode::RiversAndCanals,
            #[cfg(feature = "async-http")]
            async_prepared: false,
            #[cfg(feature = "async-http")]
            async_inflight: Arc::new(Mutex::new(HashMap::new())),
        };
        match source.descriptor.compatibility {
            Compatibility::Compatible => {
                log::info!("vectors: {}", source.descriptor.diagnostic_summary());
            }
            Compatibility::CompatibleWithUnknowns => {
                log::warn!("vectors: {}", source.descriptor.diagnostic_summary());
            }
            Compatibility::Incompatible => unreachable!("rejected above"),
        }
        Ok(source)
    }

    /// Cache-key component for the prepared physical-bridge catalog.
    pub fn bridge_catalog_tag(&self) -> String {
        self.bridge_catalog
            .as_ref()
            .map(|catalog| format!("bridges-{:016x}", catalog.content_hash()))
            .unwrap_or_else(|| "bridges-none".to_string())
    }

    pub fn source_descriptor(&self) -> &SourceDescriptor {
        &self.descriptor
    }

    /// Stable terrain-cache identity for the active semantic output modes.
    pub fn terrain_dataset_identity_tag(&self) -> String {
        format!(
            "{}-regional-landcover-{}-water-polygons-{}-watercourses-{}-bridge-roads-adaptive-v1",
            self.descriptor
                .terrain_dataset_identity(&self.bridge_catalog_tag()),
            match self.regional_landcover_mode {
                RegionalLandcoverMode::ExistingFarmlandOnly => "farmland-only",
                RegionalLandcoverMode::SelectiveRegionalTerrain => "selective-v1",
            },
            match self.water_polygon_mode {
                WaterPolygonMode::LegacyUndifferentiated => "legacy-g95",
                WaterPolygonMode::Typed => "typed-v1",
            },
            match self.watercourse_output_mode {
                WatercourseOutputMode::DisabledG96 => "disabled-g96",
                WatercourseOutputMode::RiversAndCanals => "rivers-canals-v1",
            },
        )
    }

    pub fn diagnostic_identity_tag(&self) -> String {
        self.descriptor
            .diagnostic_identity(&self.bridge_catalog_tag())
    }

    pub fn normalization_count(&self) -> u64 {
        self.normalizations
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn semantic_report(&self) -> crate::protomaps_observability::SemanticReportSnapshot {
        self.semantic_telemetry.snapshot()
    }

    pub fn set_semantic_observability(&self, enabled: bool) {
        self.semantic_telemetry.set_enabled(enabled);
    }

    pub fn with_regional_landcover_mode(mut self, mode: RegionalLandcoverMode) -> Self {
        self.regional_landcover_mode = mode;
        self
    }

    pub fn with_water_polygon_mode(mut self, mode: WaterPolygonMode) -> Self {
        self.water_polygon_mode = mode;
        self
    }

    pub fn with_watercourse_output_mode(mut self, mode: WatercourseOutputMode) -> Self {
        self.watercourse_output_mode = mode;
        self
    }

    #[cfg(feature = "async-http")]
    pub async fn new_async(
        archive: PmTiles,
        build_tag: impl Into<String>,
        source_cache: Option<SourceTileCache>,
    ) -> Result<Self, DataError> {
        assert!(
            archive.is_async(),
            "ProtomapsSource::new_async requires PmTiles::open_async"
        );
        let build_tag = build_tag.into();
        let metadata_json = archive.metadata_json_async().await.map_err(|error| {
            DataError::Decode(format!(
                "protomaps source {build_tag}: metadata unavailable: {error}"
            ))
        })?;
        let mut source = Self::from_metadata(archive, build_tag, source_cache, &metadata_json)?;
        source.async_prepared = true;
        Ok(source)
    }

    /// Open one reactor-native archive, fetching its independent startup ranges
    /// concurrently before applying the same source/schema/cache validation.
    #[cfg(feature = "async-http")]
    pub async fn open_async(
        fetcher: Arc<dyn crate::async_http::AsyncRangeFetcher>,
        build_tag: impl Into<String>,
        source_cache: Option<SourceTileCache>,
    ) -> Result<Self, DataError> {
        Self::open_async_with_startup(fetcher, build_tag, source_cache)
            .await
            .map(|(source, _)| source)
    }

    /// Open over the network and also return the archive's immutable startup
    /// bytes for [`Self::open_async_from_startup`] on a later launch.
    #[cfg(feature = "async-http")]
    pub async fn open_async_with_startup(
        fetcher: Arc<dyn crate::async_http::AsyncRangeFetcher>,
        build_tag: impl Into<String>,
        source_cache: Option<SourceTileCache>,
    ) -> Result<(Self, crate::pmtiles::PmTilesStartup), DataError> {
        let (archive, metadata, startup) = PmTiles::open_async_with_startup(fetcher).await?;
        let mut source = Self::from_metadata(archive, build_tag.into(), source_cache, &metadata)?;
        source.async_prepared = true;
        Ok((source, startup))
    }

    /// Open the build named by `build_tag` from startup bytes recorded when
    /// that same build was opened over the network. No network I/O occurs
    /// until tiles or leaf directories are prepared.
    #[cfg(feature = "async-http")]
    pub fn open_async_from_startup(
        fetcher: Arc<dyn crate::async_http::AsyncRangeFetcher>,
        build_tag: impl Into<String>,
        source_cache: Option<SourceTileCache>,
        startup: &crate::pmtiles::PmTilesStartup,
    ) -> Result<Self, DataError> {
        let (archive, metadata) = PmTiles::open_async_from_startup(fetcher, startup)?;
        let mut source = Self::from_metadata(archive, build_tag.into(), source_cache, &metadata)?;
        source.async_prepared = true;
        Ok(source)
    }

    /// Whether cache misses must be prepared through this source's async API.
    pub fn is_async(&self) -> bool {
        self.is_async_prepared()
    }

    fn is_async_prepared(&self) -> bool {
        #[cfg(feature = "async-http")]
        {
            self.async_prepared
        }
        #[cfg(not(feature = "async-http"))]
        {
            false
        }
    }

    fn location_tiles(&self, z: u8, lat: f64, lon: f64) -> Vec<(u32, u32)> {
        let (centre_x, centre_y) = mercator_tile(lat, lon, z);
        let n = 1_i64 << z;
        let mut tiles = Vec::with_capacity(9);
        for dy in -1_i64..=1 {
            let y = i64::from(centre_y) + dy;
            if !(0..n).contains(&y) {
                continue;
            }
            for dx in -1_i64..=1 {
                let x = (i64::from(centre_x) + dx).rem_euclid(n);
                tiles.push((x as u32, y as u32));
            }
        }
        tiles
    }

    pub fn location_query_plan(&self, zoom: u8, lat: f64, lon: f64) -> VectorQueryPlan {
        if lat.abs() > 1.48 {
            return VectorQueryPlan::default();
        }
        let zoom = zoom.clamp(self.archive.header.min_zoom, self.archive.header.max_zoom);
        VectorQueryPlan {
            queries: vec![PlannedVectorQuery {
                kind: VectorQueryKind::LocationContext,
                layers: LOCATION_CONTEXT_LAYERS,
                authority: QueryAuthority::ContextZoom,
                geometry: GeometryRequirement::ContextGeometry,
                ranking: QueryRanking::ExistingLocationScore,
                completeness: QueryCompleteness::Optional,
                bounds: (lat, lon, 0.0, 0.0),
                extra_margin_m: 0.0,
                zoom,
                tiles: self
                    .location_tiles(zoom, lat, lon)
                    .into_iter()
                    .map(|(x, y)| (zoom, x, y))
                    .collect(),
                capped: false,
            }],
        }
    }

    fn collect_named_locations(
        &self,
        z: u8,
        lat0: f64,
        lon0: f64,
    ) -> Result<Vec<GeographicCandidate>, DataError> {
        let plan = self.location_query_plan(z, lat0, lon0);
        let Some(query) = plan.query(VectorQueryKind::LocationContext) else {
            return Ok(Vec::new());
        };
        let z = query.zoom;
        let tiles: Vec<_> = query.tiles.iter().map(|&(_, x, y)| (x, y)).collect();
        let missing = self.uncached_of(query.zoom, &tiles);
        if !self.is_async_prepared() && missing.len() > 1 {
            vearth_platform::thread::scope(|scope| {
                for &(x, y) in &missing {
                    scope.spawn(move || {
                        let _ = self.decoded_tile(z, x, y);
                    });
                }
            });
        }

        // Some broad named areas carry their label on a companion POI while
        // the containing polygon is intentionally unnamed. Join them by
        // source id without caring what kind of area they represent.
        let mut linked_polygon_names = HashMap::new();
        for &(x, y) in &tiles {
            let Ok(holder) = self.decoded_tile(z, x, y) else {
                continue;
            };
            let Some(tile) = holder.as_ref() else {
                continue;
            };
            let Some(layer) = tile.semantic_layer("pois") else {
                continue;
            };
            for feature in &layer.features {
                if let (Some(id), Some(name)) = (
                    feature.source_id,
                    semantic_english_name(tile, layer, feature),
                ) {
                    linked_polygon_names.insert(
                        id,
                        (
                            name.to_string(),
                            semantic_localized_names(tile, layer, feature),
                        ),
                    );
                }
            }
        }

        let mut candidates = Vec::new();
        let mut first_error = None;
        let mut read_any = false;
        for (x, y) in tiles {
            let holder = match self.decoded_tile(z, x, y) {
                Ok(holder) => holder,
                Err(error) => {
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            read_any = true;
            let Some(tile) = holder.as_ref() else {
                continue;
            };

            for layer_name in ["natural", "landuse", "water", "places", "physical_point"] {
                let Some(layer) = tile.semantic_layer(layer_name) else {
                    continue;
                };
                let projection = TileProj {
                    z,
                    x,
                    y,
                    extent: layer.extent as f64,
                    lat0,
                    lon0,
                };
                let mut report = self.semantic_telemetry.consumer_report();
                let family = SemanticLayerFamily::from_name(layer_name);
                for semantic in &layer.features {
                    let Some(feature) = tile.source_feature(layer, semantic) else {
                        report.rejected(family, SemanticDiscardReason::InvalidValue);
                        continue;
                    };
                    let name = semantic_english_name(tile, layer, semantic).or_else(|| {
                        (semantic.geometry_type == GeomType::Polygon)
                            .then(|| {
                                semantic
                                    .source_id
                                    .and_then(|id| linked_polygon_names.get(&id))
                                    .map(|(name, _)| name.as_str())
                            })
                            .flatten()
                    });
                    let Some(name) = name else {
                        report.rejected(family, SemanticDiscardReason::UnsupportedSemantic);
                        continue;
                    };
                    let localized_names = if semantic_english_name(tile, layer, semantic).is_some()
                    {
                        semantic_localized_names(tile, layer, semantic)
                    } else {
                        semantic
                            .source_id
                            .and_then(|id| linked_polygon_names.get(&id))
                            .map(|(_, names)| names.clone())
                            .unwrap_or_default()
                    };
                    match semantic.geometry_type {
                        GeomType::Polygon if !feature.paths.is_empty() => {
                            let rings = projection.paths_to_en(&feature.paths);
                            if point_in_rings([0.0, 0.0], &rings) {
                                candidates.push(GeographicCandidate {
                                    id: semantic_location_id(semantic.source_id, layer_name, name),
                                    name: name.to_string(),
                                    localized_names: localized_names.clone(),
                                    score: semantic_containing_feature_score(
                                        tile, layer, semantic, z,
                                    ),
                                    distance_m: 0.0,
                                    containing: true,
                                    populated_place: false,
                                });
                                report.consumed(family);
                            } else {
                                report.discarded(family, SemanticDiscardReason::ConsumerBudget);
                            }
                        }
                        GeomType::Point
                            if !semantic_administrative_label_point(tile, layer, semantic) =>
                        {
                            let mut consumed = false;
                            for &[px, py] in feature.paths.iter().flatten() {
                                let [east, north] = projection.to_en(px, py);
                                let distance_m = f64::from(east.hypot(north));
                                let radius_m =
                                    semantic_point_influence_radius_m(tile, layer, semantic, z);
                                if distance_m <= radius_m {
                                    candidates.push(GeographicCandidate {
                                        id: semantic_location_id(
                                            semantic.source_id,
                                            layer_name,
                                            name,
                                        ),
                                        name: name.to_string(),
                                        localized_names: localized_names.clone(),
                                        score: semantic_point_feature_score(
                                            tile, layer, semantic, z, distance_m, radius_m,
                                        ),
                                        distance_m,
                                        containing: false,
                                        populated_place: layer_name == "places",
                                    });
                                    consumed = true;
                                }
                            }
                            if consumed {
                                report.consumed(family);
                            } else {
                                report.discarded(family, SemanticDiscardReason::ConsumerBudget);
                            }
                        }
                        GeomType::Point => {
                            report.discarded(family, SemanticDiscardReason::RankOrMinimumZoom)
                        }
                        _ => report.rejected(family, SemanticDiscardReason::UnsupportedGeometry),
                    }
                }
                self.semantic_telemetry.record_consumer(&report);
            }
        }
        if !read_any && let Some(error) = first_error {
            return Err(error);
        }
        Ok(candidates)
    }

    /// Resolve a coordinate to a coarse English geographic identity without
    /// calling a geocoding service. This method performs blocking PMTiles
    /// range reads on cache misses and must run on a worker thread.
    pub fn resolve_named_location(
        &self,
        lat: f64,
        lon: f64,
    ) -> Result<Option<NamedLocation>, DataError> {
        if self.is_async_prepared() || lat.abs() > 1.48 {
            return Err(DataError::NotAvailable);
        }
        let region = AdminRegionAtlas::bundled().region_at(lat, lon);
        let broad_zoom = self.archive.header.max_zoom.min(8);
        let mut candidates = self.collect_named_locations(broad_zoom, lat, lon)?;
        let overview_zoom = self.archive.header.max_zoom.min(10);
        if overview_zoom != broad_zoom {
            candidates.extend(self.collect_named_locations(overview_zoom, lat, lon)?);
        }

        if needs_location_detail(&candidates) && self.archive.header.max_zoom > overview_zoom {
            let detail_zoom = self.archive.header.max_zoom.min(12);
            candidates.extend(self.collect_named_locations(detail_zoom, lat, lon)?);
        }
        Ok(rank_named_location(candidates, region))
    }

    /// Resolve names using the terrain source's shared async tile flights and
    /// decoded cache. Retained ownership spans each semantic query, so normal
    /// terrain-cache pressure cannot turn a completed dependency into a miss.
    #[cfg(feature = "async-http")]
    pub async fn resolve_named_location_async(
        &self,
        lat: f64,
        lon: f64,
    ) -> Result<Option<NamedLocation>, DataError> {
        if !self.is_async_prepared() || lat.abs() > 1.48 {
            return Err(DataError::NotAvailable);
        }
        let region = AdminRegionAtlas::bundled().region_at(lat, lon);
        let broad_zoom = self.archive.header.max_zoom.min(8);
        let mut candidates = self
            .collect_named_locations_async(broad_zoom, lat, lon)
            .await?;
        let overview_zoom = self.archive.header.max_zoom.min(10);
        if overview_zoom != broad_zoom {
            candidates.extend(
                self.collect_named_locations_async(overview_zoom, lat, lon)
                    .await?,
            );
        }
        if needs_location_detail(&candidates) && self.archive.header.max_zoom > overview_zoom {
            let detail_zoom = self.archive.header.max_zoom.min(12);
            candidates.extend(
                self.collect_named_locations_async(detail_zoom, lat, lon)
                    .await?,
            );
        }
        Ok(rank_named_location(candidates, region))
    }

    #[cfg(feature = "async-http")]
    async fn collect_named_locations_async(
        &self,
        zoom: u8,
        lat: f64,
        lon: f64,
    ) -> Result<Vec<GeographicCandidate>, DataError> {
        use crate::async_prep::{DependencyOutcome, PreparedSources, RetainedPrepareFuture};
        let plan = self.location_query_plan(zoom, lat, lon);
        let futures: Vec<RetainedPrepareFuture<'_>> = plan
            .unique_tile_keys()
            .into_iter()
            .map(|key| {
                Box::pin(async move {
                    let mut prepared = PreparedSources::default();
                    let outcome = match self.prepare_async_tile_acquisition(key, 0.0).await {
                        AsyncVectorTileAcquisition::Tile(tile) => {
                            let outcome = if tile.is_some() {
                                DependencyOutcome::Ready
                            } else {
                                DependencyOutcome::NoData
                            };
                            prepared.retain(tile);
                            outcome
                        }
                        AsyncVectorTileAcquisition::TransientFailure
                        | AsyncVectorTileAcquisition::DecodeFailure => {
                            DependencyOutcome::TransientFailure
                        }
                    };
                    prepared.summary.record(outcome);
                    prepared
                }) as RetainedPrepareFuture<'_>
            })
            .collect();
        let retained = crate::async_prep::join_retained(futures).await;
        let result = self.collect_named_locations(zoom, lat, lon);
        drop(retained);
        result
    }

    /// Best zoom for a query of the given ground width: finest data (max
    /// zoom) once tiles are no smaller than the box, so one query touches a
    /// bounded tile count.
    fn zoom_for(&self, lat: f64, width_m: f64) -> u8 {
        let tile_span_z0 = EQUATOR_M * lat.cos().max(0.05);
        let z = (tile_span_z0 / width_m.max(1.0)).log2().floor().max(0.0) as u8;
        z.min(self.archive.header.max_zoom)
    }

    /// Fixed ground-detail zoom for optional detached night-light queries.
    /// Lowering z poleward keeps source detail and covering-tile counts stable.
    fn night_building_zoom(&self, lat: f64) -> u8 {
        let z = (NIGHT_BUILDING_EQUATOR_ZOOM + lat.cos().abs().max(0.05).log2()).round();
        (z.max(0.0) as u8).min(self.archive.header.max_zoom)
    }

    fn decode_tile_bytes(
        bytes: &[u8],
        key: (u8, u32, u32),
        normalizations: &std::sync::atomic::AtomicU64,
        telemetry: &SemanticTelemetry,
    ) -> Result<Option<SemanticTile>, DataError> {
        if bytes.is_empty() {
            telemetry.record_tile_outcome(SemanticTileOutcome::Absent);
            return Ok(None);
        }
        match mvt::decode_tile(bytes) {
            Ok(tile) => {
                normalizations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let tile = SemanticTile::normalize_at(tile, key);
                telemetry.record_normalized_tile(&tile);
                Ok(Some(tile))
            }
            Err(error) => {
                telemetry.record_tile_outcome(SemanticTileOutcome::DecodeFailure);
                let (z, x, y) = key;
                Err(DataError::Decode(format!("mvt {z}/{x}/{y}: {error}")))
            }
        }
    }

    fn decoded_uncached(
        &self,
        z: u8,
        x: u32,
        y: u32,
    ) -> Result<Arc<Option<SemanticTile>>, DataError> {
        let key = (z, x, y);
        if let Some(cache) = &self.source_cache {
            match cache.load(z, x, y) {
                Ok(Some(CachedTile::Present(bytes))) => {
                    self.disk_metrics.cache_hit(bytes.len());
                    match Self::decode_tile_bytes(
                        &bytes,
                        key,
                        &self.normalizations,
                        &self.semantic_telemetry,
                    ) {
                        Ok(tile) => return Ok(Arc::new(tile)),
                        Err(error) => {
                            log::warn!("corrupt vector disk tile {z}/{x}/{y}: {error}");
                            cache.remove_corrupt(z, x, y);
                        }
                    }
                }
                Ok(Some(CachedTile::Missing)) => {
                    self.disk_metrics.cache_hit(0);
                    self.semantic_telemetry
                        .record_tile_outcome(SemanticTileOutcome::Absent);
                    return Ok(Arc::new(None));
                }
                Ok(None) => self.disk_metrics.cache_miss(),
                Err(error) => {
                    self.disk_metrics.cache_miss();
                    log::warn!("vector source cache read {z}/{x}/{y}: {error}");
                }
            }
        }

        let raw = self.fetch_with_retry(z, x, y).inspect_err(|_error| {
            self.semantic_telemetry
                .record_tile_outcome(SemanticTileOutcome::FetchFailure);
        })?;
        let decoded = match raw.as_deref() {
            Some(bytes) => {
                Self::decode_tile_bytes(bytes, key, &self.normalizations, &self.semantic_telemetry)?
            }
            None => {
                self.semantic_telemetry
                    .record_tile_outcome(SemanticTileOutcome::Absent);
                None
            }
        };
        if let Some(cache) = &self.source_cache {
            match raw {
                Some(ref bytes) => {
                    let _ = cache.store_present(z, x, y, bytes.clone());
                }
                None => {
                    let _ = cache.store_missing(z, x, y);
                }
            }
        }
        Ok(Arc::new(decoded))
    }

    /// One retry after a short pause: worker bursts against the public
    /// bucket occasionally drop a request.
    fn fetch_with_retry(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, DataError> {
        match self.archive.tile(z, x, y) {
            Ok(t) => Ok(t),
            Err(first) => {
                vearth_platform::thread::sleep(vearth_platform::time::Duration::from_millis(250));
                self.archive.tile(z, x, y).map_err(|_| first)
            }
        }
    }

    /// Fetch+decode through the LRU, with in-flight deduplication (see
    /// `TileSlot`). Fetch *errors* are returned as Err and never stay cached
    /// — the next query retries. Genuine absence (no tile in the archive)
    /// caches as Ok(None).
    fn decoded_tile(&self, z: u8, x: u32, y: u32) -> Result<Arc<Option<SemanticTile>>, DataError> {
        use std::sync::atomic::Ordering;
        let key = (z, x, y);
        let clock = self.clock.fetch_add(1, Ordering::Relaxed);
        if self.is_async_prepared() {
            let mut cache = self.decoded.lock().unwrap();
            let Some((slot, used_at)) = cache.get_mut(&key) else {
                return Err(DataError::NotAvailable);
            };
            *used_at = clock;
            return match slot.get() {
                Some(Ok(tile)) => Ok(tile.clone()),
                Some(Err(error)) => Err(error.clone()),
                None => Err(DataError::NotAvailable),
            };
        }
        let slot = {
            let mut cache = self.decoded.lock().unwrap();
            if let Some(entry) = cache.get_mut(&key) {
                entry.1 = clock;
                entry.0.clone()
            } else {
                let slot = Arc::new(TileSlot::new());
                cache.insert(key, (slot.clone(), clock));
                Self::prune_decoded_completed(&mut cache);
                slot
            }
        };
        // Fetch + decode outside the lock; concurrent callers for the same
        // tile block here instead of fetching duplicates.
        let mut initialized_here = false;
        let result = slot
            .get_or_init(|| {
                initialized_here = true;
                match self.decoded_uncached(z, x, y) {
                    Ok(tile) => Ok(tile),
                    Err(e) => {
                        log::warn!("vector tile fetch/decode {z}/{x}/{y}: {e}");
                        Err(e)
                    }
                }
            })
            .clone();
        if initialized_here {
            let mut cache = self.decoded.lock().unwrap();
            if result.is_err() && cache.get(&key).is_some_and(|(s, _)| Arc::ptr_eq(s, &slot)) {
                // Evict the failed slot so a later query retries the fetch.
                cache.remove(&key);
            }
            Self::prune_decoded_completed(&mut cache);
        }
        result
    }

    /// Tiles from `tiles` with no cache slot at all (neither decoded nor
    /// currently being fetched by another worker).
    fn uncached_of(&self, z: u8, tiles: &[(u32, u32)]) -> Vec<(u32, u32)> {
        let cache = self.decoded.lock().unwrap();
        tiles
            .iter()
            .copied()
            .filter(|&(x, y)| !cache.contains_key(&(z, x, y)))
            .collect()
    }

    fn uncached_keys(&self, keys: &[SourceTileKey]) -> Vec<SourceTileKey> {
        let cache = self.decoded.lock().unwrap();
        keys.iter()
            .copied()
            .filter(|key| !cache.contains_key(key))
            .collect()
    }

    fn covering_tiles_at_zoom(
        &self,
        lat0: f64,
        lon0: f64,
        half_e_m: f64,
        half_n_m: f64,
        z: u8,
        limit: usize,
    ) -> Vec<(u32, u32)> {
        let z = z.min(self.archive.header.max_zoom);
        let dlat = half_n_m / MEAN_RADIUS;
        let dlon = half_e_m.abs() / (MEAN_RADIUS * lat0.cos().max(0.05));
        let (x0, y0) = mercator_tile(lat0 + dlat, lon0 - dlon, z);
        let (x1, y1) = mercator_tile(lat0 - dlat, lon0 + dlon, z);
        let n_tiles = 1u32 << z;
        let (ya, yb) = (y0.min(y1), y0.max(y1));
        let (x_start, x_count) = mercator_x_span(x0, x1, n_tiles, dlon >= std::f64::consts::PI);
        let mut out = Vec::new();
        for ty in ya..=yb.min(n_tiles - 1) {
            for step in 0..x_count {
                if out.len() >= limit {
                    return out;
                }
                let tx = (x_start + step) % n_tiles;
                out.push((tx, ty));
            }
        }
        out
    }

    fn planned_tiles_at_zoom(
        &self,
        bounds: (f64, f64, f64, f64),
        zoom: u8,
        limit: usize,
    ) -> (u8, Vec<SourceTileKey>, bool) {
        let zoom = zoom.clamp(self.archive.header.min_zoom, self.archive.header.max_zoom);
        let mut tiles =
            self.covering_tiles_at_zoom(bounds.0, bounds.1, bounds.2, bounds.3, zoom, limit + 1);
        let capped = tiles.len() > limit;
        tiles.truncate(limit);
        (
            zoom,
            tiles.into_iter().map(|(x, y)| (zoom, x, y)).collect(),
            capped,
        )
    }

    pub fn terrain_query_plan(
        &self,
        lat0: f64,
        lon0: f64,
        half_e_m: f64,
        half_n_m: f64,
    ) -> VectorQueryPlan {
        if lat0.abs() > 1.48 {
            return VectorQueryPlan::default();
        }
        let bounds = (lat0, lon0, half_e_m, half_n_m);
        let detail_zoom = self.zoom_for(lat0, half_e_m.max(half_n_m) * 2.0);
        let (detail_zoom, detail_tiles, detail_capped) =
            self.planned_tiles_at_zoom(bounds, detail_zoom, MAX_TILES_PER_QUERY);
        let landcover_zoom = self
            .descriptor
            .layers
            .get("landcover")
            .map_or(7, |layer| layer.max_zoom);
        let (landcover_zoom, landcover_tiles, landcover_capped) =
            self.planned_tiles_at_zoom(bounds, landcover_zoom, MAX_TILES_PER_QUERY);
        VectorQueryPlan {
            queries: vec![
                PlannedVectorQuery {
                    kind: VectorQueryKind::TerrainDetail,
                    layers: TERRAIN_DETAIL_LAYERS,
                    authority: QueryAuthority::AdaptiveWindow,
                    geometry: GeometryRequirement::PointLinePolygon,
                    ranking: QueryRanking::PhysicalClassification,
                    completeness: QueryCompleteness::Required,
                    bounds,
                    extra_margin_m: 0.0,
                    zoom: detail_zoom,
                    tiles: detail_tiles,
                    capped: detail_capped,
                },
                PlannedVectorQuery {
                    kind: VectorQueryKind::TerrainLandcover,
                    layers: TERRAIN_LANDCOVER_LAYERS,
                    authority: QueryAuthority::AuthoredLayerMaximum,
                    geometry: GeometryRequirement::Polygon,
                    ranking: QueryRanking::SourceOrder,
                    completeness: QueryCompleteness::Required,
                    bounds,
                    extra_margin_m: 0.0,
                    zoom: landcover_zoom,
                    tiles: landcover_tiles,
                    capped: landcover_capped,
                },
            ],
        }
        .observe_required_caps()
    }

    pub fn night_building_query_plan(
        &self,
        lat0: f64,
        lon0: f64,
        half_e_m: f64,
        half_n_m: f64,
    ) -> VectorQueryPlan {
        if lat0.abs() > 1.48 {
            return VectorQueryPlan::default();
        }
        let bounds = (lat0, lon0, half_e_m, half_n_m);
        let zoom = self.night_building_zoom(lat0);
        let (zoom, tiles, capped) =
            self.planned_tiles_at_zoom(bounds, zoom, MAX_NIGHT_TILES_PER_QUERY);
        VectorQueryPlan {
            queries: vec![PlannedVectorQuery {
                kind: VectorQueryKind::NightBuildings,
                layers: NIGHT_BUILDING_LAYERS,
                authority: QueryAuthority::FixedGroundDetail,
                geometry: GeometryRequirement::BuildingPolygon,
                ranking: QueryRanking::MinimumHeightThenSource,
                completeness: QueryCompleteness::Required,
                bounds,
                extra_margin_m: 0.0,
                zoom,
                tiles,
                capped,
            }],
        }
        .observe_required_caps()
    }

    fn warm_vector_plan(&self, plan: &VectorQueryPlan) {
        if self.is_async_prepared() {
            return;
        }
        let missing = self.uncached_keys(&plan.unique_tile_keys());
        match missing.len() {
            0 => {}
            1 => {
                let _ = self.decoded_tile(missing[0].0, missing[0].1, missing[0].2);
            }
            _ => vearth_platform::thread::scope(|scope| {
                // Keep blocking prefetch fan-out bounded.
                for batch in missing.chunks(missing.len().div_ceil(8)) {
                    scope.spawn(move || {
                        for &(z, x, y) in batch {
                            let _ = self.decoded_tile(z, x, y);
                        }
                    });
                }
            }),
        }
    }

    #[cfg(feature = "async-http")]
    fn preparation_query_plan(
        &self,
        request: &crate::async_prep::PrepareRequest,
    ) -> VectorQueryPlan {
        let mut plan = VectorQueryPlan::default();
        if let Some((lat0, lon0, half_e_m, half_n_m)) = request.vector_bounds {
            plan.queries.extend(
                self.terrain_query_plan(lat0, lon0, half_e_m, half_n_m)
                    .queries,
            );
        }
        if let Some((lat0, lon0, half_e_m, half_n_m)) = request.night_building_bounds {
            plan.queries.extend(
                self.night_building_query_plan(lat0, lon0, half_e_m, half_n_m)
                    .queries,
            );
        }
        plan
    }

    #[cfg(feature = "async-http")]
    fn cached_async_acquisition(&self, key: (u8, u32, u32)) -> Option<AsyncVectorTileAcquisition> {
        use std::sync::atomic::Ordering;

        let used_at = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut cache = self.decoded.lock().unwrap();
        let (slot, last_used) = cache.get_mut(&key)?;
        *last_used = used_at;
        match slot.get()? {
            Ok(tile) => Some(AsyncVectorTileAcquisition::Tile(tile.clone())),
            Err(DataError::Decode(_)) => Some(AsyncVectorTileAcquisition::DecodeFailure),
            Err(DataError::Http(_) | DataError::NotAvailable) => {
                Some(AsyncVectorTileAcquisition::TransientFailure)
            }
        }
    }

    #[cfg(feature = "async-http")]
    fn decode_prepared_bytes(
        bytes: Vec<u8>,
        key: (u8, u32, u32),
        normalizations: Arc<std::sync::atomic::AtomicU64>,
        telemetry: Arc<SemanticTelemetry>,
    ) -> Result<Arc<Option<SemanticTile>>, DataError> {
        Self::decode_tile_bytes(&bytes, key, &normalizations, &telemetry).map(Arc::new)
    }

    #[cfg(feature = "async-http")]
    async fn acquire_prepared_tile(
        archive: Arc<PmTiles>,
        source_cache: Option<SourceTileCache>,
        disk_metrics: crate::metrics::Acquisition,
        normalizations: Arc<std::sync::atomic::AtomicU64>,
        telemetry: Arc<SemanticTelemetry>,
        key: (u8, u32, u32),
        priority: crate::async_http::RequestPriority,
    ) -> Result<Arc<Option<SemanticTile>>, DataError> {
        let (z, x, y) = key;
        if let Some(cache) = source_cache.clone() {
            let read_cache = cache.clone();
            match vearth_platform::rt::task::spawn_blocking(move || read_cache.load(z, x, y)).await
            {
                Ok(Ok(Some(CachedTile::Present(bytes)))) => {
                    disk_metrics.cache_hit(bytes.len());
                    let decode_normalizations = normalizations.clone();
                    let decode_telemetry = telemetry.clone();
                    let decoded = vearth_platform::rt::task::spawn_blocking(move || {
                        Self::decode_prepared_bytes(
                            bytes,
                            key,
                            decode_normalizations,
                            decode_telemetry,
                        )
                    })
                    .await
                    .map_err(|error| {
                        DataError::Decode(format!("vector disk decode task: {error}"))
                    })?;
                    match decoded {
                        Ok(tile) => return Ok(tile),
                        Err(error) => {
                            log::warn!("corrupt vector disk tile {z}/{x}/{y}: {error}");
                            let _ = vearth_platform::rt::task::spawn_blocking(move || {
                                cache.remove_corrupt(z, x, y)
                            })
                            .await;
                        }
                    }
                }
                Ok(Ok(Some(CachedTile::Missing))) => {
                    disk_metrics.cache_hit(0);
                    telemetry.record_tile_outcome(SemanticTileOutcome::Absent);
                    return Ok(Arc::new(None));
                }
                Ok(Ok(None)) => disk_metrics.cache_miss(),
                Ok(Err(error)) => {
                    disk_metrics.cache_miss();
                    log::warn!("vector source cache read {z}/{x}/{y}: {error}");
                }
                Err(error) => {
                    disk_metrics.cache_miss();
                    log::warn!("vector tile {z}/{x}/{y} source-cache task: {error}");
                }
            }
        }

        let raw = match archive
            .tile_async_with_priority_handle(z, x, y, priority.clone())
            .await
        {
            Ok(tile) => tile,
            Err(first) => {
                // Reactor-native retry: never park an executor worker.
                vearth_platform::rt::time::sleep(vearth_platform::time::Duration::from_millis(250))
                    .await;
                archive
                    .tile_async_with_priority_handle(z, x, y, priority.clone())
                    .await
                    .map_err(|second| {
                        telemetry.record_tile_outcome(SemanticTileOutcome::FetchFailure);
                        log::warn!("vector tile fetch {z}/{x}/{y}: {first}; retry: {second}");
                        first
                    })?
            }
        };

        let persist = raw.clone();
        let decoded = match raw {
            Some(bytes) => vearth_platform::rt::task::spawn_blocking(move || {
                Self::decode_prepared_bytes(bytes, key, normalizations, telemetry)
            })
            .await
            .map_err(|error| DataError::Decode(format!("vector decode task: {error}")))??,
            None => {
                telemetry.record_tile_outcome(SemanticTileOutcome::Absent);
                Arc::new(None)
            }
        };

        if let Some(cache) = source_cache {
            let stored = vearth_platform::rt::task::spawn_blocking(move || match persist {
                Some(bytes) => cache.store_present(z, x, y, bytes),
                None => cache.store_missing(z, x, y),
            })
            .await;
            if let Err(error) = stored {
                log::warn!("vector tile {z}/{x}/{y} source-cache write task: {error}");
            }
        }
        Ok(decoded)
    }

    #[cfg(feature = "async-http")]
    fn publish_prepared_tile(
        decoded: &SharedDecodedTiles,
        clock: &Arc<std::sync::atomic::AtomicU64>,
        key: (u8, u32, u32),
        tile: Arc<Option<SemanticTile>>,
    ) -> Arc<Option<SemanticTile>> {
        use std::sync::atomic::Ordering;

        let used_at = clock.fetch_add(1, Ordering::Relaxed);
        let mut cache = decoded.lock().unwrap();
        if let Some((slot, last_used)) = cache.get_mut(&key) {
            *last_used = used_at;
            if let Some(Ok(canonical)) = slot.get() {
                // Consumers read the cache's canonical tile. A racing decode
                // must retain that exact Arc, not its discarded replacement.
                let canonical = canonical.clone();
                Self::prune_decoded_completed(&mut cache);
                return canonical;
            }
        }
        let slot = Arc::new(TileSlot::new());
        let _ = slot.set(Ok(tile.clone()));
        cache.insert(key, (slot, used_at));
        Self::prune_decoded_completed(&mut cache);
        tile
    }

    #[cfg(feature = "async-http")]
    async fn prepare_async_tile_acquisition(
        &self,
        key: (u8, u32, u32),
        priority: f64,
    ) -> AsyncVectorTileAcquisition {
        if !self.async_prepared {
            return AsyncVectorTileAcquisition::TransientFailure;
        }
        if let Some(acquisition) = self.cached_async_acquisition(key) {
            return acquisition;
        }

        let (slot, flight_priority, leader) = {
            let mut inflight = self.async_inflight.lock().unwrap();
            // Publication precedes flight retirement. Recheck while holding
            // the flight lock so a just-finished request cannot be followed by
            // another fetch after the earlier optimistic cache lookup missed.
            if let Some(acquisition) = self.cached_async_acquisition(key) {
                return acquisition;
            }
            if let Some((slot, flight_priority)) = inflight.get(&key) {
                (slot.clone(), flight_priority.clone(), false)
            } else {
                let slot = Arc::new(crate::pmtiles::AsyncSlot::new());
                let flight_priority = crate::async_http::RequestPriority::shared();
                inflight.insert(key, (slot.clone(), flight_priority.clone()));
                (slot, flight_priority, true)
            }
        };
        let _priority_lease = flight_priority.lease(priority);

        let waited_at = vearth_platform::time::Instant::now();
        if leader {
            let archive = self.archive.clone();
            let source_cache = self.source_cache.clone();
            let disk_metrics = self.disk_metrics.clone();
            let normalizations = self.normalizations.clone();
            let semantic_telemetry = self.semantic_telemetry.clone();
            let decoded = self.decoded.clone();
            let clock = self.clock.clone();
            let inflight = self.async_inflight.clone();
            let task_slot = slot.clone();
            vearth_platform::rt::spawn(async move {
                let acquired = Self::acquire_prepared_tile(
                    archive,
                    source_cache,
                    disk_metrics,
                    normalizations,
                    semantic_telemetry,
                    key,
                    flight_priority,
                )
                .await;
                let acquisition = match acquired {
                    Ok(tile) => {
                        let canonical = Self::publish_prepared_tile(&decoded, &clock, key, tile);
                        AsyncVectorTileAcquisition::Tile(canonical)
                    }
                    Err(error) => {
                        log::warn!("vector tile fetch {}/{}/{}: {error}", key.0, key.1, key.2);
                        match error {
                            DataError::Decode(_) => AsyncVectorTileAcquisition::DecodeFailure,
                            DataError::Http(_) | DataError::NotAvailable => {
                                AsyncVectorTileAcquisition::TransientFailure
                            }
                        }
                    }
                };

                task_slot.complete(acquisition);
                let mut flights = inflight.lock().unwrap();
                if flights
                    .get(&key)
                    .is_some_and(|(cached, _)| Arc::ptr_eq(cached, &task_slot))
                {
                    flights.remove(&key);
                }
                drop(flights);
            });
        }

        let acquisition = slot.wait().await;
        if !leader {
            self.disk_metrics.shared_wait(waited_at.elapsed());
        }
        acquisition
    }
}

#[cfg(feature = "async-http")]
impl crate::async_prep::AsyncSourcePreparer for ProtomapsSource {
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
            let plan = self.preparation_query_plan(request);
            let capped_required = plan
                .queries
                .iter()
                .filter(|query| query.capped && query.completeness == QueryCompleteness::Required)
                .count();
            let futures: Vec<crate::async_prep::RetainedPrepareFuture<'_>> = plan
                .unique_tile_keys()
                .into_iter()
                .map(|key| {
                    Box::pin(async move {
                        use crate::async_prep::DependencyOutcome;
                        let mut prepared = crate::async_prep::PreparedSources::default();
                        let outcome = match self
                            .prepare_async_tile_acquisition(key, request.priority)
                            .await
                        {
                            AsyncVectorTileAcquisition::Tile(tile) => {
                                let outcome = if tile.is_some() {
                                    DependencyOutcome::Ready
                                } else {
                                    DependencyOutcome::NoData
                                };
                                prepared.retain(tile);
                                outcome
                            }
                            AsyncVectorTileAcquisition::TransientFailure
                            | AsyncVectorTileAcquisition::DecodeFailure => {
                                DependencyOutcome::TransientFailure
                            }
                        };
                        prepared.summary.record(outcome);
                        prepared
                    }) as crate::async_prep::RetainedPrepareFuture<'_>
                })
                .collect();
            let mut prepared = crate::async_prep::join_retained(futures).await;
            for _ in 0..capped_required {
                // Preparation and synchronous consumption share the same
                // required-query completeness contract.
                prepared
                    .summary
                    .record(crate::async_prep::DependencyOutcome::TransientFailure);
            }
            prepared
        })
    }
}

/// Per-tile affine-ish projection: tile-extent coords → local EN meters.
struct TileProj {
    z: u8,
    x: u32,
    y: u32,
    extent: f64,
    lat0: f64,
    lon0: f64,
}

impl TileProj {
    fn to_en(&self, px: i32, py: i32) -> [f32; 2] {
        let n = (1u64 << self.z) as f64;
        let mx = (self.x as f64 + px as f64 / self.extent) / n;
        let my = (self.y as f64 + py as f64 / self.extent) / n;
        let lon = (mx * 360.0 - 180.0).to_radians();
        let lat = (std::f64::consts::PI * (1.0 - 2.0 * my)).sinh().atan();
        let mut dlon = lon - self.lon0;
        if dlon > std::f64::consts::PI {
            dlon -= std::f64::consts::TAU;
        } else if dlon < -std::f64::consts::PI {
            dlon += std::f64::consts::TAU;
        }
        [
            (dlon * self.lat0.cos() * MEAN_RADIUS) as f32,
            ((lat - self.lat0) * MEAN_RADIUS) as f32,
        ]
    }

    fn paths_to_en(&self, paths: &[Vec<[i32; 2]>]) -> Rings {
        paths
            .iter()
            .map(|p| p.iter().map(|&[x, y]| self.to_en(x, y)).collect())
            .collect()
    }

    fn road_geometry_with_location(&self, path: &[[i32; 2]]) -> (Vec<[f32; 2]>, Option<[f64; 2]>) {
        let geometry = path.iter().map(|&[x, y]| self.to_en(x, y)).collect();
        let Some(&first) = path.first() else {
            return (geometry, None);
        };
        // Canonical traversal fixes floating-point accumulation order when a
        // source line is reversed. The anchor stays on the actual route rather
        // than sampling a parcel enclosed by its bounding box.
        let reverse = path.iter().cmp(path.iter().rev()).is_gt();
        let point = |i: usize| path[if reverse { path.len() - 1 - i } else { i }];
        let length = |a: [i32; 2], b: [i32; 2]| {
            (f64::from(b[0]) - f64::from(a[0])).hypot(f64::from(b[1]) - f64::from(a[1]))
        };
        let total: f64 = (1..path.len())
            .map(|i| length(point(i - 1), point(i)))
            .sum();
        let mut remaining = total * 0.5;
        let mut midpoint = first.map(f64::from);
        for i in 1..path.len() {
            let a = point(i - 1);
            let b = point(i);
            let distance = length(a, b);
            if distance == 0.0 {
                continue;
            }
            if remaining <= distance {
                let fraction = remaining / distance;
                midpoint = std::array::from_fn(|axis| {
                    f64::from(a[axis]) + (f64::from(b[axis]) - f64::from(a[axis])) * fraction
                });
                break;
            }
            remaining -= distance;
        }
        let n = (1_u64 << self.z) as f64;
        let mx = (f64::from(self.x) + midpoint[0] / self.extent) / n;
        let my = (f64::from(self.y) + midpoint[1] / self.extent) / n;
        let latitude = (std::f64::consts::PI * (1.0 - 2.0 * my)).sinh().atan();
        let longitude = (mx * 360.0 - 180.0).to_radians();
        let longitude = (longitude + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        (geometry, Some([latitude, longitude]))
    }

    fn geometry_with_location(&self, paths: &[Vec<[i32; 2]>]) -> (Rings, Option<[f64; 2]>) {
        let mut lower = [i32::MAX; 2];
        let mut upper = [i32::MIN; 2];
        let rings = paths
            .iter()
            .map(|path| {
                path.iter()
                    .map(|&[x, y]| {
                        lower[0] = lower[0].min(x);
                        lower[1] = lower[1].min(y);
                        upper[0] = upper[0].max(x);
                        upper[1] = upper[1].max(y);
                        self.to_en(x, y)
                    })
                    .collect()
            })
            .collect();
        if lower[0] > upper[0] || lower[1] > upper[1] {
            return (rings, None);
        }
        let centre = [
            (f64::from(lower[0]) + f64::from(upper[0])) * 0.5,
            (f64::from(lower[1]) + f64::from(upper[1])) * 0.5,
        ];
        let n = (1_u64 << self.z) as f64;
        let mx = (f64::from(self.x) + centre[0] / self.extent) / n;
        let my = (f64::from(self.y) + centre[1] / self.extent) / n;
        let latitude = (std::f64::consts::PI * (1.0 - 2.0 * my)).sinh().atan();
        let longitude = (mx * 360.0 - 180.0).to_radians();
        let longitude = (longitude + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        (rings, Some([latitude, longitude]))
    }
}

fn road_class(kind: &str, kind_detail: Option<&str>) -> Option<RoadClass> {
    let broad = match kind {
        "highway" => RoadClass::Highway,
        "major_road" => RoadClass::Major,
        "medium_road" => RoadClass::Medium,
        "minor_road" => RoadClass::Minor,
        "other" if kind_detail == Some("living_street") => RoadClass::Minor,
        "path" => RoadClass::Path,
        "rail" => RoadClass::Rail,
        _ => return None,
    };
    if broad == RoadClass::Rail && matches!(kind_detail, Some("subway" | "disused")) {
        return None;
    }
    Some(match (broad, kind_detail) {
        (RoadClass::Major, Some("trunk" | "trunk_link")) => RoadClass::Highway,
        (RoadClass::Major, Some("tertiary" | "tertiary_link")) => RoadClass::Medium,
        (
            RoadClass::Minor,
            Some(
                "service" | "driveway" | "parking_aisle" | "alley" | "drive-through"
                | "emergency_access",
            ),
        ) => RoadClass::Service,
        _ => broad,
    })
}

fn road_class_for_feature(
    kind: &str,
    kind_detail: Option<&str>,
    is_bridge: bool,
) -> Option<RoadClass> {
    let class = road_class(kind, kind_detail)?;
    if class == RoadClass::Path && kind_detail == Some("steps") && !is_bridge {
        return None;
    }
    Some(if is_bridge && class == RoadClass::Service {
        RoadClass::Minor
    } else {
        class
    })
}

fn ground_kind(kind: &str) -> Option<GroundKind> {
    Some(match kind {
        "park" | "pitch" | "golf_course" | "cemetery" | "village_green" | "recreation_ground"
        | "allotments" | "playground" => GroundKind::Green,
        "pedestrian" | "platform" | "pier" => GroundKind::Paved,
        "beach" | "sand" | "dune" => GroundKind::Sand,
        "residential" | "commercial" | "industrial" | "retail" | "military" | "hospital"
        | "school" | "university" | "college" => GroundKind::Urban,
        _ => return None,
    })
}

fn managed_ground_kind(kind: Option<LanduseKind>) -> Option<GroundKind> {
    Some(match kind? {
        LanduseKind::Farmyard => GroundKind::Farmyard,
        LanduseKind::Railway => GroundKind::RailwayArea,
        LanduseKind::Aerodrome | LanduseKind::Airfield => GroundKind::AirportArea,
        LanduseKind::Runway | LanduseKind::Taxiway => GroundKind::RunwayArea,
        _ => return None,
    })
}

fn natural_ground_kind(kind: Option<LanduseKind>) -> Option<GroundKind> {
    Some(match kind? {
        LanduseKind::BareRock => GroundKind::BareRock,
        LanduseKind::Wetland => GroundKind::Wetland,
        LanduseKind::Scrub => GroundKind::Scrub,
        LanduseKind::Grass => GroundKind::NaturalGrass,
        LanduseKind::Glacier => GroundKind::Glacier,
        _ => return None,
    })
}

fn flora_region_kind(kind: &str) -> Option<FloraRegionKind> {
    Some(match kind {
        "farmland" => FloraRegionKind::Farmland,
        "meadow" => FloraRegionKind::Meadow,
        "garden" => FloraRegionKind::Garden,
        "orchard" => FloraRegionKind::Orchard,
        "vineyard" => FloraRegionKind::Vineyard,
        _ => return None,
    })
}

fn mix_seed(mut seed: u64) -> u64 {
    seed ^= seed >> 30;
    seed = seed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    seed ^= seed >> 27;
    seed = seed.wrapping_mul(0x94d0_49bb_1331_11eb);
    seed ^ (seed >> 31)
}

/// A source-coordinate point expressed on a zoom-independent, quantized
/// global Web-Mercator lattice. Source tile z/x/y select the coordinate
/// transform but are never themselves part of the resulting identity.
fn quantized_mercator_anchor(z: u8, x: u32, y: u32, point: [i32; 2], extent: u32) -> Option<u64> {
    if extent == 0 {
        return None;
    }
    let n = (1u64 << z) as f64;
    let gx = (x as f64 + point[0] as f64 / extent as f64) / n;
    let gy = (y as f64 + point[1] as f64 / extent as f64) / n;
    if !gx.is_finite() || !gy.is_finite() {
        return None;
    }
    // Twenty-four fractional bits are fine enough to distinguish fields
    // while absorbing harmless coordinate rounding between source zooms.
    let q = (1u64 << 24) as f64;
    let qx = (gx.rem_euclid(1.0) * q).round() as u64 & ((1 << 24) - 1);
    let qy = (gy.clamp(0.0, 1.0) * q).round() as u64 & ((1 << 25) - 1);
    Some(qx << 25 | qy)
}

/// Deterministic last-resort geometry identity on global Web-Mercator
/// coordinates. Sorting makes ring start/direction irrelevant, and source
/// tile coordinates are excluded after conversion to the fixed global grid.
fn flora_geometry_fallback(z: u8, x: u32, y: u32, paths: &[Vec<[i32; 2]>], extent: u32) -> u64 {
    let mut points: Vec<u64> = paths
        .iter()
        .flatten()
        .filter_map(|&point| quantized_mercator_anchor(z, x, y, point, extent))
        .collect();
    points.sort_unstable();
    points.dedup();
    points
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, point| {
            fnv1a(hash, &point.to_le_bytes())
        })
}

/// Stable flora identity in the contract order: MVT id, then a quantized
/// zoom-independent Web-Mercator anchor, then canonicalized global geometry.
fn flora_feature_seed(
    feature: &mvt::Feature,
    kind: FloraRegionKind,
    z: u8,
    x: u32,
    y: u32,
    extent: u32,
) -> u64 {
    let identity = if let Some(id) = feature.id {
        mix_seed(id ^ 0x666c_6f72_615f_6964)
    } else if let Some(anchor) = feature
        .paths
        .iter()
        .flatten()
        .next()
        .and_then(|&point| quantized_mercator_anchor(z, x, y, point, extent))
    {
        mix_seed(anchor ^ 0x666c_6f72_615f_616e)
    } else {
        mix_seed(flora_geometry_fallback(z, x, y, &feature.paths, extent) ^ 0x666c_6f72_615f_6765)
    };
    mix_seed(identity ^ kind.seed_tag())
}

fn regional_landcover_feature_identity(
    semantic: &crate::protomaps_semantics::SemanticFeature,
    feature: &mvt::Feature,
    kind: LandcoverKind,
    z: u8,
    x: u32,
    y: u32,
    extent: u32,
) -> u64 {
    if kind == LandcoverKind::Farmland {
        return flora_feature_seed(feature, FloraRegionKind::Farmland, z, x, y, extent);
    }
    let kind_tag = match kind {
        LandcoverKind::Barren => 0x6261_7272_656e_0000,
        LandcoverKind::Farmland => 0x6661_726d_6c61_6e64,
        LandcoverKind::Forest => 0x666f_7265_7374_0000,
        LandcoverKind::Glacier => 0x676c_6163_6965_7200,
        LandcoverKind::Grassland => 0x6772_6173_736c_616e,
        LandcoverKind::Scrub => 0x7363_7275_6200_0000,
        LandcoverKind::UrbanArea => 0x7572_6261_6e00_0000,
        LandcoverKind::Unknown => 0x756e_6b6e_6f77_6e00,
    };
    let identity = match semantic.identity {
        SourceFeatureIdentity::Provided(value) => value,
        // Clipped polygon geometry differs across source tiles. A class-global
        // fallback avoids a visible tint/variation seam when no id is present.
        SourceFeatureIdentity::DeterministicFallback(_) => kind_tag,
    };
    mix_seed(identity ^ kind_tag)
}

/// Stable building identity. Prefer the MVT source id: unlike tile-local
/// geometry it survives the vector pyramid's zoom/generalization changes.
/// The fallback hashes a zoom-independent Web-Mercator coordinate, which is
/// still stable across tile borders and substantially safer than including
/// the source tile z/x/y directly.
fn feature_seed(id: Option<u64>, z: u8, x: u32, y: u32, first: [i32; 2], extent: u32) -> u64 {
    let mut s = if let Some(id) = id {
        id ^ 0x6275_696c_6469_6e67 // "building"
    } else {
        let n = (1u64 << z) as f64;
        let gx =
            ((x as f64 + first[0] as f64 / extent as f64) / n * u32::MAX as f64).round() as u64;
        let gy =
            ((y as f64 + first[1] as f64 / extent as f64) / n * u32::MAX as f64).round() as u64;
        gx << 32 | gy
    };
    s = s.wrapping_mul(0x9E3779B97F4A7C15);
    s ^ (s >> 31)
}

#[cfg(test)]
fn value_any<'a>(
    feature: &mvt::Feature,
    layer: &'a mvt::Layer,
    names: &[&str],
) -> Option<&'a Value> {
    names.iter().find_map(|name| feature.get(layer, name))
}

#[cfg(test)]
fn number_any(feature: &mvt::Feature, layer: &mvt::Layer, names: &[&str]) -> Option<f64> {
    value_any(feature, layer, names).and_then(|value| {
        value.as_f64().or_else(|| {
            value.as_str().and_then(|text| {
                text.trim()
                    .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c.is_whitespace())
                    .trim()
                    .parse()
                    .ok()
            })
        })
    })
}

#[cfg(test)]
fn english_name<'a>(feature: &mvt::Feature, layer: &'a mvt::Layer) -> Option<&'a str> {
    value_any(feature, layer, &["name:en", "name"])
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

fn semantic_english_name<'a>(
    tile: &'a SemanticTile,
    layer: &SemanticLayer,
    feature: &SemanticFeature,
) -> Option<&'a str> {
    ["name:en", "name"].into_iter().find_map(|desired| {
        feature.common.names.iter().find_map(|name| {
            (tile.source_name_key(layer, *name) == Some(desired))
                .then(|| tile.source_text(layer, name.value))
                .flatten()
                .map(str::trim)
                .filter(|name| !name.is_empty())
        })
    })
}

fn semantic_localized_names(
    tile: &SemanticTile,
    layer: &SemanticLayer,
    feature: &SemanticFeature,
) -> Vec<(String, String)> {
    let mut names: Vec<_> = feature.common.names.iter().filter_map(|fact| {
        let key = tile.source_name_key(layer, *fact)?;
        let language = if key == "name" { "" } else { key.strip_prefix("name:")? };
        // Restrict this to language tags: `name:left`, provenance, and encoded
        // pgf:name values are not alternate names for the whole feature.
        if !language.is_empty() && !(language.len() == 2 || language.len() == 3) {
            return None;
        }
        if !language.bytes().all(|b| b.is_ascii_lowercase()) { return None; }
        let text = tile.source_text(layer, fact.value)?.trim();
        if text.is_empty() || text.chars().count() > 160 || text.chars().any(|c| {
            c.is_control() || matches!(c as u32, 0xe000..=0xf8ff | 0xf0000..=0xffffd | 0x100000..=0x10fffd | 0x202a..=0x202e | 0x2066..=0x2069)
        }) { return None; }
        Some((language.to_owned(), text.to_owned()))
    }).collect();
    names.sort();
    names.dedup();
    names
}

fn semantic_feature_min_zoom(
    tile: &SemanticTile,
    layer: &SemanticLayer,
    feature: &SemanticFeature,
    source_zoom: u8,
) -> f64 {
    feature
        .common
        .min_zoom
        .and_then(|fact| tile.source_number(layer, fact))
        .unwrap_or(f64::from(source_zoom))
        .clamp(0.0, 20.0)
}

fn semantic_administrative_label_point(
    tile: &SemanticTile,
    layer: &SemanticLayer,
    feature: &SemanticFeature,
) -> bool {
    let administrative = |fact: Option<SourceFact>| {
        fact.and_then(|fact| tile.source_text(layer, fact))
            .is_some_and(|value| {
                matches!(
                    value,
                    "country" | "state" | "province" | "region" | "county"
                )
            })
    };
    if administrative(feature.common.place) {
        return true;
    }
    match &feature.facts {
        LayerFacts::Boundaries(facts) => {
            administrative(facts.kind) || administrative(facts.kind_detail)
        }
        LayerFacts::Buildings(facts) => {
            administrative(facts.physical.kind) || administrative(facts.physical.kind_detail)
        }
        LayerFacts::Earth(facts) => administrative(facts.kind),
        LayerFacts::Landcover(facts) => administrative(facts.kind),
        LayerFacts::Landuse(facts) => administrative(facts.kind),
        LayerFacts::Places(facts) => {
            administrative(facts.kind) || administrative(facts.kind_detail)
        }
        LayerFacts::Pois(facts) => administrative(facts.kind) || administrative(facts.kind_detail),
        LayerFacts::Natural(facts) | LayerFacts::PhysicalPoint(facts) => administrative(facts.kind),
        LayerFacts::Roads(facts) => {
            administrative(facts.physical.kind) || administrative(facts.physical.kind_detail)
        }
        LayerFacts::Water(facts) => {
            administrative(facts.physical.kind) || administrative(facts.physical.kind_detail)
        }
        LayerFacts::Other => false,
    }
}

fn semantic_point_influence_radius_m(
    tile: &SemanticTile,
    layer: &SemanticLayer,
    feature: &SemanticFeature,
    source_zoom: u8,
) -> f64 {
    let min_zoom = semantic_feature_min_zoom(tile, layer, feature, source_zoom);
    let cartographic_radius = 1_000.0 * 2.0_f64.powf((14.0 - min_zoom) * 0.5);
    let population = feature
        .common
        .population
        .and_then(|fact| tile.source_number(layer, fact))
        .unwrap_or_default();
    if population > 0.0 {
        (1_500.0 + population.sqrt() * 20.0).clamp(2_000.0, 60_000.0)
    } else {
        cartographic_radius.clamp(750.0, 150_000.0)
    }
}

fn semantic_point_feature_score(
    tile: &SemanticTile,
    layer: &SemanticLayer,
    feature: &SemanticFeature,
    source_zoom: u8,
    distance_m: f64,
    radius_m: f64,
) -> f64 {
    distance_m / radius_m + semantic_feature_min_zoom(tile, layer, feature, source_zoom) * 0.006
}

fn semantic_containing_feature_score(
    tile: &SemanticTile,
    layer: &SemanticLayer,
    feature: &SemanticFeature,
    source_zoom: u8,
) -> f64 {
    0.12 + semantic_feature_min_zoom(tile, layer, feature, source_zoom) * 0.006
}

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn semantic_location_id(source_id: Option<u64>, layer_name: &str, name: &str) -> u64 {
    let mut hash = fnv1a(0xcbf2_9ce4_8422_2325, layer_name.as_bytes());
    hash = fnv1a(hash, &[0]);
    if let Some(id) = source_id {
        fnv1a(hash, &id.to_le_bytes())
    } else {
        fnv1a(hash, name.as_bytes())
    }
}

fn compose_location_id(primary_id: Option<u64>, region_id: Option<u64>) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    if let Some(id) = primary_id {
        hash = fnv1a(hash, b"feature");
        hash = fnv1a(hash, &id.to_le_bytes());
    }
    if let Some(id) = region_id {
        hash = fnv1a(hash, b"region");
        hash = fnv1a(hash, &id.to_le_bytes());
    }
    hash
}

fn same_geographic_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn starts_with_geographic_name(value: &str, prefix: &str) -> bool {
    let value = value.trim();
    let prefix = prefix.trim();
    value.eq_ignore_ascii_case(prefix)
        || value
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            && value
                .as_bytes()
                .get(prefix.len())
                .is_some_and(|next| matches!(next, b' ' | b',' | b'-'))
}

fn needs_location_detail(candidates: &[GeographicCandidate]) -> bool {
    candidates
        .iter()
        .filter(|candidate| !candidate.containing)
        .map(|candidate| candidate.distance_m)
        .min_by(f64::total_cmp)
        .unwrap_or(f64::INFINITY)
        > 10_000.0
}

fn rank_named_location(
    mut candidates: Vec<GeographicCandidate>,
    region: Option<AdminRegion>,
) -> Option<NamedLocation> {
    candidates.sort_by(|a, b| {
        a.score
            .total_cmp(&b.score)
            .then_with(|| a.distance_m.total_cmp(&b.distance_m))
            .then_with(|| a.name.cmp(&b.name))
    });
    candidates.dedup_by_key(|candidate| candidate.id);
    compose_named_location(candidates.into_iter().next(), region)
}

fn compose_named_location(
    primary: Option<GeographicCandidate>,
    region: Option<AdminRegion>,
) -> Option<NamedLocation> {
    let primary = primary.filter(|candidate| {
        !region.as_ref().is_some_and(|region| {
            same_geographic_name(&candidate.name, &region.name)
                || (!candidate.populated_place
                    && starts_with_geographic_name(&candidate.name, &region.country_name))
        })
    });
    let region = region.filter(|region| !same_geographic_name(&region.name, &region.country_name));
    let primary_name = primary.as_ref().map(|candidate| candidate.name.clone());
    let region_name = region.as_ref().map(|region| region.name.clone());
    let name = match (&primary_name, &region_name) {
        (Some(primary), Some(region)) => format!("{primary}, {region}"),
        (Some(primary), None) => primary.clone(),
        (None, Some(region)) => region.clone(),
        (None, None) => return None,
    };
    Some(NamedLocation {
        id: compose_location_id(
            primary.as_ref().map(|candidate| candidate.id),
            region.as_ref().map(|region| region.id),
        ),
        name,
        primary_name,
        region_name,
        localized_names: primary
            .as_ref()
            .map(|p| p.localized_names.clone())
            .unwrap_or_default(),
        distance_m: primary
            .as_ref()
            .map_or(0.0, |candidate| candidate.distance_m),
    })
}

fn point_in_rings(point: [f32; 2], rings: &Rings) -> bool {
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

#[cfg(test)]
fn append_buildings(
    pack: &mut FeaturePack,
    tile: &mvt::VectorTile,
    tile_key: TileKey,
    origin: (f64, f64),
    minimum_height_m: Option<f32>,
) {
    let Some(layer) = tile.layer("buildings") else {
        return;
    };
    let (z, tx, ty) = tile_key;
    let (lat0, lon0) = origin;
    let p = TileProj {
        z,
        x: tx,
        y: ty,
        extent: layer.extent as f64,
        lat0,
        lon0,
    };
    for f in &layer.features {
        if f.geom_type != GeomType::Polygon || f.paths.is_empty() {
            continue;
        }
        if number_any(f, layer, &["layer"]).is_some_and(|layer| layer < 0.0) {
            continue;
        }
        let mut kind = match f.get(layer, "kind").and_then(Value::as_str) {
            Some("building") => BuildingKind::Building,
            Some("building_part" | "part") => BuildingKind::Part,
            _ => continue,
        };
        let height = number_any(f, layer, &["height", "height_m"])
            .filter(|h| h.is_finite())
            .map(|h| h.clamp(2.0, 1000.0) as f32);
        let min_height = number_any(f, layer, &["min_height", "min_height_m"])
            .filter(|h| h.is_finite())
            .map(|h| h.clamp(0.0, 900.0) as f32);
        if kind == BuildingKind::Part
            && f.get(layer, "kind_detail").and_then(Value::as_str) == Some("roof")
            && matches!((height, min_height), (Some(top), Some(base)) if top > base)
        {
            kind = BuildingKind::RoofOnly;
        }
        let min_height = min_height.unwrap_or(0.0);
        for component in polygon_components(&f.paths) {
            let seed = feature_seed(f.id, z, tx, ty, component.seed_anchor, layer.extent);
            let height = height.unwrap_or_else(|| 5.0 + (seed % 900) as f32 / 100.0);
            if minimum_height_m.is_some_and(|minimum| height - min_height < minimum) {
                continue;
            }
            let (rings, location_rad) = p.geometry_with_location(&component.paths);
            pack.buildings.push(Building {
                rings,
                location_rad,
                height_m: height,
                min_height_m: min_height.min(height - 1.0).max(0.0),
                seed,
                kind,
                roof_shape: building_roof(f.get(layer, "roof:shape").and_then(Value::as_str)),
                use_kind: building_use(f.get(layer, "kind_detail").and_then(Value::as_str)),
            });
        }
    }
}

#[cfg(test)]
fn append_observation_hints(
    pack: &mut FeaturePack,
    tile: &mvt::VectorTile,
    z: u8,
    tx: u32,
    ty: u32,
    lat0: f64,
    lon0: f64,
) {
    // Protomaps emits the POI authority at its maximum authored detail. Do
    // not scan broad regional POI layers that cannot contain the joinable
    // viewpoint records used by the near-building model.
    if z < 15 {
        return;
    }
    let Some(layer) = tile.layer("pois") else {
        return;
    };
    let projection = TileProj {
        z,
        x: tx,
        y: ty,
        extent: layer.extent as f64,
        lat0,
        lon0,
    };
    for feature in &layer.features {
        if feature.geom_type != GeomType::Point || feature.paths.is_empty() {
            continue;
        }
        let kind = match feature.get(layer, "kind").and_then(Value::as_str) {
            Some("viewpoint") => ObservationHintKind::Viewpoint,
            Some("attraction") => ObservationHintKind::Attraction,
            _ => continue,
        };
        let Some(first) = feature.paths.iter().flatten().next().copied() else {
            continue;
        };
        let seed = feature_seed(feature.id, z, tx, ty, first, layer.extent);
        for point in projection.paths_to_en(&feature.paths).into_iter().flatten() {
            pack.observation_hints
                .push(ObservationHint { point, kind, seed });
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn append_land_regions(
    pack: &mut FeaturePack,
    tile: &mvt::VectorTile,
    layer_name: &str,
    z: u8,
    tx: u32,
    ty: u32,
    lat0: f64,
    lon0: f64,
) {
    let Some(layer) = tile.layer(layer_name) else {
        return;
    };
    let projection = TileProj {
        z,
        x: tx,
        y: ty,
        extent: layer.extent as f64,
        lat0,
        lon0,
    };
    for feature in &layer.features {
        if feature.geom_type != GeomType::Polygon || feature.paths.is_empty() {
            continue;
        }
        let Some(source_kind) = feature.get(layer, "kind").and_then(Value::as_str) else {
            continue;
        };
        if layer_name == "landuse" && matches!(source_kind, "forest" | "wood") {
            // (Not nature_reserve: reserves span glaciers and deserts;
            // actual woods are tagged within.)
            pack.forest.push(projection.paths_to_en(&feature.paths));
        } else if let Some(kind) = flora_region_kind(source_kind) {
            let (rings, location_rad) = projection.geometry_with_location(&feature.paths);
            pack.flora.push(FloraRegion {
                location_rad,
                rings,
                kind,
                seed: flora_feature_seed(feature, kind, z, tx, ty, layer.extent),
            });
        } else if layer_name == "landuse"
            && let Some(kind) = ground_kind(source_kind)
        {
            pack.ground
                .push((kind, projection.paths_to_en(&feature.paths)));
        }
    }
}

fn may_anchor_observation_tower(building: &Building) -> bool {
    if building.kind == BuildingKind::RoofOnly || building.height_m < 45.0 {
        return false;
    }
    let Some(outer) = building.rings.first() else {
        return false;
    };
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for point in outer {
        low[0] = low[0].min(point[0]);
        low[1] = low[1].min(point[1]);
        high[0] = high[0].max(point[0]);
        high[1] = high[1].max(point[1]);
    }
    let a = high[0] - low[0];
    let b = high[1] - low[1];
    a.is_finite()
        && b.is_finite()
        && a.min(b) >= 1.0
        && a.max(b) <= 60.0
        && a.max(b) / a.min(b) <= 2.2
}

fn append_semantic_buildings(
    pack: &mut FeaturePack,
    tile: &SemanticTile,
    tile_key: TileKey,
    origin: (f64, f64),
    minimum_height_m: Option<f32>,
    report: &mut SemanticConsumerReport,
) {
    let Some(layer) = tile.semantic_layer("buildings") else {
        return;
    };
    let Some(source_layer) = tile.source_layer(layer) else {
        return;
    };
    let (z, tx, ty) = tile_key;
    let p = TileProj {
        z,
        x: tx,
        y: ty,
        extent: layer.extent as f64,
        lat0: origin.0,
        lon0: origin.1,
    };
    for semantic in &layer.features {
        let LayerFacts::Buildings(facts) = &semantic.facts else {
            report.rejected(
                SemanticLayerFamily::Buildings,
                SemanticDiscardReason::InvalidValue,
            );
            continue;
        };
        let Some(feature) = tile.source_feature(layer, semantic) else {
            report.rejected(
                SemanticLayerFamily::Buildings,
                SemanticDiscardReason::InvalidValue,
            );
            continue;
        };
        if semantic.geometry_type != GeomType::Polygon || feature.paths.is_empty() {
            report.rejected(
                SemanticLayerFamily::Buildings,
                SemanticDiscardReason::UnsupportedGeometry,
            );
            continue;
        }
        if facts
            .physical
            .layer
            .and_then(|fact| tile.source_number(layer, fact))
            .is_some_and(|layer| layer < 0.0)
        {
            report.rejected(
                SemanticLayerFamily::Buildings,
                SemanticDiscardReason::UnsupportedSemantic,
            );
            continue;
        }
        let mut kind = match facts
            .physical
            .kind
            .and_then(|fact| tile.source_text(layer, fact))
        {
            Some("building") => BuildingKind::Building,
            Some("building_part" | "part") => BuildingKind::Part,
            _ => {
                let reason = if facts.physical.kind.is_some_and(|fact| {
                    fact.normalized == crate::protomaps_semantics::NormalizedValue::Invalid
                }) {
                    SemanticDiscardReason::InvalidValue
                } else {
                    SemanticDiscardReason::UnsupportedSemantic
                };
                report.rejected(SemanticLayerFamily::Buildings, reason);
                continue;
            }
        };
        let height = facts
            .physical
            .height
            .and_then(|fact| tile.source_number(layer, fact))
            .filter(|height| height.is_finite())
            .map(|height| height.clamp(2.0, 1000.0) as f32);
        let min_height = facts
            .physical
            .min_height
            .and_then(|fact| tile.source_number(layer, fact))
            .filter(|height| height.is_finite())
            .map(|height| height.clamp(0.0, 900.0) as f32);
        if kind == BuildingKind::Part
            && facts
                .physical
                .kind_detail
                .and_then(|fact| tile.source_text(layer, fact))
                == Some("roof")
            && matches!((height, min_height), (Some(top), Some(base)) if top > base)
        {
            kind = BuildingKind::RoofOnly;
        }
        let min_height = min_height.unwrap_or(0.0);
        let initial_count = pack.buildings.len();
        for component in tile
            .source_polygon_components(layer, semantic)
            .into_iter()
            .flatten()
        {
            let seed = feature_seed(
                semantic.source_id,
                z,
                tx,
                ty,
                component.seed_anchor,
                source_layer.extent,
            );
            let height = height.unwrap_or_else(|| 5.0 + (seed % 900) as f32 / 100.0);
            if minimum_height_m.is_some_and(|minimum| height - min_height < minimum) {
                continue;
            }
            let (rings, location_rad) = p.geometry_with_location(&component.paths);
            pack.buildings.push(Building {
                rings,
                location_rad,
                height_m: height,
                min_height_m: min_height.min(height - 1.0).max(0.0),
                seed,
                kind,
                roof_shape: building_roof(
                    facts
                        .physical
                        .roof_shape
                        .and_then(|fact| tile.source_text(layer, fact)),
                ),
                use_kind: building_use(
                    facts
                        .physical
                        .kind_detail
                        .and_then(|fact| tile.source_text(layer, fact)),
                ),
            });
        }
        if pack.buildings.len() == initial_count {
            report.discarded(
                SemanticLayerFamily::Buildings,
                SemanticDiscardReason::RankOrMinimumZoom,
            );
            continue;
        }
        report.consumed(SemanticLayerFamily::Buildings);
    }
}

fn append_semantic_poi_features(
    pack: &mut FeaturePack,
    tile: &SemanticTile,
    tile_key: TileKey,
    origin: (f64, f64),
    observation_enabled: bool,
    report: &mut SemanticConsumerReport,
) {
    let (z, tx, ty) = tile_key;
    let Some(layer) = tile.semantic_layer("pois") else {
        return;
    };
    if z < 15 {
        for _ in &layer.features {
            report.discarded(
                SemanticLayerFamily::Pois,
                SemanticDiscardReason::RankOrMinimumZoom,
            );
        }
        return;
    }
    let Some(source_layer) = tile.source_layer(layer) else {
        return;
    };
    let projection = TileProj {
        z,
        x: tx,
        y: ty,
        extent: layer.extent as f64,
        lat0: origin.0,
        lon0: origin.1,
    };
    for semantic in &layer.features {
        let LayerFacts::Pois(facts) = &semantic.facts else {
            report.rejected(
                SemanticLayerFamily::Pois,
                SemanticDiscardReason::InvalidValue,
            );
            continue;
        };
        let Some(feature) = tile.source_feature(layer, semantic) else {
            report.rejected(
                SemanticLayerFamily::Pois,
                SemanticDiscardReason::InvalidValue,
            );
            continue;
        };
        if semantic.geometry_type != GeomType::Point || feature.paths.is_empty() {
            report.rejected(
                SemanticLayerFamily::Pois,
                SemanticDiscardReason::UnsupportedGeometry,
            );
            continue;
        }
        let source_kind = facts.kind.and_then(|fact| tile.source_text(layer, fact));
        if source_kind == Some("tree") {
            let Some(first) = feature.paths.iter().flatten().next().copied() else {
                report.rejected(
                    SemanticLayerFamily::Pois,
                    SemanticDiscardReason::UnsupportedGeometry,
                );
                continue;
            };
            let seed = match semantic.identity {
                SourceFeatureIdentity::Provided(id) => id ^ 0x4558_4143_545f_5452,
                SourceFeatureIdentity::DeterministicFallback(_) => {
                    feature_seed(semantic.source_id, z, tx, ty, first, source_layer.extent)
                }
            };
            for point in projection.paths_to_en(&feature.paths).into_iter().flatten() {
                pack.exact_trees.push(ExactTree {
                    point,
                    identity: semantic.identity,
                    seed,
                });
            }
            report.consumed(SemanticLayerFamily::Pois);
            continue;
        }
        let kind = match source_kind {
            Some("viewpoint") if observation_enabled => ObservationHintKind::Viewpoint,
            Some("attraction") if observation_enabled => ObservationHintKind::Attraction,
            _ if !observation_enabled => {
                report.discarded(
                    SemanticLayerFamily::Pois,
                    SemanticDiscardReason::ConsumerBudget,
                );
                continue;
            }
            _ => {
                report.rejected(
                    SemanticLayerFamily::Pois,
                    SemanticDiscardReason::UnsupportedSemantic,
                );
                continue;
            }
        };
        let Some(first) = feature.paths.iter().flatten().next().copied() else {
            report.rejected(
                SemanticLayerFamily::Pois,
                SemanticDiscardReason::UnsupportedGeometry,
            );
            continue;
        };
        let seed = feature_seed(semantic.source_id, z, tx, ty, first, source_layer.extent);
        for point in projection.paths_to_en(&feature.paths).into_iter().flatten() {
            pack.observation_hints
                .push(ObservationHint { point, kind, seed });
        }
        report.consumed(SemanticLayerFamily::Pois);
    }
}

fn exact_tree_identity_key(identity: SourceFeatureIdentity) -> (u8, u64) {
    match identity {
        SourceFeatureIdentity::Provided(id) => (0, id),
        SourceFeatureIdentity::DeterministicFallback(id) => (1, id),
    }
}

/// Resolve request-tile overlap before enforcing the physical tree budget.
/// Source order and irrelevant features elsewhere in a covering tile must not
/// decide which trees enter a persisted cell.
fn finalize_exact_trees(
    pack: &mut FeaturePack,
    half_e_m: f64,
    half_n_m: f64,
    report: &mut SemanticConsumerReport,
) {
    let (half_e_m, half_n_m) = (half_e_m.abs() as f32, half_n_m.abs() as f32);
    pack.exact_trees.retain(|tree| {
        tree.point[0].is_finite()
            && tree.point[1].is_finite()
            && tree.point[0].abs() <= half_e_m
            && tree.point[1].abs() <= half_n_m
    });
    pack.exact_trees.sort_by(|left, right| {
        exact_tree_identity_key(left.identity)
            .cmp(&exact_tree_identity_key(right.identity))
            .then_with(|| left.point[0].total_cmp(&right.point[0]))
            .then_with(|| left.point[1].total_cmp(&right.point[1]))
            .then_with(|| left.seed.cmp(&right.seed))
    });
    pack.exact_trees
        .dedup_by(|left, right| left.identity == right.identity);
    if pack.exact_trees.len() > MAX_EXACT_TREES_PER_PACK {
        pack.exact_trees.clear();
        pack.complete = false;
        report.discarded(
            SemanticLayerFamily::Pois,
            SemanticDiscardReason::ConsumerBudget,
        );
    }
}

fn append_semantic_cliffs(
    pack: &mut FeaturePack,
    tile: &SemanticTile,
    tile_key: TileKey,
    origin: (f64, f64),
    report: &mut SemanticConsumerReport,
) {
    let Some(layer) = tile.semantic_layer("earth") else {
        return;
    };
    let projection = TileProj {
        z: tile_key.0,
        x: tile_key.1,
        y: tile_key.2,
        extent: layer.extent as f64,
        lat0: origin.0,
        lon0: origin.1,
    };
    for semantic in &layer.features {
        let LayerFacts::Earth(facts) = &semantic.facts else {
            continue;
        };
        if facts.kind.and_then(|fact| tile.source_text(layer, fact)) != Some("cliff") {
            continue;
        }
        if semantic.geometry_type != GeomType::Line {
            report.rejected(
                SemanticLayerFamily::Earth,
                SemanticDiscardReason::UnsupportedGeometry,
            );
            continue;
        }
        let Some(feature) = tile.source_feature(layer, semantic) else {
            report.rejected(
                SemanticLayerFamily::Earth,
                SemanticDiscardReason::InvalidValue,
            );
            continue;
        };
        let mut consumed = false;
        for points in projection.paths_to_en(&feature.paths) {
            if points.len() < 2 {
                continue;
            }
            pack.cliffs.push(CliffLine {
                pts: points,
                identity: semantic.identity,
            });
            consumed = true;
        }
        if consumed {
            report.consumed(SemanticLayerFamily::Earth);
        } else {
            report.rejected(
                SemanticLayerFamily::Earth,
                SemanticDiscardReason::UnsupportedGeometry,
            );
        }
    }
}

fn finalize_cliffs(
    pack: &mut FeaturePack,
    half_e_m: f64,
    half_n_m: f64,
    report: &mut SemanticConsumerReport,
) {
    let (half_e_m, half_n_m) = (half_e_m.abs() as f32, half_n_m.abs() as f32);
    for cliff in &mut pack.cliffs {
        cliff
            .pts
            .retain(|point| point.iter().all(|value| value.is_finite()));
        cliff.pts.dedup();
        if cliff_points_cmp_reversed(&cliff.pts).is_lt() {
            cliff.pts.reverse();
        }
    }
    pack.cliffs.retain(|cliff| {
        if cliff.pts.len() < 2 {
            return false;
        }
        let mut low = [f32::INFINITY; 2];
        let mut high = [f32::NEG_INFINITY; 2];
        for point in &cliff.pts {
            low[0] = low[0].min(point[0]);
            low[1] = low[1].min(point[1]);
            high[0] = high[0].max(point[0]);
            high[1] = high[1].max(point[1]);
        }
        low[0] <= half_e_m && high[0] >= -half_e_m && low[1] <= half_n_m && high[1] >= -half_n_m
    });
    pack.cliffs.sort_by(|left, right| {
        exact_tree_identity_key(left.identity)
            .cmp(&exact_tree_identity_key(right.identity))
            .then_with(|| cliff_points_cmp(&left.pts, &right.pts))
    });
    pack.cliffs
        .dedup_by(|left, right| left.identity == right.identity && left.pts == right.pts);
    let points = pack
        .cliffs
        .iter()
        .try_fold(0usize, |sum, cliff| sum.checked_add(cliff.pts.len()));
    let bytes = pack.cliffs.capacity() * std::mem::size_of::<CliffLine>()
        + pack
            .cliffs
            .iter()
            .map(|cliff| cliff.pts.capacity() * std::mem::size_of::<[f32; 2]>())
            .sum::<usize>();
    if pack.cliffs.len() > MAX_CLIFF_LINES_PER_PACK
        || points.is_none_or(|points| points > MAX_CLIFF_POINTS_PER_PACK)
        || bytes > MAX_CLIFF_PROJECTED_BYTES
    {
        pack.cliffs.clear();
        pack.complete = false;
        report.discarded(
            SemanticLayerFamily::Earth,
            SemanticDiscardReason::ConsumerBudget,
        );
    }
}

fn cliff_points_cmp(left: &[[f32; 2]], right: &[[f32; 2]]) -> std::cmp::Ordering {
    left.iter()
        .zip(right)
        .find_map(|(left, right)| {
            let ordering = left[0]
                .total_cmp(&right[0])
                .then_with(|| left[1].total_cmp(&right[1]));
            ordering.ne(&std::cmp::Ordering::Equal).then_some(ordering)
        })
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn cliff_points_cmp_reversed(points: &[[f32; 2]]) -> std::cmp::Ordering {
    points
        .iter()
        .rev()
        .zip(points)
        .find_map(|(reversed, forward)| {
            let ordering = reversed[0]
                .total_cmp(&forward[0])
                .then_with(|| reversed[1].total_cmp(&forward[1]));
            ordering.ne(&std::cmp::Ordering::Equal).then_some(ordering)
        })
        .unwrap_or(std::cmp::Ordering::Equal)
}

#[allow(clippy::too_many_arguments)]
fn append_semantic_land_regions(
    pack: &mut FeaturePack,
    tile: &SemanticTile,
    layer_name: &str,
    z: u8,
    tx: u32,
    ty: u32,
    lat0: f64,
    lon0: f64,
    report: &mut SemanticConsumerReport,
) {
    let Some(layer) = tile.semantic_layer(layer_name) else {
        return;
    };
    let projection = TileProj {
        z,
        x: tx,
        y: ty,
        extent: layer.extent as f64,
        lat0,
        lon0,
    };
    for semantic in &layer.features {
        let (kind, landuse_class, landcover_class) = match &semantic.facts {
            LayerFacts::Landuse(facts) => (facts.kind, facts.class, None),
            LayerFacts::Landcover(facts) => (facts.kind, None, facts.class),
            _ => (None, None, None),
        };
        let Some(feature) = tile.source_feature(layer, semantic) else {
            report.rejected(
                SemanticLayerFamily::from_name(layer_name),
                SemanticDiscardReason::InvalidValue,
            );
            continue;
        };
        if semantic.geometry_type != GeomType::Polygon || feature.paths.is_empty() {
            report.rejected(
                SemanticLayerFamily::from_name(layer_name),
                SemanticDiscardReason::UnsupportedGeometry,
            );
            continue;
        }
        let Some(source_kind) = kind.and_then(|fact| tile.source_text(layer, fact)) else {
            report.rejected(
                SemanticLayerFamily::from_name(layer_name),
                SemanticDiscardReason::InvalidValue,
            );
            continue;
        };
        if layer_name == "landcover" {
            let Some(class) = landcover_class.filter(|class| *class != LandcoverKind::Unknown)
            else {
                report.rejected(
                    SemanticLayerFamily::Landcover,
                    SemanticDiscardReason::UnsupportedSemantic,
                );
                continue;
            };
            let (rings, location_rad) = projection.geometry_with_location(&feature.paths);
            pack.regional_landcover.push(RegionalLandcoverRegion {
                location_rad,
                rings,
                kind: class,
                source_id: semantic.source_id,
                stable_identity: regional_landcover_feature_identity(
                    semantic,
                    feature,
                    class,
                    z,
                    tx,
                    ty,
                    layer.extent,
                ),
            });
        } else if layer_name == "landuse" && matches!(source_kind, "forest" | "wood") {
            pack.forest.push(projection.paths_to_en(&feature.paths));
        } else if let Some(kind) = flora_region_kind(source_kind) {
            let (rings, location_rad) = projection.geometry_with_location(&feature.paths);
            pack.flora.push(FloraRegion {
                location_rad,
                rings,
                kind,
                seed: flora_feature_seed(feature, kind, z, tx, ty, layer.extent),
            });
        } else if layer_name == "landuse"
            && let Some(kind) = natural_ground_kind(landuse_class)
        {
            pack.ground
                .push((kind, projection.paths_to_en(&feature.paths)));
        } else if layer_name == "landuse"
            && let Some(kind) = managed_ground_kind(landuse_class)
        {
            pack.ground
                .push((kind, projection.paths_to_en(&feature.paths)));
        } else if layer_name == "landuse"
            && let Some(kind) = ground_kind(source_kind)
        {
            pack.ground
                .push((kind, projection.paths_to_en(&feature.paths)));
        } else {
            report.rejected(
                SemanticLayerFamily::from_name(layer_name),
                SemanticDiscardReason::UnsupportedSemantic,
            );
            continue;
        }
        report.consumed(SemanticLayerFamily::from_name(layer_name));
    }
}

fn enforce_regional_landcover_budget(
    pack: &mut FeaturePack,
    regional_start: usize,
    report: &mut SemanticConsumerReport,
    max_projected_bytes: usize,
) -> bool {
    if pack.regional_landcover_projected_bytes() <= max_projected_bytes {
        return false;
    }
    pack.regional_landcover.truncate(regional_start);
    pack.complete = false;
    report.discard_consumed(
        SemanticLayerFamily::Landcover,
        SemanticDiscardReason::ConsumerBudget,
    );
    true
}

fn enforce_watercourse_budget(
    pack: &mut FeaturePack,
    watercourse_start: usize,
    report: &mut SemanticConsumerReport,
    max_projected_bytes: usize,
) -> bool {
    if pack.watercourse_projected_bytes() <= max_projected_bytes {
        return false;
    }
    for watercourse in &pack.watercourses[watercourse_start..] {
        report.discard_one_consumed(
            SemanticLayerFamily::Water,
            SemanticDiscardReason::ConsumerBudget,
        );
        report.reclassify_water_outcome(
            watercourse.kind.semantic_kind(),
            crate::protomaps_semantics::WaterGeometryRole::Watercourse,
            WaterSemanticOutcome::Consumed,
            WaterSemanticOutcome::Filtered,
        );
    }
    pack.watercourses.truncate(watercourse_start);
    pack.complete = false;
    true
}

/// Apply the terrain-oriented extraction contract to one decoded
/// source tile. Keeping this boundary shared with the semantic baseline tests
/// makes the regression report exercise production classification instead of
/// a copy.
#[cfg(test)]
fn append_legacy_tile_features(
    pack: &mut FeaturePack,
    tile: &mvt::VectorTile,
    z: u8,
    tx: u32,
    ty: u32,
    lat0: f64,
    lon0: f64,
) {
    let proj = |extent: u32| TileProj {
        z,
        x: tx,
        y: ty,
        extent: extent as f64,
        lat0,
        lon0,
    };

    let building_start = pack.buildings.len();
    append_buildings(pack, tile, (z, tx, ty), (lat0, lon0), None);
    if pack.buildings[building_start..]
        .iter()
        .any(may_anchor_observation_tower)
    {
        append_observation_hints(pack, tile, z, tx, ty, lat0, lon0);
    }

    if let Some(layer) = tile.layer("roads") {
        let p = proj(layer.extent);
        for f in &layer.features {
            if f.geom_type != GeomType::Line {
                continue;
            }
            let is_bridge = matches!(f.get(layer, "is_bridge"), Some(Value::Bool(true)));
            let Some(class) = f
                .get(layer, "kind")
                .and_then(Value::as_str)
                .and_then(|kind| {
                    road_class_for_feature(
                        kind,
                        f.get(layer, "kind_detail").and_then(Value::as_str),
                        is_bridge,
                    )
                })
            else {
                continue;
            };
            // Subterranean roads do not contribute surface geometry.
            if matches!(f.get(layer, "is_tunnel"), Some(Value::Bool(true))) {
                continue;
            }
            for path in &f.paths {
                if path.len() >= 2 {
                    let (geometry, location_rad) = p.road_geometry_with_location(path);
                    pack.roads.push(Road {
                        profile: RoadProfile {
                            detail: RoadDetail::from_protomaps(
                                f.get(layer, "kind_detail").and_then(Value::as_str),
                                matches!(f.get(layer, "is_link"), Some(Value::Bool(true))),
                            ),
                            seed: crate::road_profile::name_seed(
                                f.get(layer, "name").and_then(Value::as_str),
                            ),
                            location_rad,
                        },
                        pts: geometry,
                        class,
                        is_bridge,
                    });
                }
            }
        }
    }

    if let Some(layer) = tile.layer("water") {
        let p = proj(layer.extent);
        for f in &layer.features {
            if f.geom_type == GeomType::Polygon && !f.paths.is_empty() {
                let is_ocean =
                    matches!(f.get(layer, "kind").and_then(Value::as_str), Some("ocean"));
                for component in polygon_components(&f.paths) {
                    pack.water.push(Water {
                        rings: p.paths_to_en(&component.paths),
                        kind: if is_ocean {
                            WaterRegionKind::Ocean
                        } else {
                            WaterRegionKind::InlandWater
                        },
                        identity: f.id.map_or(
                            SourceFeatureIdentity::DeterministicFallback(0),
                            SourceFeatureIdentity::Provided,
                        ),
                        intermittent: false,
                        alkaline: false,
                    });
                }
            }
        }
    }

    for layer_name in ["landuse", "landcover"] {
        append_land_regions(pack, tile, layer_name, z, tx, ty, lat0, lon0);
    }
}

fn watercourse_style_identity(kind: WatercourseKind, identity: SourceFeatureIdentity) -> u64 {
    match identity {
        SourceFeatureIdentity::Provided(id) => id ^ kind.style_tag(),
        SourceFeatureIdentity::DeterministicFallback(_) => kind.style_tag(),
    }
}

fn source_text_flag_active(
    tile: &SemanticTile,
    layer: &SemanticLayer,
    fact: Option<SourceFact>,
) -> bool {
    fact.and_then(|fact| tile.source_text(layer, fact))
        .is_some_and(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "no" | "false" | "0"
            )
        })
}

/// Apply the watercourse admission contract to accepted line records. Provided ids are
/// true centerline identity: retain every clipped fragment at the highest
/// observed source zoom and collapse exact same-tile duplicates. Id-less
/// fragments remain separate because joining them by a class-global fallback
/// would connect unrelated channels.
fn reconcile_watercourse_fragments(watercourses: &mut Vec<Watercourse>) {
    watercourses.sort_by_key(|watercourse| {
        let (identity_kind, identity) = match watercourse.identity {
            SourceFeatureIdentity::Provided(id) => (0u8, id),
            SourceFeatureIdentity::DeterministicFallback(id) => (1u8, id),
        };
        (
            watercourse.kind,
            identity_kind,
            identity,
            watercourse.source_tile,
            watercourse.source_feature_index,
        )
    });
    let mut reconciled: Vec<Watercourse> = Vec::with_capacity(watercourses.len());
    let mut provided: HashMap<(WatercourseKind, u64), usize> = HashMap::new();
    let mut seen_fragments: HashSet<(WatercourseKind, u64, TileKey, usize)> = HashSet::new();
    for mut incoming in watercourses.drain(..) {
        let SourceFeatureIdentity::Provided(id) = incoming.identity else {
            reconciled.push(incoming);
            continue;
        };
        if !seen_fragments.insert((
            incoming.kind,
            id,
            incoming.source_tile,
            incoming.source_feature_index,
        )) {
            continue;
        }
        let key = (incoming.kind, id);
        let Some(&index) = provided.get(&key) else {
            provided.insert(key, reconciled.len());
            reconciled.push(incoming);
            continue;
        };
        let existing = &mut reconciled[index];
        match incoming.source_tile.0.cmp(&existing.source_tile.0) {
            std::cmp::Ordering::Greater => *existing = incoming,
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => {
                if incoming.source_tile == existing.source_tile
                    && incoming.source_feature_index == existing.source_feature_index
                {
                    continue;
                }
                existing.paths.append(&mut incoming.paths);
                existing.fragment_count += incoming.fragment_count;
                if (incoming.source_tile, incoming.source_feature_index)
                    < (existing.source_tile, existing.source_feature_index)
                {
                    existing.source_tile = incoming.source_tile;
                    existing.source_feature_index = incoming.source_feature_index;
                }
                existing.intermittent |= incoming.intermittent;
                existing.is_bridge |= incoming.is_bridge;
                existing.source_min_zoom =
                    match (existing.source_min_zoom, incoming.source_min_zoom) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (value @ Some(_), None) | (None, value @ Some(_)) => value,
                        (None, None) => None,
                    };
            }
        }
    }
    reconciled.sort_by_key(|watercourse| {
        let (identity_kind, identity) = match watercourse.identity {
            SourceFeatureIdentity::Provided(id) => (0u8, id),
            SourceFeatureIdentity::DeterministicFallback(id) => (1u8, id),
        };
        (
            watercourse.kind,
            identity_kind,
            identity,
            watercourse.source_tile,
            watercourse.source_feature_index,
        )
    });
    *watercourses = reconciled;
}

fn append_semantic_roads(
    roads: &mut Vec<Road>,
    tile: &SemanticTile,
    tile_key: TileKey,
    origin: (f64, f64),
    report: &mut SemanticConsumerReport,
) {
    if let Some(layer) = tile.semantic_layer("roads") {
        let p = TileProj {
            z: tile_key.0,
            x: tile_key.1,
            y: tile_key.2,
            extent: f64::from(layer.extent),
            lat0: origin.0,
            lon0: origin.1,
        };
        for semantic in &layer.features {
            let LayerFacts::Roads(facts) = &semantic.facts else {
                report.rejected(
                    SemanticLayerFamily::Roads,
                    SemanticDiscardReason::InvalidValue,
                );
                continue;
            };
            let Some(feature) = tile.source_feature(layer, semantic) else {
                report.rejected(
                    SemanticLayerFamily::Roads,
                    SemanticDiscardReason::InvalidValue,
                );
                continue;
            };
            if semantic.geometry_type != GeomType::Line {
                report.rejected(
                    SemanticLayerFamily::Roads,
                    SemanticDiscardReason::UnsupportedGeometry,
                );
                continue;
            }
            let raw_true = |fact: Option<SourceFact>| {
                fact.and_then(|fact| tile.source_value(layer, fact.source))
                    == Some(&Value::Bool(true))
            };
            let is_bridge = raw_true(facts.physical.is_bridge);
            let Some(class) = facts.physical.kind.and_then(|fact| {
                tile.source_text(layer, fact).and_then(|kind| {
                    road_class_for_feature(
                        kind,
                        facts
                            .physical
                            .kind_detail
                            .and_then(|detail| tile.source_text(layer, detail)),
                        is_bridge,
                    )
                })
            }) else {
                let reason = if facts.physical.kind.is_some_and(|fact| {
                    fact.normalized == crate::protomaps_semantics::NormalizedValue::Invalid
                }) {
                    SemanticDiscardReason::InvalidValue
                } else {
                    SemanticDiscardReason::UnsupportedSemantic
                };
                report.rejected(SemanticLayerFamily::Roads, reason);
                continue;
            };
            if raw_true(facts.physical.is_tunnel) {
                report.rejected(
                    SemanticLayerFamily::Roads,
                    SemanticDiscardReason::UnsupportedSemantic,
                );
                continue;
            }
            let mut emitted = false;
            for path in &feature.paths {
                if path.len() >= 2 {
                    let (geometry, location_rad) = p.road_geometry_with_location(path);
                    roads.push(Road {
                        profile: RoadProfile {
                            detail: RoadDetail::from_protomaps(
                                facts
                                    .physical
                                    .kind_detail
                                    .and_then(|fact| tile.source_text(layer, fact)),
                                raw_true(facts.physical.is_link),
                            ),
                            seed: crate::road_profile::name_seed(
                                tile.source_layer(layer)
                                    .and_then(|source| feature.get(source, "name"))
                                    .and_then(Value::as_str),
                            ),
                            location_rad,
                        },
                        pts: geometry,
                        class,
                        is_bridge,
                    });
                    emitted = true;
                }
            }
            if emitted {
                report.consumed(SemanticLayerFamily::Roads);
            } else {
                report.rejected(
                    SemanticLayerFamily::Roads,
                    SemanticDiscardReason::UnsupportedGeometry,
                );
            }
        }
    }
}

/// Select the maximum observed authored zoom for each true source identity
/// while retaining every winning fragment and its path provenance. Id-less
/// records remain independent; their fallback hash is styling continuity,
/// not evidence that two clipped records form one channel.
fn append_current_tile_features(
    pack: &mut FeaturePack,
    tile: &SemanticTile,
    tile_key: TileKey,
    watercourse_authority_zoom: u8,
    origin: (f64, f64),
    report: &mut SemanticConsumerReport,
) {
    let (z, tx, ty) = tile_key;
    let (lat0, lon0) = origin;
    let proj = |extent: u32| TileProj {
        z,
        x: tx,
        y: ty,
        extent: extent as f64,
        lat0,
        lon0,
    };

    let building_start = pack.buildings.len();
    append_semantic_buildings(pack, tile, (z, tx, ty), (lat0, lon0), None, report);
    let observation_enabled = pack.buildings[building_start..]
        .iter()
        .any(may_anchor_observation_tower);
    append_semantic_poi_features(
        pack,
        tile,
        (z, tx, ty),
        (lat0, lon0),
        observation_enabled,
        report,
    );
    append_semantic_cliffs(pack, tile, tile_key, origin, report);

    append_semantic_roads(&mut pack.roads, tile, tile_key, origin, report);

    if let Some(layer) = tile.semantic_layer("water") {
        let p = proj(layer.extent);
        for semantic in &layer.features {
            let LayerFacts::Water(facts) = &semantic.facts else {
                report.rejected(
                    SemanticLayerFamily::Water,
                    SemanticDiscardReason::InvalidValue,
                );
                continue;
            };
            let Some(feature) = tile.source_feature(layer, semantic) else {
                report.rejected(
                    SemanticLayerFamily::Water,
                    SemanticDiscardReason::InvalidValue,
                );
                continue;
            };
            let water_kind = facts.class.unwrap_or(WaterKind::Unknown);
            // OSM way 485600756 is the closed ditch centerline around Nezu's
            // gatehouse, without area=yes. Build 20260913 exports its interior
            // as a polygon. Preserve the perimeter, not a flooded gatehouse.
            // See vectors/fixtures/nezu-ditch.json for source geometry/evidence.
            let closed_ditch_centerline = semantic.geometry_type == GeomType::Polygon
                && water_kind == WaterKind::Ditch
                && semantic.identity == SourceFeatureIdentity::Provided(4_856_007_562);
            if semantic.geometry_type == GeomType::Polygon
                && !closed_ditch_centerline
                && !feature.paths.is_empty()
            {
                let reservoir = facts
                    .physical
                    .reservoir
                    .and_then(|fact| tile.source_boolean(layer, fact))
                    .unwrap_or(false);
                let intermittent = facts
                    .physical
                    .intermittent
                    .and_then(|fact| tile.source_boolean(layer, fact))
                    .unwrap_or(false);
                let alkaline = facts
                    .physical
                    .alkaline
                    .and_then(|fact| tile.source_boolean(layer, fact))
                    .unwrap_or(false);
                let kind = WaterRegionKind::from_semantic(facts, reservoir);
                if pack.water_polygon_mode == WaterPolygonMode::Typed
                    && !kind.creates_surface_water()
                {
                    report.discarded(
                        SemanticLayerFamily::Water,
                        SemanticDiscardReason::DeferredPhysical,
                    );
                    report.water_outcome(
                        water_kind,
                        facts.geometry_role,
                        WaterSemanticOutcome::Filtered,
                    );
                    continue;
                }
                // Unrelated exteriors must not add slab cuts to one another
                // during water/bed tessellation. Group in exact tile space;
                // touching or overlapping rings retain their compound parity.
                for component in tile
                    .source_polygon_components(layer, semantic)
                    .into_iter()
                    .flatten()
                {
                    pack.water.push(Water {
                        rings: p.paths_to_en(&component.paths),
                        kind,
                        identity: semantic.identity,
                        intermittent,
                        alkaline,
                    });
                }
                report.consumed(SemanticLayerFamily::Water);
                report.water_outcome(
                    water_kind,
                    facts.geometry_role,
                    WaterSemanticOutcome::Consumed,
                );
            } else if (semantic.geometry_type == GeomType::Line || closed_ditch_centerline)
                && facts.surface_disposition == WaterSurfaceDisposition::Candidate
            {
                let Some(kind) = WatercourseKind::from_semantic(facts) else {
                    report.discarded(
                        SemanticLayerFamily::Water,
                        SemanticDiscardReason::DeferredPhysical,
                    );
                    report.water_outcome(
                        water_kind,
                        facts.geometry_role,
                        WaterSemanticOutcome::Filtered,
                    );
                    continue;
                };
                let mut paths = p.paths_to_en(&feature.paths);
                paths.retain(|path| path.len() >= 2);
                if closed_ditch_centerline {
                    // MVT ClosePath is implicit in polygon paths. Centerlines
                    // require the closing segment to remain explicit.
                    for path in &mut paths {
                        if path.first() != path.last() {
                            path.push(path[0]);
                        }
                    }
                }
                if paths.is_empty() {
                    report.rejected(
                        SemanticLayerFamily::Water,
                        SemanticDiscardReason::UnsupportedGeometry,
                    );
                    continue;
                }
                let intermittent = facts
                    .physical
                    .intermittent
                    .and_then(|fact| tile.source_boolean(layer, fact))
                    .unwrap_or(false);
                pack.watercourses.push(Watercourse {
                    paths,
                    kind,
                    width_m: kind.fallback_width_m(),
                    identity: semantic.identity,
                    style_identity: watercourse_style_identity(kind, semantic.identity),
                    source_tile: (z, tx, ty),
                    authority_zoom: watercourse_authority_zoom,
                    source_feature_index: semantic.source_feature_index,
                    source_min_zoom: semantic
                        .common
                        .min_zoom
                        .and_then(|fact| tile.source_number(layer, fact))
                        .filter(|value| value.is_finite())
                        .map(|value| value.clamp(0.0, 30.0) as f32),
                    fragment_count: 1,
                    intermittent,
                    is_bridge: source_text_flag_active(tile, layer, facts.physical.bridge),
                });
                report.consumed(SemanticLayerFamily::Water);
                report.water_outcome(
                    water_kind,
                    crate::protomaps_semantics::WaterGeometryRole::Watercourse,
                    WaterSemanticOutcome::Consumed,
                );
            } else if facts.surface_disposition == WaterSurfaceDisposition::ContextOnly {
                report.discarded(
                    SemanticLayerFamily::Water,
                    SemanticDiscardReason::ContextOnly,
                );
                report.water_outcome(
                    water_kind,
                    facts.geometry_role,
                    WaterSemanticOutcome::ContextOnly,
                );
            } else if facts.surface_disposition == WaterSurfaceDisposition::SurfaceIneligible {
                report.discarded(
                    SemanticLayerFamily::Water,
                    SemanticDiscardReason::SurfaceIneligible,
                );
                report.water_outcome(
                    water_kind,
                    facts.geometry_role,
                    WaterSemanticOutcome::Filtered,
                );
            } else {
                report.rejected(
                    SemanticLayerFamily::Water,
                    SemanticDiscardReason::UnsupportedGeometry,
                );
            }
        }
    }

    append_semantic_land_regions(pack, tile, "landuse", z, tx, ty, lat0, lon0, report);
}

impl VectorSource for ProtomapsSource {
    fn features(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.regional_landcover_mode = self.regional_landcover_mode;
        pack.water_polygon_mode = self.water_polygon_mode;
        pack.watercourse_output_mode = self.watercourse_output_mode;
        if lat0.abs() > 1.48 {
            return pack; // beyond Web Mercator; no OSM tile coverage
        }
        let plan = self.terrain_query_plan(lat0, lon0, half_e_m, half_n_m);
        let watercourse_authority_zoom = self
            .descriptor
            .layers
            .get("water")
            .map_or(self.archive.header.max_zoom, |layer| layer.max_zoom)
            .min(self.archive.header.max_zoom);
        pack.complete = plan.required_complete();
        for query in &plan.queries {
            if query.capped {
                for layer in query.layers {
                    self.semantic_telemetry
                        .record_query_cap(SemanticLayerFamily::from_name(layer));
                }
            }
        }
        // Warm covering tiles that nobody has yet, fetches in flight
        // concurrently; the decode loop below then reads from memory.
        if !self.is_async_prepared() {
            let missing = self.uncached_keys(&plan.unique_tile_keys());
            if missing.len() > 1 {
                vearth_platform::thread::scope(|scope| {
                    for &(z, x, y) in &missing {
                        scope.spawn(move || {
                            let _ = self.decoded_tile(z, x, y);
                        });
                    }
                });
            }
        }
        for query in &plan.queries {
            let regional_start = pack.regional_landcover.len();
            let watercourse_start = pack.watercourses.len();
            let mut regional_report = self.semantic_telemetry.consumer_report();
            let mut detail_report = self.semantic_telemetry.consumer_report();
            for &(z, tx, ty) in &query.tiles {
                let holder = match self.decoded_tile(z, tx, ty) {
                    Ok(h) => h,
                    Err(e) => {
                        log::warn!("vector tile fetch {z}/{tx}/{ty}: {e}");
                        if query.completeness == QueryCompleteness::Required {
                            pack.complete = false;
                        }
                        continue;
                    }
                };
                let Some(tile) = holder.as_ref() else {
                    continue;
                };
                match query.kind {
                    VectorQueryKind::TerrainDetail => {
                        append_current_tile_features(
                            &mut pack,
                            tile,
                            (z, tx, ty),
                            watercourse_authority_zoom,
                            (lat0, lon0),
                            &mut detail_report,
                        );
                    }
                    VectorQueryKind::TerrainLandcover => {
                        append_semantic_land_regions(
                            &mut pack,
                            tile,
                            "landcover",
                            z,
                            tx,
                            ty,
                            lat0,
                            lon0,
                            &mut regional_report,
                        );
                    }
                    VectorQueryKind::NightBuildings | VectorQueryKind::LocationContext => {
                        unreachable!("separate consumer plan")
                    }
                }
            }
            if query.kind == VectorQueryKind::TerrainLandcover {
                enforce_regional_landcover_budget(
                    &mut pack,
                    regional_start,
                    &mut regional_report,
                    MAX_REGIONAL_LANDCOVER_PROJECTED_BYTES,
                );
                self.semantic_telemetry.record_consumer(&regional_report);
            } else if query.kind == VectorQueryKind::TerrainDetail {
                finalize_exact_trees(&mut pack, half_e_m, half_n_m, &mut detail_report);
                finalize_cliffs(&mut pack, half_e_m, half_n_m, &mut detail_report);
                if !enforce_watercourse_budget(
                    &mut pack,
                    watercourse_start,
                    &mut detail_report,
                    MAX_WATERCOURSE_PROJECTED_BYTES,
                ) {
                    reconcile_watercourse_fragments(&mut pack.watercourses);
                }
                self.semantic_telemetry.record_consumer(&detail_report);
            }
        }
        if let Some(catalog) = &self.bridge_catalog {
            pack.bridges = catalog.query(lat0, lon0, half_e_m, half_n_m);
            // Raw OSM inputs can cover only selected regions. Outside those
            // places retain the legacy bridge way instead of suppressing it
            // globally.
            pack.bridge_authority = !pack.bridges.is_empty();
        }
        pack
    }

    fn night_buildings(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) -> FeaturePack {
        let mut pack = FeaturePack::empty();
        if lat0.abs() > 1.48 {
            return pack;
        }
        let plan = self.night_building_query_plan(lat0, lon0, half_e_m, half_n_m);
        pack.complete = plan.required_complete();
        let query = plan
            .query(VectorQueryKind::NightBuildings)
            .expect("valid bounds produce night-building query");
        if query.capped {
            for layer in query.layers {
                self.semantic_telemetry
                    .record_query_cap(SemanticLayerFamily::from_name(layer));
            }
        }
        if !self.is_async_prepared() {
            let missing = self.uncached_keys(&query.tiles);
            if missing.len() > 1 {
                vearth_platform::thread::scope(|scope| {
                    for &(z, tx, ty) in &missing {
                        scope.spawn(move || {
                            let _ = self.decoded_tile(z, tx, ty);
                        });
                    }
                });
            }
        }
        for &(z, tx, ty) in &query.tiles {
            let holder = match self.decoded_tile(z, tx, ty) {
                Ok(holder) => holder,
                Err(error) => {
                    log::warn!("night-building tile fetch {z}/{tx}/{ty}: {error}");
                    pack.complete = false;
                    continue;
                }
            };
            if let Some(tile) = holder.as_ref() {
                let mut report = self.semantic_telemetry.consumer_report();
                append_semantic_buildings(
                    &mut pack,
                    tile,
                    (z, tx, ty),
                    (lat0, lon0),
                    Some(NIGHT_BUILDING_MIN_HEIGHT_M),
                    &mut report,
                );
                self.semantic_telemetry.record_consumer(&report);
            }
        }
        pack
    }

    /// Concurrent warm-up of every tile a `features` call for this window
    /// will read; runs alongside the elevation prefetch in gen workers.
    /// Cached or already-in-flight tiles spawn nothing.
    fn prefetch_vectors(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) {
        if lat0.abs() > 1.48 {
            return;
        }
        if self.is_async_prepared() {
            return;
        }
        let plan = self.terrain_query_plan(lat0, lon0, half_e_m, half_n_m);
        self.warm_vector_plan(&plan);
    }

    fn prefetch_night_buildings(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) {
        if lat0.abs() > 1.48 || self.is_async_prepared() {
            return;
        }
        let plan = self.night_building_query_plan(lat0, lon0, half_e_m, half_n_m);
        let missing = self.uncached_keys(&plan.unique_tile_keys());
        match missing.len() {
            0 => {}
            1 => {
                let _ = self.decoded_tile(missing[0].0, missing[0].1, missing[0].2);
            }
            _ => vearth_platform::thread::scope(|scope| {
                for &(z, x, y) in &missing {
                    scope.spawn(move || {
                        let _ = self.decoded_tile(z, x, y);
                    });
                }
            }),
        }
    }

    fn attribution(&self) -> String {
        format!(
            "© OpenStreetMap contributors (ODbL) via Protomaps build {}",
            self.build_tag
        )
    }
}

/// Where the site hosts Protomaps builds. Build `<tag>` is stored as
/// [`HOSTED_BUILD_BLOCK_BYTES`] objects at `<PROTOMAPS_BASE>/<tag>/<index>.bin`
/// (tools/copy-protomaps-build.py; see web/README.md). Small objects are
/// cached by the CDN, which a whole planet archive is too large for.
pub const PROTOMAPS_BASE: &str = "https://maps.lowpolyearth.online/protomaps";

/// The build both apps read. A Mac release keeps reading the build it shipped
/// with, so a build must stay hosted while releases that read it are in use.
/// Changing it re-keys the vector-tile and generated-chunk caches.
pub const PROTOMAPS_BUILD: &str = "20260930";

/// Bytes per object of a hosted build; the last object is shorter.
pub const HOSTED_BUILD_BLOCK_BYTES: u64 = 1024 * 1024;

/// Archive URL of a hosted build: the prefix of its block objects.
pub fn protomaps_build_url(tag: &str) -> String {
    format!("{PROTOMAPS_BASE}/{tag}")
}

/// Whether `url` is a hosted build, read block object by block object.
pub(crate) fn is_block_archive(url: &str) -> bool {
    url.strip_prefix(PROTOMAPS_BASE)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// URL of block `index` of the hosted build at `url`.
pub(crate) fn block_url(url: &str, index: u64) -> String {
    format!("{url}/{index}.bin")
}

/// The blocks holding bytes `offset..=end` of a hosted build, each with the
/// part of that block the read covers.
pub(crate) fn block_spans(
    offset: u64,
    end: u64,
) -> impl Iterator<Item = (u64, std::ops::Range<usize>)> {
    let size = HOSTED_BUILD_BLOCK_BYTES;
    (offset / size..=end / size).map(move |index| {
        let start = index * size;
        let from = offset.max(start) - start;
        let to = (end + 1).min(start + size) - start;
        (index, from as usize..to as usize)
    })
}

#[cfg(test)]
mod hydrography_baseline;
#[cfg(test)]
mod land_expansion_baseline;
#[cfg(test)]
mod semantic_baseline;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "async-http")]
    use crate::async_http::{AsyncRangeFetcher, FetchFuture};
    #[cfg(feature = "async-http")]
    use crate::async_prep::{AsyncSourcePreparer, PrepareRequest, SourceFootprint};
    #[cfg(feature = "async-http")]
    use crate::pmtiles::RangeFetcher;
    #[cfg(feature = "async-http")]
    use std::future::Future as _;
    #[cfg(feature = "async-http")]
    use std::path::Path;
    #[cfg(feature = "async-http")]
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    #[ignore = "bounded read-only audit; set VEARTH_COMPONENT_SOURCE_CACHE and VEARTH_COMPONENT_REPORT"]
    fn inspect_cached_building_components() {
        use std::io::Read;
        let path = vearth_platform::env::var("VEARTH_COMPONENT_SOURCE_CACHE").unwrap();
        let output = vearth_platform::env::var("VEARTH_COMPONENT_REPORT").unwrap();
        let db = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let namespace: i64 = db.query_row("SELECT id FROM source_namespaces WHERE provider='vectors' ORDER BY id DESC LIMIT 1", [], |r|r.get(0)).unwrap();
        let mut report = Vec::new();
        for zoom in 11_u8..=15 {
            let n = f64::from(1_u32 << zoom);
            let xf = (4.895168 + 180.0) / 360.0 * n;
            let yf =
                (1.0 - 52.370216_f64.to_radians().tan().asinh() / std::f64::consts::PI) * 0.5 * n;
            let mut query = db.prepare("SELECT x,y,payload FROM source_tiles WHERE namespace_id=?1 AND z=?2 AND x BETWEEN ?3 AND ?4 AND y BETWEEN ?5 AND ?6 AND state=1 ORDER BY ((x+0.5-?7)*(x+0.5-?7)+(y+0.5-?8)*(y+0.5-?8)) LIMIT 4").unwrap();
            let rows: Vec<_> = query
                .query_map(
                    rusqlite::params![
                        namespace,
                        zoom,
                        xf as i64 - 2,
                        xf as i64 + 2,
                        yf as i64 - 2,
                        yf as i64 + 2,
                        xf,
                        yf
                    ],
                    |r| {
                        Ok((
                            r.get::<_, u32>(0)?,
                            r.get::<_, u32>(1)?,
                            r.get::<_, Vec<u8>>(2)?,
                        ))
                    },
                )
                .unwrap()
                .map(Result::unwrap)
                .collect();
            for (x, y, payload) in rows {
                let raw = if payload.starts_with(&[0x1f, 0x8b]) {
                    let mut raw = Vec::new();
                    flate2::read::GzDecoder::new(&payload[..])
                        .read_to_end(&mut raw)
                        .unwrap();
                    raw
                } else {
                    payload
                };
                let tile = mvt::decode_tile(&raw).unwrap();
                let mut count = 0;
                let mut components = 0;
                let mut oversized = 0;
                let mut unsplit_oversized = 0;
                let mut grouping_ns = 0_u64;
                let mut oversized_groups = 0;
                let mut largest_component = 0;
                let mut split_sources = 0;
                let mut largest_source = 0;
                if let Some(layer) = tile.layer("buildings") {
                    for feature in &layer.features {
                        if feature.geom_type != GeomType::Polygon {
                            continue;
                        }
                        count += 1;
                        let edge_count = |paths: &[Vec<[i32; 2]>]| {
                            paths
                                .iter()
                                .map(|ring| {
                                    ring.iter()
                                        .zip(ring.iter().cycle().skip(1))
                                        .filter(|(a, b)| a != b)
                                        .count()
                                })
                                .sum::<usize>()
                        };
                        let source_edges = edge_count(&feature.paths);
                        let start = vearth_platform::time::Instant::now();
                        let parts: Vec<_> = polygon_components(&feature.paths)
                            .map(|c| c.paths)
                            .collect();
                        grouping_ns += start.elapsed().as_nanos() as u64;
                        components += parts.len();
                        oversized_groups +=
                            parts.iter().filter(|part| edge_count(part) > 512).count();
                        split_sources += usize::from(parts.len() > 1);
                        oversized += usize::from(source_edges > 512);
                        unsplit_oversized += usize::from(source_edges > 512 && parts.len() == 1);
                        largest_source = largest_source.max(source_edges);
                        largest_component = largest_component
                            .max(parts.iter().map(|p| edge_count(p)).max().unwrap_or(0));
                    }
                }
                report.push(serde_json::json!({"tile":[zoom as u32,x,y],"source_polygons":count,"emitted_components":components,"split_sources":split_sources,"oversized_sources":oversized,"unsplit_oversized_sources":unsplit_oversized,"largest_source_edges":largest_source,"largest_component_edges":largest_component,"oversized_emitted_groups":oversized_groups,"grouping_ns":grouping_ns}));
            }
        }
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        for zoom in 11_u64..=15 {
            let rows: Vec<_> = report.iter().filter(|r| r["tile"][0] == zoom).collect();
            let total = |name: &str| rows.iter().map(|r| r[name].as_u64().unwrap()).sum::<u64>();
            println!(
                "zoom={zoom} sources={} components={} oversized={} unsplit_oversized={}",
                total("source_polygons"),
                total("emitted_components"),
                total("oversized_sources"),
                total("unsplit_oversized_sources")
            );
        }
    }

    #[test]
    #[ignore = "reads two cached source tiles; set VEARTH_COMPONENT_SOURCE_CACHE and VEARTH_WATER_COMPONENT_FIXTURE"]
    fn export_cached_water_components() {
        use std::fmt::Write;
        use std::io::Read;
        let path = vearth_platform::env::var("VEARTH_COMPONENT_SOURCE_CACHE").unwrap();
        let output = vearth_platform::env::var("VEARTH_WATER_COMPONENT_FIXTURE").unwrap();
        let db =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let (namespace, revision): (i64, String) = db.query_row(
            "SELECT id,revision FROM source_namespaces WHERE provider='vectors' ORDER BY id DESC LIMIT 1",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        let origin = (
            52.228464464273564_f64.to_radians(),
            5.098867412036834_f64.to_radians(),
        );
        let mut text = format!(
            "# OpenStreetMap contributors, ODbL; Protomaps build {revision}\n# Cell face4 L12 i3770 j2228; exact TileProj EN source points\n# Origin radians {} {}; generation benchmark supplies controlled datum/ground\n",
            origin.0, origin.1,
        );
        let mut source_index = 0;
        for (z, x, y) in [(12_u8, 2105_u32, 1348_u32), (12, 2106, 1348)] {
            let payload: Vec<u8> = db.query_row(
                "SELECT payload FROM source_tiles WHERE namespace_id=?1 AND z=?2 AND x=?3 AND y=?4 AND state=1",
                rusqlite::params![namespace,z,x,y], |r| r.get(0),
            ).unwrap();
            let bytes = if payload.starts_with(&[0x1f, 0x8b]) {
                let mut bytes = Vec::new();
                flate2::read::GzDecoder::new(&payload[..])
                    .read_to_end(&mut bytes)
                    .unwrap();
                bytes
            } else {
                payload
            };
            let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), (z, x, y));
            let layer = tile.semantic_layer("water").unwrap();
            let projection = TileProj {
                z,
                x,
                y,
                extent: layer.extent as f64,
                lat0: origin.0,
                lon0: origin.1,
            };
            for semantic in &layer.features {
                if semantic.geometry_type != GeomType::Polygon {
                    continue;
                }
                let feature = tile.source_feature(layer, semantic).unwrap();
                if feature.paths.is_empty() {
                    continue;
                }
                let LayerFacts::Water(facts) = &semantic.facts else {
                    unreachable!()
                };
                let reservoir = facts
                    .physical
                    .reservoir
                    .and_then(|f| tile.source_boolean(layer, f))
                    .unwrap_or(false);
                let kind = WaterRegionKind::from_semantic(facts, reservoir);
                if !kind.creates_surface_water() {
                    continue;
                }
                let parts: Vec<_> = polygon_components(&feature.paths).collect();
                let largest = parts
                    .iter()
                    .map(|part| part.paths.iter().map(Vec::len).sum::<usize>())
                    .max()
                    .unwrap_or(0);
                println!(
                    "tile={z}/{x}/{y} id={:?} rings={} edges={} groups={} largest_group_edges={largest}",
                    feature.id,
                    feature.paths.len(),
                    feature.paths.iter().map(Vec::len).sum::<usize>(),
                    parts.len()
                );
                writeln!(
                    text,
                    "# tile {z}/{x}/{y} source {:?}\nsource {source_index} {}\noriginal",
                    feature.id, kind as u8
                )
                .unwrap();
                let mut append = |paths: &[Vec<[i32; 2]>]| {
                    for ring in projection.paths_to_en(paths) {
                        text.push_str("ring\n");
                        for [e, n] in ring {
                            writeln!(text, "{e} {n}").unwrap();
                        }
                    }
                };
                append(&feature.paths);
                for part in parts {
                    text.push_str("component\n");
                    for ring in projection.paths_to_en(&part.paths) {
                        text.push_str("ring\n");
                        for [e, n] in ring {
                            writeln!(text, "{e} {n}").unwrap();
                        }
                    }
                }
                source_index += 1;
            }
        }
        assert!(source_index > 0);
        std::fs::write(output, text).unwrap();
    }

    #[test]
    fn nezu_closed_ditch_preserves_perimeter_without_flooding_gatehouse() {
        use crate::mvt::encode::FeatureSpec;
        use crate::protomaps_semantics::WaterGeometryRole;

        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("vectors/fixtures/nezu-ditch.json")).unwrap();
        let key: TileKey = serde_json::from_value(fixture["tile"].clone()).unwrap();
        let paths: Vec<Vec<[i32; 2]>> = serde_json::from_value(fixture["paths"].clone()).unwrap();
        let id = fixture["id"].as_u64().unwrap();
        let extent = fixture["extent"].as_u64().unwrap() as u32;
        let origin = (35.719662_f64.to_radians(), 139.7609411_f64.to_radians());
        for (source_id, kind, geometry_type, corrected) in [
            (id, "ditch", GeomType::Polygon, true),
            (id + 10, "ditch", GeomType::Polygon, false),
            (id, "lake", GeomType::Polygon, false),
            (id, "ditch", GeomType::Line, false),
        ] {
            let bytes = mvt::encode::tile(
                "water",
                extent,
                &["sort_rank", "kind"],
                &[Value::I64(200), Value::Str(kind.into())],
                &[FeatureSpec {
                    id: Some(source_id),
                    geom_type: geometry_type,
                    paths: paths.clone(),
                    tags: vec![(0, 0), (1, 1)],
                }],
            );
            let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), key);
            let mut pack = FeaturePack::empty();
            pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
            let mut report = SemanticConsumerReport::default();
            append_current_tile_features(&mut pack, &tile, key, key.0, origin, &mut report);
            let layer = tile.semantic_layer("water").unwrap();
            assert_eq!(
                tile.source_feature(layer, &layer.features[0])
                    .unwrap()
                    .paths,
                paths,
                "raw source geometry remains lossless"
            );
            if corrected || geometry_type == GeomType::Line {
                assert!(pack.water.is_empty());
                assert_eq!(pack.watercourses.len(), 1);
                let watercourse = &pack.watercourses[0];
                assert_eq!(watercourse.kind, WatercourseKind::Ditch);
                assert_eq!(watercourse.width_m, 1.0);
                assert_eq!(watercourse.identity, SourceFeatureIdentity::Provided(id));
                assert_eq!(
                    watercourse.style_identity,
                    watercourse_style_identity(WatercourseKind::Ditch, watercourse.identity)
                );
                assert_eq!(watercourse.source_tile, key);
                assert_eq!(watercourse.authority_zoom, key.0);
                let projection = TileProj {
                    z: key.0,
                    x: key.1,
                    y: key.2,
                    extent: f64::from(extent),
                    lat0: origin.0,
                    lon0: origin.1,
                };
                let mut expected = projection.paths_to_en(&paths);
                if corrected {
                    let first = expected[0][0];
                    expected[0].push(first);
                    assert_eq!(watercourse.paths[0].first(), watercourse.paths[0].last());
                    for segment in watercourse.paths[0].windows(2) {
                        let [a, b] = [segment[0], segment[1]];
                        let d = [b[0] - a[0], b[1] - a[1]];
                        let t = (-(a[0] * d[0] + a[1] * d[1]) / (d[0] * d[0] + d[1] * d[1]))
                            .clamp(0.0, 1.0);
                        let distance = (a[0] + t * d[0]).hypot(a[1] + t * d[1]);
                        assert!(distance > 5.0, "the gate center is outside the drain");
                    }
                }
                assert_eq!(watercourse.paths, expected);
                assert!(enforce_watercourse_budget(&mut pack, 0, &mut report, 0));
                let telemetry = SemanticTelemetry::new_enabled("nezu-ditch", 0);
                telemetry.record_consumer(&report);
                assert_eq!(
                    telemetry.snapshot().water_outcome_count(
                        WaterKind::Ditch,
                        WaterGeometryRole::Watercourse,
                        WaterSemanticOutcome::Filtered,
                    ),
                    1
                );
            } else {
                assert!(pack.watercourses.is_empty());
                assert_eq!(pack.water.len(), 1);
                assert_eq!(
                    pack.water[0].kind,
                    if kind == "lake" {
                        WaterRegionKind::Lake
                    } else {
                        WaterRegionKind::Drainage
                    }
                );
            }
        }
    }

    #[test]
    fn water_components_preserve_projected_rings_metadata_and_per_feature_diagnostics() {
        use crate::mvt::encode::FeatureSpec;
        let mut hole = component_square(120, 120, 40);
        hole.reverse();
        let paths = vec![
            component_square(100, 100, 100),
            hole,
            component_square(130, 130, 10),
            component_square(300, 100, 100),
        ];
        let key = (12, 2105, 1348);
        let origin = tile_center(key.0, key.1, key.2);
        for provided_id in [None, Some(19)] {
            for kind in ["ocean", "water"] {
                let bytes = mvt::encode::tile(
                    "water",
                    4096,
                    &["kind", "intermittent", "alkaline"],
                    &[Value::Str(kind.into()), Value::Bool(true)],
                    &[FeatureSpec {
                        id: provided_id,
                        geom_type: GeomType::Polygon,
                        paths: paths.clone(),
                        tags: vec![(0, 0), (1, 1), (2, 1)],
                    }],
                );
                let raw_tile = mvt::decode_tile(&bytes).unwrap();
                let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), key);
                let layer = tile.semantic_layer("water").unwrap();
                let identity = layer.features[0].identity;
                let projection = TileProj {
                    z: key.0,
                    x: key.1,
                    y: key.2,
                    extent: 4096.0,
                    lat0: origin.0,
                    lon0: origin.1,
                };
                let expected: Vec<_> = [&paths[..2], &paths[2..3], &paths[3..]]
                    .map(|rings| projection.paths_to_en(rings))
                    .into_iter()
                    .collect();
                let mut legacy = FeaturePack::empty();
                append_legacy_tile_features(
                    &mut legacy,
                    &raw_tile,
                    key.0,
                    key.1,
                    key.2,
                    origin.0,
                    origin.1,
                );
                let mut typed = FeaturePack::empty();
                let mut report = SemanticConsumerReport::default();
                append_current_tile_features(&mut typed, &tile, key, 15, origin, &mut report);
                assert_eq!(
                    legacy.water.iter().map(|w| &w.rings).collect::<Vec<_>>(),
                    expected.iter().collect::<Vec<_>>()
                );
                assert_eq!(
                    typed.water.iter().map(|w| &w.rings).collect::<Vec<_>>(),
                    expected.iter().collect::<Vec<_>>()
                );
                assert!(typed.complete && legacy.complete);
                for water in &typed.water {
                    assert_eq!(water.identity, identity);
                    assert_eq!(water.is_ocean(), kind == "ocean");
                    assert!(water.intermittent && water.alkaline);
                }
                assert_eq!(
                    tile.source_feature(layer, &layer.features[0])
                        .unwrap()
                        .paths,
                    paths
                );
                let telemetry = SemanticTelemetry::new_enabled("water-components-test", 0);
                telemetry.record_consumer(&report);
                assert_eq!(
                    telemetry
                        .snapshot()
                        .layer_consumed(SemanticLayerFamily::Water),
                    1
                );
            }
        }
    }

    #[test]
    fn water_components_retain_interacting_compounds_and_single_source_parity() {
        use crate::mvt::encode::FeatureSpec;
        let mut hole = component_square(20, 20, 40);
        hole.reverse();
        let cases = [
            vec![component_square(0, 0, 100), hole.clone()],
            vec![
                component_square(0, 0, 100),
                component_square(200, 0, 50),
                component_square(50, 0, 100),
            ],
            vec![hole, component_square(200, 0, 50)],
        ];
        let key = (12, 2105, 1348);
        let origin = tile_center(key.0, key.1, key.2);
        let projection = TileProj {
            z: key.0,
            x: key.1,
            y: key.2,
            extent: 4096.0,
            lat0: origin.0,
            lon0: origin.1,
        };
        for paths in cases {
            let bytes = mvt::encode::tile(
                "water",
                4096,
                &["kind"],
                &[Value::Str("water".into())],
                &[FeatureSpec {
                    id: Some(17),
                    geom_type: GeomType::Polygon,
                    paths: paths.clone(),
                    tags: vec![(0, 0)],
                }],
            );
            let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), key);
            let mut pack = FeaturePack::empty();
            append_current_tile_features(
                &mut pack,
                &tile,
                key,
                15,
                origin,
                &mut SemanticConsumerReport::default(),
            );
            if paths.len() == 3 {
                assert_eq!(pack.water.len(), 2);
                assert_eq!(
                    pack.water[0].rings,
                    projection.paths_to_en(&[paths[0].clone(), paths[2].clone()])
                );
                assert_eq!(pack.water[1].rings, projection.paths_to_en(&paths[1..2]));
            } else {
                assert_eq!(pack.water.len(), 1);
                assert_eq!(pack.water[0].rings, projection.paths_to_en(&paths));
            }
            for y in -1..52 {
                for x in -1..127 {
                    let point = [x * 2 + 1, y * 2 + 1];
                    let Some(expected) = raw_component_contains(&paths, point) else {
                        continue;
                    };
                    let en = projection.to_en(point[0], point[1]);
                    assert_eq!(
                        pack.water.iter().any(|w| point_in_rings(en, &w.rings)),
                        expected
                    );
                }
            }
        }
    }

    fn component_square(x: i32, y: i32, size: i32) -> Vec<[i32; 2]> {
        vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]]
    }

    fn component_fixture(
        paths: &[Vec<[i32; 2]>],
        id: Option<u64>,
        physical: bool,
    ) -> (FeaturePack, FeaturePack) {
        use crate::mvt::encode::FeatureSpec;
        let key = (14, 8414, 5384);
        let keys = ["kind", "height", "min_height", "roof:shape", "kind_detail"];
        let values = [
            Value::Str("building".into()),
            Value::F64(16.5),
            Value::F64(2.0),
            Value::Str("hipped".into()),
            Value::Str("commercial".into()),
        ];
        let tags = if physical {
            vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)]
        } else {
            vec![(0, 0)]
        };
        let bytes = mvt::encode::tile(
            "buildings",
            4096,
            &keys,
            &values,
            &[FeatureSpec {
                id,
                geom_type: GeomType::Polygon,
                paths: paths.to_vec(),
                tags,
            }],
        );
        let direct = mvt::decode_tile(&bytes).unwrap();
        let semantic = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), key);
        let layer = semantic.semantic_layer("buildings").unwrap();
        let identity = layer.features[0].identity;
        let origin = tile_center(key.0, key.1, key.2);
        let mut raw = FeaturePack::empty();
        append_buildings(&mut raw, &direct, key, origin, None);
        let mut typed = FeaturePack::empty();
        let mut report = SemanticConsumerReport::default();
        append_semantic_buildings(&mut typed, &semantic, key, origin, None, &mut report);
        assert_eq!(
            semantic
                .source_feature(layer, &layer.features[0])
                .unwrap()
                .paths,
            paths
        );
        assert_eq!(layer.features[0].identity, identity);
        assert_eq!(raw.buildings.len(), typed.buildings.len());
        assert!(raw.complete && typed.complete);
        for (a, b) in raw.buildings.iter().zip(&typed.buildings) {
            assert_building_equal(a, b);
        }
        (raw, typed)
    }

    fn assert_building_equal(a: &Building, b: &Building) {
        assert_eq!(a.rings, b.rings);
        assert_eq!(a.location_rad, b.location_rad);
        assert_eq!(a.seed, b.seed);
        assert_eq!(a.height_m, b.height_m);
        assert_eq!(a.min_height_m, b.min_height_m);
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.roof_shape, b.roof_shape);
        assert_eq!(a.use_kind, b.use_kind);
    }

    #[test]
    fn building_neighbor_visitors_preserve_projection_metadata_and_source_anchors() {
        use crate::mvt::encode::FeatureSpec;
        let key = (15, 16791, 10834);
        let mut hole = component_square(1650, 3100, 80);
        hole.reverse();
        let paths = vec![
            component_square(1600, 3000, 500),
            hole,
            component_square(2600, 3100, 100),
        ];
        let bytes = mvt::encode::tile(
            "buildings",
            4096,
            &["kind", "height", "roof:shape"],
            &[
                Value::Str("building".into()),
                Value::F64(16.5),
                Value::Str("gabled".into()),
            ],
            &[FeatureSpec {
                id: Some(42),
                geom_type: GeomType::Polygon,
                paths: paths.clone(),
                tags: vec![(0, 0), (1, 1), (2, 2)],
            }],
        );
        let raw = mvt::decode_tile(&bytes).unwrap();
        let tile = ProtomapsSource::decode_tile_bytes(
            &bytes,
            key,
            &std::sync::atomic::AtomicU64::new(0),
            &SemanticTelemetry::new("test", 3),
        )
        .unwrap()
        .unwrap();
        let mut previous: Option<FeaturePack> = None;
        for origin in [(0.903, 0.078), (0.9031, 0.0782), (0.9029, 0.0778)] {
            let mut pack = FeaturePack::empty();
            append_semantic_buildings(
                &mut pack,
                &tile,
                key,
                origin,
                None,
                &mut SemanticConsumerReport::default(),
            );
            let mut legacy = FeaturePack::empty();
            append_buildings(&mut legacy, &raw, key, origin, None);
            assert_eq!(pack.buildings.len(), 2);
            assert_eq!(pack.buildings.len(), legacy.buildings.len());
            assert!(pack.complete && legacy.complete);
            for (index, (actual, expected)) in
                pack.buildings.iter().zip(&legacy.buildings).enumerate()
            {
                assert_building_equal(actual, expected);
                if let Some(previous) = &previous {
                    assert_eq!(actual.location_rad, previous.buildings[index].location_rad);
                    assert_ne!(actual.rings, previous.buildings[index].rings);
                }
            }
            previous = Some(pack);
        }
        assert_eq!(tile.decoded().layers[0].features[0].paths, paths);
    }

    #[test]
    fn building_components_preserve_holes_islands_and_source_geometry() {
        let mut hole = component_square(20, 20, 60);
        hole.reverse();
        let mut second_hole = component_square(220, 20, 60);
        second_hole.reverse();
        let paths = vec![
            component_square(0, 0, 100),
            hole,
            component_square(35, 35, 20),
            component_square(200, 0, 100),
            second_hole,
        ];
        let components: Vec<_> = polygon_components(&paths).map(|c| c.paths).collect();
        assert_eq!(
            components.iter().map(|c| c.as_ref()).collect::<Vec<_>>(),
            vec![&paths[..2], &paths[2..3], &paths[3..]]
        );
        for y in -1..52 {
            for x in -1..153 {
                let point = [x * 2 + 1, y * 2 + 1];
                let Some(original) = raw_component_contains(&paths, point) else {
                    continue;
                };
                let fills: Vec<_> = components
                    .iter()
                    .map(|c| raw_component_contains(c, point).unwrap())
                    .collect();
                assert_eq!(original, fills.iter().any(|inside| *inside));
                assert!(fills.iter().filter(|inside| **inside).count() <= 1);
            }
        }
        let (_, pack) = component_fixture(&paths, Some(73), true);
        assert_eq!(pack.buildings.len(), 3);
        let key = (14, 8414, 5384);
        let (lat0, lon0) = tile_center(key.0, key.1, key.2);
        let projection = TileProj {
            z: key.0,
            x: key.1,
            y: key.2,
            extent: 4096.0,
            lat0,
            lon0,
        };
        for (building, component) in pack.buildings.iter().zip(components) {
            let (rings, anchor) = projection.geometry_with_location(&component);
            assert_eq!(building.rings, rings);
            assert_eq!(building.location_rad, anchor);
            assert_eq!(
                building.seed,
                feature_seed(Some(73), key.0, key.1, key.2, paths[0][0], 4096)
            );
            assert_eq!(building.height_m, 16.5);
            assert_eq!(building.min_height_m, 2.0);
            assert_eq!(building.roof_shape, Some(RoofShape::Hipped));
        }
        assert_ne!(
            pack.buildings[0].location_rad,
            pack.buildings[1].location_rad
        );
    }

    #[test]
    fn building_components_single_polygon_is_bit_equivalent() {
        let outer = component_square(100, 100, 500);
        let mut hole = component_square(150, 150, 200);
        hole.reverse();
        for paths in [vec![outer.clone()], vec![outer.clone(), hole]] {
            for id in [None, Some(543)] {
                for physical in [false, true] {
                    let (_, pack) = component_fixture(&paths, id, physical);
                    assert_eq!(pack.buildings.len(), 1);
                    let key = (14, 8414, 5384);
                    let (lat0, lon0) = tile_center(key.0, key.1, key.2);
                    let projection = TileProj {
                        z: key.0,
                        x: key.1,
                        y: key.2,
                        extent: 4096.0,
                        lat0,
                        lon0,
                    };
                    let (rings, location_rad) = projection.geometry_with_location(&paths);
                    let seed = feature_seed(id, key.0, key.1, key.2, paths[0][0], 4096);
                    let original = Building {
                        rings,
                        location_rad,
                        seed,
                        height_m: if physical {
                            16.5
                        } else {
                            5.0 + (seed % 900) as f32 / 100.0
                        },
                        min_height_m: if physical { 2.0 } else { 0.0 },
                        kind: BuildingKind::Building,
                        roof_shape: physical.then_some(RoofShape::Hipped),
                        use_kind: if physical {
                            building_use(Some("commercial"))
                        } else {
                            building_use(None)
                        },
                    };
                    assert_building_equal(&original, &pack.buildings[0]);
                }
            }
        }
    }

    #[test]
    fn building_components_identity_and_local_anchor_ignore_component_order() {
        let mut hole = component_square(150, 150, 100);
        hole.reverse();
        let a = vec![component_square(100, 100, 200), hole];
        let b = vec![component_square(800, 500, 100)];
        let c = vec![component_square(1500, 900, 50)];
        let first = [a.clone(), b.clone(), c.clone()].concat();
        let second = [c, b, a].concat();
        for id in [None, Some(42)] {
            for physical in [false, true] {
                let (_, before) = component_fixture(&first, id, physical);
                let (_, after) = component_fixture(&second, id, physical);
                assert_eq!(before.buildings.len(), 3);
                for building in &before.buildings {
                    let other = after
                        .buildings
                        .iter()
                        .find(|b| b.location_rad == building.location_rad)
                        .unwrap();
                    assert_building_equal(building, other);
                }
                if id.is_some() {
                    assert!(
                        before
                            .buildings
                            .iter()
                            .all(|b| b.seed == before.buildings[0].seed)
                    );
                } else {
                    let seeds: HashSet<_> = before.buildings.iter().map(|b| b.seed).collect();
                    assert_eq!(seeds.len(), 3);
                }
            }
        }
    }

    #[test]
    fn building_components_keep_only_interacting_rings_together_in_source_order() {
        let a = component_square(0, 0, 10);
        let b = component_square(10, 0, 10);
        let c = component_square(20, 0, 10);
        let remote = component_square(1000, 1000, 10);
        for paths in [
            vec![a.clone(), remote.clone(), b.clone(), c.clone()],
            vec![c.clone(), remote.clone(), b.clone(), a.clone()],
        ] {
            let groups: Vec<_> = polygon_components(&paths).collect();
            assert_eq!(groups.len(), 2);
            assert_eq!(
                groups[0].paths.as_ref(),
                vec![paths[0].clone(), paths[2].clone(), paths[3].clone()]
            );
            assert_eq!(groups[1].paths.as_ref(), &paths[1..2]);
            assert!(matches!(groups[0].paths, std::borrow::Cow::Owned(_)));
            assert!(matches!(groups[1].paths, std::borrow::Cow::Borrowed(_)));
            assert_eq!(groups[0].seed_anchor, [0, 0]);
            let (_, pack) = component_fixture(&paths, None, false);
            assert_eq!(pack.buildings.len(), 2);
            assert_eq!(
                pack.buildings[0].seed,
                feature_seed(None, 14, 8414, 5384, [0, 0], 4096)
            );
        }
        let mut outside = component_square(100, 100, 30);
        outside.reverse();
        let paths = vec![a, outside, remote, component_square(105, 105, 20)];
        let groups: Vec<_> = polygon_components(&paths).collect();
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0].paths.as_ref(),
            vec![paths[0].clone(), paths[1].clone(), paths[3].clone()]
        );
        for x in 95..136 {
            for y in 95..136 {
                let point = [x, y];
                let Some(expected) = raw_component_contains(&paths, point) else {
                    continue;
                };
                let fills: Vec<_> = groups
                    .iter()
                    .map(|g| raw_component_contains(&g.paths, point).unwrap())
                    .collect();
                assert_eq!(expected, fills.iter().any(|inside| *inside));
                assert!(fills.iter().filter(|inside| **inside).count() <= 1);
            }
        }
        let mut aggregate: Vec<_> = (0..300)
            .map(|i| component_square(i * 20, 500, 10))
            .collect();
        aggregate[1] = component_square(10, 500, 10);
        let groups: Vec<_> = polygon_components(&aggregate).collect();
        assert_eq!(groups.len(), 299);
        assert_eq!(
            groups
                .iter()
                .map(|g| g.paths.iter().map(Vec::len).sum::<usize>())
                .max(),
            Some(8)
        );
    }

    #[test]
    fn building_components_keep_ambiguous_source_domains_compound() {
        let outer = component_square(0, 0, 100);
        let separate = component_square(200, 0, 100);
        let mut orphan = component_square(20, 20, 20);
        orphan.reverse();
        let mut outside = component_square(150, 20, 20);
        outside.reverse();
        let mut intersecting = component_square(90, 20, 20);
        intersecting.reverse();
        let mut large_hole = component_square(10, 10, 80);
        large_hole.reverse();
        let mut nested_hole = component_square(20, 20, 60);
        nested_hole.reverse();
        let cases = vec![
            vec![
                outer.clone(),
                large_hole,
                nested_hole,
                component_square(15, 15, 70),
            ],
            vec![orphan.clone(), outer.clone(), separate.clone()],
            vec![outer.clone(), vec![[5, 5], [6, 6]], separate.clone()],
            vec![
                outer.clone(),
                vec![[0, 0], [5, 5], [0, 5], [5, 0]],
                separate.clone(),
            ],
            vec![outer.clone(), outside, separate.clone()],
            vec![outer.clone(), intersecting, separate.clone()],
            vec![outer.clone(), component_square(20, 20, 20)],
            vec![outer.clone(), component_square(50, 50, 100)],
            vec![outer.clone(), component_square(100, 0, 100)],
        ];
        for paths in cases {
            let components: Vec<_> = polygon_components(&paths).map(|c| c.paths).collect();
            for y in -1..105 {
                for x in -1..155 {
                    let point = [2 * x + 1, 2 * y + 1];
                    let Some(expected) = raw_component_contains(&paths, point) else {
                        continue;
                    };
                    let fills: Vec<_> = components
                        .iter()
                        .map(|c| raw_component_contains(c, point).unwrap())
                        .collect();
                    assert_eq!(expected, fills.iter().any(|inside| *inside));
                    assert!(fills.iter().filter(|inside| **inside).count() <= 1);
                }
            }
            let (_, pack) = component_fixture(&paths, Some(3), true);
            assert_eq!(pack.buildings.len(), components.len());
        }
    }

    #[test]
    fn building_components_bound_each_disconnected_building_not_the_tile_aggregate() {
        let paths: Vec<_> = (0..300)
            .map(|i| component_square((i % 20) * 150, (i / 20) * 150, 50))
            .collect();
        assert!(paths.iter().map(Vec::len).sum::<usize>() > 512);
        let (_, pack) = component_fixture(&paths, Some(42), true);
        assert_eq!(pack.buildings.len(), paths.len());
        assert!(
            pack.buildings
                .iter()
                .all(|b| b.rings.len() == 1 && b.rings[0].len() == 4)
        );
        let large = vec![
            component_square(i32::MIN + 1, i32::MIN + 1, 100),
            component_square(i32::MAX - 101, i32::MAX - 101, 100),
        ];
        assert_eq!(polygon_components(&large).count(), 2);
        for axis in 0..2 {
            let line: Vec<_> = (0..4096)
                .map(|i| {
                    let p = if axis == 0 { [i * 10, 0] } else { [0, i * 10] };
                    component_square(p[0], p[1], 5)
                })
                .collect();
            assert_eq!(polygon_components(&line).count(), line.len());
        }
    }

    #[test]
    fn hosted_reads_map_to_block_objects() {
        let block = HOSTED_BUILD_BLOCK_BYTES;
        let url = protomaps_build_url(PROTOMAPS_BUILD);
        assert!(is_block_archive(&url));
        assert!(!is_block_archive(
            "https://build.protomaps.com/20260930.pmtiles"
        ));
        assert!(!is_block_archive(&format!("{PROTOMAPS_BASE}-other/x")));
        assert_eq!(block_url(&url, 7), format!("{url}/7.bin"));
        assert_eq!(block_spans(10, 19).collect::<Vec<_>>(), [(0, 10..20)]);
        assert_eq!(
            block_spans(block - 2, block + 1).collect::<Vec<_>>(),
            [(0, block as usize - 2..block as usize), (1, 0..2)]
        );
        assert_eq!(
            block_spans(3 * block, 3 * block).collect::<Vec<_>>(),
            [(3, 0..1)]
        );
    }

    #[test]
    fn mercator_tile_matches_probe() {
        // Manhattan z15 → 9649/12314 (verified against the live archive).
        let (x, y) = mercator_tile(40.7580f64.to_radians(), -73.9855f64.to_radians(), 15);
        assert_eq!((x, y), (9649, 12314));
    }

    #[test]
    fn mercator_x_span_wraps_across_antimeridian() {
        let z = 15;
        let n = 1u32 << z;
        let lat = 0.0;
        let lon = 179.999_f64.to_radians();
        let dlon = 500.0 / MEAN_RADIUS;
        let (west, _) = mercator_tile(lat, lon - dlon, z);
        let (east, _) = mercator_tile(lat, lon + dlon, z);
        let (start, count) = mercator_x_span(west, east, n, false);
        let xs: Vec<_> = (0..count).map(|step| (start + step) % n).collect();
        assert_eq!(xs, vec![n - 1, 0]);
    }

    #[test]
    fn exact_tree_budget_is_deterministic_filtered_and_prefix_free() {
        let tree = |id, point| ExactTree {
            point,
            identity: SourceFeatureIdentity::Provided(id),
            seed: id,
        };
        let mut pack = FeaturePack::empty();
        pack.exact_trees = vec![
            tree(2, [5.0, 0.0]),
            tree(1, [1.0, 1.0]),
            tree(1, [1.1, 1.0]),
            tree(3, [50.0, 0.0]),
        ];
        finalize_exact_trees(
            &mut pack,
            10.0,
            10.0,
            &mut SemanticConsumerReport::default(),
        );
        assert_eq!(
            pack.exact_trees
                .iter()
                .map(|tree| tree.identity)
                .collect::<Vec<_>>(),
            vec![
                SourceFeatureIdentity::Provided(1),
                SourceFeatureIdentity::Provided(2)
            ]
        );

        pack.exact_trees = (0..=MAX_EXACT_TREES_PER_PACK)
            .map(|id| tree(id as u64, [0.0, 0.0]))
            .collect();
        finalize_exact_trees(
            &mut pack,
            10.0,
            10.0,
            &mut SemanticConsumerReport::default(),
        );
        assert!(pack.exact_trees.is_empty());
        assert!(!pack.complete);
    }

    #[test]
    fn cliff_finalization_is_direction_canonical_without_merging_parallel_lines() {
        let identity = SourceFeatureIdentity::Provided(17);
        let first = vec![[-8.0, -2.0], [0.0, 0.0], [8.0, 2.0]];
        let mut reversed = first.clone();
        reversed.reverse();
        let parallel = vec![[-8.0, 3.0], [0.0, 5.0], [8.0, 7.0]];
        let mut pack = FeaturePack::empty();
        pack.cliffs = vec![
            CliffLine {
                pts: reversed,
                identity,
            },
            CliffLine {
                pts: first.clone(),
                identity,
            },
            CliffLine {
                pts: parallel.clone(),
                identity,
            },
            CliffLine {
                pts: vec![[f32::NAN, 0.0]],
                identity: SourceFeatureIdentity::Provided(18),
            },
        ];

        finalize_cliffs(
            &mut pack,
            20.0,
            20.0,
            &mut SemanticConsumerReport::default(),
        );

        assert_eq!(pack.cliffs.len(), 2);
        assert_eq!(pack.cliffs[0].pts, first);
        assert_eq!(pack.cliffs[1].pts, parallel);
        assert!(pack.complete);

        pack.cliffs = (0..=MAX_CLIFF_LINES_PER_PACK)
            .map(|id| CliffLine {
                pts: vec![[0.0, id as f32 * 0.001], [1.0, id as f32 * 0.001]],
                identity: SourceFeatureIdentity::Provided(id as u64),
            })
            .collect();
        finalize_cliffs(
            &mut pack,
            20.0,
            20.0,
            &mut SemanticConsumerReport::default(),
        );
        assert!(pack.cliffs.is_empty());
        assert!(!pack.complete);
    }

    #[test]
    fn road_kind_mapping() {
        assert_eq!(
            road_class("major_road", Some("primary")),
            Some(RoadClass::Major)
        );
        assert_eq!(
            road_class("major_road", Some("trunk")),
            Some(RoadClass::Highway)
        );
        assert_eq!(
            road_class("major_road", Some("tertiary_link")),
            Some(RoadClass::Medium)
        );
        assert_eq!(
            road_class("major_road", Some("future")),
            Some(RoadClass::Major)
        );
        assert_eq!(
            road_class("minor_road", Some("service")),
            Some(RoadClass::Service)
        );
        assert_eq!(
            road_class("minor_road", Some("parking_aisle")),
            Some(RoadClass::Service)
        );
        assert_eq!(
            road_class("minor_road", Some("residential")),
            Some(RoadClass::Minor)
        );
        assert_eq!(road_class("ferry", None), None);
        assert_eq!(road_class("rail", Some("subway")), None);
        assert_eq!(road_class("rail", Some("disused")), None);
        assert_eq!(road_class("rail", Some("tram")), Some(RoadClass::Rail));
        assert_eq!(road_class_for_feature("path", Some("steps"), false), None);
        assert_eq!(
            road_class_for_feature("path", Some("steps"), true),
            Some(RoadClass::Path)
        );
        assert_eq!(
            road_class_for_feature("path", Some("footway"), false),
            Some(RoadClass::Path)
        );
        assert!(RoadClass::Highway.width_m() > RoadClass::Path.width_m());
    }

    #[test]
    fn road_context_anchors_follow_bent_routes_in_source_coordinates() {
        let projection = TileProj {
            z: 15,
            x: 16384,
            y: 16384,
            extent: 4096.0,
            lat0: 0.0,
            lon0: 0.0,
        };
        for (path, midpoint) in [
            (vec![[0, 0], [0, 100], [100, 100]], [0, 100]),
            (vec![[0, 0], [0, 100], [100, 100], [100, 0]], [50, 100]),
            (vec![[0, 0], [0, 100], [300, 100]], [100, 100]),
            (vec![[20, 30], [20, 30]], [20, 30]),
        ] {
            let expected = projection
                .geometry_with_location(&[vec![midpoint]])
                .1
                .unwrap();
            let (geometry, anchor) = projection.road_geometry_with_location(&path);
            assert_eq!(anchor.unwrap(), expected);
            assert_eq!(
                geometry,
                projection.paths_to_en(std::slice::from_ref(&path))[0]
            );
            let reversed: Vec<_> = path.iter().rev().copied().collect();
            assert_eq!(projection.road_geometry_with_location(&reversed).1, anchor);
            let duplicated: Vec<_> = path.iter().flat_map(|&point| [point, point]).collect();
            assert_eq!(
                projection.road_geometry_with_location(&duplicated).1,
                anchor
            );
            let shifted = TileProj {
                lat0: 0.001,
                lon0: -0.001,
                ..projection
            };
            let (other_geometry, other_anchor) = shifted.road_geometry_with_location(&path);
            assert_eq!(other_anchor, anchor);
            assert_ne!(other_geometry, geometry);
        }
        assert_eq!(
            projection.road_geometry_with_location(&[]),
            (Vec::new(), None)
        );
        let bent = vec![[0, 0], [0, 100], [100, 100], [100, 0]];
        assert_ne!(
            projection.road_geometry_with_location(&bent).1,
            projection.geometry_with_location(&[bent]).1
        );
        // A non-axis-aligned route exercises the canonical floating-point sum.
        let irregular = vec![[50, 90], [179, 374], [2000, 87], [3900, 3798]];
        let reversed: Vec<_> = irregular.iter().rev().copied().collect();
        assert_eq!(
            projection.road_geometry_with_location(&irregular).1,
            projection.road_geometry_with_location(&reversed).1
        );
    }

    #[test]
    fn retained_road_profiles_share_source_identity_across_render_origins() {
        use crate::mvt::encode::FeatureSpec;
        let values = [
            Value::Str("other".into()),
            Value::Str("living_street".into()),
            Value::Str("highway".into()),
            Value::Str("motorway_link".into()),
            Value::Str("path".into()),
            Value::Str("cycleway".into()),
            Value::Str("footway".into()),
            Value::Bool(true),
            Value::Str("future_transport".into()),
        ];
        let entries = [
            (0, 1, false),
            (2, 3, false),
            (4, 5, false),
            (4, 6, false),
            (0, 8, false),
            (4, 6, true),
        ];
        let features: Vec<_> = entries
            .into_iter()
            .enumerate()
            .map(|(i, (kind, detail, tunnel))| FeatureSpec {
                id: Some(i as u64 + 100),
                geom_type: GeomType::Line,
                paths: vec![vec![
                    [500, 500 + i as i32 * 300],
                    [3500, 500 + i as i32 * 300],
                ]],
                tags: if tunnel {
                    vec![(0, kind), (1, detail), (2, 7)]
                } else {
                    vec![(0, kind), (1, detail)]
                },
            })
            .collect();
        let encoded = mvt::encode::tile(
            "roads",
            4096,
            &["kind", "kind_detail", "is_tunnel"],
            &values,
            &features,
        );
        let tile =
            SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 16384, 16384));
        let collect = |origin| {
            let mut roads = Vec::new();
            append_semantic_roads(
                &mut roads,
                &tile,
                (15, 16384, 16384),
                origin,
                &mut SemanticConsumerReport::default(),
            );
            roads
        };
        let a = collect((0.0, 0.0));
        let b = collect((0.0001, 0.0001));
        assert_eq!(a.len(), 4);
        assert_eq!(
            a.iter().map(Road::width_m).collect::<Vec<_>>(),
            vec![5.0, 6.0, 2.8, 1.8]
        );
        for (a, b) in a.iter().zip(&b) {
            assert_eq!(a.profile, b.profile);
            assert_ne!(a.pts, b.pts);
            assert!(a.profile.location_rad.is_some());
        }
        assert_eq!(a[0].profile.detail, RoadDetail::LivingStreet);
        assert_eq!(a[1].profile.detail, RoadDetail::Ramp);
    }

    #[test]
    fn road_style_identity_survives_source_merging_splitting_and_direction() {
        use crate::mvt::encode::FeatureSpec;
        let values = [
            Value::Str("path".into()),
            Value::Str("footway".into()),
            Value::Str("Riverside Walk".into()),
            Value::Str("Hill Walk".into()),
        ];
        let feature = |id, name: Option<u32>, paths| FeatureSpec {
            id: Some(id),
            geom_type: GeomType::Line,
            paths,
            tags: name.map_or_else(
                || vec![(0, 0), (1, 1)],
                |name| vec![(0, 0), (1, 1), (2, name)],
            ),
        };
        let collect = |features: &[FeatureSpec], zoom| {
            let bytes = mvt::encode::tile(
                "roads",
                4096,
                &["kind", "kind_detail", "name"],
                &values,
                features,
            );
            let tile =
                SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), (zoom, 16384, 16384));
            let mut roads = Vec::new();
            append_semantic_roads(
                &mut roads,
                &tile,
                (zoom, 16384, 16384),
                (0.0, 0.0),
                &mut SemanticConsumerReport::default(),
            );
            roads
                .into_iter()
                .map(|road| road.profile.seed)
                .collect::<Vec<_>>()
        };
        let whole = vec![vec![[500, 500], [1500, 1500], [2500, 2500]]];
        let split = vec![
            vec![[1500, 1500], [500, 500]],
            vec![[1500, 1500], [2500, 2500]],
        ];
        let expected = collect(&[feature(7, Some(2), whole.clone())], 15)[0];
        assert_ne!(expected, 7);
        assert_ne!(expected, 0);
        for zoom in [14, 15, 16] {
            assert_eq!(
                collect(&[feature(98, Some(2), split.clone())], zoom),
                vec![expected; 2]
            );
            assert_eq!(
                collect(
                    &[
                        feature(9, Some(2), vec![split[0].clone()]),
                        feature(10, Some(2), vec![split[1].clone()])
                    ],
                    zoom
                ),
                vec![expected; 2]
            );
        }
        let merged_id = collect(
            &[
                feature(7, Some(2), whole.clone()),
                feature(7, Some(3), whole.clone()),
            ],
            15,
        );
        assert_ne!(merged_id[0], merged_id[1]);
        assert_eq!(
            collect(&[feature(7, None, whole), feature(88, None, split)], 15),
            vec![0; 3]
        );
    }

    #[test]
    fn documented_road_detail_refines_the_existing_physical_hierarchy() {
        use crate::mvt::encode::FeatureSpec;

        let values = [
            Value::Str("major_road".into()),
            Value::Str("trunk".into()),
            Value::Str("tertiary_link".into()),
            Value::Str("future_detail".into()),
        ];
        let feature = |id, y, detail| FeatureSpec {
            id: Some(id),
            geom_type: GeomType::Line,
            paths: vec![vec![[100, y], [800, y]]],
            tags: vec![(0, 0), (1, detail)],
        };
        let encoded = mvt::encode::tile(
            "roads",
            4096,
            &["kind", "kind_detail"],
            &values,
            &[feature(1, 100, 1), feature(2, 200, 2), feature(3, 300, 3)],
        );
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 0, 0));
        let layer = tile.semantic_layer("roads").unwrap();
        let detail_state = |index: usize| {
            let LayerFacts::Roads(facts) = &layer.features[index].facts else {
                unreachable!()
            };
            facts.physical.kind_detail.unwrap().normalized
        };
        assert_eq!(
            detail_state(0),
            crate::protomaps_semantics::NormalizedValue::Text
        );
        assert_eq!(
            detail_state(1),
            crate::protomaps_semantics::NormalizedValue::Text
        );
        assert_eq!(
            detail_state(2),
            crate::protomaps_semantics::NormalizedValue::UnknownText
        );

        let mut pack = FeaturePack::empty();
        append_current_tile_features(
            &mut pack,
            &tile,
            (15, 0, 0),
            15,
            (0.0, 0.0),
            &mut SemanticConsumerReport::default(),
        );
        assert_eq!(
            pack.roads.iter().map(|road| road.class).collect::<Vec<_>>(),
            vec![RoadClass::Highway, RoadClass::Medium, RoadClass::Major]
        );
    }

    #[test]
    fn documented_service_details_use_the_narrow_asphalt_class_off_bridges() {
        use crate::mvt::encode::FeatureSpec;

        let values = [
            Value::Str("minor_road".into()),
            Value::Str("service".into()),
            Value::Str("parking_aisle".into()),
            Value::Str("residential".into()),
            Value::Bool(true),
        ];
        let feature = |id, y, detail, bridge| FeatureSpec {
            id: Some(id),
            geom_type: GeomType::Line,
            paths: vec![vec![[100, y], [800, y]]],
            tags: if bridge {
                vec![(0, 0), (1, detail), (2, 4)]
            } else {
                vec![(0, 0), (1, detail)]
            },
        };
        let encoded = mvt::encode::tile(
            "roads",
            4096,
            &["kind", "kind_detail", "is_bridge"],
            &values,
            &[
                feature(1, 100, 1, false),
                feature(2, 200, 2, true),
                feature(3, 300, 3, false),
            ],
        );
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 0, 0));
        let mut pack = FeaturePack::empty();
        append_current_tile_features(
            &mut pack,
            &tile,
            (15, 0, 0),
            15,
            (0.0, 0.0),
            &mut SemanticConsumerReport::default(),
        );

        assert_eq!(pack.roads[0].class, RoadClass::Service);
        assert!(!pack.roads[0].is_bridge);
        assert_eq!(pack.roads[1].class, RoadClass::Minor);
        assert!(pack.roads[1].is_bridge);
        assert_eq!(pack.roads[2].class, RoadClass::Minor);
    }

    #[test]
    fn documented_nonphysical_rail_detail_never_becomes_surface_rail() {
        use crate::mvt::encode::FeatureSpec;

        let encoded = mvt::encode::tile(
            "roads",
            4096,
            &["kind", "kind_detail", "is_tunnel"],
            &[
                Value::Str("rail".into()),
                Value::Str("subway".into()),
                Value::Str("tram".into()),
                Value::Str("disused".into()),
                Value::Bool(false),
            ],
            &[
                FeatureSpec {
                    id: Some(1),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[100, 100], [800, 100]]],
                    tags: vec![(0, 0), (1, 1), (2, 4)],
                },
                FeatureSpec {
                    id: Some(2),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[100, 200], [800, 200]]],
                    tags: vec![(0, 0), (1, 2), (2, 4)],
                },
                FeatureSpec {
                    id: Some(3),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[100, 300], [800, 300]]],
                    tags: vec![(0, 0), (1, 3), (2, 4)],
                },
            ],
        );
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 0, 0));
        let mut pack = FeaturePack::empty();
        append_current_tile_features(
            &mut pack,
            &tile,
            (15, 0, 0),
            15,
            (0.0, 0.0),
            &mut SemanticConsumerReport::default(),
        );

        assert_eq!(pack.roads.len(), 1);
        assert_eq!(pack.roads[0].class, RoadClass::Rail);
    }

    #[test]
    fn nonroad_transport_families_keep_context_without_stamping_asphalt() {
        use crate::mvt::encode::FeatureSpec;

        let feature = |id, y, tags| FeatureSpec {
            id: Some(id),
            geom_type: GeomType::Line,
            paths: vec![vec![[100, y], [800, y]]],
            tags,
        };
        let encoded = mvt::encode::tile(
            "roads",
            4096,
            &[
                "kind",
                "kind_detail",
                "ref",
                "shield_text",
                "network",
                "oneway",
            ],
            &[
                Value::Str("aeroway".into()),
                Value::Str("runway".into()),
                Value::Str("09/27".into()),
                Value::Str("09".into()),
                Value::Str("fixture-airport".into()),
                Value::Str("yes".into()),
                Value::Str("aerialway".into()),
                Value::Str("cable_car".into()),
                Value::Str("pier".into()),
                Value::Str("ferry".into()),
                Value::Str("rail".into()),
            ],
            &[
                feature(1, 100, vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]),
                feature(2, 200, vec![(0, 6), (1, 7)]),
                feature(3, 300, vec![(0, 8)]),
                feature(4, 400, vec![(0, 9)]),
                feature(5, 500, vec![(0, 10), (1, 10)]),
            ],
        );
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 0, 0));
        let layer = tile.semantic_layer("roads").unwrap();
        let LayerFacts::Roads(runway) = &layer.features[0].facts else {
            unreachable!()
        };
        assert_eq!(
            runway.physical.kind_detail.unwrap().normalized,
            crate::protomaps_semantics::NormalizedValue::UnknownText
        );
        assert_eq!(
            runway
                .context
                .reference
                .and_then(|fact| tile.source_text(layer, fact)),
            Some("09/27")
        );
        assert_eq!(runway.context.shield_texts.len(), 1);
        assert_eq!(runway.context.networks.len(), 1);
        assert_eq!(
            runway
                .context
                .oneway
                .and_then(|fact| tile.source_text(layer, fact)),
            Some("yes")
        );

        let mut pack = FeaturePack::empty();
        append_current_tile_features(
            &mut pack,
            &tile,
            (15, 0, 0),
            15,
            (0.0, 0.0),
            &mut SemanticConsumerReport::default(),
        );
        assert_eq!(pack.roads.len(), 1);
        assert_eq!(pack.roads[0].class, RoadClass::Rail);
    }

    #[test]
    fn documented_steps_do_not_become_a_flat_rideable_path() {
        use crate::mvt::encode::FeatureSpec;

        let encoded = mvt::encode::tile(
            "roads",
            4096,
            &["kind", "kind_detail", "is_bridge"],
            &[
                Value::Str("path".into()),
                Value::Str("steps".into()),
                Value::Str("footway".into()),
                Value::Bool(false),
                Value::Bool(true),
            ],
            &[
                FeatureSpec {
                    id: Some(1),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[100, 100], [800, 100]]],
                    tags: vec![(0, 0), (1, 1), (2, 3)],
                },
                FeatureSpec {
                    id: Some(2),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[100, 200], [800, 200]]],
                    tags: vec![(0, 0), (1, 1), (2, 4)],
                },
                FeatureSpec {
                    id: Some(3),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[100, 300], [800, 300]]],
                    tags: vec![(0, 0), (1, 2), (2, 3)],
                },
            ],
        );
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 0, 0));
        let mut pack = FeaturePack::empty();
        append_current_tile_features(
            &mut pack,
            &tile,
            (15, 0, 0),
            15,
            (0.0, 0.0),
            &mut SemanticConsumerReport::default(),
        );

        assert_eq!(pack.roads.len(), 2);
        assert!(pack.roads.iter().any(|road| road.is_bridge));
        assert!(pack.roads.iter().any(|road| !road.is_bridge));
        assert!(pack.roads.iter().all(|road| road.class == RoadClass::Path));
    }

    #[test]
    fn cultivation_source_anchors_do_not_depend_on_local_projection_origin() {
        use crate::mvt::encode::FeatureSpec;
        for layer in ["landuse", "landcover"] {
            let encoded = mvt::encode::tile(
                layer,
                4096,
                &["kind"],
                &[Value::Str("farmland".into())],
                &[FeatureSpec {
                    id: Some(81),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[100, 100], [800, 100], [800, 900], [100, 900]]],
                    tags: vec![(0, 0)],
                }],
            );
            let tile =
                SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (12, 1000, 1500));
            let extract = |lat0, lon0| {
                let mut pack = FeaturePack::empty();
                append_semantic_land_regions(
                    &mut pack,
                    &tile,
                    layer,
                    12,
                    1000,
                    1500,
                    lat0,
                    lon0,
                    &mut SemanticConsumerReport::default(),
                );
                if layer == "landuse" {
                    let field = pack.flora.remove(0);
                    (field.location_rad, field.seed, field.rings)
                } else {
                    let field = pack.regional_landcover.remove(0);
                    (field.location_rad, field.stable_identity, field.rings)
                }
            };
            let a = extract(0.0, 0.0);
            let b = extract(0.6, -0.8);
            assert!(a.0.is_some());
            assert_eq!(a.0, b.0);
            assert_eq!(a.1, b.1);
            assert_ne!(a.2, b.2);
        }
    }

    #[test]
    fn flora_kinds_decode_without_entering_generic_green_ground() {
        use crate::mvt::encode::FeatureSpec;

        let names = ["farmland", "meadow", "garden", "orchard", "vineyard"];
        let values: Vec<_> = names
            .iter()
            .map(|name| Value::Str((*name).to_string()))
            .collect();
        let features: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let x = 100 + index as i32 * 500;
                FeatureSpec {
                    id: Some(1000 + index as u64),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[x, 100], [x + 300, 100], [x + 300, 400], [x, 400]]],
                    tags: vec![(0, index as u32)],
                }
            })
            .collect();
        let encoded = mvt::encode::tile("landuse", 4096, &["kind"], &values, &features);
        let tile = mvt::decode_tile(&encoded).unwrap();
        let mut pack = FeaturePack::empty();
        append_land_regions(&mut pack, &tile, "landuse", 12, 1000, 1500, 0.0, 0.0);

        assert_eq!(
            pack.flora
                .iter()
                .map(|region| region.kind)
                .collect::<Vec<_>>(),
            vec![
                FloraRegionKind::Farmland,
                FloraRegionKind::Meadow,
                FloraRegionKind::Garden,
                FloraRegionKind::Orchard,
                FloraRegionKind::Vineyard,
            ]
        );
        assert!(pack.ground.is_empty());
        assert!(pack.flora.iter().all(|region| region.seed != 0));
    }

    #[test]
    fn landcover_adds_only_flora_and_does_not_expand_non_flora_semantics() {
        use crate::mvt::encode::FeatureSpec;

        let encoded = mvt::encode::tile(
            "landcover",
            4096,
            &["kind"],
            &[
                Value::Str("farmland".into()),
                Value::Str("forest".into()),
                Value::Str("grass".into()),
            ],
            &[
                FeatureSpec {
                    id: Some(1),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[100, 100], [400, 100], [400, 400], [100, 400]]],
                    tags: vec![(0, 0)],
                },
                FeatureSpec {
                    id: Some(2),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[500, 100], [800, 100], [800, 400], [500, 400]]],
                    tags: vec![(0, 1)],
                },
                FeatureSpec {
                    id: Some(3),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[900, 100], [1200, 100], [1200, 400], [900, 400]]],
                    tags: vec![(0, 2)],
                },
            ],
        );
        let tile = mvt::decode_tile(&encoded).unwrap();
        let mut pack = FeaturePack::empty();
        append_land_regions(&mut pack, &tile, "landcover", 12, 1000, 1500, 0.0, 0.0);

        assert_eq!(pack.flora.len(), 1);
        assert_eq!(pack.flora[0].kind, FloraRegionKind::Farmland);
        assert!(pack.forest.is_empty());
        assert!(pack.ground.is_empty());
    }

    #[test]
    fn flora_feature_ids_keep_seeds_across_fragments_and_source_zooms() {
        use crate::mvt::encode::FeatureSpec;

        let make_tile = |paths| {
            let bytes = mvt::encode::tile(
                "landuse",
                4096,
                &["kind"],
                &[Value::Str("farmland".into())],
                &[FeatureSpec {
                    id: Some(0x1234_5678),
                    geom_type: GeomType::Polygon,
                    paths,
                    tags: vec![(0, 0)],
                }],
            );
            mvt::decode_tile(&bytes).unwrap()
        };
        let left = make_tile(vec![vec![
            [3500, 100],
            [4096, 100],
            [4096, 900],
            [3500, 900],
        ]]);
        let right = make_tile(vec![vec![[0, 120], [800, 120], [800, 950], [0, 950]]]);
        let coarse = make_tile(vec![vec![[1700, 50], [2400, 50], [2400, 500], [1700, 500]]]);

        let seed = |tile: &mvt::VectorTile, z, x, y| {
            let layer = tile.layer("landuse").unwrap();
            flora_feature_seed(
                &layer.features[0],
                FloraRegionKind::Farmland,
                z,
                x,
                y,
                layer.extent,
            )
        };
        assert_eq!(seed(&left, 14, 4000, 6000), seed(&right, 14, 4001, 6000));
        assert_eq!(seed(&left, 14, 4000, 6000), seed(&coarse, 11, 500, 750));
    }

    #[test]
    fn quantized_flora_anchor_is_zoom_independent_without_an_id() {
        use crate::mvt::encode::FeatureSpec;

        let decode = |first: [i32; 2]| {
            let bytes = mvt::encode::tile(
                "landuse",
                4096,
                &["kind"],
                &[Value::Str("garden".into())],
                &[FeatureSpec {
                    id: None,
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![
                        first,
                        [first[0] + 100, first[1]],
                        [first[0] + 100, first[1] + 100],
                        [first[0], first[1] + 100],
                    ]],
                    tags: vec![(0, 0)],
                }],
            );
            mvt::decode_tile(&bytes).unwrap()
        };
        // (100.25, 50.5) / 2^10 == (200.5, 101.0) / 2^11.
        let z10 = decode([1024, 2048]);
        let z11 = decode([2048, 0]);
        let seed = |tile: &mvt::VectorTile, z, x, y| {
            let layer = tile.layer("landuse").unwrap();
            flora_feature_seed(
                &layer.features[0],
                FloraRegionKind::Garden,
                z,
                x,
                y,
                layer.extent,
            )
        };
        assert_eq!(seed(&z10, 10, 100, 50), seed(&z11, 11, 200, 101));
    }

    #[test]
    fn regional_identity_is_seam_stable_without_ids_and_farmland_keeps_its_old_seed() {
        use crate::mvt::encode::FeatureSpec;

        let decode = |kind: &str, id, paths, key| {
            let bytes = mvt::encode::tile(
                "landcover",
                4096,
                &["kind"],
                &[Value::Str(kind.into())],
                &[FeatureSpec {
                    id,
                    geom_type: GeomType::Polygon,
                    paths,
                    tags: vec![(0, 0)],
                }],
            );
            SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), key)
        };
        let left = decode(
            "forest",
            None,
            vec![vec![[3500, 100], [4096, 100], [4096, 900], [3500, 900]]],
            (7, 22, 49),
        );
        let right = decode(
            "forest",
            None,
            vec![vec![[0, 120], [800, 120], [800, 950], [0, 950]]],
            (7, 23, 49),
        );
        let extract = |tile: &SemanticTile, key: TileKey| {
            let mut pack = FeaturePack::empty();
            let mut report = SemanticConsumerReport::default();
            append_semantic_land_regions(
                &mut pack,
                tile,
                "landcover",
                key.0,
                key.1,
                key.2,
                0.0,
                0.0,
                &mut report,
            );
            pack.regional_landcover.remove(0)
        };
        let left = extract(&left, (7, 22, 49));
        let right = extract(&right, (7, 23, 49));
        assert_eq!(left.source_id, None);
        assert_eq!(left.stable_identity, right.stable_identity);

        let farmland = decode(
            "farmland",
            Some(55),
            vec![vec![[100, 100], [300, 100], [300, 300], [100, 300]]],
            (7, 35, 54),
        );
        let expected = {
            let layer = farmland.semantic_layer("landcover").unwrap();
            flora_feature_seed(
                farmland.source_feature(layer, &layer.features[0]).unwrap(),
                FloraRegionKind::Farmland,
                7,
                35,
                54,
                layer.extent,
            )
        };
        assert_eq!(extract(&farmland, (7, 35, 54)).stable_identity, expected);
    }

    #[test]
    fn regional_projection_budget_discards_the_whole_admission_and_marks_incomplete() {
        let telemetry = SemanticTelemetry::new_enabled("budget-test", 1);
        let mut report = telemetry.consumer_report();
        let mut pack = FeaturePack::empty();
        for stable_identity in [1, 2] {
            pack.regional_landcover.push(RegionalLandcoverRegion {
                location_rad: None,
                rings: vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]],
                kind: LandcoverKind::Forest,
                source_id: Some(stable_identity),
                stable_identity,
            });
            report.consumed(SemanticLayerFamily::Landcover);
        }
        assert!(pack.regional_landcover_projected_bytes() > 0);
        assert!(enforce_regional_landcover_budget(
            &mut pack,
            0,
            &mut report,
            0,
        ));
        telemetry.record_consumer(&report);

        assert!(pack.regional_landcover.is_empty());
        assert!(!pack.complete);
        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Landcover), 0);
        assert_eq!(snapshot.layer_discarded(SemanticLayerFamily::Landcover), 2);
        assert_eq!(
            snapshot.discard_count(
                SemanticLayerFamily::Landcover,
                SemanticDiscardReason::ConsumerBudget,
            ),
            2
        );
    }

    #[test]
    fn location_names_preserve_unicode_but_never_provider_shaped_glyphs() {
        use crate::mvt::encode::FeatureSpec;
        let keys = [
            "name",
            "name:en",
            "name:hi",
            "name:mr",
            "pgf:name:hi",
            "name:left",
            "name:de",
        ];
        let values = [
            "Jaipur",
            "Jaipur",
            "जयपुर",
            "जयपूर",
            "\u{e000}\u{e001}",
            "left label",
            "unsafe\u{202e}name",
        ]
        .map(|s| Value::Str(s.into()));
        let bytes = mvt::encode::tile(
            "places",
            4096,
            &keys,
            &values,
            &[FeatureSpec {
                id: Some(101),
                geom_type: GeomType::Point,
                paths: vec![vec![[100, 100]]],
                tags: (0..keys.len() as u32).map(|i| (i, i)).collect(),
            }],
        );
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), (8, 1, 1));
        let layer = tile.semantic_layer("places").unwrap();
        let names = semantic_localized_names(&tile, layer, &layer.features[0]);
        assert_eq!(
            names,
            vec![
                ("".into(), "Jaipur".into()),
                ("en".into(), "Jaipur".into()),
                ("hi".into(), "जयपुर".into()),
                ("mr".into(), "जयपूर".into())
            ]
        );
        let primary = GeographicCandidate {
            id: 101,
            name: "Jaipur".into(),
            localized_names: names.clone(),
            score: 0.,
            distance_m: 0.,
            containing: false,
            populated_place: true,
        };
        let location = compose_named_location(Some(primary.clone()), None).unwrap();
        assert_eq!(location.localized_names, names);
        let region = AdminRegion {
            id: 102,
            name: "Jaipur".into(),
            country_name: "India".into(),
        };
        let region_only = compose_named_location(Some(primary), Some(region)).unwrap();
        assert!(region_only.primary_name.is_none());
        assert!(
            region_only.localized_names.is_empty(),
            "a discarded city's translations must not label a region"
        );
    }

    #[test]
    fn location_label_combines_any_primary_with_region() {
        let location = compose_named_location(
            Some(GeographicCandidate {
                id: 10,
                name: "El Paso".into(),
                localized_names: Vec::new(),
                score: 0.0,
                distance_m: 120.0,
                containing: false,
                populated_place: true,
            }),
            Some(AdminRegion {
                id: 20,
                name: "Texas".into(),
                country_name: "United States of America".into(),
            }),
        )
        .unwrap();
        assert_eq!(location.name, "El Paso, Texas");
        assert_eq!(location.primary_name.as_deref(), Some("El Paso"));
        assert_eq!(location.region_name.as_deref(), Some("Texas"));
    }

    #[test]
    fn region_is_a_complete_fallback_without_country() {
        let location = compose_named_location(
            None,
            Some(AdminRegion {
                id: 30,
                name: "Gorno-Badakhshan".into(),
                country_name: "Tajikistan".into(),
            }),
        )
        .unwrap();
        assert_eq!(location.name, "Gorno-Badakhshan");
        assert_eq!(location.primary_name, None);
        assert_eq!(
            compose_named_location(None, None),
            None,
            "ocean or uncovered coordinates do not invent a country label"
        );
        assert_eq!(
            compose_named_location(
                None,
                Some(AdminRegion {
                    id: 31,
                    name: "Luxembourg".into(),
                    country_name: "Luxembourg".into(),
                }),
            ),
            None,
            "an Admin-1 shell identical to its country is not displayed"
        );
    }

    #[test]
    fn matching_primary_and_region_are_not_repeated() {
        let location = compose_named_location(
            Some(GeographicCandidate {
                id: 10,
                name: "New York".into(),
                localized_names: Vec::new(),
                score: 0.0,
                distance_m: 0.0,
                containing: false,
                populated_place: true,
            }),
            Some(AdminRegion {
                id: 20,
                name: "New York".into(),
                country_name: "United States of America".into(),
            }),
        )
        .unwrap();
        assert_eq!(location.name, "New York");
        assert_eq!(location.primary_name, None);
        assert_eq!(location.region_name.as_deref(), Some("New York"));
    }

    #[test]
    fn region_changes_are_part_of_location_identity() {
        assert_ne!(
            compose_location_id(Some(10), Some(20)),
            compose_location_id(Some(10), Some(21))
        );
    }

    #[test]
    fn country_named_area_falls_back_to_region_but_same_named_city_survives() {
        let region = || AdminRegion {
            id: 30,
            name: "Gorno-Badakhshan".into(),
            country_name: "Tajikistan".into(),
        };
        let area = compose_named_location(
            Some(GeographicCandidate {
                id: 40,
                name: "Tajikistan National Park".into(),
                localized_names: Vec::new(),
                score: 0.0,
                distance_m: 0.0,
                containing: true,
                populated_place: false,
            }),
            Some(region()),
        )
        .unwrap();
        assert_eq!(area.name, "Gorno-Badakhshan");

        let city = compose_named_location(
            Some(GeographicCandidate {
                id: 41,
                name: "Mexico City".into(),
                localized_names: Vec::new(),
                score: 0.0,
                distance_m: 0.0,
                containing: false,
                populated_place: true,
            }),
            Some(AdminRegion {
                id: 31,
                name: "Federal District".into(),
                country_name: "Mexico".into(),
            }),
        )
        .unwrap();
        assert_eq!(city.name, "Mexico City, Federal District");
    }

    #[test]
    fn production_building_properties_survive_mvt_ingestion() {
        use crate::mvt::encode::FeatureSpec;

        let keys = ["kind", "height", "min_height", "roof:shape", "kind_detail"];
        let values = [
            Value::Str("building_part".into()),
            Value::F64(14.8),
            Value::F64(3.1),
            Value::Str("hipped".into()),
            Value::Str("school".into()),
        ];
        let encoded = mvt::encode::tile(
            "buildings",
            4096,
            &keys,
            &values,
            &[FeatureSpec {
                id: Some(7),
                geom_type: GeomType::Polygon,
                paths: vec![vec![[100, 100], [300, 100], [300, 300], [100, 300]]],
                tags: (0..keys.len())
                    .map(|index| (index as u32, index as u32))
                    .collect(),
            }],
        );
        let decoded = mvt::decode_tile(&encoded).unwrap();
        let semantic = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 0, 0));
        let mut original = FeaturePack::empty();
        append_buildings(&mut original, &decoded, (15, 0, 0), (0.0, 0.0), None);
        let mut production = FeaturePack::empty();
        append_semantic_buildings(
            &mut production,
            &semantic,
            (15, 0, 0),
            (0.0, 0.0),
            None,
            &mut SemanticConsumerReport::default(),
        );
        for building in [&original.buildings[0], &production.buildings[0]] {
            assert!((building.height_m - 14.8).abs() < 0.01);
            assert!((building.min_height_m - 3.1).abs() < 0.01);
            assert_eq!(building.kind, BuildingKind::Part);
            assert_eq!(building.roof_shape, Some(RoofShape::Hipped));
            assert_eq!(building.use_kind, BuildingUse::Civic);
            assert!(building.location_rad.is_some());
        }
        assert_eq!(
            original.buildings[0].location_rad,
            production.buildings[0].location_rad
        );
        assert_eq!(original.buildings[0].rings, production.buildings[0].rings);
    }

    #[test]
    fn building_source_anchor_is_independent_of_query_origin_and_preserves_geometry() {
        use crate::mvt::encode::FeatureSpec;
        let keys = ["kind", "height", "roof:shape", "kind_detail"];
        let values = [
            Value::Str("building".into()),
            Value::F64(12.0),
            Value::Str("sawtooth".into()),
            Value::Str("warehouse".into()),
        ];
        let paths = vec![vec![[3971, 3792], [4273, 3819], [4289, 4083], [3991, 4107]]];
        let encoded = mvt::encode::tile(
            "buildings",
            4096,
            &keys,
            &values,
            &[FeatureSpec {
                id: Some(70),
                geom_type: GeomType::Polygon,
                paths: paths.clone(),
                tags: vec![(0, 0), (1, 1), (2, 2), (3, 3)],
            }],
        );
        for key in [(15, 23_106, 14_668), (15, 32_767, 12_000)] {
            let decoded = mvt::decode_tile(&encoded).unwrap();
            let semantic = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), key);
            let mut anchors = Vec::new();
            let mut first_projected = None;
            let centre = tile_center(key.0, key.1, key.2);
            for delta in [[0.0, 0.0], [0.00017, -0.00021], [-0.00043, 0.00037]] {
                let origin = (centre.0 + delta[0], centre.1 + delta[1]);
                let mut raw = FeaturePack::empty();
                append_buildings(&mut raw, &decoded, key, origin, None);
                let mut production = FeaturePack::empty();
                append_semantic_buildings(
                    &mut production,
                    &semantic,
                    key,
                    origin,
                    None,
                    &mut SemanticConsumerReport::default(),
                );
                let p = TileProj {
                    z: key.0,
                    x: key.1,
                    y: key.2,
                    extent: 4096.0,
                    lat0: origin.0,
                    lon0: origin.1,
                };
                for building in [&raw.buildings[0], &production.buildings[0]] {
                    assert_eq!(building.rings, p.paths_to_en(&paths));
                    assert_eq!(building.roof_shape, Some(RoofShape::Sawtooth));
                    assert_eq!(building.use_kind, BuildingUse::Industrial);
                    let anchor = building.location_rad.unwrap();
                    assert!(anchor[0].abs() <= std::f64::consts::FRAC_PI_2);
                    assert!(anchor[1] >= -std::f64::consts::PI && anchor[1] < std::f64::consts::PI);
                    anchors.push(anchor.map(f64::to_bits));
                }
                if let Some(first) = &first_projected {
                    assert_ne!(&production.buildings[0].rings, first);
                } else {
                    first_projected = Some(production.buildings[0].rings.clone());
                }
            }
            assert!(anchors.windows(2).all(|pair| pair[0] == pair[1]));
        }
    }

    #[test]
    fn utility_building_details_survive_raw_and_semantic_consumers() {
        use crate::mvt::encode::FeatureSpec;
        for detail in [
            "chimney",
            "smokestack",
            "silo",
            "storage_tank",
            "water_tower",
            "cooling_tower",
            "spire",
            "steeple",
        ] {
            let encoded = mvt::encode::tile(
                "buildings",
                4096,
                &["kind", "kind_detail", "height"],
                &[
                    Value::Str("building_part".into()),
                    Value::Str(detail.into()),
                    Value::F64(80.0),
                ],
                &[FeatureSpec {
                    id: Some(7),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[100, 100], [300, 100], [300, 300], [100, 300]]],
                    tags: vec![(0, 0), (1, 1), (2, 2)],
                }],
            );
            let raw = mvt::decode_tile(&encoded).unwrap();
            let mut raw_pack = FeaturePack::empty();
            append_buildings(&mut raw_pack, &raw, (15, 0, 0), (0.0, 0.0), None);
            let semantic = SemanticTile::normalize_at(raw, (15, 0, 0));
            let mut pack = FeaturePack::empty();
            append_semantic_buildings(
                &mut pack,
                &semantic,
                (15, 0, 0),
                (0.0, 0.0),
                None,
                &mut SemanticConsumerReport::default(),
            );
            for building in [&raw_pack.buildings[0], &pack.buildings[0]] {
                assert_eq!(building.use_kind, BuildingUse::Utility, "{detail}");
                assert_eq!(building.kind, BuildingKind::Part);
                assert_eq!(building.height_m, 80.0);
            }
        }
    }

    #[test]
    fn unknown_and_mistyped_building_metadata_do_not_infer_source_facts() {
        use crate::mvt::encode::FeatureSpec;
        for value in [
            Value::Str("unrecognized".into()),
            Value::Bool(true),
            Value::F64(12.0),
        ] {
            let encoded = mvt::encode::tile(
                "buildings",
                4096,
                &["kind", "roof:shape", "kind_detail"],
                &[Value::Str("building".into()), value],
                &[FeatureSpec {
                    id: Some(7),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[100, 100], [300, 100], [300, 300], [100, 300]]],
                    tags: vec![(0, 0), (1, 1), (2, 1)],
                }],
            );
            let semantic =
                SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 0, 0));
            let mut pack = FeaturePack::empty();
            append_semantic_buildings(
                &mut pack,
                &semantic,
                (15, 0, 0),
                (0.0, 0.0),
                None,
                &mut SemanticConsumerReport::default(),
            );
            assert_eq!(pack.buildings[0].roof_shape, None);
            assert_eq!(pack.buildings[0].use_kind, BuildingUse::Unknown);
        }
    }

    #[test]
    fn roof_only_parts_require_a_complete_source_interval() {
        use crate::mvt::encode::FeatureSpec;

        let keys = ["kind", "kind_detail", "height", "min_height"];
        let values = [
            Value::Str("building_part".into()),
            Value::Str("roof".into()),
            Value::F64(12.0),
            Value::F64(10.0),
        ];
        let feature = |id, x, tags| FeatureSpec {
            id: Some(id),
            geom_type: GeomType::Polygon,
            paths: vec![vec![[x, 100], [x + 100, 100], [x + 100, 200], [x, 200]]],
            tags,
        };
        let encoded = mvt::encode::tile(
            "buildings",
            4096,
            &keys,
            &values,
            &[
                feature(1, 100, vec![(0, 0), (1, 1), (2, 2), (3, 3)]),
                feature(2, 300, vec![(0, 0), (1, 1), (2, 2)]),
                feature(3, 500, vec![(0, 0), (1, 1), (2, 2), (3, 2)]),
            ],
        );
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 0, 0));
        let mut pack = FeaturePack::empty();
        append_current_tile_features(
            &mut pack,
            &tile,
            (15, 0, 0),
            15,
            (0.0, 0.0),
            &mut SemanticConsumerReport::default(),
        );

        assert_eq!(pack.buildings[0].kind, BuildingKind::RoofOnly);
        assert_eq!(pack.buildings[0].min_height_m, 10.0);
        assert_eq!(pack.buildings[1].kind, BuildingKind::Part);
        assert_eq!(pack.buildings[2].kind, BuildingKind::Part);
        assert_eq!(pack.buildings[2].min_height_m, 11.0);
    }

    #[test]
    fn negative_layer_buildings_do_not_become_ground_level_solids() {
        use crate::mvt::encode::FeatureSpec;

        let encoded = mvt::encode::tile(
            "buildings",
            4096,
            &["kind", "layer"],
            &[Value::Str("building".into()), Value::I64(-1), Value::I64(0)],
            &[
                FeatureSpec {
                    id: Some(1),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[100, 100], [800, 100], [800, 800], [100, 100]]],
                    tags: vec![(0, 0), (1, 1)],
                },
                FeatureSpec {
                    id: Some(2),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[900, 100], [1600, 100], [1600, 800], [900, 100]]],
                    tags: vec![(0, 0), (1, 2)],
                },
            ],
        );
        let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), (15, 0, 0));
        let mut pack = FeaturePack::empty();
        append_current_tile_features(
            &mut pack,
            &tile,
            (15, 0, 0),
            15,
            (0.0, 0.0),
            &mut SemanticConsumerReport::default(),
        );

        assert_eq!(pack.buildings.len(), 1);
        assert_eq!(pack.buildings[0].kind, BuildingKind::Building);
    }

    #[test]
    fn viewpoint_and_attraction_pois_survive_as_observation_hints() {
        use crate::mvt::encode::FeatureSpec;

        let encoded = mvt::encode::tile(
            "pois",
            4096,
            &["kind"],
            &[
                Value::Str("viewpoint".into()),
                Value::Str("attraction".into()),
                Value::Str("restaurant".into()),
            ],
            &[
                FeatureSpec {
                    id: Some(31),
                    geom_type: GeomType::Point,
                    paths: vec![vec![[100, 100]]],
                    tags: vec![(0, 0)],
                },
                FeatureSpec {
                    id: Some(32),
                    geom_type: GeomType::Point,
                    paths: vec![vec![[200, 200]]],
                    tags: vec![(0, 1)],
                },
                FeatureSpec {
                    id: Some(33),
                    geom_type: GeomType::Point,
                    paths: vec![vec![[300, 300]]],
                    tags: vec![(0, 2)],
                },
            ],
        );
        let tile = mvt::decode_tile(&encoded).unwrap();
        let mut pack = FeaturePack::empty();
        append_observation_hints(&mut pack, &tile, 15, 0, 0, 0.0, 0.0);
        assert_eq!(pack.observation_hints.len(), 2);
        assert_eq!(
            pack.observation_hints[0].kind,
            ObservationHintKind::Viewpoint
        );
        assert_eq!(
            pack.observation_hints[1].kind,
            ObservationHintKind::Attraction
        );
    }

    #[test]
    fn decoded_lru_never_evicts_in_flight_slots() {
        let mut cache = HashMap::new();
        for x in 0..=TILE_CACHE_MAX as u32 {
            cache.insert((15, x, 0), (Arc::new(TileSlot::new()), x as u64));
        }

        ProtomapsSource::prune_decoded_completed(&mut cache);
        assert_eq!(cache.len(), TILE_CACHE_MAX + 1);
        assert!(cache.values().all(|(slot, _)| slot.get().is_none()));

        let completed = cache.get(&(15, 0, 0)).unwrap().0.clone();
        assert!(completed.set(Ok(Arc::new(None))).is_ok());
        ProtomapsSource::prune_decoded_completed(&mut cache);
        assert_eq!(cache.len(), TILE_CACHE_MAX);
        assert!(!cache.contains_key(&(15, 0, 0)));
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn racing_vector_publication_returns_and_retains_the_canonical_cache_tile() {
        let key = (15, 0, 0);
        let decoded = Arc::new(Mutex::new(HashMap::new()));
        let clock = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let encoded = mvt::encode::tile("water", 4096, &[], &[], &[]);
        let make_tile = || {
            Arc::new(Some(SemanticTile::normalize_at(
                mvt::decode_tile(&encoded).unwrap(),
                key,
            )))
        };
        let first = make_tile();
        let first_identity = Arc::downgrade(&first);
        drop(ProtomapsSource::publish_prepared_tile(
            &decoded, &clock, key, first,
        ));
        let replacement = make_tile();
        let replacement_identity = Arc::downgrade(&replacement);
        let acquired = ProtomapsSource::publish_prepared_tile(&decoded, &clock, key, replacement);
        assert!(replacement_identity.upgrade().is_none());
        assert!(Arc::ptr_eq(&acquired, &first_identity.upgrade().unwrap()));
        let mut prepared = crate::async_prep::PreparedSources::default();
        prepared.retain(acquired);
        {
            let mut cache = decoded.lock().unwrap();
            ProtomapsSource::prune_decoded_completed_with_limits(&mut cache, 0, 0);
            let canonical = cache[&key].0.get().unwrap().as_ref().unwrap();
            assert!(Arc::ptr_eq(canonical, &first_identity.upgrade().unwrap()));
        }
        drop(prepared);
        ProtomapsSource::prune_decoded_completed_with_limits(&mut decoded.lock().unwrap(), 0, 0);
        assert!(decoded.lock().unwrap().is_empty());
        assert!(first_identity.upgrade().is_none());
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn prepared_vector_dependencies_survive_cache_pressure_until_projection() {
        async_runtime().block_on(async {
            use crate::async_prep::{AsyncSourcePreparer, PreparationCoverage};
            let payload = mvt::encode::tile("water", 4096, &[], &[], &[]);
            let (archive, tile_range) = vector_archive(Some(&payload));
            let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let range = AsyncVectorRange::new(archive, tile_range, attempts.clone());
            let source = async_vector_source(range, None).await;
            let (lat, lon) = tile_center(15, 9649, 12314);
            let request = vector_request(lat, lon);
            let prepared = source
                .prepare_retained_for_coverage(&request, PreparationCoverage::Global)
                .await;
            assert!(prepared.summary.complete());
            let fetched = attempts.load(std::sync::atomic::Ordering::SeqCst);
            {
                let mut cache = source.decoded.lock().unwrap();
                ProtomapsSource::prune_decoded_completed_with_limits(&mut cache, 0, 0);
                assert!(
                    !cache.is_empty(),
                    "an active plan owns its completed dependencies"
                );
            }
            assert!(source.features(lat, lon, 1.0, 1.0).complete);
            drop(prepared);
            let deadline = vearth_platform::time::Instant::now()
                + vearth_platform::time::Duration::from_secs(2);
            loop {
                let empty = {
                    let mut cache = source.decoded.lock().unwrap();
                    ProtomapsSource::prune_decoded_completed_with_limits(&mut cache, 0, 0);
                    cache.is_empty()
                };
                if empty {
                    break;
                }
                assert!(
                    vearth_platform::time::Instant::now() < deadline,
                    "completed plans must not pin the cache indefinitely"
                );
                vearth_platform::rt::task::yield_now().await;
            }
            assert!(!source.features(lat, lon, 1.0, 1.0).complete);
            assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), fetched);
        });
    }

    #[test]
    fn decoded_lru_enforces_measured_byte_limit_without_evicting_in_flight() {
        use crate::mvt::encode::FeatureSpec;

        let encoded = mvt::encode::tile(
            "roads",
            4096,
            &["kind"],
            &[Value::Str("major_road".into())],
            &[FeatureSpec {
                id: Some(1),
                geom_type: GeomType::Line,
                paths: vec![vec![[0, 0], [100, 100]]],
                tags: vec![(0, 0)],
            }],
        );
        let make_slot = |key| {
            let semantic = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), key);
            let bytes = semantic.estimated_heap_bytes();
            let slot = Arc::new(TileSlot::new());
            assert!(slot.set(Ok(Arc::new(Some(semantic)))).is_ok());
            (slot, bytes)
        };
        let (oldest, tile_bytes) = make_slot((15, 0, 0));
        let (newest, _) = make_slot((15, 1, 0));
        let inflight = Arc::new(TileSlot::new());
        let mut cache = HashMap::from([
            ((15, 0, 0), (oldest, 0)),
            ((15, 1, 0), (newest, 1)),
            ((15, 2, 0), (inflight.clone(), 2)),
        ]);

        ProtomapsSource::prune_decoded_completed_with_limits(&mut cache, 10, tile_bytes);
        assert!(!cache.contains_key(&(15, 0, 0)));
        assert!(cache.contains_key(&(15, 1, 0)));
        assert!(cache.contains_key(&(15, 2, 0)));
        assert!(inflight.get().is_none());
    }

    #[cfg(feature = "async-http")]
    fn varint(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    /// Minimal uncompressed archive. A payload is addressed by one broad run
    /// covering every z<=15 tile; None creates an empty root directory.
    #[cfg(feature = "async-http")]
    fn vector_archive(payload: Option<&[u8]>) -> (Vec<u8>, Option<(u64, u64)>) {
        let mut root = Vec::new();
        if let Some(payload) = payload {
            varint(&mut root, 1);
            varint(&mut root, 0); // first tile id
            varint(&mut root, u32::MAX as u64); // broad run
            varint(&mut root, payload.len() as u64);
            varint(&mut root, 1); // tile-data offset zero + one
        } else {
            varint(&mut root, 0);
        }

        let metadata = crate::protomaps::compatible_test_metadata_json();
        let root_offset = 127u64;
        let metadata_offset = root_offset + root.len() as u64;
        let tile_offset = metadata_offset + metadata.len() as u64;
        let tile_len = payload.map_or(0, |bytes| bytes.len() as u64);
        let mut archive = Vec::new();
        archive.extend_from_slice(b"PMTiles");
        archive.push(3);
        let mut u64le = |value: u64| archive.extend_from_slice(&value.to_le_bytes());
        u64le(root_offset);
        u64le(root.len() as u64);
        u64le(metadata_offset);
        u64le(metadata.len() as u64);
        u64le(tile_offset); // zero-length leaf section
        u64le(0);
        u64le(tile_offset);
        u64le(tile_len);
        u64le(u64::from(payload.is_some()));
        u64le(u64::from(payload.is_some()));
        u64le(u64::from(payload.is_some()));
        archive.push(1); // clustered
        archive.push(1); // internal compression: none
        archive.push(1); // tile compression: none
        archive.push(crate::pmtiles::TILE_TYPE_MVT);
        archive.push(0);
        archive.push(15);
        archive.resize(127, 0);
        archive.extend_from_slice(&root);
        archive.extend_from_slice(metadata.as_bytes());
        if let Some(payload) = payload {
            archive.extend_from_slice(payload);
        }
        (archive, payload.map(|_| (tile_offset, tile_len)))
    }

    #[cfg(feature = "async-http")]
    struct MemoryRange(Arc<Vec<u8>>);

    #[cfg(feature = "async-http")]
    impl RangeFetcher for MemoryRange {
        fn fetch_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, DataError> {
            let start = usize::try_from(offset).unwrap();
            let end = start + usize::try_from(len).unwrap();
            self.0
                .get(start..end)
                .map(|bytes| bytes.to_vec())
                .ok_or_else(|| DataError::Http("test range outside archive".into()))
        }
    }

    #[cfg(feature = "async-http")]
    struct AsyncVectorRange {
        bytes: Arc<Vec<u8>>,
        tile_range: Option<(u64, u64)>,
        tile_attempts: Arc<AtomicUsize>,
        metadata_range: (u64, u64),
        metadata_attempts: Arc<AtomicUsize>,
        delay: vearth_platform::time::Duration,
        fail_before: usize,
        started: Option<Arc<tokio::sync::Notify>>,
        release: Option<Arc<tokio::sync::Notify>>,
        observed_priority: Option<Arc<Mutex<Option<crate::async_http::RequestPriority>>>>,
    }

    #[cfg(feature = "async-http")]
    impl AsyncVectorRange {
        fn new(bytes: Vec<u8>, tile_range: Option<(u64, u64)>, attempts: Arc<AtomicUsize>) -> Self {
            let u64_at =
                |offset: usize| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
            let metadata_range = (u64_at(24), u64_at(32));
            Self {
                bytes: Arc::new(bytes),
                tile_range,
                tile_attempts: attempts,
                metadata_range,
                metadata_attempts: Arc::new(AtomicUsize::new(0)),
                delay: vearth_platform::time::Duration::ZERO,
                fail_before: 0,
                started: None,
                release: None,
                observed_priority: None,
            }
        }
    }

    #[cfg(feature = "async-http")]
    impl AsyncRangeFetcher for AsyncVectorRange {
        fn fetch_range(&self, offset: u64, len: u64) -> FetchFuture<'_> {
            let bytes = self.bytes.clone();
            let is_tile = self.tile_range == Some((offset, len));
            let is_metadata = self.metadata_range == (offset, len);
            let attempts = self.tile_attempts.clone();
            let metadata_attempts = self.metadata_attempts.clone();
            let delay = self.delay;
            let fail_before = self.fail_before;
            let started = self.started.clone();
            let release = self.release.clone();
            Box::pin(async move {
                if is_metadata {
                    metadata_attempts.fetch_add(1, Ordering::SeqCst);
                }
                if is_tile {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    if let Some(started) = started {
                        started.notify_one();
                    }
                    if let Some(release) = release {
                        release.notified().await;
                    }
                    if !delay.is_zero() {
                        vearth_platform::rt::time::sleep(delay).await;
                    }
                    if attempt < fail_before {
                        return Err(DataError::Http("transient vector range failure".into()));
                    }
                }
                let start = usize::try_from(offset).unwrap();
                let end = start + usize::try_from(len).unwrap();
                bytes
                    .get(start..end)
                    .map(|range| range.to_vec())
                    .ok_or_else(|| DataError::Http("test range outside archive".into()))
            })
        }

        fn fetch_range_with_priority(
            &self,
            offset: u64,
            len: u64,
            priority: crate::async_http::RequestPriority,
        ) -> FetchFuture<'_> {
            if self.tile_range == Some((offset, len))
                && let Some(observed) = &self.observed_priority
            {
                *observed.lock().unwrap() = Some(priority);
            }
            self.fetch_range(offset, len)
        }
    }

    #[cfg(feature = "async-http")]
    fn async_runtime() -> vearth_platform::rt::runtime::Runtime {
        vearth_platform::rt::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    fn tile_center(z: u8, x: u32, y: u32) -> (f64, f64) {
        let n = (1u64 << z) as f64;
        let lon = ((x as f64 + 0.5) / n * 2.0 - 1.0) * std::f64::consts::PI;
        let lat = (std::f64::consts::PI * (1.0 - 2.0 * (y as f64 + 0.5) / n))
            .sinh()
            .atan();
        (lat, lon)
    }

    #[cfg(feature = "async-http")]
    fn vector_request(lat: f64, lon: f64) -> PrepareRequest {
        PrepareRequest {
            terrain_tiles: Vec::new(),
            footprint: SourceFootprint {
                center_lat: lat,
                center_lon: lon,
                radius_rad: 1.0 / MEAN_RADIUS,
                sample_hint_m: 1.0,
            },
            vector_bounds: Some((lat, lon, 1.0, 1.0)),
            night_building_bounds: None,
            priority: 1.0,
        }
    }

    #[cfg(feature = "async-http")]
    async fn async_vector_source(
        range: AsyncVectorRange,
        source_cache: Option<SourceTileCache>,
    ) -> ProtomapsSource {
        let pm = PmTiles::open_async(Arc::new(range)).await.unwrap();
        ProtomapsSource::new_async(pm, "async-test", source_cache)
            .await
            .unwrap()
    }

    #[cfg(feature = "async-http")]
    fn night_vector_request(lat: f64, lon: f64) -> PrepareRequest {
        let mut request = vector_request(lat, lon);
        request.vector_bounds = None;
        request.night_building_bounds = Some((lat, lon, 1.0, 1.0));
        request
    }

    #[cfg(feature = "async-http")]
    fn test_vector_cache(root: &Path) -> SourceTileCache {
        let store = crate::source_cache::SourceTileStore::open(
            root.join("source_tiles.sqlite3"),
            8 * 1024 * 1024,
        )
        .unwrap();
        store
            .namespace(
                "vectors",
                "async-test",
                "application/vnd.mapbox-vector-tile",
                Some(LegacyLayout::Vectors {
                    root: root.to_path_buf(),
                    build_tag: "async-test".into(),
                }),
            )
            .unwrap()
    }

    #[cfg(feature = "async-http")]
    fn blocking_vector_source(payload: &[u8]) -> ProtomapsSource {
        let (archive, _) = vector_archive(Some(payload));
        let pm = PmTiles::open(Box::new(MemoryRange(Arc::new(archive)))).unwrap();
        ProtomapsSource::new(pm, "plan-test", None).unwrap()
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_location_names_share_validated_source_and_keep_tiles_through_eviction() {
        async_runtime().block_on(async {
            let payload = mvt::encode::tile(
                "places",
                4096,
                &["name", "kind"],
                &[Value::Str("Harbor Town".into()), Value::Str("town".into())],
                &[mvt::encode::FeatureSpec {
                    id: Some(42),
                    geom_type: GeomType::Point,
                    paths: vec![vec![[2048, 2048]]],
                    tags: vec![(0, 0), (1, 1)],
                }],
            );
            let (lat, lon) = tile_center(8, 131, 84);
            let expected = blocking_vector_source(&payload)
                .resolve_named_location(lat, lon)
                .unwrap();
            assert_eq!(
                expected.as_ref().unwrap().primary_name.as_deref(),
                Some("Harbor Town")
            );
            let (archive, tile_range) = vector_archive(Some(&payload));
            let attempts = Arc::new(AtomicUsize::new(0));
            let range = AsyncVectorRange::new(archive, tile_range, attempts.clone());
            let metadata_attempts = range.metadata_attempts.clone();
            let source = Arc::new(
                ProtomapsSource::open_async(Arc::new(range), "async-test", None)
                    .await
                    .unwrap(),
            );
            assert!(source.is_async());
            assert_eq!(metadata_attempts.load(Ordering::SeqCst), 1);
            let worker_source = source.clone();
            let first = vearth_platform::rt::spawn(async move {
                worker_source.resolve_named_location_async(lat, lon).await
            });
            let worker_source = source.clone();
            let second = vearth_platform::rt::spawn(async move {
                worker_source.resolve_named_location_async(lat, lon).await
            });
            assert_eq!(first.await.unwrap().unwrap(), expected);
            assert_eq!(second.await.unwrap().unwrap(), expected);
            assert_eq!(attempts.load(Ordering::SeqCst), 18);
            assert_eq!(
                source.resolve_named_location_async(lat, lon).await.unwrap(),
                expected
            );
            assert_eq!(attempts.load(Ordering::SeqCst), 18);

            source.decoded.lock().unwrap().clear();
            let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let pressure_source = source.clone();
            let pressure_done = done.clone();
            let pressure = vearth_platform::rt::spawn(async move {
                while !pressure_done.load(Ordering::SeqCst) {
                    ProtomapsSource::prune_decoded_completed_with_limits(
                        &mut pressure_source.decoded.lock().unwrap(),
                        0,
                        0,
                    );
                    vearth_platform::rt::task::yield_now().await;
                }
            });
            let actual = source.resolve_named_location_async(lat, lon).await;
            done.store(true, Ordering::SeqCst);
            pressure.await.unwrap();
            assert_eq!(actual.unwrap(), expected);
            assert_eq!(metadata_attempts.load(Ordering::SeqCst), 1);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_location_names_distinguish_absence_and_retry_failures() {
        async_runtime().block_on(async {
            let payload = mvt::encode::tile("earth", 4096, &[], &[], &[]);
            let (lat, lon) = tile_center(8, 131, 84);
            let (archive, tile_range) = vector_archive(Some(&payload));
            let attempts = Arc::new(AtomicUsize::new(0));
            let mut range = AsyncVectorRange::new(archive, tile_range, attempts.clone());
            range.fail_before = 18; // Nine context tiles and their single retries.
            let source = ProtomapsSource::open_async(Arc::new(range), "async-test", None)
                .await
                .unwrap();
            assert!(source.resolve_named_location_async(lat, lon).await.is_err());
            assert!(source.resolve_named_location_async(lat, lon).await.is_ok());
            assert!(attempts.load(Ordering::SeqCst) > 18);
            let (archive, tile_range) = vector_archive(None);
            let range =
                AsyncVectorRange::new(archive.clone(), tile_range, Arc::new(AtomicUsize::new(0)));
            let absent = ProtomapsSource::open_async(Arc::new(range), "async-test", None)
                .await
                .unwrap();
            let blocking = ProtomapsSource::new(
                PmTiles::open(Box::new(MemoryRange(Arc::new(archive)))).unwrap(),
                "async-test",
                None,
            )
            .unwrap();
            assert_eq!(
                absent.resolve_named_location_async(lat, lon).await.unwrap(),
                blocking.resolve_named_location(lat, lon).unwrap()
            );
            let (archive, tile_range) = vector_archive(Some(b"invalid mvt"));
            let broken = ProtomapsSource::open_async(
                Arc::new(AsyncVectorRange::new(
                    archive,
                    tile_range,
                    Arc::new(AtomicUsize::new(0)),
                )),
                "async-test",
                None,
            )
            .await
            .unwrap();
            assert!(broken.resolve_named_location_async(lat, lon).await.is_err());
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn vector_query_plan_separates_authorities_and_handles_edge_geography() {
        let payload = mvt::encode::tile("earth", 4096, &[], &[], &[]);
        let source = blocking_vector_source(&payload);
        let (fine_lat, fine_lon) = tile_center(15, 9649, 12314);
        let fine = source.terrain_query_plan(fine_lat, fine_lon, 1.0, 1.0);
        assert_eq!(
            fine,
            source.terrain_query_plan(fine_lat, fine_lon, 1.0, 1.0)
        );
        let detail = fine.query(VectorQueryKind::TerrainDetail).unwrap();
        let landcover = fine.query(VectorQueryKind::TerrainLandcover).unwrap();
        assert_eq!(detail.zoom, 15);
        assert_eq!(detail.authority, QueryAuthority::AdaptiveWindow);
        assert_eq!(landcover.zoom, 7);
        assert_eq!(landcover.layers, &["landcover"]);
        assert_eq!(landcover.authority, QueryAuthority::AuthoredLayerMaximum);
        assert_eq!(fine.unique_tile_keys().len(), 2);
        let farmland_only = blocking_vector_source(&payload)
            .with_regional_landcover_mode(RegionalLandcoverMode::ExistingFarmlandOnly);
        assert_eq!(
            farmland_only.terrain_query_plan(fine_lat, fine_lon, 1.0, 1.0),
            fine,
            "rollback mode must retain the accepted independent z7 query"
        );
        assert_ne!(
            farmland_only.terrain_dataset_identity_tag(),
            source.terrain_dataset_identity_tag()
        );
        let legacy_water = blocking_vector_source(&payload)
            .with_water_polygon_mode(WaterPolygonMode::LegacyUndifferentiated);
        assert_eq!(
            legacy_water.terrain_query_plan(fine_lat, fine_lon, 1.0, 1.0),
            fine,
            "water rollback must not change query authority"
        );
        assert_ne!(
            legacy_water.terrain_dataset_identity_tag(),
            source.terrain_dataset_identity_tag()
        );
        let line_rollback = blocking_vector_source(&payload)
            .with_watercourse_output_mode(WatercourseOutputMode::DisabledG96);
        assert_eq!(
            line_rollback.terrain_query_plan(fine_lat, fine_lon, 1.0, 1.0),
            fine,
            "line rollback must retain the accepted detail query authority"
        );
        assert_ne!(
            line_rollback.terrain_dataset_identity_tag(),
            source.terrain_dataset_identity_tag()
        );

        let overlapping = source.terrain_query_plan(fine_lat, fine_lon, 100_000.0, 100_000.0);
        assert_eq!(
            overlapping
                .query(VectorQueryKind::TerrainDetail)
                .unwrap()
                .zoom,
            7
        );
        let planned_count: usize = overlapping
            .queries
            .iter()
            .map(|query| query.tiles.len())
            .sum();
        assert!(overlapping.unique_tile_keys().len() < planned_count);

        let coarse = source.terrain_query_plan(0.0, 0.0, 5_000_000.0, 5_000_000.0);
        assert!(
            coarse
                .query(VectorQueryKind::TerrainLandcover)
                .unwrap()
                .capped
        );
        assert!(!coarse.required_complete());

        let wrapped =
            source.terrain_query_plan(0.0, std::f64::consts::PI - 0.001, 200_000.0, 1_000.0);
        let wrapped_landcover = wrapped.query(VectorQueryKind::TerrainLandcover).unwrap();
        assert!(wrapped_landcover.tiles.iter().any(|&(_, x, _)| x == 0));
        assert!(wrapped_landcover.tiles.iter().any(|&(_, x, _)| x == 127));

        let high_latitude = source.terrain_query_plan(1.47, 0.0, 1_000.0, 1_000.0);
        assert!(
            high_latitude
                .queries
                .iter()
                .all(|query| query.tiles.iter().all(|&(z, _, y)| y < (1u32 << z)))
        );

        let location = source.location_query_plan(10, 0.5, 0.5);
        let context = location.query(VectorQueryKind::LocationContext).unwrap();
        assert_eq!(context.completeness, QueryCompleteness::Optional);
        assert_eq!(context.tiles.len(), 9);
        assert!(location.required_complete());
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn plans_clamp_to_archive_zoom_and_preparation_matches_consumption() {
        let payload = mvt::encode::tile("earth", 4096, &[], &[], &[]);
        let mut source = blocking_vector_source(&payload);
        Arc::get_mut(&mut source.archive).unwrap().header.max_zoom = 12;
        let terrain = source.terrain_query_plan(0.0, 0.0, 1.0, 1.0);
        assert_eq!(
            terrain.query(VectorQueryKind::TerrainDetail).unwrap().zoom,
            12
        );
        assert_eq!(
            terrain
                .query(VectorQueryKind::TerrainLandcover)
                .unwrap()
                .zoom,
            7
        );
        assert_eq!(
            source
                .night_building_query_plan(0.0, 0.0, 1.0, 1.0)
                .query(VectorQueryKind::NightBuildings)
                .unwrap()
                .zoom,
            12
        );
        let request = vector_request(0.0, 0.0);
        assert_eq!(source.preparation_query_plan(&request), terrain);
    }

    #[cfg(feature = "async-http")]
    fn adaptive_bridge_payload() -> Vec<u8> {
        use crate::mvt::encode::FeatureSpec;
        mvt::encode::tile(
            "roads",
            4096,
            &["kind", "is_bridge", "is_tunnel"],
            &[Value::Str("minor_road".into()), Value::Bool(true)],
            &[
                FeatureSpec {
                    id: Some(1),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[2016, 2048], [2080, 2048]]],
                    tags: vec![(0, 0), (1, 1)],
                },
                FeatureSpec {
                    id: Some(2),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[2080, 2048], [2300, 2048]]],
                    tags: vec![(0, 0)],
                },
                FeatureSpec {
                    id: Some(3),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[2016, 2200], [2080, 2200]]],
                    tags: vec![(0, 0), (2, 1)],
                },
            ],
        )
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn adaptive_bridge_preparation_uses_only_shared_road_query_at_coarse_and_near_scales() {
        async_runtime().block_on(async {
            let payload = adaptive_bridge_payload();
            let (lat, lon) = tile_center(12, 1206, 1539);
            for half_width in [2_000.0, 40.0] {
                let (archive, tile_range) = vector_archive(Some(&payload));
                let attempts = Arc::new(AtomicUsize::new(0));
                let source = async_vector_source(
                    AsyncVectorRange::new(archive, tile_range, attempts.clone()),
                    None,
                )
                .await;
                let mut request = vector_request(lat, lon);
                request.vector_bounds = Some((lat, lon, half_width, half_width));
                let plan = source.preparation_query_plan(&request);
                assert_eq!(plan.queries.len(), 2);
                let detail = plan.query(VectorQueryKind::TerrainDetail).unwrap();
                assert_eq!(detail.authority, QueryAuthority::AdaptiveWindow);
                assert!(detail.layers.contains(&"roads"));
                if half_width > 1_000.0 {
                    assert!(detail.zoom < 15);
                } else {
                    assert_eq!(detail.zoom, 15);
                }
                let expected = plan.unique_tile_keys();
                assert!(!source.features(lat, lon, half_width, half_width).complete);
                assert_eq!(attempts.load(Ordering::SeqCst), 0);
                let prepared = source
                    .prepare_retained_for_coverage(
                        &request,
                        crate::async_prep::PreparationCoverage::Global,
                    )
                    .await;
                assert!(prepared.summary.complete());
                assert_eq!(prepared.summary.ready as usize, expected.len());
                assert_eq!(attempts.load(Ordering::SeqCst), expected.len());
                let before = attempts.load(Ordering::SeqCst);
                let pack = source.features(lat, lon, half_width, half_width);
                assert!(
                    pack.complete,
                    "adaptive bridge data is a complete road source"
                );
                assert!(pack.roads.iter().any(|road| road.is_bridge));
                assert!(pack.roads.iter().any(|road| !road.is_bridge));
                assert_eq!(
                    pack.roads.len(),
                    detail.tiles.len() * 2,
                    "tunnels stay excluded"
                );
                assert_eq!(attempts.load(Ordering::SeqCst), before);
                let mut actual: Vec<_> = source.decoded.lock().unwrap().keys().copied().collect();
                actual.sort_unstable();
                assert_eq!(
                    actual, expected,
                    "bridge flags must not discover extra source keys"
                );

                let blocking = blocking_vector_source(&payload);
                blocking.prefetch_vectors(lat, lon, half_width, half_width);
                let before = blocking.normalization_count();
                let reference = blocking.features(lat, lon, half_width, half_width);
                assert_eq!(blocking.normalization_count(), before);
                let paths = |roads: &[Road]| {
                    roads
                        .iter()
                        .map(|road| (road.class, road.is_bridge, road.pts.clone()))
                        .collect::<Vec<_>>()
                };
                assert_eq!(paths(&pack.roads), paths(&reference.roads));
            }
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn adaptive_bridge_identity_changes_generated_output_without_invalidating_raw_tiles() {
        async_runtime().block_on(async {
            static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "vearth-adaptive-bridge-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            let cache = test_vector_cache(&dir);
            let payload = adaptive_bridge_payload();
            let (archive, tile_range) = vector_archive(Some(&payload));
            let attempts = Arc::new(AtomicUsize::new(0));
            let source = async_vector_source(
                AsyncVectorRange::new(archive, tile_range, attempts.clone()),
                Some(cache.clone()),
            )
            .await;
            let identity = source.terrain_dataset_identity_tag();
            assert!(identity.ends_with("-bridge-roads-adaptive-v1"));
            assert_ne!(
                identity,
                identity.replace("bridge-roads-adaptive-v1", "bridge-roads-fixed-v1")
            );
            assert_eq!(
                source.source_cache.as_ref().unwrap().revision(),
                "async-test"
            );
            let (lat, lon) = tile_center(12, 1206, 1539);
            let mut request = vector_request(lat, lon);
            request.vector_bounds = Some((lat, lon, 2_000.0, 2_000.0));
            let keys = source.preparation_query_plan(&request).unique_tile_keys();
            for &(z, x, y) in &keys {
                assert!(cache.store_present(z, x, y, payload.clone()));
            }
            assert!(cache.flush());
            assert!(source.prepare(&request).await.complete());
            assert_eq!(
                attempts.load(Ordering::SeqCst),
                0,
                "the existing raw namespace remains reusable"
            );
            assert!(source.features(lat, lon, 2_000.0, 2_000.0).complete);
            for (z, x, y) in keys {
                assert_eq!(
                    cache.load(z, x, y).unwrap(),
                    Some(CachedTile::Present(payload.clone()))
                );
            }
            drop(source);
            drop(cache);
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[test]
    #[ignore = "read-only pinned source check; set VEARTH_BRIDGE_SOURCE_CACHE"]
    fn adaptive_bridge_cached_rotterdam_source_preserves_crossing_semantics() {
        let path = vearth_platform::env::var("VEARTH_BRIDGE_SOURCE_CACHE").unwrap();
        let db =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let namespace: i64 = db
            .query_row(
                "SELECT id FROM source_namespaces WHERE provider='vectors' AND revision='20260907'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let origin = (51.9225_f64.to_radians(), 4.47917_f64.to_radians());
        // Hashes identify unchanged raw MVT, not a re-encoded semantic fixture.
        for (x, y, hash, source_id, name) in [
            (
                2098_u32,
                1354_u32,
                0xece4_5652_d948_79e8_u64,
                35_184_417_751_210_u64,
                "Erasmusbrug",
            ),
            (
                2099,
                1354,
                0x40f5_20e5_d73b_9f18,
                35_184_379_610_200,
                "Willemsbrug",
            ),
        ] {
            let payload: Vec<u8> = db.query_row(
                "SELECT payload FROM source_tiles WHERE namespace_id=?1 AND z=12 AND x=?2 AND y=?3 AND state=1",
                rusqlite::params![namespace,x,y], |row| row.get(0),
            ).unwrap();
            let actual_hash = payload
                .iter()
                .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
                });
            assert_eq!(actual_hash, hash);
            let decoded = mvt::decode_tile(&payload).unwrap();
            let layer = decoded.layer("roads").unwrap();
            let feature = layer
                .features
                .iter()
                .find(|feature| feature.id == Some(source_id))
                .unwrap();
            assert_eq!(feature.get(layer, "name"), Some(&Value::Str(name.into())));
            assert_eq!(feature.get(layer, "is_bridge"), Some(&Value::Bool(true)));
            let projection = TileProj {
                z: 12,
                x,
                y,
                extent: f64::from(layer.extent),
                lat0: origin.0,
                lon0: origin.1,
            };
            let expected = projection.paths_to_en(&feature.paths);
            let tile = SemanticTile::normalize_at(decoded, (12, x, y));
            let mut roads = Vec::new();
            append_semantic_roads(
                &mut roads,
                &tile,
                (12, x, y),
                origin,
                &mut SemanticConsumerReport::default(),
            );
            for path in expected {
                assert!(roads.iter().any(|road| road.is_bridge
                    && road.class == RoadClass::Major
                    && road.pts == path));
            }
            assert!(roads.iter().any(|road| !road.is_bridge));
            println!(
                "tile=12/{x}/{y} bridge={name} id={source_id} roads={} bridges={}",
                roads.len(),
                roads.iter().filter(|road| road.is_bridge).count()
            );
        }
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn fine_terrain_query_reaches_authored_low_zoom_landcover() {
        use crate::mvt::encode::FeatureSpec;

        let payload = mvt::encode::tile(
            "landcover",
            4096,
            &["kind"],
            &[Value::Str("farmland".into())],
            &[FeatureSpec {
                id: Some(55),
                geom_type: GeomType::Polygon,
                paths: vec![vec![[100, 100], [300, 100], [300, 300], [100, 300]]],
                tags: vec![(0, 0)],
            }],
        );
        let source = blocking_vector_source(&payload);
        let (lat, lon) = tile_center(15, 9649, 12314);
        let pack = source.features(lat, lon, 1.0, 1.0);
        assert!(pack.complete);
        assert!(pack.flora.is_empty());
        assert_eq!(pack.regional_landcover.len(), 1);
        assert_eq!(pack.regional_landcover[0].kind, LandcoverKind::Farmland);
        assert_eq!(pack.regional_landcover[0].source_id, Some(55));
        assert_eq!(
            pack.regional_landcover_mode,
            RegionalLandcoverMode::SelectiveRegionalTerrain
        );
        assert!(pack.regional_landcover_projected_bytes() < MAX_REGIONAL_LANDCOVER_PROJECTED_BYTES);
        let keys: Vec<_> = source.decoded.lock().unwrap().keys().copied().collect();
        assert!(keys.iter().any(|&(z, _, _)| z == 7));
        assert!(keys.iter().any(|&(z, _, _)| z == 15));
        assert_eq!(source.normalization_count(), 2);
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_preparation_matches_blocking_and_sampling_is_cache_only() {
        async_runtime().block_on(async {
            use crate::mvt::encode::{FeatureSpec, LayerSpec};

            let water_values = [Value::Str("river".into()), Value::Str("yes".into())];
            let water_features = [
                FeatureSpec {
                    id: Some(1),
                    geom_type: GeomType::Polygon,
                    paths: vec![vec![[100, 100], [300, 100], [300, 300], [100, 300]]],
                    tags: vec![(0, 0)],
                },
                FeatureSpec {
                    id: Some(2),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[100, 400], [300, 500]]],
                    tags: vec![(0, 0), (1, 1)],
                },
                FeatureSpec {
                    id: Some(3),
                    geom_type: GeomType::Point,
                    paths: vec![vec![[200, 200]]],
                    tags: vec![(0, 0)],
                },
                FeatureSpec {
                    id: Some(4),
                    geom_type: GeomType::Line,
                    paths: vec![vec![[1500, 2048], [2600, 2048]]],
                    tags: vec![(0, 0)],
                },
            ];
            let earth_values = [Value::Str("cliff".into())];
            let earth_features = [FeatureSpec {
                id: Some(5),
                geom_type: GeomType::Line,
                paths: vec![vec![[1500, 1900], [2050, 2050], [2600, 2200]]],
                tags: vec![(0, 0)],
            }];
            let payload = mvt::encode::tile_layers(&[
                LayerSpec {
                    name: "water",
                    extent: 4096,
                    keys: &["kind", "tunnel"],
                    values: &water_values,
                    features: &water_features,
                },
                LayerSpec {
                    name: "earth",
                    extent: 4096,
                    keys: &["kind"],
                    values: &earth_values,
                    features: &earth_features,
                },
            ]);
            let (archive, tile_range) = vector_archive(Some(&payload));
            let blocking_pm =
                PmTiles::open(Box::new(MemoryRange(Arc::new(archive.clone())))).unwrap();
            let blocking = ProtomapsSource::new(blocking_pm, "async-test", None).unwrap();
            blocking.set_semantic_observability(true);
            let attempts = Arc::new(AtomicUsize::new(0));
            let range = AsyncVectorRange::new(archive.clone(), tile_range, attempts.clone());
            let metadata_attempts = range.metadata_attempts.clone();
            let source = async_vector_source(range, None).await;
            let terrain_identity = source.terrain_dataset_identity_tag();
            source.set_semantic_observability(true);
            assert_eq!(source.terrain_dataset_identity_tag(), terrain_identity);
            assert_eq!(metadata_attempts.load(Ordering::SeqCst), 1);
            assert_eq!(source.source_descriptor(), blocking.source_descriptor());
            assert_eq!(
                source.terrain_dataset_identity_tag(),
                blocking.terrain_dataset_identity_tag()
            );
            assert_eq!(
                source.diagnostic_identity_tag(),
                blocking.diagnostic_identity_tag()
            );
            let (lat, lon) = tile_center(15, 9649, 12314);
            let request = vector_request(lat, lon);
            assert_eq!(
                source
                    .terrain_query_plan(lat, lon, 1.0, 1.0)
                    .query(VectorQueryKind::TerrainDetail)
                    .unwrap()
                    .tiles
                    .len(),
                1
            );

            let cold = source.features(lat, lon, 1.0, 1.0);
            assert!(!cold.complete);
            assert_eq!(attempts.load(Ordering::SeqCst), 0);
            assert_eq!(source.normalization_count(), 0);
            let summary = source.prepare(&request).await;
            assert_eq!(summary.ready, 2);
            assert!(summary.complete());
            assert_eq!(source.normalization_count(), 2);
            assert_eq!(
                source
                    .semantic_report()
                    .tile_outcome(SemanticTileOutcome::Present),
                2
            );
            let prepared = source.features(lat, lon, 1.0, 1.0);
            let legacy = blocking.features(lat, lon, 1.0, 1.0);
            assert_eq!(source.normalization_count(), 2);
            assert!(prepared.complete && legacy.complete);
            assert_eq!(prepared.water.len(), 1);
            assert_eq!(legacy.water.len(), 1);
            assert_eq!(prepared.water[0].is_ocean(), legacy.water[0].is_ocean());
            assert_eq!(prepared.water[0].rings, legacy.water[0].rings);
            assert_eq!(prepared.watercourses, legacy.watercourses);
            assert_eq!(prepared.cliffs, legacy.cliffs);
            assert_eq!(prepared.cliffs.len(), 1);
            assert_eq!(
                prepared.cliffs[0].identity,
                SourceFeatureIdentity::Provided(5)
            );
            assert_eq!(prepared.watercourses.len(), 1);
            assert_eq!(prepared.watercourses[0].kind, WatercourseKind::River);
            assert_eq!(
                prepared.watercourses[0].identity,
                SourceFeatureIdentity::Provided(4)
            );
            let assert_water_report =
                |report: crate::protomaps_observability::SemanticReportSnapshot| {
                    assert_eq!(report.layer_consumed(SemanticLayerFamily::Water), 2);
                    assert_eq!(report.layer_discarded(SemanticLayerFamily::Water), 2);
                    assert_eq!(
                        report.discard_count(
                            SemanticLayerFamily::Water,
                            SemanticDiscardReason::SurfaceIneligible,
                        ),
                        1
                    );
                    assert_eq!(
                        report.discard_count(
                            SemanticLayerFamily::Water,
                            SemanticDiscardReason::ContextOnly,
                        ),
                        1
                    );
                };
            assert_water_report(source.semantic_report());
            assert_water_report(blocking.semantic_report());
            let sorted_keys = |source: &ProtomapsSource| {
                let mut keys: Vec<_> = source.decoded.lock().unwrap().keys().copied().collect();
                keys.sort_unstable();
                keys
            };
            assert_eq!(sorted_keys(&source), sorted_keys(&blocking));
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
            source.prefetch_vectors(lat, lon, 1.0, 1.0);
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
            assert_eq!(source.normalization_count(), 2);
            assert_eq!(metadata_attempts.load(Ordering::SeqCst), 1);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn corrupt_vector_payload_is_transient_and_never_published_as_no_data() {
        async_runtime().block_on(async {
            let (archive, tile_range) = vector_archive(Some(&[0xff]));
            let blocking_pm =
                PmTiles::open(Box::new(MemoryRange(Arc::new(archive.clone())))).unwrap();
            let blocking = ProtomapsSource::new(blocking_pm, "blocking-corrupt", None).unwrap();
            let attempts = Arc::new(AtomicUsize::new(0));
            let range = AsyncVectorRange::new(archive, tile_range, attempts.clone());
            let source = async_vector_source(range, None).await;
            source.set_semantic_observability(true);
            let (lat, lon) = tile_center(15, 9649, 12314);
            let request = vector_request(lat, lon);

            let first = source.prepare(&request).await;
            assert_eq!(first.transient_failures, 2);
            assert_eq!(first.no_data, 0);
            assert!(!first.complete());
            assert!(!source.features(lat, lon, 1.0, 1.0).complete);
            assert!(!blocking.features(lat, lon, 1.0, 1.0).complete);

            let second = source.prepare(&request).await;
            assert_eq!(second.transient_failures, 2);
            assert_eq!(attempts.load(Ordering::SeqCst), 4);
            assert_eq!(source.normalization_count(), 0);
            assert_eq!(
                source
                    .semantic_report()
                    .tile_outcome(SemanticTileOutcome::DecodeFailure),
                4
            );
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn corrupt_vector_disk_entry_is_removed_and_refetched() {
        async_runtime().block_on(async {
            static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
            let payload = mvt::encode::tile("land", 4096, &[], &[], &[]);
            let (archive, tile_range) = vector_archive(Some(&payload));
            let attempts = Arc::new(AtomicUsize::new(0));
            let range = AsyncVectorRange::new(archive, tile_range, attempts.clone());
            let dir = std::env::temp_dir().join(format!(
                "vearth-vector-corrupt-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            let path = dir.join("async-test/14/4824_6157.mvt");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, [0xff]).unwrap();

            let cache = test_vector_cache(&dir);
            let source = async_vector_source(range, Some(cache.clone())).await;
            let (lat, lon) = tile_center(15, 9649, 12314);
            let summary = source.prepare(&night_vector_request(lat, lon)).await;
            assert_eq!(summary.ready, 1);
            assert!(summary.complete());
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            assert!(cache.flush());
            assert_eq!(
                cache.load(14, 4824, 6157).unwrap(),
                Some(CachedTile::Present(payload))
            );
            assert!(!path.exists());
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_no_data_writes_zero_byte_sentinel_and_complete_pack() {
        async_runtime().block_on(async {
            static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
            let (archive, tile_range) = vector_archive(None);
            let attempts = Arc::new(AtomicUsize::new(0));
            let range = AsyncVectorRange::new(archive.clone(), tile_range, attempts.clone());
            let dir = std::env::temp_dir().join(format!(
                "vearth-vector-async-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            let cache = test_vector_cache(&dir);
            let source = async_vector_source(range, Some(cache.clone())).await;
            source.set_semantic_observability(true);
            let (lat, lon) = tile_center(15, 9649, 12314);
            let request = vector_request(lat, lon);
            let plan = source.terrain_query_plan(lat, lon, 1.0, 1.0);
            let detail = plan.query(VectorQueryKind::TerrainDetail).unwrap();
            assert_eq!(detail.tiles.len(), 1);

            let summary = source.prepare(&request).await;
            assert_eq!(summary.no_data, 2);
            let (z, x, y) = detail.tiles[0];
            assert!(cache.flush());
            assert_eq!(cache.load(z, x, y).unwrap(), Some(CachedTile::Missing));
            assert!(source.features(lat, lon, 1.0, 1.0).complete);
            assert_eq!(attempts.load(Ordering::SeqCst), 0);
            assert_eq!(source.normalization_count(), 0);
            assert_eq!(
                source
                    .semantic_report()
                    .tile_outcome(SemanticTileOutcome::Absent),
                2
            );
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn concurrent_async_vector_preparations_share_one_tile_read() {
        async_runtime().block_on(async {
            let payload = mvt::encode::tile("land", 4096, &[], &[], &[]);
            let (archive, tile_range) = vector_archive(Some(&payload));
            let attempts = Arc::new(AtomicUsize::new(0));
            let mut range = AsyncVectorRange::new(archive.clone(), tile_range, attempts.clone());
            range.delay = vearth_platform::time::Duration::from_millis(75);
            let source = Arc::new(async_vector_source(range, None).await);
            let (lat, lon) = tile_center(15, 9649, 12314);
            let request = Arc::new(vector_request(lat, lon));

            let first = {
                let source = source.clone();
                let request = request.clone();
                vearth_platform::rt::spawn(async move { source.prepare(&request).await })
            };
            let second = {
                let source = source.clone();
                let request = request.clone();
                vearth_platform::rt::spawn(async move { source.prepare(&request).await })
            };
            assert_eq!(first.await.unwrap().ready, 2);
            assert_eq!(second.await.unwrap().ready, 2);
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
            assert_eq!(source.normalization_count(), 2);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn canceling_vector_prepare_waiter_preserves_shared_fill() {
        async_runtime().block_on(async {
            let payload = mvt::encode::tile("land", 4096, &[], &[], &[]);
            let (archive, tile_range) = vector_archive(Some(&payload));
            let attempts = Arc::new(AtomicUsize::new(0));
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let mut range = AsyncVectorRange::new(archive.clone(), tile_range, attempts.clone());
            range.started = Some(started.clone());
            range.release = Some(release.clone());
            let source = Arc::new(async_vector_source(range, None).await);
            let (lat, lon) = tile_center(15, 9649, 12314);
            let request = Arc::new(night_vector_request(lat, lon));

            let first = {
                let source = source.clone();
                let request = request.clone();
                vearth_platform::rt::spawn(async move { source.prepare(&request).await })
            };
            started.notified().await;
            first.abort();
            assert!(first.await.unwrap_err().is_cancelled());
            let second = {
                let source = source.clone();
                let request = request.clone();
                vearth_platform::rt::spawn(async move { source.prepare(&request).await })
            };
            release.notify_one();
            assert_eq!(second.await.unwrap().ready, 1);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            assert_eq!(source.normalization_count(), 1);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn failed_vector_single_flight_does_not_poison_later_prepare() {
        async_runtime().block_on(async {
            let payload = mvt::encode::tile("land", 4096, &[], &[], &[]);
            let (archive, tile_range) = vector_archive(Some(&payload));
            let attempts = Arc::new(AtomicUsize::new(0));
            let mut range = AsyncVectorRange::new(archive.clone(), tile_range, attempts.clone());
            range.fail_before = 2; // initial request and its one retry
            let source = async_vector_source(range, None).await;
            source.set_semantic_observability(true);
            let (lat, lon) = tile_center(15, 9649, 12314);
            let request = night_vector_request(lat, lon);

            let failed = source.prepare(&request).await;
            assert_eq!(failed.transient_failures, 1);
            assert!(!failed.complete());
            let recovered = source.prepare(&request).await;
            assert_eq!(recovered.ready, 1);
            assert!(recovered.complete());
            assert_eq!(attempts.load(Ordering::SeqCst), 3);
            assert_eq!(source.normalization_count(), 1);
            let report = source.semantic_report();
            assert_eq!(report.tile_outcome(SemanticTileOutcome::FetchFailure), 1);
            assert_eq!(report.tile_outcome(SemanticTileOutcome::Present), 1);
        });
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn vector_flight_exposes_max_active_waiter_priority() {
        async_runtime().block_on(async {
            let payload = mvt::encode::tile("land", 4096, &[], &[], &[]);
            let (archive, tile_range) = vector_archive(Some(&payload));
            let attempts = Arc::new(AtomicUsize::new(0));
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let observed = Arc::new(Mutex::new(None));
            let mut range = AsyncVectorRange::new(archive, tile_range, attempts.clone());
            range.started = Some(started.clone());
            range.release = Some(release.clone());
            range.observed_priority = Some(observed.clone());
            let source = Arc::new(async_vector_source(range, None).await);
            let (lat, lon) = tile_center(15, 9649, 12314);
            let mut low_request = night_vector_request(lat, lon);
            low_request.priority = 1.0;
            let low_request = Arc::new(low_request);

            let low = {
                let source = source.clone();
                let request = low_request.clone();
                vearth_platform::rt::spawn(async move { source.prepare(&request).await })
            };
            started.notified().await;
            let mut high_request = night_vector_request(lat, lon);
            high_request.priority = 10.0;
            let mut high = Box::pin(source.prepare(&high_request));
            std::future::poll_fn(|cx| match high.as_mut().poll(cx) {
                std::task::Poll::Pending => std::task::Poll::Ready(()),
                std::task::Poll::Ready(_) => {
                    panic!("shared vector flight unexpectedly completed while gated")
                }
            })
            .await;
            let priority = observed.lock().unwrap().as_ref().unwrap().clone();
            assert_eq!(priority.current(), 10.0);
            drop(high);
            assert_eq!(priority.current(), 1.0);
            release.notify_one();
            assert_eq!(low.await.unwrap().ready, 1);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        });
    }

    /// Live network test — `cargo test -- --ignored`. Full stack: hosted
    /// block reads → PMTiles → MVT decode → EN projection.
    #[test]
    #[ignore]
    fn live_manhattan_features() {
        let tag = PROTOMAPS_BUILD.to_string();
        let url = protomaps_build_url(&tag);
        let pm = PmTiles::open(Box::new(crate::pmtiles::HttpRange::new(&url))).unwrap();
        assert_eq!(pm.header.tile_type, crate::pmtiles::TILE_TYPE_MVT);
        let src = ProtomapsSource::new(pm, tag, None).unwrap();
        // Times Square, ~a fine-cell-sized box (±150 m).
        let (lat, lon) = (40.7580f64.to_radians(), -73.9855f64.to_radians());
        let pack = src.features(lat, lon, 150.0, 150.0);
        assert!(
            pack.buildings.len() > 50,
            "buildings: {}",
            pack.buildings.len()
        );
        assert!(pack.roads.len() > 5, "roads: {}", pack.roads.len());
        let tallest = pack
            .buildings
            .iter()
            .map(|b| b.height_m)
            .fold(0.0f32, f32::max);
        assert!(
            tallest > 100.0,
            "midtown should have skyscrapers: {tallest}"
        );
        // Geometry must be near the query origin (projection sanity).
        let b = &pack.buildings[0];
        assert!(
            b.rings[0]
                .iter()
                .all(|p| p[0].abs() < 2000.0 && p[1].abs() < 2000.0),
            "vertices near origin: {:?}",
            &b.rings[0][..2.min(b.rings[0].len())]
        );
        // A water query over the Hudson.
        let hudson = src.features(lat, (-74.01f64).to_radians(), 300.0, 300.0);
        assert!(!hudson.water.is_empty(), "Hudson river water polygons");
    }

    /// Live location-title test — `cargo test live_location_names -- --ignored`.
    #[test]
    #[ignore]
    fn live_location_names() {
        let tag = PROTOMAPS_BUILD.to_string();
        let url = protomaps_build_url(&tag);
        let pm = PmTiles::open(Box::new(crate::pmtiles::HttpRange::new(&url))).unwrap();
        let source = ProtomapsSource::new(pm, tag, None).unwrap();
        for (expected, lat, lon) in [
            ("Sydney, New South Wales", -33.8688_f64, 151.2093_f64),
            ("Yosemite National Park, California", 37.7459, -119.5936),
            ("Death Valley National Park, California", 36.4617, -116.8668),
            ("El Paso, Texas", 31.7619, -106.4850),
            ("Gorno-Badakhshan", 39.1463, 73.4603),
        ] {
            let location = source
                .resolve_named_location(lat.to_radians(), lon.to_radians())
                .unwrap_or_else(|error| panic!("{expected}: {error}"))
                .unwrap_or_else(|| panic!("{expected}: no location"));
            assert_eq!(location.name, expected, "{location:?}");
        }
    }
}
