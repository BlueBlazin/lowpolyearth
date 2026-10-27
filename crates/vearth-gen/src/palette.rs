//! Shared surface palette for the faceted world.
//!
//! Source imagery determines local identity; these anchors coordinate
//! procedural gaps, nearby semantic surfaces, and source colour treatment.
//! Values are unlit, display-referred sRGB bytes expressed as `f32` because
//! generation blends them before quantization. Sunlight and sky colour are
//! applied by the renderer, never baked into these material values.

pub type Rgb = [f32; 3];

pub mod terrain {
    use super::Rgb;

    pub const SEABED: Rgb = [58.0, 68.0, 74.0];
    pub const SEABED_SILT: Rgb = [83.0, 84.0, 76.0];
    pub const SNOW: Rgb = [235.0, 237.0, 232.0];
    pub const DEEP_SNOW: Rgb = [219.0, 228.0, 230.0];
    pub const OLD_SNOW: Rgb = [207.0, 211.0, 204.0];
    pub const GLACIER_ICE: Rgb = [181.0, 211.0, 218.0];
    pub const DIRTY_ICE: Rgb = [151.0, 165.0, 158.0];
    pub const ROCK: Rgb = [145.0, 139.0, 125.0];
    pub const HIGH_ROCK: Rgb = [132.0, 139.0, 136.0];
    pub const ARID_ROCK: Rgb = [139.0, 111.0, 88.0];
    pub const BASALT: Rgb = [61.0, 61.0, 59.0];
    pub const OBSIDIAN: Rgb = [46.0, 44.0, 43.0];
    pub const SLATE: Rgb = [78.0, 83.0, 87.0];
    pub const SHALE: Rgb = [92.0, 83.0, 76.0];
    pub const GRANITE: Rgb = [163.0, 155.0, 143.0];
    pub const PALE_GRANITE: Rgb = [185.0, 181.0, 165.0];
    pub const GNEISS: Rgb = [139.0, 138.0, 129.0];
    pub const LIMESTONE: Rgb = [188.0, 177.0, 151.0];
    pub const CHALK: Rgb = [211.0, 207.0, 191.0];
    pub const WARM_LIMESTONE: Rgb = [178.0, 148.0, 112.0];
    pub const TAN_SANDSTONE: Rgb = [164.0, 124.0, 88.0];
    pub const RED_SANDSTONE: Rgb = [151.0, 79.0, 55.0];
    pub const VERMILION_SANDSTONE: Rgb = [174.0, 86.0, 57.0];
    pub const IRONSTONE: Rgb = [123.0, 68.0, 52.0];
    pub const TUFF: Rgb = [135.0, 119.0, 101.0];
    pub const FRESH_LAVA: Rgb = [53.0, 48.0, 44.0];
    pub const VOLCANIC_ASH: Rgb = [86.0, 83.0, 78.0];
    pub const SCREE: Rgb = [123.0, 112.0, 100.0];
    pub const COAST_SAND: Rgb = [193.0, 178.0, 137.0];
    pub const PALE_SAND: Rgb = [218.0, 204.0, 165.0];
    pub const DUNE_SAND: Rgb = [207.0, 185.0, 142.0];
    pub const GOLD_DUNE: Rgb = [196.0, 158.0, 105.0];
    pub const IRON_SAND: Rgb = [181.0, 137.0, 104.0];
    pub const RED_DUNE: Rgb = [166.0, 100.0, 68.0];
    pub const GYPSUM: Rgb = [226.0, 219.0, 198.0];
    pub const SALT: Rgb = [224.0, 222.0, 210.0];
    pub const LOESS: Rgb = [181.0, 158.0, 112.0];
    pub const DRY_CLAY: Rgb = [156.0, 116.0, 84.0];
    pub const RED_CLAY: Rgb = [141.0, 78.0, 55.0];
    pub const ALLUVIUM: Rgb = [133.0, 116.0, 91.0];
    pub const DARK_SOIL: Rgb = [101.0, 81.0, 61.0];
    pub const WET_SOIL: Rgb = [74.0, 67.0, 53.0];
    pub const LATERITE: Rgb = [132.0, 69.0, 47.0];
    pub const TUNDRA_DRY: Rgb = [126.0, 116.0, 91.0];
    pub const TUNDRA_LICHEN: Rgb = [109.0, 118.0, 92.0];
    pub const ALPINE_MEADOW: Rgb = [121.0, 147.0, 103.0];
    pub const BOREAL_DARK: Rgb = [68.0, 96.0, 79.0];
    pub const BOREAL_GREEN: Rgb = [82.0, 112.0, 94.0];
    pub const TEMPERATE_DARK: Rgb = [83.0, 111.0, 69.0];
    pub const TEMPERATE_GREEN: Rgb = [119.0, 143.0, 80.0];
    pub const TROPICAL_DARK: Rgb = [64.0, 104.0, 73.0];
    pub const TROPICAL_GREEN: Rgb = [86.0, 123.0, 82.0];
    pub const DRY_SCRUB: Rgb = [157.0, 146.0, 106.0];
    pub const HEATH: Rgb = [112.0, 117.0, 92.0];
    pub const DRY_WOODLAND: Rgb = [119.0, 119.0, 82.0];
    pub const HUMID_MEADOW: Rgb = [112.0, 147.0, 96.0];
    pub const STEPPE: Rgb = [152.0, 148.0, 105.0];
    pub const SAVANNA: Rgb = [154.0, 149.0, 91.0];
    pub const CHAPARRAL: Rgb = [119.0, 129.0, 91.0];
    pub const PRAIRIE: Rgb = [137.0, 155.0, 96.0];
    pub const MEDITERRANEAN: Rgb = [126.0, 135.0, 91.0];
    pub const TEMPERATE_FOREST: Rgb = [67.0, 94.0, 69.0];
    pub const RAINFOREST: Rgb = [54.0, 91.0, 65.0];
    pub const CLOUD_FOREST: Rgb = [73.0, 108.0, 88.0];
    pub const WETLAND: Rgb = [88.0, 113.0, 85.0];
    pub const PEAT: Rgb = [73.0, 69.0, 48.0];

