//! Non-regional tall-building families inferred from mapped geometry. These
//! profiles describe coherent facade systems; they never replace mapped
//! footprints, building parts, reported heights, or LiDAR roof geometry.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TallBuildingStyle {
    ChicagoFrame,
    ArtDecoSetback,
    InternationalSlab,
    CurtainWall,
    Brutalist,
    ResidentialLoggia,
    PostmodernCrown,
    FramedTube,
    BundledTube,
    Diagrid,
    Twisting,
    TerracedMixedUse,
    ButtressedSupertall,
    CircularResidential,
    SCurveSlab,
    RippledBalcony,
    SculptedCrescent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TallMassing {
    Tripartite,
    Setback,
    Slab,
    Box,
    MonumentalConcrete,
    LoggiaSlab,
    Pediment,
    Tube,
    BundledTube,
    TaperedDiagrid,
    Twist,
    Terraces,
    Buttressed,
    Cylinder,
    SCurve,
    Rippled,
    Crescent,
}

/// Shape facts measured from the mapped outer footprint. Curved families are
/// only inferred when the source polygon itself supplies enough vertices and
/// the expected compactness/concavity; a rectangular extrusion never gains a
/// curved identity from random seed alone.
#[derive(Clone, Copy, Debug)]
pub struct TallBuildingShape {
    pub area_m2: f32,
    pub aspect: f32,
    pub compactness: f32,
    pub vertex_count: u16,
    pub fill_ratio: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct TallBuildingProfile {
    pub id: TallBuildingStyle,
    pub slug: &'static str,
    pub label: &'static str,
    pub period: &'static str,
    pub massing: TallMassing,
    pub floor_m: f64,
    pub bay_m: f32,
    pub middle_x: (f32, f32),
    pub base_x: (f32, f32),
    pub middle_y: (f64, f64),
    pub base_y: (f64, f64),
    pub crown_y: (f64, f64),
    pub open_threshold: u8,
    /// Whole mechanical/service floor cadence; zero means crown-only plant.
    pub mechanical_every: u8,
    pub frame: [u8; 3],
    pub spandrel: [u8; 3],
    pub glass: [u8; 3],
    pub accent: [u8; 3],
    pub roof: [u8; 3],
}

// Colours are deliberately mid-value, material-derived sRGB rather than
// saturated landmark colours. The renderer supplies light/shadow variation;
// nearby buildings get only a narrow deterministic value shift.
pub const TALL_BUILDING_PROFILES: [TallBuildingProfile; 17] = [
    TallBuildingProfile {
        id: TallBuildingStyle::ChicagoFrame,
        slug: "chicago-frame",
        label: "Chicago frame · base / shaft / capital",
        period: "1890s–1920s",
        massing: TallMassing::Tripartite,
        floor_m: 3.7,
        bay_m: 3.5,
        middle_x: (0.24, 0.76),
        base_x: (0.14, 0.86),
        middle_y: (0.27, 0.72),
        base_y: (0.14, 0.82),
        crown_y: (0.34, 0.66),
        open_threshold: 239,
        mechanical_every: 0,
        frame: [163, 149, 126],
        spandrel: [115, 91, 71],
        glass: [69, 88, 104],
        accent: [191, 179, 154],
        roof: [91, 91, 87],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::ArtDecoSetback,
        slug: "art-deco-setback",
        label: "Art Deco · zoning setbacks",
        period: "1920s–1940s",
        massing: TallMassing::Setback,
        floor_m: 3.6,
        bay_m: 3.1,
        middle_x: (0.28, 0.72),
        base_x: (0.18, 0.82),
        middle_y: (0.27, 0.70),
        base_y: (0.16, 0.80),
        crown_y: (0.39, 0.63),
        open_threshold: 238,
        mechanical_every: 0,
        frame: [182, 171, 148],
        spandrel: [126, 118, 102],
        glass: [68, 86, 102],
        accent: [105, 124, 111],
        roof: [95, 96, 92],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::InternationalSlab,
        slug: "international-slab",
        label: "International Style · steel-and-glass slab",
        period: "1950s–1970s",
        massing: TallMassing::Slab,
        floor_m: 3.6,
        bay_m: 2.6,
        middle_x: (0.16, 0.84),
        base_x: (0.08, 0.92),
        middle_y: (0.20, 0.78),
        base_y: (0.10, 0.86),
        crown_y: (0.25, 0.73),
        open_threshold: 250,
        mechanical_every: 12,
        frame: [48, 51, 52],
        spandrel: [74, 77, 77],
        glass: [78, 104, 120],
        accent: [164, 166, 161],
        roof: [76, 80, 81],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::CurtainWall,
        slug: "curtain-wall-box",
        label: "Corporate curtain wall · aluminium and glass",
        period: "1960s–present",
        massing: TallMassing::Box,
        floor_m: 3.75,
        bay_m: 2.4,
        middle_x: (0.13, 0.87),
        base_x: (0.07, 0.93),
        middle_y: (0.18, 0.80),
        base_y: (0.08, 0.88),
        crown_y: (0.25, 0.73),
        open_threshold: 253,
        mechanical_every: 15,
        frame: [116, 124, 126],
        spandrel: [49, 63, 70],
        glass: [72, 108, 132],
        accent: [183, 186, 180],
        roof: [82, 91, 94],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::Brutalist,
        slug: "brutalist-concrete",
        label: "Brutalist · concrete mass and recess",
        period: "1960s–1980s",
        massing: TallMassing::MonumentalConcrete,
        floor_m: 3.8,
        bay_m: 3.8,
        middle_x: (0.34, 0.66),
        base_x: (0.30, 0.70),
        middle_y: (0.35, 0.66),
        base_y: (0.31, 0.70),
        crown_y: (0.42, 0.61),
        open_threshold: 226,
        mechanical_every: 9,
        frame: [151, 151, 143],
        spandrel: [112, 113, 108],
        glass: [65, 86, 101],
        accent: [178, 176, 165],
        roof: [113, 114, 109],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::ResidentialLoggia,
        slug: "residential-loggia",
        label: "Residential slab · loggias and balconies",
        period: "1950s–present",
        massing: TallMassing::LoggiaSlab,
        floor_m: 3.05,
        bay_m: 3.25,
        middle_x: (0.20, 0.80),
        base_x: (0.14, 0.86),
        middle_y: (0.24, 0.76),
        base_y: (0.16, 0.82),
        crown_y: (0.30, 0.69),
        open_threshold: 244,
        mechanical_every: 0,
        frame: [192, 186, 172],
        spandrel: [126, 132, 132],
        glass: [72, 96, 112],
        accent: [156, 112, 80],
        roof: [105, 109, 109],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::PostmodernCrown,
        slug: "postmodern-crown",
        label: "Postmodern · stone shaft and crown",
        period: "1980s–1990s",
        massing: TallMassing::Pediment,
        floor_m: 3.8,
        bay_m: 3.3,
        middle_x: (0.25, 0.75),
        base_x: (0.16, 0.84),
        middle_y: (0.27, 0.71),
        base_y: (0.14, 0.82),
        crown_y: (0.35, 0.66),
        open_threshold: 241,
        mechanical_every: 0,
        frame: [164, 149, 145],
        spandrel: [111, 98, 96],
        glass: [68, 88, 103],
        accent: [193, 181, 174],
        roof: [91, 86, 85],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::FramedTube,
        slug: "framed-tube",
        label: "Framed tube · expressed perimeter",
        period: "1960s–1980s",
        massing: TallMassing::Tube,
        floor_m: 3.75,
        bay_m: 3.0,
        middle_x: (0.23, 0.77),
        base_x: (0.14, 0.86),
        middle_y: (0.23, 0.75),
        base_y: (0.13, 0.84),
        crown_y: (0.29, 0.69),
        open_threshold: 249,
        mechanical_every: 18,
        frame: [51, 53, 52],
        spandrel: [69, 62, 53],
        glass: [72, 98, 115],
        accent: [125, 113, 92],
        roof: [66, 68, 67],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::BundledTube,
        slug: "bundled-tube",
        label: "Bundled tube · tiered structural cells",
        period: "1970s–present",
        massing: TallMassing::BundledTube,
        floor_m: 3.75,
        bay_m: 2.8,
        middle_x: (0.20, 0.80),
        base_x: (0.12, 0.88),
        middle_y: (0.21, 0.77),
        base_y: (0.11, 0.86),
        crown_y: (0.28, 0.69),
        open_threshold: 250,
        mechanical_every: 16,
        frame: [48, 51, 50],
        spandrel: [73, 65, 53],
        glass: [76, 101, 119],
        accent: [120, 110, 91],
        roof: [66, 68, 66],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::Diagrid,
        slug: "diagrid-taper",
        label: "Diagrid · tapered triangulated shell",
        period: "2000s–present",
        massing: TallMassing::TaperedDiagrid,
        floor_m: 3.9,
        bay_m: 3.4,
        middle_x: (0.13, 0.87),
        base_x: (0.08, 0.92),
        middle_y: (0.17, 0.81),
        base_y: (0.08, 0.90),
        crown_y: (0.22, 0.76),
        open_threshold: 253,
        mechanical_every: 14,
        frame: [141, 147, 148],
        spandrel: [66, 77, 81],
        glass: [82, 113, 134],
        accent: [137, 146, 147],
        roof: [92, 100, 102],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::Twisting,
        slug: "twisting-plates",
        label: "Twisting tower · rotating floor plates",
        period: "2000s–present",
        massing: TallMassing::Twist,
        floor_m: 3.45,
        bay_m: 2.9,
        middle_x: (0.14, 0.86),
        base_x: (0.08, 0.92),
        middle_y: (0.18, 0.80),
        base_y: (0.09, 0.89),
        crown_y: (0.23, 0.75),
        open_threshold: 252,
        mechanical_every: 17,
        frame: [155, 161, 161],
        spandrel: [68, 83, 89],
        glass: [80, 116, 139],
        accent: [194, 197, 192],
        roof: [95, 103, 105],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::TerracedMixedUse,
        slug: "terraced-mixed-use",
        label: "Mixed use · podium and sky terraces",
        period: "1990s–present",
        massing: TallMassing::Terraces,
        floor_m: 3.6,
        bay_m: 3.0,
        middle_x: (0.17, 0.83),
        base_x: (0.09, 0.91),
        middle_y: (0.20, 0.78),
        base_y: (0.10, 0.88),
        crown_y: (0.27, 0.72),
        open_threshold: 250,
        mechanical_every: 12,
        frame: [143, 150, 150],
        spandrel: [72, 84, 88],
        glass: [80, 113, 133],
        accent: [169, 145, 110],
        roof: [91, 99, 100],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::ButtressedSupertall,
        slug: "buttressed-supertall",
        label: "Buttressed supertall · stepped wings and spire",
        period: "2000s–present",
        massing: TallMassing::Buttressed,
        floor_m: 3.75,
        bay_m: 3.0,
        middle_x: (0.15, 0.85),
        base_x: (0.08, 0.92),
        middle_y: (0.18, 0.80),
        base_y: (0.09, 0.89),
        crown_y: (0.24, 0.74),
        open_threshold: 252,
        mechanical_every: 15,
        frame: [149, 156, 157],
        spandrel: [62, 75, 81],
        glass: [76, 111, 134],
        accent: [196, 198, 192],
        roof: [91, 100, 103],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::CircularResidential,
        slug: "circular-residential",
        label: "Curved high-rise · radial balconies",
        period: "1960s–present",
        massing: TallMassing::Cylinder,
        floor_m: 3.15,
        bay_m: 3.0,
        middle_x: (0.18, 0.82),
        base_x: (0.12, 0.88),
        middle_y: (0.22, 0.77),
        base_y: (0.14, 0.84),
        crown_y: (0.30, 0.69),
        open_threshold: 247,
        mechanical_every: 0,
        frame: [156, 154, 146],
        spandrel: [108, 110, 106],
        glass: [70, 94, 111],
        accent: [171, 169, 160],
        roof: [104, 106, 103],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::SCurveSlab,
        slug: "s-curve-slab",
        label: "Curved high-rise · S-plan residential slab",
        period: "1950s–present",
        massing: TallMassing::SCurve,
        floor_m: 3.1,
        bay_m: 3.2,
        middle_x: (0.21, 0.79),
        base_x: (0.14, 0.86),
        middle_y: (0.24, 0.76),
        base_y: (0.16, 0.82),
        crown_y: (0.31, 0.68),
        open_threshold: 244,
        mechanical_every: 0,
        frame: [181, 176, 162],
        spandrel: [123, 128, 125],
        glass: [72, 99, 116],
        accent: [198, 193, 177],
        roof: [107, 110, 108],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::RippledBalcony,
        slug: "rippled-balcony-skyscraper",
        label: "Curved skyscraper · undulating balconies",
        period: "2000s–present",
        massing: TallMassing::Rippled,
        floor_m: 3.35,
        bay_m: 2.9,
        middle_x: (0.15, 0.85),
        base_x: (0.09, 0.91),
        middle_y: (0.19, 0.80),
        base_y: (0.10, 0.88),
        crown_y: (0.25, 0.74),
        open_threshold: 251,
        mechanical_every: 18,
        frame: [174, 177, 173],
        spandrel: [69, 83, 91],
        glass: [74, 110, 132],
        accent: [166, 168, 164],
        roof: [94, 102, 104],
    },
    TallBuildingProfile {
        id: TallBuildingStyle::SculptedCrescent,
        slug: "sculpted-crescent-skyscraper",
        label: "Curved skyscraper · stone-and-glass crescent",
        period: "2000s–present",
        massing: TallMassing::Crescent,
        floor_m: 3.8,
        bay_m: 3.2,
        middle_x: (0.17, 0.83),
        base_x: (0.10, 0.90),
        middle_y: (0.20, 0.78),
        base_y: (0.11, 0.87),
        crown_y: (0.29, 0.70),
        open_threshold: 248,
        mechanical_every: 14,
        frame: [183, 177, 160],
        spandrel: [74, 86, 92],
        glass: [73, 112, 137],
        accent: [199, 194, 179],
        roof: [101, 105, 103],
    },
];

pub fn tall_building_profile(style: TallBuildingStyle) -> &'static TallBuildingProfile {
    &TALL_BUILDING_PROFILES[style as usize]
}

