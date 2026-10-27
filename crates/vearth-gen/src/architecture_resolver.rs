//! Select exactly one catalog entry before resolving a building's appearance.
//!
//! Local studies, named places, subdivisions and countries are alternatives,
//! not mutable finish layers. Source use and scale constrain eligibility;
//! observed roof shape is resolved before the selected entry chooses covering.

use crate::architecture_catalog::ResidentialForm;
use crate::architecture_places::PlaceInfluence;
use crate::architecture_recipes::BuildingArchetype;
use crate::architecture_styles::{ArchitectureStyle, ExpressionClass, ProjectionKind, Roofline};
use crate::material;
use crate::regional_architecture::{
    BuildingDesign, ConstructionSystem, DetailFamily, building_location, construction_for_material,
};
use crate::tall_buildings::TallBuildingShape;
use vearth_data::admin_regions::{AdminRegionAtlas, AdminRegionContext};
use vearth_data::climate::ClimateAtlas;
use vearth_data::settlement::{SettlementAtlas, SettlementClass};
use vearth_data::vectors::{Building, BuildingKind, BuildingUse, RoofShape};

/// Selected catalog identity, available to the same renderer's review gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignSource {
    Local {
        key: &'static str,
        fabric: &'static str,
    },
    Place {
        id: &'static str,
        style: &'static str,
    },
    Subdivision {
        country: &'static str,
        label: &'static str,
    },
    Country {
        country: &'static str,
        label: &'static str,
    },
    Industrial,
    Glazed,
    NonDomestic,
    Highrise,
    Unmapped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedBuildingDesign {
    pub design: BuildingDesign,
    pub source: DesignSource,
    pub archetype: &'static str,
}

/// Geospatial facts can be supplied explicitly by controlled catalog reviews.
/// Production obtains these from the source feature's stable geographic anchor.
#[derive(Clone, Copy, Debug)]
pub struct ArchitectureContext {
    pub location: (f64, f64),
    pub administrative: Option<AdminRegionContext>,
    pub settlement: SettlementClass,
}

impl ArchitectureContext {
    pub fn at(location: (f64, f64)) -> Self {
        Self {
            location,
            administrative: AdminRegionAtlas::bundled().context_at(location.0, location.1),
            settlement: SettlementAtlas::bundled().class_at(location.0, location.1),
        }
    }
}

