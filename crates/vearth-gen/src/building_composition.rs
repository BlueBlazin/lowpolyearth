use crate::regional_architecture::{BuildingDesign, DetailFamily};
use crate::tier0::building_metrics;
use vearth_data::vectors::{Building, BuildingKind, BuildingUse, RoofShape};

pub(crate) struct ComposedBuilding {
    pub parent: Option<usize>,
    pub design_source: usize,
    pub style_seed: u64,
    pub facade_openings: bool,
    pub height: f32,
}

fn compatible(a: BuildingUse, b: BuildingUse) -> bool {
    a == BuildingUse::Unknown || b == BuildingUse::Unknown || a == b
}

fn permits_openings(building: &Building, parent: Option<&Building>) -> bool {
    if building.kind == BuildingKind::RoofOnly || building.use_kind == BuildingUse::Utility {
        return false;
    }
    let shape = building_metrics(building);
    let perimeter: f32 = building.rings.first().map_or(0.0, |ring| {
        (0..ring.len())
            .map(|i| {
                let a = ring[i];
                let b = ring[(i + 1) % ring.len()];
                (a[0] - b[0]).hypot(a[1] - b[1])
            })
            .sum()
    });
    // Twice area/perimeter is a rotation-independent depth proxy: roughly
    // the width of a long fin, and half the width of a square service shaft.
    let depth = 2.0 * shape.area_m2 / perimeter.max(0.001);
    let span = building.height_m - building.min_height_m;
    if building.kind.is_part() && depth < 1.5 {
        return false;
    }
    let explicit_occupied = matches!(
        building.use_kind,
        BuildingUse::Residential | BuildingUse::Commercial | BuildingUse::Civic
    );
    if explicit_occupied {
        return true;
    }
    let industrial = building.use_kind == BuildingUse::Industrial
        || parent.is_some_and(|p| p.use_kind == BuildingUse::Industrial);
    if industrial && span >= 15.0 && shape.area_m2 <= 100.0 && depth <= 4.5 {
        return false;
    }
    if building.kind.is_part() && building.min_height_m >= 20.0 {
        if span < 2.4 || (shape.area_m2 <= 36.0 && depth <= 3.0) {
            return false;
        }
        if let Some(parent) = parent {
            let parent_area = building_metrics(parent).area_m2;
            if building.min_height_m >= parent.height_m * 0.7
                && shape.area_m2 <= 100.0
                && shape.area_m2 <= parent_area * 0.12
                && depth <= 4.5
            {
                return false;
            }
        }
    }
    true
}

pub(crate) fn resolve(buildings: &[Building]) -> Vec<ComposedBuilding> {
    let parents = crate::building_surface::composition_parents(buildings);
    let openings: Vec<_> = buildings
        .iter()
        .enumerate()
        .map(|(i, b)| permits_openings(b, parents[i].map(|j| &buildings[j])))
        .collect();
    let mut children = vec![Vec::new(); buildings.len()];
    for (i, parent) in parents.iter().enumerate() {
        if let Some(parent) = parent {
            children[*parent].push(i);
        }
    }
    // Prefer the outline's identity. If its height is only a low fallback,
    // a substantial occupied tower supplies the complex's facade family.
    let representatives: Vec<_> = buildings
        .iter()
        .enumerate()
        .map(|(i, source)| {
            children[i]
                .iter()
                .copied()
                .filter(|&j| {
                    let child = &buildings[j];
                    openings[j]
                        && compatible(source.use_kind, child.use_kind)
                        && child.height_m >= crate::highrise::MIN_HEIGHT_M
                        && source.height_m < crate::highrise::MIN_HEIGHT_M
                        && building_metrics(child).area_m2 >= 100.0
                })
                .max_by(|&a, &b| {
                    let volume = |j: usize| {
                        building_metrics(&buildings[j]).area_m2
                            * (buildings[j].height_m - buildings[j].min_height_m)
                    };
                    volume(a)
                        .total_cmp(&volume(b))
                        .then(buildings[b].seed.cmp(&buildings[a].seed))
                })
                .unwrap_or(i)
        })
        .collect();
    buildings
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let family = parents[i]
                .filter(|&p| compatible(source.use_kind, buildings[p].use_kind))
                .unwrap_or(i);
            let representative = representatives[family];
            let representative = if compatible(source.use_kind, buildings[representative].use_kind)
            {
                representative
            } else {
                i
            };
            let template = &buildings[representative];
            let parts: Vec<_> = children[i].iter().map(|&j| &buildings[j]).collect();
            ComposedBuilding {
                parent: parents[i],
                design_source: representative,
                style_seed: template.seed,
                facade_openings: openings[i],
                height: crate::building_surface::outline_fallback_height(source, &parts),
            }
        })
        .collect()
}

