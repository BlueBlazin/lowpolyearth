//! Deterministic field surfaces, crop and flower inference, generated flora
//! descriptors, and curated destination treatments.
//!
//! A field seed selects one immutable treatment, palette and row equation.
//! The equation is evaluated on the canonical cube-face lattice so clipped
//! MVT fragments, neighboring terrain cells and every L11-L19 cut sample the
//! same continuous world-space pattern. Descriptors store field eligibility
//! and terrain anchoring; individual plants remain procedural.

use std::f64::consts::TAU;

use vearth_core::cell::{CellId, NOMINAL_FACE_METERS};
use vearth_data::vectors::FloraRegionKind;

use crate::N;
use crate::flower_destinations::{
    DestinationMode, FlowerDestination, FlowerDestinationId, FlowerPattern, Rgb8,
    flower_destination,
};

pub const FLORA_COVERAGE_EDGE: usize = 16;
pub const FLORA_COVERAGE_WORDS: usize =
    FLORA_COVERAGE_EDGE * FLORA_COVERAGE_EDGE / u32::BITS as usize;
pub const FLORA_GROUND_EDGE: usize = 5;
pub const FLORA_GROUND_SAMPLES: usize = FLORA_GROUND_EDGE * FLORA_GROUND_EDGE;
pub const CROP_HEAD_MIN_PIXELS: f32 = 1.75;
pub const CROP_CLOSE_END_M: f32 = 70.0;
pub const CROP_MIDDLE_END_M: f32 = 220.0;
pub const CROP_COARSE_END_M: f32 = 650.0;
/// Distant field aggregates are the seasonal, palette-correct overview
/// representation. Keeping them beyond ordinary plant-detail range prevents
/// the immutable terrain tint from becoming the only evidence of cultivation.
pub const CROP_DISTANT_END_M: f32 = 6_000.0;
pub const FLOWER_HEAD_MIN_PIXELS: f32 = 1.75;
pub const NATURAL_WILDFLOWER_MIN_LEVEL: u8 = 17;
const CURATED_FIELD_SEED_PREFIX: u64 = 0xf109_0000_0000_0000;

/// The four authored forms that bridge a crop field's semantic surface to
/// close geometry. `SurfaceOnly` deliberately has no plant candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropRepresentation {
    Close,
    Middle,
    CoarseTuft,
    SurfaceOnly,
}

/// Physical catalogue values mirrored by `flora.wgsl`. Middle and coarse
/// records describe cluster glyphs, not enlarged individual plants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropForm {
    pub height_min_m: f32,
    pub height_max_m: f32,
    pub stem_width_m: f32,
    pub leaf_width_m: f32,
    pub head_diameter_m: f32,
    pub max_head_enlargement: f32,
}

/// Flower geometry follows the crop distance rings, but uses dedicated forms:
/// middle and coarse records are coordinated flower clusters rather than
/// enlarged individual blooms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowerRepresentation {
    Close,
    MiddleCluster,
    CoarseTuft,
    SurfaceOnly,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowerForm {
    pub height_min_m: f32,
    pub height_max_m: f32,
    pub stem_width_m: f32,
    pub leaf_width_m: f32,
    pub head_diameter_m: f32,
    pub max_head_enlargement: f32,
}

/// Managed perennial forms use the same distance ladder as annual crops, but
/// every middle/coarse glyph represents an increasingly broad row cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedRepresentation {
    Close,
    MiddleCluster,
    CoarseRow,
    SurfaceOnly,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrchardForm {
    pub trunk_height_m: f32,
    pub trunk_width_m: f32,
    pub crown_height_m: f32,
    pub crown_along_m: f32,
    pub crown_across_m: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VineyardForm {
    pub mass_bottom_m: f32,
    pub mass_height_m: f32,
    pub mass_along_m: f32,
    pub mass_across_m: f32,
    pub post_height_m: f32,
    pub post_width_m: f32,
    /// One support cue per this many deterministic managed-row slots. Zero
    /// means that the surface/coarse representation carries no post geometry.
    pub post_period: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GardenPatch {
    FlowerBed,
    MaintainedGreen,
    ShrubProxy,
    Gap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WildflowerRoute {
    MappedMeadow,
    NaturalGrass,
}

/// Time-neutral ecological inputs used to build immutable eligibility. Snow
/// exposure is a climate/height normal here; live snow and drought continue to
/// suppress the surviving geometry in the shader without regenerating cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WildflowerEnvironment {
    pub temp_c: f32,
    pub elevation_m: f32,
    pub slope_rad: f32,
    pub relief_m: f32,
    pub aridity: f32,
    pub precipitation_mm_day: f32,
    pub humidity: f32,
    pub near_water: bool,
    pub snow_exposure: f32,
}

/// Time-neutral environmental facts used to choose a plausible visual crop
/// grammar. The regional catalog uses bundled administrative boundaries and
/// climate normals; current simulation weather never changes field identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropEnvironment {
    pub lat_deg: f32,
    pub lon_deg: f32,
    pub temp_c: f32,
    pub elevation_m: f32,
    pub slope_rad: f32,
    pub relief_m: f32,
    pub aridity: f32,
    pub precipitation_mm_day: f32,
    pub humidity: f32,
    pub near_water: bool,
}

/// Stable field calendar selected from time-neutral climate and cultivation
/// semantics. Runtime weather advances the selected calendar without changing
/// the generated descriptor or invalidating a terrain chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CultivationCalendar {
    TemperateSummer,
    CoolShortSeason,
    MediterraneanWinter,
    Monsoonal,
    TropicalMultiCycle,
    PerennialOrchard,
    PerennialVineyard,
    CuratedBloom,
}

impl CultivationCalendar {
    pub const COUNT: usize = 8;

    pub fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::TemperateSummer,
            1 => Self::CoolShortSeason,
            2 => Self::MediterraneanWinter,
            3 => Self::Monsoonal,
            4 => Self::TropicalMultiCycle,
            5 => Self::PerennialOrchard,
            6 => Self::PerennialVineyard,
            7 => Self::CuratedBloom,
            _ => return None,
        })
    }
}

/// Continuous shader controls produced by a cultivation calendar. They are
/// deliberately presentation-only: candidate addresses, coverage, terrain
/// samples and field identity remain unchanged as time advances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloraSeasonState {
    pub height: f32,
    pub density: f32,
    pub bloom: f32,
    pub maturity: f32,
    pub senescence: f32,
    pub stubble: f32,
    pub dormancy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloraSeasonInputs {
    pub local_phase: f32,
    pub astronomical_phase: f32,
    pub temperature_c: f32,
    pub moisture: f32,
    pub drought: f32,
    pub snow: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropVariation {
    pub height_scale: f32,
    pub shade: f32,
    pub exists: f32,
}

/// Visual grammar selected once per stable source field. These are deliberately
/// archetypes rather than claims about a real crop species.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum FloraArchetype {
    Cereal,
    BroadleafRows,
    TallRows,
    LowDenseCrop,
    FlowerCrop,
    OrnamentalBed,
    MeadowWildflowers,
    Orchard,
    Vineyard,
}

impl FloraArchetype {
    pub const COUNT: usize = 9;

    pub fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Cereal,
            1 => Self::BroadleafRows,
            2 => Self::TallRows,
            3 => Self::LowDenseCrop,
            4 => Self::FlowerCrop,
            5 => Self::OrnamentalBed,
            6 => Self::MeadowWildflowers,
            7 => Self::Orchard,
            8 => Self::Vineyard,
            _ => return None,
        })
    }
}

// Source fragments can differ slightly at MVT clip edges. Climate is a
// one-degree prior; an eighth-degree context avoids interpreting projection
// ulps or small fragment offsets as different agricultural identities.
pub(crate) fn field_context_location(location: [f64; 2]) -> [f64; 2] {
    let lat = (location[0].to_degrees() * 8.0).round() / 8.0;
    let lon = (location[1].to_degrees() * 8.0).round() / 8.0;
    [
        lat.clamp(-90.0, 90.0).to_radians(),
        ((lon + 180.0).rem_euclid(360.0) - 180.0).to_radians(),
    ]
}

/// Choose one climate-compatible calendar for the whole stable field. The
/// inputs are annual/time-neutral generation facts, so the result never
/// changes with wall clock or current weather.
pub fn select_cultivation_calendar(
    archetype: FloraArchetype,
    environment: CropEnvironment,
    curated_style: u8,
) -> CultivationCalendar {
    use CultivationCalendar::{
        CoolShortSeason, CuratedBloom, MediterraneanWinter, Monsoonal, PerennialOrchard,
        PerennialVineyard, TemperateSummer, TropicalMultiCycle,
    };
    if curated_style != 0 {
        return CuratedBloom;
    }
    match archetype {
        FloraArchetype::Orchard => return PerennialOrchard,
        FloraArchetype::Vineyard => return PerennialVineyard,
        _ => {}
    }

    let abs_lat = environment.lat_deg.abs();
    let tropical = abs_lat < 18.0 && environment.temp_c >= 20.0;
    let monsoonal = environment.temp_c >= 17.0
        && environment.precipitation_mm_day >= 2.2
        && environment.humidity >= 0.64
        && environment.aridity < 0.52;
    if monsoonal && (!tropical || environment.precipitation_mm_day >= 3.2) {
        Monsoonal
    } else if tropical {
        TropicalMultiCycle
    } else if (25.0..=48.0).contains(&abs_lat)
        && (8.0..=26.0).contains(&environment.temp_c)
        && environment.aridity >= 0.44
        && environment.precipitation_mm_day < 1.8
    {
        MediterraneanWinter
    } else if abs_lat >= 50.0 || environment.temp_c <= 10.0 || environment.elevation_m >= 1_800.0 {
        CoolShortSeason
    } else {
        TemperateSummer
    }
}

