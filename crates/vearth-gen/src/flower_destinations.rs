//! Bundled flower-destination registry.
//!
//! Coordinates and treatments were surveyed on 2026-07-30 from the official
//! property, park, land-management or tourism sources named by each entry.
//! This is visual world-building metadata, not a tourism directory.

use std::f64::consts::PI;
use vearth_core::ellipsoid::MEAN_RADIUS;

pub const FLOWER_DESTINATION_REGISTRY_VERSION: u16 = 1;
pub const FLOWER_DESTINATION_COUNT: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum FlowerDestinationId {
    FarmTomita = 1,
    ShikisaiNoOka,
    HitachiSeasidePark,
    AshikagaFlowerPark,
    Keukenhof,
    Bollenstreek,
    ValensolePlateau,
    SaultPlateau,
    CarlsbadFlowerFields,
    SkagitValley,
    WoodenShoeTulipFarm,
    SrinagarTulipGarden,
    DubaiMiracleGarden,
    ButchartGardens,
    TesselaarTulipFarm,
    LuopingCanolaFields,
    WuyuanRapeseedFields,
    JejuCanolaFields,
    CastelluccioPlain,
    Namaqualand,
    Postberg,
    ValleyOfFlowers,
    KaasPlateau,
    AntelopeValley,
    CarrizoPlain,
    AnzaBorrego,
    TexasHillCountry,
    LakeTekapo,
    MountRainierParadise,
    AtacamaDesert,
}

