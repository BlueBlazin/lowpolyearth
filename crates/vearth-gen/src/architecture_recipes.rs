//! Complete, evidence-scoped construction archetypes compiled from source JSON.
//!
//! Country and subdivision defaults are expanded at build time. A selected
//! archetype owns its wall, roof, trim and geometry choices; runtime resolution
//! cannot borrow finishes from another construction or geographic entry.

use crate::architecture_catalog::ResidentialForm;
use crate::architecture_choices::{Weighted, choose, choose_where};
use crate::architecture_styles::ArchitectureStyle;
use crate::material;
use crate::regional_architecture::{BuildingDesign, ConstructionSystem, DetailFamily};
use vearth_data::admin_regions::AdminRegionContext;
use vearth_data::settlement::SettlementClass;
use vearth_data::vectors::{BuildingUse, RoofShape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finish {
    pub material: u8,
    pub color: [u8; 3],
    pub variant: u8,
}

/// Explicit source-use/scale admission. The inherited stock permits small
/// shops/civic buildings sharing ordinary street fabric, not office towers.
#[derive(Clone, Copy, Debug)]
pub struct Applicability {
    pub uses: u8,
    pub min_height_m: f32,
    pub max_height_m: f32,
    pub min_area_m2: f32,
    pub max_area_m2: f32,
    pub ordinary_stock: bool,
}

impl Applicability {
    pub const ORDINARY_STOCK: Self = Self {
        uses: 15,
        min_height_m: 0.0,
        max_height_m: f32::MAX,
        min_area_m2: 0.0,
        max_area_m2: f32::MAX,
        ordinary_stock: true,
    };

    pub fn permits(self, use_kind: BuildingUse, height: f32, area: f32) -> bool {
        let bit = match use_kind {
            BuildingUse::Unknown => 1,
            BuildingUse::Residential => 2,
            BuildingUse::Commercial => 4,
            BuildingUse::Civic => 8,
            BuildingUse::Industrial => 16,
            BuildingUse::Utility => return false,
        };
        self.uses & bit != 0
            && height.is_finite()
            && area.is_finite()
            && height >= self.min_height_m
            && height <= self.max_height_m
            && area >= self.min_area_m2
            && area <= self.max_area_m2
            && (!self.ordinary_stock
                || matches!(use_kind, BuildingUse::Unknown | BuildingUse::Residential)
                || (height < 19.0 && area < 900.0))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BuildingArchetype {
    pub name: &'static str,
    pub form: ResidentialForm,
    pub style: ArchitectureStyle,
    pub expression: u8,
    pub applicability: Applicability,
    pub facade: Option<crate::highrise::System>,
    pub roof_shapes: &'static [Weighted<RoofShape>],
    pub walls: &'static [Weighted<Finish>],
    pub roofs: &'static [Weighted<Finish>],
    pub flat_roofs: &'static [Weighted<Finish>],
    pub trim: &'static [Weighted<[u8; 3]>],
    pub joinery: &'static [Weighted<[u8; 3]>],
}

#[derive(Clone, Copy, Debug)]
pub struct CountryArchitecture {
    pub country: &'static str,
    pub label: &'static str,
    pub urban: &'static [Weighted<&'static BuildingArchetype>],
    pub town: &'static [Weighted<&'static BuildingArchetype>],
    pub rural: &'static [Weighted<&'static BuildingArchetype>],
    /// Explicit masonry fallback for source scales absent from domestic stock.
    pub infill: &'static BuildingArchetype,
}

pub struct RegionalArchitecture {
    pub country: &'static str,
    pub names: &'static [&'static str],
    pub profile: CountryArchitecture,
}

/// Complete alternatives bound to an existing geographic place identity.
/// Geographic admission remains owned by `architecture_places`.
pub struct PlaceArchitecture {
    pub source_id: &'static str,
    pub style_key: &'static str,
    pub archetypes: &'static [Weighted<&'static BuildingArchetype>],
}

include!("../assets/architecture/countries-generated.rs");