/// Convert the stored uniform byte into a centered, calendar-specific timing
/// offset. Inferred fields move by at most 6.5% of a local year. Curated
/// destinations use the same byte across the full cyclic half-year so their
/// surveyed bloom midpoint can be represented in either hemisphere.
pub fn bounded_phenology_offset(calendar: CultivationCalendar, encoded: u8) -> f32 {
    use CultivationCalendar::*;
    let maximum = match calendar {
        TemperateSummer => 0.055,
        CoolShortSeason => 0.035,
        MediterraneanWinter => 0.045,
        Monsoonal => 0.035,
        TropicalMultiCycle => 0.025,
        PerennialOrchard | PerennialVineyard => 0.045,
        CuratedBloom => 0.5,
    };
    (f32::from(encoded) * (2.0 / 255.0) - 1.0) * maximum
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn cyclic_window(phase: f32, start: f32, end: f32, feather: f32) -> f32 {
    let phase = phase.rem_euclid(1.0);
    let start = start.rem_euclid(1.0);
    let end = end.rem_euclid(1.0);
    if start <= end {
        smoothstep(start - feather, start + feather, phase)
            * (1.0 - smoothstep(end - feather, end + feather, phase))
    } else {
        (smoothstep(start - feather, start + feather, phase)
            + (1.0 - smoothstep(end - feather, end + feather, phase)))
        .clamp(0.0, 1.0)
    }
}

/// CPU reference for the calendar equations mirrored in `flora.wgsl`. The
/// astronomical phase is used only by wet-season and multi-cycle calendars;
/// temperate and perennial calendars consume the already hemisphere-correct
/// local phase.
pub fn flora_season_state(
    calendar: CultivationCalendar,
    encoded_offset: u8,
    inputs: FloraSeasonInputs,
) -> FloraSeasonState {
    use CultivationCalendar::*;
    let offset = bounded_phenology_offset(calendar, encoded_offset);
    let local = (inputs.local_phase + offset).rem_euclid(1.0);
    let astronomical = (inputs.astronomical_phase + offset).rem_euclid(1.0);
    let wet = inputs.moisture.clamp(0.0, 1.0);
    let (emergence, vegetative, bloom, maturity, senescence, stubble, dormancy) = match calendar {
        TemperateSummer | CuratedBloom => (
            cyclic_window(local, 0.10, 0.28, 0.045),
            cyclic_window(local, 0.18, 0.58, 0.055),
            cyclic_window(local, 0.30, 0.54, 0.045),
            cyclic_window(local, 0.48, 0.70, 0.045),
            cyclic_window(local, 0.65, 0.84, 0.045),
            cyclic_window(local, 0.78, 0.96, 0.035),
            cyclic_window(local, 0.90, 0.14, 0.045),
        ),
        CoolShortSeason => (
            cyclic_window(local, 0.20, 0.34, 0.035),
            cyclic_window(local, 0.27, 0.56, 0.040),
            cyclic_window(local, 0.40, 0.56, 0.035),
            cyclic_window(local, 0.50, 0.67, 0.035),
            cyclic_window(local, 0.63, 0.77, 0.035),
            cyclic_window(local, 0.72, 0.93, 0.035),
            cyclic_window(local, 0.86, 0.23, 0.040),
        ),
        MediterraneanWinter => (
            cyclic_window(local, 0.68, 0.84, 0.040),
            cyclic_window(local, 0.76, 0.30, 0.055),
            cyclic_window(local, 0.12, 0.31, 0.040),
            cyclic_window(local, 0.25, 0.44, 0.040),
            cyclic_window(local, 0.40, 0.57, 0.040),
            cyclic_window(local, 0.52, 0.72, 0.040),
            cyclic_window(local, 0.58, 0.76, 0.045),
        ),
        Monsoonal => {
            let wet_growth = smoothstep(0.22, 0.62, wet);
            let wet_bloom = smoothstep(0.34, 0.72, wet);
            let drying = 1.0 - smoothstep(0.16, 0.46, wet);
            (
                wet_growth * cyclic_window(astronomical, 0.02, 0.26, 0.08),
                wet_growth,
                wet_bloom * cyclic_window(astronomical, 0.16, 0.48, 0.10),
                wet_growth * cyclic_window(astronomical, 0.38, 0.72, 0.08),
                drying,
                drying * cyclic_window(astronomical, 0.58, 0.96, 0.08),
                drying,
            )
        }
        TropicalMultiCycle => {
            let cycle = (astronomical * 2.0).fract();
            let moisture_gate = smoothstep(0.12, 0.48, wet);
            (
                cyclic_window(cycle, 0.00, 0.20, 0.06) * moisture_gate,
                cyclic_window(cycle, 0.10, 0.62, 0.07) * moisture_gate,
                cyclic_window(cycle, 0.32, 0.56, 0.055) * moisture_gate,
                cyclic_window(cycle, 0.50, 0.76, 0.055) * moisture_gate,
                cyclic_window(cycle, 0.72, 0.92, 0.055),
                cyclic_window(cycle, 0.86, 0.08, 0.055),
                (1.0 - moisture_gate) * 0.80,
            )
        }
        PerennialOrchard => (
            cyclic_window(local, 0.10, 0.24, 0.035),
            cyclic_window(local, 0.18, 0.72, 0.055),
            cyclic_window(local, 0.12, 0.29, 0.035),
            cyclic_window(local, 0.42, 0.70, 0.055),
            cyclic_window(local, 0.66, 0.86, 0.045),
            0.0,
            cyclic_window(local, 0.84, 0.18, 0.050),
        ),
        PerennialVineyard => (
            cyclic_window(local, 0.16, 0.30, 0.035),
            cyclic_window(local, 0.23, 0.75, 0.055),
            cyclic_window(local, 0.34, 0.50, 0.040),
            cyclic_window(local, 0.52, 0.72, 0.050),
            cyclic_window(local, 0.68, 0.88, 0.045),
            0.0,
            cyclic_window(local, 0.86, 0.20, 0.050),
        ),
    };

    let live = (emergence * 0.35 + vegetative.max(maturity) + bloom * 0.18).clamp(0.0, 1.0);
    let drought = inputs.drought.clamp(0.0, 1.0);
    let snow = inputs.snow.clamp(0.0, 1.0);
    // Lifecycle owns field structure. An hourly frost may change shading or
    // snow cover, but it must not erase an otherwise in-season planted field.
    // Emergence becomes dense while remaining short; senescence keeps standing
    // stalks; harvest keeps dense low stubble.
    let emerging_structure = smoothstep(0.04, 0.22, emergence);
    let standing_structure = emerging_structure
        .max(vegetative)
        .max(bloom)
        .max(maturity)
        .max(senescence)
        .max(stubble);
    let structural_height = live.max(senescence * 0.82).max(stubble * 0.15);
    FloraSeasonState {
        height: (0.08 + structural_height * 0.92) * (1.0 - drought * 0.45) * (1.0 - snow * 0.75),
        density: (0.08 + standing_structure * 0.92) * (1.0 - drought * 0.70),
        bloom: bloom * (1.0 - drought * 0.85) * (1.0 - snow),
        maturity: maturity * (1.0 - snow * 0.88),
        senescence: senescence.max(drought * 0.72),
        stubble,
        dormancy: dormancy.max(snow),
    }
}

pub const fn crop_form(
    archetype: FloraArchetype,
    representation: CropRepresentation,
) -> Option<CropForm> {
    use CropRepresentation::{Close, CoarseTuft, Middle, SurfaceOnly};
    use FloraArchetype::{BroadleafRows, Cereal, LowDenseCrop, TallRows};
    if matches!(representation, SurfaceOnly) {
        return None;
    }
    Some(match (archetype, representation) {
        (Cereal, Close) => CropForm {
            height_min_m: 0.65,
            height_max_m: 1.10,
            stem_width_m: 0.035,
            leaf_width_m: 0.075,
            head_diameter_m: 0.12,
            max_head_enlargement: 2.20,
        },
        (BroadleafRows, Close) => CropForm {
            height_min_m: 0.40,
            height_max_m: 0.78,
            stem_width_m: 0.055,
            leaf_width_m: 0.18,
            head_diameter_m: 0.22,
            max_head_enlargement: 2.00,
        },
        (TallRows, Close) => CropForm {
            height_min_m: 1.40,
            height_max_m: 2.20,
            stem_width_m: 0.060,
            leaf_width_m: 0.13,
            head_diameter_m: 0.18,
            max_head_enlargement: 2.10,
        },
        (LowDenseCrop, Close) => CropForm {
            height_min_m: 0.28,
            height_max_m: 0.55,
            stem_width_m: 0.050,
            leaf_width_m: 0.16,
            head_diameter_m: 0.20,
            max_head_enlargement: 1.90,
        },
        (Cereal, Middle) => CropForm {
            height_min_m: 0.78,
            height_max_m: 1.12,
            stem_width_m: 0.10,
            leaf_width_m: 0.24,
            head_diameter_m: 0.36,
            max_head_enlargement: 1.80,
        },
        (BroadleafRows, Middle) => CropForm {
            height_min_m: 0.48,
            height_max_m: 0.82,
            stem_width_m: 0.12,
            leaf_width_m: 0.30,
            head_diameter_m: 0.46,
            max_head_enlargement: 1.70,
        },
        (TallRows, Middle) => CropForm {
            height_min_m: 1.45,
            height_max_m: 2.15,
            stem_width_m: 0.12,
            leaf_width_m: 0.28,
            head_diameter_m: 0.42,
            max_head_enlargement: 1.80,
        },
        (LowDenseCrop, Middle) => CropForm {
            height_min_m: 0.34,
            height_max_m: 0.62,
            stem_width_m: 0.10,
            leaf_width_m: 0.28,
            head_diameter_m: 0.48,
            max_head_enlargement: 1.65,
        },
        (Cereal, CoarseTuft) => CropForm {
            height_min_m: 0.68,
            height_max_m: 0.96,
            stem_width_m: 0.18,
            leaf_width_m: 0.55,
            head_diameter_m: 1.30,
            max_head_enlargement: 1.50,
        },
        (BroadleafRows, CoarseTuft) => CropForm {
            height_min_m: 0.48,
            height_max_m: 0.78,
            stem_width_m: 0.18,
            leaf_width_m: 0.60,
            head_diameter_m: 1.35,
            max_head_enlargement: 1.45,
        },
        (TallRows, CoarseTuft) => CropForm {
            height_min_m: 0.95,
            height_max_m: 1.45,
            stem_width_m: 0.20,
            leaf_width_m: 0.65,
            head_diameter_m: 1.40,
            max_head_enlargement: 1.45,
        },
        (LowDenseCrop, CoarseTuft) => CropForm {
            height_min_m: 0.38,
            height_max_m: 0.64,
            stem_width_m: 0.18,
            leaf_width_m: 0.58,
            head_diameter_m: 1.32,
            max_head_enlargement: 1.45,
        },
        _ => return None,
    })
}

pub const fn flower_form(
    archetype: FloraArchetype,
    representation: FlowerRepresentation,
) -> Option<FlowerForm> {
    use FloraArchetype::{FlowerCrop, MeadowWildflowers, OrnamentalBed};
    use FlowerRepresentation::{Close, CoarseTuft, MiddleCluster, SurfaceOnly};
    if matches!(representation, SurfaceOnly) {
        return None;
    }
    Some(match (archetype, representation) {
        (FlowerCrop, Close) => FlowerForm {
            height_min_m: 0.38,
            height_max_m: 0.78,
            stem_width_m: 0.045,
            leaf_width_m: 0.13,
            head_diameter_m: 0.18,
            max_head_enlargement: 2.20,
        },
        (OrnamentalBed, Close) => FlowerForm {
            height_min_m: 0.30,
            height_max_m: 0.72,
            stem_width_m: 0.042,
            leaf_width_m: 0.14,
            head_diameter_m: 0.16,
            max_head_enlargement: 2.25,
        },
        (MeadowWildflowers, Close) => FlowerForm {
            height_min_m: 0.22,
            height_max_m: 0.58,
            stem_width_m: 0.032,
            leaf_width_m: 0.10,
            head_diameter_m: 0.12,
            max_head_enlargement: 2.35,
        },
        (FlowerCrop, MiddleCluster) => FlowerForm {
            height_min_m: 0.46,
            height_max_m: 0.82,
            stem_width_m: 0.11,
            leaf_width_m: 0.30,
            head_diameter_m: 0.55,
            max_head_enlargement: 1.75,
        },
        (OrnamentalBed, MiddleCluster) => FlowerForm {
            height_min_m: 0.38,
            height_max_m: 0.76,
            stem_width_m: 0.10,
            leaf_width_m: 0.28,
            head_diameter_m: 0.50,
            max_head_enlargement: 1.80,
        },
        (MeadowWildflowers, MiddleCluster) => FlowerForm {
            height_min_m: 0.30,
            height_max_m: 0.64,
            stem_width_m: 0.085,
            leaf_width_m: 0.24,
            head_diameter_m: 0.44,
            max_head_enlargement: 1.85,
        },
        (FlowerCrop, CoarseTuft) => FlowerForm {
            height_min_m: 0.50,
            height_max_m: 0.82,
            stem_width_m: 0.22,
            leaf_width_m: 0.64,
            head_diameter_m: 1.38,
            max_head_enlargement: 1.45,
        },
        (OrnamentalBed, CoarseTuft) => FlowerForm {
            height_min_m: 0.44,
            height_max_m: 0.76,
            stem_width_m: 0.20,
            leaf_width_m: 0.60,
            head_diameter_m: 1.32,
            max_head_enlargement: 1.50,
        },
        (MeadowWildflowers, CoarseTuft) => FlowerForm {
            height_min_m: 0.36,
            height_max_m: 0.68,
            stem_width_m: 0.18,
            leaf_width_m: 0.54,
            head_diameter_m: 1.25,
            max_head_enlargement: 1.55,
        },
        _ => return None,
    })
}

pub const fn orchard_form(representation: ManagedRepresentation) -> Option<OrchardForm> {
    use ManagedRepresentation::{Close, CoarseRow, MiddleCluster, SurfaceOnly};
    Some(match representation {
        Close => OrchardForm {
            trunk_height_m: 1.15,
            trunk_width_m: 0.18,
            crown_height_m: 1.85,
            crown_along_m: 2.45,
            crown_across_m: 2.15,
        },
        MiddleCluster => OrchardForm {
            trunk_height_m: 1.05,
            trunk_width_m: 0.28,
            crown_height_m: 1.75,
            crown_along_m: 3.10,
            crown_across_m: 2.45,
        },
        CoarseRow => OrchardForm {
            trunk_height_m: 0.78,
            trunk_width_m: 0.42,
            crown_height_m: 1.62,
            crown_along_m: 5.20,
            crown_across_m: 2.70,
        },
        SurfaceOnly => return None,
    })
}

pub const fn vineyard_form(representation: ManagedRepresentation) -> Option<VineyardForm> {
    use ManagedRepresentation::{Close, CoarseRow, MiddleCluster, SurfaceOnly};
    Some(match representation {
        Close => VineyardForm {
            mass_bottom_m: 0.52,
            mass_height_m: 0.92,
            mass_along_m: 2.25,
            mass_across_m: 0.62,
            post_height_m: 1.72,
            post_width_m: 0.075,
            post_period: 4,
        },
        MiddleCluster => VineyardForm {
            mass_bottom_m: 0.46,
            mass_height_m: 0.98,
            mass_along_m: 3.80,
            mass_across_m: 0.78,
            post_height_m: 1.68,
            post_width_m: 0.11,
            post_period: 6,
        },
        CoarseRow => VineyardForm {
            mass_bottom_m: 0.34,
            mass_height_m: 0.90,
            mass_along_m: 6.40,
            mass_across_m: 1.02,
            post_height_m: 0.0,
            post_width_m: 0.0,
            post_period: 0,
        },
        SurfaceOnly => return None,
    })
}

/// Perspective projection used by the authored-form tests and mirrored in
/// WGSL. `framebuffer_height_px` is the physical render target height.
pub fn projected_diameter_pixels(
    diameter_m: f32,
    distance_m: f32,
    framebuffer_height_px: f32,
    tan_half_vertical_fov: f32,
) -> f32 {
    diameter_m * framebuffer_height_px
        / (2.0 * tan_half_vertical_fov.max(1.0e-4) * distance_m.max(1.0e-3))
}

pub fn surviving_head_pixels(
    form: CropForm,
    distance_m: f32,
    framebuffer_height_px: f32,
    tan_half_vertical_fov: f32,
) -> f32 {
    let authored = projected_diameter_pixels(
        form.head_diameter_m,
        distance_m,
        framebuffer_height_px,
        tan_half_vertical_fov,
    );
    let scale = (CROP_HEAD_MIN_PIXELS / authored.max(1.0e-4)).clamp(1.0, form.max_head_enlargement);
    authored * scale
}

pub fn surviving_flower_head_pixels(
    form: FlowerForm,
    distance_m: f32,
    framebuffer_height_px: f32,
    tan_half_vertical_fov: f32,
) -> f32 {
    let authored = projected_diameter_pixels(
        form.head_diameter_m,
        distance_m,
        framebuffer_height_px,
        tan_half_vertical_fov,
    );
    let scale =
        (FLOWER_HEAD_MIN_PIXELS / authored.max(1.0e-4)).clamp(1.0, form.max_head_enlargement);
    authored * scale
}

pub fn crop_archetype_is_plausible(
    archetype: FloraArchetype,
    environment: CropEnvironment,
) -> bool {
    use FloraArchetype::{BroadleafRows, Cereal, LowDenseCrop, TallRows};
    match archetype {
        Cereal => true,
        BroadleafRows => {
            environment.temp_c > 2.0
                && environment.temp_c < 34.0
                && environment.elevation_m < 3_400.0
                && environment.aridity < 0.90
        }
        TallRows => {
            // Annual climate normals are cooler than the growing season.
            environment.temp_c >= 8.0
                && environment.temp_c <= 32.0
                && environment.elevation_m < 2_000.0
                && environment.slope_rad < 0.28
                && environment.relief_m < 140.0
                && environment.aridity < 0.62
                && (environment.precipitation_mm_day >= 0.85
                    || environment.humidity >= 0.58
                    || environment.near_water)
        }
        LowDenseCrop => {
            environment.temp_c > -2.0
                && environment.temp_c < 36.0
                && environment.elevation_m < 3_800.0
                && environment.slope_rad < 0.36
        }
        _ => false,
    }
}

/// Infer one whole-field crop grammar. The categorical regime is deliberately
/// broad; the stable field seed provides variation inside it. Implausible or
/// extreme inputs always retain the generic cereal cultivated form.
pub fn infer_crop_archetype(seed: u64, environment: CropEnvironment) -> FloraArchetype {
    crate::agriculture_catalog::crop_recipe(seed, environment).archetype
}

/// Immutable visual record for one winning field identity in a generated
/// terrain cell. A large field still produces only one record per intersected
/// cell; candidate plants are never stored.
#[derive(Clone, Debug, PartialEq)]
pub struct FloraCell {
    pub archetype: FloraArchetype,
    pub calendar: CultivationCalendar,
    pub field_seed: u64,
    pub row_heading_rad: f32,
    /// Terrain and plants share this field-wide palette. Annual catalog crops
    /// use `agriculture_catalog::CROP_PALETTE_BASE` and above; other surfaces
    /// retain their local palettes, while curated sites use `curated_style`.
    pub palette: u8,
    pub base_density: u8,
    pub coverage: [u32; FLORA_COVERAGE_WORDS],
    /// Centimetre origin for `ground_cm`, in the owning cell's k=0 local
    /// vertical frame. Keeping the origin explicit makes high plateaus and
    /// below-sea terrain representable without spending 32 bits per sample.
    pub ground_base_cm: i32,
    /// 5x5 signed centimetre offsets from `ground_base_cm`.
    pub ground_cm: [i16; FLORA_GROUND_SAMPLES],
    pub phenology_offset: u8,
    pub curated_style: u8,
}

impl FloraCell {
    pub fn covered_samples(&self) -> u32 {
        self.coverage.iter().map(|word| word.count_ones()).sum()
    }

    /// Piecewise planar ground height in metres at a fractional cell position,
    /// using the same 00→11 diagonal as the terrain surface.
    pub fn ground_height_m(&self, fx: f32, fz: f32) -> f32 {
        let gx = fx.clamp(0.0, 1.0) * (FLORA_GROUND_EDGE - 1) as f32;
        let gz = fz.clamp(0.0, 1.0) * (FLORA_GROUND_EDGE - 1) as f32;
        let x0 = gx.floor() as usize;
        let z0 = gz.floor() as usize;
        let x1 = (x0 + 1).min(FLORA_GROUND_EDGE - 1);
        let z1 = (z0 + 1).min(FLORA_GROUND_EDGE - 1);
        let sample = |x: usize, z: usize| {
            (self.ground_base_cm + i32::from(self.ground_cm[z * FLORA_GROUND_EDGE + x])) as f32
                * 0.01
        };
        let (fx, fz) = (gx.fract(), gz.fract());
        let (h00, h10, h01, h11) = (
            sample(x0, z0),
            sample(x1, z0),
            sample(x0, z1),
            sample(x1, z1),
        );
        if fz <= fx {
            h00 + (h10 - h00) * fx + (h11 - h10) * fz
        } else {
            h00 + (h11 - h01) * fx + (h01 - h00) * fz
        }
    }
}

/// Quantize a local-height grid around its midpoint. Terrain spanning more
/// than the signed 16-bit centimetre range uses a deterministic clamp; that
/// is the safe steep-terrain fallback because roots remain bounded and never
/// affect solid geometry.
pub fn quantize_ground_grid(
    heights_m: [f32; FLORA_GROUND_SAMPLES],
) -> (i32, [i16; FLORA_GROUND_SAMPLES]) {
    let (lo, hi) = heights_m
        .iter()
        .copied()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), height| {
            (lo.min(height), hi.max(height))
        });
    let base_cm =
        (((lo + hi) * 50.0).round() as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let mut offsets = [0i16; FLORA_GROUND_SAMPLES];
    for (offset, height) in offsets.iter_mut().zip(heights_m) {
        let height_cm =
            ((height * 100.0).round() as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        *offset = height_cm
            .saturating_sub(base_cm)
            .clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }
    (base_cm, offsets)
}

pub(crate) fn descriptor_archetype(
    kind: FloraRegionKind,
    surface: FieldSurfaceKind,
    seed: u64,
    environment: CropEnvironment,
) -> FloraArchetype {
    match kind {
        FloraRegionKind::Garden => FloraArchetype::OrnamentalBed,
        FloraRegionKind::Meadow => FloraArchetype::MeadowWildflowers,
        FloraRegionKind::Orchard => FloraArchetype::Orchard,
        FloraRegionKind::Vineyard => FloraArchetype::Vineyard,
        // Surface kind and plant archetype form one stable field identity; a
        // flower surface must retain the matching flower-crop archetype.
        FloraRegionKind::Farmland if surface == FieldSurfaceKind::FlowerField => {
            FloraArchetype::FlowerCrop
        }
        FloraRegionKind::Farmland => infer_crop_archetype(seed, environment),
    }
}

pub(crate) fn descriptor_density(archetype: FloraArchetype) -> u8 {
    match archetype {
        // Annual crops are deliberately fuller than flower fields. A glyph is
        // still a bounded procedural cluster rather than an individual plant.
        FloraArchetype::Cereal => 250,
        FloraArchetype::BroadleafRows => 246,
        FloraArchetype::TallRows => 242,
        FloraArchetype::LowDenseCrop => 252,
        FloraArchetype::FlowerCrop => 232,
        FloraArchetype::OrnamentalBed => 220,
        FloraArchetype::MeadowWildflowers => 112,
        // Managed candidates collapse onto deterministic row slots. Density
        // therefore expresses restrained missing trees/segments, not random
        // free placement inside the field.
        FloraArchetype::Orchard => 232,
        FloraArchetype::Vineyard => 244,
    }
}

pub(crate) fn descriptor_phenology_offset(seed: u64) -> u8 {
    (splitmix(seed ^ 0x7068_656e_6f6c_6f67) >> 56) as u8
}

/// Semantic surface treatment carried from vector rasterization into the
/// ordinary terrain material. Dormant and harvested remain distinct so
/// phenology can select them without changing the surface vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum FieldSurfaceKind {
    Crop,
    FlowerField,
    Orchard,
    Vineyard,
    Meadow,
    Dormant,
    Harvested,
}