impl FlowerDestinationId {
    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::FarmTomita,
            2 => Self::ShikisaiNoOka,
            3 => Self::HitachiSeasidePark,
            4 => Self::AshikagaFlowerPark,
            5 => Self::Keukenhof,
            6 => Self::Bollenstreek,
            7 => Self::ValensolePlateau,
            8 => Self::SaultPlateau,
            9 => Self::CarlsbadFlowerFields,
            10 => Self::SkagitValley,
            11 => Self::WoodenShoeTulipFarm,
            12 => Self::SrinagarTulipGarden,
            13 => Self::DubaiMiracleGarden,
            14 => Self::ButchartGardens,
            15 => Self::TesselaarTulipFarm,
            16 => Self::LuopingCanolaFields,
            17 => Self::WuyuanRapeseedFields,
            18 => Self::JejuCanolaFields,
            19 => Self::CastelluccioPlain,
            20 => Self::Namaqualand,
            21 => Self::Postberg,
            22 => Self::ValleyOfFlowers,
            23 => Self::KaasPlateau,
            24 => Self::AntelopeValley,
            25 => Self::CarrizoPlain,
            26 => Self::AnzaBorrego,
            27 => Self::TexasHillCountry,
            28 => Self::LakeTekapo,
            29 => Self::MountRainierParadise,
            30 => Self::AtacamaDesert,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DestinationShape {
    Circle {
        lat_deg: f64,
        lon_deg: f64,
        radius_m: f32,
    },
    OrientedRectangle {
        lat_deg: f64,
        lon_deg: f64,
        half_east_m: f32,
        half_north_m: f32,
        heading_deg: f32,
    },
    Polygon {
        vertices_deg: &'static [[f64; 2]],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestinationMode {
    ExactSite,
    MappedFieldOverride,
    EcologicalBoost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowerPattern {
    ParallelStrips,
    AlternatingStrips,
    FormalBeds,
    BroadColorBlocks,
    Terraces,
    DenseNaturalMosaic,
    SparseSuperbloomClusters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceMask(u8);

impl SurfaceMask {
    pub const FARMLAND: Self = Self(1 << 0);
    pub const GARDEN: Self = Self(1 << 1);
    pub const MEADOW: Self = Self(1 << 2);
    pub const NATURAL_GRASS: Self = Self(1 << 3);
    pub const GARDEN_GRASS: Self = Self(Self::GARDEN.0 | Self::NATURAL_GRASS.0);
    pub const FARMLAND_MEADOW: Self = Self(Self::FARMLAND.0 | Self::MEADOW.0);
    pub const MEADOW_GRASS: Self = Self(Self::MEADOW.0 | Self::NATURAL_GRASS.0);
    pub const GARDEN_MEADOW_GRASS: Self =
        Self(Self::GARDEN.0 | Self::MEADOW.0 | Self::NATURAL_GRASS.0);
    pub const FLOWERING_LAND: Self =
        Self(Self::FARMLAND.0 | Self::GARDEN.0 | Self::MEADOW.0 | Self::NATURAL_GRASS.0);

    pub const fn contains(self, surface: Self) -> bool {
        self.0 & surface.0 != 0
    }
}

impl std::ops::BitOr for SurfaceMask {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb8(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CuratedBloomCalendar {
    pub start_month: u8,
    pub end_month: u8,
    pub moisture_gated: bool,
    pub snowmelt_gated: bool,
}

impl CuratedBloomCalendar {
    pub fn local_peak_phase(self, southern_hemisphere: bool) -> f32 {
        let start = (f32::from(self.start_month.clamp(1, 12)) - 0.5) / 12.0;
        let end = (f32::from(self.end_month.clamp(1, 12)) - 0.5) / 12.0;
        let astronomical = (start + (end - start).rem_euclid(1.0) * 0.5).rem_euclid(1.0);
        (astronomical + if southern_hemisphere { 0.5 } else { 0.0 }).rem_euclid(1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowerDestination {
    pub id: FlowerDestinationId,
    pub name: &'static str,
    pub country_region: &'static str,
    pub influence: DestinationShape,
    pub mode: DestinationMode,
    pub allowed_surfaces: SurfaceMask,
    pub pattern: FlowerPattern,
    pub heading_deg: f32,
    pub strip_width_m: f32,
    pub density: u8,
    pub palette: [Rgb8; 3],
    pub calendar: CuratedBloomCalendar,
    pub priority: u8,
    pub source_note: &'static str,
}

const fn rgb(r: u8, g: u8, b: u8) -> Rgb8 {
    Rgb8(r, g, b)
}
const fn bloom(start_month: u8, end_month: u8) -> CuratedBloomCalendar {
    CuratedBloomCalendar {
        start_month,
        end_month,
        moisture_gated: false,
        snowmelt_gated: false,
    }
}
const fn wet(start_month: u8, end_month: u8) -> CuratedBloomCalendar {
    CuratedBloomCalendar {
        start_month,
        end_month,
        moisture_gated: true,
        snowmelt_gated: false,
    }
}
const fn snowmelt(start_month: u8, end_month: u8) -> CuratedBloomCalendar {
    CuratedBloomCalendar {
        start_month,
        end_month,
        moisture_gated: true,
        snowmelt_gated: true,
    }
}
const VALLEY_OF_FLOWERS: &[[f64; 2]] = &[
    [30.665, 79.555],
    [30.685, 79.520],
    [30.760, 79.575],
    [30.780, 79.650],
    [30.720, 79.680],
];

macro_rules! circle {
    ($lat:expr, $lon:expr, $radius:expr) => {
        DestinationShape::Circle {
            lat_deg: $lat,
            lon_deg: $lon,
            radius_m: $radius,
        }
    };
}
macro_rules! rect {
    ($lat:expr, $lon:expr, $east:expr, $north:expr, $heading:expr) => {
        DestinationShape::OrientedRectangle {
            lat_deg: $lat,
            lon_deg: $lon,
            half_east_m: $east,
            half_north_m: $north,
            heading_deg: $heading,
        }
    };
}
macro_rules! destination {
    ($id:ident, $name:expr, $country:expr, $shape:expr, $mode:ident, $surfaces:expr,
     $pattern:ident, $heading:expr, $strip:expr, $density:expr, $palette:expr,
     $calendar:expr, $priority:expr, $source:expr) => {
        FlowerDestination {
            id: FlowerDestinationId::$id,
            name: $name,
            country_region: $country,
            influence: $shape,
            mode: DestinationMode::$mode,
            allowed_surfaces: $surfaces,
            pattern: FlowerPattern::$pattern,
            heading_deg: $heading,
            strip_width_m: $strip,
            density: $density,
            palette: $palette,
            calendar: $calendar,
            priority: $priority,
            source_note: $source,
        }
    };
}

/// Registry v1 acceptance list. Official source notes include access/maps and
/// the characteristic bloom or treatment surveyed for each entry.
pub static FLOWER_DESTINATIONS: [FlowerDestination; FLOWER_DESTINATION_COUNT] = [
    destination!(
        FarmTomita,
        "Farm Tomita, Furano",
        "Japan",
        rect!(43.4172, 142.4254, 650.0, 520.0, 18.0),
        ExactSite,
        SurfaceMask::FLOWERING_LAND,
        AlternatingStrips,
        18.0,
        18.0,
        250,
        [rgb(100, 45, 155), rgb(245, 205, 45), rgb(225, 70, 115)],
        bloom(7, 7),
        250,
        "Farm Tomita official Farm Guide, flower calendar and access map"
    ),
    destination!(
        ShikisaiNoOka,
        "Shikisai no Oka, Biei",
        "Japan",
        rect!(43.5896, 142.4632, 720.0, 680.0, 32.0),
        ExactSite,
        SurfaceMask::FLOWERING_LAND,
        BroadColorBlocks,
        32.0,
        24.0,
        246,
        [rgb(238, 52, 88), rgb(248, 210, 45), rgb(116, 65, 180)],
        bloom(6, 10),
        245,
        "Shikisai no Oka official flower calendar and access information"
    ),
    destination!(
        HitachiSeasidePark,
        "Hitachi Seaside Park",
        "Japan",
        circle!(36.4019, 140.5913, 1900.0),
        ExactSite,
        SurfaceMask::GARDEN_MEADOW_GRASS,
        BroadColorBlocks,
        8.0,
        42.0,
        244,
        [rgb(80, 145, 225), rgb(225, 50, 45), rgb(245, 230, 205)],
        bloom(4, 5),
        240,
        "Hitachi Seaside Park official English flower calendar and park map"
    ),
    destination!(
        AshikagaFlowerPark,
        "Ashikaga Flower Park",
        "Japan",
        circle!(36.3153, 139.5221, 620.0),
        ExactSite,
        SurfaceMask::GARDEN_GRASS,
        FormalBeds,
        0.0,
        20.0,
        250,
        [rgb(112, 62, 170), rgb(205, 155, 225), rgb(245, 220, 242)],
        bloom(4, 5),
        245,
        "Ashikaga Flower Park official Great Wisteria Festival and access pages"
    ),
    destination!(
        Keukenhof,
        "Keukenhof",
        "Netherlands",
        rect!(52.2711, 4.5464, 850.0, 850.0, 4.0),
        ExactSite,
        SurfaceMask::GARDEN_GRASS,
        FormalBeds,
        4.0,
        16.0,
        252,
        [rgb(230, 35, 80), rgb(248, 190, 30), rgb(132, 55, 180)],
        bloom(3, 5),
        255,
        "Keukenhof official park map, history and opening-season pages"
    ),
    destination!(
        Bollenstreek,
        "Bollenstreek/Lisse bulb fields",
        "Netherlands",
        rect!(52.2700, 4.5200, 8500.0, 11500.0, 2.0),
        MappedFieldOverride,
        SurfaceMask::FARMLAND,
        AlternatingStrips,
        2.0,
        22.0,
        248,
        [rgb(225, 35, 70), rgb(250, 205, 35), rgb(120, 45, 175)],
        bloom(3, 5),
        160,
        "Bollenstreek tourism board official flower-map and bloom-season pages"
    ),
    destination!(
        ValensolePlateau,
        "Valensole Plateau lavender fields",
        "France",
        circle!(43.8373, 5.9835, 16000.0),
        MappedFieldOverride,
        SurfaceMask::FARMLAND,
        ParallelStrips,
        8.0,
        13.0,
        238,
        [rgb(92, 48, 145), rgb(126, 75, 175), rgb(185, 135, 205)],
        bloom(6, 7),
        145,
        "Provence-Alpes-Cote d'Azur official Valensole lavender route and season guide"
    ),
    destination!(
        SaultPlateau,
        "Sault Plateau lavender fields",
        "France",
        circle!(44.0917, 5.4086, 13500.0),
        MappedFieldOverride,
        SurfaceMask::FARMLAND,
        Terraces,
        24.0,
        15.0,
        228,
        [rgb(86, 45, 135), rgb(122, 74, 165), rgb(178, 132, 195)],
        bloom(7, 8),
        145,
        "Ventoux Provence tourism official Sault lavender route and flowering calendar"
    ),
    destination!(
        CarlsbadFlowerFields,
        "The Flower Fields at Carlsbad Ranch",
        "United States",
        rect!(33.1408, -117.3187, 620.0, 580.0, 9.0),
        ExactSite,
        SurfaceMask::FLOWERING_LAND,
        AlternatingStrips,
        9.0,
        16.0,
        250,
        [rgb(220, 40, 70), rgb(242, 142, 35), rgb(245, 215, 55)],
        bloom(3, 5),
        245,
        "The Flower Fields official bloom-status, history and directions pages"
    ),
    destination!(
        SkagitValley,
        "Skagit Valley tulip fields",
        "United States",
        rect!(48.4215, -122.3895, 12000.0, 8500.0, 2.0),
        MappedFieldOverride,
        SurfaceMask::FARMLAND,
        AlternatingStrips,
        2.0,
        24.0,
        248,
        [rgb(224, 35, 65), rgb(245, 190, 25), rgb(135, 42, 175)],
        bloom(4, 4),
        150,
        "Skagit Valley Tulip Festival official field map and bloom information"
    ),
    destination!(
        WoodenShoeTulipFarm,
        "Wooden Shoe Tulip Farm",
        "United States",
        rect!(45.2150, -122.7440, 720.0, 760.0, 12.0),
        ExactSite,
        SurfaceMask::FLOWERING_LAND,
        AlternatingStrips,
        12.0,
        18.0,
        248,
        [rgb(225, 38, 70), rgb(250, 205, 30), rgb(116, 50, 180)],
        bloom(3, 5),
        240,
        "Wooden Shoe Tulip Farm official field report, festival and directions pages"
    ),
    destination!(
        SrinagarTulipGarden,
        "Indira Gandhi Memorial Tulip Garden, Srinagar",
        "India",
        rect!(34.0968, 74.8804, 520.0, 430.0, 16.0),
        ExactSite,
        SurfaceMask::GARDEN_GRASS,
        Terraces,
        16.0,
        14.0,
        250,
        [rgb(225, 35, 72), rgb(245, 186, 32), rgb(133, 48, 178)],
        bloom(3, 4),
        245,
        "Jammu and Kashmir tourism official Tulip Garden festival and location pages"
    ),
    destination!(
        DubaiMiracleGarden,
        "Dubai Miracle Garden",
        "United Arab Emirates",
        circle!(25.0600, 55.2444, 620.0),
        ExactSite,
        SurfaceMask::GARDEN_GRASS,
        FormalBeds,
        0.0,
        12.0,
        255,
        [rgb(235, 45, 95), rgb(250, 180, 35), rgb(142, 55, 185)],
        bloom(11, 3),
        250,
        "Dubai Miracle Garden official attractions, season and location pages"
    ),
    destination!(
        ButchartGardens,
        "Butchart Gardens",
        "Canada",
        circle!(48.5653, -123.4697, 720.0),
        ExactSite,
        SurfaceMask::GARDEN_GRASS,
        FormalBeds,
        0.0,
        14.0,
        246,
        [rgb(225, 55, 105), rgb(242, 185, 55), rgb(110, 72, 175)],
        bloom(3, 10),
        245,
        "Butchart Gardens official garden map and seasonal flower guide"
    ),
    destination!(
        TesselaarTulipFarm,
        "Tesselaar Tulip Farm",
        "Australia",
        rect!(-37.8518, 145.3697, 620.0, 620.0, 7.0),
        ExactSite,
        SurfaceMask::FLOWERING_LAND,
        AlternatingStrips,
        7.0,
        18.0,
        248,
        [rgb(226, 38, 72), rgb(245, 197, 35), rgb(118, 52, 180)],
        bloom(9, 10),
        245,
        "Tesselaar Tulip Festival official farm location and flowering-season pages"
    ),
    destination!(
        LuopingCanolaFields,
        "Luoping canola fields",
        "China",
        circle!(24.9300, 104.3100, 22000.0),
        MappedFieldOverride,
        SurfaceMask::FARMLAND,
        BroadColorBlocks,
        22.0,
        54.0,
        238,
        [rgb(246, 205, 24), rgb(235, 178, 18), rgb(255, 225, 62)],
        bloom(2, 3),
        140,
        "Yunnan provincial tourism official Luoping rapeseed flowering guide"
    ),
    destination!(
        WuyuanRapeseedFields,
        "Wuyuan rapeseed fields",
        "China",
        circle!(29.2500, 117.7500, 22000.0),
        MappedFieldOverride,
        SurfaceMask::FARMLAND,
        Terraces,
        31.0,
        38.0,
        232,
        [rgb(246, 207, 25), rgb(232, 176, 16), rgb(255, 228, 70)],
        bloom(3, 4),
        140,
        "Jiangxi/Wuyuan official tourism flowering-season and scenic-area guides"
    ),
    destination!(
        JejuCanolaFields,
        "Jeju canola fields",
        "South Korea",
        circle!(33.4050, 126.5200, 24000.0),
        MappedFieldOverride,
        SurfaceMask::FARMLAND_MEADOW,
        BroadColorBlocks,
        12.0,
        44.0,
        235,
        [rgb(247, 207, 26), rgb(235, 181, 20), rgb(255, 229, 75)],
        bloom(3, 4),
        135,
        "Jeju Tourism Organization official canola flower and festival guide"
    ),
    destination!(
        CastelluccioPlain,
        "Castelluccio di Norcia flowering plain",
        "Italy",
        circle!(42.8294, 13.2074, 6800.0),
        MappedFieldOverride,
        SurfaceMask::FARMLAND_MEADOW,
        DenseNaturalMosaic,
        0.0,
        48.0,
        206,
        [rgb(200, 40, 70), rgb(235, 195, 42), rgb(95, 90, 185)],
        bloom(6, 7),
        150,
        "Umbria tourism official Castelluccio flowering information and park map"
    ),
    destination!(
        Namaqualand,
        "Namaqualand",
        "South Africa",
        circle!(-30.0500, 17.8000, 225000.0),
        EcologicalBoost,
        SurfaceMask::MEADOW_GRASS,
        SparseSuperbloomClusters,
        0.0,
        64.0,
        96,
        [rgb(238, 112, 24), rgb(244, 190, 35), rgb(230, 70, 72)],
        wet(8, 9),
        80,
        "SANParks and Northern Cape tourism official Namaqua bloom and rainfall guidance"
    ),
    destination!(
        Postberg,
        "Postberg, West Coast National Park",
        "South Africa",
        circle!(-33.0660, 17.9650, 13500.0),
        MappedFieldOverride,
        SurfaceMask::MEADOW_GRASS,
        DenseNaturalMosaic,
        0.0,
        44.0,
        186,
        [rgb(238, 104, 25), rgb(245, 192, 40), rgb(224, 70, 82)],
        wet(8, 9),
        155,
        "SANParks official West Coast National Park flower-season information and map"
    ),
    destination!(
        ValleyOfFlowers,
        "Valley of Flowers National Park",
        "India",
        DestinationShape::Polygon {
            vertices_deg: VALLEY_OF_FLOWERS
        },
        EcologicalBoost,
        SurfaceMask::MEADOW_GRASS,
        DenseNaturalMosaic,
        0.0,
        40.0,
        118,
        [rgb(92, 92, 200), rgb(224, 80, 135), rgb(244, 190, 48)],
        snowmelt(7, 8),
        105,
        "UNESCO and Uttarakhand tourism official Valley of Flowers location and season notes"
    ),
    destination!(
        KaasPlateau,
        "Kaas Plateau",
        "India",
        circle!(17.7200, 73.8230, 5200.0),
        MappedFieldOverride,
        SurfaceMask::MEADOW_GRASS,
        DenseNaturalMosaic,
        0.0,
        36.0,
        174,
        [rgb(118, 76, 190), rgb(226, 80, 142), rgb(245, 196, 55)],
        wet(8, 9),
        160,
        "UNESCO Western Ghats and Maharashtra tourism official Kaas season guidance"
    ),
    destination!(
        AntelopeValley,
        "Antelope Valley California Poppy Reserve",
        "United States",
        circle!(34.7240, -118.3968, 9500.0),
        EcologicalBoost,
        SurfaceMask::MEADOW_GRASS,
        SparseSuperbloomClusters,
        0.0,
        58.0,
        108,
        [rgb(242, 102, 14), rgb(247, 157, 24), rgb(238, 198, 55)],
        wet(2, 5),
        100,
        "California State Parks official reserve map and bloom-update guidance"
    ),
    destination!(
        CarrizoPlain,
        "Carrizo Plain National Monument",
        "United States",
        rect!(35.1915, -119.7920, 22000.0, 55000.0, -24.0),
        EcologicalBoost,
        SurfaceMask::MEADOW_GRASS,
        SparseSuperbloomClusters,
        -24.0,
        72.0,
        104,
        [rgb(238, 112, 18), rgb(246, 197, 35), rgb(112, 76, 188)],
        wet(3, 5),
        90,
        "US Bureau of Land Management official Carrizo Plain map and wildflower guidance"
    ),
    destination!(
        AnzaBorrego,
        "Anza-Borrego Desert State Park",
        "United States",
        circle!(33.2550, -116.4000, 82000.0),
        EcologicalBoost,
        SurfaceMask::MEADOW_GRASS,
        SparseSuperbloomClusters,
        0.0,
        78.0,
        76,
        [rgb(238, 95, 22), rgb(245, 190, 42), rgb(182, 72, 168)],
        wet(2, 4),
        70,
        "California State Parks official Anza-Borrego map and variable bloom guidance"
    ),
    destination!(
        TexasHillCountry,
        "Texas Hill Country bluebonnet region",
        "United States",
        circle!(30.5000, -98.0000, 235000.0),
        EcologicalBoost,
        SurfaceMask::MEADOW_GRASS,
        SparseSuperbloomClusters,
        0.0,
        64.0,
        88,
        [rgb(48, 76, 176), rgb(88, 112, 205), rgb(238, 230, 220)],
        wet(3, 5),
        65,
        "Texas Parks and Wildlife and TxDOT official wildflower/bluebonnet guidance"
    ),
    destination!(
        LakeTekapo,
        "Lake Tekapo/Mackenzie Basin lupin displays",
        "New Zealand",
        circle!(-44.0000, 170.4800, 78000.0),
        EcologicalBoost,
        SurfaceMask::MEADOW_GRASS,
        SparseSuperbloomClusters,
        0.0,
        52.0,
        92,
        [rgb(102, 64, 180), rgb(210, 105, 175), rgb(235, 190, 220)],
        bloom(11, 2),
        75,
        "Mackenzie District visitor guidance and Department of Conservation context"
    ),
    destination!(
        MountRainierParadise,
        "Paradise meadows, Mount Rainier",
        "United States",
        circle!(46.7853, -121.7350, 7500.0),
        EcologicalBoost,
        SurfaceMask::MEADOW_GRASS,
        DenseNaturalMosaic,
        0.0,
        38.0,
        108,
        [rgb(210, 75, 130), rgb(235, 195, 54), rgb(92, 98, 195)],
        snowmelt(7, 8),
        110,
        "US National Park Service official Paradise map and wildflower-season guidance"
    ),
    destination!(
        AtacamaDesert,
        "Atacama flowering desert",
        "Chile",
        circle!(-27.2000, -70.0000, 225000.0),
        EcologicalBoost,
        SurfaceMask::MEADOW_GRASS,
        SparseSuperbloomClusters,
        0.0,
        82.0,
        64,
        [rgb(205, 62, 135), rgb(238, 105, 174), rgb(242, 190, 55)],
        wet(8, 11),
        60,
        "Chile CONAF and SERNATUR official desierto florido extent and rainfall guidance"
    ),
];

pub fn flower_destination(id: FlowerDestinationId) -> &'static FlowerDestination {
    &FLOWER_DESTINATIONS[id as usize - 1]
}

pub fn query_flower_destination(lat_rad: f64, lon_rad: f64) -> Option<&'static FlowerDestination> {
    FLOWER_DESTINATIONS
        .iter()
        .filter(|destination| destination.influence.contains(lat_rad, lon_rad))
        .max_by_key(|destination| (destination.priority, destination.id))
}

impl DestinationShape {
    /// Conservative geodetic bounding circle used to reject ordinary cells
    /// before allocating an otherwise-empty vector overlay.
    pub fn bounding_circle(self) -> (f64, f64, f64) {
        match self {
            Self::Circle {
                lat_deg,
                lon_deg,
                radius_m,
            } => (
                lat_deg.to_radians(),
                lon_deg.to_radians(),
                f64::from(radius_m),
            ),
            Self::OrientedRectangle {
                lat_deg,
                lon_deg,
                half_east_m,
                half_north_m,
                ..
            } => (
                lat_deg.to_radians(),
                lon_deg.to_radians(),
                f64::from(half_east_m).hypot(f64::from(half_north_m)),
            ),
            Self::Polygon { vertices_deg } => {
                let count = vertices_deg.len() as f64;
                let lat_deg = vertices_deg.iter().map(|point| point[0]).sum::<f64>() / count;
                let lon_deg = vertices_deg.iter().map(|point| point[1]).sum::<f64>() / count;
                let radius_m = vertices_deg
                    .iter()
                    .map(|point| {
                        let (east, north) = local_offset_m(
                            lat_deg,
                            lon_deg,
                            point[0].to_radians(),
                            point[1].to_radians(),
                        );
                        east.hypot(north)
                    })
                    .fold(0.0, f64::max);
                (lat_deg.to_radians(), lon_deg.to_radians(), radius_m)
            }
        }
    }

    pub fn contains(self, lat_rad: f64, lon_rad: f64) -> bool {
        match self {
            Self::Circle {
                lat_deg,
                lon_deg,
                radius_m,
            } => {
                let (e, n) = local_offset_m(lat_deg, lon_deg, lat_rad, lon_rad);
                e.hypot(n) <= f64::from(radius_m)
            }
            Self::OrientedRectangle {
                lat_deg,
                lon_deg,
                half_east_m,
                half_north_m,
                heading_deg,
            } => {
                let (e, n) = local_offset_m(lat_deg, lon_deg, lat_rad, lon_rad);
                let h = f64::from(heading_deg).to_radians();
                let along = e * h.cos() + n * h.sin();
                let across = -e * h.sin() + n * h.cos();
                along.abs() <= f64::from(half_east_m) && across.abs() <= f64::from(half_north_m)
            }
            Self::Polygon { vertices_deg } => point_in_polygon(lat_rad, lon_rad, vertices_deg),
        }
    }
}

fn local_offset_m(lat0_deg: f64, lon0_deg: f64, lat: f64, lon: f64) -> (f64, f64) {
    let lat0 = lat0_deg.to_radians();
    let lon0 = lon0_deg.to_radians();
    let dlon = (lon - lon0 + PI).rem_euclid(2.0 * PI) - PI;
    (dlon * MEAN_RADIUS * lat0.cos(), (lat - lat0) * MEAN_RADIUS)
}

fn point_in_polygon(lat: f64, lon: f64, vertices: &[[f64; 2]]) -> bool {
    let (lat, lon) = (lat.to_degrees(), lon.to_degrees());
    let mut inside = false;
    let mut previous = vertices[vertices.len() - 1];
    for &current in vertices {
        if (current[0] > lat) != (previous[0] > lat) {
            let crossing = (previous[1] - current[1]) * (lat - current[0])
                / (previous[0] - current[0])
                + current[1];
            inside ^= lon < crossing;
        }
        previous = current;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center(shape: DestinationShape) -> (f64, f64) {
        match shape {
            DestinationShape::Circle {
                lat_deg, lon_deg, ..
            }
            | DestinationShape::OrientedRectangle {
                lat_deg, lon_deg, ..
            } => (lat_deg.to_radians(), lon_deg.to_radians()),
            DestinationShape::Polygon { vertices_deg } => {
                let n = vertices_deg.len() as f64;
                (
                    (vertices_deg.iter().map(|p| p[0]).sum::<f64>() / n).to_radians(),
                    (vertices_deg.iter().map(|p| p[1]).sum::<f64>() / n).to_radians(),
                )
            }
        }
    }

    #[test]
    fn registry_v1_has_thirty_unique_complete_entries() {
        assert_eq!(FLOWER_DESTINATION_REGISTRY_VERSION, 1);
        assert_eq!(FLOWER_DESTINATIONS.len(), 30);
        for (index, entry) in FLOWER_DESTINATIONS.iter().enumerate() {
            assert_eq!(entry.id as usize, index + 1);
            assert!(!entry.name.is_empty() && !entry.source_note.is_empty());
            assert!((1..=12).contains(&entry.calendar.start_month));
            assert!((1..=12).contains(&entry.calendar.end_month));
            assert!(entry.density > 0 && entry.strip_width_m > 0.0);
        }
    }

    #[test]
    fn every_destination_resolves_at_its_authored_center() {
        for entry in &FLOWER_DESTINATIONS {
            let (lat, lon) = center(entry.influence);
            assert_eq!(
                query_flower_destination(lat, lon).map(|found| found.id),
                Some(entry.id),
                "{}",
                entry.name
            );
        }
    }

    #[test]
    fn destinations_are_bounded() {
        for entry in &FLOWER_DESTINATIONS {
            let (lat, lon) = center(entry.influence);
            let far = (lat + 8.0_f64.to_radians())
                .clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
            assert!(!entry.influence.contains(far, lon), "{}", entry.name);
        }
        assert!(query_flower_destination(0.0, 0.0).is_none());
    }

    #[test]
    fn registry_covers_every_mode_and_pattern() {
        for mode in [
            DestinationMode::ExactSite,
            DestinationMode::MappedFieldOverride,
            DestinationMode::EcologicalBoost,
        ] {
            assert!(FLOWER_DESTINATIONS.iter().any(|entry| entry.mode == mode));
        }
        for pattern in [
            FlowerPattern::ParallelStrips,
            FlowerPattern::AlternatingStrips,
            FlowerPattern::FormalBeds,
            FlowerPattern::BroadColorBlocks,
            FlowerPattern::Terraces,
            FlowerPattern::DenseNaturalMosaic,
            FlowerPattern::SparseSuperbloomClusters,
        ] {
            assert!(
                FLOWER_DESTINATIONS
                    .iter()
                    .any(|entry| entry.pattern == pattern)
            );
        }
    }

    #[test]
    fn northern_and_southern_tulips_share_local_spring_timing() {
        let north = flower_destination(FlowerDestinationId::Bollenstreek)
            .calendar
            .local_peak_phase(false);
        let south = flower_destination(FlowerDestinationId::TesselaarTulipFarm)
            .calendar
            .local_peak_phase(true);
        assert!((north - south).abs() < 0.10);
    }

    #[test]
    fn broad_natural_destinations_are_sparse_ecological_boosts() {
        for id in [
            FlowerDestinationId::Namaqualand,
            FlowerDestinationId::ValleyOfFlowers,
            FlowerDestinationId::AntelopeValley,
            FlowerDestinationId::CarrizoPlain,
            FlowerDestinationId::AnzaBorrego,
            FlowerDestinationId::TexasHillCountry,
            FlowerDestinationId::LakeTekapo,
            FlowerDestinationId::MountRainierParadise,
            FlowerDestinationId::AtacamaDesert,
        ] {
            let entry = flower_destination(id);
            assert_eq!(entry.mode, DestinationMode::EcologicalBoost);
            assert!(entry.density <= 118);
        }
    }
}