pub fn for_region(context: AdminRegionContext) -> Option<&'static CountryArchitecture> {
    // Generated entries are grouped in country-name order, so catalog growth
    // does not require scanning other countries for each source building.
    let first = REGIONAL_ARCHITECTURE.partition_point(|entry| entry.country < context.country_name);
    REGIONAL_ARCHITECTURE[first..]
        .iter()
        .take_while(|entry| entry.country == context.country_name)
        .find(|entry| entry.names.contains(&context.name))
        .map(|entry| &entry.profile)
}

/// Sorted static lookup allocates nothing per source building.
pub fn for_country(country: &str) -> Option<&'static CountryArchitecture> {
    COUNTRY_ARCHITECTURE
        .binary_search_by_key(&country, |entry| entry.country)
        .ok()
        .map(|index| &COUNTRY_ARCHITECTURE[index])
}

pub fn for_place(source_id: &str, style_key: &str) -> Option<&'static PlaceArchitecture> {
    PLACE_ARCHITECTURE
        .binary_search_by_key(&(source_id, style_key), |entry| {
            (entry.source_id, entry.style_key)
        })
        .ok()
        .map(|index| &PLACE_ARCHITECTURE[index])
}

impl PlaceArchitecture {
    pub(crate) fn archetype(
        &self,
        state: u64,
        compatible: impl Fn(&BuildingArchetype) -> bool,
    ) -> Option<&'static BuildingArchetype> {
        choose_where(self.archetypes, state, 0x464f524d, compatible)
    }
}

impl CountryArchitecture {
    pub(crate) fn archetype(
        &self,
        settlement: SettlementClass,
        state: u64,
        compatible: impl Fn(&BuildingArchetype) -> bool,
    ) -> Option<&'static BuildingArchetype> {
        let mix = match settlement {
            SettlementClass::City => self.urban,
            SettlementClass::Town | SettlementClass::Suburban | SettlementClass::Unknown => {
                self.town
            }
            SettlementClass::Rural | SettlementClass::Village => self.rural,
        };
        choose_where(mix, state, 0x464f524d, &compatible)
            .or_else(|| compatible(self.infill).then_some(self.infill))
    }
}

impl BuildingArchetype {
    pub(crate) fn suggested_roof(&self, state: u64) -> RoofShape {
        choose(self.roof_shapes, state, 0x5348415045).unwrap_or(RoofShape::Flat)
    }