    /// Auditable authored vocabulary. These are appearance anchors, not one
    /// renderer material per colour: physically similar families deliberately
    /// share ROCK, SOIL, GRASS, ICE, and so on.
    pub const AUTHORED_SURFACE_ANCHORS: &[Rgb] = &[
        SEABED,
        SEABED_SILT,
        SNOW,
        DEEP_SNOW,
        OLD_SNOW,
        GLACIER_ICE,
        DIRTY_ICE,
        ROCK,
        HIGH_ROCK,
        ARID_ROCK,
        BASALT,
        OBSIDIAN,
        SLATE,
        SHALE,
        GRANITE,
        PALE_GRANITE,
        GNEISS,
        LIMESTONE,
        CHALK,
        WARM_LIMESTONE,
        TAN_SANDSTONE,
        RED_SANDSTONE,
        VERMILION_SANDSTONE,
        IRONSTONE,
        TUFF,
        FRESH_LAVA,
        VOLCANIC_ASH,
        SCREE,
        COAST_SAND,
        PALE_SAND,
        DUNE_SAND,
        GOLD_DUNE,
        IRON_SAND,
        RED_DUNE,
        GYPSUM,
        SALT,
        LOESS,
        DRY_CLAY,
        RED_CLAY,
        ALLUVIUM,
        DARK_SOIL,
        WET_SOIL,
        LATERITE,
        TUNDRA_DRY,
        TUNDRA_LICHEN,
        ALPINE_MEADOW,
        BOREAL_DARK,
        BOREAL_GREEN,
        TEMPERATE_DARK,
        TEMPERATE_GREEN,
        TROPICAL_DARK,
        TROPICAL_GREEN,
        DRY_SCRUB,
        HEATH,
        DRY_WOODLAND,
        HUMID_MEADOW,
        STEPPE,
        SAVANNA,
        CHAPARRAL,
        PRAIRIE,
        MEDITERRANEAN,
        TEMPERATE_FOREST,
        RAINFOREST,
        CLOUD_FOREST,
        WETLAND,
        PEAT,
    ];
}

pub mod infrastructure {
    use super::Rgb;

