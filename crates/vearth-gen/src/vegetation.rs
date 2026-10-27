//! Provider-free regional vegetation profiles.
//!
//! The renderer deliberately does not fetch a species or canopy service at
//! runtime. Compiled woodland communities map geographic and climate contexts
//! to compatible canopy forms. A continuous habitat field gives large forests
//! a shared identity. Profiles describe morphology, not a
//! claim that an inferred tree is the named biological species.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TreeProfileId {
    EnglishOak,
    EuropeanBeech,
    SugarMaple,
    SilverBirch,
    Aspen,
    WeepingWillow,
    LondonPlane,
    Elm,
    ColumnarPoplar,
    HorseChestnut,
    NorwaySpruce,
    SilverFir,
    ScotsPine,
    StonePine,
    EuropeanLarch,
    Cedar,
    Redwood,
    ItalianCypress,
    Banyan,
    Kapok,
    RainforestEmergent,
    TropicalBroadleaf,
    RainforestUnderstory,
    TropicalDry,
    Mahogany,
    TropicalFig,
    UmbrellaAcacia,
    SavannaThorn,
    Olive,
    HolmOak,
    CorkOak,
    Mesquite,
    Baobab,
    RiverRedGum,
    OpenEucalyptus,
    Paperbark,
    Pohutukawa,
    NorfolkIslandPine,
    CoconutPalm,
    DatePalm,
    FanPalm,
    RoyalPalm,
    RedMangrove,
    Orchard,
    FloweringOrnamental,
    StreetColumnar,
    PlantationConifer,
    ParkRound,
}

pub const TREE_PROFILE_COUNT: usize = 48;

/// Runtime seasonal response retained in the voxel material id.  Numeric
/// values are part of the compact foliage-material mapping; append rather
/// than reorder once released.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FoliageSeasonClass {
    EvergreenNeedle,
    EvergreenBroadleaf,
    DeciduousYellow,
    DeciduousOrange,
    DeciduousRed,
    DeciduousConifer,
    DroughtDeciduous,
    SpringFlowering,
    OrchardFlowering,
    TropicalBroadleaf,
    Palm,
}

pub const FOLIAGE_SEASON_CLASS_COUNT: u8 = 11;

/// Wind response for shading-only v1 foliage motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FoliageMotionClass {
    Stiff,
    Broadleaf,
    Flexible,
    Frond,
}

pub const FOLIAGE_MOTION_CLASS_COUNT: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FoliageTraits {
    pub season: FoliageSeasonClass,
    pub motion: FoliageMotionClass,
}

