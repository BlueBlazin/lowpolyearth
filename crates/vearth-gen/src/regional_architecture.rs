//! Source-anchored construction choices for ordinary buildings.
//!
//! Administrative families are priors, not observations of an individual
//! building's age, material or condition. Settlement data chooses the mix;
//! mapped geometry and reported roof/use facts remain authoritative.

use crate::architecture_catalog::{PaletteFamily, ResidentialForm};
use crate::architecture_styles::ArchitectureStyle;
use crate::material;
use crate::tall_buildings::{TallBuildingShape, TallBuildingStyle};
use vearth_core::ellipsoid::MEAN_RADIUS;
use vearth_data::vectors::{Building, BuildingUse, RoofShape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstructionSystem {
    Masonry,
    Rendered,
    Timber,
    Concrete,
    Glazed,
    Industrial,
    Earth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailFamily {
    Plain,
    Classical,
    Timber,
    Shaded,
    Balcony,
    Modern,
    Industrial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildingDesign {
    pub style: ArchitectureStyle,
    /// Authored geometric expression, attenuated for incompatible use and scale.
    pub expression: u8,
    pub use_kind: BuildingUse,
    /// Resolved facade representation; source use remains independent.
    pub facade: Option<crate::highrise::System>,
    pub construction: ConstructionSystem,
    pub detail: DetailFamily,
    pub wall: [u8; 3],
    pub roof: [u8; 3],
    pub accent: [u8; 3],
    pub joinery: [u8; 3],
    pub glass: [u8; 3],
    pub wall_material: u8,
    pub roof_material: u8,
    pub wall_variant: u8,
    pub roof_variant: u8,
    /// Authored finish variation, independent of nationality or income.
    pub weathering: u8,
    pub inferred_roof: RoofShape,
}

impl Default for BuildingDesign {
    fn default() -> Self {
        Self {
            style: ArchitectureStyle::Contemporary,
            expression: 0,
            use_kind: BuildingUse::Unknown,
            facade: None,
            construction: ConstructionSystem::Rendered,
            detail: DetailFamily::Plain,
            wall: [195, 190, 178],
            roof: [118, 116, 111],
            accent: [221, 216, 201],
            joinery: [72, 78, 70],
            glass: [62, 70, 69],
            wall_material: material::PLASTER_WALL,
            roof_material: material::ROOF,
            wall_variant: 0,
            roof_variant: 0,
            weathering: 24,
            inferred_roof: RoofShape::Flat,
        }
    }
}

/// Ingested features preserve their source anchor before local f32 projection.
/// Synthetic callers may reconstruct an approximate anchor from full bounds;
/// cell-clipped bounds and camera position must never choose a regional style.
pub fn building_location(building: &Building, origin: (f64, f64)) -> (f64, f64) {
    if let Some([lat, lon]) = building.location_rad {
        return (lat, lon);
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for point in building.rings.iter().flatten() {
        for axis in 0..2 {
            lo[axis] = lo[axis].min(f64::from(point[axis]));
            hi[axis] = hi[axis].max(f64::from(point[axis]));
        }
    }
    if !lo.into_iter().chain(hi).all(f64::is_finite) {
        return origin;
    }
    let lat = (origin.0 + (lo[1] + hi[1]) * 0.5 / MEAN_RADIUS)
        .clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
    let lon = origin.1 + (lo[0] + hi[0]) * 0.5 / (MEAN_RADIUS * origin.0.cos().abs().max(1e-6));
    (
        lat,
        (lon + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI,
    )
}

pub use crate::architecture_resolver::building_design;

pub(crate) fn construction_for_material(material: u8) -> ConstructionSystem {
    match material {
        material::BRICK_WALL
        | material::STONE_WALL
        | material::WEATHERED_LIMESTONE
        | material::VEINED_MARBLE => ConstructionSystem::Masonry,
        material::TIMBER_WALL | material::SIDING_WALL => ConstructionSystem::Timber,
        material::PANEL_WALL => ConstructionSystem::Concrete,
        material::EARTHEN_PLASTER => ConstructionSystem::Earth,
        _ => ConstructionSystem::Rendered,
    }
}

pub fn design_for_form(
    form: ResidentialForm,
    palette: PaletteFamily,
    state: u64,
) -> BuildingDesign {
    use ResidentialForm::*;
    let slot = ((state >> 12) & 3) as usize;
    let mut d = BuildingDesign::default();
    d.wall = palette_colors(palette)[slot];
    d.accent = d.wall.map(|c| (u16::from(c) * 3 / 4 + 55).min(237) as u8);
    // Condition variation is shared globally. A minority receives stronger
    // fading; every region includes maintained and recently finished stock.
    d.weathering = if state % 7 == 0 {
        75 + ((state >> 24) & 31) as u8
    } else {
        8 + ((state >> 24) & 31) as u8
    };
    d.inferred_roof = if state & 4 == 0 {
        RoofShape::Hipped
    } else {
        RoofShape::Gabled
    };
    d.roof_material = material::TILE_ROOF;
    d.roof = [[147, 91, 65], [167, 112, 78], [126, 83, 65], [164, 100, 73]][slot];
    match form {
        BrickPitch | BrickSlab => {
            d.construction = ConstructionSystem::Masonry;
            d.detail = DetailFamily::Plain;
            d.wall_material = material::BRICK_WALL;
            d.wall = [
                [151, 94, 71],
                [167, 118, 84],
                [183, 143, 106],
                [129, 89, 73],
            ][slot];
            d.accent = [206, 196, 172];
        }
        StonePitch | StoneSlab => {
            d.construction = ConstructionSystem::Masonry;
            d.detail = DetailFamily::Plain;
            d.wall_material = material::STONE_WALL;
            d.wall = [
                [167, 157, 137],
                [191, 180, 153],
                [148, 151, 143],
                [186, 174, 150],
            ][slot];
        }
        TimberPitch | TimberMetal => {
            d.construction = ConstructionSystem::Timber;
            d.detail = DetailFamily::Timber;
            d.wall_material = material::SIDING_WALL;
            d.wall = if palette == PaletteFamily::Timber {
                [
                    [135, 66, 49],
                    [201, 194, 171],
                    [109, 137, 138],
                    [168, 140, 84],
                ][slot]
            } else {
                d.wall
            };
        }
        RenderPitch => {
            d.detail = DetailFamily::Plain;
        }
        TropicalRenderPitch | CourtyardRender => {
            d.detail = DetailFamily::Shaded;
        }
        RenderSlab => {
            d.detail = if state & 8 == 0 {
                DetailFamily::Shaded
            } else {
                DetailFamily::Plain
            };
        }
        MasonryMetal => {
            d.detail = DetailFamily::Shaded;
        }
        EarthFlat | EarthPitch => {
            d.construction = ConstructionSystem::Earth;
            d.wall_material = material::EARTHEN_PLASTER;
            d.detail = DetailFamily::Plain;
            d.wall = [
                [177, 137, 92],
                [190, 151, 108],
                [157, 117, 80],
                [206, 175, 129],
            ][slot];
            d.accent = [172, 142, 107];
        }
    }
    if matches!(form, MasonryMetal | TimberMetal) {
        d.roof_material = material::METAL_ROOF;
        d.roof = [
            [153, 165, 167],
            [117, 139, 143],
            [157, 103, 83],
            [108, 121, 115],
        ][slot];
    } else if matches!(form, TimberPitch | StonePitch | BrickPitch)
        && matches!(
            palette,
            PaletteFamily::CoolMasonry | PaletteFamily::RedBrick | PaletteFamily::Timber
        )
    {
        d.roof_material = material::SLATE_ROOF;
        d.roof = [[77, 85, 91], [105, 107, 104], [93, 99, 105], [106, 93, 85]][slot];
    }
    if matches!(
        form,
        RenderSlab | BrickSlab | StoneSlab | EarthFlat | CourtyardRender
    ) {
        d.inferred_roof = RoofShape::Flat;
        d.roof_material = material::ROOF;
        d.roof = [144, 137, 122];
    }
    d
}

fn palette_colors(palette: PaletteFamily) -> [[u8; 3]; 4] {
    match palette {
        PaletteFamily::CoolMasonry => [
            [199, 199, 187],
            [171, 181, 179],
            [215, 208, 187],
            [176, 165, 147],
        ],
        PaletteFamily::WarmMasonry => [
            [210, 190, 156],
            [186, 160, 130],
            [222, 207, 179],
            [199, 174, 146],
        ],
        PaletteFamily::RedBrick => [
            [185, 138, 102],
            [207, 190, 158],
            [160, 110, 89],
            [202, 180, 155],
        ],
        PaletteFamily::Ochre => [
            [205, 175, 130],
            [183, 142, 102],
            [223, 202, 162],
            [195, 168, 142],
        ],
        PaletteFamily::LightRender => [
            [221, 218, 202],
            [202, 196, 177],
            [230, 220, 198],
            [191, 202, 196],
        ],
        PaletteFamily::TropicalPastel => [
            [214, 202, 175],
            [182, 200, 186],
            [216, 185, 164],
            [184, 193, 199],
        ],
        PaletteFamily::Timber => [
            [172, 146, 112],
            [208, 197, 174],
            [139, 124, 99],
            [166, 178, 170],
        ],
        PaletteFamily::Earth => [
            [185, 148, 109],
            [202, 174, 134],
            [167, 137, 105],
            [209, 188, 157],
        ],
        PaletteFamily::Neutral => [
            [200, 196, 183],
            [177, 181, 177],
            [213, 203, 182],
            [185, 177, 164],
        ],
    }
}

pub fn tall_style(
    design: BuildingDesign,
    height: f32,
    shape: TallBuildingShape,
    _seed: u64,
) -> Option<TallBuildingStyle> {
    let system = design.facade?;
    if height < 36.0 || !height.is_finite() {
        return None;
    }
    if !system.residential() {
        return Some(
            if system == crate::highrise::System::StoneOffice || shape.aspect >= 2.35 {
                TallBuildingStyle::InternationalSlab
            } else {
                TallBuildingStyle::CurtainWall
            },
        );
    }
    if shape.vertex_count >= 8 && shape.compactness >= 0.70 && shape.aspect <= 1.35 {
        return Some(TallBuildingStyle::CircularResidential);
    }
    Some(if system == crate::highrise::System::ConcreteFrame {
        TallBuildingStyle::Brutalist
    } else {
        TallBuildingStyle::ResidentialLoggia
    })
}

pub fn roof_profile(
    building: &Building,
    design: BuildingDesign,
) -> (Option<RoofShape>, Option<f32>) {
    let span = building.height_m - building.min_height_m;
    let roof = design.inferred_roof;
    let rise = (roof != RoofShape::Flat).then(|| {
        if building.kind == vearth_data::vectors::BuildingKind::RoofOnly {
            (span - 0.6).max(0.0)
        } else {
            let (fraction, cap) = match roof {
                // These are inferred roof volumes within the reported source top.
                // Explicit roof facts keep the ordinary source-shape policy.
                _ if building.roof_shape.is_none()
                    && matches!(roof, RoofShape::Gabled | RoofShape::Hipped)
                    && design.style == ArchitectureStyle::ShirakawaOgimachi =>
                {
                    (0.52, 8.0)
                }
                _ if building.roof_shape.is_none()
                    && roof == RoofShape::Pyramidal
                    && design.style == ArchitectureStyle::AlberobelloTrulli =>
                {
                    // The selected cone is a building volume, independent of
                    // how much facade ornament survives the height fade.
                    (0.48, 4.5)
                }
                RoofShape::Mansard | RoofShape::Gambrel => (0.28, 7.2),
                _ if design.expression >= 96 => (0.24, 6.4),
                _ => (0.18, 5.5),
            };
            (span * fraction).clamp(0.6, cap).min((span - 2.0).max(0.0))
        }
    });
    (Some(roof), rise)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_data::vectors::BuildingKind;

    fn source(height: f32, seed: u64) -> Building {
        Building {
            location_rad: Some([18.5204_f64.to_radians(), 73.8567_f64.to_radians()]),
            rings: vec![vec![[-8.0, -6.0], [8.0, -6.0], [8.0, 6.0], [-8.0, 6.0]]],
            height_m: height,
            min_height_m: 0.0,
            seed,
            kind: BuildingKind::Building,
            roof_shape: None,
            use_kind: BuildingUse::Residential,
        }
    }

    fn shape() -> TallBuildingShape {
        TallBuildingShape {
            area_m2: 192.0,
            aspect: 1.33,
            compactness: 0.76,
            vertex_count: 4,
            fill_ratio: 1.0,
        }
    }

    #[test]
    fn source_anchor_makes_design_independent_of_query_origin_and_cell_clipping() {
        let mut building = source(11.0, 27);
        let expected = building_design(&building, (0.0, 0.0), shape());
        for origin in [(0.0, 0.0), (1.0, -2.5), (-0.5, 3.0)] {
            for p in building.rings.iter_mut().flatten() {
                p[0] += 50_000.0;
                p[1] -= 30_000.0;
            }
            assert_eq!(building_design(&building, origin, shape()), expected);
        }
    }

    #[test]
    fn observed_use_prevents_civic_and_commercial_residential_balconies() {
        for use_kind in [
            BuildingUse::Commercial,
            BuildingUse::Civic,
            BuildingUse::Industrial,
        ] {
            for height in [8.0, 23.0, 48.0, 150.0] {
                for seed in 0..24 {
                    let mut building = source(height, seed);
                    building.use_kind = use_kind;
                    let design = building_design(&building, (0.0, 0.0), shape());
                    assert_ne!(design.detail, DetailFamily::Balcony);
                    assert!(!matches!(
                        tall_style(design, height, shape(), seed),
                        Some(
                            TallBuildingStyle::ResidentialLoggia
                                | TallBuildingStyle::CircularResidential
                        )
                    ));
                    if use_kind == BuildingUse::Industrial {
                        assert_eq!(design.wall_material, material::PANEL_WALL);
                        assert_eq!(design.roof_material, material::METAL_ROOF);
                    }
                }
            }
        }
    }

    #[test]
    fn local_roof_volumes_respect_observed_shape_and_source_height() {
        for (style, roof) in [
            (ArchitectureStyle::ShirakawaOgimachi, RoofShape::Gabled),
            (ArchitectureStyle::AlberobelloTrulli, RoofShape::Pyramidal),
        ] {
            let mut building = source(8.0, 0);
            let design = BuildingDesign {
                style,
                expression: 255,
                inferred_roof: roof,
                ..Default::default()
            };
            let (_, rise) = roof_profile(&building, design);
            assert!(rise.unwrap() > 3.5 && rise.unwrap() <= building.height_m - 2.0);
            building.roof_shape = Some(roof);
            let (_, observed_rise) = roof_profile(&building, design);
            assert!(observed_rise.unwrap() < rise.unwrap());
        }
    }

    #[test]
    fn vernacular_roof_volumes_survive_facade_expression_fade() {
        for (style, roof) in [
            (ArchitectureStyle::AlberobelloTrulli, RoofShape::Pyramidal),
            (ArchitectureStyle::ShirakawaOgimachi, RoofShape::Gabled),
            (ArchitectureStyle::ShirakawaOgimachi, RoofShape::Hipped),
        ] {
            for height in [5.0, 10.0, 13.5, 18.0] {
                let mut building = source(height, 0);
                let mut design = BuildingDesign {
                    style,
                    inferred_roof: roof,
                    expression: 255,
                    ..Default::default()
                };
                let expected = roof_profile(&building, design);
                for expression in [0, 64, 95, 96, 170] {
                    design.expression = expression;
                    assert_eq!(roof_profile(&building, design), expected);
                }
                assert!(expected.1.unwrap() <= height - 2.0);
                building.roof_shape = Some(roof);
                assert!(roof_profile(&building, design).1.unwrap() < expected.1.unwrap());
                building.kind = BuildingKind::RoofOnly;
                assert_eq!(roof_profile(&building, design).1, Some(height - 0.6));
                design.inferred_roof = RoofShape::Flat;
                assert_eq!(
                    roof_profile(&building, design),
                    (Some(RoofShape::Flat), None)
                );
            }
        }
    }

    #[test]
    fn observed_roofs_override_every_regional_prior_without_changing_source_height() {
        for height in [3.0, 12.0, 40.0, 100.0] {
            for roof in [
                RoofShape::Flat,
                RoofShape::Hipped,
                RoofShape::Gabled,
                RoofShape::Mansard,
                RoofShape::Dome,
                RoofShape::Sawtooth,
                RoofShape::Onion,
            ] {
                let mut building = source(height, 4);
                building.roof_shape = Some(roof);
                let design = building_design(&building, (0.0, 0.0), shape());
                let (actual, rise) = roof_profile(&building, design);
                assert_eq!(actual, Some(roof));
                assert!(rise.unwrap_or(0.0) <= height - 2.0);
                assert_eq!(building.height_m, height);
            }
        }
    }

    #[test]
    fn missing_geographic_context_is_neutral_and_large_roofs_do_not_imply_industry() {
        let mut building = source(9.0, 32);
        building.location_rad = Some([0.0, 0.0]);
        building.use_kind = BuildingUse::Unknown;
        let design = building_design(&building, (0.0, 0.0), shape());
        assert_eq!(design.construction, ConstructionSystem::Rendered);
        assert_eq!(design.inferred_roof, RoofShape::Flat);
        let mut large = shape();
        large.area_m2 = 4_000.0;
        let design = building_design(&building, (0.0, 0.0), large);
        assert_ne!(design.construction, ConstructionSystem::Industrial);
    }

    #[test]
    fn tall_rural_housing_stays_masonry_and_medium_brick_retains_its_material() {
        for seed in 0..64 {
            let mut building = source(90.0, seed);
            building.location_rad = Some([42.5_f64.to_radians(), (-75.8_f64).to_radians()]);
            let design = building_design(&building, (0.0, 0.0), shape());
            assert!(!matches!(
                design.construction,
                ConstructionSystem::Earth | ConstructionSystem::Timber | ConstructionSystem::Glazed
            ));
            assert_eq!(design.inferred_roof, RoofShape::Flat);
        }
        for state in 0..64 {
            let design = design_for_form(
                ResidentialForm::BrickSlab,
                PaletteFamily::WarmMasonry,
                state,
            );
            assert_eq!(design.wall_material, material::BRICK_WALL);
            assert_ne!(design.detail, DetailFamily::Classical);
        }
    }
}
