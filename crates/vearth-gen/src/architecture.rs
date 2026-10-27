//! Optional architectural design studies for the model catalogue.
//!
//! These authored profiles drive the staging scene only. Production buildings
//! use source footprints and `regional_architecture` construction choices; these
//! catalogue examples do not select styles for geographic locations.

use vearth_data::vectors::RoofShape;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ArchitectureRegion {
    CongoBasin,
    Nordic,
    Sydney,
    Salvador,
    Santorini,
    AtlanticEurope,
    ContinentalEurope,
    Alpine,
    NorthAmerica,
    Mexico,
    Andes,
    Sahel,
    SwahiliCoast,
    Maghreb,
    Arabia,
    SouthAsia,
    SoutheastAsia,
    Japan,
    Korea,
    Pacific,
    CentralAsia,
    China,
    SouthernAfrica,
    Caribbean,
    BritishIsles,
    Iberia,
    Italy,
    Baltic,
    WestAfrica,
    EthiopianHighlands,
    USSouthwest,
    NewEngland,
    SouthernCone,
    Indonesia,
    Himalaya,
    AtlanticBrazil,
}

pub const ARCHITECTURE_REGION_COUNT: usize = 36;

/// Authored grammar shared by the short-house and medium-building stage studies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildingGrammar {
    TropicalShade,
    NordicTimber,
    VerandaBungalow,
    ColonialRow,
    Cycladic,
    AtlanticRow,
    Continental,
    Chalet,
    PorchHouse,
    Courtyard,
    Hillside,
    EarthenSahel,
    Swahili,
    Riad,
    ArabianTower,
    IndianPol,
    TropicalStilt,
    Machiya,
    Hanok,
    PacificFale,
    BritishTerrace,
    IberianHouse,
    ItalianPortico,
    BalticTown,
    AsanteCourtyard,
    HarariHouse,
    Pueblo,
    CapeCod,
    Colonia,
    IndonesianRoof,
    NewariHouse,
    AzulejoRow,
}

#[derive(Clone, Copy, Debug)]
pub struct ArchitectureProfile {
    pub id: ArchitectureRegion,
    pub slug: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub grammar: BuildingGrammar,
    pub walls: [[u8; 3]; 4],
    pub accents: [[u8; 3]; 3],
    pub roofs: [[u8; 3]; 3],
    pub short_roofs: [RoofShape; 2],
    pub medium_roofs: [RoofShape; 2],
    pub short_storeys: u8,
    pub medium_storeys: u8,
}

const fn profile(
    id: ArchitectureRegion,
    slug: &'static str,
    label: &'static str,
    group: &'static str,
    grammar: BuildingGrammar,
    walls: [[u8; 3]; 4],
    accents: [[u8; 3]; 3],
    roofs: [[u8; 3]; 3],
    short_roofs: [RoofShape; 2],
    medium_roofs: [RoofShape; 2],
    short_storeys: u8,
    medium_storeys: u8,
) -> ArchitectureProfile {
    ArchitectureProfile {
        id,
        slug,
        label,
        group,
        grammar,
        walls,
        accents,
        roofs,
        short_roofs,
        medium_roofs,
        short_storeys,
        medium_storeys,
    }
}

use ArchitectureRegion::*;
use BuildingGrammar::*;
use RoofShape::*;