impl TreeProfileId {
    /// Exhaustive by construction: adding a profile fails compilation until
    /// its seasonal and wind behavior is selected deliberately.
    pub const fn foliage_traits(self) -> FoliageTraits {
        use FoliageMotionClass as Motion;
        use FoliageSeasonClass as Season;
        use TreeProfileId::*;

        let (season, motion) = match self {
            EnglishOak => (Season::DeciduousOrange, Motion::Broadleaf),
            EuropeanBeech => (Season::DeciduousYellow, Motion::Broadleaf),
            SugarMaple => (Season::DeciduousRed, Motion::Broadleaf),
            SilverBirch | Aspen => (Season::DeciduousYellow, Motion::Flexible),
            WeepingWillow => (Season::DeciduousYellow, Motion::Flexible),
            LondonPlane => (Season::DeciduousOrange, Motion::Broadleaf),
            Elm => (Season::DeciduousYellow, Motion::Broadleaf),
            ColumnarPoplar => (Season::DeciduousYellow, Motion::Flexible),
            HorseChestnut => (Season::DeciduousOrange, Motion::Broadleaf),
            NorwaySpruce | SilverFir | ScotsPine | StonePine | Cedar | Redwood | ItalianCypress
            | NorfolkIslandPine | PlantationConifer => (Season::EvergreenNeedle, Motion::Stiff),
            EuropeanLarch => (Season::DeciduousConifer, Motion::Flexible),
            Banyan | Mahogany => (Season::TropicalBroadleaf, Motion::Broadleaf),
            Kapok | RainforestUnderstory | TropicalFig | RedMangrove => {
                (Season::TropicalBroadleaf, Motion::Flexible)
            }
            RainforestEmergent | TropicalBroadleaf => {
                (Season::TropicalBroadleaf, Motion::Broadleaf)
            }
            TropicalDry => (Season::DroughtDeciduous, Motion::Flexible),
            UmbrellaAcacia | SavannaThorn | Mesquite | Baobab => {
                (Season::DroughtDeciduous, Motion::Stiff)
            }
            Olive | HolmOak | CorkOak => (Season::EvergreenBroadleaf, Motion::Stiff),
            RiverRedGum | OpenEucalyptus | Paperbark => {
                (Season::EvergreenBroadleaf, Motion::Flexible)
            }
            Pohutukawa => (Season::SpringFlowering, Motion::Broadleaf),
            CoconutPalm | DatePalm | FanPalm | RoyalPalm => (Season::Palm, Motion::Frond),
            Orchard => (Season::OrchardFlowering, Motion::Broadleaf),
            FloweringOrnamental => (Season::SpringFlowering, Motion::Flexible),
            StreetColumnar => (Season::DeciduousYellow, Motion::Stiff),
            ParkRound => (Season::DeciduousOrange, Motion::Broadleaf),
        };
        FoliageTraits { season, motion }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CrownGrammar {
    Round,
    Spreading,
    Columnar,
    Weeping,
    Cone,
    Pine,
    Umbrella,
    Open,
    Emergent,
    Palm,
    Baobab,
    Rooted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootStyle {
    None,
    Banyan,
    Mangrove,
}

#[derive(Clone, Copy, Debug)]
pub struct TreeProfile {
    pub id: TreeProfileId,
    pub name: &'static str,
    pub grammar: CrownGrammar,
    pub height_m: [f32; 2],
    /// Crown radius as a fraction of total height.
    pub radius_ratio: [f32; 2],
    /// Lowest ordinary foliage as a fraction of total height.
    pub crown_base: f32,
    /// Trunk radius as a fraction of total height, clamped by the stamper.
    pub trunk_ratio: f32,
    /// Probability-like hint used to open gaps in the crown.
    pub openness: f32,
    pub bark: [u8; 3],
    pub leaves: [u8; 3],
    /// Persistent rendering behavior encoded into stamped crown materials.
    pub foliage: FoliageTraits,
    pub root_style: RootStyle,
    /// Grammar-specific variant used by feather/fan palms and conifer tiers.
    pub variant: u8,
}

#[allow(clippy::too_many_arguments)]
const fn profile(
    id: TreeProfileId,
    name: &'static str,
    grammar: CrownGrammar,
    height_m: [f32; 2],
    radius_ratio: [f32; 2],
    crown_base: f32,
    trunk_ratio: f32,
    openness: f32,
    bark: [u8; 3],
    leaves: [u8; 3],
    root_style: Option<RootStyle>,
    variant: u8,
) -> TreeProfile {
    TreeProfile {
        id,
        name,
        grammar,
        height_m,
        radius_ratio,
        crown_base,
        trunk_ratio,
        openness,
        bark,
        leaves,
        foliage: id.foliage_traits(),
        root_style: match root_style {
            Some(style) => style,
            None => RootStyle::None,
        },
        variant,
    }
}

use CrownGrammar::*;
use TreeProfileId::*;

/// Forty-eight compact visual profiles.  They are data rows over twelve
/// procedural grammars, not meshes or separately loaded assets.
pub const TREE_PROFILES: [TreeProfile; TREE_PROFILE_COUNT] = [
    profile(
        EnglishOak,
        "english-oak",
        Spreading,
        [12.0, 24.0],
        [0.34, 0.46],
        0.30,
        0.055,
        0.12,
        [91, 67, 44],
        [67, 103, 43],
        None,
        0,
    ),
    profile(
        EuropeanBeech,
        "european-beech",
        Round,
        [14.0, 26.0],
        [0.28, 0.38],
        0.36,
        0.042,
        0.08,
        [104, 83, 65],
        [66, 111, 48],
        None,
        0,
    ),
    profile(
        SugarMaple,
        "sugar-maple",
        Round,
        [12.0, 23.0],
        [0.30, 0.40],
        0.38,
        0.044,
        0.10,
        [91, 73, 56],
        [72, 120, 50],
        None,
        1,
    ),
    profile(
        SilverBirch,
        "silver-birch",
        Round,
        [11.0, 22.0],
        [0.18, 0.26],
        0.38,
        0.025,
        0.28,
        [205, 205, 190],
        [91, 132, 57],
        None,
        2,
    ),
    profile(
        Aspen,
        "aspen",
        Round,
        [12.0, 24.0],
        [0.18, 0.27],
        0.40,
        0.026,
        0.22,
        [176, 180, 157],
        [92, 139, 59],
        None,
        3,
    ),
    profile(
        WeepingWillow,
        "weeping-willow",
        Weeping,
        [10.0, 20.0],
        [0.35, 0.48],
        0.30,
        0.050,
        0.10,
        [91, 72, 49],
        [103, 138, 53],
        None,
        0,
    ),
    profile(
        LondonPlane,
        "london-plane",
        Spreading,
        [14.0, 28.0],
        [0.32, 0.43],
        0.38,
        0.050,
        0.15,
        [131, 119, 91],
        [72, 113, 51],
        None,
        1,
    ),
    profile(
        Elm,
        "elm",
        Spreading,
        [13.0, 25.0],
        [0.30, 0.42],
        0.35,
        0.045,
        0.14,
        [92, 72, 53],
        [62, 108, 45],
        None,
        2,
    ),
    profile(
        ColumnarPoplar,
        "columnar-poplar",
        Columnar,
        [14.0, 29.0],
        [0.10, 0.17],
        0.22,
        0.026,
        0.08,
        [108, 91, 69],
        [72, 121, 49],
        None,
        0,
    ),
    profile(
        HorseChestnut,
        "horse-chestnut",
        Round,
        [12.0, 23.0],
        [0.30, 0.40],
        0.34,
        0.048,
        0.08,
        [91, 69, 49],
        [61, 105, 42],
        None,
        4,
    ),
    profile(
        NorwaySpruce,
        "norway-spruce",
        Cone,
        [13.0, 30.0],
        [0.20, 0.28],
        0.12,
        0.033,
        0.08,
        [92, 68, 48],
        [43, 73, 43],
        None,
        0,
    ),
    profile(
        SilverFir,
        "silver-fir",
        Cone,
        [14.0, 31.0],
        [0.19, 0.26],
        0.16,
        0.034,
        0.10,
        [91, 79, 65],
        [49, 84, 52],
        None,
        1,
    ),
    profile(
        ScotsPine,
        "scots-pine",
        Pine,
        [14.0, 29.0],
        [0.22, 0.32],
        0.52,
        0.034,
        0.30,
        [151, 84, 47],
        [50, 84, 46],
        None,
        0,
    ),
    profile(
        StonePine,
        "stone-pine",
        Umbrella,
        [10.0, 22.0],
        [0.30, 0.43],
        0.58,
        0.038,
        0.18,
        [105, 73, 49],
        [46, 83, 45],
        None,
        1,
    ),
    profile(
        EuropeanLarch,
        "european-larch",
        Cone,
        [13.0, 28.0],
        [0.18, 0.25],
        0.20,
        0.030,
        0.18,
        [104, 74, 48],
        [78, 112, 47],
        None,
        2,
    ),
    profile(
        Cedar,
        "cedar",
        Pine,
        [15.0, 32.0],
        [0.25, 0.35],
        0.34,
        0.042,
        0.22,
        [89, 67, 50],
        [44, 77, 48],
        None,
        1,
    ),
    profile(
        Redwood,
        "redwood",
        Cone,
        [24.0, 38.0],
        [0.13, 0.19],
        0.18,
        0.040,
        0.16,
        [132, 68, 48],
        [45, 76, 48],
        None,
        3,
    ),
    profile(
        ItalianCypress,
        "italian-cypress",
        Columnar,
        [12.0, 26.0],
        [0.07, 0.12],
        0.08,
        0.027,
        0.04,
        [87, 68, 51],
        [39, 69, 47],
        None,
        1,
    ),
    profile(
        Banyan,
        "banyan",
        Rooted,
        [14.0, 25.0],
        [0.45, 0.62],
        0.30,
        0.060,
        0.12,
        [82, 68, 57],
        [52, 99, 45],
        Some(RootStyle::Banyan),
        0,
    ),
    profile(
        Kapok,
        "kapok",
        Emergent,
        [25.0, 40.0],
        [0.25, 0.36],
        0.66,
        0.050,
        0.20,
        [143, 132, 111],
        [49, 96, 45],
        None,
        0,
    ),
    profile(
        RainforestEmergent,
        "rainforest-emergent",
        Emergent,
        [22.0, 37.0],
        [0.25, 0.36],
        0.62,
        0.043,
        0.18,
        [101, 79, 58],
        [45, 94, 42],
        None,
        1,
    ),
    profile(
        TropicalBroadleaf,
        "tropical-broadleaf",
        Round,
        [15.0, 29.0],
        [0.31, 0.43],
        0.36,
        0.047,
        0.10,
        [91, 74, 55],
        [47, 101, 43],
        None,
        5,
    ),
    profile(
        RainforestUnderstory,
        "rainforest-understory",
        Round,
        [6.0, 14.0],
        [0.30, 0.45],
        0.24,
        0.045,
        0.16,
        [88, 71, 52],
        [62, 115, 47],
        None,
        6,
    ),
    profile(
        TropicalDry,
        "tropical-dry",
        Open,
        [8.0, 20.0],
        [0.32, 0.48],
        0.42,
        0.045,
        0.34,
        [103, 77, 52],
        [82, 105, 44],
        None,
        0,
    ),
    profile(
        Mahogany,
        "mahogany",
        Emergent,
        [18.0, 34.0],
        [0.25, 0.36],
        0.52,
        0.046,
        0.16,
        [115, 72, 48],
        [48, 95, 42],
        None,
        2,
    ),
    profile(
        TropicalFig,
        "tropical-fig",
        Spreading,
        [12.0, 25.0],
        [0.40, 0.54],
        0.32,
        0.055,
        0.12,
        [92, 75, 58],
        [49, 102, 44],
        None,
        3,
    ),
    profile(
        UmbrellaAcacia,
        "umbrella-acacia",
        Umbrella,
        [7.0, 16.0],
        [0.40, 0.58],
        0.55,
        0.045,
        0.22,
        [77, 59, 44],
        [88, 103, 43],
        None,
        0,
    ),
    profile(
        SavannaThorn,
        "savanna-thorn",
        Umbrella,
        [5.0, 13.0],
        [0.34, 0.50],
        0.48,
        0.040,
        0.34,
        [83, 61, 44],
        [96, 108, 49],
        None,
        2,
    ),
    profile(
        Olive,
        "olive",
        Umbrella,
        [5.0, 12.0],
        [0.34, 0.48],
        0.30,
        0.065,
        0.22,
        [105, 88, 69],
        [91, 108, 73],
        None,
        3,
    ),
    profile(
        HolmOak,
        "holm-oak",
        Spreading,
        [8.0, 18.0],
        [0.34, 0.47],
        0.30,
        0.055,
        0.12,
        [83, 66, 50],
        [48, 83, 45],
        None,
        4,
    ),
    profile(
        CorkOak,
        "cork-oak",
        Spreading,
        [8.0, 17.0],
        [0.36, 0.50],
        0.32,
        0.070,
        0.16,
        [101, 74, 52],
        [55, 91, 48],
        None,
        5,
    ),
    profile(
        Mesquite,
        "mesquite",
        Umbrella,
        [5.0, 12.0],
        [0.38, 0.56],
        0.38,
        0.050,
        0.38,
        [87, 64, 46],
        [96, 108, 53],
        None,
        4,
    ),
    profile(
        TreeProfileId::Baobab,
        "baobab",
        CrownGrammar::Baobab,
        [10.0, 21.0],
        [0.30, 0.44],
        0.64,
        0.145,
        0.42,
        [134, 116, 94],
        [86, 107, 53],
        None,
        0,
    ),
    profile(
        RiverRedGum,
        "river-red-gum",
        Open,
        [16.0, 32.0],
        [0.30, 0.42],
        0.42,
        0.055,
        0.30,
        [176, 162, 137],
        [72, 104, 67],
        None,
        1,
    ),
    profile(
        OpenEucalyptus,
        "open-eucalyptus",
        Open,
        [12.0, 27.0],
        [0.27, 0.40],
        0.45,
        0.043,
        0.38,
        [159, 148, 126],
        [73, 108, 68],
        None,
        2,
    ),
    profile(
        Paperbark,
        "paperbark",
        Open,
        [8.0, 19.0],
        [0.26, 0.38],
        0.34,
        0.060,
        0.28,
        [193, 184, 159],
        [69, 108, 66],
        None,
        3,
    ),
    profile(
        Pohutukawa,
        "pohutukawa",
        Spreading,
        [7.0, 17.0],
        [0.40, 0.56],
        0.28,
        0.060,
        0.16,
        [92, 72, 55],
        [49, 94, 49],
        None,
        6,
    ),
    profile(
        NorfolkIslandPine,
        "norfolk-island-pine",
        Cone,
        [15.0, 32.0],
        [0.15, 0.23],
        0.20,
        0.032,
        0.20,
        [101, 75, 54],
        [47, 86, 51],
        None,
        4,
    ),
    profile(
        CoconutPalm,
        "coconut-palm",
        Palm,
        [10.0, 24.0],
        [0.22, 0.32],
        0.80,
        0.022,
        0.46,
        [117, 89, 59],
        [66, 119, 48],
        None,
        0,
    ),
    profile(
        DatePalm,
        "date-palm",
        Palm,
        [8.0, 19.0],
        [0.20, 0.29],
        0.76,
        0.030,
        0.28,
        [111, 85, 58],
        [64, 105, 48],
        None,
        1,
    ),
    profile(
        FanPalm,
        "fan-palm",
        Palm,
        [7.0, 18.0],
        [0.20, 0.30],
        0.76,
        0.028,
        0.20,
        [111, 84, 58],
        [73, 116, 57],
        None,
        2,
    ),
    profile(
        RoyalPalm,
        "royal-palm",
        Palm,
        [13.0, 27.0],
        [0.18, 0.26],
        0.82,
        0.025,
        0.24,
        [151, 145, 125],
        [58, 111, 50],
        None,
        3,
    ),
    profile(
        RedMangrove,
        "red-mangrove",
        Rooted,
        [5.0, 13.0],
        [0.32, 0.45],
        0.40,
        0.055,
        0.12,
        [105, 68, 50],
        [49, 96, 49],
        Some(RootStyle::Mangrove),
        1,
    ),
    profile(
        Orchard,
        "orchard",
        Round,
        [4.0, 9.0],
        [0.32, 0.46],
        0.28,
        0.060,
        0.10,
        [102, 73, 49],
        [76, 121, 49],
        None,
        7,
    ),
    profile(
        FloweringOrnamental,
        "flowering-ornamental",
        Round,
        [4.0, 11.0],
        [0.30, 0.43],
        0.30,
        0.050,
        0.16,
        [105, 77, 56],
        [83, 127, 66],
        None,
        8,
    ),
    profile(
        StreetColumnar,
        "street-columnar",
        Columnar,
        [7.0, 17.0],
        [0.12, 0.20],
        0.22,
        0.032,
        0.08,
        [108, 86, 65],
        [72, 119, 52],
        None,
        2,
    ),
    profile(
        PlantationConifer,
        "plantation-conifer",
        Cone,
        [9.0, 22.0],
        [0.18, 0.25],
        0.16,
        0.031,
        0.04,
        [99, 73, 51],
        [48, 79, 45],
        None,
        5,
    ),
    profile(
        ParkRound,
        "park-round",
        Round,
        [7.0, 17.0],
        [0.31, 0.42],
        0.34,
        0.048,
        0.08,
        [96, 73, 53],
        [73, 119, 50],
        None,
        9,
    ),
];

#[inline]
pub fn tree_profile(id: TreeProfileId) -> &'static TreeProfile {
    &TREE_PROFILES[id as usize]
}

/// Rich deterministic tree colour for both streamed and object-local trees.
/// `detail` selects a small bark/leaf palette variation while `height_frac`
/// provides restrained vertical lighting and moss weighting. Keeping this in
/// the generator makes the near and distant representations agree on species.
pub fn tree_color(
    id: TreeProfileId,
    trunk: bool,
    tree_shade: f32,
    detail: f32,
    height_frac: f32,
) -> [u8; 3] {
    let profile = tree_profile(id);
    let detail = detail.clamp(0.0, 1.0);
    let height = height_frac.clamp(0.0, 1.0);
    let blend = |a: [f32; 3], b: [f32; 3], t: f32| {
        let t = t.clamp(0.0, 1.0);
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let to_u8 = |color: [f32; 3]| color.map(|channel| channel.round().clamp(0.0, 255.0) as u8);

    if trunk {
        let base = profile.bark.map(f32::from);
        let value = 0.76 + tree_shade * 0.14 + detail * 0.24 + height * 0.035;
        let mut color = base.map(|channel| channel * value);
        if detail < 0.22 {
            color = blend(color, [42.0, 34.0, 27.0], (0.22 - detail) * 0.72);
        } else if detail > 0.76 {
            color = blend(color, [178.0, 151.0, 112.0], (detail - 0.76) * 0.46);
        }
        // Occasional muted moss patches weight old tropical trunks and roots
        // without painting every species bright green.
        if height < 0.34 && detail > 0.84 {
            color = blend(color, [66.0, 78.0, 43.0], (detail - 0.84) * 0.62);
        }
        return to_u8(color);
    }

    let base = profile.leaves.map(f32::from);
    let value = 0.70 + tree_shade * 0.15 + detail * 0.25 + height * 0.10;
    let hue = detail - 0.5;
    to_u8([
        base[0] * value * (1.0 - hue * 0.12),
        base[1] * value * (1.0 + hue * 0.18),
        base[2] * value * (1.0 - hue * 0.08),
    ])
}

#[derive(Clone, Copy, Debug)]
pub struct TreeEnvironment {
    pub lat_deg: f32,
    pub lon_deg: f32,
    pub temp_c: f32,
    pub arid: f32,
    pub elevation_m: f32,
    pub near_water: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteKind {
    Boreal,
    Alpine,
    EuropeanMontane,
    EuropeanSubalpine,
    TemperateEurope,
    TemperateNorthAmericaEast,
    TemperateNorthAmericaWest,
    TemperateEastAsia,
    TemperateGlobal,
    Mediterranean,
    TropicalAmericas,
    TropicalAfrica,
    TropicalAsia,
    AfricanSavanna,
    AustralianDry,
    AustralianTropical,
    AustralianTemperate,
    NewZealand,
    AridGlobal,
}

#[derive(Clone, Copy)]
struct WeightedProfile(TreeProfileId, u8);

const BOREAL: &[WeightedProfile] = &[
    WeightedProfile(NorwaySpruce, 26),
    WeightedProfile(SilverFir, 15),
    WeightedProfile(ScotsPine, 20),
    WeightedProfile(EuropeanLarch, 12),
    WeightedProfile(SilverBirch, 12),
    WeightedProfile(Aspen, 10),
    WeightedProfile(PlantationConifer, 5),
];
const ALPINE: &[WeightedProfile] = &[
    WeightedProfile(NorwaySpruce, 42),
    WeightedProfile(EuropeanLarch, 25),
    WeightedProfile(SilverFir, 18),
    WeightedProfile(ScotsPine, 15),
];
const EUROPEAN_MONTANE: &[WeightedProfile] = &[
    WeightedProfile(NorwaySpruce, 55),
    WeightedProfile(SilverFir, 25),
    WeightedProfile(EuropeanBeech, 15),
    WeightedProfile(ScotsPine, 5),
];
const EUROPEAN_SUBALPINE: &[WeightedProfile] = &[
    WeightedProfile(NorwaySpruce, 65),
    WeightedProfile(EuropeanLarch, 25),
    WeightedProfile(ScotsPine, 8),
    WeightedProfile(SilverFir, 2),
];
const EUROPE: &[WeightedProfile] = &[
    WeightedProfile(EnglishOak, 18),
    WeightedProfile(EuropeanBeech, 16),
    WeightedProfile(SilverBirch, 10),
    WeightedProfile(Elm, 9),
    WeightedProfile(HorseChestnut, 8),
    WeightedProfile(WeepingWillow, 6),
    WeightedProfile(ColumnarPoplar, 7),
    WeightedProfile(LondonPlane, 8),
    WeightedProfile(ScotsPine, 8),
    WeightedProfile(SilverFir, 5),
    WeightedProfile(SugarMaple, 5),
];
const NA_EAST: &[WeightedProfile] = &[
    WeightedProfile(SugarMaple, 20),
    WeightedProfile(EnglishOak, 16),
    WeightedProfile(Aspen, 10),
    WeightedProfile(Elm, 10),
    WeightedProfile(HorseChestnut, 7),
    WeightedProfile(WeepingWillow, 5),
    WeightedProfile(ScotsPine, 12),
    WeightedProfile(SilverFir, 10),
    WeightedProfile(ParkRound, 10),
];
const NA_WEST: &[WeightedProfile] = &[
    WeightedProfile(ScotsPine, 20),
    WeightedProfile(SilverFir, 14),
    WeightedProfile(Redwood, 12),
    WeightedProfile(EnglishOak, 10),
    WeightedProfile(Aspen, 10),
    WeightedProfile(Cedar, 12),
    WeightedProfile(StonePine, 8),
    WeightedProfile(ParkRound, 8),
    WeightedProfile(PlantationConifer, 6),
];
const EAST_ASIA: &[WeightedProfile] = &[
    WeightedProfile(EuropeanBeech, 12),
    WeightedProfile(SugarMaple, 12),
    WeightedProfile(SilverBirch, 10),
    WeightedProfile(Cedar, 13),
    WeightedProfile(ScotsPine, 12),
    WeightedProfile(Elm, 10),
    WeightedProfile(FloweringOrnamental, 10),
    WeightedProfile(ColumnarPoplar, 8),
    WeightedProfile(ParkRound, 13),
];
const TEMPERATE_GLOBAL: &[WeightedProfile] = &[
    WeightedProfile(EnglishOak, 20),
    WeightedProfile(EuropeanBeech, 14),
    WeightedProfile(SilverBirch, 12),
    WeightedProfile(Aspen, 10),
    WeightedProfile(ScotsPine, 16),
    WeightedProfile(SilverFir, 10),
    WeightedProfile(WeepingWillow, 6),
    WeightedProfile(ParkRound, 12),
];
const MEDITERRANEAN: &[WeightedProfile] = &[
    WeightedProfile(StonePine, 18),
    WeightedProfile(ItalianCypress, 12),
    WeightedProfile(Olive, 16),
    WeightedProfile(HolmOak, 18),
    WeightedProfile(CorkOak, 14),
    WeightedProfile(Cedar, 8),
    WeightedProfile(Orchard, 7),
    WeightedProfile(FloweringOrnamental, 7),
];
const TROPICAL_AMERICAS: &[WeightedProfile] = &[
    WeightedProfile(Kapok, 8),
    WeightedProfile(RainforestEmergent, 14),
    WeightedProfile(TropicalBroadleaf, 22),
    WeightedProfile(RainforestUnderstory, 13),
    WeightedProfile(Mahogany, 10),
    WeightedProfile(TropicalFig, 13),
    WeightedProfile(CoconutPalm, 6),
    WeightedProfile(RoyalPalm, 6),
    WeightedProfile(TropicalDry, 8),
];
const TROPICAL_AFRICA: &[WeightedProfile] = &[
    WeightedProfile(RainforestEmergent, 15),
    WeightedProfile(TropicalBroadleaf, 24),
    WeightedProfile(RainforestUnderstory, 15),
    WeightedProfile(TropicalFig, 12),
    WeightedProfile(Mahogany, 10),
    WeightedProfile(TropicalDry, 10),
    WeightedProfile(RoyalPalm, 6),
    WeightedProfile(UmbrellaAcacia, 8),
];
const TROPICAL_ASIA: &[WeightedProfile] = &[
    WeightedProfile(Banyan, 10),
    WeightedProfile(RainforestEmergent, 14),
    WeightedProfile(TropicalBroadleaf, 23),
    WeightedProfile(RainforestUnderstory, 13),
    WeightedProfile(TropicalFig, 14),
    WeightedProfile(Mahogany, 8),
    WeightedProfile(CoconutPalm, 9),
    WeightedProfile(RoyalPalm, 5),
    WeightedProfile(TropicalDry, 4),
];
const AFRICAN_SAVANNA: &[WeightedProfile] = &[
    WeightedProfile(UmbrellaAcacia, 30),
    WeightedProfile(SavannaThorn, 24),
    WeightedProfile(TreeProfileId::Baobab, 8),
    WeightedProfile(TropicalDry, 14),
    WeightedProfile(Mesquite, 12),
    WeightedProfile(DatePalm, 4),
    WeightedProfile(Olive, 8),
];
const AUSTRALIAN_DRY: &[WeightedProfile] = &[
    WeightedProfile(OpenEucalyptus, 34),
    WeightedProfile(RiverRedGum, 22),
    WeightedProfile(UmbrellaAcacia, 14),
    WeightedProfile(SavannaThorn, 10),
    WeightedProfile(Paperbark, 8),
    WeightedProfile(TropicalDry, 7),
    WeightedProfile(NorfolkIslandPine, 5),
];
const AUSTRALIAN_TROPICAL: &[WeightedProfile] = &[
    WeightedProfile(Paperbark, 20),
    WeightedProfile(RiverRedGum, 18),
    WeightedProfile(OpenEucalyptus, 18),
    WeightedProfile(TropicalBroadleaf, 12),
    WeightedProfile(RainforestUnderstory, 9),
    WeightedProfile(Banyan, 5),
    WeightedProfile(CoconutPalm, 7),
    WeightedProfile(FanPalm, 5),
    WeightedProfile(NorfolkIslandPine, 6),
];
const AUSTRALIAN_TEMPERATE: &[WeightedProfile] = &[
    WeightedProfile(RiverRedGum, 24),
    WeightedProfile(OpenEucalyptus, 28),
    WeightedProfile(Paperbark, 13),
    WeightedProfile(Pohutukawa, 5),
    WeightedProfile(NorfolkIslandPine, 8),
    WeightedProfile(EnglishOak, 5),
    WeightedProfile(ScotsPine, 4),
    WeightedProfile(ParkRound, 8),
    WeightedProfile(FloweringOrnamental, 5),
];
const NEW_ZEALAND: &[WeightedProfile] = &[
    WeightedProfile(Pohutukawa, 24),
    WeightedProfile(NorfolkIslandPine, 15),
    WeightedProfile(TropicalBroadleaf, 12),
    WeightedProfile(RainforestUnderstory, 12),
    WeightedProfile(OpenEucalyptus, 7),
    WeightedProfile(SilverFir, 8),
    WeightedProfile(EnglishOak, 8),
    WeightedProfile(ParkRound, 14),
];
const ARID_GLOBAL: &[WeightedProfile] = &[
    WeightedProfile(Mesquite, 25),
    WeightedProfile(SavannaThorn, 19),
    WeightedProfile(UmbrellaAcacia, 16),
    WeightedProfile(Olive, 12),
    WeightedProfile(TropicalDry, 10),
    WeightedProfile(DatePalm, 8),
    WeightedProfile(FanPalm, 5),
    WeightedProfile(Orchard, 5),
];
const MANAGED_COOL: &[WeightedProfile] = &[
    WeightedProfile(LondonPlane, 18),
    WeightedProfile(StreetColumnar, 14),
    WeightedProfile(ParkRound, 18),
    WeightedProfile(FloweringOrnamental, 14),
    WeightedProfile(Orchard, 9),
    WeightedProfile(HorseChestnut, 10),
    WeightedProfile(WeepingWillow, 7),
    WeightedProfile(PlantationConifer, 10),
];
const MANAGED_WARM: &[WeightedProfile] = &[
    WeightedProfile(ParkRound, 18),
    WeightedProfile(FloweringOrnamental, 15),
    WeightedProfile(StreetColumnar, 11),
    WeightedProfile(Orchard, 12),
    WeightedProfile(FanPalm, 10),
    WeightedProfile(DatePalm, 8),
    WeightedProfile(RoyalPalm, 8),
    WeightedProfile(Olive, 8),
    WeightedProfile(TropicalFig, 10),
];

fn longitude(lon: f32) -> f32 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Coarse European mountain envelope used only where elevation confirms a
/// montane site. It intentionally covers the Alpine arc and neighboring high
/// ranges rather than treating every lowland coordinate in the box as alpine.
fn european_mountain_region(lat: f32, lon: f32) -> bool {
    (43.0..=49.5).contains(&lat) && (4.0..=18.0).contains(&lon)
}

/// OSM forest polygons remain the authority below the high-mountain belt.
/// Near the Alpine treeline, however, generalized polygons can cover pasture,
/// heath and rock. Fade candidate survival from closed subalpine forest to the
/// sparse treeline ecotone, then reject full-size trees above it.
pub fn natural_tree_density_factor(env: TreeEnvironment) -> f32 {
    const CLOSED_FOREST_LIMIT_M: f32 = 1_850.0;
    const ALPINE_TREELINE_M: f32 = 2_400.0;

    let lon = longitude(env.lon_deg);
    if !european_mountain_region(env.lat_deg, lon) || env.elevation_m <= CLOSED_FOREST_LIMIT_M {
        return 1.0;
    }
    let t = ((ALPINE_TREELINE_M - env.elevation_m) / (ALPINE_TREELINE_M - CLOSED_FOREST_LIMIT_M))
        .clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Broad compiled geographic palette.  It is deliberately coarse: climate,
/// altitude and local land use refine it, while no runtime dataset is loaded.
pub fn palette_kind(env: TreeEnvironment) -> PaletteKind {
    let lat = env.lat_deg;
    let lon = longitude(env.lon_deg);

    if european_mountain_region(lat, lon) && env.elevation_m >= 650.0 && env.temp_c < 14.0 {
        return if env.elevation_m >= 1_550.0 {
            PaletteKind::EuropeanSubalpine
        } else {
            PaletteKind::EuropeanMontane
        };
    }
    if env.elevation_m > 2_000.0 && env.temp_c < 12.0 {
        return PaletteKind::Alpine;
    }
    if env.temp_c < 2.5 || lat.abs() > 61.0 {
        return PaletteKind::Boreal;
    }

    let australia = (-45.0..=-10.0).contains(&lat) && (110.0..=155.0).contains(&lon);
    let new_zealand = (-49.0..=-32.0).contains(&lat) && (164.0..=180.0).contains(&lon);
    let africa = (-36.0..=38.0).contains(&lat) && (-20.0..=53.0).contains(&lon);
    let europe = (35.0..=72.0).contains(&lat) && (-12.0..=45.0).contains(&lon);
    let north_america = (15.0..=72.0).contains(&lat) && (-170.0..=-50.0).contains(&lon);
    let south_america = (-56.0..=14.0).contains(&lat) && (-84.0..=-34.0).contains(&lon);
    let east_asia = (20.0..=58.0).contains(&lat) && (100.0..=150.0).contains(&lon);
    let tropical_asia = (-11.0..=25.0).contains(&lat) && (60.0..=150.0).contains(&lon);

    if australia {
        return if env.temp_c > 22.0 && lat > -25.0 && env.arid < 0.48 {
            PaletteKind::AustralianTropical
        } else if env.arid > 0.48 {
            PaletteKind::AustralianDry
        } else {
            PaletteKind::AustralianTemperate
        };
    }
    if new_zealand {
        return PaletteKind::NewZealand;
    }
    if env.arid > 0.70 && env.temp_c > 10.0 {
        return if africa {
            PaletteKind::AfricanSavanna
        } else {
            PaletteKind::AridGlobal
        };
    }
    if env.temp_c > 19.0 && lat.abs() < 30.0 {
        return if south_america {
            PaletteKind::TropicalAmericas
        } else if africa {
            PaletteKind::TropicalAfrica
        } else if tropical_asia || lon > 45.0 {
            PaletteKind::TropicalAsia
        } else {
            PaletteKind::TropicalAmericas
        };
    }
    if europe && env.temp_c > 9.0 && (env.arid > 0.34 || lat < 44.0) {
        return PaletteKind::Mediterranean;
    }
    if europe {
        return PaletteKind::TemperateEurope;
    }
    if north_america {
        return if lon < -105.0 {
            PaletteKind::TemperateNorthAmericaWest
        } else {
            PaletteKind::TemperateNorthAmericaEast
        };
    }
    if east_asia {
        return PaletteKind::TemperateEastAsia;
    }
    PaletteKind::TemperateGlobal
}

fn palette(kind: PaletteKind) -> &'static [WeightedProfile] {
    match kind {
        PaletteKind::Boreal => BOREAL,
        PaletteKind::Alpine => ALPINE,
        PaletteKind::EuropeanMontane => EUROPEAN_MONTANE,
        PaletteKind::EuropeanSubalpine => EUROPEAN_SUBALPINE,
        PaletteKind::TemperateEurope => EUROPE,
        PaletteKind::TemperateNorthAmericaEast => NA_EAST,
        PaletteKind::TemperateNorthAmericaWest => NA_WEST,
        PaletteKind::TemperateEastAsia => EAST_ASIA,
        PaletteKind::TemperateGlobal => TEMPERATE_GLOBAL,
        PaletteKind::Mediterranean => MEDITERRANEAN,
        PaletteKind::TropicalAmericas => TROPICAL_AMERICAS,
        PaletteKind::TropicalAfrica => TROPICAL_AFRICA,
        PaletteKind::TropicalAsia => TROPICAL_ASIA,
        PaletteKind::AfricanSavanna => AFRICAN_SAVANNA,
        PaletteKind::AustralianDry => AUSTRALIAN_DRY,
        PaletteKind::AustralianTropical => AUSTRALIAN_TROPICAL,
        PaletteKind::AustralianTemperate => AUSTRALIAN_TEMPERATE,
        PaletteKind::NewZealand => NEW_ZEALAND,
        PaletteKind::AridGlobal => ARID_GLOBAL,
    }
}

#[inline]
fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

#[inline]
fn unit(seed: u64) -> f32 {
    ((splitmix(seed) >> 40) as f32) / (1u32 << 24) as f32
}

fn choose_weighted(items: &[WeightedProfile], draw: f32) -> TreeProfileId {
    let total: u32 = items.iter().map(|item| item.1 as u32).sum();
    let mut ticket = (draw.clamp(0.0, 0.999_999) * total as f32) as u32;
    for item in items {
        if ticket < item.1 as u32 {
            return item.0;
        }
        ticket -= item.1 as u32;
    }
    items.last().expect("tree palette is non-empty").0
}

// These are regional canopy priors expressed with the existing visual assets,
// not species observations. Companions belong to the same woodland community;
// ornamental and cultivated profiles are confined to managed placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WoodlandCommunity {
    dominant: TreeProfileId,
    companion: TreeProfileId,
}

macro_rules! woodlands {
    ($($name:ident: $dominant:ident, $companion:ident;)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum WoodlandKind { $($name),+ }
        impl WoodlandKind {
            fn community(self) -> WoodlandCommunity {
                match self { $(Self::$name => WoodlandCommunity {
                    dominant: $dominant, companion: $companion,
                }),+ }
            }
        }
        #[cfg(test)]
        const WOODLANDS: &[WoodlandKind] = &[$(WoodlandKind::$name),+];
    };
}

woodlands! {
    AtlanticOak: EnglishOak, EuropeanBeech;
    ContinentalBeech: EuropeanBeech, EnglishOak;
    BalticPine: ScotsPine, SilverBirch;
    NordicSpruce: NorwaySpruce, SilverBirch;
    NordicBirch: SilverBirch, Aspen;
    SiberianLarch: EuropeanLarch, ScotsPine;
    CanadianSpruce: NorwaySpruce, SilverFir;
    CanadianAspen: Aspen, SilverBirch;
    AlpineSpruce: NorwaySpruce, SilverFir;
    AlpineLarch: EuropeanLarch, NorwaySpruce;
    MontaneFir: SilverFir, NorwaySpruce;
    MontaneBeech: EuropeanBeech, SilverFir;
    HimalayanCedar: Cedar, ScotsPine;
    HimalayanPine: ScotsPine, SilverFir;
    JapaneseCedar: Cedar, Cedar;
    EastAsianBeech: EuropeanBeech, SugarMaple;
    EastAsianPine: ScotsPine, EuropeanBeech;
    NortheastMaple: SugarMaple, EnglishOak;
    AppalachianOak: EnglishOak, SugarMaple;
    SouthernPine: ScotsPine, ScotsPine;
    PacificFir: SilverFir, Cedar;
    PacificRedwood: Redwood, SilverFir;
    WesternPine: ScotsPine, ScotsPine;
    WesternOak: EnglishOak, EnglishOak;
    IberianCork: CorkOak, HolmOak;
    MediterraneanOak: HolmOak, HolmOak;
    MediterraneanPine: StonePine, HolmOak;
    AtlasCedar: Cedar, HolmOak;
    AmazonCanopy: TropicalBroadleaf, RainforestEmergent;
    AtlanticRainforest: TropicalBroadleaf, Mahogany;
    CentralAmericanCanopy: Mahogany, TropicalBroadleaf;
    AmericanDryForest: TropicalDry, TropicalDry;
    CongoCanopy: TropicalBroadleaf, RainforestUnderstory;
    GuineanCanopy: Mahogany, TropicalBroadleaf;
    AfricanThornveld: SavannaThorn, UmbrellaAcacia;
    AfricanAcacia: UmbrellaAcacia, SavannaThorn;
    AfricanMiombo: TropicalDry, SavannaThorn;
    IndianMonsoon: TropicalBroadleaf, TropicalDry;
    IndianDryForest: TropicalDry, TropicalDry;
    SoutheastAsianCanopy: TropicalBroadleaf, RainforestEmergent;
    SundalandCanopy: RainforestEmergent, TropicalBroadleaf;
    QueenslandRainforest: TropicalBroadleaf, RainforestUnderstory;
    NorthernPaperbark: Paperbark, RiverRedGum;
    AustralianGum: OpenEucalyptus, RiverRedGum;
    AustralianRiparian: RiverRedGum, Paperbark;
    AustralianDryGum: OpenEucalyptus, OpenEucalyptus;
    NewZealandBroadleaf: TropicalBroadleaf, RainforestUnderstory;
    NewZealandCoastal: Pohutukawa, Pohutukawa;
    PatagonianBeech: EuropeanBeech, EuropeanBeech;
    AndeanWoodland: TropicalDry, TropicalBroadleaf;
    SouthAmericanDry: TropicalDry, Mesquite;
    AmericanMesquite: Mesquite, Mesquite;
    AsianDryThorn: SavannaThorn, TropicalDry;
    TemperateOak: EnglishOak, EuropeanBeech;
}

fn habitat_noise(point: [f64; 3], salt: u64) -> f64 {
    let base = point.map(|p| p.floor() as i64);
    let blend = std::array::from_fn::<_, 3, _>(|axis| {
        let f = point[axis] - base[axis] as f64;
        f * f * (3.0 - 2.0 * f)
    });
    let mut result = 0.0;
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let offset = [dx, dy, dz];
                let mut seed = salt;
                let mut weight = 1.0;
                for axis in 0..3 {
                    seed = splitmix(seed ^ (base[axis] + offset[axis]) as u64);
                    weight *= if offset[axis] == 0 {
                        1.0 - blend[axis]
                    } else {
                        blend[axis]
                    };
                }
                result += weight * f64::from(unit(seed));
            }
        }
    }
    result
}

