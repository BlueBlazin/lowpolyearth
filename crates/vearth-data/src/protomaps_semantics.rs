//! Source-preserving semantic normalization for decoded Protomaps MVT tiles.
//!
//! Geometry and raw property tables remain owned by the decoded tile. Typed
//! facts store compact indices into that source truth, so normalization does
//! not duplicate paths or strings and never erases the original MVT value
//! type. Consumer projection, clipping, precedence, and presentation stay out
//! of this tile-oriented boundary.

use crate::mvt::{self, GeomType, Value};
use std::collections::HashMap;

/// In-memory normalized-fact schema. Semantic tiles are rebuilt from immutable
/// raw MVT on every process start.
pub const SEMANTIC_SCHEMA_VERSION: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceValueRef {
    value_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NormalizedValue {
    Text,
    UnknownText,
    Number(f64),
    Boolean(bool),
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceFact {
    pub source: SourceValueRef,
    pub normalized: NormalizedValue,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NameFact {
    pub key_index: u32,
    pub role: NameRole,
    pub value: SourceFact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameRole {
    Local,
    Localized,
    Preferred,
    Script,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceFeatureIdentity {
    Provided(u64),
    DeterministicFallback(u64),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommonFacts {
    pub min_zoom: Option<SourceFact>,
    pub sort_rank: Option<SourceFact>,
    pub population_rank: Option<SourceFact>,
    pub sort_key: Option<SourceFact>,
    pub place: Option<SourceFact>,
    pub population: Option<SourceFact>,
    pub names: Vec<NameFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoundaryFacts {
    pub kind: Option<SourceFact>,
    pub kind_detail: Option<SourceFact>,
    pub disputed: Option<SourceFact>,
    pub brk_a3: Option<SourceFact>,
    pub maritime: Option<SourceFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuildingPhysicalFacts {
    pub kind: Option<SourceFact>,
    pub kind_detail: Option<SourceFact>,
    pub height: Option<SourceFact>,
    pub min_height: Option<SourceFact>,
    pub layer: Option<SourceFact>,
    pub roof_shape: Option<SourceFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuildingContextFacts {
    pub addr_housenumber: Option<SourceFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuildingFacts {
    pub physical: BuildingPhysicalFacts,
    pub context: BuildingContextFacts,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EarthFacts {
    pub kind: Option<SourceFact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LandcoverKind {
    Barren,
    Farmland,
    Forest,
    Glacier,
    Grassland,
    Scrub,
    UrbanArea,
    Unknown,
}

impl LandcoverKind {
    pub fn from_source(value: &str) -> Self {
        match value {
            "barren" => Self::Barren,
            "farmland" => Self::Farmland,
            "forest" => Self::Forest,
            "glacier" => Self::Glacier,
            "grassland" => Self::Grassland,
            "scrub" => Self::Scrub,
            "urban_area" => Self::UrbanArea,
            _ => Self::Unknown,
        }
    }

    pub fn known_source(value: &str) -> Option<&'static str> {
        match value {
            "barren" => Some("barren"),
            "farmland" => Some("farmland"),
            "forest" => Some("forest"),
            "glacier" => Some("glacier"),
            "grassland" => Some("grassland"),
            "scrub" => Some("scrub"),
            "urban_area" => Some("urban_area"),
            _ => None,
        }
    }
}

pub const KNOWN_LANDCOVER_KINDS: &[&str] = &[
    "barren",
    "farmland",
    "forest",
    "glacier",
    "grassland",
    "scrub",
    "urban_area",
];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LandcoverFacts {
    pub kind: Option<SourceFact>,
    pub class: Option<LandcoverKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LanduseKind {
    Aerodrome,
    Airfield,
    Allotments,
    BareRock,
    Beach,
    Cemetery,
    College,
    Commercial,
    Dam,
    DogPark,
    Dune,
    Farm,
    Farmland,
    Farmyard,
    Forest,
    Garden,
    Glacier,
    GolfCourse,
    Grass,
    Grassland,
    Hospital,
    Industrial,
    Kindergarten,
    Meadow,
    Military,
    NationalPark,
    NatureReserve,
    Orchard,
    Other,
    Park,
    Pedestrian,
    Pier,
    Pitch,
    Platform,
    Playground,
    Quarry,
    Railway,
    RecreationGround,
    Residential,
    Retail,
    Runway,
    Sand,
    School,
    Scrub,
    Taxiway,
    University,
    VillageGreen,
    Vineyard,
    Wetland,
    Wood,
    Zoo,
    Unknown,
}

macro_rules! landuse_kinds {
    ($macro:ident, $value:expr) => {
        $macro! {
            $value;
            "aerodrome" => Aerodrome,
            "airfield" => Airfield,
            "allotments" => Allotments,
            "bare_rock" => BareRock,
            "beach" => Beach,
            "cemetery" => Cemetery,
            "college" => College,
            "commercial" => Commercial,
            "dam" => Dam,
            "dog_park" => DogPark,
            "dune" => Dune,
            "farm" => Farm,
            "farmland" => Farmland,
            "farmyard" => Farmyard,
            "forest" => Forest,
            "garden" => Garden,
            "glacier" => Glacier,
            "golf_course" => GolfCourse,
            "grass" => Grass,
            "grassland" => Grassland,
            "hospital" => Hospital,
            "industrial" => Industrial,
            "kindergarten" => Kindergarten,
            "meadow" => Meadow,
            "military" => Military,
            "national_park" => NationalPark,
            "nature_reserve" => NatureReserve,
            "orchard" => Orchard,
            "other" => Other,
            "park" => Park,
            "pedestrian" => Pedestrian,
            "pier" => Pier,
            "pitch" => Pitch,
            "platform" => Platform,
            "playground" => Playground,
            "quarry" => Quarry,
            "railway" => Railway,
            "recreation_ground" => RecreationGround,
            "residential" => Residential,
            "retail" => Retail,
            "runway" => Runway,
            "sand" => Sand,
            "school" => School,
            "scrub" => Scrub,
            "taxiway" => Taxiway,
            "university" => University,
            "village_green" => VillageGreen,
            "vineyard" => Vineyard,
            "wetland" => Wetland,
            "wood" => Wood,
            "zoo" => Zoo,
        }
    };
}

macro_rules! parse_landuse_kind {
    ($value:expr; $($source:literal => $variant:ident,)*) => {
        match $value {
            $($source => Self::$variant,)*
            _ => Self::Unknown,
        }
    };
}

macro_rules! known_landuse_source {
    ($value:expr; $($source:literal => $variant:ident,)*) => {
        match $value {
            $($source => Some($source),)*
            _ => None,
        }
    };
}

impl LanduseKind {
    pub fn from_source(value: &str) -> Self {
        landuse_kinds!(parse_landuse_kind, value)
    }

    pub fn known_source(value: &str) -> Option<&'static str> {
        landuse_kinds!(known_landuse_source, value)
    }
}

pub const KNOWN_LANDUSE_KINDS: &[&str] = &[
    "aerodrome",
    "airfield",
    "allotments",
    "bare_rock",
    "beach",
    "cemetery",
    "college",
    "commercial",
    "dam",
    "dog_park",
    "dune",
    "farm",
    "farmland",
    "farmyard",
    "forest",
    "garden",
    "glacier",
    "golf_course",
    "grass",
    "grassland",
    "hospital",
    "industrial",
    "kindergarten",
    "meadow",
    "military",
    "national_park",
    "nature_reserve",
    "orchard",
    "other",
    "park",
    "pedestrian",
    "pier",
    "pitch",
    "platform",
    "playground",
    "quarry",
    "railway",
    "recreation_ground",
    "residential",
    "retail",
    "runway",
    "sand",
    "school",
    "scrub",
    "taxiway",
    "university",
    "village_green",
    "vineyard",
    "wetland",
    "wood",
    "zoo",
];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LanduseFacts {
    pub kind: Option<SourceFact>,
    pub class: Option<LanduseKind>,
    pub sport: Option<SourceFact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandEvidenceAuthority {
    InferredTerrain,
    RegionalLandcover,
    DetailedLanduse,
    ExistingMappedFloraOrForest,
    PhysicalOverlay,
}

impl LandEvidenceAuthority {
    const fn priority(self) -> u8 {
        match self {
            Self::InferredTerrain => 0,
            Self::RegionalLandcover => 1,
            Self::DetailedLanduse => 2,
            Self::ExistingMappedFloraOrForest => 3,
            Self::PhysicalOverlay => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandAuthorityCandidate {
    pub authority: LandEvidenceAuthority,
    pub stable_identity: u64,
}

/// Select one overlapping land authority independently of source iteration.
/// Stronger evidence wins; equal authority uses the lowest stable identity.
pub fn preferred_land_authority(
    left: LandAuthorityCandidate,
    right: LandAuthorityCandidate,
) -> LandAuthorityCandidate {
    match left.authority.priority().cmp(&right.authority.priority()) {
        std::cmp::Ordering::Less => right,
        std::cmp::Ordering::Greater => left,
        std::cmp::Ordering::Equal if right.stable_identity < left.stable_identity => right,
        std::cmp::Ordering::Equal => left,
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlaceFacts {
    pub kind: Option<SourceFact>,
    pub kind_detail: Option<SourceFact>,
    pub capital: Option<SourceFact>,
    pub population: Option<SourceFact>,
    pub wikidata: Option<SourceFact>,
    pub references: Vec<SourceFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PoiFacts {
    pub kind: Option<SourceFact>,
    pub kind_detail: Option<SourceFact>,
    pub elevation: Option<SourceFact>,
    pub iata: Option<SourceFact>,
    pub cuisine: Option<SourceFact>,
    pub religion: Option<SourceFact>,
    pub sport: Option<SourceFact>,
    pub wikidata: Option<SourceFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NamedExtensionFacts {
    pub kind: Option<SourceFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoadPhysicalFacts {
    pub kind: Option<SourceFact>,
    pub kind_detail: Option<SourceFact>,
    pub is_bridge: Option<SourceFact>,
    pub is_link: Option<SourceFact>,
    pub is_tunnel: Option<SourceFact>,
    pub layer: Option<SourceFact>,
    pub surface: Option<SourceFact>,
    pub width: Option<SourceFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoadContextFacts {
    pub access: Option<SourceFact>,
    pub oneway: Option<SourceFact>,
    pub reference: Option<SourceFact>,
    pub route: Option<SourceFact>,
    pub service: Option<SourceFact>,
    pub networks: Vec<SourceFact>,
    pub shield_texts: Vec<SourceFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoadFacts {
    pub physical: RoadPhysicalFacts,
    pub context: RoadContextFacts,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaterPhysicalFacts {
    pub kind: Option<SourceFact>,
    pub kind_detail: Option<SourceFact>,
    pub bridge: Option<SourceFact>,
    pub tunnel: Option<SourceFact>,
    pub layer: Option<SourceFact>,
    pub intermittent: Option<SourceFact>,
    pub reservoir: Option<SourceFact>,
    pub alkaline: Option<SourceFact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
pub enum WaterKind {
    Water,
    Lake,
    Playa,
    Ocean,
    Other,
    Basin,
    Bay,
    Canal,
    Ditch,
    Dock,
    Drain,
    Fountain,
    Reef,
    River,
    Sea,
    Strait,
    Stream,
    SwimmingPool,
    Unknown,
}

impl WaterKind {
    pub const ALL: [Self; 19] = [
        Self::Water,
        Self::Lake,
        Self::Playa,
        Self::Ocean,
        Self::Other,
        Self::Basin,
        Self::Bay,
        Self::Canal,
        Self::Ditch,
        Self::Dock,
        Self::Drain,
        Self::Fountain,
        Self::Reef,
        Self::River,
        Self::Sea,
        Self::Strait,
        Self::Stream,
        Self::SwimmingPool,
        Self::Unknown,
    ];

    pub fn from_source(value: &str) -> Self {
        match value {
            "water" => Self::Water,
            "lake" => Self::Lake,
            "playa" => Self::Playa,
            "ocean" => Self::Ocean,
            "other" => Self::Other,
            "basin" => Self::Basin,
            "bay" => Self::Bay,
            "canal" => Self::Canal,
            "ditch" => Self::Ditch,
            "dock" => Self::Dock,
            "drain" => Self::Drain,
            "fountain" => Self::Fountain,
            "reef" => Self::Reef,
            "river" => Self::River,
            "sea" => Self::Sea,
            "strait" => Self::Strait,
            "stream" => Self::Stream,
            "swimming_pool" => Self::SwimmingPool,
            _ => Self::Unknown,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Water => "water",
            Self::Lake => "lake",
            Self::Playa => "playa",
            Self::Ocean => "ocean",
            Self::Other => "other",
            Self::Basin => "basin",
            Self::Bay => "bay",
            Self::Canal => "canal",
            Self::Ditch => "ditch",
            Self::Dock => "dock",
            Self::Drain => "drain",
            Self::Fountain => "fountain",
            Self::Reef => "reef",
            Self::River => "river",
            Self::Sea => "sea",
            Self::Strait => "strait",
            Self::Stream => "stream",
            Self::SwimmingPool => "swimming_pool",
            Self::Unknown => "unknown",
        }
    }
}

pub const KNOWN_WATER_KINDS: &[&str] = &[
    "water",
    "lake",
    "playa",
    "ocean",
    "other",
    "basin",
    "bay",
    "canal",
    "ditch",
    "dock",
    "drain",
    "fountain",
    "reef",
    "river",
    "sea",
    "strait",
    "stream",
    "swimming_pool",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WaterKindDetail {
    Basin,
    Canal,
    Ditch,
    Dock,
    Drain,
    Lake,
    Reservoir,
    River,
    Riverbank,
    Stream,
    Rock,
    Sand,
    Unknown,
}

impl WaterKindDetail {
    pub fn from_source(value: &str) -> Self {
        match value {
            "basin" => Self::Basin,
            "canal" => Self::Canal,
            "ditch" => Self::Ditch,
            "dock" => Self::Dock,
            "drain" => Self::Drain,
            "lake" => Self::Lake,
            "reservoir" => Self::Reservoir,
            "river" => Self::River,
            "riverbank" => Self::Riverbank,
            "stream" => Self::Stream,
            "rock" => Self::Rock,
            "sand" => Self::Sand,
            _ => Self::Unknown,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Basin => "basin",
            Self::Canal => "canal",
            Self::Ditch => "ditch",
            Self::Dock => "dock",
            Self::Drain => "drain",
            Self::Lake => "lake",
            Self::Reservoir => "reservoir",
            Self::River => "river",
            Self::Riverbank => "riverbank",
            Self::Stream => "stream",
            Self::Rock => "rock",
            Self::Sand => "sand",
            Self::Unknown => "unknown",
        }
    }
}

pub const KNOWN_WATER_KIND_DETAILS: &[&str] = &[
    "basin",
    "canal",
    "ditch",
    "dock",
    "drain",
    "lake",
    "reservoir",
    "river",
    "riverbank",
    "stream",
    "rock",
    "sand",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum WaterGeometryRole {
    Area,
    Watercourse,
    Label,
    #[default]
    Unknown,
}

impl WaterGeometryRole {
    pub const ALL: [Self; 4] = [Self::Area, Self::Watercourse, Self::Label, Self::Unknown];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Area => "area",
            Self::Watercourse => "watercourse",
            Self::Label => "label",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum WaterSurfaceDisposition {
    Candidate,
    ContextOnly,
    SurfaceIneligible,
    #[default]
    Invalid,
}

impl WaterSurfaceDisposition {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::ContextOnly => "context_only",
            Self::SurfaceIneligible => "surface_ineligible",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaterFacts {
    pub class: Option<WaterKind>,
    pub detail_class: Option<WaterKindDetail>,
    pub geometry_role: WaterGeometryRole,
    pub surface_disposition: WaterSurfaceDisposition,
    pub physical: WaterPhysicalFacts,
}

/// Minimal source coordinates needed to make water admission deterministic
/// without treating clipped fragments or polygon/line companions as duplicate
/// physical geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterAdmissionCandidate {
    pub identity: SourceFeatureIdentity,
    pub geometry_role: WaterGeometryRole,
    pub source_tile: (u8, u32, u32),
    pub source_feature_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterAdmissionDecision {
    KeepBothDistinct,
    KeepBothFragments,
    KeepBothCompanions,
    KeepExisting,
    ReplaceExisting,
}

/// Compare two candidates that may have arrived from overlapping source
/// queries. Higher source zoom wins for the same identity and geometry role.
/// At the winning zoom, fragments from different tiles are all retained;
/// polygon, line, and label companions are also retained independently. Exact
/// duplicates within one tile use the lower source feature index.
pub fn water_admission_decision(
    existing: WaterAdmissionCandidate,
    incoming: WaterAdmissionCandidate,
) -> WaterAdmissionDecision {
    if existing.identity != incoming.identity {
        return WaterAdmissionDecision::KeepBothDistinct;
    }
    if existing.geometry_role != incoming.geometry_role {
        return WaterAdmissionDecision::KeepBothCompanions;
    }
    match existing.source_tile.0.cmp(&incoming.source_tile.0) {
        std::cmp::Ordering::Less => WaterAdmissionDecision::ReplaceExisting,
        std::cmp::Ordering::Greater => WaterAdmissionDecision::KeepExisting,
        std::cmp::Ordering::Equal if existing.source_tile != incoming.source_tile => {
            WaterAdmissionDecision::KeepBothFragments
        }
        std::cmp::Ordering::Equal => {
            if incoming.source_feature_index < existing.source_feature_index {
                WaterAdmissionDecision::ReplaceExisting
            } else {
                WaterAdmissionDecision::KeepExisting
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayerFacts {
    Boundaries(BoundaryFacts),
    Buildings(BuildingFacts),
    Earth(EarthFacts),
    Landcover(LandcoverFacts),
    Landuse(LanduseFacts),
    Places(PlaceFacts),
    Pois(PoiFacts),
    Natural(NamedExtensionFacts),
    PhysicalPoint(NamedExtensionFacts),
    Roads(RoadFacts),
    Water(WaterFacts),
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticFeature {
    pub source_feature_index: usize,
    pub source_id: Option<u64>,
    pub identity: SourceFeatureIdentity,
    pub geometry_type: GeomType,
    pub common: CommonFacts,
    pub facts: LayerFacts,
}

impl SemanticFeature {
    pub fn kind_and_detail(&self) -> (Option<SourceFact>, Option<SourceFact>) {
        match &self.facts {
            LayerFacts::Boundaries(facts) => (facts.kind, facts.kind_detail),
            LayerFacts::Buildings(facts) => (facts.physical.kind, facts.physical.kind_detail),
            LayerFacts::Earth(facts) => (facts.kind, None),
            LayerFacts::Landcover(facts) => (facts.kind, None),
            LayerFacts::Landuse(facts) => (facts.kind, None),
            LayerFacts::Places(facts) => (facts.kind, facts.kind_detail),
            LayerFacts::Pois(facts) => (facts.kind, facts.kind_detail),
            LayerFacts::Natural(facts) | LayerFacts::PhysicalPoint(facts) => (facts.kind, None),
            LayerFacts::Roads(facts) => (facts.physical.kind, facts.physical.kind_detail),
            LayerFacts::Water(facts) => (facts.physical.kind, facts.physical.kind_detail),
            LayerFacts::Other => (None, None),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticLayer {
    pub source_layer_index: usize,
    pub name: String,
    pub extent: u32,
    pub features: Vec<SemanticFeature>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticDiagnosticKind {
    UnknownText,
    InvalidValue,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemanticDiagnostic<'a> {
    pub layer: &'a str,
    pub source_feature_index: usize,
    pub field: &'static str,
    pub source: SourceValueRef,
    pub kind: SemanticDiagnosticKind,
}

pub struct SemanticTile {
    decoded: mvt::VectorTile,
    layers: Vec<SemanticLayer>,
    raw_heap_bytes: usize,
    semantic_heap_bytes: usize,
    component_heap_bytes: std::sync::atomic::AtomicUsize,
}

impl SemanticTile {
    pub fn normalize(decoded: mvt::VectorTile) -> Self {
        Self::normalize_at(decoded, (0, 0, 0))
    }

    pub fn normalize_at(decoded: mvt::VectorTile, tile_key: (u8, u32, u32)) -> Self {
        let raw_heap_bytes = decoded.estimated_heap_bytes();
        let layers: Vec<_> = decoded
            .layers
            .iter()
            .enumerate()
            .map(|(source_layer_index, layer)| {
                let features = layer
                    .features
                    .iter()
                    .enumerate()
                    .map(|(source_feature_index, feature)| {
                        normalize_feature(layer, feature, source_feature_index, tile_key)
                    })
                    .collect();
                SemanticLayer {
                    source_layer_index,
                    name: layer.name.clone(),
                    extent: layer.extent,
                    features,
                }
            })
            .collect();
        let semantic_heap_bytes = estimated_semantic_heap_bytes(&layers, layers.capacity());
        Self {
            decoded,
            layers,
            raw_heap_bytes,
            semantic_heap_bytes,
            component_heap_bytes: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn semantic_layers(&self) -> &[SemanticLayer] {
        &self.layers
    }

    pub fn semantic_layer(&self, name: &str) -> Option<&SemanticLayer> {
        self.layers.iter().find(|layer| layer.name == name)
    }

    pub fn source_layer(&self, layer: &SemanticLayer) -> Option<&mvt::Layer> {
        self.decoded.layers.get(layer.source_layer_index)
    }

    pub fn source_feature<'a>(
        &'a self,
        layer: &SemanticLayer,
        feature: &SemanticFeature,
    ) -> Option<&'a mvt::Feature> {
        self.source_layer(layer)?
            .features
            .get(feature.source_feature_index)
    }

    /// Reuse the exact tile-space topology plan across all projected cell consumers.
    pub(crate) fn source_polygon_components<'a>(
        &'a self,
        layer: &SemanticLayer,
        feature: &SemanticFeature,
    ) -> Option<mvt::polygon::PolygonComponents<'a>> {
        let source = self.source_feature(layer, feature)?;
        if source.geom_type != GeomType::Polygon {
            return None;
        }
        let groups = source.component_groups.get_or_init(|| {
            let groups = mvt::polygon::polygon_component_groups(&source.paths);
            self.component_heap_bytes.fetch_add(
                mvt::polygon::groups_heap_bytes(&groups),
                std::sync::atomic::Ordering::Relaxed,
            );
            groups
        });
        Some(mvt::polygon::PolygonComponents::from_plan(
            &source.paths,
            groups,
        ))
    }

    pub fn source_value<'a>(
        &'a self,
        layer: &SemanticLayer,
        value: SourceValueRef,
    ) -> Option<&'a Value> {
        self.source_layer(layer)?
            .values
            .get(value.value_index as usize)
    }

    pub fn source_name_key<'a>(&'a self, layer: &SemanticLayer, name: NameFact) -> Option<&'a str> {
        self.source_layer(layer)?
            .keys
            .get(name.key_index as usize)
            .map(String::as_str)
    }

    pub fn raw_heap_bytes(&self) -> usize {
        self.raw_heap_bytes
    }

    pub fn semantic_heap_bytes(&self) -> usize {
        self.semantic_heap_bytes
    }

    pub fn estimated_heap_bytes(&self) -> usize {
        self.raw_heap_bytes
            + self.semantic_heap_bytes
            + self
                .component_heap_bytes
                .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Raw decoded access is test-only parity evidence. Production consumers
    /// must route properties through typed facts and use explicit source
    /// geometry accessors.
    #[cfg(test)]
    pub fn decoded(&self) -> &mvt::VectorTile {
        &self.decoded
    }

    pub fn source_text<'a>(&'a self, layer: &SemanticLayer, fact: SourceFact) -> Option<&'a str> {
        self.source_value(layer, fact.source)?.as_str()
    }

    pub fn source_number(&self, layer: &SemanticLayer, fact: SourceFact) -> Option<f64> {
        match fact.normalized {
            NormalizedValue::Number(value) => Some(value),
            _ => self.source_value(layer, fact.source)?.as_f64(),
        }
    }

    pub fn source_boolean(&self, _layer: &SemanticLayer, fact: SourceFact) -> Option<bool> {
        match fact.normalized {
            NormalizedValue::Boolean(value) => Some(value),
            _ => None,
        }
    }

    /// Unknown open strings and malformed typed values remain observable
    /// without copying their raw values into the semantic sidecar.
    pub fn diagnostics(&self) -> Vec<SemanticDiagnostic<'_>> {
        let mut diagnostics = Vec::new();
        for layer in &self.layers {
            for feature in &layer.features {
                macro_rules! record {
                    ($field:literal, $fact:expr) => {
                        if let Some(fact) = $fact {
                            let kind = match fact.normalized {
                                NormalizedValue::UnknownText => {
                                    Some(SemanticDiagnosticKind::UnknownText)
                                }
                                NormalizedValue::Invalid => {
                                    Some(SemanticDiagnosticKind::InvalidValue)
                                }
                                _ => None,
                            };
                            if let Some(kind) = kind {
                                diagnostics.push(SemanticDiagnostic {
                                    layer: &layer.name,
                                    source_feature_index: feature.source_feature_index,
                                    field: $field,
                                    source: fact.source,
                                    kind,
                                });
                            }
                        }
                    };
                }
                record!("min_zoom", feature.common.min_zoom);
                record!("sort_rank", feature.common.sort_rank);
                record!("population_rank", feature.common.population_rank);
                record!("sort_key", feature.common.sort_key);
                record!("place", feature.common.place);
                record!("population", feature.common.population);
                match &feature.facts {
                    LayerFacts::Boundaries(facts) => {
                        record!("kind", facts.kind);
                        record!("kind_detail", facts.kind_detail);
                        record!("disputed", facts.disputed);
                        record!("brk_a3", facts.brk_a3);
                        record!("maritime", facts.maritime);
                    }
                    LayerFacts::Buildings(facts) => {
                        record!("kind", facts.physical.kind);
                        record!("kind_detail", facts.physical.kind_detail);
                        record!("height", facts.physical.height);
                        record!("min_height", facts.physical.min_height);
                        record!("layer", facts.physical.layer);
                        record!("roof:shape", facts.physical.roof_shape);
                        record!("addr_housenumber", facts.context.addr_housenumber);
                    }
                    LayerFacts::Earth(facts) => record!("kind", facts.kind),
                    LayerFacts::Landcover(facts) => record!("kind", facts.kind),
                    LayerFacts::Landuse(facts) => {
                        record!("kind", facts.kind);
                        record!("sport", facts.sport);
                    }
                    LayerFacts::Places(facts) => {
                        record!("kind", facts.kind);
                        record!("kind_detail", facts.kind_detail);
                        record!("capital", facts.capital);
                        record!("population", facts.population);
                        record!("wikidata", facts.wikidata);
                    }
                    LayerFacts::Pois(facts) => {
                        record!("kind", facts.kind);
                        record!("kind_detail", facts.kind_detail);
                        record!("elevation", facts.elevation);
                        record!("iata", facts.iata);
                        record!("cuisine", facts.cuisine);
                        record!("religion", facts.religion);
                        record!("sport", facts.sport);
                        record!("wikidata", facts.wikidata);
                    }
                    LayerFacts::Natural(facts) | LayerFacts::PhysicalPoint(facts) => {
                        record!("kind", facts.kind);
                    }
                    LayerFacts::Roads(facts) => {
                        record!("kind", facts.physical.kind);
                        record!("kind_detail", facts.physical.kind_detail);
                        record!("is_bridge", facts.physical.is_bridge);
                        record!("is_link", facts.physical.is_link);
                        record!("is_tunnel", facts.physical.is_tunnel);
                        record!("layer", facts.physical.layer);
                        record!("surface", facts.physical.surface);
                        record!("width", facts.physical.width);
                        record!("access", facts.context.access);
                        record!("oneway", facts.context.oneway);
                        record!("ref", facts.context.reference);
                        record!("route", facts.context.route);
                        record!("service", facts.context.service);
                    }
                    LayerFacts::Water(facts) => {
                        record!("kind", facts.physical.kind);
                        record!("kind_detail", facts.physical.kind_detail);
                        record!("bridge", facts.physical.bridge);
                        record!("tunnel", facts.physical.tunnel);
                        record!("layer", facts.physical.layer);
                        record!("intermittent", facts.physical.intermittent);
                        record!("reservoir", facts.physical.reservoir);
                        record!("alkaline", facts.physical.alkaline);
                    }
                    LayerFacts::Other => {}
                }
            }
        }
        diagnostics
    }
}

#[derive(Clone, Copy)]
enum ExpectedValue {
    Text,
    Number,
    Boolean,
}

fn normalize_feature(
    layer: &mvt::Layer,
    feature: &mvt::Feature,
    source_feature_index: usize,
    tile_key: (u8, u32, u32),
) -> SemanticFeature {
    let mut properties = HashMap::new();
    for (key_index, value_index) in feature.tag_pairs() {
        if let Some(key) = layer.keys.get(key_index as usize) {
            properties
                .entry(key.as_str())
                .or_insert((key_index, value_index));
        }
    }
    let fact = |name: &str, expected| {
        properties
            .get(name)
            .copied()
            .map(|(_, value_index)| source_fact(layer, value_index, expected))
    };
    let open_fact = |name: &str, known: &[&str]| {
        properties
            .get(name)
            .copied()
            .map(|(_, value_index)| open_text_fact(layer, value_index, known))
    };
    let common = CommonFacts {
        min_zoom: fact("min_zoom", ExpectedValue::Number),
        sort_rank: fact("sort_rank", ExpectedValue::Number),
        population_rank: fact("population_rank", ExpectedValue::Number),
        sort_key: fact("sort_key", ExpectedValue::Number),
        place: fact("place", ExpectedValue::Text),
        population: fact("population", ExpectedValue::Number),
        names: feature
            .tag_pairs()
            .filter_map(|(key_index, value_index)| {
                let key = layer.keys.get(key_index as usize)?;
                is_name_key(key).then(|| NameFact {
                    key_index,
                    role: name_role(key),
                    value: source_fact(layer, value_index, ExpectedValue::Text),
                })
            })
            .collect(),
    };
    let facts = match layer.name.as_str() {
        "boundaries" => LayerFacts::Boundaries(BoundaryFacts {
            kind: open_fact("kind", &["country", "region", "county", "locality"]),
            kind_detail: fact("kind_detail", ExpectedValue::Number),
            disputed: fact("disputed", ExpectedValue::Boolean),
            brk_a3: fact("brk_a3", ExpectedValue::Text),
            maritime: fact("maritime", ExpectedValue::Boolean),
        }),
        "buildings" => LayerFacts::Buildings(BuildingFacts {
            physical: BuildingPhysicalFacts {
                kind: open_fact("kind", &["building", "building_part", "part", "address"]),
                kind_detail: open_fact("kind_detail", &["roof"]),
                height: fact("height", ExpectedValue::Number)
                    .or_else(|| fact("height_m", ExpectedValue::Number)),
                min_height: fact("min_height", ExpectedValue::Number)
                    .or_else(|| fact("min_height_m", ExpectedValue::Number)),
                layer: fact("layer", ExpectedValue::Number),
                roof_shape: fact("roof:shape", ExpectedValue::Text),
            },
            context: BuildingContextFacts {
                addr_housenumber: fact("addr_housenumber", ExpectedValue::Text),
            },
        }),
        "earth" => LayerFacts::Earth(EarthFacts {
            kind: open_fact("kind", &["land", "cliff"]),
        }),
        "landcover" => {
            let kind = open_fact("kind", KNOWN_LANDCOVER_KINDS);
            LayerFacts::Landcover(LandcoverFacts {
                kind,
                class: classified_source_text(layer, kind).map(LandcoverKind::from_source),
            })
        }
        "landuse" => {
            let kind = open_fact("kind", KNOWN_LANDUSE_KINDS);
            LayerFacts::Landuse(LanduseFacts {
                kind,
                class: classified_source_text(layer, kind).map(LanduseKind::from_source),
                sport: fact("sport", ExpectedValue::Text),
            })
        }
        "places" => LayerFacts::Places(PlaceFacts {
            kind: open_fact("kind", &["locality", "region", "country"]),
            kind_detail: open_fact(
                "kind_detail",
                &[
                    "city", "town", "village", "hamlet", "state", "province", "county",
                ],
            ),
            capital: fact("capital", ExpectedValue::Text),
            population: fact("population", ExpectedValue::Number),
            wikidata: fact("wikidata", ExpectedValue::Text),
            references: prefixed_facts(layer, feature, "ref", ExpectedValue::Text),
        }),
        "pois" => LayerFacts::Pois(PoiFacts {
            kind: open_fact("kind", &["viewpoint", "attraction", "restaurant", "tree"]),
            kind_detail: open_fact("kind_detail", &[]),
            elevation: fact("elevation", ExpectedValue::Number),
            iata: fact("iata", ExpectedValue::Text),
            cuisine: fact("cuisine", ExpectedValue::Text),
            religion: fact("religion", ExpectedValue::Text),
            sport: fact("sport", ExpectedValue::Text),
            wikidata: fact("wikidata", ExpectedValue::Text),
        }),
        "natural" => LayerFacts::Natural(NamedExtensionFacts {
            kind: open_fact("kind", &[]),
        }),
        "physical_point" => LayerFacts::PhysicalPoint(NamedExtensionFacts {
            kind: open_fact("kind", &[]),
        }),
        "roads" => LayerFacts::Roads(RoadFacts {
            physical: RoadPhysicalFacts {
                kind: open_fact(
                    "kind",
                    &[
                        "highway",
                        "major_road",
                        "medium_road",
                        "minor_road",
                        "path",
                        "rail",
                        "aeroway",
                        "ferry",
                    ],
                ),
                kind_detail: open_fact(
                    "kind_detail",
                    &[
                        "motorway",
                        "motorway_link",
                        "trunk",
                        "trunk_link",
                        "primary",
                        "primary_link",
                        "secondary",
                        "secondary_link",
                        "tertiary",
                        "tertiary_link",
                        "service",
                        "driveway",
                        "parking_aisle",
                        "alley",
                        "drive-through",
                        "emergency_access",
                        "subway",
                    ],
                ),
                is_bridge: fact("is_bridge", ExpectedValue::Boolean),
                is_link: fact("is_link", ExpectedValue::Boolean),
                is_tunnel: fact("is_tunnel", ExpectedValue::Boolean),
                layer: fact("layer", ExpectedValue::Number),
                surface: fact("surface", ExpectedValue::Text),
                width: fact("width", ExpectedValue::Number),
            },
            context: RoadContextFacts {
                access: fact("access", ExpectedValue::Text),
                oneway: fact("oneway", ExpectedValue::Text),
                reference: fact("ref", ExpectedValue::Text),
                route: fact("route", ExpectedValue::Text),
                service: fact("service", ExpectedValue::Text),
                networks: prefixed_facts(layer, feature, "network", ExpectedValue::Text),
                shield_texts: prefixed_facts(layer, feature, "shield_text", ExpectedValue::Text),
            },
        }),
        "water" => {
            let kind = open_fact("kind", KNOWN_WATER_KINDS);
            let kind_detail = open_fact("kind_detail", KNOWN_WATER_KIND_DETAILS);
            let tunnel = fact("tunnel", ExpectedValue::Text);
            let water_layer = fact("layer", ExpectedValue::Number);
            let geometry_role = match feature.geom_type {
                GeomType::Polygon => WaterGeometryRole::Area,
                GeomType::Line => WaterGeometryRole::Watercourse,
                GeomType::Point => WaterGeometryRole::Label,
            };
            let tunnel_active = classified_source_text(layer, tunnel).is_some_and(|value| {
                !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "no" | "false" | "0"
                )
            });
            let below_surface = water_layer.is_some_and(
                |fact| matches!(fact.normalized, NormalizedValue::Number(value) if value < 0.0),
            );
            let surface_disposition = match geometry_role {
                WaterGeometryRole::Label => WaterSurfaceDisposition::ContextOnly,
                WaterGeometryRole::Watercourse if tunnel_active || below_surface => {
                    WaterSurfaceDisposition::SurfaceIneligible
                }
                WaterGeometryRole::Area | WaterGeometryRole::Watercourse => {
                    WaterSurfaceDisposition::Candidate
                }
                WaterGeometryRole::Unknown => WaterSurfaceDisposition::Invalid,
            };
            LayerFacts::Water(WaterFacts {
                class: classified_source_text(layer, kind).map(WaterKind::from_source),
                detail_class: classified_source_text(layer, kind_detail)
                    .map(WaterKindDetail::from_source),
                geometry_role,
                surface_disposition,
                physical: WaterPhysicalFacts {
                    kind,
                    kind_detail,
                    bridge: fact("bridge", ExpectedValue::Text),
                    tunnel,
                    layer: water_layer,
                    intermittent: fact("intermittent", ExpectedValue::Boolean),
                    reservoir: fact("reservoir", ExpectedValue::Boolean),
                    alkaline: fact("alkaline", ExpectedValue::Boolean),
                },
            })
        }
        _ => LayerFacts::Other,
    };
    SemanticFeature {
        source_feature_index,
        source_id: feature.id,
        identity: feature.id.map_or_else(
            || {
                SourceFeatureIdentity::DeterministicFallback(fallback_identity(
                    tile_key,
                    layer,
                    feature,
                    source_feature_index,
                ))
            },
            SourceFeatureIdentity::Provided,
        ),
        geometry_type: feature.geom_type,
        common,
        facts,
    }
}

fn source_fact(layer: &mvt::Layer, value_index: u32, expected: ExpectedValue) -> SourceFact {
    let normalized = layer
        .values
        .get(value_index as usize)
        .map_or(NormalizedValue::Invalid, |value| {
            normalize_value(value, expected)
        });
    SourceFact {
        source: SourceValueRef { value_index },
        normalized,
    }
}

fn open_text_fact(layer: &mvt::Layer, value_index: u32, known: &[&str]) -> SourceFact {
    let normalized = match layer.values.get(value_index as usize) {
        Some(Value::Str(value)) if known.contains(&value.as_str()) => NormalizedValue::Text,
        Some(Value::Str(_)) => NormalizedValue::UnknownText,
        _ => NormalizedValue::Invalid,
    };
    SourceFact {
        source: SourceValueRef { value_index },
        normalized,
    }
}

fn classified_source_text(layer: &mvt::Layer, fact: Option<SourceFact>) -> Option<&str> {
    let fact = fact?;
    match fact.normalized {
        NormalizedValue::Text | NormalizedValue::UnknownText => {
            layer.values.get(fact.source.value_index as usize)?.as_str()
        }
        NormalizedValue::Number(_) | NormalizedValue::Boolean(_) | NormalizedValue::Invalid => None,
    }
}

fn fallback_identity(
    tile_key: (u8, u32, u32),
    layer: &mvt::Layer,
    feature: &mvt::Feature,
    source_feature_index: usize,
) -> u64 {
    fn hash(mut state: u64, bytes: &[u8]) -> u64 {
        for byte in bytes {
            state = (state ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
        }
        state
    }

    let mut state = hash(0xcbf2_9ce4_8422_2325, layer.name.as_bytes());
    state = hash(state, &[feature.geom_type as u8]);
    if let Some(kind) = feature.get(layer, "kind").and_then(Value::as_str) {
        state = hash(state, kind.as_bytes());
    }
    let world_extent = (1_i128 << tile_key.0) * i128::from(layer.extent);
    for point in feature.paths.iter().flatten() {
        let global_x = (i128::from(tile_key.1) * i128::from(layer.extent) + i128::from(point[0]))
            * (1_i128 << 32)
            / world_extent;
        let global_y = (i128::from(tile_key.2) * i128::from(layer.extent) + i128::from(point[1]))
            * (1_i128 << 32)
            / world_extent;
        state = hash(state, &(global_x as i64).to_le_bytes());
        state = hash(state, &(global_y as i64).to_le_bytes());
    }
    if feature.paths.iter().all(Vec::is_empty) {
        state = hash(state, &[tile_key.0]);
        state = hash(state, &tile_key.1.to_le_bytes());
        state = hash(state, &tile_key.2.to_le_bytes());
        state = hash(state, &(source_feature_index as u64).to_le_bytes());
    }
    state
}

fn normalize_value(value: &Value, expected: ExpectedValue) -> NormalizedValue {
    match expected {
        ExpectedValue::Text => match value {
            Value::Str(_) => NormalizedValue::Text,
            _ => NormalizedValue::Invalid,
        },
        ExpectedValue::Number => value
            .as_f64()
            .or_else(|| {
                value.as_str().and_then(|text| {
                    text.trim()
                        .trim_end_matches(|character: char| {
                            character.is_ascii_alphabetic() || character.is_whitespace()
                        })
                        .trim()
                        .parse()
                        .ok()
                })
            })
            .map_or(NormalizedValue::Invalid, NormalizedValue::Number),
        ExpectedValue::Boolean => match value {
            Value::Bool(value) => NormalizedValue::Boolean(*value),
            Value::I64(value) => match value {
                0 => NormalizedValue::Boolean(false),
                1 => NormalizedValue::Boolean(true),
                _ => NormalizedValue::Invalid,
            },
            Value::Str(value) => match value.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => NormalizedValue::Boolean(true),
                "false" | "no" | "0" => NormalizedValue::Boolean(false),
                _ => NormalizedValue::Invalid,
            },
            Value::F64(_) => NormalizedValue::Invalid,
        },
    }
}

fn prefixed_facts(
    layer: &mvt::Layer,
    feature: &mvt::Feature,
    prefix: &str,
    expected: ExpectedValue,
) -> Vec<SourceFact> {
    feature
        .tag_pairs()
        .filter_map(|(key_index, value_index)| {
            let key = layer.keys.get(key_index as usize)?;
            (key == prefix
                || key
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('_') || suffix.starts_with(':')))
            .then(|| source_fact(layer, value_index, expected))
        })
        .collect()
}

fn is_name_key(key: &str) -> bool {
    matches!(
        key,
        "name" | "name2" | "name3" | "script" | "script2" | "script3"
    ) || key.starts_with("name:")
        || key.starts_with("pgf:name")
}

fn name_role(key: &str) -> NameRole {
    if key.starts_with("script") {
        NameRole::Script
    } else if key.starts_with("pgf:name") {
        NameRole::Preferred
    } else if key.starts_with("name:") {
        NameRole::Localized
    } else {
        NameRole::Local
    }
}

fn estimated_semantic_heap_bytes(layers: &[SemanticLayer], layer_capacity: usize) -> usize {
    layer_capacity * std::mem::size_of::<SemanticLayer>()
        + layers
            .iter()
            .map(|layer| {
                layer.name.capacity()
                    + layer.features.capacity() * std::mem::size_of::<SemanticFeature>()
                    + layer
                        .features
                        .iter()
                        .map(|feature| {
                            feature.common.names.capacity() * std::mem::size_of::<NameFact>()
                                + match &feature.facts {
                                    LayerFacts::Places(facts) => {
                                        facts.references.capacity()
                                            * std::mem::size_of::<SourceFact>()
                                    }
                                    LayerFacts::Roads(facts) => {
                                        (facts.context.networks.capacity()
                                            + facts.context.shield_texts.capacity())
                                            * std::mem::size_of::<SourceFact>()
                                    }
                                    _ => 0,
                                }
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvt::encode::{FeatureSpec, LayerSpec};

    fn polygon_plan_tile(paths: Vec<Vec<[i32; 2]>>) -> SemanticTile {
        let bytes = mvt::encode::tile(
            "water",
            4096,
            &["kind"],
            &[Value::Str("lake".into())],
            &[FeatureSpec {
                id: Some(42),
                geom_type: GeomType::Polygon,
                paths,
                tags: vec![(0, 0)],
            }],
        );
        SemanticTile::normalize(mvt::decode_tile(&bytes).unwrap())
    }

    #[test]
    fn polygon_plan_is_shared_concurrently_and_its_heap_is_counted_once() {
        let paths = vec![
            vec![[0, 0], [100, 0], [100, 100], [0, 100]],
            vec![[20, 20], [20, 80], [80, 80], [80, 20]],
            vec![[200, 0], [300, 0], [300, 100], [200, 100]],
        ];
        let tile = polygon_plan_tile(paths);
        let layer = tile.semantic_layer("water").unwrap();
        let feature = &layer.features[0];
        let raw = tile.source_feature(layer, feature).unwrap();
        let expected: Vec<_> = mvt::polygon::polygon_components(&raw.paths)
            .map(|component| (component.paths.into_owned(), component.seed_anchor))
            .collect();
        assert_eq!(expected.len(), 2);
        assert_eq!(expected[0].0, raw.paths[..2]);
        assert_eq!(expected[1].0, raw.paths[2..]);
        let initial_bytes = tile.estimated_heap_bytes();
        assert_eq!(
            initial_bytes,
            tile.raw_heap_bytes() + tile.semantic_heap_bytes()
        );
        assert!(raw.component_groups.get().is_none());
        let barrier = std::sync::Barrier::new(8);
        vearth_platform::thread::scope(|scope| {
            for _ in 0..8 {
                let barrier = &barrier;
                let tile = &tile;
                let expected = &expected;
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..16 {
                        let actual: Vec<_> = tile
                            .source_polygon_components(layer, feature)
                            .unwrap()
                            .map(|component| (component.paths.into_owned(), component.seed_anchor))
                            .collect();
                        assert_eq!(&actual, expected);
                    }
                });
            }
        });
        let plan = raw.component_groups.get().unwrap();
        let plan_bytes = mvt::polygon::groups_heap_bytes(plan);
        assert!(plan_bytes > 0);
        assert_eq!(tile.estimated_heap_bytes(), initial_bytes + plan_bytes);
        assert_eq!(feature.source_id, Some(42));
        assert_eq!(feature.identity, SourceFeatureIdentity::Provided(42));
    }

    #[test]
    fn polygon_plan_compound_fallback_is_cached_and_owned_by_its_tile() {
        let first = polygon_plan_tile(vec![vec![[0, 0], [100, 0], [100, 100], [0, 100]]]);
        let second = polygon_plan_tile(vec![vec![[200, 0], [300, 0], [300, 100], [200, 100]]]);
        for tile in [&first, &second] {
            let layer = tile.semantic_layer("water").unwrap();
            let feature = &layer.features[0];
            let raw = tile.source_feature(layer, feature).unwrap();
            let initial_bytes = tile.estimated_heap_bytes();
            let components: Vec<_> = tile
                .source_polygon_components(layer, feature)
                .unwrap()
                .collect();
            assert_eq!(components.len(), 1);
            assert_eq!(components[0].paths.as_ref(), raw.paths);
            assert_eq!(components[0].seed_anchor, raw.paths[0][0]);
            assert_eq!(raw.component_groups.get(), Some(&None));
            assert_eq!(tile.estimated_heap_bytes(), initial_bytes);
        }
        assert_ne!(
            first.decoded.layers[0].features[0].paths,
            second.decoded.layers[0].features[0].paths,
        );
    }

    #[test]
    fn normalization_preserves_raw_types_and_marks_recoverable_invalid_values() {
        let keys = [
            "kind",
            "kind_detail",
            "is_bridge",
            "min_zoom",
            "name:en",
            "service",
        ];
        let values = [
            Value::Str("major_road".into()),
            Value::Str("primary".into()),
            Value::Str("yes".into()),
            Value::Str("12.5".into()),
            Value::Str("Fixture Road".into()),
        ];
        let features = [FeatureSpec {
            id: Some(42),
            geom_type: GeomType::Line,
            paths: vec![vec![[0, 0], [100, 100]]],
            tags: vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 999)],
        }];
        let bytes = mvt::encode::tile_layers(&[LayerSpec {
            name: "roads",
            extent: 4096,
            keys: &keys,
            values: &values,
            features: &features,
        }]);
        let tile = SemanticTile::normalize(mvt::decode_tile(&bytes).unwrap());
        let layer = tile.semantic_layer("roads").unwrap();
        let feature = &layer.features[0];
        assert_eq!(feature.source_id, Some(42));
        assert_eq!(feature.identity, SourceFeatureIdentity::Provided(42));
        assert_eq!(feature.geometry_type, GeomType::Line);
        assert_eq!(
            feature.common.min_zoom.unwrap().normalized,
            NormalizedValue::Number(12.5)
        );
        assert_eq!(feature.common.names.len(), 1);
        assert_eq!(feature.common.names[0].role, NameRole::Localized);
        assert_eq!(
            tile.source_name_key(layer, feature.common.names[0]),
            Some("name:en")
        );
        let LayerFacts::Roads(road) = &feature.facts else {
            panic!("road facts")
        };
        assert_eq!(
            road.physical.is_bridge.unwrap().normalized,
            NormalizedValue::Boolean(true)
        );
        assert!(
            matches!(tile.source_value(layer, road.physical.kind.unwrap().source), Some(Value::Str(value)) if value == "major_road")
        );
        assert_eq!(
            road.context.service.unwrap().normalized,
            NormalizedValue::Invalid
        );
        assert_eq!(
            tile.source_value(layer, road.context.service.unwrap().source),
            None,
            "invalid indices remain explicit and recoverable"
        );
        assert!(tile.diagnostics().iter().any(|diagnostic| {
            diagnostic.field == "service" && diagnostic.kind == SemanticDiagnosticKind::InvalidValue
        }));
        assert_eq!(
            tile.source_feature(layer, feature).unwrap().paths[0].len(),
            2
        );
        assert!(tile.raw_heap_bytes() > 0);
        assert!(tile.semantic_heap_bytes() > 0);
    }

    #[test]
    fn unknown_open_text_and_missing_ids_have_stable_raw_fallbacks() {
        let bytes = mvt::encode::tile(
            "water",
            4096,
            &["kind"],
            &[Value::Str("future_water".into())],
            &[FeatureSpec {
                id: None,
                geom_type: GeomType::Polygon,
                paths: vec![vec![[10, 20], [30, 20], [30, 40]]],
                tags: vec![(0, 0)],
            }],
        );
        let normalize =
            || SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), (12, 2048, 2048));
        let first = normalize();
        let second = normalize();
        let layer = first.semantic_layer("water").unwrap();
        let feature = &layer.features[0];
        assert!(matches!(
            feature.identity,
            SourceFeatureIdentity::DeterministicFallback(_)
        ));
        assert_eq!(
            feature.identity,
            second.semantic_layer("water").unwrap().features[0].identity
        );
        let finer_bytes = mvt::encode::tile(
            "water",
            4096,
            &["kind"],
            &[Value::Str("future_water".into())],
            &[FeatureSpec {
                id: None,
                geom_type: GeomType::Polygon,
                paths: vec![vec![[20, 40], [60, 40], [60, 80]]],
                tags: vec![(0, 0)],
            }],
        );
        let finer =
            SemanticTile::normalize_at(mvt::decode_tile(&finer_bytes).unwrap(), (13, 4096, 4096));
        assert_eq!(
            feature.identity,
            finer.semantic_layer("water").unwrap().features[0].identity,
            "fallback identity uses global source geometry rather than tile zoom"
        );
        let LayerFacts::Water(facts) = &feature.facts else {
            panic!("water facts")
        };
        let kind = facts.physical.kind.unwrap();
        assert_eq!(kind.normalized, NormalizedValue::UnknownText);
        assert_eq!(first.source_text(layer, kind), Some("future_water"));
        assert_eq!(first.diagnostics().len(), 1);
        assert_eq!(
            first.diagnostics()[0].kind,
            SemanticDiagnosticKind::UnknownText
        );
    }
}