impl ComposedBuilding {
    pub(crate) fn design(
        &self,
        source: &Building,
        buildings: &[Building],
        origin: (f64, f64),
        cache: &mut [Option<BuildingDesign>],
    ) -> BuildingDesign {
        let template = &buildings[self.design_source];
        let mut design = *cache[self.design_source].get_or_insert_with(|| {
            crate::regional_architecture::building_design(
                template,
                origin,
                building_metrics(template),
            )
        });
        // Source roof shape and use remain independent of the shared facade.
        if source.roof_shape.is_some() && source.roof_shape != template.roof_shape {
            let own = crate::regional_architecture::building_design(
                source,
                origin,
                building_metrics(source),
            );
            design.roof = own.roof;
            design.roof_material = own.roof_material;
            design.roof_variant = own.roof_variant;
        }
        design.inferred_roof = source.roof_shape.unwrap_or_else(|| {
            if source.kind.is_part() && source.height_m >= 28.0 {
                RoofShape::Flat
            } else {
                design.inferred_roof
            }
        });
        if source.use_kind != BuildingUse::Unknown {
            design.use_kind = source.use_kind;
        }
        if !self.facade_openings {
            design.facade = None;
            design.expression = 0;
            design.detail = DetailFamily::Plain;
            design.inferred_roof = source.roof_shape.unwrap_or(RoofShape::Flat);
        }
        design
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn building(
        seed: u64,
        width: f32,
        depth: f32,
        height: f32,
        bottom: f32,
        kind: BuildingKind,
    ) -> Building {
        Building {
            rings: vec![vec![[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]]],
            location_rad: Some([40.75_f64.to_radians(), -73.98_f64.to_radians()]),
            seed,
            height_m: height,
            min_height_m: bottom,
            kind,
            roof_shape: None,
            use_kind: BuildingUse::Unknown,
        }
    }

    #[test]
    fn parts_share_a_family_but_keep_roof_facts_and_unrelated_uses() {
        let mut sources = vec![
            building(1, 40.0, 40.0, 150.0, 0.0, BuildingKind::Building),
            building(2, 25.0, 25.0, 150.0, 0.0, BuildingKind::Part),
            building(3, 12.0, 12.0, 80.0, 0.0, BuildingKind::Part),
            building(4, 10.0, 10.0, 15.0, 0.0, BuildingKind::Part),
        ];
        sources[0].use_kind = BuildingUse::Commercial;
        sources[2].roof_shape = Some(RoofShape::Dome);
        sources[3].use_kind = BuildingUse::Industrial;
        let result = resolve(&sources);
        let mut cache = vec![None; sources.len()];
        let designs: Vec<_> = result
            .iter()
            .zip(&sources)
            .map(|(r, b)| r.design(b, &sources, (0.0, 0.0), &mut cache))
            .collect();
        for i in [1, 2] {
            assert_eq!(result[i].parent, Some(0));
            assert_eq!(designs[i].facade, designs[0].facade);
            assert_eq!(designs[i].wall, designs[0].wall);
            assert_eq!(designs[i].joinery, designs[0].joinery);
            assert_eq!(result[i].style_seed, result[0].style_seed);
        }
        assert_eq!(designs[2].inferred_roof, RoofShape::Dome);
        assert_eq!(designs[3].use_kind, BuildingUse::Industrial);
        assert_eq!(designs[3].facade, None);
        assert_eq!(sources[2].height_m, 80.0);
    }

    #[test]
    fn low_outline_uses_occupied_tower_family_not_its_spire() {
        let sources = vec![
            building(1, 50.0, 50.0, 8.0, 0.0, BuildingKind::Building),
            building(2, 20.0, 20.0, 180.0, 0.0, BuildingKind::Part),
            building(3, 2.0, 2.0, 230.0, 180.0, BuildingKind::Part),
        ];
        let result = resolve(&sources);
        assert_eq!(result[0].design_source, 1);
        assert_eq!(result[1].design_source, 1);
        assert!(!result[2].facade_openings);
        assert_eq!(result[2].height, 230.0);
    }

    #[test]
    fn service_volumes_are_windowless_but_occupied_setbacks_survive() {
        let parent = building(1, 40.0, 40.0, 180.0, 0.0, BuildingKind::Building);
        let fin = building(2, 1.0, 20.0, 190.0, 170.0, BuildingKind::Part);
        let plant = building(3, 8.0, 8.0, 190.0, 170.0, BuildingKind::Part);
        let setback = building(4, 15.0, 15.0, 180.0, 100.0, BuildingKind::Part);
        assert!(!permits_openings(&fin, Some(&parent)));
        assert!(!permits_openings(&plant, Some(&parent)));
        assert!(permits_openings(&setback, Some(&parent)));
        let mut residential = building(5, 8.0, 8.0, 150.0, 0.0, BuildingKind::Building);
        residential.use_kind = BuildingUse::Residential;
        assert!(permits_openings(&residential, None));
        let mut factory = building(6, 40.0, 40.0, 20.0, 0.0, BuildingKind::Building);
        factory.use_kind = BuildingUse::Industrial;
        let stack = building(7, 5.0, 5.0, 80.0, 0.0, BuildingKind::Part);
        assert!(!permits_openings(&stack, Some(&factory)));
        assert!(permits_openings(&factory, None));
        residential.use_kind = BuildingUse::Utility;
        assert!(!permits_openings(&residential, None));
    }

    #[test]
    fn association_rejects_courtyard_fill_and_concavity_crossings() {
        let mut courtyard = building(1, 40.0, 40.0, 60.0, 0.0, BuildingKind::Building);
        courtyard
            .rings
            .push(vec![[10.0, 10.0], [10.0, 30.0], [30.0, 30.0], [30.0, 10.0]]);
        let fill = building(2, 40.0, 40.0, 55.0, 0.0, BuildingKind::Part);
        assert_eq!(resolve(&[courtyard, fill])[1].parent, None);
        let mut concave = building(3, 40.0, 40.0, 60.0, 0.0, BuildingKind::Building);
        concave.rings[0] = vec![
            [0.0, 0.0],
            [40.0, 0.0],
            [40.0, 40.0],
            [30.0, 40.0],
            [30.0, 10.0],
            [10.0, 10.0],
            [10.0, 40.0],
            [0.0, 40.0],
        ];
        let crossing = building(4, 40.0, 40.0, 55.0, 0.0, BuildingKind::Part);
        assert_eq!(resolve(&[concave, crossing])[1].parent, None);
    }

    #[test]
    fn thin_outline_residual_lowers_only_the_fallback_volume() {
        let sources = vec![
            building(1, 40.0, 40.0, 160.0, 0.0, BuildingKind::Building),
            building(2, 39.8, 40.0, 60.0, 0.0, BuildingKind::Part),
        ];
        let result = resolve(&sources);
        assert_eq!(result[0].height, 60.0);
        assert_eq!(result[1].height, 60.0);
        assert_eq!(sources[0].height_m, 160.0);
        let broad = vec![
            building(1, 40.0, 40.0, 160.0, 0.0, BuildingKind::Building),
            building(2, 30.0, 40.0, 60.0, 0.0, BuildingKind::Part),
        ];
        assert_eq!(resolve(&broad)[0].height, 160.0);
        let elevated = vec![
            building(1, 40.0, 40.0, 160.0, 0.0, BuildingKind::Building),
            building(2, 39.8, 40.0, 160.0, 140.0, BuildingKind::Part),
        ];
        assert_eq!(resolve(&elevated)[0].height, 160.0);
    }

    #[test]
    fn composition_is_rotation_translation_and_source_order_independent() {
        let mut sources = vec![
            building(1, 40.0, 40.0, 160.0, 0.0, BuildingKind::Building),
            building(2, 39.8, 40.0, 60.0, 0.0, BuildingKind::Part),
            building(3, 1.0, 20.0, 180.0, 160.0, BuildingKind::Part),
        ];
        let original = resolve(&sources);
        let angle = 0.71_f32;
        for source in &mut sources {
            for point in source.rings.iter_mut().flatten() {
                let [x, y] = *point;
                *point = [
                    x * angle.cos() - y * angle.sin() + 1000.0,
                    x * angle.sin() + y * angle.cos() - 700.0,
                ];
            }
        }
        sources.reverse();
        let moved = resolve(&sources);
        for (a, b) in original.iter().zip(moved.iter().rev()) {
            assert_eq!(a.height, b.height);
            assert_eq!(a.facade_openings, b.facade_openings);
            assert_eq!(a.style_seed, b.style_seed);
        }
    }
}