// Sampling a sphere avoids cube-face and longitude seams. The two continuous
// fields bend community edges across roughly 40 km and 10 km habitat scales;
// neighboring 300 m stands no longer draw unrelated tree families.
fn woodland_habitat(env: TreeEnvironment) -> f64 {
    let lat = f64::from(env.lat_deg).to_radians();
    let lon = f64::from(longitude(env.lon_deg)).to_radians();
    let direction = [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()];
    habitat_noise(direction.map(|p| p * 160.0), 0x574f_4f44_4c41_4e44) * 0.85
        + habitat_noise(direction.map(|p| p * 640.0), 0x4841_4249_5441_545f) * 0.15
}

fn woodland_kind(env: TreeEnvironment) -> WoodlandKind {
    use WoodlandKind::*;
    let lat = env.lat_deg;
    let lon = longitude(env.lon_deg);
    let habitat = woodland_habitat(env);
    match palette_kind(env) {
        PaletteKind::Boreal => {
            if (-170.0..=-50.0).contains(&lon) {
                if habitat < 0.38 {
                    CanadianAspen
                } else {
                    CanadianSpruce
                }
            } else if lon > 65.0 {
                SiberianLarch
            } else if habitat < 0.30 {
                NordicBirch
            } else if habitat > 0.68 {
                BalticPine
            } else {
                NordicSpruce
            }
        }
        PaletteKind::Alpine => {
            if (60.0..=105.0).contains(&lon) && lat > 20.0 {
                if habitat < 0.45 {
                    HimalayanCedar
                } else {
                    HimalayanPine
                }
            } else if lon < -50.0 {
                WesternPine
            } else {
                AlpineSpruce
            }
        }
        PaletteKind::EuropeanMontane => {
            if env.elevation_m < 1000.0 && habitat < 0.45 {
                MontaneBeech
            } else if habitat < 0.35 {
                MontaneFir
            } else {
                AlpineSpruce
            }
        }
        PaletteKind::EuropeanSubalpine => {
            if habitat > 0.67 {
                AlpineLarch
            } else {
                AlpineSpruce
            }
        }
        PaletteKind::TemperateEurope => {
            if lat > 55.0 || env.arid > 0.4 {
                BalticPine
            } else if habitat < 0.48 {
                AtlanticOak
            } else {
                ContinentalBeech
            }
        }
        PaletteKind::TemperateNorthAmericaEast => {
            if lat < 36.0 && habitat > 0.38 {
                SouthernPine
            } else if lat > 40.0 && habitat > 0.42 {
                NortheastMaple
            } else {
                AppalachianOak
            }
        }
        PaletteKind::TemperateNorthAmericaWest => {
            if env.arid > 0.40 || env.elevation_m > 1100.0 {
                WesternPine
            } else if lat < 39.0 && lon < -121.0 && habitat < 0.46 {
                PacificRedwood
            } else if lat < 40.0 {
                WesternOak
            } else {
                PacificFir
            }
        }
        PaletteKind::TemperateEastAsia => {
            if lon > 129.0 && habitat > 0.56 {
                JapaneseCedar
            } else if env.arid > 0.36 || habitat < 0.32 {
                EastAsianPine
            } else {
                EastAsianBeech
            }
        }
        PaletteKind::Mediterranean => {
            if lon < 0.0 && habitat < 0.48 {
                IberianCork
            } else if habitat > 0.61 {
                MediterraneanPine
            } else {
                MediterraneanOak
            }
        }
        PaletteKind::TropicalAmericas => {
            if env.arid > 0.40 {
                AmericanDryForest
            } else if lat > 7.0 {
                CentralAmericanCanopy
            } else if lat < -15.0 && lon > -50.0 {
                AtlanticRainforest
            } else {
                AmazonCanopy
            }
        }
        PaletteKind::TropicalAfrica => {
            if env.arid > 0.40 {
                AfricanMiombo
            } else if lon < 10.0 {
                GuineanCanopy
            } else {
                CongoCanopy
            }
        }
        PaletteKind::TropicalAsia => {
            if env.arid > 0.40 {
                IndianDryForest
            } else if lon < 95.0 {
                IndianMonsoon
            } else if lat < 8.0 && habitat > 0.40 {
                SundalandCanopy
            } else {
                SoutheastAsianCanopy
            }
        }
        PaletteKind::AfricanSavanna => {
            if lat > 28.0 && env.elevation_m > 900.0 {
                AtlasCedar
            } else if habitat > 0.55 {
                AfricanAcacia
            } else {
                AfricanThornveld
            }
        }
        PaletteKind::AustralianDry => {
            if env.near_water {
                AustralianRiparian
            } else {
                AustralianDryGum
            }
        }
        PaletteKind::AustralianTropical => {
            if lon > 143.0 && env.arid < 0.24 {
                QueenslandRainforest
            } else if env.near_water || habitat > 0.61 {
                NorthernPaperbark
            } else {
                AustralianGum
            }
        }
        PaletteKind::AustralianTemperate => {
            if env.near_water {
                AustralianRiparian
            } else {
                AustralianGum
            }
        }
        PaletteKind::NewZealand => {
            if lat > -38.0 && env.elevation_m < 120.0 && habitat > 0.58 {
                NewZealandCoastal
            } else {
                NewZealandBroadleaf
            }
        }
        PaletteKind::AridGlobal => {
            if lon < -30.0 {
                AmericanMesquite
            } else {
                AsianDryThorn
            }
        }
        PaletteKind::TemperateGlobal => {
            if (-84.0..=-34.0).contains(&lon) && lat < -15.0 {
                if lat < -38.0 {
                    PatagonianBeech
                } else if env.arid > 0.38 {
                    SouthAmericanDry
                } else {
                    AndeanWoodland
                }
            } else if (60.0..=105.0).contains(&lon) && env.elevation_m > 900.0 {
                HimalayanCedar
            } else if (60.0..=100.0).contains(&lon) && lat < 32.0 {
                IndianMonsoon
            } else {
                TemperateOak
            }
        }
    }
}