/// Thirty-six regional design studies in their catalogue display order.
pub const ARCHITECTURE_PROFILES: [ArchitectureProfile; ARCHITECTURE_REGION_COUNT] = [
    profile(
        CongoBasin,
        "congo-basin",
        "Congo Basin",
        "Africa",
        TropicalShade,
        [
            [151, 104, 67],
            [181, 139, 91],
            [116, 90, 67],
            [199, 177, 132],
        ],
        [[54, 77, 57], [225, 205, 143], [82, 54, 39]],
        [[116, 91, 52], [142, 116, 68], [88, 91, 84]],
        [Hipped, Pyramidal],
        [Skillion, Flat],
        1,
        4,
    ),
    profile(
        Nordic,
        "nordic-village",
        "Nordic Village",
        "Europe",
        NordicTimber,
        [
            [139, 55, 45],
            [211, 201, 177],
            [104, 127, 129],
            [177, 145, 79],
        ],
        [[239, 235, 214], [56, 73, 74], [104, 36, 31]],
        [[62, 70, 77], [95, 99, 101], [128, 76, 51]],
        [Gabled, Gambrel],
        [Gabled, Mansard],
        2,
        4,
    ),
    profile(
        Sydney,
        "sydney",
        "Sydney",
        "Oceania",
        VerandaBungalow,
        [
            [130, 76, 55],
            [205, 193, 172],
            [177, 164, 137],
            [151, 91, 65],
        ],
        [[225, 219, 197], [75, 96, 91], [75, 62, 55]],
        [[154, 67, 48], [177, 83, 58], [90, 91, 91]],
        [Hipped, Gabled],
        [Flat, Hipped],
        1,
        4,
    ),
    profile(
        Salvador,
        "salvador-da-bahia",
        "Salvador da Bahia",
        "South America",
        ColonialRow,
        [
            [202, 113, 98],
            [221, 177, 92],
            [119, 171, 166],
            [177, 135, 170],
        ],
        [[238, 229, 208], [80, 117, 139], [91, 60, 50]],
        [[150, 76, 58], [171, 91, 65], [103, 94, 86]],
        [Hipped, Gabled],
        [Hipped, Flat],
        2,
        4,
    ),
    profile(
        Santorini,
        "santorini",
        "Santorini",
        "Europe",
        Cycladic,
        [
            [238, 237, 225],
            [225, 229, 222],
            [244, 241, 225],
            [211, 218, 215],
        ],
        [[49, 100, 148], [137, 72, 61], [79, 127, 150]],
        [[229, 232, 224], [53, 101, 151], [157, 88, 72]],
        [Flat, Dome],
        [Flat, Dome],
        1,
        3,
    ),
    profile(
        AtlanticEurope,
        "low-countries",
        "Low Countries",
        "Europe",
        AtlanticRow,
        [
            [146, 79, 61],
            [201, 188, 163],
            [173, 118, 89],
            [217, 211, 197],
        ],
        [[225, 221, 206], [62, 72, 77], [94, 62, 47]],
        [[70, 75, 82], [96, 84, 76], [145, 72, 51]],
        [Gabled, Hipped],
        [Mansard, Gabled],
        2,
        4,
    ),
    profile(
        ContinentalEurope,
        "continental-europe",
        "Continental Europe",
        "Europe",
        Continental,
        [
            [211, 177, 120],
            [172, 183, 159],
            [157, 178, 188],
            [193, 146, 140],
        ],
        [[231, 224, 205], [78, 91, 91], [102, 70, 55]],
        [[96, 82, 76], [132, 76, 58], [75, 80, 85]],
        [Hipped, Mansard],
        [Mansard, Hipped],
        2,
        5,
    ),
    profile(
        Alpine,
        "alpine",
        "Alpine",
        "Europe",
        Chalet,
        [
            [203, 189, 160],
            [139, 103, 72],
            [220, 216, 198],
            [164, 130, 94],
        ],
        [[84, 58, 40], [225, 219, 198], [73, 103, 83]],
        [[111, 78, 54], [87, 88, 84], [132, 68, 48]],
        [Gabled, Gabled],
        [Gabled, Hipped],
        2,
        4,
    ),
    profile(
        NorthAmerica,
        "north-america",
        "North America",
        "North America",
        PorchHouse,
        [
            [214, 210, 194],
            [155, 174, 178],
            [186, 166, 135],
            [174, 75, 57],
        ],
        [[239, 236, 220], [57, 83, 97], [102, 67, 48]],
        [[74, 78, 83], [112, 105, 96], [137, 71, 51]],
        [Gabled, Gambrel],
        [Flat, Mansard],
        2,
        5,
    ),
    profile(
        Mexico,
        "mexico-central-america",
        "Mexico & Central America",
        "North America",
        Courtyard,
        [
            [210, 95, 65],
            [226, 165, 70],
            [92, 156, 143],
            [217, 205, 169],
        ],
        [[238, 226, 192], [74, 108, 136], [99, 52, 41]],
        [[164, 75, 49], [185, 94, 58], [211, 199, 172]],
        [Flat, Hipped],
        [Flat, Hipped],
        1,
        3,
    ),
    profile(
        Andes,
        "andean-pacific",
        "Andean Pacific",
        "South America",
        Hillside,
        [
            [181, 105, 79],
            [112, 148, 157],
            [202, 166, 91],
            [169, 121, 140],
        ],
        [[224, 215, 190], [59, 79, 86], [111, 61, 44]],
        [[102, 95, 89], [126, 78, 59], [76, 83, 87]],
        [Skillion, Gabled],
        [Flat, Skillion],
        2,
        4,
    ),
    profile(
        Sahel,
        "sahel",
        "Sahel",
        "Africa",
        EarthenSahel,
        [
            [173, 126, 79],
            [191, 145, 92],
            [151, 103, 67],
            [210, 173, 116],
        ],
        [[72, 57, 43], [220, 204, 161], [102, 70, 48]],
        [[158, 116, 73], [181, 139, 89], [118, 91, 65]],
        [Flat, Flat],
        [Flat, Flat],
        1,
        3,
    ),
    profile(
        SwahiliCoast,
        "swahili-coast",
        "Swahili Coast",
        "Africa",
        Swahili,
        [
            [218, 211, 184],
            [194, 184, 151],
            [229, 224, 202],
            [174, 163, 135],
        ],
        [[82, 57, 42], [66, 111, 117], [228, 217, 180]],
        [[202, 194, 166], [119, 103, 82], [93, 91, 82]],
        [Flat, Hipped],
        [Flat, Flat],
        1,
        3,
    ),
    profile(
        Maghreb,
        "maghreb",
        "Maghreb",
        "Africa",
        Riad,
        [
            [210, 191, 153],
            [189, 154, 111],
            [224, 216, 191],
            [172, 128, 90],
        ],
        [[65, 118, 111], [92, 61, 44], [235, 226, 198]],
        [[198, 185, 153], [151, 119, 84], [105, 89, 70]],
        [Flat, Flat],
        [Flat, Dome],
        1,
        3,
    ),
    profile(
        Arabia,
        "arabian-peninsula",
        "Arabian Peninsula",
        "Asia",
        ArabianTower,
        [
            [181, 134, 87],
            [203, 160, 105],
            [154, 111, 74],
            [218, 187, 140],
        ],
        [[238, 228, 195], [87, 60, 43], [104, 126, 119]],
        [[180, 146, 103], [135, 105, 76], [216, 190, 149]],
        [Flat, Flat],
        [Flat, Flat],
        2,
        6,
    ),
    profile(
        SouthAsia,
        "south-asia",
        "South Asia",
        "Asia",
        IndianPol,
        [
            [180, 111, 70],
            [218, 176, 107],
            [160, 132, 93],
            [206, 146, 94],
        ],
        [[82, 55, 38], [224, 212, 171], [64, 112, 121]],
        [[151, 94, 58], [113, 92, 71], [203, 168, 111]],
        [Flat, Hipped],
        [Flat, Hipped],
        2,
        4,
    ),
    profile(
        SoutheastAsia,
        "southeast-asia",
        "Southeast Asia",
        "Asia",
        TropicalStilt,
        [
            [134, 90, 57],
            [177, 129, 75],
            [202, 179, 125],
            [106, 83, 62],
        ],
        [[225, 203, 139], [69, 92, 72], [82, 52, 37]],
        [[113, 67, 43], [151, 92, 53], [92, 91, 79]],
        [Gabled, Hipped],
        [Hipped, Gabled],
        1,
        3,
    ),
    profile(
        Japan,
        "japan",
        "Japan",
        "Asia",
        Machiya,
        [
            [180, 167, 141],
            [119, 93, 68],
            [217, 211, 191],
            [96, 79, 63],
        ],
        [[57, 48, 42], [226, 218, 195], [117, 50, 42]],
        [[74, 76, 76], [103, 94, 82], [126, 92, 62]],
        [Gabled, Hipped],
        [Hipped, Gabled],
        2,
        4,
    ),
    profile(
        Korea,
        "korea",
        "Korea",
        "Asia",
        Hanok,
        [
            [222, 215, 193],
            [172, 155, 124],
            [202, 192, 167],
            [139, 116, 84],
        ],
        [[67, 58, 49], [230, 225, 207], [101, 65, 47]],
        [[74, 77, 79], [96, 96, 91], [128, 98, 66]],
        [Hipped, Gabled],
        [Hipped, Gabled],
        1,
        3,
    ),
    profile(
        Pacific,
        "pacific-islands",
        "Pacific Islands",
        "Oceania",
        PacificFale,
        [
            [166, 126, 78],
            [205, 177, 119],
            [127, 101, 73],
            [221, 204, 157],
        ],
        [[72, 91, 69], [234, 218, 165], [89, 61, 43]],
        [[137, 111, 66], [165, 137, 81], [91, 88, 74]],
        [Hipped, Pyramidal],
        [Hipped, Gabled],
        1,
        3,
    ),
    profile(
        CentralAsia,
        "central-asia",
        "Central Asia",
        "Asia",
        Courtyard,
        [
            [194, 158, 105],
            [218, 193, 145],
            [169, 128, 83],
            [227, 211, 173],
        ],
        [[48, 130, 145], [83, 70, 52], [227, 218, 186]],
        [[173, 139, 96], [122, 101, 78], [207, 187, 145]],
        [Flat, Hipped],
        [Flat, Dome],
        1,
        3,
    ),
    profile(
        China,
        "china",
        "China",
        "Asia",
        Hanok,
        [
            [204, 199, 184],
            [151, 139, 119],
            [188, 177, 157],
            [126, 113, 94],
        ],
        [[139, 46, 37], [226, 207, 155], [55, 52, 49]],
        [[63, 67, 69], [84, 82, 76], [116, 103, 82]],
        [Hipped, Gabled],
        [Hipped, Gabled],
        1,
        4,
    ),
    profile(
        SouthernAfrica,
        "southern-africa",
        "Southern Africa",
        "Africa",
        PorchHouse,
        [
            [232, 225, 205],
            [205, 186, 151],
            [181, 157, 119],
            [218, 211, 190],
        ],
        [[73, 93, 72], [119, 71, 48], [67, 62, 56]],
        [[75, 77, 76], [121, 91, 67], [160, 129, 83]],
        [Gabled, Hipped],
        [Hipped, Gabled],
        1,
        3,
    ),
    profile(
        Caribbean,
        "caribbean",
        "Caribbean",
        "North America",
        ColonialRow,
        [
            [110, 164, 171],
            [218, 160, 93],
            [204, 125, 130],
            [208, 196, 145],
        ],
        [[241, 233, 204], [60, 118, 124], [80, 61, 49]],
        [[121, 78, 56], [155, 91, 61], [90, 91, 88]],
        [Hipped, Gabled],
        [Hipped, Flat],
        1,
        4,
    ),
    profile(
        BritishIsles,
        "british-isles",
        "British Isles",
        "Europe",
        BritishTerrace,
        [
            [151, 84, 64],
            [207, 201, 184],
            [181, 157, 119],
            [196, 180, 161],
        ],
        [[229, 226, 211], [67, 83, 72], [76, 61, 53]],
        [[72, 76, 82], [99, 91, 84], [134, 76, 57]],
        [Gabled, Hipped],
        [Gabled, Mansard],
        2,
        4,
    ),
    profile(
        Iberia,
        "iberia",
        "Iberia",
        "Europe",
        IberianHouse,
        [
            [229, 225, 208],
            [210, 168, 103],
            [151, 177, 183],
            [194, 145, 130],
        ],
        [[77, 111, 133], [91, 67, 54], [238, 231, 211]],
        [[157, 80, 58], [178, 98, 68], [113, 94, 81]],
        [Hipped, Flat],
        [Hipped, Flat],
        1,
        4,
    ),
    profile(
        Italy,
        "italy",
        "Italy",
        "Europe",
        ItalianPortico,
        [
            [209, 194, 160],
            [195, 151, 118],
            [224, 214, 191],
            [184, 145, 133],
        ],
        [[75, 91, 68], [223, 215, 192], [88, 61, 48]],
        [[151, 95, 73], [174, 116, 88], [115, 87, 74]],
        [Hipped, Gabled],
        [Hipped, Gabled],
        2,
        4,
    ),
    profile(
        Baltic,
        "baltic",
        "Baltic",
        "Europe",
        BalticTown,
        [
            [139, 154, 132],
            [208, 187, 126],
            [139, 166, 178],
            [158, 92, 70],
        ],
        [[228, 224, 206], [74, 87, 87], [79, 57, 48]],
        [[70, 75, 81], [101, 93, 82], [134, 74, 56]],
        [Gabled, Hipped],
        [Gabled, Mansard],
        2,
        4,
    ),
    profile(
        WestAfrica,
        "west-africa",
        "West Africa",
        "Africa",
        AsanteCourtyard,
        [
            [161, 105, 67],
            [188, 143, 86],
            [133, 88, 59],
            [204, 174, 119],
        ],
        [[75, 57, 42], [210, 171, 72], [224, 211, 173]],
        [[126, 99, 57], [151, 124, 72], [91, 88, 78]],
        [Hipped, Gabled],
        [Hipped, Gabled],
        1,
        3,
    ),
    profile(
        EthiopianHighlands,
        "ethiopian-highlands",
        "Ethiopian Highlands",
        "Africa",
        HarariHouse,
        [
            [211, 194, 158],
            [182, 145, 99],
            [194, 151, 125],
            [223, 215, 191],
        ],
        [[86, 112, 86], [99, 67, 49], [231, 220, 190]],
        [[177, 151, 111], [128, 104, 79], [205, 188, 149]],
        [Flat, Hipped],
        [Flat, Hipped],
        1,
        3,
    ),
    profile(
        USSouthwest,
        "us-southwest",
        "US Southwest",
        "North America",
        Pueblo,
        [
            [190, 145, 101],
            [202, 164, 126],
            [215, 192, 153],
            [176, 126, 91],
        ],
        [[75, 122, 125], [101, 71, 52], [229, 215, 181]],
        [[178, 146, 107], [145, 112, 83], [205, 178, 136]],
        [Flat, Flat],
        [Flat, Flat],
        1,
        3,
    ),
    profile(
        NewEngland,
        "new-england",
        "New England",
        "North America",
        CapeCod,
        [
            [221, 219, 207],
            [143, 161, 169],
            [210, 195, 146],
            [169, 164, 151],
        ],
        [[237, 234, 220], [68, 88, 75], [76, 64, 56]],
        [[70, 75, 81], [93, 94, 92], [119, 105, 88]],
        [Gabled, Gambrel],
        [Gabled, Mansard],
        1,
        4,
    ),
    profile(
        SouthernCone,
        "southern-cone",
        "Southern Cone",
        "South America",
        Colonia,
        [
            [222, 215, 194],
            [199, 159, 104],
            [143, 169, 177],
            [186, 136, 125],
        ],
        [[225, 221, 204], [74, 96, 91], [90, 62, 49]],
        [[156, 81, 58], [174, 98, 68], [101, 92, 83]],
        [Hipped, Gabled],
        [Hipped, Gabled],
        1,
        3,
    ),
    profile(
        Indonesia,
        "indonesia",
        "Indonesia",
        "Asia",
        IndonesianRoof,
        [
            [119, 83, 58],
            [157, 111, 69],
            [190, 156, 101],
            [102, 78, 61],
        ],
        [[220, 197, 137], [101, 48, 39], [66, 58, 49]],
        [[91, 71, 48], [123, 94, 55], [72, 72, 66]],
        [Gabled, Gambrel],
        [Gabled, Hipped],
        1,
        3,
    ),
    profile(
        Himalaya,
        "himalaya",
        "Himalaya",
        "Asia",
        NewariHouse,
        [
            [153, 83, 62],
            [188, 143, 102],
            [111, 87, 68],
            [207, 190, 157],
        ],
        [[74, 56, 46], [218, 204, 171], [97, 72, 56]],
        [[131, 70, 52], [88, 84, 79], [117, 91, 68]],
        [Gabled, Hipped],
        [Hipped, Gabled],
        2,
        4,
    ),
    profile(
        AtlanticBrazil,
        "atlantic-brazil",
        "Atlantic Brazil",
        "South America",
        AzulejoRow,
        [
            [134, 165, 177],
            [218, 214, 197],
            [199, 167, 102],
            [147, 169, 143],
        ],
        [[234, 228, 211], [67, 102, 127], [85, 62, 51]],
        [[152, 78, 58], [174, 95, 65], [102, 94, 85]],
        [Hipped, Gabled],
        [Hipped, Flat],
        2,
        4,
    ),
];