/// Systems whose distant facade is primarily read as a glazed envelope.
/// Masonry, concrete and residential balcony families retain their authored
/// wall colour instead of receiving the curtain-wall distance treatment.
pub const fn is_glass_dominant(style: TallBuildingStyle) -> bool {
    matches!(
        style,
        TallBuildingStyle::InternationalSlab
            | TallBuildingStyle::CurtainWall
            | TallBuildingStyle::FramedTube
            | TallBuildingStyle::BundledTube
            | TallBuildingStyle::Diagrid
            | TallBuildingStyle::Twisting
            | TallBuildingStyle::TerracedMixedUse
            | TallBuildingStyle::ButtressedSupertall
            | TallBuildingStyle::RippledBalcony
            | TallBuildingStyle::SculptedCrescent
    )
}

fn srgb_channel_to_linear(channel: u8) -> f32 {
    let value = f32::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_channel_to_srgb(channel: f32) -> u8 {
    let value = if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    };
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Blue-sky reflectance target used only once detailed panes are represented
/// by a coarser facade patch. It is intentionally brighter than the close-up
/// glass albedo: the latter describes the pane, while this value preserves
/// the reflected-sky energy that sub-pixel mullions would otherwise erase.
pub fn distant_glass_color(style: TallBuildingStyle) -> [u8; 3] {
    let glass = tall_building_profile(style).glass;
    [glass[0].max(104), glass[1].max(142), glass[2].max(168)]
}

/// Linear-light area average for a minified curtain-wall patch. This preserves
/// dark structural frames without losing the energy and blue cast of the much
/// larger glazed area at L16-L18.
pub fn distant_facade_color(style: TallBuildingStyle) -> [u8; 3] {
    let profile = tall_building_profile(style);
    let frame = crate::art_direction::architecture_surface(crate::material::WALL, profile.frame);
    let spandrel =
        crate::art_direction::architecture_surface(crate::material::WALL, profile.spandrel);
    if !is_glass_dominant(style) {
        return frame;
    }

    let opening_coverage = ((profile.middle_x.1 - profile.middle_x.0)
        * (profile.middle_y.1 - profile.middle_y.0) as f32
        * (f32::from(profile.open_threshold) / 255.0))
        .clamp(0.32, 0.58);
    let reflected_glass = distant_glass_color(style);
    let frame_floor = [68u8, 76, 82];
    let mut out = [0u8; 3];
    for channel in 0..3 {
        let frame = frame[channel].max(frame_floor[channel]);
        let structure =
            srgb_channel_to_linear(frame) * 0.78 + srgb_channel_to_linear(spandrel[channel]) * 0.22;
        let average = structure * (1.0 - opening_coverage)
            + srgb_channel_to_linear(reflected_glass[channel]) * opening_coverage;
        out[channel] = linear_channel_to_srgb(average);
    }
    out[0] = out[0].max(92);
    out[1] = out[1].max(116);
    out[2] = out[2].max(136);
    out[2] = out[2].max(out[0].saturating_add(30));
    out
}

/// Select a complete high-rise system from the height and shape facts retained
/// by the production vector pipeline. Seed is only a stable tie-breaker among
/// geometrically plausible systems.
pub fn select_tall_building_style(
    height_m: f32,
    shape: TallBuildingShape,
    seed: u64,
) -> Option<TallBuildingStyle> {
    if height_m < 24.0 {
        return None;
    }

    let mapped_curve = shape.vertex_count >= 8;
    let mapped_concavity = shape.vertex_count >= 10 && shape.fill_ratio <= 0.72;
    if mapped_curve {
        if height_m >= 170.0 && mapped_concavity {
            return Some(TallBuildingStyle::SculptedCrescent);
        }
        if height_m < 170.0 && mapped_concavity {
            return Some(TallBuildingStyle::SCurveSlab);
        }
        if shape.compactness >= 0.70 && shape.aspect <= 1.35 {
            return Some(TallBuildingStyle::CircularResidential);
        }
    }

    if height_m >= 260.0 {
        return Some(match seed & 3 {
            0 => TallBuildingStyle::ButtressedSupertall,
            1 => TallBuildingStyle::BundledTube,
            2 => TallBuildingStyle::Diagrid,
            _ => TallBuildingStyle::Twisting,
        });
    }
    if height_m >= 170.0 {
        return Some(match seed & 3 {
            0 => TallBuildingStyle::BundledTube,
            1 => TallBuildingStyle::Diagrid,
            2 => TallBuildingStyle::Twisting,
            _ => TallBuildingStyle::FramedTube,
        });
    }
    if height_m >= 105.0 {
        return Some(match seed % 5 {
            0 => TallBuildingStyle::FramedTube,
            1 => TallBuildingStyle::Diagrid,
            2 => TallBuildingStyle::TerracedMixedUse,
            3 => TallBuildingStyle::CurtainWall,
            _ => TallBuildingStyle::Twisting,
        });
    }
    if shape.aspect >= 2.35 || shape.area_m2 >= 2_400.0 {
        return Some(TallBuildingStyle::InternationalSlab);
    }
    Some(match seed % 4 {
        0 => TallBuildingStyle::CurtainWall,
        1 => TallBuildingStyle::InternationalSlab,
        2 => TallBuildingStyle::TerracedMixedUse,
        _ => TallBuildingStyle::PostmodernCrown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_shape(area_m2: f32, aspect: f32) -> TallBuildingShape {
        TallBuildingShape {
            area_m2,
            aspect,
            compactness: 0.78,
            vertex_count: 4,
            fill_ratio: 1.0,
        }
    }

    #[test]
    fn low_rise_does_not_get_a_tower_profile() {
        assert_eq!(
            select_tall_building_style(18.0, simple_shape(900.0, 1.0), 1),
            None
        );
    }

    #[test]
    fn mapped_curvature_selects_curved_families_without_seed_guessing() {
        let circular = TallBuildingShape {
            area_m2: 900.0,
            aspect: 1.05,
            compactness: 0.90,
            vertex_count: 24,
            fill_ratio: 0.82,
        };
        let sinuous = TallBuildingShape {
            area_m2: 1_800.0,
            aspect: 2.1,
            compactness: 0.45,
            vertex_count: 32,
            fill_ratio: 0.58,
        };
        for seed in 0..16 {
            assert_eq!(
                select_tall_building_style(95.0, circular, seed),
                Some(TallBuildingStyle::CircularResidential)
            );
            assert_eq!(
                select_tall_building_style(120.0, sinuous, seed),
                Some(TallBuildingStyle::SCurveSlab)
            );
            assert_eq!(
                select_tall_building_style(
                    260.0,
                    TallBuildingShape {
                        fill_ratio: 0.80,
                        ..circular
                    },
                    seed,
                ),
                Some(TallBuildingStyle::CircularResidential)
            );
            assert_eq!(
                select_tall_building_style(320.0, sinuous, seed),
                Some(TallBuildingStyle::SculptedCrescent)
            );
        }
        assert!(!matches!(
            select_tall_building_style(100.0, simple_shape(900.0, 1.0), 0),
            Some(
                TallBuildingStyle::CircularResidential
                    | TallBuildingStyle::SCurveSlab
                    | TallBuildingStyle::RippledBalcony
                    | TallBuildingStyle::SculptedCrescent
            )
        ));
    }

    #[test]
    fn every_style_has_a_unique_slug_and_restrained_palette() {
        let mut slugs = std::collections::HashSet::new();
        for profile in TALL_BUILDING_PROFILES {
            assert!(slugs.insert(profile.slug));
            for color in [
                profile.frame,
                profile.spandrel,
                profile.glass,
                profile.accent,
                profile.roof,
            ] {
                assert!(color.iter().all(|channel| (32..=200).contains(channel)));
                let chroma = color.iter().max().unwrap() - color.iter().min().unwrap();
                assert!(
                    chroma <= 76,
                    "oversaturated {} colour: {color:?}",
                    profile.slug
                );
            }
        }
    }

    #[test]
    fn high_rise_glass_keeps_a_visible_blue_reflective_floor() {
        for profile in TALL_BUILDING_PROFILES {
            assert!(
                profile.glass[0] >= 65,
                "{} glass is too dark: {:?}",
                profile.slug,
                profile.glass
            );
            assert!(
                profile.glass[2] >= profile.glass[0] + 25,
                "{} glass is not visibly blue: {:?}",
                profile.slug,
                profile.glass
            );
        }
    }

    #[test]
    fn minified_glass_systems_keep_an_area_averaged_blue_floor() {
        for profile in TALL_BUILDING_PROFILES {
            let distant = distant_facade_color(profile.id);
            if is_glass_dominant(profile.id) {
                assert!(
                    distant[0] >= 92 && distant[1] >= 116 && distant[2] >= 136,
                    "{} distant facade is too dark: {distant:?}",
                    profile.slug
                );
                assert!(
                    distant[2] >= distant[0] + 30,
                    "{} distant facade lost its blue cast: {distant:?}",
                    profile.slug
                );
            } else {
                assert_eq!(
                    distant,
                    crate::art_direction::architecture_surface(
                        crate::material::WALL,
                        profile.frame
                    ),
                    "{} is not a glass-envelope system",
                    profile.slug
                );
            }
        }
    }
}
