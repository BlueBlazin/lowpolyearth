//! Regional cultivation priors over the same administrative atlas as buildings.
//!
//! These are visual crop grammars, not observations of a particular farm or
//! measured crop shares. A source field owns one treatment; environmental gates
//! remove implausible choices before its deterministic weighted selection.

use vearth_data::admin_regions::AdminRegionAtlas;

use crate::architecture_catalog::{RegionalFamily, family_for_context};
use crate::flora::{CropEnvironment, FloraArchetype, crop_archetype_is_plausible};

pub const CROP_PALETTE_BASE: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CropKind {
    Wheat,
    Barley,
    Oats,
    Rye,
    Rice,
    Maize,
    Sorghum,
    Millet,
    Soybean,
    Pulses,
    LeafVegetables,
    RootVegetables,
    Cotton,
    Sugarcane,
    Tea,
    Flax,
}

/// Display-sRGB colors are shared by terrain and generated shader constants.
/// Mature leaf crops deliberately remain green; only grain heads turn straw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropTreatment {
    pub kind: CropKind,
    pub label: &'static str,
    pub archetype: FloraArchetype,
    pub green: [u8; 3],
    pub mature: [u8; 3],
    pub soil: [u8; 3],
    pub height_scale: f32,
}

macro_rules! treatments {
    ($($kind:ident, $label:literal, $form:ident, $green:expr, $mature:expr, $soil:expr, $height:expr;)+) => {
        pub const CROP_TREATMENTS: &[CropTreatment] = &[$(CropTreatment {
            kind: CropKind::$kind, label: $label, archetype: FloraArchetype::$form,
            green: $green, mature: $mature, soil: $soil, height_scale: $height,
        }),+];
    }
}

treatments! {
    Wheat, "wheat", Cereal, [106, 135, 71], [179, 155, 88], [105, 91, 65], 1.00;
    Barley, "barley", Cereal, [124, 145, 82], [187, 169, 110], [110, 96, 69], 0.87;
    Oats, "oats", Cereal, [108, 140, 100], [155, 156, 111], [94, 88, 69], 1.05;
    Rye, "rye", Cereal, [101, 130, 91], [162, 149, 105], [96, 88, 69], 1.15;
    Rice, "rice", LowDenseCrop, [102, 150, 70], [163, 169, 91], [78, 88, 66], 1.15;
    Maize, "maize", TallRows, [80, 126, 65], [114, 136, 73], [109, 87, 60], 1.10;
    Sorghum, "sorghum", Cereal, [111, 135, 75], [145, 109, 77], [126, 97, 67], 1.32;
    Millet, "millet", Cereal, [118, 143, 80], [167, 149, 92], [130, 104, 73], 1.08;
    Soybean, "soybean", BroadleafRows, [78, 121, 61], [111, 135, 69], [99, 84, 62], 0.92;
    Pulses, "pulses", BroadleafRows, [104, 132, 79], [124, 141, 85], [115, 97, 72], 0.78;
    LeafVegetables, "leaf vegetables", BroadleafRows, [80, 135, 99], [88, 142, 107], [91, 79, 61], 0.65;
    RootVegetables, "root vegetables", LowDenseCrop, [98, 137, 73], [111, 144, 81], [104, 87, 65], 0.70;
    Cotton, "cotton", BroadleafRows, [92, 129, 78], [162, 167, 137], [122, 96, 69], 1.16;
    Sugarcane, "sugarcane", TallRows, [84, 135, 80], [118, 149, 85], [101, 85, 62], 1.28;
    Tea, "tea hedges", LowDenseCrop, [66, 110, 68], [76, 122, 72], [109, 84, 64], 2.05;
    Flax, "flax", LowDenseCrop, [93, 133, 111], [133, 149, 125], [96, 85, 66], 0.90;
}

impl CropKind {
    pub const fn palette(self) -> u8 {
        CROP_PALETTE_BASE + self as u8
    }
    pub fn treatment(self) -> &'static CropTreatment {
        &CROP_TREATMENTS[self as usize]
    }
}

pub fn treatment_for_palette(palette: u8) -> Option<&'static CropTreatment> {
    CROP_TREATMENTS.get(usize::from(palette.checked_sub(CROP_PALETTE_BASE)?))
}