    pub(crate) fn resolve(&self, roof: RoofShape, state: u64) -> BuildingDesign {
        let wall = choose(self.walls, state, 0x57414c4c).unwrap_or(Finish {
            material: material::PLASTER_WALL,
            color: [195, 195, 190],
            variant: 0,
        });
        let coverings = if roof == RoofShape::Flat {
            self.flat_roofs
        } else {
            self.roofs
        };
        let covering = choose(coverings, state, 0x524f4f46).unwrap_or(Finish {
            material: if roof == RoofShape::Flat {
                material::FLAT_ROOF
            } else {
                material::ROOF
            },
            color: [120, 120, 120],
            variant: 0,
        });
        let detail = match self.form {
            ResidentialForm::TimberPitch | ResidentialForm::TimberMetal => DetailFamily::Timber,
            ResidentialForm::TropicalRenderPitch
            | ResidentialForm::CourtyardRender
            | ResidentialForm::MasonryMetal => DetailFamily::Shaded,
            _ => DetailFamily::Plain,
        };
        BuildingDesign {
            style: self.style,
            expression: self.expression,
            facade: self.facade,
            detail,
            // Construction eligibility belongs to the archetype. A visible
            // plank finish can be fiber cement on a multi-storey slab building.
            construction: match self.form {
                ResidentialForm::RenderSlab => ConstructionSystem::Concrete,
                ResidentialForm::TimberPitch | ResidentialForm::TimberMetal => {
                    ConstructionSystem::Timber
                }
                ResidentialForm::EarthFlat | ResidentialForm::EarthPitch => {
                    ConstructionSystem::Earth
                }
                ResidentialForm::BrickPitch
                | ResidentialForm::BrickSlab
                | ResidentialForm::StonePitch
                | ResidentialForm::StoneSlab
                | ResidentialForm::MasonryMetal => ConstructionSystem::Masonry,
                ResidentialForm::RenderPitch
                | ResidentialForm::CourtyardRender
                | ResidentialForm::TropicalRenderPitch => ConstructionSystem::Rendered,
            },
            wall: wall.color,
            wall_material: wall.material,
            wall_variant: wall.variant,
            roof: covering.color,
            roof_material: covering.material,
            roof_variant: covering.variant,
            accent: choose(self.trim, state, 0x5452494d).unwrap_or(wall.color),
            joinery: choose(self.joinery, state, 0x4a4f494e).unwrap_or([80, 80, 75]),
            inferred_roof: roof,
            weathering: 8 + ((state >> 24) & 31) as u8,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use vearth_data::admin_regions::AdminRegionAtlas;

    #[test]
    fn every_source_country_has_complete_positive_weight_archetypes() {
        let expected: BTreeSet<_> = AdminRegionAtlas::bundled()
            .contexts()
            .map(|c| c.country_name)
            .collect();
        let actual: BTreeSet<_> = COUNTRY_ARCHITECTURE.iter().map(|c| c.country).collect();
        assert_eq!(expected, actual);
        assert_eq!(actual.len(), COUNTRY_ARCHITECTURE.len());
        assert!(
            REGIONAL_ARCHITECTURE
                .windows(2)
                .all(|pair| pair[0].country <= pair[1].country)
        );
        for region in REGIONAL_ARCHITECTURE {
            for name in region.names {
                let context = AdminRegionAtlas::bundled()
                    .contexts()
                    .find(|c| c.country_name == region.country && c.name == *name)
                    .expect("compiled subdivision must exist in the source atlas");
                assert!(std::ptr::eq(for_region(context).unwrap(), &region.profile));
            }
        }
        for entry in COUNTRY_ARCHITECTURE {
            assert_eq!(for_country(entry.country).unwrap().label, entry.label);
            for mix in [entry.urban, entry.town, entry.rural] {
                assert!(!mix.is_empty());
                for choice in mix {
                    assert!(choice.weight > 0);
                    let a = choice.value;
                    assert!(
                        !a.walls.is_empty()
                            && !a.flat_roofs.is_empty()
                            && !a.roof_shapes.is_empty()
                    );
                    assert!(
                        a.walls
                            .iter()
                            .all(|f| f.weight > 0
                                && material::is_architecture_wall(f.value.material))
                    );
                    assert!(
                        a.roofs
                            .iter()
                            .all(|f| f.weight > 0
                                && material::is_architecture_roof(f.value.material))
                    );
                }
            }
        }
        assert!(for_country("Unknown future country").is_none());
    }

    #[test]
    fn selected_archetype_owns_material_pigment_and_source_roof_compatibility() {
        for profile in COUNTRY_ARCHITECTURE
            .iter()
            .chain(REGIONAL_ARCHITECTURE.iter().map(|r| &r.profile))
        {
            for choice in profile
                .urban
                .iter()
                .chain(profile.town)
                .chain(profile.rural)
            {
                let a = choice.value;
                for state in 0..16 {
                    let design = a.resolve(RoofShape::Gabled, state);
                    let expected = choose(a.walls, state, 0x57414c4c).unwrap();
                    assert_eq!(
                        (design.wall_material, design.wall_variant, design.wall),
                        (expected.material, expected.variant, expected.color)
                    );
                    let flat = a.resolve(RoofShape::Flat, state);
                    assert_eq!(flat.roof_material, material::FLAT_ROOF);
                    assert_eq!(
                        flat.roof,
                        choose(a.flat_roofs, state, 0x524f4f46).unwrap().color
                    );
                }
            }
        }
    }
}