/// Compact field-wide surface identity copied through the per-column overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldSurface {
    pub kind: FieldSurfaceKind,
    pub palette: u8,
    /// Quantized turns.  Quantization makes equality exact and avoids slightly
    /// different trigonometric inputs in separately generated cells.
    pub row_heading: u16,
    pub row_spacing_m: f32,
    pub row_width: f32,
}

impl FieldSurface {
    pub fn heading_rad(self) -> f64 {
        // The descriptor and shader carry f32. Quantize here too so the
        // surface and procedural geometry use one exact field-wide axis.
        f64::from((f64::from(self.row_heading) * TAU / 65_536.0) as f32)
    }
}

/// A surface sample returned to the voxel palette code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldSurfaceSample {
    pub kind: FieldSurfaceKind,
    pub palette: u8,
    /// Coverage of the planted row after analytic footprint attenuation.
    pub row: f32,
    /// Wider field-scale strip used by flower fields and restrained as a
    /// secondary shade variation for other cultivated treatments.
    pub band: f32,
    /// Exact authored colour band shared with procedural flower heads.
    pub flower_variant: u8,
    /// Continuous, low-frequency variation for meadow and within-row shading.
    pub variation: f32,
}

#[inline]
fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Integer mixer mirrored by `flora.wgsl::hash_u32`.
pub fn flora_hash_u32(mut value: u32) -> u32 {
    value = (value ^ (value >> 16)).wrapping_mul(0x7feb_352d);
    value = (value ^ (value >> 15)).wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

/// Candidate identity mirrored by `flora.wgsl::candidate_hash`.
pub fn flora_candidate_hash(seed: u64, gx: u32, gz: u32, layer: u32, salt: u32) -> u32 {
    // `identity.x/y` are the low/high seed halves in the shader.
    let mut hash = seed as u32 ^ ((seed >> 32) as u32).wrapping_mul(0x9e37_79b9);
    hash = flora_hash_u32(hash ^ gx.wrapping_mul(0x85eb_ca6b));
    hash = flora_hash_u32(hash ^ gz.wrapping_mul(0xc2b2_ae35));
    flora_hash_u32(hash ^ layer.wrapping_mul(0x27d4_eb2d) ^ salt)
}

/// Exact 24-bit unit conversion used by the WGSL shader.
pub fn flora_hash_unit(value: u32) -> f32 {
    (value >> 8) as f32 * (1.0 / 16_777_216.0)
}

#[derive(Clone, Copy, Debug)]
pub struct FloraAdmissionInput<'a> {
    pub seed: u64,
    pub glyph: u32,
    pub candidate_limit: u32,
    pub managed_inside: bool,
    pub coverage: &'a [u32; FLORA_COVERAGE_WORDS],
    pub mask_x: u32,
    pub mask_z: u32,
    pub visual_gx: u32,
    pub visual_gz: u32,
    pub visual_layer: u32,
    /// `base_density / 255` from the descriptor/instance.
    pub base_density: f32,
    pub season_density: f32,
    /// True only for the clustered wildflower archetype (`archetype == 6`).
    pub clustered_wildflowers: bool,
    /// Unsnapped lattice coordinates used by the cluster modulation.
    pub source_gx: u32,
    pub source_gz: u32,
}

/// CPU reference for the shader's deterministic candidate-existence predicate.
pub fn flora_candidate_admitted(input: FloraAdmissionInput<'_>) -> bool {
    if input.glyph >= input.candidate_limit
        || !input.managed_inside
        || input.mask_x >= 16
        || input.mask_z >= 16
    {
        return false;
    }
    let bit = input.mask_z * 16 + input.mask_x;
    if input.coverage[(bit >> 5) as usize] & (1 << (bit & 31)) == 0 {
        return false;
    }
    let existence = flora_hash_unit(flora_candidate_hash(
        input.seed,
        input.visual_gx,
        input.visual_gz,
        input.visual_layer,
        0x6578_6973,
    ));
    let mut threshold = input.base_density * input.season_density;
    if input.clustered_wildflowers {
        let cluster = flora_hash_unit(flora_candidate_hash(
            input.seed,
            input.source_gx >> 3,
            input.source_gz >> 3,
            0,
            0x6465_6e73,
        ));
        let t = ((cluster - 0.30) / (0.82 - 0.30)).clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t);
        threshold *= 0.12 + (1.0 - 0.12) * smooth;
    }
    existence < threshold
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FloraPrimitiveRule {
    Always,
    /// Crop/flower head predicate (`0x68656164 + primitive`).
    Head {
        primitive: u32,
        presence: f32,
    },
    /// Orchard/vine foliage predicate (`0x73656173`).
    Seasonal {
        density: f32,
    },
    /// Vineyard support post predicate.
    VineyardPost {
        period: u32,
    },
}

/// CPU reference for shader predicates that can collapse a primitive after
/// its candidate was admitted.
pub fn flora_primitive_exists(
    candidate_exists: bool,
    seed: u64,
    visual_gx: u32,
    visual_gz: u32,
    visual_layer: u32,
    rule: FloraPrimitiveRule,
) -> bool {
    if !candidate_exists {
        return false;
    }
    match rule {
        FloraPrimitiveRule::Always => true,
        FloraPrimitiveRule::Head {
            primitive,
            presence,
        } => {
            flora_hash_unit(flora_candidate_hash(
                seed,
                visual_gx,
                visual_gz,
                visual_layer,
                0x6865_6164u32.wrapping_add(primitive),
            )) < presence
        }
        FloraPrimitiveRule::Seasonal { density } => {
            flora_hash_unit(flora_candidate_hash(
                seed,
                visual_gx,
                visual_gz,
                visual_layer,
                0x7365_6173,
            )) < density
        }
        FloraPrimitiveRule::VineyardPost { period } => {
            period > 0
                && flora_hash_u32(visual_gx ^ visual_gz.wrapping_mul(0x9e37_79b9))
                    .is_multiple_of(period)
        }
    }
}

#[inline]
fn hash_unit(seed: u64) -> f64 {
    (splitmix(seed) >> 11) as f64 * (1.0 / ((1u64 << 53) as f64))
}

pub fn commercial_flower_suitable(environment: CropEnvironment) -> bool {
    environment.temp_c.is_finite()
        && (4.0..=32.0).contains(&environment.temp_c)
        && environment.elevation_m < 3_200.0
        && environment.slope_rad < 0.26
        && environment.relief_m < 160.0
        && environment.aridity < 0.76
        && (environment.precipitation_mm_day >= 0.55
            || environment.humidity >= 0.50
            || environment.near_water)
}

/// Stable low-frequency garden partition. It is evaluated in canonical
/// face-metres so beds do not restart at vector, terrain-cell, or LOD edges.
pub fn garden_patch_at(seed: u64, cell: CellId, fx: f64, fz: f64) -> GardenPatch {
    let (east, north) = canonical_face_m(cell, fx, fz);
    let phase_a = hash_unit(seed ^ 0x6761_7264_656e_5f61);
    let phase_b = hash_unit(seed ^ 0x6761_7264_656e_5f62);
    let broad = ((east / 17.0 + north / 29.0 + phase_a) * TAU).sin();
    let cross = ((east / 31.0 - north / 13.0 + phase_b) * TAU).cos();
    let value = (0.5 + broad * 0.29 + cross * 0.21).clamp(0.0, 1.0);
    if value >= 0.64 {
        GardenPatch::FlowerBed
    } else if value >= 0.34 {
        GardenPatch::MaintainedGreen
    } else if value >= 0.16 {
        GardenPatch::ShrubProxy
    } else {
        GardenPatch::Gap
    }
}