/// Four weighted entries express an agricultural region's dominant visual
/// vocabulary. Repeated entries are intentional; neighboring fields need not
/// exhaust a catalog to communicate its identity.
pub fn regional_crops(region: RegionalFamily) -> [CropKind; 4] {
    use CropKind::*;
    use RegionalFamily::*;
    match region {
        AtlanticSlate | PortugueseAtlantic => regional_crops(AtlanticEurope),
        NorthEuropeanBrick => regional_crops(Baltic),
        JulianAlpine | Adriatic => regional_crops(Balkan),
        Aegean | Maltese => regional_crops(Mediterranean),
        Pueblo => regional_crops(NorthAmericanDry),
        PeruvianCoast => regional_crops(Andean),
        ChileanLakes => regional_crops(Patagonian),
        Tamil => regional_crops(MonsoonCoast),
        Gujarati => regional_crops(Deccan),
        Bhutanese => regional_crops(Himalayan),
        Hui => regional_crops(ChineseSouth),
        Cape => regional_crops(SouthAfrican),
        Tasmanian => regional_crops(NewZealand),
        AtlanticEurope => [Wheat, Barley, RootVegetables, Flax],
        BritishIsles => [Barley, Wheat, Oats, RootVegetables],
        Nordic | Arctic => [Barley, Oats, Barley, RootVegetables],
        Baltic => [Rye, Barley, RootVegetables, Flax],
        CentralEurope => [Wheat, RootVegetables, Barley, Maize],
        Alpine => [Barley, RootVegetables, Oats, Maize],
        Mediterranean | Iberian => [Wheat, Barley, LeafVegetables, Pulses],
        Italian => [Wheat, Rice, Maize, LeafVegetables],
        Balkan => [Wheat, Maize, RootVegetables, Pulses],
        EastEuropean => [Wheat, Rye, Barley, RootVegetables],
        Caucasus => [Wheat, Barley, Maize, LeafVegetables],
        Anatolian => [Wheat, Barley, Pulses, Cotton],
        NorthAmerican => [Maize, Soybean, Wheat, Soybean],
        NewEngland | Quebec => [Maize, RootVegetables, Oats, LeafVegetables],
        NorthAmericanSouth => [Cotton, Soybean, Maize, Rice],
        PacificNorthwest => [Wheat, Barley, RootVegetables, Pulses],
        NorthAmericanDry => [Wheat, Cotton, LeafVegetables, Sorghum],
        Caribbean | MexicanTropical | MesoamericanTropical => [Sugarcane, Maize, Rice, Pulses],
        MexicanHighland => [Maize, Maize, Pulses, Barley],
        MexicanDry => [Sorghum, Maize, Wheat, Cotton],
        Andean => [RootVegetables, Barley, Maize, Pulses],
        SouthAmericanTropical => [Rice, Maize, Sugarcane, RootVegetables],
        SouthernCone => [Soybean, Wheat, Maize, Barley],
        Patagonian => [Barley, Oats, RootVegetables, Wheat],
        BrazilianAtlantic => [Sugarcane, Soybean, Maize, Pulses],
        BrazilianNortheast => [Sugarcane, Pulses, Maize, Cotton],
        Maghreb | Levantine => [Barley, Wheat, Pulses, LeafVegetables],
        Saharan | GulfArabian => [Barley, Millet, LeafVegetables, Pulses],
        Nile => [Cotton, Wheat, Rice, Sugarcane],
        Sahel | Horn => [Millet, Sorghum, Millet, Pulses],
        WestAfrican => [Sorghum, Rice, RootVegetables, Maize],
        CentralAfrican => [RootVegetables, Maize, Rice, Pulses],
        EastAfrican => [Maize, Pulses, Tea, Sorghum],
        Swahili => [Rice, Maize, Sugarcane, RootVegetables],
        EthiopianHighland | YemeniHighland => [Barley, Millet, Sorghum, Pulses],
        SouthAfrican => [Maize, Wheat, Sorghum, Sugarcane],
        Madagascan => [Rice, Rice, RootVegetables, Maize],
        IndianOcean => [Sugarcane, Rice, Tea, RootVegetables],
        Iranian | CentralAsian => [Wheat, Cotton, Barley, Pulses],
        Caspian => [Rice, Tea, Wheat, LeafVegetables],
        IndianPlain => [Wheat, Rice, Sugarcane, Pulses],
        Deccan => [Millet, Cotton, Sorghum, Pulses],
        MonsoonCoast => [Rice, Rice, Sugarcane, RootVegetables],
        BengalDelta => [Rice, Rice, Pulses, RootVegetables],
        Himalayan => [RootVegetables, Barley, Maize, Rice],
        HimalayanDry => [Barley, Barley, Pulses, RootVegetables],
        NortheastIndian => [Rice, Tea, Rice, Maize],
        ChineseNorth => [Wheat, Maize, Soybean, Millet],
        ChineseSouth => [Rice, Rice, Tea, LeafVegetables],
        Japanese => [Rice, Rice, LeafVegetables, Tea],
        JapaneseSnow => [Rice, RootVegetables, Soybean, Wheat],
        Ryukyuan => [Sugarcane, RootVegetables, Rice, LeafVegetables],
        Korean => [Rice, Rice, Soybean, LeafVegetables],
        SoutheastAsian | Vietnamese => [Rice, Rice, Maize, Sugarcane],
        Malay | Indonesian | Philippine => [Rice, Rice, Sugarcane, RootVegetables],
        Australian => [Wheat, Barley, Pulses, Cotton],
        AustralianTropical => [Sugarcane, Sorghum, Rice, Cotton],
        NewZealand => [Barley, RootVegetables, Wheat, LeafVegetables],
        PacificIsland => [RootVegetables, Sugarcane, Rice, Maize],
    }
}