const MANAGED_GROVE_SHARE: f32 = 0.84;
const MANAGED_PALETTE_SHARE: f32 = 0.58;

/// Select a stable visual profile. Natural forests inherit a regional woodland
/// community; managed groves use a local stand seed and ornamental palettes.
pub fn select_profile(
    env: TreeEnvironment,
    managed: bool,
    candidate_seed: u64,
    stand_seed: u64,
) -> TreeProfileId {
    if !managed {
        let community = woodland_kind(env).community();
        return if unit(candidate_seed ^ 0x434f_4d50_414e_494f) < 0.035 {
            community.companion
        } else {
            community.dominant
        };
    }

    let base = palette(palette_kind(env));
    let managed_palette = if env.temp_c < 13.0 {
        MANAGED_COOL
    } else {
        MANAGED_WARM
    };
    let stand_share = MANAGED_GROVE_SHARE;
    let shared = unit(candidate_seed ^ 0x5354_414e_444d_4958) < stand_share;
    let selection_seed = if shared { stand_seed } else { candidate_seed };
    let items = if unit(selection_seed ^ 0x004d_414e_4147_4544) < MANAGED_PALETTE_SHARE {
        managed_palette
    } else {
        base
    };
    let draw = if shared {
        unit(stand_seed ^ 0x5354_414e_4450_524f)
    } else {
        unit(candidate_seed ^ 0x5452_4545_5052_4f46)
    };
    choose_weighted(items, draw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn env(lat: f32, lon: f32, temp: f32, arid: f32) -> TreeEnvironment {
        TreeEnvironment {
            lat_deg: lat,
            lon_deg: lon,
            temp_c: temp,
            arid,
            elevation_m: 20.0,
            near_water: false,
        }
    }

    #[test]
    fn profile_table_is_complete_and_index_aligned() {
        assert_eq!(TREE_PROFILES.len(), TREE_PROFILE_COUNT);
        let mut names = HashSet::new();
        for (index, profile) in TREE_PROFILES.iter().enumerate() {
            assert_eq!(profile.id as usize, index);
            assert!(names.insert(profile.name), "duplicate {}", profile.name);
            assert!(profile.height_m[0] > 0.0 && profile.height_m[1] >= profile.height_m[0]);
            assert!(
                profile.radius_ratio[0] > 0.0 && profile.radius_ratio[1] >= profile.radius_ratio[0]
            );
        }
    }

    #[test]
    fn every_profile_has_explicit_round_tripping_foliage_traits() {
        let mut semantic_materials = HashSet::new();
        for profile in TREE_PROFILES {
            assert_eq!(profile.foliage, profile.id.foliage_traits());
            let material = crate::material::foliage(profile.foliage);
            assert!(crate::material::is_foliage(material));
            assert_eq!(
                crate::material::foliage_traits(material),
                Some(profile.foliage)
            );
            semantic_materials.insert(material);
        }
        assert!(
            semantic_materials.len() > 12,
            "profiles must retain varied semantics"
        );
        assert_eq!(
            EuropeanLarch.foliage_traits().season,
            FoliageSeasonClass::DeciduousConifer
        );
        assert_eq!(
            NorwaySpruce.foliage_traits().season,
            FoliageSeasonClass::EvergreenNeedle
        );
        assert_eq!(
            CoconutPalm.foliage_traits().motion,
            FoliageMotionClass::Frond
        );
    }

    #[test]
    fn managed_palettes_and_reserved_habitats_preserve_the_full_asset_catalog() {
        let palettes = [
            BOREAL,
            ALPINE,
            EUROPEAN_MONTANE,
            EUROPEAN_SUBALPINE,
            EUROPE,
            NA_EAST,
            NA_WEST,
            EAST_ASIA,
            TEMPERATE_GLOBAL,
            MEDITERRANEAN,
            TROPICAL_AMERICAS,
            TROPICAL_AFRICA,
            TROPICAL_ASIA,
            AFRICAN_SAVANNA,
            AUSTRALIAN_DRY,
            AUSTRALIAN_TROPICAL,
            AUSTRALIAN_TEMPERATE,
            NEW_ZEALAND,
            ARID_GLOBAL,
            MANAGED_COOL,
            MANAGED_WARM,
        ];
        let mut reachable: HashSet<_> = palettes
            .into_iter()
            .flat_map(|items| items.iter().map(|item| item.0))
            .collect();
        // Preserve the mangrove asset for explicit tidal habitat data; generic
        // nearby water cannot establish a mangrove planting location.
        reachable.insert(RedMangrove);
        assert_eq!(reachable.len(), TREE_PROFILE_COUNT);
        assert!(
            TREE_PROFILES
                .iter()
                .all(|profile| reachable.contains(&profile.id))
        );
    }

    #[test]
    fn representative_places_choose_the_intended_palette() {
        assert_eq!(
            palette_kind(env(52.37, 4.90, 10.0, 0.15)),
            PaletteKind::TemperateEurope
        );
        assert_eq!(
            palette_kind(env(40.71, -74.0, 13.0, 0.18)),
            PaletteKind::TemperateNorthAmericaEast
        );
        assert_eq!(
            palette_kind(env(49.28, -123.1, 11.0, 0.18)),
            PaletteKind::TemperateNorthAmericaWest
        );
        assert_eq!(
            palette_kind(env(35.68, 139.7, 15.0, 0.20)),
            PaletteKind::TemperateEastAsia
        );
        assert_eq!(
            palette_kind(env(41.90, 12.5, 16.0, 0.38)),
            PaletteKind::Mediterranean
        );
        assert_eq!(
            palette_kind(env(-23.55, -46.6, 24.0, 0.15)),
            PaletteKind::TropicalAmericas
        );
        assert_eq!(
            palette_kind(env(0.35, 20.0, 26.0, 0.08)),
            PaletteKind::TropicalAfrica
        );
        assert_eq!(
            palette_kind(env(-33.86, 151.21, 18.0, 0.20)),
            PaletteKind::AustralianTemperate
        );
        assert_eq!(
            palette_kind(env(-12.46, 130.84, 27.0, 0.16)),
            PaletteKind::AustralianTropical
        );
        assert_eq!(
            palette_kind(env(-23.0, 133.0, 22.0, 0.80)),
            PaletteKind::AustralianDry
        );
        assert_eq!(
            palette_kind(env(-36.85, 174.76, 16.0, 0.20)),
            PaletteKind::NewZealand
        );
        assert_eq!(
            palette_kind(env(1.0, 114.0, 26.0, 0.08)),
            PaletteKind::TropicalAsia
        );
        assert_eq!(
            palette_kind(env(13.0, 20.0, 25.0, 0.82)),
            PaletteKind::AfricanSavanna
        );
        assert_eq!(
            palette_kind(env(25.2, 55.3, 28.0, 0.82)),
            PaletteKind::AridGlobal
        );
        let mut alps = env(46.5, 8.0, 5.0, 0.10);
        alps.elevation_m = 2_300.0;
        assert_eq!(palette_kind(alps), PaletteKind::EuropeanSubalpine);
        let mut alpine_valley = env(46.5, 8.0, 8.0, 0.10);
        alpine_valley.elevation_m = 900.0;
        assert_eq!(palette_kind(alpine_valley), PaletteKind::EuropeanMontane);
        assert_eq!(
            palette_kind(env(64.0, 25.0, 1.0, 0.10)),
            PaletteKind::Boreal
        );
        assert_eq!(
            palette_kind(env(-34.6, -58.4, 17.0, 0.18)),
            PaletteKind::TemperateGlobal
        );
    }

    #[test]
    fn natural_stands_are_dominant_without_becoming_monocultures() {
        let sydney = env(-33.86, 151.21, 18.0, 0.20);
        let stand = 91;
        let picks: Vec<_> = (0..256)
            .map(|seed| select_profile(sydney, false, seed, stand))
            .collect();
        let dominant = picks
            .iter()
            .map(|pick| picks.iter().filter(|other| *other == pick).count())
            .max()
            .unwrap();
        let distinct: HashSet<_> = picks.iter().copied().collect();
        assert!(
            dominant >= 235,
            "at least 92% of a natural stand should share one profile: {dominant}/{}",
            picks.len()
        );
        assert!(
            distinct.len() >= 2,
            "rare individual variation should keep the stand from being absolute monoculture"
        );
    }

    #[test]
    fn european_mountain_forests_follow_altitude_bands() {
        let mut alps = env(46.5, 8.0, 8.0, 0.10);
        alps.elevation_m = 1_100.0;
        let montane: HashSet<_> = [NorwaySpruce, SilverFir, EuropeanBeech, ScotsPine]
            .into_iter()
            .collect();
        for stand in 0..512 {
            for candidate in 0..16 {
                let selected = select_profile(alps, false, candidate, stand);
                assert!(
                    montane.contains(&selected),
                    "unexpected montane profile {selected:?}"
                );
            }
        }

        alps.elevation_m = 1_900.0;
        let subalpine: HashSet<_> = [NorwaySpruce, EuropeanLarch, ScotsPine, SilverFir]
            .into_iter()
            .collect();
        for stand in 0..512 {
            for candidate in 0..16 {
                let selected = select_profile(alps, false, candidate, stand);
                assert!(
                    subalpine.contains(&selected),
                    "unexpected subalpine profile {selected:?}"
                );
            }
        }
    }

    #[test]
    fn alpine_tree_cover_thins_to_zero_at_the_treeline() {
        let mut alps = env(46.5, 8.0, 8.0, 0.10);
        alps.elevation_m = 1_600.0;
        assert_eq!(natural_tree_density_factor(alps), 1.0);
        alps.elevation_m = 2_125.0;
        let ecotone = natural_tree_density_factor(alps);
        assert!((0.45..=0.55).contains(&ecotone), "ecotone={ecotone}");
        alps.elevation_m = 2_400.0;
        assert_eq!(natural_tree_density_factor(alps), 0.0);
        alps.elevation_m = 3_000.0;
        assert_eq!(natural_tree_density_factor(alps), 0.0);
    }

    #[test]
    fn managed_groves_vary_more_but_still_have_a_visual_identity() {
        let sydney = env(-33.86, 151.21, 18.0, 0.20);
        let picks: Vec<_> = (0..256)
            .map(|seed| select_profile(sydney, true, seed, 91))
            .collect();
        let dominant = picks
            .iter()
            .map(|pick| picks.iter().filter(|other| *other == pick).count())
            .max()
            .unwrap();
        assert!(
            dominant >= 195,
            "at least three quarters of a managed grove should remain coherent: {dominant}/{}",
            picks.len()
        );
        assert!(picks.iter().any(|pick| *pick != picks[0]));
    }

    #[test]
    fn freshwater_proximity_does_not_invent_a_tidal_mangrove_forest() {
        let mut inland = env(1.3, 103.8, 27.0, 0.10);
        for seed in 0..256 {
            let ordinary = select_profile(inland, false, seed, 7);
            inland.near_water = true;
            assert_eq!(select_profile(inland, false, seed, 7), ordinary);
            assert_ne!(ordinary, RedMangrove);
            inland.near_water = false;
        }
    }

    #[test]
    fn natural_catalog_excludes_ornamentals_and_keeps_compatible_companions() {
        assert!(WOODLANDS.len() >= 50);
        let managed = [
            LondonPlane,
            HorseChestnut,
            ColumnarPoplar,
            ItalianCypress,
            Olive,
            CoconutPalm,
            DatePalm,
            FanPalm,
            RoyalPalm,
            Orchard,
            FloweringOrnamental,
            StreetColumnar,
            PlantationConifer,
            ParkRound,
        ];
        for &woodland in WOODLANDS {
            let community = woodland.community();
            assert!(!managed.contains(&community.dominant), "{woodland:?}");
            assert!(!managed.contains(&community.companion), "{woodland:?}");
        }
        for latitude in (-60..75).step_by(5) {
            for longitude in (-175..180).step_by(5) {
                let environment = env(latitude as f32, longitude as f32, 18.0, 0.2);
                let community = woodland_kind(environment).community();
                for seed in 0..16 {
                    let selected = select_profile(environment, false, seed, seed * 11);
                    assert!([community.dominant, community.companion].contains(&selected));
                    assert_eq!(
                        selected,
                        select_profile(environment, false, seed, seed * 97)
                    );
                }
            }
        }
    }

    #[test]
    fn habitats_are_continuous_across_old_stand_and_longitude_boundaries() {
        for latitude in [-45.0, 0.0, 46.5, 65.0] {
            let west = woodland_habitat(env(latitude, -180.0, 10.0, 0.2));
            let east = woodland_habitat(env(latitude, 180.0, 10.0, 0.2));
            assert!((east - west).abs() < 1e-12);
        }
        let mut changes = 0;
        let mut previous = None;
        // A 20 km transect crosses dozens of former 300 m stand cells. The
        // woodland should read as a few large connected communities.
        for step in 0..200 {
            let woodland = woodland_kind(env(48.0, 8.0 + step as f32 * 0.00135, 10.0, 0.2));
            if previous.is_some_and(|old| old != woodland) {
                changes += 1;
            }
            previous = Some(woodland);
        }
        assert!(changes <= 3, "fragmented woodland: {changes} transitions");
    }

    #[test]
    fn regional_woodlands_preserve_distinct_canopy_identities() {
        let samples = [
            (env(52.0, 8.0, 12.0, 0.2), vec![EnglishOak, EuropeanBeech]),
            (env(64.0, 110.0, 0.0, 0.2), vec![EuropeanLarch, ScotsPine]),
            (env(49.0, -123.0, 11.0, 0.2), vec![SilverFir, Cedar]),
            (
                env(-33.0, 151.0, 18.0, 0.2),
                vec![OpenEucalyptus, RiverRedGum],
            ),
            (env(-46.0, -72.0, 9.0, 0.2), vec![EuropeanBeech]),
            (env(22.0, 78.0, 25.0, 0.5), vec![TropicalDry]),
        ];
        for (environment, expected) in samples {
            for candidate in 0..256 {
                assert!(expected.contains(&select_profile(environment, false, candidate, 1)));
            }
        }
    }

    #[test]
    fn australian_water_edges_prefer_native_riparian_forms() {
        let mut sydney = env(-33.86, 151.21, 18.0, 0.20);
        sydney.near_water = true;
        let picks: HashSet<_> = (0..512)
            .map(|candidate| select_profile(sydney, false, candidate, 11))
            .collect();
        assert!(picks.contains(&RiverRedGum));
        assert!(picks.contains(&Paperbark));
        assert!(
            picks
                .iter()
                .all(|pick| matches!(pick, RiverRedGum | Paperbark))
        );
    }
}