/// Conservative ecological score. A mapped meadow is trusted as vegetation
/// semantics but still obeys physical climate gates; unclassified natural
/// grass requires a narrower, wetter, gentler envelope.
pub fn wildflower_suitability(route: WildflowerRoute, environment: WildflowerEnvironment) -> f32 {
    if !environment.temp_c.is_finite()
        || !environment.elevation_m.is_finite()
        || !environment.slope_rad.is_finite()
        || environment.temp_c < -1.0
        || environment.temp_c > 32.0
        || environment.elevation_m > 4_200.0
        || environment.snow_exposure >= 0.45
    {
        return 0.0;
    }

    let moisture = ((environment.precipitation_mm_day - 0.25) / 2.5)
        .clamp(0.0, 1.0)
        .max(((environment.humidity - 0.34) / 0.42).clamp(0.0, 1.0))
        .max(if environment.near_water { 0.72 } else { 0.0 });
    let drought = environment.aridity.clamp(0.0, 1.0);
    let alpine = environment.elevation_m >= 1_400.0 && environment.temp_c <= 13.0;
    let valley =
        environment.near_water && environment.relief_m >= 8.0 && environment.relief_m <= 190.0;
    let (slope_limit, relief_limit, drought_limit, moisture_floor) = match route {
        WildflowerRoute::MappedMeadow => (0.34, 220.0, 0.78, 0.18),
        WildflowerRoute::NaturalGrass => (0.22, 130.0, 0.62, 0.34),
    };
    if environment.slope_rad > slope_limit
        || environment.relief_m > relief_limit
        || drought > drought_limit
        || moisture < moisture_floor
    {
        return 0.0;
    }

    let slope_score = 1.0 - environment.slope_rad / slope_limit;
    let drought_score = 1.0 - drought / drought_limit;
    let climate_score = (1.0 - ((environment.temp_c - 14.0).abs() / 19.0)).clamp(0.15, 1.0);
    let terrain_bonus = if alpine {
        0.18
    } else if valley {
        0.12
    } else {
        0.0
    };
    let route_scale = match route {
        WildflowerRoute::MappedMeadow => 1.0,
        WildflowerRoute::NaturalGrass => 0.70,
    };
    ((0.30 * moisture + 0.24 * slope_score + 0.20 * drought_score + 0.16 * climate_score)
        * route_scale
        + terrain_bonus)
        .clamp(0.0, 1.0)
}

pub fn wildflower_patch_exists_at(
    seed: u64,
    cell: CellId,
    fx: f64,
    fz: f64,
    route: WildflowerRoute,
    suitability: f32,
) -> bool {
    if suitability <= 0.0 {
        return false;
    }
    let (east, north) = canonical_face_m(cell, fx, fz);
    let slow = ((east / 41.0 + north / 67.0 + hash_unit(seed ^ 0x7769_6c64_5f73_6c6f)) * TAU).sin();
    let cross =
        ((east / 97.0 - north / 37.0 + hash_unit(seed ^ 0x7769_6c64_5f63_726f)) * TAU).cos();
    let patch = (0.5 + 0.31 * slow + 0.19 * cross).clamp(0.0, 1.0) as f32;
    let threshold = match route {
        WildflowerRoute::MappedMeadow => 0.72 - suitability * 0.46,
        WildflowerRoute::NaturalGrass => 0.90 - suitability * 0.30,
    };
    patch >= threshold
}

/// A fixed L13 ecological identity keeps parent/child descriptors compatible
/// while allowing broad natural mosaics to vary every few kilometres.
pub fn ecological_region_seed(cell: CellId) -> u64 {
    const REGION_LEVEL: u8 = 13;
    let shift = cell.level.saturating_sub(REGION_LEVEL);
    let i = cell.i >> shift;
    let j = cell.j >> shift;
    splitmix(
        (u64::from(cell.face.0) << 58)
            ^ (u64::from(i) << 29)
            ^ u64::from(j)
            ^ 0x6563_6f5f_666c_6f72,
    )
}

pub fn crop_variation(
    seed: u64,
    candidate_index: u32,
    canonical_east_m: f64,
    canonical_north_m: f64,
) -> CropVariation {
    let candidate =
        seed ^ u64::from(candidate_index).wrapping_mul(0x9e37_79b9) ^ 0x7661_7269_6174_696f;
    let individual = hash_unit(candidate) as f32;
    let slow = ((canonical_east_m / 47.0
        + canonical_north_m / 71.0
        + hash_unit(seed ^ 0x6865_6967_6874_5f76))
        * TAU)
        .sin() as f32;
    let patch = ((canonical_east_m / 113.0 - canonical_north_m / 89.0
        + hash_unit(seed ^ 0x7061_7463_685f_7661))
        * TAU)
        .cos() as f32;
    CropVariation {
        height_scale: (0.88 + individual * 0.18 + slow * 0.045).clamp(0.82, 1.12),
        shade: (0.42 + individual * 0.34 + patch * 0.12).clamp(0.22, 0.88),
        exists: hash_unit(candidate ^ 0x6578_6973_7473_5f76) as f32,
    }
}

pub fn field_row_phase(seed: u64) -> f32 {
    hash_unit(seed ^ 0x726f_775f_7068_6173) as f32
}

pub fn field_row_perturbation_phase(seed: u64) -> f32 {
    hash_unit(seed ^ 0x726f_775f_776f_6262) as f32
}

pub fn crop_row_spacing_m(seed: u64) -> f32 {
    let spacing_jitter = (hash_unit(seed ^ 0x7370_6163_696e_675f) as f32 - 0.5) * 0.16;
    // This is an aggregate visual row, not an individual drill line. Tight
    // spacing keeps ordinary crops visually denser than commercial flowers.
    0.86 * (1.0 + spacing_jitter)
}

pub fn flower_row_spacing_m(seed: u64) -> f32 {
    let spacing_jitter = (hash_unit(seed ^ 0x7370_6163_696e_675f) as f32 - 0.5) * 0.16;
    3.6 * (1.0 + spacing_jitter)
}

pub fn flower_band_spacing_m(seed: u64) -> f32 {
    if seed & 0xffff_0000_0000_0000 == CURATED_FIELD_SEED_PREFIX
        && let Some(id) = FlowerDestinationId::from_u8(seed as u8)
    {
        return flower_destination(id).strip_width_m * 3.0;
    }
    let palette = (splitmix(seed ^ 0x7061_6c65_7474_655f) % 4) as f32;
    34.0 + palette * 7.0
}

pub const fn curated_field_seed(id: FlowerDestinationId) -> u64 {
    CURATED_FIELD_SEED_PREFIX | id as u64
}

pub fn curated_archetype(destination: &FlowerDestination) -> FloraArchetype {
    match destination.pattern {
        FlowerPattern::ParallelStrips
        | FlowerPattern::AlternatingStrips
        | FlowerPattern::BroadColorBlocks
        | FlowerPattern::Terraces => FloraArchetype::FlowerCrop,
        FlowerPattern::FormalBeds => FloraArchetype::OrnamentalBed,
        FlowerPattern::DenseNaturalMosaic | FlowerPattern::SparseSuperbloomClusters => {
            FloraArchetype::MeadowWildflowers
        }
    }
}

pub fn curated_region_kind(destination: &FlowerDestination) -> FloraRegionKind {
    match curated_archetype(destination) {
        FloraArchetype::FlowerCrop => FloraRegionKind::Farmland,
        FloraArchetype::OrnamentalBed => FloraRegionKind::Garden,
        FloraArchetype::MeadowWildflowers => FloraRegionKind::Meadow,
        _ => unreachable!("curated flower destinations only use flower archetypes"),
    }
}

pub fn curated_field_surface(destination: &FlowerDestination) -> FieldSurface {
    let turns = (f64::from(destination.heading_deg).to_radians() / TAU).rem_euclid(1.0);
    let ordered = matches!(
        destination.pattern,
        FlowerPattern::ParallelStrips
            | FlowerPattern::AlternatingStrips
            | FlowerPattern::BroadColorBlocks
            | FlowerPattern::Terraces
    );
    FieldSurface {
        kind: FieldSurfaceKind::FlowerField,
        palette: 0,
        row_heading: (turns * 65_536.0).round() as u16,
        row_spacing_m: if ordered {
            flower_row_spacing_m(curated_field_seed(destination.id))
        } else {
            0.0
        },
        row_width: match destination.pattern {
            FlowerPattern::ParallelStrips | FlowerPattern::AlternatingStrips => 0.70,
            FlowerPattern::BroadColorBlocks => 0.82,
            FlowerPattern::Terraces => 0.58,
            FlowerPattern::FormalBeds => 0.54,
            FlowerPattern::DenseNaturalMosaic | FlowerPattern::SparseSuperbloomClusters => 0.0,
        },
    }
}

pub fn curated_local_phase_shift(destination: &FlowerDestination) -> f32 {
    let southern = destination_center_lat_deg(destination) < 0.0;
    let peak = destination.calendar.local_peak_phase(southern);
    (0.42 - peak + 0.5).rem_euclid(1.0) - 0.5
}

pub fn curated_phenology_offset(destination: &FlowerDestination) -> u8 {
    (((curated_local_phase_shift(destination) / 0.5 + 1.0) * 127.5).round() as i32).clamp(0, 255)
        as u8
}

pub fn curated_palette_linear(style: u8) -> Option<[[f32; 3]; 3]> {
    let destination = flower_destination(FlowerDestinationId::from_u8(style)?);
    Some(destination.palette.map(|Rgb8(r, g, b)| {
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        ]
    }))
}

/// The generic flower colors shared by the terrain equation and the GPU
/// flower-head shader. The first two colors are authored in `palette::field`;
/// the third is their restrained midpoint, so a three-band grammar does not
/// invent a second palette at render time.
pub fn generic_flower_palette_linear(palette: u8) -> [[f32; 3]; 3] {
    let (_, first, second) =
        crate::palette::field::FLOWER[usize::from(palette) % crate::palette::field::FLOWER.len()];
    let normalize = |rgb: [f32; 3]| rgb.map(|value| value / 255.0);
    let first = normalize(first);
    let second = normalize(second);
    [
        first,
        second,
        std::array::from_fn(|channel| (first[channel] + second[channel]) * 0.5),
    ]
}

fn destination_center_lat_deg(destination: &FlowerDestination) -> f64 {
    use crate::flower_destinations::DestinationShape;
    match destination.influence {
        DestinationShape::Circle { lat_deg, .. }
        | DestinationShape::OrientedRectangle { lat_deg, .. } => lat_deg,
        DestinationShape::Polygon { vertices_deg } => {
            vertices_deg.iter().map(|point| point[0]).sum::<f64>() / vertices_deg.len() as f64
        }
    }
}

pub fn curated_ecological_suitability(
    destination: &FlowerDestination,
    environment: WildflowerEnvironment,
) -> f32 {
    debug_assert_eq!(destination.mode, DestinationMode::EcologicalBoost);
    let mut suitability = wildflower_suitability(WildflowerRoute::NaturalGrass, environment);
    if destination.calendar.moisture_gated {
        let moisture_normal = (environment.precipitation_mm_day / 1.2)
            .max(environment.humidity)
            .clamp(0.0, 1.0);
        suitability *= moisture_normal;
        // Desert superblooms need immutable potential coverage even though
        // their annual climate normal is dry. Live drought/snow and the
        // curated bloom calendar still decide whether those sparse candidates
        // become visible; generation never turns the whole region on.
        let rare_wet_year_potential = smoothstep(-5.0, 8.0, environment.temp_c)
            * (1.0 - smoothstep(34.0, 42.0, environment.temp_c))
            * (1.0 - smoothstep(0.24, 0.52, environment.slope_rad))
            * (1.0 - smoothstep(180.0, 420.0, environment.relief_m))
            * 0.24;
        suitability = suitability.max(rare_wet_year_potential);
    }
    if destination.calendar.snowmelt_gated {
        let alpine = smoothstep(850.0, 1_900.0, environment.elevation_m)
            * (1.0 - smoothstep(3_700.0, 4_500.0, environment.elevation_m));
        suitability *= alpine * (1.0 - environment.snow_exposure.clamp(0.0, 1.0) * 0.55);
    }
    suitability.clamp(0.0, 1.0)
}

pub fn managed_row_spacing_m(archetype: FloraArchetype, seed: u64) -> Option<f32> {
    let jitter = (hash_unit(seed ^ 0x7370_6163_696e_675f) as f32 - 0.5) * 0.16;
    match archetype {
        FloraArchetype::Orchard => Some(8.5 * (1.0 + jitter)),
        FloraArchetype::Vineyard => Some(2.8 * (1.0 + jitter)),
        _ => None,
    }
}

pub fn managed_along_spacing_m(archetype: FloraArchetype, seed: u64) -> Option<f32> {
    let jitter = (hash_unit(seed ^ 0x6d61_6e61_6765_645f) as f32 - 0.5) * 0.12;
    let authored = match archetype {
        FloraArchetype::Orchard => 6.2 * (1.0 + jitter),
        FloraArchetype::Vineyard => 2.2 * (1.0 + jitter),
        _ => return None,
    };
    // The GPU stores this period as integer centimetres. Quantize before
    // deriving the canonical origin phase so cell-local stepping cannot use
    // a different period from the world-space planting grid.
    Some((authored * 100.0).round() * 0.01)
}

fn reduced_row_phase(cell: CellId, row_heading_rad: f32, seed: u64, spacing_m: f32) -> f32 {
    let (east, north) = canonical_face_m(cell, 0.0, 0.0);
    let heading = f64::from(row_heading_rad);
    let across = -east * heading.sin() + north * heading.cos();
    (across / f64::from(spacing_m) + f64::from(field_row_phase(seed))).rem_euclid(1.0) as f32
}

/// Pruned tea rows leave picking alleys and keep the same curved-row phase
/// through independently generated terrain cells. The third word is a 48 m
/// along-row phase, matching ordinary crops without a new GPU instance layout.
pub fn tea_row_gpu_data(cell: CellId, row_heading_rad: f32, seed: u64) -> [u32; 3] {
    let spacing = 4.2_f32;
    [
        spacing.to_bits(),
        reduced_row_phase(cell, row_heading_rad, seed, spacing).to_bits(),
        crop_row_cell_phases(cell, row_heading_rad, seed)[1].to_bits(),
    ]
}

fn reduced_along_phase(cell: CellId, row_heading_rad: f32, seed: u64, spacing_m: f32) -> f32 {
    let (east, north) = canonical_face_m(cell, 0.0, 0.0);
    let heading = f64::from(row_heading_rad);
    let along = east * heading.cos() + north * heading.sin();
    (along / f64::from(spacing_m) + hash_unit(seed ^ 0x616c_6f6e_675f_7068)).rem_euclid(1.0) as f32
}