pub fn crop_is_plausible(kind: CropKind, e: CropEnvironment) -> bool {
    if !crop_archetype_is_plausible(kind.treatment().archetype, e) {
        return false;
    }
    let moist =
        e.near_water || (e.aridity < 0.56 && (e.precipitation_mm_day >= 1.2 || e.humidity >= 0.65));
    match kind {
        CropKind::Rice => {
            e.temp_c >= 12.0 && e.elevation_m < 2_300.0 && e.slope_rad < 0.16 && moist
        }
        CropKind::Tea => {
            (12.0..=29.0).contains(&e.temp_c)
                && e.humidity >= 0.56
                && ((e.aridity < 0.50 && e.precipitation_mm_day >= 1.4)
                    // Coastal/highland atlas rainfall can be missing while
                    // observed humidity still establishes a moist climate.
                    || (e.humidity >= 0.76 && e.aridity < 0.85))
        }
        CropKind::Cotton => e.temp_c >= 17.0 && e.elevation_m < 1_700.0,
        CropKind::Sugarcane => e.temp_c >= 21.0 && e.elevation_m < 1_300.0 && moist,
        CropKind::Soybean => e.temp_c >= 10.0 && e.aridity < 0.65,
        CropKind::Sorghum | CropKind::Millet => e.temp_c >= 12.0,
        _ => true,
    }
}

pub fn crop_recipe(seed: u64, e: CropEnvironment) -> &'static CropTreatment {
    use CropKind::*;
    if !e.temp_c.is_finite()
        || !e.elevation_m.is_finite()
        || !e.slope_rad.is_finite()
        || e.temp_c < -8.0
        || e.temp_c > 40.0
        || e.elevation_m > 4_300.0
        || e.slope_rad > 0.48
        || e.relief_m > 260.0
    {
        return Wheat.treatment();
    }
    if crop_is_plausible(Tea, e)
        && cultivation_prior(seed, e, TEA_LANDSCAPES, 0x5445_415f_4849_4c4c)
    {
        return Tea.treatment();
    }
    let region = AdminRegionAtlas::bundled()
        .context_at(
            f64::from(e.lat_deg).to_radians(),
            f64::from(e.lon_deg).to_radians(),
        )
        .and_then(|context| {
            let family = family_for_context(context)?;
            // Facade/roof refinements do not establish different crop stock.
            Some(match (context.country_name, family) {
                ("Slovenia", _) => RegionalFamily::Balkan,
                ("United Kingdom", RegionalFamily::AtlanticSlate) => RegionalFamily::BritishIsles,
                _ => family,
            })
        });
    let choices = region.map(regional_crops).unwrap_or_else(|| {
        if e.temp_c < 12.0 {
            [Barley, Oats, Rye, RootVegetables]
        } else if e.aridity >= 0.65 {
            [Wheat, Millet, Sorghum, Pulses]
        } else if e.temp_c >= 21.0 {
            [Rice, Maize, Sugarcane, Pulses]
        } else {
            [Wheat, Barley, Pulses, RootVegetables]
        }
    });
    // Each weighted slot has its own stable score. Removing an implausible
    // alternative cannot reshuffle fields whose selected crop is still valid.
    choices
        .into_iter()
        .enumerate()
        .filter(|(_, kind)| crop_is_plausible(*kind, e))
        .max_by_key(|(slot, _)| {
            let mut value = seed ^ (*slot as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
            value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            value ^ (value >> 31)
        })
        .map_or(Wheat, |(_, kind)| kind)
        .treatment()
}