    pub const HIGHWAY: Rgb = [83.0, 86.0, 84.0];
    pub const MAJOR_ROAD: Rgb = [78.0, 81.0, 80.0];
    pub const MEDIUM_ROAD: Rgb = [73.0, 77.0, 75.0];
    pub const MINOR_ROAD: Rgb = [69.0, 73.0, 71.0];
    pub const PATH: Rgb = [168.0, 150.0, 119.0];
    pub const RAIL_STEEL: Rgb = [108.0, 112.0, 115.0];
    pub const PAVED: Rgb = [178.0, 174.0, 159.0];
}

pub mod ground {
    use super::Rgb;

    pub const GREEN: Rgb = [125.0, 148.0, 85.0];
    pub const FOREST: Rgb = [70.0, 99.0, 69.0];
    pub const PAVED: Rgb = super::infrastructure::PAVED;
    pub const SAND: Rgb = [197.0, 183.0, 146.0];
    pub const URBAN: Rgb = [164.0, 160.0, 145.0];
}

/// Semantic field surfaces. Each pair is a coordinated base/row or base/strip
/// treatment selected once from the stable field seed. These are
/// visual crop grammars rather than claims about real species.
pub mod field {
    use super::Rgb;

    pub const CROP: &[(Rgb, Rgb)] = &[
        ([124.0, 148.0, 83.0], [108.0, 128.0, 76.0]),
        ([168.0, 158.0, 104.0], [145.0, 144.0, 92.0]),
        ([103.0, 137.0, 97.0], [86.0, 116.0, 84.0]),
        ([139.0, 155.0, 105.0], [116.0, 136.0, 93.0]),
    ];
    pub const FLOWER: &[(Rgb, Rgb, Rgb)] = &[
        (
            [72.0, 104.0, 58.0],
            [203.0, 58.0, 65.0],
            [236.0, 176.0, 54.0],
        ),
        (
            [66.0, 101.0, 61.0],
            [151.0, 83.0, 174.0],
            [232.0, 171.0, 192.0],
        ),
        (
            [74.0, 109.0, 62.0],
            [231.0, 194.0, 62.0],
            [229.0, 233.0, 220.0],
        ),
        (
            [69.0, 103.0, 59.0],
            [216.0, 85.0, 48.0],
            [119.0, 91.0, 175.0],
        ),
    ];
    pub const ORCHARD: &[(Rgb, Rgb)] = &[
        ([91.0, 113.0, 65.0], [52.0, 86.0, 48.0]),
        ([106.0, 119.0, 70.0], [61.0, 91.0, 50.0]),
        ([82.0, 105.0, 61.0], [46.0, 77.0, 43.0]),
    ];
    pub const VINEYARD: &[(Rgb, Rgb)] = &[
        ([111.0, 116.0, 67.0], [62.0, 101.0, 50.0]),
        ([124.0, 126.0, 70.0], [72.0, 107.0, 52.0]),
        ([102.0, 111.0, 65.0], [53.0, 91.0, 49.0]),
    ];
    pub const MEADOW: &[(Rgb, Rgb)] = &[
        ([121.0, 145.0, 88.0], [138.0, 154.0, 98.0]),
        ([102.0, 135.0, 98.0], [121.0, 150.0, 111.0]),
        ([139.0, 148.0, 100.0], [153.0, 157.0, 110.0]),
    ];
    pub const DORMANT: &[(Rgb, Rgb)] = &[
        ([139.0, 124.0, 82.0], [110.0, 99.0, 69.0]),
        ([151.0, 134.0, 88.0], [122.0, 109.0, 72.0]),
        ([128.0, 116.0, 79.0], [99.0, 91.0, 65.0]),
    ];
    pub const HARVESTED: &[(Rgb, Rgb)] = &[
        ([156.0, 134.0, 85.0], [91.0, 75.0, 53.0]),
        ([169.0, 147.0, 94.0], [104.0, 84.0, 57.0]),
        ([143.0, 124.0, 82.0], [82.0, 70.0, 52.0]),
    ];
}

pub mod water {
    use super::Rgb;

