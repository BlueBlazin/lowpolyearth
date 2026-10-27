//! Visual building grammars, separate from construction-stock priors.
//!
//! Profiles describe compatible inferred expression, not a claim that a source
//! building is historic. Geographic place and source metadata refine admission;
//! mapped footprints, roof shapes and heights remain the envelope authority.

use crate::material;
use vearth_data::vectors::RoofShape;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OpeningShape {
    Rectangular,
    SegmentalArch,
    RoundArch,
    PointedArch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShutterKind {
    None,
    Panel,
    Louvered,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProjectionKind {
    None,
    Juliet,
    Balcony,
    Gallery,
    TimberBay,
    StoneBay,
    ScreenBay,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Roofline {
    Plain,
    Parapet,
    SteppedParapet,
    FrontGable,
    ShapedGable,
    Mansard,
    DeepEave,
    TieredHip,
    Conical,
    EyebrowDormers,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroundStorey {
    Residential,
    Shopfront,
    Arcade,
    Veranda,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Banding {
    None,
    BaseAndCrown,
    EveryFloor,
    Alternating,
    TimberFrame,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExpressionClass {
    Contemporary,
    Vernacular,
    Heritage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleColors {
    pub wall: [u8; 3],
    pub trim: [u8; 3],
    pub joinery: [u8; 3],
    pub roof: [u8; 3],
    pub glass: [u8; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StylePalette {
    pub walls: &'static [[u8; 3]],
    pub trim: [u8; 3],
    pub joinery: &'static [[u8; 3]],
    pub roof: [u8; 3],
    pub glass: [u8; 3],
}

impl StylePalette {
    pub const fn colors(self, state: u64) -> StyleColors {
        StyleColors {
            wall: self.walls[((state >> 12) as usize) % self.walls.len()],
            trim: self.trim,
            joinery: self.joinery[((state >> 18) as usize) % self.joinery.len()],
            roof: self.roof,
            glass: self.glass,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleProfile {
    pub expression: ExpressionClass,
    pub opening: OpeningShape,
    pub entrance: OpeningShape,
    pub shutters: ShutterKind,
    pub projection: ProjectionKind,
    pub roofline: Roofline,
    pub ground_storey: GroundStorey,
    pub banding: Banding,
    pub pilasters: bool,
    pub bay_width_m: f32,
    pub floor_height_m: f32,
    pub window_width_ratio: f32,
    pub window_height_ratio: f32,
    pub sill_ratio: f32,
    pub projection_m: f32,
    pub eave_m: f32,
    pub base_height_m: f32,
    pub band_width_m: f32,
    pub frame_m: f32,
    /// A scale hint for expression attenuation, never a new source height.
    pub expression_height_m: f32,
    pub palette: StylePalette,
    /// Only an inferred finish; observed source facts win.
    pub wall_material: Option<u8>,
    pub roof_material: Option<u8>,
    /// Missing source roof shape may use this prior. Decorative rooflines do
    /// not establish roof geometry: a stepped wall can hide a pitched roof.
    pub inferred_roof: Option<RoofShape>,
    /// Optional lower-wall finish, bounded by `base_height_m`.
    pub plinth_material: Option<u8>,
}

const fn palette(
    walls: &'static [[u8; 3]],
    trim: [u8; 3],
    joinery: &'static [[u8; 3]],
    roof: [u8; 3],
    glass: [u8; 3],
) -> StylePalette {
    StylePalette {
        walls,
        trim,
        joinery,
        roof,
        glass,
    }
}
const NEUTRAL: StylePalette = palette(
    &[
        [218, 215, 202],
        [192, 198, 191],
        [230, 225, 209],
        [182, 187, 186],
    ],
    [232, 227, 210],
    &[[66, 70, 66], [100, 99, 85]],
    [105, 107, 105],
    [62, 69, 68],
);
const BRICK: StylePalette = palette(
    &[
        [155, 85, 62],
        [178, 112, 82],
        [128, 83, 70],
        [185, 139, 105],
    ],
    [216, 202, 174],
    &[[65, 63, 58], [97, 108, 92]],
    [91, 91, 89],
    [64, 68, 64],
);
const TROPICAL: StylePalette = palette(
    &[
        [221, 199, 153],
        [174, 204, 190],
        [225, 183, 154],
        [194, 207, 218],
    ],
    [236, 228, 202],
    &[[69, 98, 94], [93, 110, 128]],
    [154, 109, 79],
    [65, 78, 72],
);
const ASIAN: StylePalette = palette(
    &[
        [225, 226, 217],
        [194, 204, 201],
        [218, 210, 190],
        [180, 187, 184],
    ],
    [231, 230, 215],
    &[[66, 70, 70], [93, 96, 91]],
    [80, 89, 91],
    [57, 66, 65],
);
const LIMESTONE: StylePalette = palette(
    &[
        [220, 207, 178],
        [204, 187, 156],
        [231, 220, 195],
        [186, 178, 158],
    ],
    [239, 225, 197],
    &[[65, 80, 71], [86, 88, 76]],
    [95, 98, 97],
    [64, 69, 64],
);
const WHITEWOOD: StylePalette = palette(
    &[
        [225, 220, 199],
        [210, 199, 176],
        [233, 229, 213],
        [195, 207, 192],
    ],
    [238, 230, 208],
    &[[64, 48, 37], [100, 68, 44]],
    [111, 99, 85],
    [57, 60, 54],
);
const NORDIC: StylePalette = palette(
    &[
        [157, 67, 50],
        [213, 171, 79],
        [217, 223, 211],
        [112, 145, 149],
    ],
    [236, 230, 209],
    &[[234, 225, 198], [83, 95, 88]],
    [85, 96, 98],
    [64, 74, 71],
);
const MEDITERRANEAN: StylePalette = palette(
    &[
        [231, 216, 177],
        [226, 195, 143],
        [232, 221, 196],
        [216, 167, 133],
    ],
    [242, 226, 198],
    &[[69, 100, 79], [84, 105, 115]],
    [163, 105, 73],
    [59, 73, 67],
);
const AMALFI: StylePalette = palette(
    &[
        [235, 224, 195],
        [229, 190, 145],
        [219, 158, 130],
        [220, 208, 176],
    ],
    [242, 232, 206],
    &[[70, 107, 85], [80, 106, 125]],
    [174, 133, 94],
    [66, 77, 71],
);
const BAHIA: StylePalette = palette(
    &[
        [222, 175, 84],
        [125, 174, 169],
        [208, 145, 147],
        [176, 195, 210],
    ],
    [242, 230, 196],
    &[[59, 93, 79], [81, 103, 127]],
    [156, 99, 69],
    [56, 63, 60],
);
const IBERIAN: StylePalette = palette(
    &[
        [218, 191, 145],
        [228, 216, 186],
        [150, 178, 185],
        [213, 164, 137],
    ],
    [239, 226, 192],
    &[[61, 98, 76], [72, 96, 125]],
    [151, 102, 73],
    [61, 69, 63],
);
const CREOLE: StylePalette = palette(
    &[
        [206, 170, 133],
        [141, 182, 170],
        [223, 202, 151],
        [198, 160, 162],
    ],
    [240, 229, 203],
    &[[50, 103, 86], [82, 119, 132]],
    [110, 121, 122],
    [59, 73, 66],
);
const DRYEARTH: StylePalette = palette(
    &[
        [190, 144, 102],
        [206, 168, 124],
        [174, 125, 87],
        [223, 190, 149],
    ],
    [224, 195, 153],
    &[[73, 83, 64], [92, 68, 47]],
    [158, 128, 94],
    [61, 65, 54],
);
const TUNIS: StylePalette = palette(
    &[
        [239, 237, 219],
        [228, 228, 211],
        [242, 234, 213],
        [218, 223, 208],
    ],
    [242, 232, 210],
    &[[39, 91, 136], [195, 157, 61]],
    [188, 180, 153],
    [56, 74, 78],
);
const MOROCCOBLUE: StylePalette = palette(
    &[
        [226, 231, 219],
        [151, 191, 212],
        [195, 217, 221],
        [233, 230, 210],
    ],
    [235, 236, 221],
    &[[45, 95, 143], [47, 111, 146]],
    [167, 173, 154],
    [59, 76, 79],
);
const MOROCCORED: StylePalette = palette(
    &[
        [190, 116, 85],
        [208, 139, 103],
        [218, 158, 122],
        [176, 111, 86],
    ],
    [230, 187, 146],
    &[[77, 98, 80], [101, 73, 48]],
    [165, 126, 94],
    [66, 65, 55],
);
const SWAHILI: StylePalette = palette(
    &[
        [227, 218, 190],
        [213, 201, 171],
        [236, 226, 203],
        [194, 185, 158],
    ],
    [234, 224, 198],
    &[[92, 61, 37], [67, 99, 85]],
    [149, 152, 140],
    [59, 68, 60],
);
const SANAA: StylePalette = palette(
    &[
        [159, 103, 76],
        [179, 123, 88],
        [150, 100, 73],
        [192, 140, 99],
    ],
    [241, 229, 201],
    &[[84, 67, 49], [92, 103, 91]],
    [160, 138, 108],
    [72, 76, 65],
);
const JAISALMER: StylePalette = palette(
    &[
        [197, 158, 88],
        [215, 179, 108],
        [181, 140, 74],
        [228, 194, 128],
    ],
    [234, 198, 131],
    &[[79, 61, 41], [96, 83, 54]],
    [191, 158, 103],
    [70, 67, 52],
);
const JODHPUR: StylePalette = palette(
    &[
        [115, 167, 202],
        [146, 190, 217],
        [184, 207, 218],
        [229, 222, 199],
    ],
    [231, 230, 214],
    &[[71, 98, 116], [78, 94, 82]],
    [165, 160, 142],
    [54, 70, 74],
);
const JAIPUR: StylePalette = palette(
    &[
        [208, 126, 92],
        [225, 150, 115],
        [190, 106, 78],
        [230, 171, 137],
    ],
    [246, 219, 176],
    &[[70, 81, 61], [97, 73, 51]],
    [183, 139, 100],
    [65, 71, 58],
);
const BHUTAN: StylePalette = palette(
    &[
        [225, 220, 200],
        [215, 199, 169],
        [233, 226, 207],
        [203, 199, 180],
    ],
    [179, 93, 53],
    &[[88, 49, 36], [116, 63, 39]],
    [142, 138, 120],
    [60, 64, 52],
);
const HIMALAYA: StylePalette = palette(
    &[
        [199, 171, 129],
        [176, 135, 98],
        [220, 201, 164],
        [198, 202, 187],
    ],
    [221, 198, 155],
    &[[78, 51, 36], [100, 71, 49]],
    [126, 139, 139],
    [57, 65, 58],
);
const HUIZHOU: StylePalette = palette(
    &[
        [231, 231, 216],
        [221, 225, 213],
        [237, 234, 218],
        [212, 218, 207],
    ],
    [210, 210, 194],
    &[[71, 61, 45], [80, 80, 64]],
    [72, 81, 83],
    [54, 65, 60],
);
const KYOTO: StylePalette = palette(
    &[
        [214, 207, 180],
        [179, 163, 130],
        [225, 218, 198],
        [170, 156, 131],
    ],
    [218, 200, 158],
    &[[75, 46, 32], [99, 58, 38]],
    [73, 80, 80],
    [56, 62, 54],
);
const MALAGASY: StylePalette = palette(
    &[
        [169, 89, 67],
        [189, 115, 82],
        [207, 153, 110],
        [216, 189, 148],
    ],
    [236, 215, 176],
    &[[82, 64, 43], [97, 109, 86]],
    [147, 90, 60],
    [65, 67, 55],
);
const PANELPAINT: StylePalette = palette(
    &[
        [146, 174, 178],
        [190, 117, 84],
        [219, 188, 115],
        [176, 197, 181],
    ],
    [232, 223, 192],
    &[[66, 89, 89], [101, 100, 81]],
    [137, 151, 155],
    [66, 78, 73],
);

const BASE: StyleProfile = StyleProfile {
    expression: ExpressionClass::Contemporary,
    opening: OpeningShape::Rectangular,
    entrance: OpeningShape::Rectangular,
    shutters: ShutterKind::None,
    projection: ProjectionKind::None,
    roofline: Roofline::Plain,
    ground_storey: GroundStorey::Residential,
    banding: Banding::BaseAndCrown,
    pilasters: false,
    bay_width_m: 3.1,
    floor_height_m: 3.0,
    window_width_ratio: 0.47,
    window_height_ratio: 0.55,
    sill_ratio: 0.25,
    projection_m: 0.0,
    eave_m: 0.30,
    base_height_m: 0.45,
    band_width_m: 0.12,
    frame_m: 0.08,
    expression_height_m: 120.0,
    palette: NEUTRAL,
    wall_material: None,
    roof_material: None,
    inferred_roof: None,
    plinth_material: None,
};

macro_rules! styles {
    ($($name:ident => ($key:literal, $profile:expr)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum ArchitectureStyle { $($name),+ }
        pub const ARCHITECTURE_STYLES: &[ArchitectureStyle] = &[$(ArchitectureStyle::$name),+];
        impl ArchitectureStyle {
            pub const fn key(self) -> &'static str { match self { $(Self::$name => $key),+ } }
            pub fn from_key(key: &str) -> Option<Self> { match key { $($key => Some(Self::$name),)+ _ => None } }
            pub const fn profile(self) -> StyleProfile { match self { $(Self::$name => $profile),+ } }
            pub const fn colors(self, state: u64) -> StyleColors { self.profile().palette.colors(state) }
        }
    };
}

styles! {
    Contemporary => ("contemporary", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        projection: ProjectionKind::Balcony,
        projection_m: 0.8,
        banding: Banding::EveryFloor,
        window_width_ratio: 0.55,
        ..BASE
    }),
    PlainApartment => ("plain-apartment", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        window_width_ratio: 0.62,
        floor_height_m: 2.9,
        ..BASE
    }),
    ContemporaryBrick => ("contemporary-brick", StyleProfile {
        palette: BRICK,
        wall_material: Some(material::BRICK_WALL),
        projection: ProjectionKind::Juliet,
        projection_m: 0.2,
        banding: Banding::EveryFloor,
        window_height_ratio: 0.65,
        ..BASE
    }),
    ContemporaryTropical => ("contemporary-tropical", StyleProfile {
        palette: TROPICAL,
        projection: ProjectionKind::Gallery,
        projection_m: 1.1,
        ground_storey: GroundStorey::Veranda,
        roofline: Roofline::DeepEave,
        eave_m: 0.85,
        window_width_ratio: 0.6,
        banding: Banding::EveryFloor,
        ..BASE
    }),
    ContemporaryAsian => ("contemporary-asian", StyleProfile {
        palette: ASIAN,
        projection: ProjectionKind::Balcony,
        projection_m: 0.85,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::Alternating,
        window_width_ratio: 0.62,
        window_height_ratio: 0.61,
        ..BASE
    }),
    BritishTerrace => ("british-terrace", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: BRICK,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::BRICK_WALL),
        roof_material: Some(material::SLATE_ROOF),
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        ground_storey: GroundStorey::Residential,
        bay_width_m: 2.5,
        window_width_ratio: 0.46,
        window_height_ratio: 0.67,
        sill_ratio: 0.19,
        base_height_m: 0.8,
        expression_height_m: 23.0,
        ..BASE
    }),
    ParisMansard => ("paris-mansard", StyleProfile {
        inferred_roof: Some(RoofShape::Mansard),
        palette: LIMESTONE,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::STONE_WALL),
        roof_material: Some(material::SLATE_ROOF),
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        projection: ProjectionKind::Juliet,
        projection_m: 0.38,
        roofline: Roofline::Mansard,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::Alternating,
        pilasters: true,
        bay_width_m: 2.7,
        floor_height_m: 3.3,
        window_width_ratio: 0.44,
        window_height_ratio: 0.71,
        sill_ratio: 0.12,
        base_height_m: 1.1,
        band_width_m: 0.22,
        expression_height_m: 33.0,
        ..BASE
    }),
    DutchGable => ("dutch-gable", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: BRICK,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::BRICK_WALL),
        roof_material: Some(material::TILE_ROOF),
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        roofline: Roofline::ShapedGable,
        bay_width_m: 2.05,
        window_width_ratio: 0.5,
        window_height_ratio: 0.66,
        sill_ratio: 0.19,
        band_width_m: 0.19,
        pilasters: true,
        base_height_m: 0.85,
        expression_height_m: 23.0,
        ..BASE
    }),
    HalfTimber => ("half-timber", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: WHITEWOOD,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        banding: Banding::TimberFrame,
        roofline: Roofline::FrontGable,
        shutters: ShutterKind::Panel,
        bay_width_m: 2.3,
        window_width_ratio: 0.46,
        window_height_ratio: 0.56,
        eave_m: 0.55,
        expression_height_m: 19.0,
        ..BASE
    }),
    AlpineGallery => ("alpine-gallery", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: WHITEWOOD,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::TIMBER_WALL),
        roofline: Roofline::DeepEave,
        projection: ProjectionKind::Gallery,
        projection_m: 1.25,
        eave_m: 1.1,
        shutters: ShutterKind::Panel,
        banding: Banding::TimberFrame,
        ground_storey: GroundStorey::Veranda,
        bay_width_m: 2.6,
        base_height_m: 1.5,
        expression_height_m: 18.0,
        ..BASE
    }),
    NordicClapboard => ("nordic-clapboard", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: NORDIC,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::SIDING_WALL),
        roofline: Roofline::FrontGable,
        projection: ProjectionKind::Juliet,
        projection_m: 0.18,
        shutters: ShutterKind::Panel,
        bay_width_m: 2.45,
        window_width_ratio: 0.48,
        window_height_ratio: 0.6,
        frame_m: 0.11,
        expression_height_m: 20.0,
        ..BASE
    }),
    BalkanOriel => ("balkan-oriel", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: WHITEWOOD,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        projection: ProjectionKind::TimberBay,
        projection_m: 0.85,
        roofline: Roofline::DeepEave,
        eave_m: 0.85,
        banding: Banding::TimberFrame,
        window_width_ratio: 0.6,
        window_height_ratio: 0.65,
        base_height_m: 1.15,
        expression_height_m: 19.0,
        ..BASE
    }),
    MediterraneanShutter => ("mediterranean-shutter", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: MEDITERRANEAN,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        shutters: ShutterKind::Louvered,
        projection: ProjectionKind::Juliet,
        projection_m: 0.34,
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        bay_width_m: 2.7,
        window_width_ratio: 0.43,
        window_height_ratio: 0.68,
        sill_ratio: 0.17,
        base_height_m: 0.6,
        expression_height_m: 26.0,
        ..BASE
    }),
    AmalfiTerrace => ("amalfi-terrace", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: AMALFI,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        shutters: ShutterKind::Louvered,
        projection: ProjectionKind::Gallery,
        projection_m: 1.2,
        opening: OpeningShape::RoundArch,
        entrance: OpeningShape::RoundArch,
        roofline: Roofline::Parapet,
        ground_storey: GroundStorey::Arcade,
        bay_width_m: 3.2,
        window_width_ratio: 0.5,
        window_height_ratio: 0.7,
        sill_ratio: 0.13,
        base_height_m: 0.6,
        band_width_m: 0.2,
        expression_height_m: 24.0,
        ..BASE
    }),
    IberianTile => ("iberian-tile", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: IBERIAN,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::Juliet,
        projection_m: 0.36,
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::EveryFloor,
        pilasters: true,
        bay_width_m: 2.65,
        window_width_ratio: 0.43,
        window_height_ratio: 0.73,
        sill_ratio: 0.12,
        expression_height_m: 26.0,
        ..BASE
    }),
    BahiaSobrado => ("bahia-sobrado", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: BAHIA,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        shutters: ShutterKind::Louvered,
        projection: ProjectionKind::Juliet,
        projection_m: 0.5,
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::EveryFloor,
        pilasters: true,
        bay_width_m: 2.45,
        floor_height_m: 3.25,
        window_width_ratio: 0.47,
        window_height_ratio: 0.74,
        sill_ratio: 0.1,
        frame_m: 0.14,
        base_height_m: 0.85,
        band_width_m: 0.21,
        expression_height_m: 21.0,
        ..BASE
    }),
    HispanicPatio => ("hispanic-patio", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: IBERIAN,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::Juliet,
        projection_m: 0.25,
        opening: OpeningShape::RoundArch,
        entrance: OpeningShape::RoundArch,
        roofline: Roofline::Parapet,
        ground_storey: GroundStorey::Arcade,
        bay_width_m: 3.1,
        window_width_ratio: 0.4,
        window_height_ratio: 0.65,
        sill_ratio: 0.18,
        base_height_m: 0.85,
        frame_m: 0.15,
        expression_height_m: 20.0,
        ..BASE
    }),
    CaribbeanGallery => ("caribbean-gallery", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: CREOLE,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::SIDING_WALL),
        roof_material: Some(material::METAL_ROOF),
        shutters: ShutterKind::Louvered,
        projection: ProjectionKind::Gallery,
        projection_m: 1.55,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Veranda,
        banding: Banding::EveryFloor,
        bay_width_m: 2.85,
        window_width_ratio: 0.48,
        window_height_ratio: 0.74,
        sill_ratio: 0.1,
        eave_m: 0.9,
        base_height_m: 0.85,
        expression_height_m: 20.0,
        ..BASE
    }),
    WillemstadGable => ("willemstad-gable", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: BAHIA,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::Gallery,
        projection_m: 0.85,
        roofline: Roofline::ShapedGable,
        ground_storey: GroundStorey::Shopfront,
        pilasters: true,
        bay_width_m: 2.4,
        window_width_ratio: 0.48,
        window_height_ratio: 0.68,
        sill_ratio: 0.16,
        frame_m: 0.13,
        band_width_m: 0.2,
        expression_height_m: 23.0,
        ..BASE
    }),
    NorthAmericanPorch => ("north-american-porch", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: CREOLE,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::SIDING_WALL),
        roof_material: Some(material::SHINGLE_ROOF),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::Gallery,
        projection_m: 1.4,
        roofline: Roofline::FrontGable,
        ground_storey: GroundStorey::Veranda,
        bay_width_m: 3.25,
        window_width_ratio: 0.45,
        window_height_ratio: 0.64,
        sill_ratio: 0.19,
        base_height_m: 0.9,
        expression_height_m: 17.0,
        ..BASE
    }),
    NorthAmericanBay => ("north-american-bay", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: LIMESTONE,
        expression: ExpressionClass::Heritage,
        projection: ProjectionKind::StoneBay,
        projection_m: 0.95,
        roofline: Roofline::FrontGable,
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        banding: Banding::EveryFloor,
        bay_width_m: 2.7,
        window_width_ratio: 0.48,
        window_height_ratio: 0.7,
        sill_ratio: 0.14,
        base_height_m: 1.1,
        expression_height_m: 24.0,
        ..BASE
    }),
    PuebloTerrace => ("pueblo-terrace", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: DRYEARTH,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        projection: ProjectionKind::TimberBay,
        projection_m: 0.4,
        roofline: Roofline::SteppedParapet,
        shutters: ShutterKind::Panel,
        banding: Banding::None,
        bay_width_m: 3.4,
        window_width_ratio: 0.32,
        window_height_ratio: 0.45,
        sill_ratio: 0.3,
        eave_m: 0.15,
        frame_m: 0.15,
        base_height_m: 0.5,
        expression_height_m: 16.0,
        ..BASE
    }),
    AndeanGallery => ("andean-gallery", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: WHITEWOOD,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        projection: ProjectionKind::TimberBay,
        projection_m: 0.85,
        shutters: ShutterKind::Panel,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Arcade,
        bay_width_m: 2.7,
        window_width_ratio: 0.43,
        window_height_ratio: 0.6,
        eave_m: 0.65,
        base_height_m: 1.2,
        expression_height_m: 20.0,
        ..BASE
    }),
    ChileanCladding => ("chilean-cladding", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: PANELPAINT,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::SIDING_WALL),
        roof_material: Some(material::METAL_ROOF),
        projection: ProjectionKind::TimberBay,
        projection_m: 0.55,
        roofline: Roofline::FrontGable,
        bay_width_m: 2.6,
        window_width_ratio: 0.48,
        window_height_ratio: 0.61,
        base_height_m: 0.9,
        expression_height_m: 19.0,
        ..BASE
    }),
    TunisianMedina => ("tunisian-medina", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: TUNIS,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::ScreenBay,
        projection_m: 0.35,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        roofline: Roofline::Parapet,
        bay_width_m: 3.75,
        window_width_ratio: 0.34,
        window_height_ratio: 0.53,
        sill_ratio: 0.25,
        frame_m: 0.15,
        band_width_m: 0.16,
        expression_height_m: 21.0,
        ..BASE
    }),
    CycladicHouse => ("cycladic-house", StyleProfile {
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        inferred_roof: Some(RoofShape::Flat),
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        roofline: Roofline::Parapet,
        projection: ProjectionKind::None,
        banding: Banding::None,
        pilasters: false,
        ground_storey: GroundStorey::Residential,
        eave_m: 0.0,
        frame_m: 0.08,
        bay_width_m: 3.0,
        floor_height_m: 2.9,
        window_width_ratio: 0.32,
        window_height_ratio: 0.48,
        expression_height_m: 12.0,
        ..BASE
    }),
    MoroccanWhiteBlue => ("moroccan-white-blue", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: MOROCCOBLUE,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::Juliet,
        projection_m: 0.28,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        roofline: Roofline::Parapet,
        bay_width_m: 3.2,
        window_width_ratio: 0.34,
        window_height_ratio: 0.55,
        sill_ratio: 0.24,
        frame_m: 0.14,
        expression_height_m: 20.0,
        ..BASE
    }),
    MoroccanRedMedina => ("moroccan-red-medina", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: MOROCCORED,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::ScreenBay,
        projection_m: 0.42,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::PointedArch,
        roofline: Roofline::Parapet,
        bay_width_m: 3.65,
        window_width_ratio: 0.31,
        window_height_ratio: 0.56,
        sill_ratio: 0.23,
        banding: Banding::None,
        frame_m: 0.16,
        expression_height_m: 22.0,
        ..BASE
    }),
    AtlasKasbah => ("atlas-kasbah", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: DRYEARTH,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        opening: OpeningShape::Rectangular,
        roofline: Roofline::SteppedParapet,
        banding: Banding::Alternating,
        pilasters: true,
        bay_width_m: 3.4,
        window_width_ratio: 0.25,
        window_height_ratio: 0.44,
        sill_ratio: 0.34,
        frame_m: 0.13,
        band_width_m: 0.22,
        base_height_m: 0.9,
        expression_height_m: 24.0,
        ..BASE
    }),
    SahelRelief => ("sahel-relief", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: DRYEARTH,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::SegmentalArch,
        roofline: Roofline::SteppedParapet,
        banding: Banding::BaseAndCrown,
        pilasters: true,
        bay_width_m: 3.3,
        window_width_ratio: 0.28,
        window_height_ratio: 0.49,
        sill_ratio: 0.3,
        band_width_m: 0.24,
        frame_m: 0.17,
        expression_height_m: 15.0,
        ..BASE
    }),
    WestAfricanVeranda => ("west-african-veranda", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: TROPICAL,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::METAL_ROOF),
        shutters: ShutterKind::Louvered,
        projection: ProjectionKind::Gallery,
        projection_m: 1.25,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Veranda,
        bay_width_m: 3.3,
        window_width_ratio: 0.52,
        window_height_ratio: 0.64,
        sill_ratio: 0.17,
        eave_m: 0.8,
        base_height_m: 0.7,
        expression_height_m: 23.0,
        ..BASE
    }),
    SwahiliStoneTown => ("swahili-stone-town", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: SWAHILI,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        shutters: ShutterKind::Louvered,
        projection: ProjectionKind::TimberBay,
        projection_m: 0.75,
        opening: OpeningShape::RoundArch,
        entrance: OpeningShape::RoundArch,
        roofline: Roofline::Parapet,
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 3.1,
        window_width_ratio: 0.46,
        window_height_ratio: 0.69,
        sill_ratio: 0.15,
        frame_m: 0.18,
        base_height_m: 0.9,
        expression_height_m: 23.0,
        ..BASE
    }),
    CapeGable => ("cape-gable", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: TUNIS,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::Gallery,
        projection_m: 0.8,
        roofline: Roofline::ShapedGable,
        ground_storey: GroundStorey::Veranda,
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        bay_width_m: 2.8,
        window_width_ratio: 0.46,
        window_height_ratio: 0.64,
        band_width_m: 0.22,
        frame_m: 0.12,
        expression_height_m: 17.0,
        ..BASE
    }),
    MalagasyVeranda => ("malagasy-veranda", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: MALAGASY,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::BRICK_WALL),
        roof_material: Some(material::TILE_ROOF),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::Gallery,
        projection_m: 1.05,
        roofline: Roofline::FrontGable,
        ground_storey: GroundStorey::Veranda,
        banding: Banding::EveryFloor,
        bay_width_m: 2.6,
        window_width_ratio: 0.46,
        window_height_ratio: 0.68,
        sill_ratio: 0.17,
        expression_height_m: 18.0,
        ..BASE
    }),
    AsmaraModernist => ("asmara-modernist", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: TROPICAL,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        projection: ProjectionKind::Balcony,
        projection_m: 0.9,
        roofline: Roofline::SteppedParapet,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::EveryFloor,
        bay_width_m: 3.6,
        window_width_ratio: 0.7,
        window_height_ratio: 0.42,
        sill_ratio: 0.32,
        band_width_m: 0.24,
        frame_m: 0.06,
        expression_height_m: 29.0,
        ..BASE
    }),
    HejaziRoshan => ("hejazi-roshan", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: SWAHILI,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        projection: ProjectionKind::ScreenBay,
        projection_m: 1.05,
        roofline: Roofline::Parapet,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::EveryFloor,
        bay_width_m: 3.0,
        floor_height_m: 3.2,
        window_width_ratio: 0.68,
        window_height_ratio: 0.73,
        sill_ratio: 0.11,
        frame_m: 0.12,
        base_height_m: 0.8,
        expression_height_m: 31.0,
        ..BASE
    }),
    SanaaniTower => ("sanaani-tower", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: SANAA,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::BRICK_WALL),
        projection: ProjectionKind::None,
        opening: OpeningShape::RoundArch,
        entrance: OpeningShape::RoundArch,
        roofline: Roofline::SteppedParapet,
        banding: Banding::EveryFloor,
        bay_width_m: 2.55,
        floor_height_m: 3.0,
        window_width_ratio: 0.38,
        window_height_ratio: 0.64,
        sill_ratio: 0.19,
        band_width_m: 0.22,
        frame_m: 0.14,
        base_height_m: 1.3,
        expression_height_m: 31.0,
        ..BASE
    }),
    PersianCourtyard => ("persian-courtyard", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: DRYEARTH,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::BRICK_WALL),
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::PointedArch,
        projection: ProjectionKind::Gallery,
        projection_m: 0.9,
        roofline: Roofline::Parapet,
        ground_storey: GroundStorey::Arcade,
        banding: Banding::BaseAndCrown,
        bay_width_m: 3.2,
        window_width_ratio: 0.36,
        window_height_ratio: 0.64,
        sill_ratio: 0.19,
        frame_m: 0.17,
        expression_height_m: 22.0,
        ..BASE
    }),
    JaisalmerHaveli => ("jaisalmer-haveli", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: JAISALMER,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::STONE_WALL),
        opening: OpeningShape::PointedArch,
        entrance: OpeningShape::PointedArch,
        projection: ProjectionKind::StoneBay,
        projection_m: 0.85,
        roofline: Roofline::SteppedParapet,
        banding: Banding::EveryFloor,
        pilasters: true,
        bay_width_m: 2.5,
        floor_height_m: 3.15,
        window_width_ratio: 0.46,
        window_height_ratio: 0.65,
        sill_ratio: 0.17,
        eave_m: 0.65,
        base_height_m: 1.0,
        frame_m: 0.15,
        band_width_m: 0.2,
        expression_height_m: 25.0,
        ..BASE
    }),
    JodhpurHaveli => ("jodhpur-haveli", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: JODHPUR,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        projection: ProjectionKind::StoneBay,
        projection_m: 0.55,
        roofline: Roofline::Parapet,
        shutters: ShutterKind::Panel,
        bay_width_m: 2.9,
        window_width_ratio: 0.36,
        window_height_ratio: 0.59,
        sill_ratio: 0.23,
        frame_m: 0.14,
        expression_height_m: 24.0,
        ..BASE
    }),
    JaipurBazaar => ("jaipur-bazaar", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: JAIPUR,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        opening: OpeningShape::RoundArch,
        entrance: OpeningShape::RoundArch,
        projection: ProjectionKind::Juliet,
        projection_m: 0.45,
        roofline: Roofline::SteppedParapet,
        ground_storey: GroundStorey::Arcade,
        banding: Banding::EveryFloor,
        pilasters: true,
        bay_width_m: 2.85,
        floor_height_m: 3.25,
        window_width_ratio: 0.52,
        window_height_ratio: 0.66,
        sill_ratio: 0.15,
        eave_m: 0.6,
        frame_m: 0.14,
        band_width_m: 0.24,
        base_height_m: 0.7,
        expression_height_m: 23.0,
        ..BASE
    }),
    GujaratiPol => ("gujarati-pol", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: HIMALAYA,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        projection: ProjectionKind::TimberBay,
        projection_m: 0.72,
        shutters: ShutterKind::Panel,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Veranda,
        banding: Banding::TimberFrame,
        bay_width_m: 2.3,
        window_width_ratio: 0.55,
        window_height_ratio: 0.65,
        sill_ratio: 0.16,
        eave_m: 0.65,
        base_height_m: 0.8,
        frame_m: 0.12,
        expression_height_m: 22.0,
        ..BASE
    }),
    KeralaVeranda => ("kerala-veranda", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: WHITEWOOD,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::Gallery,
        projection_m: 1.5,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Veranda,
        banding: Banding::TimberFrame,
        bay_width_m: 2.9,
        window_width_ratio: 0.43,
        window_height_ratio: 0.55,
        eave_m: 1.2,
        base_height_m: 0.9,
        expression_height_m: 18.0,
        ..BASE
    }),
    BhutaneseRabsel => ("bhutanese-rabsel", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: BHUTAN,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        projection: ProjectionKind::TimberBay,
        projection_m: 0.7,
        roofline: Roofline::DeepEave,
        banding: Banding::TimberFrame,
        bay_width_m: 2.65,
        window_width_ratio: 0.65,
        window_height_ratio: 0.61,
        sill_ratio: 0.2,
        eave_m: 1.1,
        band_width_m: 0.24,
        base_height_m: 1.2,
        frame_m: 0.14,
        expression_height_m: 25.0,
        ..BASE
    }),
    HimalayanTimber => ("himalayan-timber", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: HIMALAYA,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::BRICK_WALL),
        roof_material: Some(material::METAL_ROOF),
        projection: ProjectionKind::TimberBay,
        projection_m: 0.48,
        shutters: ShutterKind::Panel,
        roofline: Roofline::DeepEave,
        banding: Banding::TimberFrame,
        bay_width_m: 2.5,
        window_width_ratio: 0.52,
        window_height_ratio: 0.56,
        eave_m: 0.7,
        base_height_m: 1.0,
        expression_height_m: 21.0,
        ..BASE
    }),
    HuizhouGable => ("huizhou-gable", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: HUIZHOU,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        opening: OpeningShape::Rectangular,
        roofline: Roofline::SteppedParapet,
        shutters: ShutterKind::Panel,
        banding: Banding::BaseAndCrown,
        bay_width_m: 3.25,
        window_width_ratio: 0.32,
        window_height_ratio: 0.48,
        sill_ratio: 0.31,
        eave_m: 0.65,
        frame_m: 0.13,
        expression_height_m: 18.0,
        ..BASE
    }),
    KyotoMachiya => ("kyoto-machiya", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: KYOTO,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::TIMBER_WALL),
        roof_material: Some(material::TILE_ROOF),
        projection: ProjectionKind::ScreenBay,
        projection_m: 0.32,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::TimberFrame,
        bay_width_m: 2.4,
        floor_height_m: 2.8,
        window_width_ratio: 0.72,
        window_height_ratio: 0.47,
        sill_ratio: 0.28,
        eave_m: 0.75,
        base_height_m: 0.55,
        expression_height_m: 15.0,
        ..BASE
    }),
    KoreanCourtyard => ("korean-courtyard", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: WHITEWOOD,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        projection: ProjectionKind::Gallery,
        projection_m: 0.85,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Veranda,
        banding: Banding::TimberFrame,
        bay_width_m: 2.8,
        window_width_ratio: 0.58,
        window_height_ratio: 0.58,
        eave_m: 0.95,
        base_height_m: 0.7,
        expression_height_m: 13.0,
        ..BASE
    }),
    StraitsShophouse => ("straits-shophouse", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        palette: BAHIA,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::SegmentalArch,
        shutters: ShutterKind::Louvered,
        projection: ProjectionKind::Juliet,
        projection_m: 0.32,
        roofline: Roofline::ShapedGable,
        ground_storey: GroundStorey::Arcade,
        banding: Banding::EveryFloor,
        pilasters: true,
        bay_width_m: 2.5,
        floor_height_m: 3.5,
        window_width_ratio: 0.56,
        window_height_ratio: 0.73,
        sill_ratio: 0.1,
        frame_m: 0.13,
        band_width_m: 0.23,
        expression_height_m: 21.0,
        ..BASE
    }),
    VietnameseTube => ("vietnamese-tube", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        palette: TROPICAL,
        expression: ExpressionClass::Contemporary,
        wall_material: Some(material::PLASTER_WALL),
        projection: ProjectionKind::Balcony,
        projection_m: 0.95,
        roofline: Roofline::Parapet,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::EveryFloor,
        pilasters: true,
        bay_width_m: 2.05,
        floor_height_m: 3.2,
        window_width_ratio: 0.65,
        window_height_ratio: 0.69,
        sill_ratio: 0.13,
        base_height_m: 0.4,
        band_width_m: 0.2,
        expression_height_m: 29.0,
        ..BASE
    }),
    MalayVeranda => ("malay-veranda", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: WHITEWOOD,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::SIDING_WALL),
        roof_material: Some(material::TILE_ROOF),
        projection: ProjectionKind::Gallery,
        projection_m: 1.4,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Veranda,
        shutters: ShutterKind::Louvered,
        banding: Banding::TimberFrame,
        bay_width_m: 2.8,
        window_width_ratio: 0.48,
        window_height_ratio: 0.65,
        eave_m: 1.0,
        base_height_m: 0.9,
        expression_height_m: 16.0,
        ..BASE
    }),
    IndonesianTiered => ("indonesian-tiered", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: MALAGASY,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        projection: ProjectionKind::Gallery,
        projection_m: 1.35,
        roofline: Roofline::TieredHip,
        ground_storey: GroundStorey::Veranda,
        shutters: ShutterKind::Panel,
        banding: Banding::TimberFrame,
        bay_width_m: 3.2,
        window_width_ratio: 0.45,
        window_height_ratio: 0.54,
        eave_m: 1.25,
        base_height_m: 0.7,
        expression_height_m: 16.0,
        ..BASE
    }),
    PhilippineBahay => ("philippine-bahay", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: WHITEWOOD,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::TIMBER_WALL),
        roof_material: Some(material::METAL_ROOF),
        projection: ProjectionKind::TimberBay,
        projection_m: 0.6,
        roofline: Roofline::DeepEave,
        shutters: ShutterKind::Panel,
        banding: Banding::TimberFrame,
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 2.25,
        window_width_ratio: 0.75,
        window_height_ratio: 0.7,
        sill_ratio: 0.15,
        eave_m: 0.9,
        base_height_m: 2.6,
        expression_height_m: 17.0,
        ..BASE
    }),
    Queenslander => ("queenslander", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: CREOLE,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::SIDING_WALL),
        roof_material: Some(material::METAL_ROOF),
        projection: ProjectionKind::Gallery,
        projection_m: 1.8,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Veranda,
        shutters: ShutterKind::Louvered,
        banding: Banding::TimberFrame,
        bay_width_m: 2.8,
        window_width_ratio: 0.49,
        window_height_ratio: 0.7,
        sill_ratio: 0.15,
        eave_m: 1.1,
        base_height_m: 1.3,
        expression_height_m: 16.0,
        ..BASE
    }),
    AustralasianCottage => ("australasian-cottage", StyleProfile {
        palette: WHITEWOOD,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::SIDING_WALL),
        roof_material: Some(material::METAL_ROOF),
        inferred_roof: Some(RoofShape::Hipped),
        projection: ProjectionKind::Gallery,
        projection_m: 1.1,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Veranda,
        banding: Banding::BaseAndCrown,
        bay_width_m: 2.8,
        window_width_ratio: 0.46,
        window_height_ratio: 0.65,
        eave_m: 0.75,
        base_height_m: 0.7,
        expression_height_m: 15.0,
        ..BASE
    }),
    ChineseCourtyard => ("chinese-courtyard", StyleProfile {
        palette: StylePalette {
            walls: &[[158,153,136], [178,174,155], [206,203,179], [142,146,136]],
            trim: [202,194,170], joinery: &[[81,65,48], [105,67,49]],
            roof: [87,91,86], glass: [53,60,56],
        },
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::BRICK_WALL),
        roof_material: Some(material::TILE_ROOF),
        inferred_roof: Some(RoofShape::Gabled),
        roofline: Roofline::DeepEave,
        shutters: ShutterKind::Panel,
        banding: Banding::TimberFrame,
        bay_width_m: 3.25,
        window_width_ratio: 0.37,
        window_height_ratio: 0.5,
        sill_ratio: 0.27,
        eave_m: 0.7,
        base_height_m: 0.7,
        expression_height_m: 15.0,
        ..BASE
    }),
    HoiAnMerchant => ("hoi-an-merchant", StyleProfile {
        palette: StylePalette {
            walls: &[[221,187,105], [229,201,134], [205,178,114], [224,213,172]],
            trim: [214,203,171], joinery: &[[74,58,42], [91,67,43]],
            roof: [126,104,78], glass: [52,62,57],
        },
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        inferred_roof: Some(RoofShape::Gabled),
        roofline: Roofline::DeepEave,
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::Gallery,
        projection_m: 0.65,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::TimberFrame,
        bay_width_m: 2.35,
        floor_height_m: 2.9,
        window_width_ratio: 0.63,
        window_height_ratio: 0.62,
        sill_ratio: 0.2,
        eave_m: 0.8,
        expression_height_m: 15.0,
        ..BASE
    }),
    SidiBouSaid => ("sidi-bou-said", StyleProfile {
        palette: StylePalette {
            walls: &[[242,241,226], [232,233,221], [239,237,222], [228,232,218]],
            trim: [235,235,219], joinery: &[[35,93,154], [42,111,169]],
            roof: [184,183,160], glass: [54,71,77],
        },
        projection: ProjectionKind::ScreenBay,
        projection_m: 0.42,
        inferred_roof: Some(RoofShape::Flat),
        ..ArchitectureStyle::TunisianMedina.profile()
    }),
    TetouanMedina => ("tetouan-medina", StyleProfile {
        palette: StylePalette {
            walls: &[[239,236,216], [230,230,213], [242,238,218], [220,223,207]],
            trim: [239,232,212], joinery: &[[55,103,78], [50,117,89]],
            roof: [131,145,112], glass: [54,71,63],
        },
        projection: ProjectionKind::Juliet,
        projection_m: 0.24,
        inferred_roof: Some(RoofShape::Flat),
        ..ArchitectureStyle::TunisianMedina.profile()
    }),
    ClassicalLimestone => ("classical-limestone", StyleProfile {
        palette: LIMESTONE,
        expression: ExpressionClass::Heritage,
        wall_material: Some(material::STONE_WALL),
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::RoundArch,
        projection: ProjectionKind::Juliet,
        projection_m: 0.3,
        ground_storey: GroundStorey::Shopfront,
        banding: Banding::EveryFloor,
        pilasters: true,
        bay_width_m: 2.75,
        floor_height_m: 3.25,
        window_width_ratio: 0.43,
        window_height_ratio: 0.71,
        sill_ratio: 0.13,
        base_height_m: 0.9,
        band_width_m: 0.2,
        frame_m: 0.13,
        expression_height_m: 32.0,
        ..BASE
    }),
    CornicedApartment => ("corniced-apartment", StyleProfile {
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        projection: ProjectionKind::None,
        pilasters: false,
        floor_height_m: 3.3,
        bay_width_m: 2.9,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    RectangularCornicedApartment => ("rectangular-corniced-apartment", StyleProfile {
        entrance: OpeningShape::Rectangular,
        ..ArchitectureStyle::CornicedApartment.profile()
    }),
    BerlinMietshaus => ("berlin-mietshaus", StyleProfile {
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        wall_material: Some(material::PLASTER_WALL),
        projection: ProjectionKind::Juliet,
        projection_m: 0.35,
        pilasters: false,
        floor_height_m: 3.2,
        bay_width_m: 2.8,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    MadridCasaDePisos => ("madrid-casa-de-pisos", StyleProfile {
        opening: OpeningShape::Rectangular,
        wall_material: Some(material::PLASTER_WALL),
        projection: ProjectionKind::Juliet,
        projection_m: 0.4,
        pilasters: false,
        floor_height_m: 3.0,
        bay_width_m: 2.7,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    MalteseGallarija => ("maltese-gallarija", StyleProfile {
        palette: StylePalette {
            walls: &[[218,201,159], [230,215,176], [204,188,150], [234,220,183]],
            trim: [231,217,176], joinery: &[[46,101,78], [64,88,119]],
            roof: [189,176,143], glass: [54,68,65],
        },
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::STONE_WALL),
        inferred_roof: Some(RoofShape::Flat),
        entrance: OpeningShape::SegmentalArch,
        projection: ProjectionKind::TimberBay,
        projection_m: 0.8,
        roofline: Roofline::Parapet,
        shutters: ShutterKind::Panel,
        banding: Banding::EveryFloor,
        bay_width_m: 3.1,
        floor_height_m: 3.2,
        window_width_ratio: 0.64,
        window_height_ratio: 0.65,
        sill_ratio: 0.19,
        base_height_m: 0.75,
        band_width_m: 0.18,
        frame_m: 0.13,
        expression_height_m: 27.0,
        ..BASE
    }),
    LusoWhitewash => ("luso-whitewash", StyleProfile {
        entrance: OpeningShape::Rectangular,
        palette: StylePalette {
            walls: &[[238,235,217], [230,229,211], [243,238,219], [222,225,209]],
            trim: [230,226,205], joinery: &[[40,109,106], [70,110,166]],
            roof: [150,99,68], glass: [52,66,63],
        },
        opening: OpeningShape::Rectangular,
        ground_storey: GroundStorey::Residential,
        pilasters: false,
        ..ArchitectureStyle::BahiaSobrado.profile()
    }),
    PacificVeranda => ("pacific-veranda", StyleProfile {
        inferred_roof: Some(RoofShape::Hipped),
        palette: PANELPAINT,
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::SIDING_WALL),
        roof_material: Some(material::METAL_ROOF),
        projection: ProjectionKind::Gallery,
        projection_m: 1.35,
        roofline: Roofline::DeepEave,
        ground_storey: GroundStorey::Veranda,
        shutters: ShutterKind::Louvered,
        banding: Banding::BaseAndCrown,
        bay_width_m: 3.1,
        window_width_ratio: 0.57,
        window_height_ratio: 0.57,
        eave_m: 1.1,
        base_height_m: 0.8,
        expression_height_m: 14.0,
        ..BASE
    }),
    PlainJulietRow => ("plain-juliet-row", StyleProfile {
        expression: ExpressionClass::Vernacular,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::Rectangular,
        shutters: ShutterKind::None,
        projection: ProjectionKind::Juliet,
        projection_m: 0.18,
        roofline: Roofline::Parapet,
        ground_storey: GroundStorey::Residential,
        banding: Banding::BaseAndCrown,
        bay_width_m: 2.8,
        floor_height_m: 3.1,
        window_width_ratio: 0.45,
        window_height_ratio: 0.68,
        sill_ratio: 0.15,
        frame_m: 0.14,
        expression_height_m: 22.0,
        ..BASE
    }),
    MediterraneanUrbanRow => ("mediterranean-urban-row", StyleProfile {
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        bay_width_m: 2.8,
        ground_storey: GroundStorey::Residential,
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    VeniceLagoon => ("venice-lagoon", StyleProfile {
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        projection: ProjectionKind::None,
        projection_m: 0.0,
        bay_width_m: 2.8,
        ground_storey: GroundStorey::Residential,
        palette: StylePalette { walls: &[[202,157,133],[219,189,164],[174,113,96],[222,209,183]], ..MEDITERRANEAN },
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    LisbonAlfama => ("lisbon-alfama", StyleProfile {
        opening: OpeningShape::Rectangular,
        bay_width_m: 2.4,
        pilasters: false,
        ground_storey: GroundStorey::Residential,
        palette: StylePalette { walls: &[[224,220,198],[198,214,211],[218,190,152],[196,207,218]], ..IBERIAN },
        ..ArchitectureStyle::IberianTile.profile()
    }),
    BarcelonaEixample => ("barcelona-eixample", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        projection: ProjectionKind::Balcony,
        projection_m: 0.65,
        opening: OpeningShape::Rectangular,
        eave_m: 0.05,
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 3.2,
        floor_height_m: 3.5,
        expression_height_m: 32.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    IstanbulZeyrek => ("istanbul-zeyrek", StyleProfile {
        wall_material: Some(material::TIMBER_WALL),
        projection_m: 0.65,
        bay_width_m: 2.7,
        palette: StylePalette { walls: &[[175,137,102],[210,189,156],[150,125,101],[198,174,140]], ..HIMALAYA },
        ..ArchitectureStyle::BalkanOriel.profile()
    }),
    FezMedina => ("fez-medina", StyleProfile {
        palette: DRYEARTH,
        shutters: ShutterKind::Panel,
        window_width_ratio: 0.32,
        projection: ProjectionKind::None,
        ground_storey: GroundStorey::Residential,
        ..ArchitectureStyle::TunisianMedina.profile()
    }),
    TrinidadHouse => ("trinidad-house", StyleProfile {
        entrance: OpeningShape::Rectangular,
        opening: OpeningShape::Rectangular,
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::None,
        ground_storey: GroundStorey::Residential,
        pilasters: false,
        ..ArchitectureStyle::LusoWhitewash.profile()
    }),
    HavanaVieja => ("havana-vieja", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        roof_material: Some(material::TILE_ROOF),
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        ground_storey: GroundStorey::Residential,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        projection: ProjectionKind::Balcony,
        eave_m: 0.2,
        floor_height_m: 3.8,
        projection_m: 0.65,
        expression_height_m: 26.0,
        ..ArchitectureStyle::CaribbeanGallery.profile()
    }),
    CartagenaBalconies => ("cartagena-balconies", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        ground_storey: GroundStorey::Residential,
        shutters: ShutterKind::Panel,
        projection_m: 0.9,
        bay_width_m: 2.8,
        floor_height_m: 3.5,
        roof_material: Some(material::TILE_ROOF),
        inferred_roof: Some(RoofShape::Gabled),
        ..ArchitectureStyle::CaribbeanGallery.profile()
    }),
    BeijingHutong => ("beijing-hutong", StyleProfile {
        ground_storey: GroundStorey::Residential,
        bay_width_m: 3.0,
        window_width_ratio: 0.32,
        expression_height_m: 12.0,
        palette: StylePalette { walls: &[[142,146,139],[167,169,154],[185,181,160],[153,151,140]], ..ASIAN },
        ..ArchitectureStyle::ChineseCourtyard.profile()
    }),
    LuangPrabangPeninsula => ("luang-prabang-peninsula", StyleProfile {
        projection_m: 0.8,
        bay_width_m: 3.1,
        roof_material: Some(material::TILE_ROOF),
        palette: StylePalette { walls: &[[210,193,153],[163,135,96],[224,217,190],[185,169,133]], ..KYOTO },
        ..ArchitectureStyle::MalayVeranda.profile()
    }),
    SrinagarJhelum => ("srinagar-jhelum", StyleProfile {
        wall_material: Some(material::BRICK_WALL),
        projection: ProjectionKind::TimberBay,
        projection_m: 0.6,
        banding: Banding::TimberFrame,
        palette: StylePalette { walls: &[[161,114,88],[185,147,117],[205,187,157],[166,141,111]], ..HIMALAYA },
        ..ArchitectureStyle::HimalayanTimber.profile()
    }),
    LondonPimlico => ("london-pimlico", StyleProfile {
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        wall_material: Some(material::PLASTER_WALL),
        palette: LIMESTONE,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        projection: ProjectionKind::Juliet,
        projection_m: 0.22,
        banding: Banding::EveryFloor,
        ..ArchitectureStyle::BritishTerrace.profile()
    }),
    EdinburghTenement => ("edinburgh-tenement", StyleProfile {
        wall_material: Some(material::STONE_WALL),
        palette: StylePalette { walls: &[[164,155,133],[183,176,155],[150,147,132],[199,190,164]], ..LIMESTONE },
        bay_width_m: 2.6,
        opening: OpeningShape::Rectangular,
        expression_height_m: 32.0,
        ..ArchitectureStyle::BritishTerrace.profile()
    }),
    BathGeorgian => ("bath-georgian", StyleProfile {
        ground_storey: GroundStorey::Residential,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        bay_width_m: 2.6,
        window_width_ratio: 0.43,
        inferred_roof: Some(RoofShape::Gabled),
        roof_material: Some(material::SLATE_ROOF),
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    GeorgianTerrace => ("georgian-terrace", StyleProfile {
        projection: ProjectionKind::None,
        projection_m: 0.0,
        pilasters: false,
        banding: Banding::BaseAndCrown,
        ..ArchitectureStyle::BathGeorgian.profile()
    }),
    DublinGeorgian => ("dublin-georgian", StyleProfile {
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        bay_width_m: 2.8,
        floor_height_m: 3.3,
        palette: StylePalette { walls: &[[153,92,70],[180,116,84],[191,143,100],[160,120,88]], ..BRICK },
        ..ArchitectureStyle::BritishTerrace.profile()
    }),
    BrusselsSaintGilles => ("brussels-saint-gilles", StyleProfile {
        wall_material: Some(material::BRICK_WALL),
        palette: BRICK,
        opening: OpeningShape::SegmentalArch,
        projection: ProjectionKind::StoneBay,
        projection_m: 0.55,
        ground_storey: GroundStorey::Residential,
        bay_width_m: 3.0,
        banding: Banding::Alternating,
        ..ArchitectureStyle::NorthAmericanBay.profile()
    }),
    PragueOldTown => ("prague-old-town", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: MEDITERRANEAN,
        ground_storey: GroundStorey::Shopfront,
        roofline: Roofline::ShapedGable,
        inferred_roof: Some(RoofShape::Gabled),
        roof_material: Some(material::TILE_ROOF),
        bay_width_m: 2.9,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    BudapestAndrassy => ("budapest-andrassy", StyleProfile {
        floor_height_m: 3.8,
        bay_width_m: 3.2,
        projection: ProjectionKind::Balcony,
        projection_m: 0.55,
        ground_storey: GroundStorey::Shopfront,
        expression_height_m: 32.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    KrakowTownhouse => ("krakow-townhouse", StyleProfile {
        shutters: ShutterKind::None,
        roofline: Roofline::ShapedGable,
        ground_storey: GroundStorey::Shopfront,
        pilasters: true,
        bay_width_m: 3.1,
        palette: StylePalette { walls: &[[216,193,161],[201,171,146],[224,217,188],[191,196,174]], ..MEDITERRANEAN },
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    TallinnMerchant => ("tallinn-merchant", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: LIMESTONE,
        bay_width_m: 2.6,
        shutters: ShutterKind::Panel,
        projection: ProjectionKind::None,
        window_width_ratio: 0.35,
        ..ArchitectureStyle::DutchGable.profile()
    }),
    RigaArtNouveau => ("riga-art-nouveau", StyleProfile {
        projection: ProjectionKind::StoneBay,
        projection_m: 0.55,
        opening: OpeningShape::Rectangular,
        pilasters: false,
        banding: Banding::BaseAndCrown,
        bay_width_m: 3.0,
        floor_height_m: 3.6,
        palette: StylePalette { walls: &[[211,215,192],[195,211,210],[227,217,187],[195,184,165]], ..LIMESTONE },
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    StockholmGamlaStan => ("stockholm-gamla-stan", StyleProfile {
        opening: OpeningShape::Rectangular,
        shutters: ShutterKind::None,
        roof_material: Some(material::METAL_ROOF),
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 2.5,
        projection: ProjectionKind::None,
        palette: StylePalette { walls: &[[206,163,87],[179,110,79],[222,192,126],[189,155,110]], ..MEDITERRANEAN },
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    NordicUrbanRow => ("nordic-urban-row", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        roofline: Roofline::Plain,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::Rectangular,
        shutters: ShutterKind::None,
        projection: ProjectionKind::None,
        pilasters: false,
        bay_width_m: 2.65,
        ..ArchitectureStyle::StockholmGamlaStan.profile()
    }),
    CopenhagenNyhavn => ("copenhagen-nyhavn", StyleProfile {
        roofline: Roofline::FrontGable,
        wall_material: Some(material::PLASTER_WALL),
        palette: CREOLE,
        bay_width_m: 2.65,
        window_height_ratio: 0.64,
        sill_ratio: 0.19,
        ..ArchitectureStyle::DutchGable.profile()
    }),
    RomeTrastevere => ("rome-trastevere", StyleProfile {
        opening: OpeningShape::Rectangular,
        projection: ProjectionKind::None,
        palette: StylePalette { walls: &[[202,153,107],[222,183,133],[193,129,100],[223,202,164]], ..MEDITERRANEAN },
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 2.55,
        projection_m: 0.2,
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    FlorenceOltrarno => ("florence-oltrarno", StyleProfile {
        opening: OpeningShape::Rectangular,
        projection: ProjectionKind::None,
        palette: StylePalette { walls: &[[211,190,151],[197,169,132],[227,211,176],[182,164,136]], ..MEDITERRANEAN },
        frame_m: 0.16,
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 2.8,
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    NaplesSpagnoli => ("naples-spagnoli", StyleProfile {
        opening: OpeningShape::Rectangular,
        projection: ProjectionKind::Balcony,
        projection_m: 0.65,
        bay_width_m: 2.4,
        floor_height_m: 3.2,
        expression_height_m: 31.0,
        palette: StylePalette { walls: &[[205,168,132],[223,193,151],[183,147,124],[220,210,182]], ..MEDITERRANEAN },
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    AlgiersCasbah => ("algiers-casbah", StyleProfile {
        projection: ProjectionKind::TimberBay,
        projection_m: 0.4,
        window_width_ratio: 0.32,
        palette: StylePalette { walls: &[[230,225,205],[216,215,199],[237,231,211],[204,201,185]], ..SWAHILI },
        ..ArchitectureStyle::TunisianMedina.profile()
    }),
    GhardaiaKsar => ("ghardaia-ksar", StyleProfile {
        roofline: Roofline::Parapet,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        window_width_ratio: 0.25,
        palette: StylePalette { walls: &[[204,171,127],[220,194,151],[192,157,118],[228,211,172]], ..DRYEARTH },
        ..ArchitectureStyle::AtlasKasbah.profile()
    }),
    CairoHistoric => ("cairo-historic", StyleProfile {
        palette: LIMESTONE,
        wall_material: Some(material::STONE_WALL),
        projection_m: 0.6,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::PointedArch,
        banding: Banding::Alternating,
        bay_width_m: 3.1,
        ..ArchitectureStyle::HejaziRoshan.profile()
    }),
    HararJugol => ("harar-jugol", StyleProfile {
        palette: StylePalette { walls: &[[219,206,174],[191,184,151],[211,195,157],[226,217,187]], ..DRYEARTH },
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::Rectangular,
        window_width_ratio: 0.28,
        bay_width_m: 2.8,
        expression_height_m: 12.0,
        ..ArchitectureStyle::TunisianMedina.profile()
    }),
    MombasaOldTown => ("mombasa-old-town", StyleProfile {
        projection: ProjectionKind::Gallery,
        projection_m: 0.85,
        bay_width_m: 3.0,
        floor_height_m: 3.4,
        frame_m: 0.22,
        ..ArchitectureStyle::SwahiliStoneTown.profile()
    }),
    DamascusCourtyard => ("damascus-courtyard", StyleProfile {
        wall_material: Some(material::STONE_WALL),
        palette: LIMESTONE,
        projection: ProjectionKind::TimberBay,
        projection_m: 0.45,
        banding: Banding::Alternating,
        ground_storey: GroundStorey::Residential,
        window_width_ratio: 0.34,
        ..ArchitectureStyle::PersianCourtyard.profile()
    }),
    AleppoLimestone => ("aleppo-limestone", StyleProfile {
        wall_material: Some(material::STONE_WALL),
        palette: LIMESTONE,
        projection: ProjectionKind::None,
        ground_storey: GroundStorey::Residential,
        frame_m: 0.22,
        bay_width_m: 2.8,
        ..ArchitectureStyle::PersianCourtyard.profile()
    }),
    JerusalemOldCity => ("jerusalem-old-city", StyleProfile {
        opening: OpeningShape::RoundArch,
        entrance: OpeningShape::RoundArch,
        pilasters: false,
        banding: Banding::None,
        ground_storey: GroundStorey::Residential,
        window_width_ratio: 0.34,
        bay_width_m: 2.8,
        expression_height_m: 20.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    MardinStone => ("mardin-stone", StyleProfile {
        opening: OpeningShape::PointedArch,
        entrance: OpeningShape::PointedArch,
        pilasters: false,
        projection: ProjectionKind::None,
        ground_storey: GroundStorey::Residential,
        bay_width_m: 2.7,
        window_width_ratio: 0.36,
        expression_height_m: 18.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    TbilisiBalcony => ("tbilisi-balcony", StyleProfile {
        projection: ProjectionKind::Gallery,
        projection_m: 1.0,
        shutters: ShutterKind::None,
        palette: StylePalette { walls: &[[206,194,164],[191,175,158],[221,209,181],[197,186,151]], joinery: &[[80,137,150],[77,117,103]], ..MEDITERRANEAN },
        ..ArchitectureStyle::BalkanOriel.profile()
    }),
    BukharaMahalla => ("bukhara-mahalla", StyleProfile {
        ground_storey: GroundStorey::Residential,
        palette: DRYEARTH,
        window_width_ratio: 0.3,
        bay_width_m: 2.9,
        projection_m: 0.6,
        expression_height_m: 14.0,
        ..ArchitectureStyle::PersianCourtyard.profile()
    }),
    SamarkandMahalla => ("samarkand-mahalla", StyleProfile {
        ground_storey: GroundStorey::Veranda,
        window_width_ratio: 0.35,
        bay_width_m: 3.3,
        projection_m: 0.8,
        palette: StylePalette { walls: &[[210,184,140],[225,207,170],[192,169,132],[215,202,167]], ..DRYEARTH },
        ..ArchitectureStyle::PersianCourtyard.profile()
    }),
    KhivaItchanKala => ("khiva-itchan-kala", StyleProfile {
        ground_storey: GroundStorey::Residential,
        projection: ProjectionKind::None,
        bay_width_m: 2.7,
        frame_m: 0.18,
        window_width_ratio: 0.28,
        expression_height_m: 13.0,
        ..ArchitectureStyle::PersianCourtyard.profile()
    }),
    UdaipurLakefront => ("udaipur-lakefront", StyleProfile {
        palette: LIMESTONE,
        wall_material: Some(material::PLASTER_WALL),
        projection_m: 0.6,
        bay_width_m: 2.8,
        roofline: Roofline::Parapet,
        ..ArchitectureStyle::JaisalmerHaveli.profile()
    }),
    VaranasiRiverfront => ("varanasi-riverfront", StyleProfile {
        palette: StylePalette { walls: &[[191,150,110],[218,189,144],[170,121,88],[221,207,175]], ..JAISALMER },
        projection_m: 0.45,
        bay_width_m: 2.65,
        expression_height_m: 29.0,
        ground_storey: GroundStorey::Residential,
        ..ArchitectureStyle::JaisalmerHaveli.profile()
    }),
    LucknowChowk => ("lucknow-chowk", StyleProfile {
        palette: LIMESTONE,
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::RoundArch,
        projection_m: 0.55,
        bay_width_m: 2.8,
        expression_height_m: 20.0,
        ..ArchitectureStyle::JaipurBazaar.profile()
    }),
    PuducherryTamil => ("puducherry-tamil", StyleProfile {
        ground_storey: GroundStorey::Veranda,
        projection_m: 0.7,
        bay_width_m: 2.7,
        eave_m: 0.6,
        wall_material: Some(material::PLASTER_WALL),
        palette: StylePalette { walls: &[[214,189,142],[228,214,177],[198,178,146],[215,209,181]], ..MEDITERRANEAN },
        ..ArchitectureStyle::KeralaVeranda.profile()
    }),
    GalleFort => ("galle-fort", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: LIMESTONE,
        bay_width_m: 3.3,
        projection_m: 0.8,
        roof_material: Some(material::TILE_ROOF),
        shutters: ShutterKind::Louvered,
        ..ArchitectureStyle::KeralaVeranda.profile()
    }),
    LahoreWalledCity => ("lahore-walled-city", StyleProfile {
        wall_material: Some(material::BRICK_WALL),
        palette: BRICK,
        projection_m: 0.65,
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::PointedArch,
        bay_width_m: 2.6,
        inferred_roof: Some(RoofShape::Flat),
        ..ArchitectureStyle::GujaratiPol.profile()
    }),
    LijiangDayan => ("lijiang-dayan", StyleProfile {
        wall_material: Some(material::TIMBER_WALL),
        palette: KYOTO,
        ground_storey: GroundStorey::Shopfront,
        window_width_ratio: 0.48,
        eave_m: 0.85,
        bay_width_m: 3.1,
        ..ArchitectureStyle::ChineseCourtyard.profile()
    }),
    PingyaoWalledCity => ("pingyao-walled-city", StyleProfile {
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 2.8,
        base_height_m: 0.9,
        frame_m: 0.12,
        palette: StylePalette { walls: &[[149,150,135],[173,172,153],[190,185,163],[158,159,145]], ..ASIAN },
        ..ArchitectureStyle::ChineseCourtyard.profile()
    }),
    SeoulBukchon => ("seoul-bukchon", StyleProfile {
        bay_width_m: 2.7,
        eave_m: 0.9,
        expression_height_m: 12.0,
        ground_storey: GroundStorey::Residential,
        ..ArchitectureStyle::KoreanCourtyard.profile()
    }),
    BangkokTalatNoi => ("bangkok-talat-noi", StyleProfile {
        pilasters: false,
        bay_width_m: 2.8,
        projection_m: 0.3,
        palette: StylePalette { walls: &[[213,205,176],[191,185,163],[226,215,183],[191,202,190]], ..TROPICAL },
        ..ArchitectureStyle::StraitsShophouse.profile()
    }),
    QuebecOldTown => ("quebec-old-town", StyleProfile {
        expression_height_m: 22.0,
        bay_width_m: 2.7,
        projection: ProjectionKind::None,
        wall_material: Some(material::STONE_WALL),
        roof_material: Some(material::METAL_ROOF),
        palette: LIMESTONE,
        ..ArchitectureStyle::ParisMansard.profile()
    }),
    MontrealPlateau => ("montreal-plateau", StyleProfile {
        wall_material: Some(material::BRICK_WALL),
        palette: BRICK,
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        projection: ProjectionKind::Balcony,
        projection_m: 0.85,
        bay_width_m: 2.8,
        expression_height_m: 18.0,
        ..ArchitectureStyle::NorthAmericanPorch.profile()
    }),
    BrooklynWilloughbyHart => ("brooklyn-willoughby-hart", StyleProfile {
        wall_material: Some(material::STONE_WALL),
        palette: StylePalette { walls: &[[140,103,79],[159,117,88],[131,102,85],[176,138,107]], ..BRICK },
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        projection: ProjectionKind::StoneBay,
        projection_m: 0.55,
        expression_height_m: 20.0,
        ..ArchitectureStyle::NorthAmericanBay.profile()
    }),
    PhiladelphiaSocietyHill => ("philadelphia-society-hill", StyleProfile {
        entrance: OpeningShape::Rectangular,
        opening: OpeningShape::Rectangular,
        bay_width_m: 2.4,
        window_width_ratio: 0.42,
        base_height_m: 0.65,
        expression_height_m: 18.0,
        ..ArchitectureStyle::BritishTerrace.profile()
    }),
    MexicoCentro => ("mexico-centro", StyleProfile {
        wall_material: Some(material::STONE_WALL),
        palette: StylePalette { walls: &[[155,107,95],[194,165,140],[200,191,169],[175,142,127]], ..MEDITERRANEAN },
        bay_width_m: 3.0,
        floor_height_m: 3.5,
        ground_storey: GroundStorey::Shopfront,
        expression_height_m: 26.0,
        ..ArchitectureStyle::HispanicPatio.profile()
    }),
    PueblaCentro => ("puebla-centro", StyleProfile {
        palette: StylePalette { walls: &[[209,176,137],[196,156,126],[222,214,186],[171,192,198]], ..IBERIAN },
        bay_width_m: 2.9,
        floor_height_m: 3.6,
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        ..ArchitectureStyle::IberianTile.profile()
    }),
    BuenosAiresSanTelmo => ("buenos-aires-san-telmo", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: MEDITERRANEAN,
        projection: ProjectionKind::Juliet,
        projection_m: 0.3,
        bay_width_m: 2.8,
        expression_height_m: 23.0,
        ground_storey: GroundStorey::Shopfront,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    LimaBalconies => ("lima-balconies", StyleProfile {
        projection: ProjectionKind::TimberBay,
        projection_m: 0.7,
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        floor_height_m: 3.5,
        bay_width_m: 3.0,
        ..ArchitectureStyle::AndeanGallery.profile()
    }),
    SydneyPaddington => ("sydney-paddington", StyleProfile {
        projection: ProjectionKind::Gallery,
        projection_m: 0.8,
        ground_storey: GroundStorey::Veranda,
        wall_material: Some(material::PLASTER_WALL),
        palette: MEDITERRANEAN,
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        ..ArchitectureStyle::BritishTerrace.profile()
    }),
    MelbourneCarlton => ("melbourne-carlton", StyleProfile {
        projection: ProjectionKind::Gallery,
        projection_m: 0.9,
        ground_storey: GroundStorey::Veranda,
        roofline: Roofline::Parapet,
        palette: StylePalette { walls: &[[164,109,77],[194,160,118],[218,206,174],[185,145,106]], ..BRICK },
        bay_width_m: 2.8,
        ..ArchitectureStyle::BritishTerrace.profile()
    }),
    DunedinExchange => ("dunedin-exchange", StyleProfile {
        palette: LIMESTONE,
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 3.1,
        floor_height_m: 3.8,
        expression_height_m: 27.0,
        banding: Banding::Alternating,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    AntwerpZurenborg => ("antwerp-zurenborg", StyleProfile {
        wall_material: Some(material::BRICK_WALL),
        palette: BRICK,
        roofline: Roofline::ShapedGable,
        inferred_roof: Some(RoofShape::Gabled),
        projection: ProjectionKind::StoneBay,
        projection_m: 0.6,
        banding: Banding::Alternating,
        bay_width_m: 2.9,
        ..ArchitectureStyle::NorthAmericanBay.profile()
    }),
    ViennaInnereStadt => ("vienna-innere-stadt", StyleProfile {
        floor_height_m: 3.6,
        bay_width_m: 3.0,
        ground_storey: GroundStorey::Shopfront,
        expression_height_m: 32.0,
        palette: StylePalette { walls: &[[225,216,188],[210,201,175],[232,224,204],[203,196,179]], ..LIMESTONE },
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    SalzburgAltstadt => ("salzburg-altstadt", StyleProfile {
        shutters: ShutterKind::None,
        projection: ProjectionKind::None,
        roofline: Roofline::Parapet,
        inferred_roof: Some(RoofShape::Gabled),
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 2.6,
        palette: LIMESTONE,
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    CeskyKrumlovTownhouse => ("cesky-krumlov-townhouse", StyleProfile {
        roofline: Roofline::ShapedGable,
        projection: ProjectionKind::None,
        shutters: ShutterKind::None,
        bay_width_m: 2.8,
        palette: StylePalette { walls: &[[220,199,158],[207,179,153],[230,222,195],[183,195,176]], ..MEDITERRANEAN },
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    GdanskMainTown => ("gdansk-main-town", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: MEDITERRANEAN,
        bay_width_m: 2.5,
        window_height_ratio: 0.69,
        sill_ratio: 0.17,
        pilasters: true,
        expression_height_m: 28.0,
        ..ArchitectureStyle::DutchGable.profile()
    }),
    VilniusOldTown => ("vilnius-old-town", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: LIMESTONE,
        inferred_roof: Some(RoofShape::Gabled),
        roof_material: Some(material::TILE_ROOF),
        bay_width_m: 2.8,
        expression_height_m: 23.0,
        ground_storey: GroundStorey::Shopfront,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    HelsinkiKatajanokka => ("helsinki-katajanokka", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: StylePalette { walls: &[[210,177,123],[185,192,157],[218,199,169],[182,168,150]], ..LIMESTONE },
        projection: ProjectionKind::StoneBay,
        projection_m: 0.65,
        bay_width_m: 3.1,
        opening: OpeningShape::Rectangular,
        pilasters: false,
        banding: Banding::BaseAndCrown,
        inferred_roof: Some(RoofShape::Gabled),
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    BolognaPortico => ("bologna-portico", StyleProfile {
        ground_storey: GroundStorey::Arcade,
        entrance: OpeningShape::RoundArch,
        bay_width_m: 3.5,
        floor_height_m: 3.7,
        palette: StylePalette { walls: &[[191,126,89],[214,160,106],[225,188,128],[175,111,81]], ..MEDITERRANEAN },
        projection: ProjectionKind::None,
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    SienaBrick => ("siena-brick", StyleProfile {
        wall_material: Some(material::BRICK_WALL),
        palette: BRICK,
        opening: OpeningShape::RoundArch,
        entrance: OpeningShape::RoundArch,
        projection: ProjectionKind::None,
        bay_width_m: 2.9,
        frame_m: 0.15,
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    MateraSassi => ("matera-sassi", StyleProfile {
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        pilasters: false,
        banding: Banding::None,
        projection: ProjectionKind::None,
        bay_width_m: 2.7,
        window_width_ratio: 0.3,
        expression_height_m: 13.0,
        ground_storey: GroundStorey::Residential,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    AlberobelloTrulli => ("alberobello-trulli", StyleProfile {
        expression: ExpressionClass::Vernacular,
        wall_material: Some(material::PLASTER_WALL),
        palette: StylePalette { walls: &[[237,233,216],[232,228,211],[240,236,219],[225,223,207]], roof: [145,145,132], ..LIMESTONE },
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        pilasters: false,
        banding: Banding::None,
        roofline: Roofline::Conical,
        inferred_roof: Some(RoofShape::Pyramidal),
        roof_material: Some(material::SLATE_ROOF),
        window_width_ratio: 0.26,
        expression_height_m: 10.0,
        ground_storey: GroundStorey::Residential,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    MeknesMedina => ("meknes-medina", StyleProfile {
        palette: StylePalette { walls: &[[220,204,167],[204,186,145],[232,218,183],[200,182,151]], ..DRYEARTH },
        projection: ProjectionKind::None,
        window_width_ratio: 0.3,
        bay_width_m: 2.7,
        ground_storey: GroundStorey::Residential,
        ..ArchitectureStyle::TunisianMedina.profile()
    }),
    RabatUdayas => ("rabat-udayas", StyleProfile {
        projection: ProjectionKind::None,
        window_width_ratio: 0.32,
        bay_width_m: 2.6,
        expression_height_m: 12.0,
        ground_storey: GroundStorey::Residential,
        ..ArchitectureStyle::MoroccanWhiteBlue.profile()
    }),
    GhadamesOldTown => ("ghadames-old-town", StyleProfile {
        palette: LIMESTONE,
        roofline: Roofline::Parapet,
        projection: ProjectionKind::None,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        window_width_ratio: 0.24,
        expression_height_m: 14.0,
        ..ArchitectureStyle::AtlasKasbah.profile()
    }),
    MassawaIsland => ("massawa-island", StyleProfile {
        ground_storey: GroundStorey::Arcade,
        projection_m: 0.55,
        bay_width_m: 3.3,
        floor_height_m: 3.5,
        palette: LIMESTONE,
        ..ArchitectureStyle::SwahiliStoneTown.profile()
    }),
    MozambiqueStoneTown => ("mozambique-stone-town", StyleProfile {
        ground_storey: GroundStorey::Veranda,
        projection: ProjectionKind::Gallery,
        projection_m: 0.65,
        bay_width_m: 3.2,
        palette: SWAHILI,
        expression_height_m: 18.0,
        ..ArchitectureStyle::LusoWhitewash.profile()
    }),
    MuscatMuttrah => ("muscat-muttrah", StyleProfile {
        projection: ProjectionKind::TimberBay,
        projection_m: 0.55,
        bay_width_m: 2.8,
        palette: LIMESTONE,
        entrance: OpeningShape::PointedArch,
        expression_height_m: 20.0,
        ..ArchitectureStyle::SwahiliStoneTown.profile()
    }),
    ShibamTower => ("shibam-tower", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: DRYEARTH,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::Rectangular,
        roofline: Roofline::Parapet,
        banding: Banding::BaseAndCrown,
        window_width_ratio: 0.28,
        expression_height_m: 32.0,
        ..ArchitectureStyle::SanaaniTower.profile()
    }),
    BushehrBalcony => ("bushehr-balcony", StyleProfile {
        projection: ProjectionKind::Gallery,
        projection_m: 0.9,
        palette: LIMESTONE,
        shutters: ShutterKind::Louvered,
        bay_width_m: 3.2,
        floor_height_m: 3.6,
        ..ArchitectureStyle::SwahiliStoneTown.profile()
    }),
    AmritsarWalledCity => ("amritsar-walled-city", StyleProfile {
        wall_material: Some(material::BRICK_WALL),
        palette: BRICK,
        projection_m: 0.5,
        opening: OpeningShape::SegmentalArch,
        bay_width_m: 2.7,
        expression_height_m: 20.0,
        inferred_roof: Some(RoofShape::Flat),
        ..ArchitectureStyle::GujaratiPol.profile()
    }),
    KaraikudiChettinad => ("karaikudi-chettinad", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        ground_storey: GroundStorey::Veranda,
        entrance: OpeningShape::RoundArch,
        pilasters: true,
        frame_m: 0.18,
        bay_width_m: 3.4,
        floor_height_m: 3.8,
        palette: LIMESTONE,
        ..ArchitectureStyle::KeralaVeranda.profile()
    }),
    DhakaShankharibazar => ("dhaka-shankharibazar", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: MEDITERRANEAN,
        projection_m: 0.5,
        bay_width_m: 2.2,
        floor_height_m: 3.1,
        inferred_roof: Some(RoofShape::Flat),
        ground_storey: GroundStorey::Shopfront,
        expression_height_m: 23.0,
        ..ArchitectureStyle::GujaratiPol.profile()
    }),
    DaliBai => ("dali-bai", StyleProfile {
        palette: HUIZHOU,
        wall_material: Some(material::PLASTER_WALL),
        bay_width_m: 3.3,
        eave_m: 0.8,
        frame_m: 0.13,
        ground_storey: GroundStorey::Residential,
        ..ArchitectureStyle::ChineseCourtyard.profile()
    }),
    SuzhouPingjiang => ("suzhou-pingjiang", StyleProfile {
        palette: HUIZHOU,
        wall_material: Some(material::PLASTER_WALL),
        bay_width_m: 2.8,
        eave_m: 0.65,
        window_width_ratio: 0.4,
        ground_storey: GroundStorey::Shopfront,
        ..ArchitectureStyle::ChineseCourtyard.profile()
    }),
    QuanzhouRedBrick => ("quanzhou-red-brick", StyleProfile {
        palette: BRICK,
        wall_material: Some(material::BRICK_WALL),
        roofline: Roofline::ShapedGable,
        bay_width_m: 3.4,
        eave_m: 0.7,
        roof_material: Some(material::TILE_ROOF),
        ..ArchitectureStyle::ChineseCourtyard.profile()
    }),
    TainanShennong => ("tainan-shennong", StyleProfile {
        palette: BRICK,
        wall_material: Some(material::BRICK_WALL),
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 2.4,
        window_width_ratio: 0.5,
        eave_m: 0.5,
        ..ArchitectureStyle::HoiAnMerchant.profile()
    }),
    YangonDowntown => ("yangon-downtown", StyleProfile {
        bay_width_m: 3.1,
        floor_height_m: 3.6,
        projection: ProjectionKind::Balcony,
        projection_m: 0.55,
        expression_height_m: 29.0,
        palette: LIMESTONE,
        ..ArchitectureStyle::StraitsShophouse.profile()
    }),
    PhuketOldTown => ("phuket-old-town", StyleProfile {
        bay_width_m: 2.7,
        projection_m: 0.25,
        floor_height_m: 3.4,
        palette: StylePalette { walls: &[[217,211,189],[209,188,156],[194,210,202],[215,181,165]], ..TROPICAL },
        ..ArchitectureStyle::StraitsShophouse.profile()
    }),
    GuanajuatoCentro => ("guanajuato-centro", StyleProfile {
        opening: OpeningShape::Rectangular,
        inferred_roof: Some(RoofShape::Flat),
        roofline: Roofline::Parapet,
        eave_m: 0.15,
        palette: CREOLE,
        bay_width_m: 2.65,
        projection: ProjectionKind::Juliet,
        projection_m: 0.3,
        ground_storey: GroundStorey::Shopfront,
        expression_height_m: 21.0,
        ..ArchitectureStyle::HispanicPatio.profile()
    }),
    MoreliaCantera => ("morelia-cantera", StyleProfile {
        palette: StylePalette { walls: &[[194,164,151],[207,184,168],[185,158,147],[220,197,179]], ..LIMESTONE },
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 3.0,
        expression_height_m: 23.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    OaxacaCantera => ("oaxaca-cantera", StyleProfile {
        palette: StylePalette { walls: &[[178,190,161],[207,189,149],[212,166,127],[224,212,180]], ..MEDITERRANEAN },
        bay_width_m: 3.1,
        expression_height_m: 16.0,
        frame_m: 0.2,
        projection: ProjectionKind::None,
        ..ArchitectureStyle::HispanicPatio.profile()
    }),
    MeridaCentro => ("merida-centro", StyleProfile {
        palette: CREOLE,
        bay_width_m: 3.3,
        floor_height_m: 3.8,
        expression_height_m: 16.0,
        pilasters: true,
        window_height_ratio: 0.73,
        sill_ratio: 0.12,
        ..ArchitectureStyle::HispanicPatio.profile()
    }),
    ZacatecasCantera => ("zacatecas-cantera", StyleProfile {
        palette: StylePalette { walls: &[[197,151,137],[216,177,154],[185,143,132],[223,194,172]], ..LIMESTONE },
        bay_width_m: 2.8,
        ground_storey: GroundStorey::Shopfront,
        frame_m: 0.2,
        expression_height_m: 22.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    SantiagoCubaGallery => ("santiago-cuba-gallery", StyleProfile {
        roof_material: Some(material::TILE_ROOF),
        inferred_roof: Some(RoofShape::Gabled),
        bay_width_m: 2.9,
        projection_m: 0.7,
        expression_height_m: 20.0,
        ..ArchitectureStyle::CaribbeanGallery.profile()
    }),
    NapierArtDeco => ("napier-art-deco", StyleProfile {
        bay_width_m: 3.2,
        floor_height_m: 3.7,
        expression_height_m: 20.0,
        palette: StylePalette { walls: &[[222,209,170],[199,218,209],[222,190,171],[230,224,202]], ..TROPICAL },
        ground_storey: GroundStorey::Shopfront,
        ..ArchitectureStyle::AsmaraModernist.profile()
    }),
    OamaruLimestone => ("oamaru-limestone", StyleProfile {
        bay_width_m: 3.4,
        floor_height_m: 4.0,
        expression_height_m: 22.0,
        ground_storey: GroundStorey::Shopfront,
        palette: StylePalette { walls: &[[225,216,185],[213,207,180],[232,223,197],[219,209,177]], ..LIMESTONE },
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    HamburgKontorhaus => ("hamburg-kontorhaus", StyleProfile {
        expression: ExpressionClass::Heritage,
        palette: StylePalette { walls: &[[129,79,62],[148,90,67],[120,78,64],[158,103,79]], ..BRICK },
        projection: ProjectionKind::None,
        pilasters: true,
        bay_width_m: 2.5,
        floor_height_m: 3.3,
        ground_storey: GroundStorey::Shopfront,
        expression_height_m: 35.0,
        ..ArchitectureStyle::ContemporaryBrick.profile()
    }),
    LubeckMerchant => ("lubeck-merchant", StyleProfile {
        palette: BRICK,
        bay_width_m: 2.7,
        window_width_ratio: 0.36,
        entrance: OpeningShape::PointedArch,
        projection: ProjectionKind::None,
        ..ArchitectureStyle::DutchGable.profile()
    }),
    BremenSchnoor => ("bremen-schnoor", StyleProfile {
        bay_width_m: 2.2,
        floor_height_m: 2.7,
        palette: LIMESTONE,
        expression_height_m: 14.0,
        projection: ProjectionKind::None,
        ground_storey: GroundStorey::Shopfront,
        ..ArchitectureStyle::HalfTimber.profile()
    }),
    HeidelbergAltstadt => ("heidelberg-altstadt", StyleProfile {
        bay_width_m: 2.8,
        projection: ProjectionKind::None,
        ground_storey: GroundStorey::Shopfront,
        palette: StylePalette { walls: &[[218,190,153],[201,173,153],[226,215,188],[198,195,165]], ..MEDITERRANEAN },
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    RothenburgHalfTimber => ("rothenburg-half-timber", StyleProfile {
        bay_width_m: 2.6,
        floor_height_m: 2.9,
        roofline: Roofline::FrontGable,
        palette: StylePalette { walls: &[[222,210,176],[213,195,161],[228,219,194],[206,194,165]], ..HIMALAYA },
        ..ArchitectureStyle::HalfTimber.profile()
    }),
    BambergTownhouse => ("bamberg-townhouse", StyleProfile {
        bay_width_m: 2.9,
        projection: ProjectionKind::None,
        roofline: Roofline::FrontGable,
        ground_storey: GroundStorey::Shopfront,
        palette: StylePalette { walls: &[[220,207,172],[214,188,158],[194,204,180],[231,220,190]], ..MEDITERRANEAN },
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    RegensburgPatrician => ("regensburg-patrician", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: MEDITERRANEAN,
        opening: OpeningShape::RoundArch,
        bay_width_m: 2.8,
        inferred_roof: Some(RoofShape::Gabled),
        roof_material: Some(material::TILE_ROOF),
        ground_storey: GroundStorey::Shopfront,
        expression_height_m: 27.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    DresdenNeustadt => ("dresden-neustadt", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: LIMESTONE,
        bay_width_m: 2.9,
        floor_height_m: 3.3,
        inferred_roof: Some(RoofShape::Mansard),
        roof_material: Some(material::TILE_ROOF),
        roofline: Roofline::Mansard,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    NurembergWeissgerber => ("nuremberg-weissgerber", StyleProfile {
        bay_width_m: 2.4,
        floor_height_m: 2.8,
        ground_storey: GroundStorey::Shopfront,
        palette: StylePalette { walls: &[[220,210,180],[206,194,160],[227,217,191],[196,188,167]], ..HIMALAYA },
        ..ArchitectureStyle::HalfTimber.profile()
    }),
    LeipzigWaldstrasse => ("leipzig-waldstrasse", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: LIMESTONE,
        bay_width_m: 3.0,
        floor_height_m: 3.5,
        projection: ProjectionKind::StoneBay,
        projection_m: 0.55,
        inferred_roof: Some(RoofShape::Mansard),
        roofline: Roofline::Mansard,
        expression_height_m: 30.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    LyonCanut => ("lyon-canut", StyleProfile {
        opening: OpeningShape::Rectangular,
        shutters: ShutterKind::None,
        projection: ProjectionKind::None,
        bay_width_m: 2.4,
        floor_height_m: 4.0,
        window_height_ratio: 0.75,
        sill_ratio: 0.12,
        expression_height_m: 32.0,
        palette: StylePalette { walls: &[[217,185,138],[227,205,168],[196,166,133],[215,199,166]], ..MEDITERRANEAN },
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    DijonTimber => ("dijon-timber", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        bay_width_m: 2.7,
        roof_material: Some(material::TILE_ROOF),
        palette: LIMESTONE,
        ground_storey: GroundStorey::Shopfront,
        ..ArchitectureStyle::HalfTimber.profile()
    }),
    RouenTimber => ("rouen-timber", StyleProfile {
        bay_width_m: 2.35,
        floor_height_m: 2.8,
        ground_storey: GroundStorey::Shopfront,
        palette: StylePalette { walls: &[[221,211,183],[208,203,175],[230,219,194],[211,193,170]], joinery: &[[94,69,53],[92,102,110]], ..HIMALAYA },
        ..ArchitectureStyle::HalfTimber.profile()
    }),
    VeronaErbe => ("verona-erbe", StyleProfile {
        bay_width_m: 2.8,
        ground_storey: GroundStorey::Shopfront,
        palette: StylePalette { walls: &[[214,172,132],[206,153,122],[226,199,160],[199,175,145]], ..MEDITERRANEAN },
        frame_m: 0.14,
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    LecceStone => ("lecce-stone", StyleProfile {
        palette: StylePalette { walls: &[[221,203,159],[232,215,174],[208,190,150],[226,212,177]], ..LIMESTONE },
        frame_m: 0.22,
        bay_width_m: 2.9,
        projection: ProjectionKind::Balcony,
        projection_m: 0.45,
        expression_height_m: 20.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    TurinPortico => ("turin-portico", StyleProfile {
        ground_storey: GroundStorey::Arcade,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        bay_width_m: 3.5,
        floor_height_m: 3.7,
        palette: StylePalette { walls: &[[219,193,151],[210,181,144],[228,209,173],[203,183,149]], ..LIMESTONE },
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    WroclawRynek => ("wroclaw-rynek", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: CREOLE,
        bay_width_m: 2.8,
        pilasters: true,
        expression_height_m: 27.0,
        ..ArchitectureStyle::DutchGable.profile()
    }),
    TorunBrick => ("torun-brick", StyleProfile {
        palette: BRICK,
        entrance: OpeningShape::PointedArch,
        opening: OpeningShape::PointedArch,
        bay_width_m: 2.6,
        window_width_ratio: 0.37,
        frame_m: 0.12,
        ..ArchitectureStyle::DutchGable.profile()
    }),
    SibiuEyebrow => ("sibiu-eyebrow", StyleProfile {
        projection: ProjectionKind::None,
        bay_width_m: 2.8,
        roofline: Roofline::EyebrowDormers,
        inferred_roof: Some(RoofShape::Hipped),
        palette: StylePalette { walls: &[[215,192,149],[224,209,178],[200,178,146],[198,202,177]], ..MEDITERRANEAN },
        expression_height_m: 20.0,
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    SighisoaraCitadel => ("sighisoara-citadel", StyleProfile {
        projection: ProjectionKind::None,
        bay_width_m: 2.6,
        expression_height_m: 17.0,
        palette: StylePalette { walls: &[[215,179,136],[194,168,153],[221,207,161],[178,196,175]], ..MEDITERRANEAN },
        ..ArchitectureStyle::MediterraneanShutter.profile()
    }),
    YerevanTuff => ("yerevan-tuff", StyleProfile {
        palette: StylePalette { walls: &[[195,145,129],[209,163,143],[172,135,124],[221,180,158]], ..LIMESTONE },
        bay_width_m: 3.0,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::RoundArch,
        ground_storey: GroundStorey::Shopfront,
        expression_height_m: 26.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    ShekiMerchant => ("sheki-merchant", StyleProfile {
        wall_material: Some(material::BRICK_WALL),
        palette: BRICK,
        projection: ProjectionKind::Gallery,
        projection_m: 0.75,
        roof_material: Some(material::TILE_ROOF),
        bay_width_m: 3.1,
        expression_height_m: 17.0,
        ..ArchitectureStyle::BalkanOriel.profile()
    }),
    BakuIcherisheher => ("baku-icherisheher", StyleProfile {
        projection: ProjectionKind::TimberBay,
        projection_m: 0.45,
        pilasters: false,
        banding: Banding::BaseAndCrown,
        ground_storey: GroundStorey::Residential,
        bay_width_m: 2.7,
        expression_height_m: 21.0,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    NizwaAlAqr => ("nizwa-al-aqr", StyleProfile {
        roofline: Roofline::Parapet,
        opening: OpeningShape::Rectangular,
        entrance: OpeningShape::PointedArch,
        window_width_ratio: 0.28,
        bay_width_m: 2.8,
        expression_height_m: 15.0,
        ..ArchitectureStyle::AtlasKasbah.profile()
    }),
    KashgarOldCity => ("kashgar-old-city", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        projection: ProjectionKind::TimberBay,
        projection_m: 0.55,
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 2.8,
        palette: DRYEARTH,
        expression_height_m: 18.0,
        ..ArchitectureStyle::PersianCourtyard.profile()
    }),
    LhasaBarkhor => ("lhasa-barkhor", StyleProfile {
        projection: ProjectionKind::None,
        roofline: Roofline::Parapet,
        inferred_roof: Some(RoofShape::Flat),
        eave_m: 0.25,
        window_width_ratio: 0.36,
        banding: Banding::BaseAndCrown,
        palette: StylePalette { walls: &[[226,221,199],[235,226,207],[216,213,193],[229,220,192]], trim: [69,55,43], joinery: &[[93,52,36],[109,62,39]], ..BHUTAN },
        ..ArchitectureStyle::BhutaneseRabsel.profile()
    }),
    ShaxiSideng => ("shaxi-sideng", StyleProfile {
        wall_material: Some(material::TIMBER_WALL),
        palette: KYOTO,
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 3.4,
        eave_m: 0.8,
        window_width_ratio: 0.48,
        ..ArchitectureStyle::ChineseCourtyard.profile()
    }),
    FenghuangRiverfront => ("fenghuang-riverfront", StyleProfile {
        wall_material: Some(material::TIMBER_WALL),
        palette: KYOTO,
        projection: ProjectionKind::Gallery,
        projection_m: 0.7,
        bay_width_m: 2.8,
        eave_m: 0.85,
        expression_height_m: 18.0,
        ..ArchitectureStyle::ChineseCourtyard.profile()
    }),
    HongcunHuizhou => ("hongcun-huizhou", StyleProfile {
        bay_width_m: 3.1,
        window_width_ratio: 0.32,
        eave_m: 0.6,
        expression_height_m: 16.0,
        ..ArchitectureStyle::HuizhouGable.profile()
    }),
    NaraNaramachi => ("nara-naramachi", StyleProfile {
        bay_width_m: 2.7,
        eave_m: 0.75,
        expression_height_m: 12.0,
        projection_m: 0.25,
        ..ArchitectureStyle::KyotoMachiya.profile()
    }),
    KurashikiBikan => ("kurashiki-bikan", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: HUIZHOU,
        banding: Banding::BaseAndCrown,
        window_width_ratio: 0.35,
        ground_storey: GroundStorey::Residential,
        base_height_m: 1.2,
        plinth_material: Some(material::NAMAKO_WALL),
        frame_m: 0.14,
        bay_width_m: 3.0,
        ..ArchitectureStyle::KyotoMachiya.profile()
    }),
    ShirakawaOgimachi => ("shirakawa-ogimachi", StyleProfile {
        wall_material: Some(material::TIMBER_WALL),
        palette: StylePalette { roof: [139,116,78], ..KYOTO },
        roof_material: Some(material::THATCH_ROOF),
        projection: ProjectionKind::None,
        ground_storey: GroundStorey::Residential,
        roofline: Roofline::FrontGable,
        inferred_roof: Some(RoofShape::Gabled),
        bay_width_m: 3.3,
        expression_height_m: 16.0,
        eave_m: 1.0,
        ..ArchitectureStyle::HimalayanTimber.profile()
    }),
    HakodateMotomachi => ("hakodate-motomachi", StyleProfile {
        palette: StylePalette { walls: &[[192,213,208],[215,213,182],[184,200,211],[219,202,180]], ..NEUTRAL },
        bay_width_m: 2.8,
        floor_height_m: 3.0,
        window_height_ratio: 0.65,
        sill_ratio: 0.18,
        roof_material: Some(material::METAL_ROOF),
        expression_height_m: 15.0,
        ..ArchitectureStyle::NordicClapboard.profile()
    }),
    GyeongjuGyochon => ("gyeongju-gyochon", StyleProfile {
        ground_storey: GroundStorey::Veranda,
        bay_width_m: 3.0,
        eave_m: 1.0,
        expression_height_m: 12.0,
        ..ArchitectureStyle::KoreanCourtyard.profile()
    }),
    BandungBraga => ("bandung-braga", StyleProfile {
        ground_storey: GroundStorey::Shopfront,
        bay_width_m: 3.4,
        floor_height_m: 3.6,
        palette: LIMESTONE,
        expression_height_m: 20.0,
        ..ArchitectureStyle::AsmaraModernist.profile()
    }),
    SemarangKotaLama => ("semarang-kota-lama", StyleProfile {
        palette: LIMESTONE,
        bay_width_m: 3.4,
        floor_height_m: 3.8,
        projection: ProjectionKind::None,
        expression_height_m: 22.0,
        ..ArchitectureStyle::StraitsShophouse.profile()
    }),
    PadangBatangArau => ("padang-batang-arau", StyleProfile {
        palette: LIMESTONE,
        bay_width_m: 3.0,
        floor_height_m: 3.5,
        shutters: ShutterKind::Louvered,
        roof_material: Some(material::METAL_ROOF),
        projection_m: 0.25,
        expression_height_m: 20.0,
        ..ArchitectureStyle::StraitsShophouse.profile()
    }),
    SantiagoYungay => ("santiago-yungay", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: MEDITERRANEAN,
        bay_width_m: 2.9,
        floor_height_m: 3.6,
        expression_height_m: 18.0,
        ground_storey: GroundStorey::Residential,
        projection: ProjectionKind::Juliet,
        projection_m: 0.25,
        ..ArchitectureStyle::ClassicalLimestone.profile()
    }),
    HobartBatteryPoint => ("hobart-battery-point", StyleProfile {
        wall_material: Some(material::STONE_WALL),
        palette: LIMESTONE,
        bay_width_m: 2.8,
        projection: ProjectionKind::None,
        eave_m: 0.35,
        expression_height_m: 13.0,
        ..ArchitectureStyle::AustralasianCottage.profile()
    }),
    SanCristobalCentro => ("san-cristobal-centro", StyleProfile {
        inferred_roof: Some(RoofShape::Gabled),
        roof_material: Some(material::TILE_ROOF),
        palette: MEDITERRANEAN,
        projection: ProjectionKind::None,
        bay_width_m: 2.8,
        expression_height_m: 14.0,
        eave_m: 0.6,
        ..ArchitectureStyle::HispanicPatio.profile()
    }),
    SanJoseAmon => ("san-jose-amon", StyleProfile {
        wall_material: Some(material::PLASTER_WALL),
        palette: CREOLE,
        opening: OpeningShape::SegmentalArch,
        entrance: OpeningShape::RoundArch,
        pilasters: true,
        bay_width_m: 3.0,
        expression_height_m: 16.0,
        ..ArchitectureStyle::AustralasianCottage.profile()
    }),
}

impl Default for ArchitectureStyle {
    fn default() -> Self {
        Self::Contemporary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn atlas_keys_are_unique_and_round_trip() {
        let mut keys = HashSet::new();
        for style in ARCHITECTURE_STYLES {
            let key = style.key();
            assert!(keys.insert(key), "duplicate style key {key}");
            assert!(key.bytes().all(|c| c.is_ascii_lowercase() || c == b'-'));
            assert_eq!(ArchitectureStyle::from_key(key), Some(*style));
        }
        assert_eq!(ArchitectureStyle::from_key("unresearched-temple"), None);
    }

    #[test]
    fn profiles_fit_coherent_facade_cells() {
        for style in ARCHITECTURE_STYLES {
            let p = style.profile();
            assert!((2.0..=4.5).contains(&p.bay_width_m), "{}", style.key());
            assert!((2.5..=4.5).contains(&p.floor_height_m), "{}", style.key());
            assert!(
                (0.2..=0.8).contains(&p.window_width_ratio),
                "{}",
                style.key()
            );
            assert!(
                p.sill_ratio + p.window_height_ratio < 0.96,
                "{}",
                style.key()
            );
            assert!((0.0..=2.0).contains(&p.projection_m), "{}", style.key());
            assert!((0.0..=1.5).contains(&p.eave_m), "{}", style.key());
            assert!(p.base_height_m < p.floor_height_m, "{}", style.key());
            assert!(p.expression_height_m.is_finite() && p.expression_height_m > 8.0);
            assert!(p.palette.glass.into_iter().all(|channel| channel < 120));
            let c = style.colors(0x3456789abcdef);
            assert!(p.palette.walls.contains(&c.wall));
            assert!(p.palette.joinery.contains(&c.joinery));
        }
    }

    #[test]
    fn priority_places_have_distinct_composition_and_color_roles() {
        use ArchitectureStyle::*;
        let signatures = [
            JaisalmerHaveli,
            JodhpurHaveli,
            JaipurBazaar,
            BahiaSobrado,
            AmalfiTerrace,
            DutchGable,
            KyotoMachiya,
            HejaziRoshan,
            BhutaneseRabsel,
            AsmaraModernist,
        ]
        .map(|s| {
            let p = s.profile();
            (
                p.opening,
                p.shutters,
                p.projection,
                p.roofline,
                p.ground_storey,
                p.banding,
                p.pilasters,
                p.palette.walls,
                p.palette.joinery,
            )
        });
        assert_eq!(signatures.into_iter().collect::<HashSet<_>>().len(), 10);
        assert_ne!(
            SidiBouSaid.colors(0).joinery,
            TetouanMedina.colors(0).joinery
        );
        assert_ne!(
            SidiBouSaid.profile().palette.walls,
            MoroccanWhiteBlue.profile().palette.walls
        );
        assert_ne!(
            TunisianMedina.profile().palette.joinery,
            SidiBouSaid.profile().palette.joinery
        );
        assert_eq!(AtlasKasbah.profile().inferred_roof, Some(RoofShape::Flat));
        assert_eq!(
            HuizhouGable.profile().inferred_roof,
            Some(RoofShape::Gabled)
        );
    }
}