/// Authored perennial-crop likelihood within mapped agricultural landscapes.
/// These priors fill missing crop-use detail; they do not identify observed
/// parcels or authorize planting on other land-cover classes.
#[derive(Clone, Copy)]
struct CultivationLandscape {
    center: [f32; 2],
    radius: [f32; 2],
    field_share: f32,
}

// Ellipses describe broad visual contexts in degrees, with feathered edges.
// Duplicate/overlapping contexts use their strongest prior, never additive
// coverage. Even wine country retains ordinary annual fields.
const fn cultivation_landscape(
    center: [f32; 2],
    radius: [f32; 2],
    field_share: f32,
) -> CultivationLandscape {
    CultivationLandscape {
        center,
        radius,
        field_share,
    }
}

const WINE_LANDSCAPES: &[CultivationLandscape] = &[
    cultivation_landscape([44.85, -0.50], [0.60, 0.85], 0.45), // Bordeaux
    cultivation_landscape([44.90, -0.16], [0.22, 0.34], 0.72), // Saint-Emilion
    cultivation_landscape([45.12, -0.77], [0.40, 0.30], 0.65), // Medoc
    cultivation_landscape([47.03, 4.86], [0.43, 0.28], 0.64),  // Burgundy
    cultivation_landscape([46.25, 4.65], [0.30, 0.30], 0.58),  // Beaujolais
    cultivation_landscape([47.22, 0.60], [0.30, 1.05], 0.44),  // Loire
    cultivation_landscape([47.15, -1.45], [0.24, 0.42], 0.52), // Muscadet
    cultivation_landscape([47.33, 2.84], [0.20, 0.30], 0.58),  // Sancerre
    cultivation_landscape([49.08, 4.00], [0.36, 0.50], 0.52),  // Champagne
    cultivation_landscape([48.12, 7.35], [0.58, 0.26], 0.62),  // Alsace
    cultivation_landscape([44.10, 4.85], [0.55, 0.48], 0.57),  // Southern Rhone
    cultivation_landscape([45.16, 4.82], [0.43, 0.22], 0.48),  // Northern Rhone
    cultivation_landscape([43.25, 3.00], [0.40, 0.95], 0.53),  // Languedoc
    cultivation_landscape([43.47, 6.20], [0.34, 0.58], 0.46),  // Provence
    cultivation_landscape([43.48, 11.23], [0.52, 0.56], 0.58), // Tuscany
    cultivation_landscape([44.74, 8.05], [0.40, 0.55], 0.64),  // Piedmont
    cultivation_landscape([45.52, 10.91], [0.22, 0.50], 0.48), // Veneto
    cultivation_landscape([42.45, -2.60], [0.31, 0.69], 0.62), // Rioja
    cultivation_landscape([41.65, -4.10], [0.25, 0.85], 0.50), // Ribera del Duero
    cultivation_landscape([41.20, -7.55], [0.24, 0.74], 0.64), // Douro
    cultivation_landscape([38.70, -7.75], [0.48, 0.65], 0.38), // Alentejo
    cultivation_landscape([49.95, 7.02], [0.40, 0.70], 0.44),  // Mosel/Rhine
    cultivation_landscape([38.50, -122.40], [0.28, 0.24], 0.62), // Napa
    cultivation_landscape([38.45, -122.85], [0.35, 0.42], 0.53), // Sonoma
    cultivation_landscape([35.62, -120.69], [0.32, 0.40], 0.48), // Paso Robles
    cultivation_landscape([-33.14, -68.89], [0.58, 0.46], 0.63), // Mendoza
    cultivation_landscape([-34.58, -71.15], [0.52, 0.48], 0.52), // Central Chile
    cultivation_landscape([-33.88, 18.94], [0.42, 0.40], 0.62), // Cape Winelands
    cultivation_landscape([-34.56, 138.94], [0.30, 0.40], 0.61), // Barossa
    cultivation_landscape([-35.20, 138.58], [0.25, 0.30], 0.53), // McLaren Vale
    cultivation_landscape([-37.70, 145.48], [0.26, 0.35], 0.44), // Yarra
    cultivation_landscape([-31.78, 115.99], [0.19, 0.25], 0.42), // Swan Valley
    cultivation_landscape([-33.96, 115.04], [0.50, 0.25], 0.49), // Margaret River
    cultivation_landscape([-41.52, 173.86], [0.24, 0.42], 0.66), // Marlborough
    cultivation_landscape([-39.62, 176.85], [0.32, 0.40], 0.51), // Hawke's Bay
    cultivation_landscape([-45.04, 169.18], [0.30, 0.60], 0.42), // Central Otago
];