    // Shallow anchors represent the returning seabed light before absorption;
    // MID anchors are the optically thick water-column response, all in sRGB.
    pub const SHALLOW: Rgb = [111.0, 174.0, 177.0];
    pub const LAGOON: Rgb = [208.0, 226.0, 204.0];
    pub const MID: Rgb = [8.0, 38.0, 58.0];
    pub const TROPICAL_MID: Rgb = [6.0, 43.0, 65.0];
    pub const DEEP: Rgb = [3.0, 12.0, 25.0];
    pub const GLACIAL: Rgb = [134.0, 197.0, 185.0];
    pub const GLACIAL_MID: Rgb = [53.0, 137.0, 144.0];
    pub const MINERAL: Rgb = [99.0, 135.0, 119.0];
    pub const SEDIMENT: Rgb = [157.0, 157.0, 112.0];
    pub const SEDIMENT_MID: Rgb = [84.0, 103.0, 74.0];
    pub const TANNIN: Rgb = [88.0, 96.0, 74.0];
    pub const TURBID: Rgb = [116.0, 119.0, 94.0];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_colours_stay_inside_a_restrained_display_range() {
        let mut colours = terrain::AUTHORED_SURFACE_ANCHORS.to_vec();
        colours.extend([
            infrastructure::HIGHWAY,
            infrastructure::PATH,
            ground::GREEN,
            ground::FOREST,
            field::CROP[0].0,
            field::CROP[0].1,
            field::FLOWER[0].0,
            field::FLOWER[0].1,
            field::FLOWER[0].2,
            field::ORCHARD[0].0,
            field::ORCHARD[0].1,
            field::VINEYARD[0].0,
            field::VINEYARD[0].1,
            field::MEADOW[0].0,
            field::MEADOW[0].1,
            field::DORMANT[0].0,
            field::DORMANT[0].1,
            field::HARVESTED[0].0,
            field::HARVESTED[0].1,
            water::SHALLOW,
            water::LAGOON,
            water::MID,
            water::TROPICAL_MID,
            water::DEEP,
            water::GLACIAL,
            water::GLACIAL_MID,
            water::MINERAL,
            water::SEDIMENT,
            water::SEDIMENT_MID,
            water::TANNIN,
            water::TURBID,
        ]);
        for colour in colours {
            assert!(
                colour
                    .into_iter()
                    .all(|channel| (0.0..=245.0).contains(&channel))
            );
        }
    }

    #[test]
    fn procedural_surface_vocabulary_is_genuinely_broad() {
        assert!(terrain::AUTHORED_SURFACE_ANCHORS.len() >= 60);
    }

    #[test]
    fn vegetation_families_have_distinct_hues_and_landscape_value_groups() {
        let value = |rgb: Rgb| rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        assert!(value(ground::GREEN) > value(ground::FOREST) + 35.0);
        assert!(value(terrain::PALE_GRANITE) > value(ground::GREEN) + 35.0);
        assert!(value(ground::GREEN) > value(infrastructure::HIGHWAY) + 35.0);
        let cool = terrain::BOREAL_GREEN;
        let warm = terrain::TEMPERATE_GREEN;
        assert!(warm[0] - warm[2] > 30.0);
        assert!(cool[2] - cool[0] > 10.0);
        assert!(terrain::RAINFOREST[1] > terrain::RAINFOREST[0] + 30.0);
        assert!(terrain::SAVANNA[0] > terrain::SAVANNA[1]);
    }

    #[test]
    fn rail_steel_stays_a_restrained_dark_gray() {
        let luminance = |color: Rgb| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
        let steel = luminance(infrastructure::RAIL_STEEL);
        assert!(
            (100.0..120.0).contains(&steel),
            "rail steel should stay slightly dark: {steel}"
        );
        let channels = infrastructure::RAIL_STEEL;
        let spread = channels.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - channels.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            spread < 10.0,
            "rail steel must remain neutral gray: {channels:?}"
        );
    }

    #[test]
    fn water_depth_and_chemistry_keep_readable_distinctions() {
        let value = |rgb: Rgb| rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        assert!(value(water::SHALLOW) > value(water::MID) + 25.0);
        assert!(value(water::MID) > value(water::DEEP) * 2.5);
        assert!(water::SHALLOW[2] > water::SHALLOW[1]);
        assert!(water::MID[2] > water::MID[1]);
        assert!(water::DEEP[2] > water::DEEP[1]);
        assert!(water::GLACIAL[1] > water::GLACIAL[0]);
        assert!(water::SEDIMENT[0] > water::SHALLOW[0] + 20.0);
        assert!(water::LAGOON[1] > water::LAGOON[2]);
        assert!(water::TROPICAL_MID[2] > water::TROPICAL_MID[1]);
    }
}