fn hash(mut state: u64) -> u64 {
    state = (state ^ (state >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    state = (state ^ (state >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    state ^ (state >> 31)
}

#[derive(Clone, Copy)]
struct Requirements {
    industrial: bool,
    glazed: bool,
    large: bool,
    domestic: bool,
    use_kind: BuildingUse,
    height_m: f32,
    area_m2: f32,
    highrise_height: Option<f32>,
}

impl Requirements {
    fn from_source(building: &Building, shape: TallBuildingShape) -> Self {
        let industrial = matches!(
            building.use_kind,
            BuildingUse::Industrial | BuildingUse::Utility
        );
        let span = building.height_m - building.min_height_m;
        let height = if building.kind.is_part() {
            building.height_m
        } else {
            span
        };
        Self {
            industrial,
            use_kind: building.use_kind,
            height_m: building.height_m,
            area_m2: shape.area_m2,
            highrise_height: (building.kind != BuildingKind::RoofOnly
                && !industrial
                && span >= 2.4
                && height.is_finite()
                && height >= crate::highrise::MIN_HEIGHT_M)
                .then_some(height),
            glazed: !industrial
                && building.height_m >= 60.0
                && building.use_kind == BuildingUse::Commercial,
            large: building.height_m >= 19.0 || shape.area_m2 >= 900.0,
            domestic: matches!(
                building.use_kind,
                BuildingUse::Residential | BuildingUse::Unknown
            ) && building.kind != BuildingKind::RoofOnly,
        }
    }

    fn permits(self, archetype: &BuildingArchetype) -> bool {
        if !archetype
            .applicability
            .permits(self.use_kind, self.height_m, self.area_m2)
            || (self.use_kind == BuildingUse::Residential
                && archetype.facade.is_some_and(|system| !system.residential()))
        {
            return false;
        }
        // Ordinary-stock scope supplies no evidence for a tower envelope.
        // Explicit use/scale admission may describe a high-rise, including an
        // authored non-panel facade; the generic prior is a separate provider.
        if self.highrise_height.is_some() && archetype.applicability.ordinary_stock {
            return false;
        }
        if self.large {
            !matches!(
                archetype.form,
                ResidentialForm::TimberPitch
                    | ResidentialForm::TimberMetal
                    | ResidentialForm::EarthFlat
                    | ResidentialForm::EarthPitch
            )
        } else {
            true
        }
    }

    fn roof(self, building: &Building, suggested: RoofShape) -> RoofShape {
        if let Some(observed) = building.roof_shape {
            return observed;
        }
        if building.kind == BuildingKind::RoofOnly
            && (building.height_m - building.min_height_m < 1.8 || building.height_m > 28.0)
        {
            return RoofShape::Flat;
        }
        if building.use_kind == BuildingUse::Utility {
            return RoofShape::Flat;
        }
        if self.industrial {
            return RoofShape::Gabled;
        }
        if building.height_m >= 28.0 || (self.large && suggested == RoofShape::Skillion) {
            RoofShape::Flat
        } else {
            suggested
        }
    }
}

// An enum makes competing providers mutually exclusive. Resolving one arm
// never calls another provider or writes into a previously resolved design.
enum CatalogChoice {
    Local(crate::regional_finishes::LocalRecipe),
    Place {
        influence: PlaceInfluence,
        infill: bool,
    },
    Archetype {
        archetype: &'static BuildingArchetype,
        source: DesignSource,
    },
    Generic(DesignSource),
    Highrise(crate::highrise::System),
}

enum PlaceRecipe {
    Archetype(&'static BuildingArchetype),
    Legacy,
}

impl CatalogChoice {
    fn select(
        context: ArchitectureContext,
        requirements: Requirements,
        building: &Building,
        shape: TallBuildingShape,
        state: u64,
    ) -> Self {
        if requirements.industrial {
            return Self::Generic(DesignSource::Industrial);
        }
        if requirements.domestic && !requirements.large {
            if let Some(recipe) = crate::regional_finishes::recipe_at(context.location, state) {
                return Self::Local(recipe);
            }
        }
        if building.kind != BuildingKind::RoofOnly {
            let admission = (hash(state ^ 0x4c4f_4341_4c53_5459) >> 40) as f32 / 16_777_216.0;
            let selected = crate::architecture_places::select_at(
                context.location.0,
                context.location.1,
                |place| {
                    // A local study owns its identity's geography as well as
                    // its finishes; the superseded broad extent cannot return.
                    if crate::regional_finishes::owns_place(place.source_id, place.style_key)
                        || (place.scope_tier == crate::architecture_places::PlaceScope::City
                            && matches!(
                                context.settlement,
                                SettlementClass::Rural | SettlementClass::Village
                            ))
                        || building.height_m > place.max_height_m
                        || shape.area_m2 > place.max_footprint_m2
                    {
                        return None;
                    }
                    if let Some(profile) =
                        crate::architecture_recipes::for_place(place.source_id, place.style_key)
                    {
                        return profile
                            .archetype(state, |a| requirements.permits(a))
                            .map(PlaceRecipe::Archetype);
                    }
                    if requirements.highrise_height.is_some()
                        || (!requirements.domestic && requirements.large)
                        || (requirements.large
                            && place.style.profile().wall_material.is_some_and(|m| {
                                matches!(
                                    m,
                                    material::TIMBER_WALL
                                        | material::SIDING_WALL
                                        | material::EARTHEN_PLASTER
                                )
                            }))
                    {
                        return None;
                    }
                    Some(PlaceRecipe::Legacy)
                },
                |place| admission < place.strength * place.palette_confidence,
            );
            if let Some((place, archetype)) = selected {
                if let PlaceRecipe::Archetype(archetype) = archetype {
                    return Self::Archetype {
                        archetype,
                        source: DesignSource::Place {
                            id: place.source_id,
                            style: place.style_key,
                        },
                    };
                }
                let infill = (hash(state ^ 0x494e_4649_4c4c_4558) >> 40) as f32 / 16_777_216.0
                    < place.contemporary_infill;
                return Self::Place {
                    influence: place,
                    infill,
                };
            }
        }
        if let Some(admin) = context.administrative {
            for (profile, regional) in [
                (crate::architecture_recipes::for_region(admin), true),
                (
                    crate::architecture_recipes::for_country(admin.country_name),
                    false,
                ),
            ] {
                if let Some(profile) = profile {
                    if let Some(archetype) =
                        profile.archetype(context.settlement, state, |a| requirements.permits(a))
                    {
                        let source = if regional {
                            DesignSource::Subdivision {
                                country: profile.country,
                                label: profile.label,
                            }
                        } else {
                            DesignSource::Country {
                                country: profile.country,
                                label: profile.label,
                            }
                        };
                        return Self::Archetype { archetype, source };
                    }
                }
            }
        }
        if let Some(height) = requirements.highrise_height {
            return Self::Highrise(crate::highrise::fallback_system(
                building.use_kind,
                height,
                building.seed,
                building.height_m >= 60.0
                    && context.settlement == SettlementClass::City
                    && state % 5 == 0,
            ));
        }
        if requirements.glazed {
            return Self::Generic(DesignSource::Glazed);
        }
        if !requirements.domestic {
            return Self::Generic(DesignSource::NonDomestic);
        }
        Self::Generic(DesignSource::Unmapped)
    }

    fn suggested_roof(&self, state: u64) -> RoofShape {
        match self {
            Self::Local(recipe) => recipe.suggested_roof(state),
            Self::Place { influence, infill } => {
                if *infill {
                    RoofShape::Flat
                } else {
                    influence
                        .style
                        .profile()
                        .inferred_roof
                        .unwrap_or(RoofShape::Gabled)
                }
            }
            Self::Archetype { archetype, .. } => archetype.suggested_roof(state),
            Self::Generic(DesignSource::Industrial) => RoofShape::Gabled,
            Self::Generic(_) => RoofShape::Flat,
            Self::Highrise(_) => RoofShape::Flat,
        }
    }

    fn resolve(self, roof: RoofShape, state: u64) -> ResolvedBuildingDesign {
        let (design, source, archetype) = match self {
            Self::Local(recipe) => (
                recipe.resolve(roof, state),
                DesignSource::Local {
                    key: recipe.key(),
                    fabric: recipe.fabric_name(),
                },
                recipe.fabric_name(),
            ),
            Self::Place { influence, infill } => (
                resolve_place(influence, infill, roof, state),
                DesignSource::Place {
                    id: influence.source_id,
                    style: influence.style_key,
                },
                if infill {
                    "named-place contemporary infill"
                } else {
                    influence.style_key
                },
            ),
            Self::Archetype { archetype, source } => {
                (archetype.resolve(roof, state), source, archetype.name)
            }
            Self::Generic(source) => (
                resolve_generic(source, roof, state),
                source,
                match source {
                    DesignSource::Industrial => "industrial envelope",
                    DesignSource::Glazed => "glazed envelope",
                    DesignSource::NonDomestic => "non-domestic mineral envelope",
                    _ => "unspecified envelope",
                },
            ),
            Self::Highrise(system) => {
                let profile = system.profile();
                (
                    BuildingDesign {
                        construction: if system.residential() {
                            ConstructionSystem::Concrete
                        } else {
                            ConstructionSystem::Glazed
                        },
                        detail: DetailFamily::Modern,
                        facade: Some(system),
                        wall_material: material::PANEL_WALL,
                        wall: profile.wall,
                        accent: profile.trim,
                        joinery: profile.trim,
                        glass: profile.glass,
                        inferred_roof: roof,
                        roof_material: if roof == RoofShape::Flat {
                            material::FLAT_ROOF
                        } else {
                            material::METAL_ROOF
                        },
                        roof: [139, 149, 152],
                        weathering: 8,
                        ..Default::default()
                    },
                    DesignSource::Highrise,
                    "procedural high-rise envelope",
                )
            }
        };
        ResolvedBuildingDesign {
            design,
            source,
            archetype,
        }
    }
}

fn resolve_place(
    place: PlaceInfluence,
    infill: bool,
    roof: RoofShape,
    state: u64,
) -> BuildingDesign {
    let profile = place.style.profile();
    let colors = place.style.colors(hash(state ^ 0x5041_4c45_5454_4553));
    let wall_material = profile.wall_material.unwrap_or(material::PLASTER_WALL);
    let construction = construction_for_material(wall_material);
    let detail = if infill {
        DetailFamily::Plain
    } else if matches!(
        profile.projection,
        ProjectionKind::TimberBay | ProjectionKind::Gallery
    ) && construction == ConstructionSystem::Timber
    {
        DetailFamily::Timber
    } else if profile.projection == ProjectionKind::Balcony {
        DetailFamily::Balcony
    } else if matches!(profile.roofline, Roofline::DeepEave | Roofline::TieredHip)
        || profile.projection == ProjectionKind::ScreenBay
    {
        DetailFamily::Shaded
    } else if profile.expression == ExpressionClass::Heritage {
        DetailFamily::Classical
    } else {
        DetailFamily::Plain
    };
    BuildingDesign {
        style: if infill {
            ArchitectureStyle::Contemporary
        } else {
            place.style
        },
        expression: if infill {
            0
        } else {
            (place.character_confidence * 255.0).round() as u8
        },
        construction,
        detail,
        wall_material,
        roof_material: if roof == RoofShape::Flat {
            material::FLAT_ROOF
        } else {
            profile.roof_material.unwrap_or(material::ROOF)
        },
        wall: colors.wall,
        accent: colors.trim,
        joinery: colors.joinery,
        glass: colors.glass,
        // A pitched covering's pigment says nothing about an unobserved flat
        // terrace. Legacy place entries without that role use neutral concrete.
        roof: if roof == RoofShape::Flat {
            [145, 145, 140]
        } else {
            colors.roof
        },
        inferred_roof: roof,
        ..Default::default()
    }
}

fn resolve_generic(source: DesignSource, roof: RoofShape, state: u64) -> BuildingDesign {
    let (construction, detail, wall_material, wall, pitched_material) = match source {
        DesignSource::Industrial => (
            ConstructionSystem::Industrial,
            DetailFamily::Industrial,
            material::PANEL_WALL,
            [180, 184, 181],
            material::METAL_ROOF,
        ),
        DesignSource::NonDomestic => (
            ConstructionSystem::Concrete,
            DetailFamily::Modern,
            material::PANEL_WALL,
            [179, 175, 163],
            material::METAL_ROOF,
        ),
        DesignSource::Glazed => (
            ConstructionSystem::Glazed,
            DetailFamily::Modern,
            material::PANEL_WALL,
            [149, 159, 164],
            material::METAL_ROOF,
        ),
        _ => (
            ConstructionSystem::Rendered,
            DetailFamily::Plain,
            material::PLASTER_WALL,
            [195, 195, 190],
            material::ROOF,
        ),
    };
    let facade = match source {
        DesignSource::Glazed => Some(match state % 4 {
            0 => crate::highrise::System::ClearOffice,
            1 => crate::highrise::System::SilverOffice,
            2 => crate::highrise::System::BronzeOffice,
            _ => crate::highrise::System::HorizontalOffice,
        }),
        DesignSource::NonDomestic => Some(crate::highrise::System::StoneOffice),
        _ => None,
    };
    let palette = facade.map(crate::highrise::System::profile);
    BuildingDesign {
        construction,
        detail,
        facade,
        wall_material,
        wall: palette.map_or(wall, |p| p.wall),
        glass: palette.map_or([62, 70, 69], |p| p.glass),
        joinery: palette.map_or([72, 78, 70], |p| p.trim),
        inferred_roof: roof,
        roof_material: if roof == RoofShape::Flat {
            material::FLAT_ROOF
        } else {
            pitched_material
        },
        roof: [139, 149, 152],
        weathering: 8 + ((state >> 24) & 31) as u8,
        ..Default::default()
    }
}

pub fn building_design(
    building: &Building,
    origin: (f64, f64),
    shape: TallBuildingShape,
) -> BuildingDesign {
    resolve_building_design(
        building,
        ArchitectureContext::at(building_location(building, origin)),
        shape,
    )
    .design
}

/// The sole production resolver; controlled reviews call the same function.
/// After a provider resolves surfaces, only geometric expression and exposure
/// are constrained here. No downstream catalog may repaint the selected result.
pub fn resolve_building_design(
    building: &Building,
    context: ArchitectureContext,
    shape: TallBuildingShape,
) -> ResolvedBuildingDesign {
    let state = hash(building.seed ^ 0x454e_5630_3642_4c44);
    let requirements = Requirements::from_source(building, shape);
    let choice = CatalogChoice::select(context, requirements, building, shape, state);
    let roof = requirements.roof(building, choice.suggested_roof(state));
    let mut resolved = choice.resolve(roof, state);
    let design = &mut resolved.design;
    design.use_kind = building.use_kind;
    if requirements.large && design.construction == ConstructionSystem::Rendered {
        design.construction = ConstructionSystem::Concrete;
    }
    let max_height = design.style.profile().expression_height_m;
    let span = (building.height_m - building.min_height_m).max(0.0);
    let t = ((building.height_m - max_height) / (max_height * 0.65).max(0.01)).clamp(0.0, 1.0);
    let expression_scale = (1.0 - t * t * (3.0 - 2.0 * t)) * (span / 3.5).clamp(0.0, 1.0);
    design.expression = (f32::from(design.expression) * expression_scale).round() as u8;
    if building.kind == BuildingKind::RoofOnly {
        design.expression = 0;
    }
    if !requirements.domestic && !requirements.industrial {
        design.detail = DetailFamily::Modern;
        if building.use_kind == BuildingUse::Civic {
            design.expression = design.expression.min(76);
        }
    } else if requirements.large && !requirements.industrial && !requirements.glazed {
        design.detail = if state % 5 < 3 {
            DetailFamily::Balcony
        } else {
            DetailFamily::Modern
        };
    }
    if requirements.highrise_height.is_none()
        || (design.expression >= 96
            && design.style.profile().expression != ExpressionClass::Contemporary)
    {
        design.facade = None;
    }
    if let Some(system) = design.facade {
        // The packed system owns one of the existing glass palettes. CPU and
        // GPU share those constants; opaque wall and frame remain authored.
        design.glass = system.profile().glass;
    }
    let climate = ClimateAtlas::bundled().annual_surface(context.location.0, context.location.1);
    let exposure = (climate.precipitation_mm_day / 8.0).clamp(0.0, 1.0)
        * climate.relative_humidity.clamp(0.0, 1.0);
    design.weathering = (f32::from(design.weathering) * (0.90 + 0.20 * exposure)).round() as u8;
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(seed: u64) -> (Building, TallBuildingShape) {
        (
            Building {
                location_rad: None,
                rings: vec![vec![[-4., -4.], [4., -4.], [4., 4.], [-4., 4.]]],
                height_m: 8.,
                min_height_m: 0.,
                seed,
                kind: BuildingKind::Building,
                roof_shape: None,
                use_kind: BuildingUse::Residential,
            },
            TallBuildingShape {
                area_m2: 64.,
                aspect: 1.,
                compactness: 0.78,
                vertex_count: 4,
                fill_ratio: 1.,
            },
        )
    }

    #[test]
    fn unscoped_housing_recipes_cannot_replace_highrise_families() {
        for (lat, lon) in [(-33.8677_f64, 151.211_f64), (40.7545, -73.98)] {
            let context = ArchitectureContext::at((lat.to_radians(), lon.to_radians()));
            let mut seen = [false; 8];
            for seed in 0..128 {
                let (mut building, mut shape) = source(seed);
                building.use_kind = BuildingUse::Unknown;
                building.height_m = 120.0;
                shape.area_m2 = 1600.0;
                let resolved = resolve_building_design(&building, context, shape);
                let system = resolved.design.facade.unwrap();
                seen[system as usize] = true;
                let urban_glazed = context.settlement == SettlementClass::City
                    && hash(seed ^ 0x454e_5630_3642_4c44) % 5 == 0;
                assert_eq!(system.residential(), seed % 4 == 0 && !urban_glazed);
                assert_eq!(resolved.design.wall, system.profile().wall);
                assert_eq!(resolved.design.joinery, system.profile().trim);
                assert_eq!(resolved.design.use_kind, BuildingUse::Unknown);
            }
            assert!(
                seen[..5].iter().all(|v| *v),
                "all office systems remain reachable"
            );
        }
    }

    #[test]
    fn highrise_selection_preserves_source_parts_roofs_and_use() {
        let context =
            ArchitectureContext::at((-33.8677_f64.to_radians(), 151.211_f64.to_radians()));
        for use_kind in [
            BuildingUse::Unknown,
            BuildingUse::Residential,
            BuildingUse::Commercial,
            BuildingUse::Civic,
            BuildingUse::Industrial,
        ] {
            for roof in [
                None,
                Some(RoofShape::Flat),
                Some(RoofShape::Gabled),
                Some(RoofShape::Dome),
            ] {
                for (kind, top, base, tall) in [
                    (BuildingKind::Building, 35.9, 0.0, false),
                    (BuildingKind::Building, 36.0, 0.0, true),
                    (BuildingKind::Building, 120.0, 100.0, false),
                    (BuildingKind::Part, 120.0, 100.0, true),
                    (BuildingKind::Part, 120.0, 119.0, false),
                    (BuildingKind::RoofOnly, 120.0, 100.0, false),
                ] {
                    let (mut building, shape) = source(1);
                    building.use_kind = use_kind;
                    building.kind = kind;
                    building.height_m = top;
                    building.min_height_m = base;
                    building.roof_shape = roof;
                    let resolved = resolve_building_design(&building, context, shape);
                    assert_eq!(
                        resolved.design.facade.is_some(),
                        tall && use_kind != BuildingUse::Industrial
                    );
                    assert_eq!(resolved.design.use_kind, use_kind);
                    if let Some(roof) = roof {
                        assert_eq!(resolved.design.inferred_roof, roof);
                    }
                    if let Some(system) = resolved.design.facade {
                        assert_eq!(resolved.source, DesignSource::Highrise);
                        assert_eq!(resolved.design.wall, system.profile().wall);
                        assert_eq!(resolved.design.joinery, system.profile().trim);
                        if use_kind == BuildingUse::Residential {
                            assert!(system.residential());
                        }
                        if matches!(use_kind, BuildingUse::Commercial | BuildingUse::Civic) {
                            assert!(!system.residential());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn local_recipe_owns_the_entire_result_even_inside_an_alpine_country_and_named_place() {
        let context = ArchitectureContext::at((46.3692_f64.to_radians(), 14.1136_f64.to_radians()));
        for seed in 0..128 {
            let (building, shape) = source(seed);
            let state = hash(seed ^ 0x454e_5630_3642_4c44);
            let local = crate::regional_finishes::recipe_at(context.location, state).unwrap();
            let resolved = resolve_building_design(&building, context, shape);
            assert_eq!(
                resolved.source,
                DesignSource::Local {
                    key: local.key(),
                    fabric: local.fabric_name()
                }
            );
            let expected = local.resolve(local.suggested_roof(state), state);
            assert_eq!(
                (
                    resolved.design.wall,
                    resolved.design.roof,
                    resolved.design.wall_material,
                    resolved.design.roof_material,
                    resolved.design.wall_variant,
                    resolved.design.roof_variant,
                    resolved.design.style,
                    resolved.design.accent,
                    resolved.design.joinery
                ),
                (
                    expected.wall,
                    expected.roof,
                    expected.wall_material,
                    expected.roof_material,
                    expected.wall_variant,
                    expected.roof_variant,
                    expected.style,
                    expected.accent,
                    expected.joinery
                )
            );
        }
    }

    #[test]
    fn reviewed_named_places_own_finishes_and_respect_observed_roof_shape() {
        for (id, style, lat, lon) in [
            (
                "wikidata:Q1165566",
                "havana-vieja",
                23.1359444_f64,
                -82.3583333_f64,
            ),
            (
                "wikidata:Q9054598",
                "cartagena-balconies",
                10.423166667,
                -75.55158333,
            ),
        ] {
            let context = ArchitectureContext::at((lat.to_radians(), lon.to_radians()));
            let profile = crate::architecture_recipes::for_place(id, style).unwrap();
            let mut selected = 0;
            for seed in 0..128 {
                for roof in [RoofShape::Flat, RoofShape::Gabled] {
                    let (mut building, shape) = source(seed);
                    building.roof_shape = Some(roof);
                    let resolved = resolve_building_design(&building, context, shape);
                    if resolved.source != (DesignSource::Place { id, style }) {
                        continue;
                    }
                    selected += 1;
                    let state = hash(seed ^ 0x454e_5630_3642_4c44);
                    let expected = profile
                        .archetype(state, |_| true)
                        .unwrap()
                        .resolve(roof, state);
                    assert_eq!(resolved.design.wall_material, material::PLASTER_WALL);
                    assert_eq!(resolved.design.wall, expected.wall);
                    assert_eq!(resolved.design.roof, expected.roof);
                    assert_eq!(resolved.design.roof_material, expected.roof_material);
                    assert_eq!(resolved.design.inferred_roof, roof);
                    assert_eq!(resolved.design.joinery, expected.joinery);
                }
            }
            assert!(
                selected > 192,
                "the reviewed place must be reached in its core"
            );
        }
    }

    #[test]
    fn london_city_fallback_respects_pimlico_specificity_and_source_scale() {
        let (lat, lon) = (51.4887_f64, -0.1395_f64);
        let district_source = DesignSource::Place {
            id: "wikidata:Q123306",
            style: "london-pimlico",
        };
        let city_source = DesignSource::Place {
            id: "geonames:2643743",
            style: "contemporary",
        };
        let core = ArchitectureContext::at((lat.to_radians(), lon.to_radians()));
        // A point in the district's fading edge must retain a stronger city
        // fallback without letting that city's strength reorder specificity.
        let edge = (1..256)
            .find_map(|step| {
                let t = f64::from(step) / 256.0;
                let location = (
                    (lat + t * (51.48683588655 - lat)).to_radians(),
                    (lon + t * (-0.14494380020 - lon)).to_radians(),
                );
                crate::architecture_places::influences_at(location.0, location.1)
                    .into_iter()
                    .flatten()
                    .any(|p| p.style_key == "london-pimlico" && (0.1..0.3).contains(&p.strength))
                    .then(|| ArchitectureContext::at(location))
            })
            .expect("Pimlico has an inward transition toward the adjacent estate");
        let city =
            crate::architecture_recipes::for_place("geonames:2643743", "contemporary").unwrap();
        let mut core_district = 0;
        let mut edge_district = 0;
        let mut edge_city = 0;
        let mut large_city = 0;
        let mut aggregate_terrace = 0;
        for seed in 0..256 {
            let (mut building, mut shape) = source(seed);
            let resolved = resolve_building_design(&building, core, shape);
            core_district += usize::from(resolved.source == district_source);
            assert_ne!(resolved.source, city_source);
            let resolved = resolve_building_design(&building, edge, shape);
            edge_district += usize::from(resolved.source == district_source);
            edge_city += usize::from(resolved.source == city_source);

            building.height_m = 18.0;
            building.roof_shape = Some(RoofShape::Flat);
            shape.area_m2 = 6000.0;
            let resolved = resolve_building_design(&building, core, shape);
            aggregate_terrace += usize::from(resolved.source == district_source);

            building.height_m = 30.0;
            building.roof_shape = Some(RoofShape::Flat);
            shape.area_m2 = 2000.0;
            let resolved = resolve_building_design(&building, core, shape);
            assert_ne!(resolved.source, district_source);
            if resolved.source == city_source {
                large_city += 1;
                let state = hash(seed ^ 0x454e_5630_3642_4c44);
                let expected = city
                    .archetype(state, |_| true)
                    .unwrap()
                    .resolve(RoofShape::Flat, state);
                assert_eq!(resolved.design.wall, expected.wall);
                assert_eq!(resolved.design.roof, expected.roof);
                assert_eq!(resolved.design.wall_material, expected.wall_material);
                assert_eq!(resolved.design.inferred_roof, RoofShape::Flat);
            }
        }
        assert!(core_district > 192);
        assert!(edge_district > 0 && edge_city > edge_district);
        assert!(large_city > 192);
        assert!(aggregate_terrace > 192);
    }

    #[test]
    fn city_recipes_exclude_rural_settlements_without_excluding_named_districts() {
        let context =
            ArchitectureContext::at((51.4887_f64.to_radians(), (-0.1395_f64).to_radians()));
        let city_source = DesignSource::Place {
            id: "geonames:2643743",
            style: "contemporary",
        };
        let district_source = DesignSource::Place {
            id: "wikidata:Q123306",
            style: "london-pimlico",
        };
        let influences =
            crate::architecture_places::influences_at(context.location.0, context.location.1);
        for (id, tier) in [
            (
                "geonames:2643743",
                crate::architecture_places::PlaceScope::City,
            ),
            (
                "wikidata:Q123306",
                crate::architecture_places::PlaceScope::District,
            ),
        ] {
            assert!(
                influences
                    .iter()
                    .flatten()
                    .any(|p| p.source_id == id && p.scope_tier == tier)
            );
        }

        for (settlement, city_eligible) in [
            (SettlementClass::City, true),
            (SettlementClass::Town, true),
            (SettlementClass::Suburban, true),
            (SettlementClass::Rural, false),
            (SettlementClass::Village, false),
        ] {
            let context = ArchitectureContext {
                settlement,
                ..context
            };
            let mut city_count = 0;
            let mut district_count = 0;
            for seed in 0..128 {
                let (mut building, mut shape) = source(seed);
                let district = resolve_building_design(&building, context, shape);
                district_count += usize::from(district.source == district_source);

                // The same location and admission seeds reach the city recipe
                // when source height makes the narrower terrace study ineligible.
                building.height_m = 30.0;
                building.roof_shape = Some(RoofShape::Flat);
                shape.area_m2 = 2000.0;
                let city = resolve_building_design(&building, context, shape);
                assert_ne!(city.source, district_source);
                city_count += usize::from(city.source == city_source);
                if !city_eligible {
                    assert!(matches!(
                        city.source,
                        DesignSource::Country { .. } | DesignSource::Subdivision { .. }
                    ));
                }
            }
            assert!(
                district_count > 96,
                "{settlement:?}: named district remains eligible"
            );
            if city_eligible {
                assert!(
                    city_count > 96,
                    "{settlement:?}: city stock remains eligible"
                );
            } else {
                assert_eq!(
                    city_count, 0,
                    "{settlement:?}: city stock must not replace rural stock"
                );
            }
        }
    }

    #[test]
    fn churchill_gardens_slab_uses_city_stock_instead_of_pimlico_terraces() {
        // Historic England's Gilbert House location lies near Pimlico's edge.
        // Its ten-storey source scale exceeds the terrace recipe's actual limit.
        let context = ArchitectureContext::at((
            51.48683588655_f64.to_radians(),
            (-0.14494380020_f64).to_radians(),
        ));
        let mut city_count = 0;
        for seed in 0..128 {
            let (mut building, mut shape) = source(seed);
            building.height_m = 30.0;
            building.roof_shape = Some(RoofShape::Flat);
            shape.area_m2 = 1000.0;
            let resolved = resolve_building_design(&building, context, shape);
            assert_ne!(
                resolved.source,
                DesignSource::Place {
                    id: "wikidata:Q123306",
                    style: "london-pimlico"
                }
            );
            city_count += usize::from(
                resolved.source
                    == DesignSource::Place {
                        id: "geonames:2643743",
                        style: "contemporary",
                    },
            );
            assert_eq!(resolved.design.inferred_roof, RoofShape::Flat);
        }
        assert!(city_count > 96);
    }

    #[test]
    fn mineral_plank_cladding_does_not_turn_a_slab_building_into_a_cottage() {
        let profile = crate::architecture_recipes::REGIONAL_ARCHITECTURE
            .iter()
            .find(|p| p.country == "Canada" && p.names.contains(&"British Columbia"))
            .unwrap();
        let archetype = profile
            .profile
            .urban
            .iter()
            .map(|a| a.value)
            .find(|a| {
                a.form == ResidentialForm::RenderSlab
                    && a.walls
                        .iter()
                        .any(|f| f.value.material == material::SIDING_WALL)
            })
            .unwrap();
        let (mut building, shape) = source(12);
        building.height_m = 24.;
        let requirements = Requirements::from_source(&building, shape);
        assert!(requirements.permits(archetype));
        assert_eq!(
            archetype.resolve(RoofShape::Flat, 12).construction,
            ConstructionSystem::Concrete
        );

        let bahamas = crate::architecture_recipes::for_country("The Bahamas").unwrap();
        let fiber_cement = bahamas
            .urban
            .iter()
            .map(|a| a.value)
            .find(|a| a.name == "Fiber-cement board stock")
            .unwrap();
        assert!(requirements.permits(fiber_cement));
        let design = fiber_cement.resolve(RoofShape::Gabled, 12);
        assert_eq!(design.wall_material, material::SIDING_WALL);
        assert_eq!(design.construction, ConstructionSystem::Rendered);
    }

    #[test]
    fn subdivision_is_selected_instead_of_national_before_any_finishes_are_sampled() {
        for region in crate::architecture_recipes::REGIONAL_ARCHITECTURE {
            let admin = AdminRegionAtlas::bundled()
                .contexts()
                .find(|c| c.country_name == region.country && region.names.contains(&c.name))
                .unwrap();
            let context = ArchitectureContext {
                location: (0., 0.),
                administrative: Some(admin),
                settlement: SettlementClass::Town,
            };
            for seed in 0..16 {
                let (building, shape) = source(seed);
                let resolved = resolve_building_design(&building, context, shape);
                assert_eq!(
                    resolved.source,
                    DesignSource::Subdivision {
                        country: region.country,
                        label: region.profile.label
                    }
                );
                let state = hash(seed ^ 0x454e_5630_3642_4c44);
                let archetype = region
                    .profile
                    .archetype(context.settlement, state, |_| true)
                    .unwrap();
                let roof = archetype.suggested_roof(state);
                let expected = archetype.resolve(roof, state);
                assert_eq!(resolved.design.wall, expected.wall);
                assert_eq!(resolved.design.roof, expected.roof);
            }
        }
    }

    #[test]
    fn source_constraints_select_compatible_recipe_and_covering_once() {
        for local in crate::regional_finishes::reviewed_finishes() {
            let context = ArchitectureContext::at((
                local.latitude_deg.to_radians(),
                local.longitude_deg.to_radians(),
            ));
            for use_kind in [
                BuildingUse::Residential,
                BuildingUse::Industrial,
                BuildingUse::Commercial,
                BuildingUse::Civic,
            ] {
                for height in [8., 24., 90.] {
                    for observed in [RoofShape::Flat, RoofShape::Gabled, RoofShape::Dome] {
                        let (mut building, shape) = source(3);
                        building.height_m = height;
                        building.use_kind = use_kind;
                        building.roof_shape = Some(observed);
                        let result = resolve_building_design(&building, context, shape);
                        assert_eq!(result.design.inferred_roof, observed);
                        assert_eq!(
                            result.design.roof_material == material::FLAT_ROOF,
                            observed == RoofShape::Flat
                        );
                        if height >= 19. || use_kind != BuildingUse::Residential {
                            assert!(!matches!(result.source, DesignSource::Local { .. }));
                        }
                        if use_kind == BuildingUse::Industrial {
                            assert_eq!(result.source, DesignSource::Industrial);
                            assert_eq!(result.design.wall_material, material::PANEL_WALL);
                        }
                        if height >= 19. {
                            assert!(!matches!(
                                result.design.construction,
                                ConstructionSystem::Timber | ConstructionSystem::Earth
                            ));
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn ordinary_stock_accepts_small_shops_but_not_office_tower_envelopes() {
        let context = ArchitectureContext::at((51.498_f64.to_radians(), (-0.02_f64).to_radians()));
        for seed in 0..128 {
            let (mut building, mut shape) = source(seed);
            building.use_kind = BuildingUse::Commercial;
            let small = resolve_building_design(&building, context, shape);
            assert!(matches!(
                small.source,
                DesignSource::Place { .. } | DesignSource::Country { .. }
            ));
            building.height_m = 45.0;
            shape.area_m2 = 1600.0;
            let large = resolve_building_design(&building, context, shape);
            assert_eq!(large.source, DesignSource::Highrise);
            assert_eq!(
                large.design.facade,
                Some(crate::highrise::fallback_system(
                    BuildingUse::Commercial,
                    45.0,
                    seed,
                    false
                ))
            );
            assert_ne!(large.design.wall_material, material::BRICK_WALL);
            assert_eq!(large.design.use_kind, BuildingUse::Commercial);
            assert_eq!(
                large.design.glass,
                large.design.facade.unwrap().profile().glass
            );
        }
    }

    #[test]
    fn observed_residential_use_survives_geometry_and_seed() {
        let context = ArchitectureContext::at((51.498_f64.to_radians(), (-0.02_f64).to_radians()));
        for use_kind in [BuildingUse::Residential] {
            for seed in 0..128 {
                let (mut building, mut shape) = source(seed);
                building.height_m = 120.0;
                building.use_kind = use_kind;
                shape.area_m2 = 1600.0;
                let resolved = resolve_building_design(&building, context, shape);
                let system = resolved.design.facade.unwrap();
                assert!(
                    system.residential(),
                    "domestic recipe was reclassified downstream"
                );
                assert_eq!(resolved.design.use_kind, use_kind);
                assert_eq!(
                    crate::highrise::resolve(resolved.design, 120.0, !seed),
                    Some(system)
                );
                assert!(!matches!(
                    crate::regional_architecture::tall_style(resolved.design, 120.0, shape, !seed),
                    Some(
                        crate::tall_buildings::TallBuildingStyle::CurtainWall
                            | crate::tall_buildings::TallBuildingStyle::InternationalSlab
                    )
                ));
            }
        }
    }

    #[test]
    fn canary_office_recipe_is_scoped_and_preserves_observed_residential_use() {
        for (lat, lon, office_core) in [
            (51.505_f64, -0.020_f64, true),
            (51.503, -0.013, false), // Wood Wharf mixed-use stock.
            (51.501, -0.020, false), // Marsh Wall housing.
            (51.507, -0.020, false), // North Quay.
        ] {
            let context = ArchitectureContext::at((lat.to_radians(), lon.to_radians()));
            for use_kind in [BuildingUse::Unknown, BuildingUse::Residential] {
                for seed in 0..64 {
                    let (mut building, mut shape) = source(seed);
                    building.height_m = 235.0;
                    building.use_kind = use_kind;
                    shape.area_m2 = 1600.0;
                    let resolved = resolve_building_design(&building, context, shape);
                    let is_office = !resolved.design.facade.unwrap().residential();
                    if use_kind == BuildingUse::Residential {
                        assert!(!is_office);
                        assert_eq!(resolved.source, DesignSource::Highrise);
                    } else if office_core {
                        assert!(is_office);
                        assert!(matches!(resolved.source, DesignSource::Place { .. }));
                    } else {
                        assert_eq!(resolved.source, DesignSource::Highrise);
                    }
                    assert_eq!(resolved.design.use_kind, use_kind);
                }
            }
        }
    }

    #[test]
    fn unmapped_highrises_keep_their_prior_without_relabeling_source_use() {
        let context = ArchitectureContext {
            location: (0.0, 0.0),
            administrative: None,
            settlement: SettlementClass::City,
        };
        for seed in 0..128 {
            let (mut building, mut shape) = source(seed);
            building.height_m = 120.0;
            building.use_kind = BuildingUse::Unknown;
            shape.area_m2 = 1600.0;
            let resolved = resolve_building_design(&building, context, shape);
            assert_eq!(resolved.source, DesignSource::Highrise);
            assert_eq!(
                resolved.design.facade.unwrap().residential(),
                seed % 4 == 0 && hash(seed ^ 0x454e_5630_3642_4c44) % 5 != 0
            );
            assert_eq!(resolved.design.use_kind, BuildingUse::Unknown);
        }
    }

    #[test]
    fn small_shops_can_share_local_timber_stock_without_admitting_tall_timber() {
        let mut archetype = *crate::architecture_recipes::for_country("Norway")
            .unwrap()
            .rural[0]
            .value;
        archetype.form = ResidentialForm::TimberPitch;
        let (mut building, mut shape) = source(0);
        building.use_kind = BuildingUse::Commercial;
        assert!(Requirements::from_source(&building, shape).permits(&archetype));
        building.height_m = 19.0;
        assert!(!Requirements::from_source(&building, shape).permits(&archetype));
        building.height_m = 8.0;
        shape.area_m2 = 900.0;
        assert!(!Requirements::from_source(&building, shape).permits(&archetype));
    }

    #[test]
    fn explicit_facade_admission_respects_source_use_scale_and_material() {
        use crate::architecture_recipes::Applicability;
        let mut archetype = *crate::architecture_recipes::for_country("United Kingdom")
            .unwrap()
            .urban[0]
            .value;
        archetype.form = ResidentialForm::RenderSlab;
        archetype.facade = Some(crate::highrise::System::SilverOffice);
        archetype.applicability = Applicability {
            uses: 1 | 4,
            min_height_m: 36.0,
            max_height_m: 200.0,
            min_area_m2: 400.0,
            max_area_m2: 5000.0,
            ordinary_stock: false,
        };
        let (mut building, mut shape) = source(4);
        building.height_m = 120.0;
        building.use_kind = BuildingUse::Commercial;
        shape.area_m2 = 1600.0;
        let requirements = |b: &Building, s: TallBuildingShape| Requirements::from_source(b, s);
        assert!(requirements(&building, shape).permits(&archetype));
        building.use_kind = BuildingUse::Unknown;
        assert!(requirements(&building, shape).permits(&archetype));
        building.use_kind = BuildingUse::Residential;
        assert!(!requirements(&building, shape).permits(&archetype));
        building.use_kind = BuildingUse::Commercial;
        building.height_m = 250.0;
        assert!(!requirements(&building, shape).permits(&archetype));
        building.height_m = 120.0;
        shape.area_m2 = 8000.0;
        assert!(!requirements(&building, shape).permits(&archetype));
        shape.area_m2 = 1600.0;
        archetype.form = ResidentialForm::TimberPitch;
        assert!(!requirements(&building, shape).permits(&archetype));
    }
}