// Tea locations are supported by estate and national tourism sources. The
// extents and field shares are authored visual priors, not surveyed boundaries.
// BOH: https://bohtea.com/contact-us/
// VNAT: https://vietnamtourism.gov.vn/en/post/15690 (Moc Chau),
// https://vietnamtourism.gov.vn/en/post/15664 (Thai Nguyen),
// https://vietnamtourism.gov.vn/en/post/20671 (Bao Loc).
// Vietnam Tourism identifies the Cau Dat tea hills east of Dalat.
// Indonesia Travel identifies Rancabali in Ciwidey; TAT identifies Mae Salong.
const TEA_LANDSCAPES: &[CultivationLandscape] = &[
    cultivation_landscape([4.48, 101.42], [0.16, 0.15], 0.72), // Cameron Highlands/BOH
    cultivation_landscape([20.84, 104.65], [0.23, 0.25], 0.65), // Moc Chau
    cultivation_landscape([21.60, 105.80], [0.23, 0.25], 0.58), // Thai Nguyen
    cultivation_landscape([11.60, 107.80], [0.18, 0.25], 0.62), // Bao Loc
    cultivation_landscape([11.88, 108.57], [0.18, 0.22], 0.68), // Cau Dat/Dalat
    cultivation_landscape([-7.13, 107.40], [0.17, 0.21], 0.68), // Rancabali
    cultivation_landscape([20.16, 99.63], [0.17, 0.19], 0.63), // Mae Salong
];

pub(crate) fn infer_vineyard(seed: u64, environment: CropEnvironment) -> bool {
    if !(7.0..=25.0).contains(&environment.temp_c)
        || !(25.0..=52.0).contains(&environment.lat_deg.abs())
        || !environment.aridity.is_finite()
        || environment.aridity > 0.94
    {
        return false;
    }
    cultivation_prior(seed, environment, WINE_LANDSCAPES, 0x5649_4e45_5941_5244)
}