fn reduced_flower_band_phase(cell: CellId, row_heading_rad: f32, seed: u64) -> f32 {
    let (east, north) = canonical_face_m(cell, 0.0, 0.0);
    let heading = f64::from(row_heading_rad);
    let across = -east * heading.sin() + north * heading.cos();
    (across / f64::from(flower_band_spacing_m(seed)) + hash_unit(seed ^ 0x6261_6e64_7068_6173))
        .rem_euclid(1.0) as f32
}

/// Reduced row phases at local `(fx=0, fz=0)` for one owning cell. Passing
/// these to the GPU avoids subtracting multi-million-metre face coordinates
/// in f32 while remaining exactly compatible with the f64 surface equation.
pub fn crop_row_cell_phases(cell: CellId, row_heading_rad: f32, seed: u64) -> [f32; 2] {
    let (east, north) = canonical_face_m(cell, 0.0, 0.0);
    let heading = f64::from(row_heading_rad);
    let along = east * heading.cos() + north * heading.sin();
    let across = -east * heading.sin() + north * heading.cos();
    [
        (across / f64::from(crop_row_spacing_m(seed)) + f64::from(field_row_phase(seed)))
            .rem_euclid(1.0) as f32,
        (along / 48.0 + f64::from(field_row_perturbation_phase(seed))).rem_euclid(1.0) as f32,
    ]
}

/// Reduced cross-row and along-row phases for an orchard or vineyard. These
/// are derived from canonical face metres, so separately generated cells and
/// parent/child terrain cuts snap to the same managed planting grid.
pub fn managed_row_cell_phases(
    cell: CellId,
    archetype: FloraArchetype,
    row_heading_rad: f32,
    seed: u64,
) -> Option<[f32; 2]> {
    let row_spacing = managed_row_spacing_m(archetype, seed)?;
    let along_spacing = managed_along_spacing_m(archetype, seed)?;
    Some([
        reduced_row_phase(cell, row_heading_rad, seed, row_spacing),
        reduced_along_phase(cell, row_heading_rad, seed, along_spacing),
    ])
}

/// Three alignment words carried in the fixed-size GPU instance tail. Crop
/// grammars retain their two f32 phases. Flower crops and managed perennials
/// pack a second centimetre spacing plus quantized phase into the final word,
/// keeping all representations on the same field-wide equations.
pub fn flora_row_gpu_data(
    cell: CellId,
    archetype: FloraArchetype,
    row_heading_rad: f32,
    seed: u64,
) -> [u32; 3] {
    if archetype == FloraArchetype::FlowerCrop {
        let row_spacing = flower_row_spacing_m(seed);
        let band_spacing = flower_band_spacing_m(seed);
        let band_phase = reduced_flower_band_phase(cell, row_heading_rad, seed);
        let packed_band = ((band_phase * 65_536.0).floor() as u32).min(u16::MAX as u32) << 16
            | (band_spacing * 100.0).round() as u32 & 0xffff;
        [
            row_spacing.to_bits(),
            reduced_row_phase(cell, row_heading_rad, seed, row_spacing).to_bits(),
            packed_band,
        ]
    } else if let Some(phases) = managed_row_cell_phases(cell, archetype, row_heading_rad, seed) {
        let along_spacing =
            managed_along_spacing_m(archetype, seed).expect("managed phase requires spacing");
        let packed_along = ((phases[1] * 65_536.0).floor() as u32).min(u16::MAX as u32) << 16
            | (along_spacing * 100.0).round() as u32 & 0xffff;
        [
            managed_row_spacing_m(archetype, seed)
                .expect("managed phase requires managed row spacing")
                .to_bits(),
            phases[0].to_bits(),
            packed_along,
        ]
    } else {
        let phases = crop_row_cell_phases(cell, row_heading_rad, seed);
        [
            crop_row_spacing_m(seed).to_bits(),
            phases[0].to_bits(),
            phases[1].to_bits(),
        ]
    }
}

/// Signed cross-row displacement from a canonical face point to its authored
/// crop row. This is used in tests and mirrored by the GPU candidate path.
pub fn crop_row_offset_m(
    seed: u64,
    row_heading_rad: f32,
    canonical_east_m: f64,
    canonical_north_m: f64,
    within_row_offset_m: f32,
) -> f32 {
    let heading = f64::from(row_heading_rad);
    let along = canonical_east_m * heading.cos() + canonical_north_m * heading.sin();
    let across = -canonical_east_m * heading.sin() + canonical_north_m * heading.cos();
    let spacing = f64::from(crop_row_spacing_m(seed));
    let wobble =
        ((along / 48.0 + f64::from(field_row_perturbation_phase(seed))) * TAU).sin() * 0.16;
    let phase = f64::from(field_row_phase(seed));
    let center = (((across - wobble) / spacing + phase).floor() + 0.5 - phase) * spacing + wobble;
    (center + f64::from(within_row_offset_m.clamp(-0.34, 0.34)) - across)
        .clamp(-spacing * 0.50, spacing * 0.50) as f32
}

/// Select the immutable surface identity. The catalogue is bounded: farmland
/// treatments are visual grammars, not crop
/// species claims.
pub fn select_field_surface(kind: FloraRegionKind, seed: u64) -> FieldSurface {
    let surface_kind = match kind {
        FloraRegionKind::Garden => FieldSurfaceKind::FlowerField,
        FloraRegionKind::Orchard => FieldSurfaceKind::Orchard,
        FloraRegionKind::Vineyard => FieldSurfaceKind::Vineyard,
        FloraRegionKind::Meadow => FieldSurfaceKind::Meadow,
        FloraRegionKind::Farmland => FieldSurfaceKind::Crop,
    };
    let palette_count = match surface_kind {
        FieldSurfaceKind::Crop => 4,
        FieldSurfaceKind::FlowerField => 4,
        FieldSurfaceKind::Orchard => 3,
        FieldSurfaceKind::Vineyard => 3,
        FieldSurfaceKind::Meadow => 3,
        FieldSurfaceKind::Dormant => 3,
        FieldSurfaceKind::Harvested => 3,
    };
    let palette = (splitmix(seed ^ 0x7061_6c65_7474_655f) % palette_count) as u8;
    let row_heading = (splitmix(seed ^ 0x726f_775f_6865_6164) >> 48) as u16;
    let (spacing, width) = match surface_kind {
        FieldSurfaceKind::Crop => (crop_row_spacing_m(seed), 0.58),
        FieldSurfaceKind::FlowerField => (flower_row_spacing_m(seed), 0.68),
        FieldSurfaceKind::Orchard => (
            managed_row_spacing_m(FloraArchetype::Orchard, seed).expect("orchard spacing"),
            0.34,
        ),
        FieldSurfaceKind::Vineyard => (
            managed_row_spacing_m(FloraArchetype::Vineyard, seed).expect("vineyard spacing"),
            0.30,
        ),
        FieldSurfaceKind::Meadow => (0.0, 0.0),
        FieldSurfaceKind::Dormant => (crop_row_spacing_m(seed), 0.48),
        FieldSurfaceKind::Harvested => (crop_row_spacing_m(seed), 0.24),
    };
    FieldSurface {
        kind: surface_kind,
        palette,
        row_heading,
        row_spacing_m: spacing,
        row_width: width,
    }
}

/// Canonical coordinates in metres on a cube face.  The formula is a pure
/// function of global quadtree coordinates, so a point expressed through a
/// parent or child cell is bit-identical before the final multiplication.
fn canonical_face_m(cell: CellId, fx: f64, fz: f64) -> (f64, f64) {
    let edge = (1u64 << cell.level) as f64;
    let u = (cell.i as f64 + fx) / edge;
    // ChunkFrame local +z follows cube-face -v.
    let v = (cell.j as f64 + 1.0 - fz) / edge;
    (
        (u - 0.5) * NOMINAL_FACE_METERS,
        (v - 0.5) * NOMINAL_FACE_METERS,
    )
}

#[inline]
fn pulse(phase: f64, width: f32) -> f32 {
    let centered = (phase.rem_euclid(1.0) - 0.5).abs() * 2.0;
    let half_soft_edge = 0.08f64;
    let edge = f64::from(width.clamp(0.0, 1.0));
    (1.0 - ((centered - edge) / half_soft_edge).clamp(0.0, 1.0)) as f32
}

#[inline]
fn resolved_contrast(spacing_m: f32, sample_m: f64) -> f32 {
    if spacing_m <= 0.0 {
        return 0.0;
    }
    // Analytic mip policy: unresolved rows converge to the palette mean
    // instead of aliasing or changing phase at a terrain-LOD boundary.
    let ratio = sample_m / f64::from(spacing_m);
    ((1.35 - ratio) / 0.95).clamp(0.0, 1.0) as f32
}

/// Evaluate the shared field equation at a fractional position in a terrain
/// cell. `sample_m` is the surface footprint used to attenuate sub-pixel rows.
pub fn sample_field_surface(
    surface: FieldSurface,
    seed: u64,
    cell: CellId,
    fx: f64,
    fz: f64,
    sample_m: f64,
) -> FieldSurfaceSample {
    let (east, north) = canonical_face_m(cell, fx, fz);
    let heading = surface.heading_rad();
    let along_rows = east * heading.cos() + north * heading.sin();
    let across_rows = -east * heading.sin() + north * heading.cos();
    let phase_offset = f64::from(field_row_phase(seed));

    let perturbed_across_rows = if matches!(
        surface.kind,
        FieldSurfaceKind::Crop | FieldSurfaceKind::Dormant | FieldSurfaceKind::Harvested
    ) {
        across_rows
            - ((along_rows / 48.0 + f64::from(field_row_perturbation_phase(seed))) * TAU).sin()
                * 0.16
    } else {
        across_rows
    };
    let row = if surface.row_spacing_m > 0.0 {
        let raw = pulse(
            perturbed_across_rows / f64::from(surface.row_spacing_m) + phase_offset,
            surface.row_width,
        );
        let mean = surface.row_width.clamp(0.0, 1.0);
        mean + (raw - mean) * resolved_contrast(surface.row_spacing_m, sample_m)
    } else {
        0.5
    };

    // Flower colour strips are tens of metres wide and therefore survive
    // aerial views before individual cultivation rows resolve.  Other field
    // kinds use the same equation only as restrained broad shading.
    let band_spacing = if surface.kind == FieldSurfaceKind::FlowerField {
        f64::from(flower_band_spacing_m(seed))
    } else {
        46.0
    };
    let band_phase =
        (across_rows / band_spacing + hash_unit(seed ^ 0x6261_6e64_7068_6173)).rem_euclid(1.0);
    let band_raw = pulse(
        band_phase,
        if surface.kind == FieldSurfaceKind::FlowerField {
            0.52
        } else {
            0.64
        },
    );
    let band_mean = if surface.kind == FieldSurfaceKind::FlowerField {
        0.52
    } else {
        0.64
    };
    let band =
        band_mean + (band_raw - band_mean) * resolved_contrast(band_spacing as f32, sample_m);

    // Two continuous waves give meadows a clustered read without cell-local
    // noise and keep variation restrained inside cultivated rows.
    let slow =
        ((along_rows / 73.0 + across_rows / 109.0 + hash_unit(seed ^ 0x7661_725f_736c_6f77)) * TAU)
            .sin();
    let cross =
        ((along_rows / 181.0 - across_rows / 61.0 + hash_unit(seed ^ 0x7661_725f_6372_6f73)) * TAU)
            .cos();
    let raw_variation = (0.5 + 0.25 * slow + 0.25 * cross).clamp(0.0, 1.0) as f32;
    let variation = 0.5 + (raw_variation - 0.5) * resolved_contrast(73.0, sample_m);

    FieldSurfaceSample {
        kind: surface.kind,
        palette: surface.palette,
        row,
        band,
        flower_variant: (band_phase * 3.0).floor().clamp(0.0, 2.0) as u8,
        variation,
    }
}