pub const fn architecture_profile(id: ArchitectureRegion) -> &'static ArchitectureProfile {
    &ARCHITECTURE_PROFILES[id as usize]
}

/// Choose one of the four authored wall colourways for a building. Wall and
/// roof selection use this same index so variation remains coherent rather
/// than combining unrelated colours independently.
pub fn architecture_colorway(seed: u64) -> usize {
    let mut z = seed ^ 0x43_4f_4c_4f_55_52_57_41;
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 62) as usize
}

/// Calibrate authored reference colours for the stage lighting model. A small
/// luma blend keeps saturated paint readable beside earth, brick and timber
/// without turning the world into a row of toy-coloured blocks.
pub fn architecture_tint(rgb: [u8; 3]) -> [u8; 3] {
    let neutral = (u16::from(rgb[0]) * 3 + u16::from(rgb[1]) * 6 + u16::from(rgb[2])) / 10;
    rgb.map(|channel| ((u16::from(channel) * 4 + neutral) / 5) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_order_matches_enum_and_slugs_are_unique() {
        let mut slugs = std::collections::HashSet::new();
        assert_eq!(ARCHITECTURE_PROFILES.len(), ARCHITECTURE_REGION_COUNT);
        for (index, profile) in ARCHITECTURE_PROFILES.iter().enumerate() {
            assert_eq!(profile.id as usize, index);
            assert!(slugs.insert(profile.slug));
            assert!(!profile.label.is_empty());
        }
    }

    #[test]
    fn every_profile_has_two_distinct_catalogue_scales() {
        for profile in ARCHITECTURE_PROFILES {
            assert!(profile.short_storeys <= 2, "{} is not short", profile.slug);
            assert!(
                profile.medium_storeys >= 3,
                "{} is not medium",
                profile.slug
            );
        }
    }

    #[test]
    fn building_colourways_are_varied_and_deterministic() {
        let observed = (0..128)
            .map(architecture_colorway)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(observed, std::collections::HashSet::from([0, 1, 2, 3]));
        assert_eq!(architecture_colorway(42), architecture_colorway(42));
        for profile in ARCHITECTURE_PROFILES {
            assert!(
                profile
                    .walls
                    .into_iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    >= 3,
                "{} lacks meaningful wall-colour variation",
                profile.slug
            );
        }
        let vivid = [220, 80, 50];
        let toned = architecture_tint(vivid);
        let chroma = |rgb: [u8; 3]| rgb.into_iter().max().unwrap() - rgb.into_iter().min().unwrap();
        assert!(chroma(toned) < chroma(vivid));
    }
}
