//! Local material and color priors for ordinary buildings.
//!
//! Observed roofscapes, scope limitations and authored relative weights are
//! recorded in `assets/architecture/regional-finishes-SOURCES.md`.

use crate::architecture_choices::{Weighted, choose};
use crate::architecture_styles::ArchitectureStyle;
use crate::material;
use crate::regional_architecture::{BuildingDesign, ConstructionSystem, DetailFamily};
use vearth_data::vectors::RoofShape;

#[derive(Clone, Copy, Debug)]
pub struct ReviewedFinish {
    pub key: &'static str,
    pub label: &'static str,
    pub world_region: &'static str,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    // Elliptical influence limits in degrees, not surveyed heritage boundaries.
    latitude_radius: f64,
    longitude_radius: f64,
    fabrics: &'static [Weighted<LocalFabric>],
}

#[derive(Clone, Copy, Debug)]
struct LocalFabric {
    name: &'static str,
    wall_material: u8,
    wall_variant: u8,
    walls: &'static [Weighted<[u8; 3]>],
    pitched_roofs: &'static [Weighted<RoofFinish>],
    flat_roofs: &'static [Weighted<[u8; 3]>],
    joinery: &'static [Weighted<[u8; 3]>],
    trim: &'static [Weighted<[u8; 3]>],
    // Roof suggestions are inputs to the central source constraints.
    missing_roofs: &'static [Weighted<RoofShape>],
    restrained_style: Option<(ArchitectureStyle, u8, DetailFamily)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RoofFinish {
    material: u8,
    variant: u8,
    color: [u8; 3],
}

// Exact catalog identities only: nearby towns sharing a style are not owned.
// Explicit None entries make absent named records visible during catalog review.
const NAMED_PLACE_OWNERSHIP: &[(&str, Option<(&str, &str)>)] = &[
    ("bled", None),
    ("keswick", None),
    ("penrith", None),
    ("bath", Some(("wikidata:Q22889", "bath-georgian"))),
    ("albi", None),
    ("porto", Some(("geonames:2735943", "iberian-tile"))),
    ("visby", None),
    ("kyoto", Some(("geonames:1857910", "kyoto-machiya"))),
    ("okinawa", None),
    ("bhaktapur", Some(("geonames:1283617", "himalayan-timber"))),
    ("hoi-an", Some(("geonames:1580541", "hoi-an-merchant"))),
    (
        "luang-prabang",
        Some(("wikidata:Q190165", "luang-prabang-peninsula")),
    ),
    ("singapore", Some(("geonames:1880252", "straits-shophouse"))),
    ("santa-fe", Some(("geonames:5490263", "pueblo-terrace"))),
    ("san-diego", None),
    (
        "charleston",
        Some(("geonames:4574324", "north-american-porch")),
    ),
    ("quebec", Some(("wikidata:Q2114279", "quebec-old-town"))),
    ("bermuda", None),
    ("puebla", Some(("wikidata:Q3848455", "puebla-centro"))),
    ("zanzibar", Some(("geonames:148730", "swahili-stone-town"))),
    ("cape-town", Some(("geonames:3369157", "cape-gable"))),
    ("djenne", Some(("geonames:2458589", "sahel-relief"))),
    ("ait-ben-haddou", Some(("wikidata:Q309436", "atlas-kasbah"))),
    ("cuenca", Some(("geonames:3658666", "andean-gallery"))),
    ("arequipa", Some(("geonames:3947322", "hispanic-patio"))),
    (
        "paramaribo",
        Some(("geonames:3383330", "caribbean-gallery")),
    ),
    ("valparaiso", Some(("geonames:3868626", "chilean-cladding"))),
    ("brisbane", Some(("geonames:2174003", "queenslander"))),
    ("feilding", None),
    ("oamaru", Some(("wikidata:Q1584920", "oamaru-limestone"))),
];

/// Whether a reviewed local study owns this exact named catalog identity.
/// Ownership does not depend on a building's random local admission: the wider
/// legacy envelope must not recolor neighboring streets outside the study.
pub(crate) fn owns_place(source_id: &str, style_key: &str) -> bool {
    NAMED_PLACE_OWNERSHIP
        .iter()
        .any(|(_, identity)| *identity == Some((source_id, style_key)))
}

static REVIEWED_FINISHES: &[ReviewedFinish] = &[
    ReviewedFinish {
        key: "bled",
        label: "Bled ordinary plaster houses and mixed tile",
        world_region: "Europe",
        latitude_deg: 46.369,
        longitude_deg: 14.113,
        latitude_radius: 0.035,
        longitude_radius: 0.055,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "ordinary lime/plaster house",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([238, 238, 232], 7),
                        Weighted::new([226, 229, 225], 4),
                        Weighted::new([222, 219, 207], 2),
                        Weighted::new([238, 225, 180], 1),
                        Weighted::new([233, 226, 214], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 1,
                                color: [104, 109, 113],
                            },
                            6,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 1,
                                color: [77, 84, 91],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 1,
                                color: [126, 132, 135],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [190, 101, 59],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [171, 84, 51],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([83, 68, 53], 3),
                        Weighted::new([235, 235, 227], 5),
                        Weighted::new([109, 116, 113], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Gabled, 5),
                        Weighted::new(RoofShape::Hipped, 2),
                    ],
                    restrained_style: Some((
                        ArchitectureStyle::Contemporary,
                        0,
                        DetailFamily::Plain,
                    )),
                },
                12,
            ),
            Weighted::new(
                LocalFabric {
                    name: "house with restrained timber gallery",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([237, 236, 230], 6),
                        Weighted::new([224, 226, 222], 3),
                        Weighted::new([233, 226, 207], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 1,
                                color: [91, 98, 104],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [180, 94, 56],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Gabled, 5),
                        Weighted::new(RoofShape::Hipped, 2),
                    ],
                    restrained_style: Some((
                        ArchitectureStyle::AlpineGallery,
                        96,
                        DetailFamily::Shaded,
                    )),
                },
                1,
            ),
        ],
    },
    ReviewedFinish {
        key: "keswick",
        label: "Keswick slate and roughcast",
        world_region: "Europe",
        latitude_deg: 54.6,
        longitude_deg: -3.137,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "green-gray local stone terrace",
                    wall_material: material::STONE_WALL,
                    wall_variant: 1,
                    walls: &[
                        Weighted::new([149, 160, 154], 5),
                        Weighted::new([171, 178, 163], 2),
                        Weighted::new([137, 148, 151], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [72, 85, 96],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [92, 112, 109],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [111, 118, 119],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                5,
            ),
            Weighted::new(
                LocalFabric {
                    name: "limewashed and roughcast terrace",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([237, 233, 218], 4),
                        Weighted::new([220, 217, 198], 2),
                        Weighted::new([230, 221, 196], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [72, 85, 96],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [92, 112, 109],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [111, 118, 119],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                3,
            ),
        ],
    },
    ReviewedFinish {
        key: "penrith",
        label: "Penrith red sandstone",
        world_region: "Europe",
        latitude_deg: 54.664,
        longitude_deg: -2.753,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "red sandstone frontage",
                    wall_material: material::STONE_WALL,
                    wall_variant: 1,
                    walls: &[
                        Weighted::new([178, 126, 108], 5),
                        Weighted::new([190, 148, 121], 3),
                        Weighted::new([157, 122, 112], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [72, 85, 96],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [92, 112, 109],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [111, 118, 119],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                5,
            ),
            Weighted::new(
                LocalFabric {
                    name: "roughcast and limewash",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([236, 232, 215], 4),
                        Weighted::new([219, 211, 189], 2),
                        Weighted::new([231, 221, 193], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [72, 85, 96],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [92, 112, 109],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [111, 118, 119],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 1,
                                color: [131, 126, 112],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([44, 65, 89], 3),
                        Weighted::new([236, 235, 226], 4),
                        Weighted::new([135, 116, 132], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                3,
            ),
        ],
    },
    ReviewedFinish {
        key: "bath",
        label: "Bath limestone and Welsh slate",
        world_region: "Europe",
        latitude_deg: 51.381,
        longitude_deg: -2.36,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "Bath ashlar",
                    wall_material: material::STONE_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([219, 204, 169], 5),
                        Weighted::new([231, 220, 188], 3),
                        Weighted::new([199, 188, 163], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [72, 85, 96],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [92, 112, 109],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [111, 118, 119],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 1,
                                color: [138, 111, 89],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                8,
            ),
            Weighted::new(
                LocalFabric {
                    name: "lime render over rubble",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([230, 222, 199], 4),
                        Weighted::new([215, 208, 185], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [72, 85, 96],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [92, 112, 109],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SLATE_ROOF,
                                variant: 0,
                                color: [111, 118, 119],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                1,
            ),
        ],
    },
    ReviewedFinish {
        key: "albi",
        label: "Albi brick, render and weathered clay",
        world_region: "Europe",
        latitude_deg: 43.928,
        longitude_deg: 2.147,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "long red brick",
                    wall_material: material::BRICK_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([180, 117, 94], 5),
                        Weighted::new([194, 139, 110], 3),
                        Weighted::new([157, 102, 87], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [121, 107, 91],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [145, 113, 91],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [179, 108, 73],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([158, 185, 190], 3),
                        Weighted::new([147, 163, 137], 3),
                        Weighted::new([229, 224, 208], 2),
                        Weighted::new([80, 66, 54], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                5,
            ),
            Weighted::new(
                LocalFabric {
                    name: "lime render over masonry",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([205, 194, 169], 4),
                        Weighted::new([227, 218, 192], 3),
                        Weighted::new([187, 184, 169], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [121, 107, 91],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [145, 113, 91],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [179, 108, 73],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([159, 183, 187], 3),
                        Weighted::new([139, 158, 133], 3),
                        Weighted::new([234, 228, 211], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                4,
            ),
        ],
    },
    ReviewedFinish {
        key: "porto",
        label: "Porto granite and lime finishes",
        world_region: "Europe",
        latitude_deg: 41.145,
        longitude_deg: -8.615,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "painted or ceramic-faced masonry",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([235, 226, 204], 4),
                        Weighted::new([218, 180, 113], 3),
                        Weighted::new([140, 185, 204], 2),
                        Weighted::new([184, 90, 75], 2),
                        Weighted::new([228, 204, 170], 2),
                        Weighted::new([161, 189, 180], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [175, 99, 65],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [198, 119, 79],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [139, 94, 76],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([231, 231, 215], 5),
                        Weighted::new([54, 99, 81], 2),
                        Weighted::new([121, 77, 65], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                8,
            ),
            Weighted::new(
                LocalFabric {
                    name: "exposed granite",
                    wall_material: material::STONE_WALL,
                    wall_variant: 1,
                    walls: &[
                        Weighted::new([164, 163, 151], 3),
                        Weighted::new([187, 180, 160], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [175, 99, 65],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [198, 119, 79],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [139, 94, 76],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                1,
            ),
        ],
    },
    ReviewedFinish {
        key: "visby",
        label: "Visby lime render and clay",
        world_region: "Europe",
        latitude_deg: 57.64,
        longitude_deg: 18.295,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "limewashed roughcast",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([230, 215, 186], 5),
                        Weighted::new([211, 177, 164], 3),
                        Weighted::new([222, 194, 150], 3),
                        Weighted::new([235, 231, 218], 2),
                        Weighted::new([168, 170, 153], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [121, 107, 91],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [145, 113, 91],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [179, 108, 73],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                7,
            ),
            Weighted::new(
                LocalFabric {
                    name: "painted timber house",
                    wall_material: material::SIDING_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([159, 70, 53], 3),
                        Weighted::new([218, 190, 124], 2),
                        Weighted::new([222, 226, 217], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [175, 99, 65],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [198, 119, 79],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [139, 94, 76],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "kyoto",
        label: "Kyoto muted plaster and silver tile",
        world_region: "Asia",
        latitude_deg: 35.008,
        longitude_deg: 135.766,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "pale plaster housing",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([229, 227, 214], 5),
                        Weighted::new([196, 196, 181], 3),
                        Weighted::new([216, 209, 187], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [76, 86, 96],
                            },
                            6,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [104, 112, 119],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [140, 146, 148],
                            },
                            1,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [100, 112, 119],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [116, 94, 81],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                7,
            ),
            Weighted::new(
                LocalFabric {
                    name: "dark boarded machiya",
                    wall_material: material::TIMBER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([77, 65, 55], 4),
                        Weighted::new([99, 78, 58], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [85, 94, 104],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [112, 117, 123],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "okinawa",
        label: "Kitanakagusuku concrete and tiled housing",
        world_region: "Asia",
        latitude_deg: 26.324,
        longitude_deg: 127.8,
        latitude_radius: 0.023,
        longitude_radius: 0.030,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "ordinary rendered concrete house",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([236, 235, 226], 5),
                        Weighted::new([208, 212, 206], 3),
                        Weighted::new([228, 222, 201], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [166, 91, 57],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [70, 77, 79],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[
                        Weighted::new([229, 230, 221], 5),
                        Weighted::new([158, 166, 164], 3),
                    ],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Flat, 7),
                        Weighted::new(RoofShape::Hipped, 2),
                    ],
                    restrained_style: Some((
                        ArchitectureStyle::Contemporary,
                        0,
                        DetailFamily::Plain,
                    )),
                },
                8,
            ),
            Weighted::new(
                LocalFabric {
                    name: "traditional tiled house",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([220, 217, 199], 3),
                        Weighted::new([193, 196, 184], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [183, 103, 65],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [152, 92, 68],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [69, 77, 80],
                            },
                            3,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[Weighted::new(RoofShape::Hipped, 1)],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "bhaktapur",
        label: "Bhaktapur brick and timber",
        world_region: "Asia",
        latitude_deg: 27.672,
        longitude_deg: 85.429,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "brick residential street fabric",
                    wall_material: material::BRICK_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([174, 106, 80], 5),
                        Weighted::new([154, 94, 77], 3),
                        Weighted::new([189, 133, 101], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [121, 107, 91],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [145, 113, 91],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [179, 108, 73],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [158, 164, 166],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [150, 68, 59],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [66, 102, 130],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Gabled, 3),
                        Weighted::new(RoofShape::Flat, 2),
                    ],
                    restrained_style: None,
                },
                7,
            ),
            Weighted::new(
                LocalFabric {
                    name: "rendered concrete additions",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([224, 222, 208], 5),
                        Weighted::new([216, 166, 157], 2),
                        Weighted::new([132, 176, 181], 1),
                        Weighted::new([184, 190, 179], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [161, 171, 172],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [161, 67, 58],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [66, 112, 145],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Flat, 5),
                        Weighted::new(RoofShape::Gabled, 1),
                    ],
                    restrained_style: Some((
                        ArchitectureStyle::ContemporaryAsian,
                        0,
                        DetailFamily::Plain,
                    )),
                },
                3,
            ),
        ],
    },
    ReviewedFinish {
        key: "hoi-an",
        label: "Hoi An ochre lime and clay tile",
        world_region: "Asia",
        latitude_deg: 15.8774,
        longitude_deg: 108.3285,
        latitude_radius: 0.0013,
        longitude_radius: 0.0045,
        fabrics: &[Weighted::new(
            LocalFabric {
                name: "ochre and limewashed shophouse",
                wall_material: material::PLASTER_WALL,
                wall_variant: 0,
                walls: &[
                    Weighted::new([224, 185, 77], 6),
                    Weighted::new([211, 167, 65], 4),
                    Weighted::new([235, 225, 192], 2),
                    Weighted::new([216, 210, 186], 1),
                ],
                pitched_roofs: &[
                    Weighted::new(
                        RoofFinish {
                            material: material::TILE_ROOF,
                            variant: 0,
                            color: [106, 95, 82],
                        },
                        5,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::TILE_ROOF,
                            variant: 0,
                            color: [131, 105, 85],
                        },
                        4,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::TILE_ROOF,
                            variant: 0,
                            color: [158, 104, 78],
                        },
                        1,
                    ),
                ],
                flat_roofs: &[],
                joinery: &[
                    Weighted::new([75, 59, 46], 5),
                    Weighted::new([107, 81, 57], 2),
                ],
                trim: &[Weighted::new([235, 234, 223], 1)],
                missing_roofs: &[],
                restrained_style: None,
            },
            1,
        )],
    },
    ReviewedFinish {
        key: "luang-prabang",
        label: "Luang Prabang timber and ceramic",
        world_region: "Asia",
        latitude_deg: 19.8937,
        longitude_deg: 102.1402,
        latitude_radius: 0.0024,
        longitude_radius: 0.002,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "lime-rendered town house",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([233, 232, 217], 5),
                        Weighted::new([222, 215, 187], 3),
                        Weighted::new([235, 222, 190], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [104, 99, 87],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [123, 96, 77],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [165, 107, 76],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Gabled, 3),
                        Weighted::new(RoofShape::Hipped, 2),
                    ],
                    restrained_style: None,
                },
                6,
            ),
            Weighted::new(
                LocalFabric {
                    name: "timber dwelling",
                    wall_material: material::TIMBER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([121, 87, 62], 5),
                        Weighted::new([156, 120, 81], 2),
                        Weighted::new([91, 71, 57], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [121, 107, 91],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [145, 113, 91],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [179, 108, 73],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Gabled, 3),
                        Weighted::new(RoofShape::Hipped, 2),
                    ],
                    restrained_style: None,
                },
                3,
            ),
        ],
    },
    ReviewedFinish {
        key: "singapore",
        label: "Singapore Chinatown shophouses",
        world_region: "Asia",
        latitude_deg: 1.281,
        longitude_deg: 103.844,
        latitude_radius: 0.006,
        longitude_radius: 0.007,
        fabrics: &[Weighted::new(
            LocalFabric {
                name: "rendered conservation shophouse",
                wall_material: material::PLASTER_WALL,
                wall_variant: 0,
                walls: &[
                    Weighted::new([237, 235, 218], 6),
                    Weighted::new([194, 216, 217], 3),
                    Weighted::new([217, 209, 225], 2),
                    Weighted::new([231, 209, 160], 2),
                    Weighted::new([185, 210, 188], 2),
                    Weighted::new([215, 181, 179], 1),
                    Weighted::new([113, 140, 151], 1),
                ],
                pitched_roofs: &[
                    Weighted::new(
                        RoofFinish {
                            material: material::TILE_ROOF,
                            variant: 0,
                            color: [175, 99, 65],
                        },
                        5,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::TILE_ROOF,
                            variant: 0,
                            color: [198, 119, 79],
                        },
                        3,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::TILE_ROOF,
                            variant: 0,
                            color: [139, 94, 76],
                        },
                        2,
                    ),
                ],
                flat_roofs: &[],
                joinery: &[
                    Weighted::new([233, 233, 222], 5),
                    Weighted::new([52, 88, 71], 3),
                    Weighted::new([99, 68, 51], 1),
                ],
                trim: &[Weighted::new([235, 234, 223], 1)],
                missing_roofs: &[Weighted::new(RoofShape::Gabled, 1)],
                restrained_style: None,
            },
            1,
        )],
    },
    ReviewedFinish {
        key: "santa-fe",
        label: "Santa Fe earthen stucco",
        world_region: "North America",
        latitude_deg: 35.687,
        longitude_deg: -105.938,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "smooth earth-toned stucco",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([190, 149, 113], 5),
                        Weighted::new([207, 171, 131], 3),
                        Weighted::new([173, 132, 104], 3),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [140, 127, 113],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [110, 113, 111],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[
                        Weighted::new([190, 162, 127], 3),
                        Weighted::new([162, 144, 121], 2),
                        Weighted::new([213, 196, 166], 1),
                    ],
                    joinery: &[
                        Weighted::new([82, 67, 51], 5),
                        Weighted::new([72, 135, 136], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Flat, 7),
                        Weighted::new(RoofShape::Hipped, 1),
                    ],
                    restrained_style: None,
                },
                5,
            ),
            Weighted::new(
                LocalFabric {
                    name: "earthen adobe",
                    wall_material: material::EARTHEN_PLASTER,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([190, 149, 113], 5),
                        Weighted::new([207, 171, 131], 3),
                        Weighted::new([173, 132, 104], 3),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [140, 127, 113],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [110, 113, 111],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[
                        Weighted::new([190, 162, 127], 3),
                        Weighted::new([162, 144, 121], 2),
                        Weighted::new([213, 196, 166], 1),
                    ],
                    joinery: &[
                        Weighted::new([82, 67, 51], 5),
                        Weighted::new([72, 135, 136], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Flat, 7),
                        Weighted::new(RoofShape::Hipped, 1),
                    ],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "san-diego",
        label: "San Diego mission stucco and red tile",
        world_region: "North America",
        latitude_deg: 32.752,
        longitude_deg: -117.195,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "stucco residential bungalow",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([235, 228, 205], 5),
                        Weighted::new([218, 205, 174], 3),
                        Weighted::new([230, 219, 190], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [175, 99, 65],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [198, 119, 79],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [139, 94, 76],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SHINGLE_ROOF,
                                variant: 0,
                                color: [116, 110, 101],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Gabled, 5),
                        Weighted::new(RoofShape::Hipped, 2),
                    ],
                    restrained_style: None,
                },
                6,
            ),
            Weighted::new(
                LocalFabric {
                    name: "painted wood bungalow",
                    wall_material: material::SIDING_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([162, 190, 162], 3),
                        Weighted::new([234, 230, 210], 4),
                        Weighted::new([173, 197, 201], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::SHINGLE_ROOF,
                                variant: 0,
                                color: [110, 107, 100],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SHINGLE_ROOF,
                                variant: 0,
                                color: [86, 91, 93],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[Weighted::new(RoofShape::Gabled, 1)],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "charleston",
        label: "Charleston plaster and painted metal",
        world_region: "North America",
        latitude_deg: 32.787,
        longitude_deg: -79.939,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "painted wood single house",
                    wall_material: material::SIDING_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([235, 234, 220], 5),
                        Weighted::new([190, 199, 192], 2),
                        Weighted::new([122, 147, 163], 2),
                        Weighted::new([222, 182, 163], 2),
                        Weighted::new([220, 210, 174], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [172, 179, 178],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [156, 74, 65],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [106, 123, 133],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SHINGLE_ROOF,
                                variant: 0,
                                color: [86, 88, 87],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                6,
            ),
            Weighted::new(
                LocalFabric {
                    name: "stucco or limewash",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([234, 227, 207], 5),
                        Weighted::new([231, 186, 166], 2),
                        Weighted::new([195, 212, 194], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [164, 173, 173],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [167, 84, 72],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [111, 116, 121],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                3,
            ),
            Weighted::new(
                LocalFabric {
                    name: "exposed brick",
                    wall_material: material::BRICK_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([159, 101, 79], 3),
                        Weighted::new([181, 129, 99], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [153, 165, 166],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::SHINGLE_ROOF,
                                variant: 0,
                                color: [88, 91, 91],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                1,
            ),
        ],
    },
    ReviewedFinish {
        key: "quebec",
        label: "Quebec masonry and sheet metal",
        world_region: "North America",
        latitude_deg: 46.811,
        longitude_deg: -71.214,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "gray limestone townhouse",
                    wall_material: material::STONE_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([174, 176, 164], 4),
                        Weighted::new([198, 192, 168], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [181, 191, 195],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [109, 145, 161],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [72, 94, 112],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                4,
            ),
            Weighted::new(
                LocalFabric {
                    name: "painted masonry townhouse",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([236, 232, 216], 5),
                        Weighted::new([210, 209, 191], 2),
                        Weighted::new([229, 207, 165], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [178, 187, 192],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [87, 130, 154],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [148, 66, 61],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([52, 87, 76], 4),
                        Weighted::new([230, 231, 219], 3),
                        Weighted::new([132, 58, 50], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                4,
            ),
            Weighted::new(
                LocalFabric {
                    name: "brick townhouse",
                    wall_material: material::BRICK_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([155, 92, 72], 3),
                        Weighted::new([177, 112, 90], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [168, 183, 189],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 0,
                                color: [83, 114, 134],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "bermuda",
        label: "Bermuda limewashed roof stone",
        world_region: "North America",
        latitude_deg: 32.381,
        longitude_deg: -64.677,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[Weighted::new(
            LocalFabric {
                name: "painted limestone/render house",
                wall_material: material::PLASTER_WALL,
                wall_variant: 0,
                walls: &[
                    Weighted::new([234, 195, 184], 4),
                    Weighted::new([233, 234, 221], 4),
                    Weighted::new([233, 216, 162], 2),
                    Weighted::new([113, 201, 194], 2),
                    Weighted::new([185, 215, 197], 2),
                    Weighted::new([198, 205, 226], 1),
                ],
                pitched_roofs: &[
                    Weighted::new(
                        RoofFinish {
                            material: material::SLATE_ROOF,
                            variant: 0,
                            color: [239, 241, 233],
                        },
                        7,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::SLATE_ROOF,
                            variant: 0,
                            color: [221, 227, 220],
                        },
                        2,
                    ),
                ],
                flat_roofs: &[],
                joinery: &[
                    Weighted::new([64, 124, 95], 5),
                    Weighted::new([235, 236, 224], 3),
                ],
                trim: &[Weighted::new([235, 234, 223], 1)],
                missing_roofs: &[
                    Weighted::new(RoofShape::Hipped, 5),
                    Weighted::new(RoofShape::Gabled, 2),
                ],
                restrained_style: None,
            },
            1,
        )],
    },
    ReviewedFinish {
        key: "puebla",
        label: "Puebla painted masonry and flat terraces",
        world_region: "North America",
        latitude_deg: 19.043,
        longitude_deg: -98.198,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "painted masonry courtyard block",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([191, 86, 67], 3),
                        Weighted::new([220, 179, 92], 3),
                        Weighted::new([234, 226, 202], 5),
                        Weighted::new([103, 148, 179], 2),
                        Weighted::new([208, 167, 177], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [175, 99, 65],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [198, 119, 79],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [139, 94, 76],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[
                        Weighted::new([168, 78, 66], 5),
                        Weighted::new([142, 133, 118], 3),
                        Weighted::new([204, 201, 182], 2),
                    ],
                    joinery: &[
                        Weighted::new([75, 77, 67], 4),
                        Weighted::new([229, 223, 203], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[Weighted::new(RoofShape::Flat, 1)],
                    restrained_style: None,
                },
                8,
            ),
            Weighted::new(
                LocalFabric {
                    name: "exposed brick infill",
                    wall_material: material::BRICK_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([165, 110, 88], 3),
                        Weighted::new([183, 134, 105], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [166, 173, 172],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [119, 132, 137],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [74, 88, 100],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[Weighted::new(RoofShape::Flat, 1)],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "zanzibar",
        label: "Zanzibar coral limewash",
        world_region: "Africa",
        latitude_deg: -6.162,
        longitude_deg: 39.192,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[Weighted::new(
            LocalFabric {
                name: "limewashed coral masonry",
                wall_material: material::PLASTER_WALL,
                wall_variant: 0,
                walls: &[
                    Weighted::new([228, 223, 203], 5),
                    Weighted::new([212, 211, 191], 3),
                    Weighted::new([234, 231, 217], 2),
                    Weighted::new([226, 211, 167], 1),
                ],
                pitched_roofs: &[
                    Weighted::new(
                        RoofFinish {
                            material: material::METAL_ROOF,
                            variant: 1,
                            color: [164, 173, 173],
                        },
                        6,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::METAL_ROOF,
                            variant: 1,
                            color: [143, 97, 68],
                        },
                        3,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::METAL_ROOF,
                            variant: 1,
                            color: [179, 109, 70],
                        },
                        2,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::METAL_ROOF,
                            variant: 1,
                            color: [66, 167, 166],
                        },
                        1,
                    ),
                ],
                flat_roofs: &[],
                joinery: &[
                    Weighted::new([75, 59, 46], 5),
                    Weighted::new([107, 81, 57], 2),
                ],
                trim: &[Weighted::new([235, 234, 223], 1)],
                missing_roofs: &[
                    Weighted::new(RoofShape::Gabled, 5),
                    Weighted::new(RoofShape::Hipped, 2),
                ],
                restrained_style: None,
            },
            1,
        )],
    },
    ReviewedFinish {
        key: "cape-town",
        label: "Bo-Kaap painted plaster terraces",
        world_region: "Africa",
        latitude_deg: -33.9215,
        longitude_deg: 18.4155,
        latitude_radius: 0.004,
        longitude_radius: 0.005,
        fabrics: &[Weighted::new(
            LocalFabric {
                name: "Bo-Kaap painted plaster terrace",
                wall_material: material::PLASTER_WALL,
                wall_variant: 0,
                walls: &[
                    Weighted::new([218, 171, 53], 3),
                    Weighted::new([193, 92, 160], 2),
                    Weighted::new([74, 164, 197], 2),
                    Weighted::new([145, 179, 75], 2),
                    Weighted::new([220, 107, 59], 2),
                    Weighted::new([225, 190, 181], 1),
                    Weighted::new([235, 234, 216], 2),
                ],
                pitched_roofs: &[
                    Weighted::new(
                        RoofFinish {
                            material: material::METAL_ROOF,
                            variant: 1,
                            color: [160, 170, 172],
                        },
                        4,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::METAL_ROOF,
                            variant: 1,
                            color: [79, 89, 93],
                        },
                        2,
                    ),
                    Weighted::new(
                        RoofFinish {
                            material: material::METAL_ROOF,
                            variant: 1,
                            color: [152, 71, 61],
                        },
                        1,
                    ),
                ],
                flat_roofs: &[
                    Weighted::new([165, 166, 161], 3),
                    Weighted::new([211, 213, 207], 2),
                ],
                joinery: &[
                    Weighted::new([233, 233, 224], 7),
                    Weighted::new([68, 91, 80], 2),
                    Weighted::new([51, 65, 77], 1),
                ],
                trim: &[Weighted::new([235, 234, 223], 1)],
                missing_roofs: &[
                    Weighted::new(RoofShape::Flat, 6),
                    Weighted::new(RoofShape::Gabled, 2),
                ],
                restrained_style: None,
            },
            1,
        )],
    },
    ReviewedFinish {
        key: "djenne",
        label: "Djenne maintained earthen render",
        world_region: "Africa",
        latitude_deg: 13.906,
        longitude_deg: -4.555,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[Weighted::new(
            LocalFabric {
                name: "maintained earthen courtyard",
                wall_material: material::EARTHEN_PLASTER,
                wall_variant: 0,
                walls: &[
                    Weighted::new([183, 150, 111], 5),
                    Weighted::new([204, 174, 134], 3),
                    Weighted::new([167, 139, 107], 2),
                ],
                pitched_roofs: &[],
                flat_roofs: &[
                    Weighted::new([189, 159, 120], 5),
                    Weighted::new([205, 181, 146], 2),
                ],
                joinery: &[
                    Weighted::new([93, 78, 60], 4),
                    Weighted::new([161, 69, 55], 1),
                    Weighted::new([118, 157, 150], 1),
                ],
                trim: &[],
                missing_roofs: &[Weighted::new(RoofShape::Flat, 1)],
                restrained_style: None,
            },
            1,
        )],
    },
    ReviewedFinish {
        key: "ait-ben-haddou",
        label: "Ait Ben Haddou red earthen finish",
        world_region: "Africa",
        latitude_deg: 31.04722,
        longitude_deg: -7.12889,
        latitude_radius: 0.001,
        longitude_radius: 0.0012,
        fabrics: &[Weighted::new(
            LocalFabric {
                name: "earthen kasbah dwelling",
                wall_material: material::EARTHEN_PLASTER,
                wall_variant: 0,
                walls: &[
                    Weighted::new([189, 128, 89], 5),
                    Weighted::new([206, 148, 105], 3),
                    Weighted::new([174, 117, 85], 1),
                ],
                pitched_roofs: &[],
                flat_roofs: &[
                    Weighted::new([183, 133, 98], 3),
                    Weighted::new([201, 153, 116], 2),
                ],
                joinery: &[
                    Weighted::new([75, 59, 46], 5),
                    Weighted::new([107, 81, 57], 2),
                ],
                trim: &[],
                missing_roofs: &[Weighted::new(RoofShape::Flat, 1)],
                restrained_style: None,
            },
            1,
        )],
    },
    ReviewedFinish {
        key: "cuenca",
        label: "Cuenca adobe render and clay tile",
        world_region: "South America",
        latitude_deg: -2.897,
        longitude_deg: -79.005,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "rendered town house",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([232, 227, 209], 5),
                        Weighted::new([222, 207, 167], 3),
                        Weighted::new([222, 188, 174], 1),
                        Weighted::new([204, 200, 176], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [175, 99, 65],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [198, 119, 79],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [139, 94, 76],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 2,
                                color: [164, 169, 163],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 2,
                                color: [138, 155, 110],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Gabled, 5),
                        Weighted::new(RoofShape::Hipped, 2),
                        Weighted::new(RoofShape::Flat, 1),
                    ],
                    restrained_style: None,
                },
                7,
            ),
            Weighted::new(
                LocalFabric {
                    name: "brick and concrete infill",
                    wall_material: material::BRICK_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([182, 126, 101], 3),
                        Weighted::new([163, 111, 94], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 2,
                                color: [160, 169, 164],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 2,
                                color: [182, 128, 115],
                            },
                            1,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [175, 99, 65],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [198, 119, 79],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 0,
                                color: [139, 94, 76],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[
                        Weighted::new(RoofShape::Gabled, 2),
                        Weighted::new(RoofShape::Flat, 1),
                    ],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "arequipa",
        label: "Arequipa pearl and rose sillar",
        world_region: "South America",
        latitude_deg: -16.398,
        longitude_deg: -71.537,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "sillar street frontage",
                    wall_material: material::STONE_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([231, 226, 209], 5),
                        Weighted::new([212, 210, 192], 2),
                        Weighted::new([231, 214, 205], 1),
                    ],
                    pitched_roofs: &[],
                    flat_roofs: &[
                        Weighted::new([190, 185, 164], 3),
                        Weighted::new([215, 212, 193], 2),
                    ],
                    joinery: &[
                        Weighted::new([75, 59, 46], 5),
                        Weighted::new([107, 81, 57], 2),
                    ],
                    trim: &[],
                    missing_roofs: &[Weighted::new(RoofShape::Flat, 1)],
                    restrained_style: None,
                },
                5,
            ),
            Weighted::new(
                LocalFabric {
                    name: "plastered masonry infill",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([235, 229, 211], 5),
                        Weighted::new([210, 199, 170], 2),
                        Weighted::new([220, 192, 162], 1),
                    ],
                    pitched_roofs: &[Weighted::new(
                        RoofFinish {
                            material: material::METAL_ROOF,
                            variant: 1,
                            color: [152, 159, 159],
                        },
                        1,
                    )],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[Weighted::new(RoofShape::Flat, 1)],
                    restrained_style: None,
                },
                4,
            ),
        ],
    },
    ReviewedFinish {
        key: "paramaribo",
        label: "Paramaribo painted timber",
        world_region: "South America",
        latitude_deg: 5.827,
        longitude_deg: -55.153,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "painted timber heritage house",
                    wall_material: material::SIDING_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([237, 236, 222], 6),
                        Weighted::new([220, 223, 211], 2),
                        Weighted::new([233, 220, 178], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [169, 64, 54],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [174, 185, 187],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [84, 105, 118],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([57, 85, 66], 5),
                        Weighted::new([234, 235, 222], 2),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                7,
            ),
            Weighted::new(
                LocalFabric {
                    name: "painted masonry infill",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([235, 233, 218], 5),
                        Weighted::new([234, 197, 181], 2),
                        Weighted::new([219, 212, 171], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [172, 183, 184],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [167, 76, 66],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [101, 148, 105],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                3,
            ),
        ],
    },
    ReviewedFinish {
        key: "valparaiso",
        label: "Valparaiso painted coastal cladding",
        world_region: "South America",
        latitude_deg: -33.044,
        longitude_deg: -71.628,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "painted corrugated metal on frame",
                    wall_material: material::PANEL_WALL,
                    wall_variant: 1,
                    walls: &[
                        Weighted::new([92, 113, 175], 4),
                        Weighted::new([209, 91, 77], 3),
                        Weighted::new([216, 193, 99], 3),
                        Weighted::new([119, 181, 170], 2),
                        Weighted::new([213, 158, 164], 2),
                        Weighted::new([220, 222, 207], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [153, 101, 78],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [174, 176, 165],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [81, 111, 129],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                7,
            ),
            Weighted::new(
                LocalFabric {
                    name: "painted render",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([231, 220, 191], 3),
                        Weighted::new([193, 118, 92], 2),
                        Weighted::new([125, 179, 179], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [166, 173, 172],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [119, 132, 137],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [74, 88, 100],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "brisbane",
        label: "Brisbane weatherboard and iron",
        world_region: "Oceania",
        latitude_deg: -27.461,
        longitude_deg: 153.006,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "weatherboard Queenslander",
                    wall_material: material::SIDING_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([233, 229, 211], 5),
                        Weighted::new([215, 203, 172], 3),
                        Weighted::new([174, 198, 184], 2),
                        Weighted::new([177, 205, 210], 2),
                        Weighted::new([125, 151, 166], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [163, 177, 184],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [86, 124, 148],
                            },
                            3,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [172, 81, 64],
                            },
                            1,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [69, 99, 88],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([234, 234, 222], 6),
                        Weighted::new([89, 124, 145], 2),
                        Weighted::new([85, 112, 89], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                7,
            ),
            Weighted::new(
                LocalFabric {
                    name: "plastered infill",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([237, 236, 230], 6),
                        Weighted::new([224, 226, 222], 3),
                        Weighted::new([233, 226, 207], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [166, 173, 172],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [119, 132, 137],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [74, 88, 100],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
    ReviewedFinish {
        key: "feilding",
        label: "Feilding weatherboard and iron",
        world_region: "Oceania",
        latitude_deg: -40.225,
        longitude_deg: 175.566,
        latitude_radius: 0.012,
        longitude_radius: 0.016,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "weatherboard cottage",
                    wall_material: material::SIDING_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([235, 236, 228], 6),
                        Weighted::new([217, 214, 189], 2),
                        Weighted::new([172, 197, 190], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [62, 77, 94],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [157, 170, 174],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [96, 127, 104],
                            },
                            1,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [155, 82, 68],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                5,
            ),
            Weighted::new(
                LocalFabric {
                    name: "rendered/block housing",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([216, 215, 191], 4),
                        Weighted::new([231, 228, 209], 2),
                        Weighted::new([195, 196, 182], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [178, 175, 143],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [140, 151, 146],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::TILE_ROOF,
                                variant: 1,
                                color: [123, 128, 118],
                            },
                            2,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                3,
            ),
        ],
    },
    ReviewedFinish {
        key: "oamaru",
        label: "Oamaru creamy limestone",
        world_region: "Oceania",
        latitude_deg: -45.100,
        longitude_deg: 170.970,
        latitude_radius: 0.008,
        longitude_radius: 0.010,
        fabrics: &[
            Weighted::new(
                LocalFabric {
                    name: "limestone historic frontage",
                    wall_material: material::STONE_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([232, 226, 199], 6),
                        Weighted::new([217, 211, 183], 3),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [163, 72, 57],
                            },
                            4,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [143, 154, 152],
                            },
                            3,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([74, 123, 99], 4),
                        Weighted::new([93, 55, 45], 2),
                        Weighted::new([228, 228, 207], 1),
                    ],
                    trim: &[],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                6,
            ),
            Weighted::new(
                LocalFabric {
                    name: "lime render and later infill",
                    wall_material: material::PLASTER_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([237, 236, 230], 6),
                        Weighted::new([224, 226, 222], 3),
                        Weighted::new([233, 226, 207], 2),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [166, 173, 172],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [119, 132, 137],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [74, 88, 100],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                2,
            ),
            Weighted::new(
                LocalFabric {
                    name: "weatherboard house",
                    wall_material: material::SIDING_WALL,
                    wall_variant: 0,
                    walls: &[
                        Weighted::new([232, 231, 214], 5),
                        Weighted::new([192, 210, 201], 1),
                    ],
                    pitched_roofs: &[
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [166, 173, 172],
                            },
                            5,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [119, 132, 137],
                            },
                            2,
                        ),
                        Weighted::new(
                            RoofFinish {
                                material: material::METAL_ROOF,
                                variant: 1,
                                color: [74, 88, 100],
                            },
                            1,
                        ),
                    ],
                    flat_roofs: &[],
                    joinery: &[
                        Weighted::new([233, 233, 224], 7),
                        Weighted::new([68, 91, 80], 2),
                        Weighted::new([51, 65, 77], 1),
                    ],
                    trim: &[Weighted::new([235, 234, 223], 1)],
                    missing_roofs: &[],
                    restrained_style: None,
                },
                2,
            ),
        ],
    },
];

impl ReviewedFinish {
    pub fn fabric_names(&self) -> impl Iterator<Item = (&'static str, u16)> + '_ {
        self.fabrics.iter().map(|f| (f.value.name, f.weight))
    }
}

pub fn reviewed_finishes() -> &'static [ReviewedFinish] {
    REVIEWED_FINISHES
}

fn influence_at(profile: &ReviewedFinish, lat: f64, lon: f64) -> f32 {
    let y = (lat - profile.latitude_deg) / profile.latitude_radius;
    let x = (lon - profile.longitude_deg) / profile.longitude_radius;
    ((1.0 - x * x - y * y) / 0.75).clamp(0.0, 1.0) as f32
}

/// One selected local fabric, independent of any broader architectural prior.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalRecipe {
    profile: &'static ReviewedFinish,
    fabric: LocalFabric,
}

impl LocalRecipe {
    pub(crate) fn key(self) -> &'static str {
        self.profile.key
    }

    pub(crate) fn fabric_name(self) -> &'static str {
        self.fabric.name
    }

    pub(crate) fn suggested_roof(self, state: u64) -> RoofShape {
        choose(self.fabric.missing_roofs, state, 0x5348_4150_45).unwrap_or(RoofShape::Gabled)
    }

    /// Resolve surfaces for the roof selected by the central source constraints.
    /// Unknown coverings use neutral generic material rather than borrowing a
    /// country or place finish from outside this recipe.
    pub(crate) fn resolve(self, roof_shape: RoofShape, state: u64) -> BuildingDesign {
        let fabric = self.fabric;
        let wall = choose(fabric.walls, state, 0x5741_4c4c).unwrap_or([218, 218, 211]);
        let construction = match fabric.wall_material {
            material::STONE_WALL | material::BRICK_WALL => ConstructionSystem::Masonry,
            material::TIMBER_WALL | material::SIDING_WALL => ConstructionSystem::Timber,
            material::PANEL_WALL if fabric.wall_variant == 1 => ConstructionSystem::Timber,
            material::EARTHEN_PLASTER => ConstructionSystem::Earth,
            _ => ConstructionSystem::Rendered,
        };
        let restrained = if fabric.wall_material == material::BRICK_WALL {
            ArchitectureStyle::ContemporaryBrick
        } else {
            ArchitectureStyle::Contemporary
        };
        let (style, expression, detail) =
            fabric
                .restrained_style
                .unwrap_or((restrained, 0, DetailFamily::Plain));
        let covering = if roof_shape == RoofShape::Flat {
            RoofFinish {
                material: material::FLAT_ROOF,
                variant: 0,
                color: choose(fabric.flat_roofs, state, 0x464c_4154).unwrap_or([161, 164, 164]),
            }
        } else if matches!(
            roof_shape,
            RoofShape::Gabled
                | RoofShape::Hipped
                | RoofShape::Pyramidal
                | RoofShape::Mansard
                | RoofShape::Gambrel
                | RoofShape::Skillion
        ) {
            choose(fabric.pitched_roofs, state, 0x524f_4f46).unwrap_or(RoofFinish {
                material: material::ROOF,
                variant: 0,
                color: [124, 128, 131],
            })
        } else {
            RoofFinish {
                material: material::ROOF,
                variant: 0,
                color: [124, 128, 131],
            }
        };
        BuildingDesign {
            style,
            expression,
            construction,
            detail,
            wall,
            wall_material: fabric.wall_material,
            wall_variant: fabric.wall_variant,
            roof: vary_roof(covering.color, state),
            roof_material: covering.material,
            roof_variant: covering.variant,
            inferred_roof: roof_shape,
            // Unpainted earthen and stone moldings inherit their wall pigment.
            accent: choose(fabric.trim, state, 0x5452_494d).unwrap_or(wall),
            joinery: choose(fabric.joinery, state, 0x4a4f_494e_4552_59).unwrap_or([88, 84, 75]),
            weathering: 8 + ((state >> 24) & 31) as u8,
            ..BuildingDesign::default()
        }
    }
}

/// Select a complete local recipe. The caller owns source-use and scale eligibility.
pub(crate) fn recipe_at(location: (f64, f64), state: u64) -> Option<LocalRecipe> {
    let lat = location.0.to_degrees();
    let lon = location.1.to_degrees();
    let (profile, influence) = REVIEWED_FINISHES
        .iter()
        .filter_map(|p| {
            let weight = influence_at(p, lat, lon);
            (weight > 0.0).then_some((p, weight))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    // Admit a whole recipe at the boundary, never pigments interpolated from
    // independent local and national catalogs.
    let admission = [
        Weighted::new(true, (influence * 65535.0).round() as u16),
        Weighted::new(false, ((1.0 - influence) * 65535.0).round() as u16),
    ];
    if choose(&admission, state, 0x4c4f_4341_4c42) != Some(true) {
        return None;
    }
    Some(LocalRecipe {
        profile,
        fabric: choose(profile.fabrics, state, 0x4641_4252_4943)?,
    })
}

pub(crate) fn vary_roof(color: [u8; 3], state: u64) -> [u8; 3] {
    // Neutral coverings remain neutral; pigment variation is authored in the
    // weighted finish, while this small value shift avoids repeated flat fills.
    let light = ((state >> 32) % 13) as i16 - 6;
    color.map(|channel| (i16::from(channel) + light).clamp(0, 255) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn location(profile: &ReviewedFinish) -> (f64, f64) {
        (
            profile.latitude_deg.to_radians(),
            profile.longitude_deg.to_radians(),
        )
    }

    #[test]
    fn local_ownership_is_complete_and_exact_for_named_catalog_entries() {
        let keys: HashSet<_> = NAMED_PLACE_OWNERSHIP.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys.len(), reviewed_finishes().len());
        assert!(reviewed_finishes().iter().all(|p| keys.contains(p.key)));
        for (id, style) in [
            ("wikidata:Q309436", "atlas-kasbah"),
            ("geonames:1580541", "hoi-an-merchant"),
            ("wikidata:Q190165", "luang-prabang-peninsula"),
            ("geonames:3868626", "chilean-cladding"),
        ] {
            assert!(owns_place(id, style));
            assert!(!owns_place(id, "contemporary"));
            assert!(!owns_place("unrelated-town", style));
        }
        assert!(!owns_place("wikidata:Q16593169", "atlas-kasbah"));
    }

    #[test]
    fn local_catalog_contains_complete_weighted_fabrics_with_bounded_scope() {
        let profiles = reviewed_finishes();
        assert_eq!(
            profiles.iter().map(|p| p.key).collect::<HashSet<_>>().len(),
            profiles.len()
        );
        for p in profiles {
            assert!(p.latitude_radius > 0.0 && p.longitude_radius > 0.0);
            assert_eq!(influence_at(p, p.latitude_deg, p.longitude_deg), 1.0);
            assert_eq!(influence_at(p, p.latitude_deg + 1.0, p.longitude_deg), 0.0);
            assert!(!p.fabrics.is_empty());
            for weighted in p.fabrics {
                let f = weighted.value;
                assert!(weighted.weight > 0 && !f.name.is_empty());
                assert!(!f.walls.is_empty() && !f.joinery.is_empty());
                assert!(f.walls.iter().all(|c| c.weight > 0));
                for roof in f.pitched_roofs {
                    assert!(roof.weight > 0);
                    assert_ne!(roof.value.material, material::FLAT_ROOF);
                    assert!(roof.value.variant <= 2);
                    assert!(
                        roof.value.variant == 0
                            || matches!(
                                roof.value.material,
                                material::TILE_ROOF | material::METAL_ROOF | material::SHINGLE_ROOF
                            )
                    );
                }
            }
        }
    }

    #[test]
    fn recipe_resolves_compatible_surfaces_for_the_centrally_selected_roof() {
        for p in reviewed_finishes() {
            let recipe = recipe_at(location(p), 41).unwrap();
            assert_eq!(recipe.key(), p.key);
            assert!(
                p.fabric_names()
                    .any(|(name, _)| name == recipe.fabric_name())
            );
            for roof in [RoofShape::Flat, RoofShape::Gabled, RoofShape::Mansard] {
                let design = recipe.resolve(roof, 41);
                assert_eq!(design.inferred_roof, roof);
                assert_eq!(design, recipe.resolve(roof, 41));
                if roof == RoofShape::Flat {
                    assert_eq!(
                        (design.roof_material, design.roof_variant),
                        (material::FLAT_ROOF, 0)
                    );
                } else {
                    assert_ne!(design.roof_material, material::FLAT_ROOF);
                }
            }
        }
        assert!(recipe_at((0.0, 0.0), 41).is_none());
        assert!(recipe_at((f64::NAN, 0.0), 41).is_none());
    }

    #[test]
    fn historic_studies_exclude_modern_neighbors_and_keep_observed_roof_shapes() {
        for (key, outside) in [
            ("ait-ben-haddou", (31.0426_f64, -7.1328_f64)),
            ("hoi-an", (15.886_f64, 108.338_f64)),
            ("luang-prabang", (19.881_f64, 102.134_f64)),
        ] {
            let profile = reviewed_finishes().iter().find(|p| p.key == key).unwrap();
            assert_eq!(influence_at(profile, outside.0, outside.1), 0.0);
            assert_eq!(recipe_at(location(profile), 41).unwrap().key(), key);
        }
        let luang = reviewed_finishes()
            .iter()
            .find(|p| p.key == "luang-prabang")
            .unwrap();
        let shapes: Vec<_> = (0..64)
            .map(|state| {
                recipe_at(location(luang), state)
                    .unwrap()
                    .suggested_roof(state)
            })
            .collect();
        assert!(shapes.contains(&RoofShape::Gabled));
        assert!(shapes.contains(&RoofShape::Hipped));
    }

    #[test]
    fn bled_local_recipe_owns_gray_and_orange_roofs_and_restrained_house_mix() {
        let p = &reviewed_finishes()[0];
        let mut gray = 0;
        let mut orange = 0;
        let mut simple = 0;
        let mut combinations = HashSet::new();
        for state in 0..256 {
            let recipe = recipe_at(location(p), state).unwrap();
            let design = recipe.resolve(recipe.suggested_roof(state), state);
            assert!(matches!(
                design.inferred_roof,
                RoofShape::Gabled | RoofShape::Hipped
            ));
            assert_eq!(design.roof_material, material::TILE_ROOF);
            assert_eq!(design.wall_material, material::PLASTER_WALL);
            gray += usize::from(design.roof_variant == 1);
            orange += usize::from(design.roof_variant == 0);
            simple += usize::from(
                design.style == ArchitectureStyle::Contemporary && design.expression == 0,
            );
            combinations.insert((design.wall, design.roof_variant));
        }
        assert!(gray > orange && orange > 0);
        assert!(simple > 128 && simple < 256);
        assert!(
            combinations
                .iter()
                .any(|(wall, _)| combinations.contains(&(*wall, 0))
                    && combinations.contains(&(*wall, 1)))
        );
    }

    #[test]
    fn bhaktapur_local_recipe_owns_brick_clay_metal_and_flat_infill() {
        let p = reviewed_finishes()
            .iter()
            .find(|p| p.key == "bhaktapur")
            .unwrap();
        let mut finishes = HashSet::new();
        let mut shapes = Vec::new();
        for state in 0..256 {
            let recipe = recipe_at(location(p), state).unwrap();
            let design = recipe.resolve(recipe.suggested_roof(state), state);
            finishes.insert((design.wall_material, design.roof_material));
            shapes.push(design.inferred_roof);
        }
        assert!(finishes.contains(&(material::BRICK_WALL, material::TILE_ROOF)));
        assert!(finishes.contains(&(material::BRICK_WALL, material::METAL_ROOF)));
        assert!(finishes.contains(&(material::PLASTER_WALL, material::FLAT_ROOF)));
        assert!(shapes.contains(&RoofShape::Flat) && shapes.contains(&RoofShape::Gabled));
    }
}