/// Convenience for the center of one generated terrain column.
pub fn sample_field_column(
    surface: FieldSurface,
    seed: u64,
    cell: CellId,
    x: usize,
    z: usize,
) -> FieldSurfaceSample {
    sample_field_surface(
        surface,
        seed,
        cell,
        (x as f64 + 0.5) / N as f64,
        (z as f64 + 0.5) / N as f64,
        cell.size_m() / N as f64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::cubesphere::Face;

    #[test]
    fn kind_seed_selects_one_stable_field_identity() {
        for kind in [
            FloraRegionKind::Farmland,
            FloraRegionKind::Meadow,
            FloraRegionKind::Garden,
            FloraRegionKind::Orchard,
            FloraRegionKind::Vineyard,
        ] {
            let a = select_field_surface(kind, 0x1234_5678_9abc_def0);
            let b = select_field_surface(kind, 0x1234_5678_9abc_def0);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn ordinary_farmland_never_invents_commercial_flower_fields() {
        for seed in 1..10_000 {
            assert_eq!(
                select_field_surface(FloraRegionKind::Farmland, seed).kind,
                FieldSurfaceKind::Crop
            );
        }
        assert_eq!(
            select_field_surface(FloraRegionKind::Garden, 1).kind,
            FieldSurfaceKind::FlowerField
        );
    }

    fn temperate_crop_environment() -> CropEnvironment {
        CropEnvironment {
            lat_deg: 47.5,
            lon_deg: 8.5,
            temp_c: 13.5,
            elevation_m: 420.0,
            slope_rad: 0.04,
            relief_m: 18.0,
            aridity: 0.34,
            precipitation_mm_day: 2.1,
            humidity: 0.66,
            near_water: false,
        }
    }

    #[test]
    fn crop_catalogue_authors_every_crop_at_every_geometry_lod() {
        for archetype in [
            FloraArchetype::Cereal,
            FloraArchetype::BroadleafRows,
            FloraArchetype::TallRows,
            FloraArchetype::LowDenseCrop,
        ] {
            let close = crop_form(archetype, CropRepresentation::Close).unwrap();
            let middle = crop_form(archetype, CropRepresentation::Middle).unwrap();
            let coarse = crop_form(archetype, CropRepresentation::CoarseTuft).unwrap();
            assert!(close.height_min_m < close.height_max_m);
            assert!(close.stem_width_m < close.leaf_width_m);
            assert!(middle.head_diameter_m > close.head_diameter_m);
            assert!(coarse.head_diameter_m > middle.head_diameter_m);
            assert_eq!(crop_form(archetype, CropRepresentation::SurfaceOnly), None);
        }
        assert_eq!(
            crop_form(FloraArchetype::FlowerCrop, CropRepresentation::Close),
            None
        );
        let flower_density = descriptor_density(FloraArchetype::FlowerCrop);
        for archetype in [
            FloraArchetype::Cereal,
            FloraArchetype::BroadleafRows,
            FloraArchetype::TallRows,
            FloraArchetype::LowDenseCrop,
        ] {
            assert!(
                descriptor_density(archetype) > flower_density,
                "{archetype:?} must be fuller than a generic flower field"
            );
        }
    }

    #[test]
    fn catalogue_authors_every_flower_at_every_geometry_lod() {
        for archetype in [
            FloraArchetype::FlowerCrop,
            FloraArchetype::OrnamentalBed,
            FloraArchetype::MeadowWildflowers,
        ] {
            let close = flower_form(archetype, FlowerRepresentation::Close).unwrap();
            let middle = flower_form(archetype, FlowerRepresentation::MiddleCluster).unwrap();
            let coarse = flower_form(archetype, FlowerRepresentation::CoarseTuft).unwrap();
            assert!(close.height_min_m < close.height_max_m);
            assert!(close.stem_width_m < close.leaf_width_m);
            assert!(middle.head_diameter_m > close.head_diameter_m);
            assert!(coarse.head_diameter_m > middle.head_diameter_m);
            assert_eq!(
                flower_form(archetype, FlowerRepresentation::SurfaceOnly),
                None
            );
        }
        assert_eq!(
            flower_form(FloraArchetype::Cereal, FlowerRepresentation::Close),
            None
        );
    }

    #[test]
    fn catalogue_authors_compact_managed_forms_at_every_geometry_lod() {
        let orchard_close = orchard_form(ManagedRepresentation::Close).unwrap();
        let orchard_middle = orchard_form(ManagedRepresentation::MiddleCluster).unwrap();
        let orchard_coarse = orchard_form(ManagedRepresentation::CoarseRow).unwrap();
        assert!(orchard_close.trunk_width_m < orchard_close.crown_across_m);
        assert!(orchard_close.trunk_height_m + orchard_close.crown_height_m <= 3.5);
        assert!(orchard_middle.crown_along_m > orchard_close.crown_along_m);
        assert!(orchard_coarse.crown_along_m > orchard_middle.crown_along_m);
        assert_eq!(orchard_form(ManagedRepresentation::SurfaceOnly), None);

        let vine_close = vineyard_form(ManagedRepresentation::Close).unwrap();
        let vine_middle = vineyard_form(ManagedRepresentation::MiddleCluster).unwrap();
        let vine_coarse = vineyard_form(ManagedRepresentation::CoarseRow).unwrap();
        assert!(vine_close.mass_bottom_m + vine_close.mass_height_m < 1.6);
        assert!(vine_close.mass_across_m < vine_close.mass_along_m);
        assert!(vine_close.post_period > 0);
        assert!(vine_middle.post_period > vine_close.post_period);
        assert_eq!(vine_coarse.post_period, 0);
        assert!(vine_coarse.mass_along_m > vine_middle.mass_along_m);
        assert_eq!(vineyard_form(ManagedRepresentation::SurfaceOnly), None);

        assert!((220..u8::MAX).contains(&descriptor_density(FloraArchetype::Orchard)));
        assert!((220..u8::MAX).contains(&descriptor_density(FloraArchetype::Vineyard)));
    }

    #[test]
    fn every_flower_head_survives_each_active_lod_boundary() {
        let tan_half_fov = (60.0f32.to_radians() * 0.5).tan();
        for archetype in [
            FloraArchetype::FlowerCrop,
            FloraArchetype::OrnamentalBed,
            FloraArchetype::MeadowWildflowers,
        ] {
            for (representation, start, end) in [
                (FlowerRepresentation::Close, 2.0, CROP_CLOSE_END_M),
                (
                    FlowerRepresentation::MiddleCluster,
                    CROP_CLOSE_END_M,
                    CROP_MIDDLE_END_M,
                ),
                (
                    FlowerRepresentation::CoarseTuft,
                    CROP_MIDDLE_END_M,
                    CROP_COARSE_END_M,
                ),
            ] {
                let form = flower_form(archetype, representation).unwrap();
                for boundary in [start, end] {
                    let pixels = surviving_flower_head_pixels(form, boundary, 720.0, tan_half_fov);
                    assert!(
                        pixels >= FLOWER_HEAD_MIN_PIXELS - 1.0e-4,
                        "{archetype:?} {representation:?} projected to {pixels} px at {boundary} m"
                    );
                    let authored = projected_diameter_pixels(
                        form.head_diameter_m,
                        boundary,
                        720.0,
                        tan_half_fov,
                    );
                    assert!(pixels <= authored * form.max_head_enlargement + 1.0e-4);
                }
            }
        }
    }

    #[test]
    fn every_crop_head_survives_its_active_lod_boundary() {
        let tan_half_fov = (60.0f32.to_radians() * 0.5).tan();
        for archetype in [
            FloraArchetype::Cereal,
            FloraArchetype::BroadleafRows,
            FloraArchetype::TallRows,
            FloraArchetype::LowDenseCrop,
        ] {
            for (representation, boundary) in [
                (CropRepresentation::Close, CROP_CLOSE_END_M),
                (CropRepresentation::Middle, CROP_MIDDLE_END_M),
                (CropRepresentation::CoarseTuft, CROP_COARSE_END_M),
            ] {
                let form = crop_form(archetype, representation).unwrap();
                let pixels = surviving_head_pixels(form, boundary, 720.0, tan_half_fov);
                assert!(
                    pixels >= CROP_HEAD_MIN_PIXELS - 1.0e-4,
                    "{archetype:?} {representation:?} projected to {pixels} px"
                );
                let authored =
                    projected_diameter_pixels(form.head_diameter_m, boundary, 720.0, tan_half_fov);
                assert!(
                    pixels <= authored * form.max_head_enlargement + 1.0e-4,
                    "{archetype:?} {representation:?} exceeded its enlargement cap"
                );
            }
        }
    }

    #[test]
    fn crop_inference_is_deterministic_bounded_and_climate_gated() {
        let temperate = temperate_crop_environment();
        for seed in 0..2_000 {
            let a = infer_crop_archetype(seed, temperate);
            let b = infer_crop_archetype(seed, temperate);
            assert_eq!(a, b);
            assert!(crop_archetype_is_plausible(a, temperate));
        }

        let mut warm = temperate;
        warm.lat_deg = 14.0;
        warm.lon_deg = 101.0;
        warm.temp_c = 25.0;
        warm.aridity = 0.22;
        warm.precipitation_mm_day = 4.2;
        warm.humidity = 0.82;
        assert!(
            (0..2_000).any(|seed| infer_crop_archetype(seed, warm) == FloraArchetype::TallRows)
        );

        let mut cold = temperate;
        cold.temp_c = 5.0;
        cold.lat_deg = 61.0;
        assert!(
            (0..2_000).all(|seed| infer_crop_archetype(seed, cold) != FloraArchetype::TallRows)
        );

        let mut impossible = warm;
        impossible.elevation_m = 4_700.0;
        assert_eq!(infer_crop_archetype(77, impossible), FloraArchetype::Cereal);

        let mut east_antimeridian = temperate;
        east_antimeridian.lon_deg = 179.9;
        let mut west_antimeridian = east_antimeridian;
        west_antimeridian.lon_deg = -179.9;
        assert_eq!(
            infer_crop_archetype(0x414e_5449_4d45_5249, east_antimeridian),
            infer_crop_archetype(0x414e_5449_4d45_5249, west_antimeridian)
        );
    }

    #[test]
    fn flower_surface_and_plant_archetype_cannot_diverge_at_climate_edges() {
        let suitable = temperate_crop_environment();
        assert!(commercial_flower_suitable(suitable));
        assert_eq!(
            descriptor_archetype(
                FloraRegionKind::Farmland,
                FieldSurfaceKind::FlowerField,
                91,
                suitable,
            ),
            FloraArchetype::FlowerCrop
        );
        for unsuitable in [
            CropEnvironment {
                aridity: 0.92,
                precipitation_mm_day: 0.12,
                humidity: 0.18,
                near_water: false,
                ..suitable
            },
            CropEnvironment {
                slope_rad: 0.42,
                ..suitable
            },
            CropEnvironment {
                temp_c: -4.0,
                ..suitable
            },
        ] {
            assert!(!commercial_flower_suitable(unsuitable));
            assert_eq!(
                descriptor_archetype(
                    FloraRegionKind::Farmland,
                    FieldSurfaceKind::FlowerField,
                    91,
                    unsuitable,
                ),
                FloraArchetype::FlowerCrop,
                "a stable flower surface must never fall back to crop heads"
            );
        }
    }

    #[test]
    fn garden_partition_is_stable_bounded_and_continuous() {
        let cell = CellId {
            face: Face(0),
            level: 19,
            i: 271_000,
            j: 191_000,
        };
        let seed = 0x4741_5244_454e_4636;
        let mut found = [false; 4];
        for z in 0..64 {
            for x in 0..64 {
                let patch = garden_patch_at(seed, cell, x as f64 / 63.0, z as f64 / 63.0);
                found[patch as usize] = true;
                assert_eq!(
                    patch,
                    garden_patch_at(seed, cell, x as f64 / 63.0, z as f64 / 63.0)
                );
            }
        }
        assert!(found.into_iter().all(|value| value));
        let right = CellId {
            i: cell.i + 1,
            ..cell
        };
        assert_eq!(
            garden_patch_at(seed, cell, 1.0, 0.37),
            garden_patch_at(seed, right, 0.0, 0.37)
        );
    }

    #[test]
    fn wildflowers_admit_suitable_valleys_and_alpine_meadows_but_reject_stress() {
        let valley = WildflowerEnvironment {
            temp_c: 14.0,
            elevation_m: 520.0,
            slope_rad: 0.05,
            relief_m: 72.0,
            aridity: 0.28,
            precipitation_mm_day: 2.4,
            humidity: 0.68,
            near_water: true,
            snow_exposure: 0.0,
        };
        let alpine = WildflowerEnvironment {
            temp_c: 6.0,
            elevation_m: 2_150.0,
            slope_rad: 0.12,
            relief_m: 110.0,
            aridity: 0.22,
            precipitation_mm_day: 1.8,
            humidity: 0.62,
            near_water: false,
            snow_exposure: 0.12,
        };
        assert!(wildflower_suitability(WildflowerRoute::NaturalGrass, valley) > 0.45);
        assert!(wildflower_suitability(WildflowerRoute::NaturalGrass, alpine) > 0.35);
        for rejected in [
            WildflowerEnvironment {
                aridity: 0.91,
                precipitation_mm_day: 0.08,
                humidity: 0.14,
                near_water: false,
                ..valley
            },
            WildflowerEnvironment {
                slope_rad: 0.46,
                ..valley
            },
            WildflowerEnvironment {
                snow_exposure: 0.82,
                ..alpine
            },
        ] {
            assert_eq!(
                wildflower_suitability(WildflowerRoute::MappedMeadow, rejected),
                0.0
            );
            assert_eq!(
                wildflower_suitability(WildflowerRoute::NaturalGrass, rejected),
                0.0
            );
        }

        let mapped = wildflower_suitability(WildflowerRoute::MappedMeadow, valley);
        let natural = wildflower_suitability(WildflowerRoute::NaturalGrass, valley);
        assert!(
            mapped > natural,
            "natural grass must be the conservative route"
        );
    }

    #[test]
    fn wildflower_patches_cluster_and_ecological_identity_survives_refinement() {
        let parent = CellId {
            face: Face(2),
            level: 17,
            i: 71_000,
            j: 82_000,
        };
        let child = parent.children()[2];
        assert_eq!(
            ecological_region_seed(parent),
            ecological_region_seed(child)
        );
        let seed = ecological_region_seed(parent);
        let mut kept = 0;
        for z in 0..32 {
            for x in 0..32 {
                kept += usize::from(wildflower_patch_exists_at(
                    seed,
                    parent,
                    (x as f64 + 0.5) / 32.0,
                    (z as f64 + 0.5) / 32.0,
                    WildflowerRoute::NaturalGrass,
                    0.80,
                ));
            }
        }
        assert!(kept > 0 && kept < 32 * 32, "patches must not be uniform");
    }

    #[test]
    fn crop_rows_and_within_field_variation_are_stable_and_restrained() {
        let seed = 0x465f_355f_524f_5753;
        assert!((0.75..=0.95).contains(&crop_row_spacing_m(seed)));
        assert!(crop_row_spacing_m(seed) < flower_row_spacing_m(seed));
        let heading = 0.71;
        let offset = crop_row_offset_m(seed, heading, 12_345.0, -8_765.0, 0.12);
        assert_eq!(
            offset,
            crop_row_offset_m(seed, heading, 12_345.0, -8_765.0, 0.12)
        );
        assert!(offset.abs() <= crop_row_spacing_m(seed) * 0.50 + 1.0e-5);
        let centered = crop_row_offset_m(seed, heading, 12_345.0, -8_765.0, 0.0);
        let adjusted_east = 12_345.0 - f64::from(heading.sin() * centered);
        let adjusted_north = -8_765.0 + f64::from(heading.cos() * centered);
        let along =
            adjusted_east * f64::from(heading.cos()) + adjusted_north * f64::from(heading.sin());
        let across =
            -adjusted_east * f64::from(heading.sin()) + adjusted_north * f64::from(heading.cos());
        let wobble =
            ((along / 48.0 + f64::from(field_row_perturbation_phase(seed))) * TAU).sin() * 0.16;
        let row_phase = (across - wobble) / f64::from(crop_row_spacing_m(seed))
            + f64::from(field_row_phase(seed));
        assert!((row_phase.rem_euclid(1.0) - 0.5).abs() < 1.0e-3);

        let cell = CellId {
            face: Face(3),
            level: 19,
            i: 271_828,
            j: 161_803,
        };
        let (fx, fz) = (0.37, 0.64);
        let (east, north) = canonical_face_m(cell, fx, fz);
        let expected = crop_row_offset_m(seed, heading, east, north, -0.08);
        let phases = crop_row_cell_phases(cell, heading, seed);
        let local_east = fx * cell.size_m();
        let local_north = -fz * cell.size_m();
        let local_along =
            local_east * f64::from(heading.cos()) + local_north * f64::from(heading.sin());
        let local_across =
            -local_east * f64::from(heading.sin()) + local_north * f64::from(heading.cos());
        let local_wobble = ((local_along / 48.0 + f64::from(phases[1])) * TAU).sin() * 0.16;
        let spacing = f64::from(crop_row_spacing_m(seed));
        let local_center = ((local_across - local_wobble) / spacing + f64::from(phases[0])).floor();
        let actual = ((local_center + 0.5 - f64::from(phases[0])) * spacing + local_wobble
            - 0.08
            - local_across)
            .clamp(-spacing * 0.5, spacing * 0.5) as f32;
        assert!((actual - expected).abs() < 2.0e-4);

        let a = crop_variation(seed, 42, 12_345.0, -8_765.0);
        let b = crop_variation(seed, 42, 12_345.0, -8_765.0);
        let c = crop_variation(seed, 43, 12_345.0, -8_765.0);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!((0.82..=1.12).contains(&a.height_scale));
        assert!((0.22..=0.88).contains(&a.shade));
        assert!((0.0..1.0).contains(&a.exists));
    }

    #[test]
    fn commercial_flower_gpu_rows_and_color_strips_match_the_surface_equation() {
        let cell = CellId {
            face: Face(4),
            level: 19,
            i: 301_004,
            j: 211_337,
        };
        let seed = 0x464c_4f57_4552_4636;
        let surface = select_field_surface(FloraRegionKind::Garden, seed);
        let generic_palette = generic_flower_palette_linear(surface.palette);
        assert_ne!(generic_palette[0], generic_palette[1]);
        assert_eq!(
            generic_palette[2],
            std::array::from_fn(|channel| {
                (generic_palette[0][channel] + generic_palette[1][channel]) * 0.5
            })
        );
        let heading = surface.heading_rad() as f32;
        let encoded = flora_row_gpu_data(cell, FloraArchetype::FlowerCrop, heading, seed);
        assert_eq!(f32::from_bits(encoded[0]), surface.row_spacing_m);
        let packed_band = encoded[2];
        assert!(
            ((packed_band & 0xffff) as f32 * 0.01 - flower_band_spacing_m(seed)).abs() <= 0.005
        );
        let encoded_phase = (packed_band >> 16) as f32 / 65_536.0;
        let expected_phase = reduced_flower_band_phase(cell, heading, seed);
        let phase_error = (encoded_phase - expected_phase)
            .abs()
            .min(1.0 - (encoded_phase - expected_phase).abs());
        assert!(phase_error <= 1.0 / 65_536.0);

        // The GPU derives the flower-head palette index from this packed
        // phase. Check representative positions against the CPU surface
        // sample so regional substrate and solid heads cannot select
        // unrelated authored colours again.
        let band_spacing = (packed_band & 0xffff) as f32 * 0.01;
        for (fx, fz) in [(0.11, 0.17), (0.29, 0.73), (0.53, 0.41), (0.88, 0.62)] {
            let local_east = fx as f32 * cell.size_m() as f32;
            let local_north = -(fz as f32) * cell.size_m() as f32;
            let across = -local_east * heading.sin() + local_north * heading.cos();
            let shader_phase = (across / band_spacing + encoded_phase).rem_euclid(1.0);
            let shader_variant = (shader_phase * 3.0).floor().clamp(0.0, 2.0) as u8;
            let sample = sample_field_surface(surface, seed, cell, fx, fz, 0.0);
            assert_eq!(
                sample.flower_variant, shader_variant,
                "surface/GPU palette mismatch at ({fx:.2}, {fz:.2})"
            );
        }
    }

    #[test]
    fn managed_grid_phase_period_matches_the_gpu_centimetre_encoding() {
        let cell = CellId {
            face: Face(1),
            level: 17,
            i: 51_200,
            j: 48_650,
        };
        for archetype in [FloraArchetype::Orchard, FloraArchetype::Vineyard] {
            for seed in 0..512 {
                let spacing = managed_along_spacing_m(archetype, seed).unwrap();
                let packed = flora_row_gpu_data(cell, archetype, 1.3, seed)[2];
                let decoded = (packed & 0xffff) as f32 * 0.01;
                assert_eq!(
                    spacing.to_bits(),
                    decoded.to_bits(),
                    "{archetype:?} seed {seed} changed the phase period during GPU packing"
                );
            }
        }
    }

    #[test]
    fn tea_picking_rows_keep_curvature_across_cells_and_lods() {
        let left = CellId {
            face: Face(1),
            level: 18,
            i: 102_400,
            j: 97_300,
        };
        let right = CellId {
            i: left.i + 1,
            ..left
        };
        let child = left.children()[3];
        let phase = |cell: CellId, heading: f32, seed: u64, fx: f64, fz: f64| {
            let words = tea_row_gpu_data(cell, heading, seed);
            let [spacing, across_origin, along_origin] = words.map(f32::from_bits).map(f64::from);
            let east = fx * cell.size_m();
            let north = -fz * cell.size_m();
            let heading = f64::from(heading);
            let along = east * heading.cos() + north * heading.sin();
            let across = -east * heading.sin() + north * heading.cos();
            ((across - ((along / 48.0 + along_origin) * TAU).sin() * 2.2) / spacing + across_origin)
                .rem_euclid(1.0)
        };
        for seed in 0..64 {
            let heading = (seed as f32 * 0.731).rem_euclid(std::f32::consts::TAU);
            for (a, b) in [
                (
                    phase(left, heading, seed, 1.0, 0.37),
                    phase(right, heading, seed, 0.0, 0.37),
                ),
                (
                    phase(left, heading, seed, 0.73, 0.21),
                    phase(child, heading, seed, 0.46, 0.42),
                ),
            ] {
                let error = (a - b).abs();
                assert!(
                    error.min(1.0 - error) < 0.0001,
                    "tea row discontinuity: {error}"
                );
            }
        }
    }

    #[test]
    fn managed_gpu_grid_matches_surfaces_and_is_continuous_across_cells_and_lods() {
        fn snapped_center(
            cell: CellId,
            archetype: FloraArchetype,
            heading: f32,
            seed: u64,
            fx: f64,
            fz: f64,
        ) -> (f64, f64) {
            let encoded = flora_row_gpu_data(cell, archetype, heading, seed);
            let row_spacing = f64::from(f32::from_bits(encoded[0]));
            let row_phase = f64::from(f32::from_bits(encoded[1]));
            let packed_along = encoded[2];
            let along_spacing = f64::from((packed_along & 0xffff) as f32 * 0.01);
            let along_phase = f64::from((packed_along >> 16) as f32 / 65_536.0);
            let local_east = fx * cell.size_m();
            let local_north = -fz * cell.size_m();
            let sin = f64::from(heading.sin());
            let cos = f64::from(heading.cos());
            let along = local_east * cos + local_north * sin;
            let across = -local_east * sin + local_north * cos;
            let along_center =
                ((along / along_spacing + along_phase).floor() + 0.5 - along_phase) * along_spacing;
            let across_center =
                ((across / row_spacing + row_phase).floor() + 0.5 - row_phase) * row_spacing;
            let along_delta = along_center - along;
            let across_delta = across_center - across;
            let (east, north) = canonical_face_m(cell, fx, fz);
            (
                east + cos * along_delta - sin * across_delta,
                north + sin * along_delta + cos * across_delta,
            )
        }

        let left = CellId {
            face: Face(1),
            level: 18,
            i: 102_400,
            j: 97_300,
        };
        let right = CellId {
            i: left.i + 1,
            ..left
        };
        let child = left.children()[3];
        let seed = 0x4d41_4e41_4745_4437;
        for archetype in [FloraArchetype::Orchard, FloraArchetype::Vineyard] {
            let surface_kind = if archetype == FloraArchetype::Orchard {
                FloraRegionKind::Orchard
            } else {
                FloraRegionKind::Vineyard
            };
            let surface = select_field_surface(surface_kind, seed);
            let heading = surface.heading_rad() as f32;
            assert_eq!(
                f32::from_bits(flora_row_gpu_data(left, archetype, heading, seed)[0]),
                surface.row_spacing_m
            );
            let a = snapped_center(left, archetype, heading, seed, 1.0, 0.37);
            let b = snapped_center(right, archetype, heading, seed, 0.0, 0.37);
            assert!((a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02);

            // Child 3 covers this point after accounting for local +z = -v.
            let parent_point = snapped_center(left, archetype, heading, seed, 0.73, 0.21);
            let child_point = snapped_center(child, archetype, heading, seed, 0.46, 0.42);
            assert!(
                (parent_point.0 - child_point.0).abs() < 0.02
                    && (parent_point.1 - child_point.1).abs() < 0.02,
                "{archetype:?} managed grid reorganized across terrain LOD"
            );
        }
    }

    #[test]
    fn row_equation_is_continuous_across_cell_borders() {
        let left = CellId {
            face: Face(0),
            level: 17,
            i: 42_000,
            j: 61_000,
        };
        let right = CellId {
            i: left.i + 1,
            ..left
        };
        let seed = 0x1020_3040_5060_7080;
        let surface = select_field_surface(FloraRegionKind::Vineyard, seed);
        let a = sample_field_surface(surface, seed, left, 1.0, 0.371, 0.25);
        let b = sample_field_surface(surface, seed, right, 0.0, 0.371, 0.25);
        assert_eq!(a, b);

        let lower = CellId {
            j: left.j + 1,
            ..left
        };
        let a = sample_field_surface(surface, seed, left, 0.419, 0.0, 0.25);
        let b = sample_field_surface(surface, seed, lower, 0.419, 1.0, 0.25);
        assert_eq!(a, b);
    }

    #[test]
    fn row_equation_has_one_identity_from_l11_through_l19() {
        let root = CellId {
            face: Face(4),
            level: 11,
            i: 731,
            j: 812,
        };
        let seed = 0xfedc_ba98_7654_3210;
        let surface = select_field_surface(FloraRegionKind::Garden, seed);
        let root_fx = 0.3125;
        let root_fz = 0.6875;
        let expected = sample_field_surface(surface, seed, root, root_fx, root_fz, 0.1);
        for level in 12..=19 {
            let scale = 1u32 << (level - root.level);
            let gx = root_fx * scale as f64;
            let gv = (1.0 - root_fz) * scale as f64;
            let cell = CellId {
                face: root.face,
                level,
                i: root.i * scale + gx.floor() as u32,
                j: root.j * scale + gv.floor() as u32,
            };
            let actual =
                sample_field_surface(surface, seed, cell, gx.fract(), 1.0 - gv.fract(), 0.1);
            assert_eq!(actual, expected, "level {level}");
        }
    }

    #[test]
    fn unresolved_rows_converge_to_the_palette_mean() {
        let cell = CellId {
            face: Face(0),
            level: 11,
            i: 1_024,
            j: 1_024,
        };
        let seed = 77;
        let surface = select_field_surface(FloraRegionKind::Orchard, seed);
        let sample = sample_field_surface(surface, seed, cell, 0.5, 0.5, 500.0);
        assert_eq!(sample.row, surface.row_width);
    }

    fn calendar_environment() -> CropEnvironment {
        CropEnvironment {
            lat_deg: 42.0,
            lon_deg: 12.0,
            temp_c: 15.0,
            elevation_m: 240.0,
            slope_rad: 0.05,
            relief_m: 18.0,
            aridity: 0.30,
            precipitation_mm_day: 1.6,
            humidity: 0.60,
            near_water: false,
        }
    }

    fn season_inputs(
        local_phase: f32,
        astronomical_phase: f32,
        temperature_c: f32,
        moisture: f32,
        drought: f32,
        snow: f32,
    ) -> FloraSeasonInputs {
        FloraSeasonInputs {
            local_phase,
            astronomical_phase,
            temperature_c,
            moisture,
            drought,
            snow,
        }
    }

    #[test]
    fn cultivation_calendar_selection_uses_climate_not_hemisphere_alone() {
        use CultivationCalendar::*;
        let temperate = calendar_environment();
        assert_eq!(
            select_cultivation_calendar(FloraArchetype::Cereal, temperate, 0),
            TemperateSummer
        );
        assert_eq!(
            select_cultivation_calendar(
                FloraArchetype::Cereal,
                CropEnvironment {
                    lat_deg: -55.0,
                    temp_c: 8.0,
                    ..temperate
                },
                0,
            ),
            CoolShortSeason
        );
        assert_eq!(
            select_cultivation_calendar(
                FloraArchetype::Cereal,
                CropEnvironment {
                    lat_deg: -34.0,
                    temp_c: 17.0,
                    aridity: 0.62,
                    precipitation_mm_day: 0.7,
                    ..temperate
                },
                0,
            ),
            MediterraneanWinter
        );
        assert_eq!(
            select_cultivation_calendar(
                FloraArchetype::TallRows,
                CropEnvironment {
                    lat_deg: 24.0,
                    temp_c: 25.0,
                    aridity: 0.24,
                    precipitation_mm_day: 4.1,
                    humidity: 0.78,
                    ..temperate
                },
                0,
            ),
            Monsoonal
        );
        assert_eq!(
            select_cultivation_calendar(
                FloraArchetype::BroadleafRows,
                CropEnvironment {
                    lat_deg: -8.0,
                    temp_c: 26.0,
                    aridity: 0.34,
                    precipitation_mm_day: 1.8,
                    humidity: 0.66,
                    ..temperate
                },
                0,
            ),
            TropicalMultiCycle
        );
        assert_eq!(
            select_cultivation_calendar(FloraArchetype::Orchard, temperate, 0),
            PerennialOrchard
        );
        assert_eq!(
            select_cultivation_calendar(FloraArchetype::Vineyard, temperate, 0),
            PerennialVineyard
        );
        assert_eq!(
            select_cultivation_calendar(FloraArchetype::FlowerCrop, temperate, 1),
            CuratedBloom
        );
    }

    #[test]
    fn local_phase_produces_opposite_northern_and_southern_timing() {
        let atlas = vearth_data::climate::ClimateAtlas::bundled();
        let astronomical_phase = 0.40;
        let northern_climate = atlas.sample(35.0_f64.to_radians(), 0.0, astronomical_phase);
        let southern_climate = atlas.sample(-35.0_f64.to_radians(), 0.0, astronomical_phase);
        assert!((northern_climate.phenology_phase - 0.40).abs() < 1.0e-6);
        assert!((southern_climate.phenology_phase - 0.90).abs() < 1.0e-6);
        let northern = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(
                northern_climate.phenology_phase,
                astronomical_phase as f32,
                18.0,
                0.7,
                0.0,
                0.0,
            ),
        );
        let southern = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(
                southern_climate.phenology_phase,
                astronomical_phase as f32,
                18.0,
                0.7,
                0.0,
                0.0,
            ),
        );
        assert!(northern.bloom > 0.90);
        assert!(southern.bloom < 0.05);
        assert!(southern.stubble > 0.80 || southern.dormancy > 0.80);
    }

    #[test]
    fn curated_tulip_calendars_bloom_at_their_northern_and_southern_local_springs() {
        use crate::flower_destinations::{FlowerDestinationId, flower_destination};

        for (id, southern) in [
            (FlowerDestinationId::Bollenstreek, false),
            (FlowerDestinationId::TesselaarTulipFarm, true),
        ] {
            let destination = flower_destination(id);
            let peak = destination.calendar.local_peak_phase(southern);
            let encoded = curated_phenology_offset(destination);
            let in_season = flora_season_state(
                CultivationCalendar::CuratedBloom,
                encoded,
                season_inputs(peak, peak, 16.0, 0.72, 0.0, 0.0),
            );
            let out_of_season = flora_season_state(
                CultivationCalendar::CuratedBloom,
                encoded,
                season_inputs((peak + 0.5).rem_euclid(1.0), peak, 16.0, 0.72, 0.0, 0.0),
            );
            assert!(in_season.bloom > 0.95, "{}", destination.name);
            assert!(out_of_season.bloom < 0.05, "{}", destination.name);
        }
    }

    #[test]
    fn houston_reported_coordinate_is_a_wildflower_boost_not_an_orchard() {
        let destination = crate::flower_destinations::query_flower_destination(
            29.6075_f64.to_radians(),
            -95.9668_f64.to_radians(),
        )
        .expect("reported coordinate is inside the Texas Hill Country boost");
        assert_eq!(
            destination.id,
            crate::flower_destinations::FlowerDestinationId::TexasHillCountry
        );
        assert_eq!(
            destination.mode,
            crate::flower_destinations::DestinationMode::EcologicalBoost
        );
        assert_eq!(
            curated_archetype(destination),
            FloraArchetype::MeadowWildflowers
        );
        assert_ne!(curated_archetype(destination), FloraArchetype::Orchard);
    }

    #[test]
    fn atacama_wet_year_fixture_activates_and_drought_suppresses_bloom() {
        use crate::flower_destinations::{FlowerDestinationId, flower_destination};

        let destination = flower_destination(FlowerDestinationId::AtacamaDesert);
        let southern = destination_center_lat_deg(destination) < 0.0;
        let peak = destination.calendar.local_peak_phase(southern);
        let encoded = curated_phenology_offset(destination);
        let wet_year = flora_season_state(
            CultivationCalendar::CuratedBloom,
            encoded,
            season_inputs(peak, peak, 18.0, 0.86, 0.0, 0.0),
        );
        let dry_year = flora_season_state(
            CultivationCalendar::CuratedBloom,
            encoded,
            season_inputs(peak, peak, 18.0, 0.08, 1.0, 0.0),
        );
        assert!(wet_year.bloom > 0.95, "{wet_year:?}");
        assert!(wet_year.density > 0.90, "{wet_year:?}");
        assert!(dry_year.bloom < 0.16, "{dry_year:?}");
        assert!(dry_year.density <= 0.30, "{dry_year:?}");
    }

    #[test]
    fn special_climate_calendars_are_not_simple_hemisphere_inversions() {
        let mediterranean_winter = flora_season_state(
            CultivationCalendar::MediterraneanWinter,
            128,
            season_inputs(0.02, 0.02, 15.0, 0.65, 0.0, 0.0),
        );
        let temperate_winter = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(0.02, 0.02, 15.0, 0.65, 0.0, 0.0),
        );
        assert!(mediterranean_winter.density > temperate_winter.density + 0.50);

        let monsoon_north = flora_season_state(
            CultivationCalendar::Monsoonal,
            128,
            season_inputs(0.18, 0.32, 27.0, 0.82, 0.0, 0.0),
        );
        let monsoon_south = flora_season_state(
            CultivationCalendar::Monsoonal,
            128,
            season_inputs(0.68, 0.32, 27.0, 0.82, 0.0, 0.0),
        );
        assert_eq!(monsoon_north, monsoon_south);

        let tropical_first = flora_season_state(
            CultivationCalendar::TropicalMultiCycle,
            128,
            season_inputs(0.20, 0.20, 27.0, 0.72, 0.0, 0.0),
        );
        let tropical_second = flora_season_state(
            CultivationCalendar::TropicalMultiCycle,
            128,
            season_inputs(0.70, 0.70, 27.0, 0.72, 0.0, 0.0),
        );
        assert!((tropical_first.bloom - tropical_second.bloom).abs() < 1.0e-5);
        assert!((tropical_first.density - tropical_second.density).abs() < 1.0e-5);
    }

    #[test]
    fn field_offsets_and_live_climate_response_are_bounded() {
        for calendar in [
            CultivationCalendar::TemperateSummer,
            CultivationCalendar::CoolShortSeason,
            CultivationCalendar::MediterraneanWinter,
            CultivationCalendar::Monsoonal,
            CultivationCalendar::TropicalMultiCycle,
            CultivationCalendar::PerennialOrchard,
            CultivationCalendar::PerennialVineyard,
            CultivationCalendar::CuratedBloom,
        ] {
            let lo = bounded_phenology_offset(calendar, 0);
            let hi = bounded_phenology_offset(calendar, u8::MAX);
            assert!(lo <= 0.0 && hi >= 0.0);
            assert!((lo + hi).abs() < 1.0e-6);
            let bound = if calendar == CultivationCalendar::CuratedBloom {
                0.5
            } else {
                0.065
            };
            assert!(lo.abs() <= bound && hi.abs() <= bound);
        }

        let healthy = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(0.42, 0.42, 19.0, 0.75, 0.0, 0.0),
        );
        let drought = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(0.42, 0.42, 19.0, 0.05, 0.9, 0.0),
        );
        let snow = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(0.42, 0.42, -3.0, 0.75, 0.0, 0.9),
        );
        assert!(drought.height < healthy.height && drought.density < healthy.density);
        assert!(drought.bloom < healthy.bloom && drought.senescence > healthy.senescence);
        assert!(snow.height < healthy.height);
        assert!((snow.density - healthy.density).abs() < 1.0e-6);
        assert!(snow.bloom < healthy.bloom && snow.dormancy > healthy.dormancy);
    }

    #[test]
    fn hourly_temperature_does_not_erase_lifecycle_owned_field_structure() {
        let cold = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(0.42, 0.42, -10.0, 0.75, 0.0, 0.0),
        );
        let hot = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(0.42, 0.42, 30.0, 0.75, 0.0, 0.0),
        );
        assert_eq!(cold, hot);
        assert!(cold.density > 0.95);
    }

    #[test]
    fn emergence_is_dense_and_short_while_harvest_keeps_dense_stubble() {
        let emerging = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(0.14, 0.14, 12.0, 0.72, 0.0, 0.0),
        );
        assert!(emerging.density > 0.90, "{emerging:?}");
        assert!(emerging.height < 0.65, "{emerging:?}");

        let harvested = flora_season_state(
            CultivationCalendar::TemperateSummer,
            128,
            season_inputs(0.86, 0.86, 12.0, 0.52, 0.0, 0.0),
        );
        assert!(harvested.stubble > 0.95, "{harvested:?}");
        assert!(harvested.density > 0.90, "{harvested:?}");
        assert!(harvested.height < 0.35, "{harvested:?}");
    }

    #[test]
    fn advancing_time_changes_only_runtime_state_not_the_descriptor() {
        let descriptor = FloraCell {
            archetype: FloraArchetype::FlowerCrop,
            calendar: CultivationCalendar::TemperateSummer,
            field_seed: 0x7365_6173_6f6e_5f38,
            row_heading_rad: 0.75,
            palette: 1,
            base_density: 232,
            coverage: [u32::MAX; FLORA_COVERAGE_WORDS],
            ground_base_cm: 120,
            ground_cm: [0; FLORA_GROUND_SAMPLES],
            phenology_offset: 128,
            curated_style: 0,
        };
        let identity = descriptor.clone();
        let spring = flora_season_state(
            descriptor.calendar,
            descriptor.phenology_offset,
            season_inputs(0.36, 0.36, 18.0, 0.7, 0.0, 0.0),
        );
        let autumn = flora_season_state(
            descriptor.calendar,
            descriptor.phenology_offset,
            season_inputs(0.76, 0.76, 18.0, 0.5, 0.2, 0.0),
        );
        assert_ne!(spring, autumn);
        assert_eq!(descriptor, identity);
    }

    #[test]
    fn flora_cell_stays_inside_the_descriptor_budget() {
        assert!(
            std::mem::size_of::<FloraCell>() <= 192,
            "descriptor grew to {} bytes",
            std::mem::size_of::<FloraCell>()
        );
    }

    #[test]
    fn ground_grid_quantization_and_interpolation_are_centimetre_bounded() {
        let heights = std::array::from_fn(|index| {
            let x = (index % FLORA_GROUND_EDGE) as f32;
            let z = (index / FLORA_GROUND_EDGE) as f32;
            1_842.123 + x * 1.25 + z * 2.5
        });
        let (ground_base_cm, ground_cm) = quantize_ground_grid(heights);
        let descriptor = FloraCell {
            archetype: FloraArchetype::Cereal,
            calendar: CultivationCalendar::TemperateSummer,
            field_seed: 1,
            palette: 0,
            row_heading_rad: 0.0,
            base_density: 1,
            coverage: [u32::MAX; FLORA_COVERAGE_WORDS],
            ground_base_cm,
            ground_cm,
            phenology_offset: 0,
            curated_style: 0,
        };
        for (index, expected) in heights.into_iter().enumerate() {
            let x = index % FLORA_GROUND_EDGE;
            let z = index / FLORA_GROUND_EDGE;
            let actual = descriptor.ground_height_m(
                x as f32 / (FLORA_GROUND_EDGE - 1) as f32,
                z as f32 / (FLORA_GROUND_EDGE - 1) as f32,
            );
            assert!((actual - expected).abs() <= 0.006);
        }
        let center = descriptor.ground_height_m(0.5, 0.5);
        assert!((center - heights[12]).abs() <= 0.006);
    }

    #[test]
    fn extreme_relief_uses_a_deterministic_bounded_fallback() {
        let mut heights = [4_000.0; FLORA_GROUND_SAMPLES];
        heights[0] = 3_000.0;
        heights[FLORA_GROUND_SAMPLES - 1] = 5_000.0;
        let a = quantize_ground_grid(heights);
        let b = quantize_ground_grid(heights);
        assert_eq!(a, b);
        assert_eq!(a.1[0], i16::MIN);
        assert_eq!(a.1[FLORA_GROUND_SAMPLES - 1], i16::MAX);
        let represented_span_m =
            (f32::from(a.1[FLORA_GROUND_SAMPLES - 1]) - f32::from(a.1[0])) * 0.01;
        assert!(represented_span_m <= 655.35);
        assert!(represented_span_m < heights[FLORA_GROUND_SAMPLES - 1] - heights[0]);
    }

    #[test]
    fn cpu_flora_admission_and_primitive_references_are_deterministic() {
        let seed = 0x0123_4567_89ab_cdef;
        let coverage = [u32::MAX; FLORA_COVERAGE_WORDS];
        let input = FloraAdmissionInput {
            seed,
            glyph: 17,
            candidate_limit: 64,
            managed_inside: true,
            coverage: &coverage,
            mask_x: 4,
            mask_z: 9,
            visual_gx: 1_234,
            visual_gz: 5_678,
            visual_layer: 1,
            base_density: 0.82,
            season_density: 0.75,
            clustered_wildflowers: false,
            source_gx: 1_234,
            source_gz: 5_678,
        };
        let admitted = flora_candidate_admitted(input);
        assert_eq!(admitted, flora_candidate_admitted(input));
        assert!(!flora_candidate_admitted(FloraAdmissionInput {
            candidate_limit: 17,
            ..input
        }));
        let uncovered = [0; FLORA_COVERAGE_WORDS];
        assert!(!flora_candidate_admitted(FloraAdmissionInput {
            coverage: &uncovered,
            ..input
        }));
        assert_eq!(
            flora_primitive_exists(
                admitted,
                seed,
                1_234,
                5_678,
                1,
                FloraPrimitiveRule::Seasonal { density: 0.6 },
            ),
            flora_primitive_exists(
                admitted,
                seed,
                1_234,
                5_678,
                1,
                FloraPrimitiveRule::Seasonal { density: 0.6 },
            )
        );
        assert!(!flora_primitive_exists(
            false,
            seed,
            1_234,
            5_678,
            1,
            FloraPrimitiveRule::Always,
        ));
        assert!(!flora_primitive_exists(
            true,
            seed,
            1_234,
            5_678,
            1,
            FloraPrimitiveRule::VineyardPost { period: 0 },
        ));
    }
}