fn cultivation_prior(
    seed: u64,
    environment: CropEnvironment,
    landscapes: &[CultivationLandscape],
    salt: u64,
) -> bool {
    let likelihood = landscapes.iter().fold(0.0f32, |strongest, landscape| {
        let north = (environment.lat_deg - landscape.center[0]) / landscape.radius[0];
        if north.abs() >= 1.0 {
            return strongest;
        }
        let east = ((environment.lon_deg - landscape.center[1] + 180.0).rem_euclid(360.0) - 180.0)
            / landscape.radius[1];
        if east.abs() >= 1.0 {
            return strongest;
        }
        let radius = north.hypot(east);
        let edge = ((1.0 - radius) / 0.30).clamp(0.0, 1.0);
        strongest.max(landscape.field_share * edge * edge * (3.0 - 2.0 * edge))
    });
    let mut value = seed ^ salt;
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    let draw = (value >> 40) as f32 / (1u32 << 24) as f32;
    draw < likelihood
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(lat: f32, lon: f32) -> CropEnvironment {
        CropEnvironment {
            lat_deg: lat,
            lon_deg: lon,
            temp_c: 24.0,
            elevation_m: 80.0,
            slope_rad: 0.03,
            relief_m: 12.0,
            aridity: 0.25,
            precipitation_mm_day: 3.5,
            humidity: 0.78,
            near_water: true,
        }
    }

    #[test]
    fn tea_priors_cover_quantized_highland_contexts_but_not_unrelated_lowlands() {
        for landscape in TEA_LANDSCAPES {
            let location = crate::flora::field_context_location(
                landscape.center.map(|v| f64::from(v).to_radians()),
            );
            let e = environment(
                location[0].to_degrees() as f32,
                location[1].to_degrees() as f32,
            );
            let count = (0..1024)
                .filter(|&seed| crop_recipe(seed, e).kind == CropKind::Tea)
                .count();
            assert!(
                (350..900).contains(&count),
                "tea fields {count} at {:?}",
                landscape.center
            );
        }
        for (lat, lon) in [
            (1.3, 103.8),
            (3.15, 101.7),
            (13.7, 100.5),
            (-6.2, 106.8),
            (10.3, 106.0),
        ] {
            assert!(
                (0..256).all(|seed| crop_recipe(seed, environment(lat, lon)).kind != CropKind::Tea)
            );
        }
        let mut e = environment(4.5, 101.375);
        e.precipitation_mm_day = 0.0;
        e.humidity = 0.84;
        e.aridity = 0.76;
        assert!((0..256).any(|seed| crop_recipe(seed, e).kind == CropKind::Tea));
        e.humidity = 0.40;
        e.aridity = 0.90;
        assert!((0..256).all(|seed| crop_recipe(seed, e).kind != CropKind::Tea));
        e = environment(4.5, 101.375);
        for temperature in [5.0, 33.0] {
            e.temp_c = temperature;
            assert!((0..256).all(|seed| crop_recipe(seed, e).kind != CropKind::Tea));
        }
    }

    #[test]
    fn mapped_tea_source_retains_evergreen_identity_across_parent_and_children() {
        use crate::{Col, N, flora, tier0};
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            frame::ChunkFrame,
        };
        use vearth_data::vectors::FloraRegionKind;
        let location = [4.5_f64.to_radians(), 101.375_f64.to_radians()];
        let parent = CellId::from_dir(
            vearth_core::ellipsoid::geodetic_to_ecef(location[0], location[1], 0.0).normalize(),
            18,
        );
        let parent_frame = ChunkFrame::of(ChunkAddr { cell: parent, k: 0 });
        let mut cols = [[Col::EMPTY; N]; N];
        let empty = tier0::CellOverlay::empty();
        let mut stamp = tier0::FloraStamp {
            location_rad: Some(location),
            kind: FloraRegionKind::Farmland,
            seed: 0,
            curated_style: 0,
            surface: flora::select_field_surface(FloraRegionKind::Farmland, 0),
        };
        let environment =
            crate::field_crop_environment(stamp, &parent_frame, &empty, &cols, [u32::MAX; 8]);
        let seed = (0..1024)
            .find(|&seed| crop_recipe(seed, environment).kind == CropKind::Tea)
            .expect("observed Cameron climate admits tea");
        stamp.seed = seed;
        stamp.surface = flora::select_field_surface(FloraRegionKind::Farmland, seed);
        for (index, cell) in std::iter::once(parent).chain(parent.children()).enumerate() {
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            for col in cols.iter_mut().flatten() {
                col.elev = index as f32 * 400.0;
                col.slope = index as f32 * 0.08;
            }
            let mut overlay = tier0::CellOverlay::empty();
            for col in overlay.cols.iter_mut().flatten() {
                col.flora = Some(stamp);
            }
            crate::resolve_crop_catalog(&mut overlay, &frame, &cols);
            let descriptors =
                crate::build_flora_descriptors(cell, &frame, Some(&overlay), &cols, &[], None);
            assert_eq!(descriptors.len(), 1);
            assert_eq!(descriptors[0].palette, CropKind::Tea.palette());
            assert_eq!(descriptors[0].archetype, FloraArchetype::LowDenseCrop);
            assert_eq!(
                descriptors[0].calendar,
                flora::CultivationCalendar::PerennialOrchard
            );
        }
    }

    #[test]
    fn wine_landscapes_select_some_fields_without_spreading_to_unrelated_regions() {
        for landscape in WINE_LANDSCAPES {
            let e = environment(landscape.center[0], landscape.center[1]);
            let count = (0..1024).filter(|&seed| infer_vineyard(seed, e)).count();
            assert!(
                (250..850).contains(&count),
                "wine prior {} at {:?}",
                count,
                landscape.center
            );
        }
        for (lat, lon) in [
            (42.0, -93.0),
            (35.7, 139.0),
            (18.5, 74.5),
            (52.0, 8.0),
            (0.0, 20.0),
        ] {
            for seed in 0..256 {
                assert!(!infer_vineyard(seed, environment(lat, lon)));
            }
        }
        for temperature in [-4.0, 30.0] {
            let mut e = environment(44.875, -0.125);
            e.temp_c = temperature;
            assert!((0..256).all(|seed| !infer_vineyard(seed, e)));
        }
    }

    #[test]
    fn inferred_vineyard_owns_kind_surface_and_calendar_without_changing_explicit_landuse() {
        use crate::{Col, N, flora, tier0};
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            frame::ChunkFrame,
        };
        use vearth_data::vectors::FloraRegionKind;
        let location = [44.875_f64.to_radians(), (-0.125_f64).to_radians()];
        let seed = (0..1024)
            .find(|&seed| infer_vineyard(seed, environment(44.875, -0.125)))
            .unwrap();
        let parent = CellId::from_dir(
            vearth_core::ellipsoid::geodetic_to_ecef(location[0], location[1], 0.0).normalize(),
            18,
        );
        let cols = [[Col::EMPTY; N]; N];
        for cell in std::iter::once(parent).chain(parent.children()) {
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            for kind in [
                FloraRegionKind::Farmland,
                FloraRegionKind::Garden,
                FloraRegionKind::Meadow,
                FloraRegionKind::Orchard,
                FloraRegionKind::Vineyard,
            ] {
                let stamp = tier0::FloraStamp {
                    location_rad: Some(location),
                    kind,
                    seed,
                    curated_style: 0,
                    surface: flora::select_field_surface(kind, seed),
                };
                let mut overlay = tier0::CellOverlay::empty();
                for col in overlay.cols.iter_mut().flatten() {
                    col.flora = Some(stamp);
                }
                crate::resolve_crop_catalog(&mut overlay, &frame, &cols);
                let resolved = overlay.cols[0][0].flora.unwrap();
                if kind == FloraRegionKind::Farmland {
                    assert_eq!(resolved.kind, FloraRegionKind::Vineyard);
                    assert_eq!(
                        resolved.surface,
                        flora::select_field_surface(FloraRegionKind::Vineyard, seed)
                    );
                    assert_eq!(resolved.seed, seed);
                    let descriptors = crate::build_flora_descriptors(
                        cell,
                        &frame,
                        Some(&overlay),
                        &cols,
                        &[],
                        None,
                    );
                    assert_eq!(descriptors.len(), 1);
                    assert_eq!(descriptors[0].archetype, FloraArchetype::Vineyard);
                    assert_eq!(
                        descriptors[0].calendar,
                        flora::CultivationCalendar::PerennialVineyard
                    );
                    let ground = crate::field_surface_appearance(resolved, &cols[0][0], cell, 0, 0);
                    assert_eq!(ground.material, crate::material::FIELD_VINEYARD);
                } else {
                    assert_eq!(resolved, stamp);
                }
                let mut uncultivated = tier0::CellOverlay::empty();
                crate::resolve_crop_catalog(&mut uncultivated, &frame, &cols);
                assert!(
                    uncultivated
                        .cols
                        .iter()
                        .flatten()
                        .all(|col| col.flora.is_none())
                );
            }
        }
    }

    #[test]
    fn every_administrative_family_has_supported_crop_treatments() {
        for &region in crate::architecture_catalog::REGIONAL_FAMILIES {
            for crop in regional_crops(region) {
                assert_eq!(treatment_for_palette(crop.palette()).unwrap().kind, crop);
            }
        }
        assert_eq!(CROP_TREATMENTS.len(), 16);
        assert!(
            CROP_TREATMENTS
                .iter()
                .filter(|t| t.mature[1] > t.mature[0])
                .count()
                >= 8
        );
    }

    #[test]
    fn rice_dominates_warm_japanese_fields_and_corn_soy_dominate_iowa() {
        let japan = environment(35.7, 139.0);
        let iowa = environment(42.0, -93.5);
        let rice = (0..1024)
            .filter(|&seed| crop_recipe(seed, japan).kind == CropKind::Rice)
            .count();
        let corn_soy = (0..1024)
            .filter(|&seed| {
                matches!(
                    crop_recipe(seed, iowa).kind,
                    CropKind::Maize | CropKind::Soybean
                )
            })
            .count();
        assert!(rice > 430, "rice fields: {rice}");
        assert!(corn_soy > 650, "corn/soy fields: {corn_soy}");
    }

    #[test]
    fn excluding_rice_does_not_reselect_other_valid_crops() {
        let lowland = environment(35.7, 139.0);
        let mut hillside = lowland;
        hillside.slope_rad = 0.17;
        let mut retained = 0;
        for seed in 0..1024 {
            let selected = crop_recipe(seed, lowland).kind;
            if crop_is_plausible(selected, hillside) {
                assert_eq!(crop_recipe(seed, hillside).kind, selected);
                retained += 1;
            }
        }
        assert!(retained > 250);
    }

    #[test]
    fn source_field_palette_and_calendar_survive_terrain_lod_and_local_context_changes() {
        use crate::{Col, N, flora, tier0};
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            frame::ChunkFrame,
        };
        use vearth_data::vectors::FloraRegionKind;
        let location = [35.7_f64.to_radians(), 139.0_f64.to_radians()];
        let parent = CellId::from_dir(
            vearth_core::ellipsoid::geodetic_to_ecef(location[0], location[1], 0.0).normalize(),
            18,
        );
        let stamp = tier0::FloraStamp {
            location_rad: Some(location),
            kind: FloraRegionKind::Farmland,
            seed: 37,
            curated_style: 0,
            surface: flora::select_field_surface(FloraRegionKind::Farmland, 37),
        };
        let mut expected = None;
        for (index, cell) in std::iter::once(parent).chain(parent.children()).enumerate() {
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let mut cols = [[Col::EMPTY; N]; N];
            for col in cols.iter_mut().flatten() {
                col.elev = index as f32 * 300.0;
                col.top = col.elev;
                col.slope = index as f32 * 0.08;
                col.climate = crate::Climate {
                    temp_c: 24.0 - index as f32 * 4.0,
                    arid: 0.15 + index as f32 * 0.15,
                    precipitation_mm_day: 4.0 - index as f32 * 0.6,
                    humidity: 0.8,
                };
            }
            let mut overlay = tier0::CellOverlay::empty();
            for (z, row) in overlay.cols.iter_mut().enumerate() {
                for (x, col) in row.iter_mut().enumerate() {
                    if x >= index * 2 && z >= index * 2 {
                        col.flora = Some(stamp);
                    }
                    col.water_ecology = index % 2 == 0;
                }
            }
            crate::resolve_crop_catalog(&mut overlay, &frame, &cols);
            let descriptors =
                crate::build_flora_descriptors(cell, &frame, Some(&overlay), &cols, &[], None);
            assert_eq!(descriptors.len(), 1);
            let identity = (
                descriptors[0].palette,
                descriptors[0].archetype,
                descriptors[0].calendar,
            );
            if let Some(expected) = expected {
                assert_eq!(identity, expected);
            } else {
                expected = Some(identity);
            }
        }
    }

    #[test]
    fn dry_deccan_and_cool_nordic_fields_exclude_tropical_water_crops() {
        for mut e in [environment(17.0, 77.0), environment(60.0, 15.0)] {
            if e.lat_deg < 20.0 {
                e.aridity = 0.78;
                e.precipitation_mm_day = 0.4;
                e.near_water = false;
            } else {
                e.temp_c = 6.0;
            }
            for seed in 0..256 {
                let treatment = crop_recipe(seed, e);
                assert!(crop_is_plausible(treatment.kind, e));
                assert!(!matches!(
                    treatment.kind,
                    CropKind::Rice | CropKind::Sugarcane | CropKind::Tea
                ));
            }
        }
    }
    #[test]
    fn resolved_field_shares_one_recipe_between_ground_and_plants() {
        use crate::{Col, N, flora, tier0};
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            cubesphere::Face,
            frame::ChunkFrame,
        };
        use vearth_data::vectors::FloraRegionKind;
        let cell = CellId {
            face: Face(0),
            level: 19,
            i: 1 << 18,
            j: 1 << 18,
        };
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let mut cols = [[Col::EMPTY; N]; N];
        for col in cols.iter_mut().flatten() {
            col.top = 50.0;
            col.elev = 50.0;
            col.climate = crate::Climate {
                temp_c: 24.0,
                arid: 0.25,
                precipitation_mm_day: 3.5,
                humidity: 0.78,
            };
        }
        let seed = (0..100)
            .find(|&seed| {
                flora::select_field_surface(FloraRegionKind::Farmland, seed).kind
                    == flora::FieldSurfaceKind::Crop
            })
            .unwrap();
        let stamp = tier0::FloraStamp {
            location_rad: None,
            kind: FloraRegionKind::Farmland,
            seed,
            curated_style: 0,
            surface: flora::select_field_surface(FloraRegionKind::Farmland, seed),
        };
        let mut overlay = tier0::CellOverlay::empty();
        for col in overlay.cols.iter_mut().flatten() {
            col.flora = Some(stamp);
        }
        crate::resolve_crop_catalog(&mut overlay, &frame, &cols);
        let resolved = overlay.cols[0][0].flora.unwrap();
        assert!(
            overlay
                .cols
                .iter()
                .flatten()
                .all(|col| col.flora == Some(resolved))
        );
        let treatment = treatment_for_palette(resolved.surface.palette).unwrap();
        let descriptors =
            crate::build_flora_descriptors(cell, &frame, Some(&overlay), &cols, &[], None);
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].palette, resolved.surface.palette);
        assert_eq!(descriptors[0].archetype, treatment.archetype);
        let ground = crate::field_surface_appearance(resolved, &cols[0][0], cell, 0, 0);
        assert_eq!(ground.material, crate::material::FIELD_CROP);
        assert!(ground.tint.iter().all(|&channel| channel < 155));
        for col in overlay.cols.iter_mut().flatten() {
            col.flora.as_mut().unwrap().surface.palette = CropKind::Tea.palette();
        }
        let tea = crate::build_flora_descriptors(cell, &frame, Some(&overlay), &cols, &[], None);
        assert_eq!(
            tea[0].calendar,
            flora::CultivationCalendar::PerennialOrchard
        );
    }
}
