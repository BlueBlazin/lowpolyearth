//! Authored paving preferences constrained by mapped route function and land use.
//! Regional mixes are art direction, not measured shares of individual surfaces.

use crate::{SurfaceAppearance, material, tier0::SourceSurfaceQuery};
use vearth_core::{ellipsoid::MEAN_RADIUS, frame::ChunkFrame};
use vearth_data::{
    admin_regions::AdminRegionAtlas,
    road_profile::RoadDetail,
    settlement::SettlementAtlas,
    vectors::{FeaturePack, GroundKind, Road, RoadClass},
};

use crate::road_catalog::{self, FINISHES, Finish, Recipe};

fn hash(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn eligible(road: &Road) -> bool {
    road.class == RoadClass::Path
        || road.class == RoadClass::Service
        || road.profile.detail == RoadDetail::LivingStreet
}

pub(crate) struct Resolver<'a> {
    origin: (f64, f64),
    ground: Option<SourceSurfaceQuery<'a>>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(frame: &ChunkFrame, pack: &'a FeaturePack) -> Self {
        Self {
            origin: frame.column_latlon(0.5, 0.5),
            ground: pack
                .roads
                .iter()
                .any(|road| {
                    road.profile.location_rad.is_some_and(|[lat, lon]| {
                        eligible(road)
                            || (road.class == RoadClass::Minor
                                && road_catalog::local_finish(
                                    lat,
                                    lon,
                                    road.profile.detail,
                                    road.class,
                                    road.profile.seed,
                                )
                                .is_some())
                    })
                })
                .then(|| SourceSurfaceQuery::new(pack)),
        }
    }

    pub(crate) fn appearance(&self, road: &Road) -> SurfaceAppearance {
        let baseline = crate::road_appearance(road.class, 0.5, 1.0);
        if !eligible(road) && road.class != RoadClass::Minor {
            return baseline;
        }
        let Some([lat, lon]) = road.profile.location_rad.filter(|p| {
            p.iter().all(|v| v.is_finite()) && p[0].abs() <= std::f64::consts::FRAC_PI_2
        }) else {
            return baseline;
        };
        let dl = (lon - self.origin.1 + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        let point = [
            (dl * self.origin.0.cos() * MEAN_RADIUS) as f32,
            ((lat - self.origin.0) * MEAN_RADIUS) as f32,
        ];
        let (ground, forest) = self.ground.as_ref().map_or((None, false), |query| {
            (query.at(point).0, query.contains_forest(point))
        });
        if !natural_ground(ground, forest)
            && let Some(local) = road_catalog::local_finish(
                lat,
                lon,
                road.profile.detail,
                road.class,
                road.profile.seed,
            )
        {
            return appearance(local.finish, local.color, road.profile.seed);
        }
        if !eligible(road) {
            return baseline;
        }
        let region = AdminRegionAtlas::bundled().context_at(lat, lon);
        let recipe = road_catalog::recipe(lat, lon, region);
        let development = SettlementAtlas::bundled()
            .development_at(lat, lon)
            .unwrap_or(0.65);
        let mix = weights(
            road.profile.detail,
            road.class,
            development,
            ground,
            forest,
            &recipe,
        );
        let finish = choose(mix, road.profile.seed);
        appearance(finish, recipe.colors[finish.index()], road.profile.seed)
    }
}

fn natural_ground(ground: Option<GroundKind>, forest: bool) -> bool {
    if matches!(ground, Some(GroundKind::Paved | GroundKind::RunwayArea)) {
        return false;
    }
    forest
        || matches!(
            ground,
            Some(
                GroundKind::Forest
                    | GroundKind::NaturalGrass
                    | GroundKind::Scrub
                    | GroundKind::BareRock
                    | GroundKind::Sand
                    | GroundKind::Wetland
            )
        )
}

fn weights(
    detail: RoadDetail,
    class: RoadClass,
    development: f32,
    ground: Option<GroundKind>,
    forest: bool,
    recipe: &Recipe,
) -> [f32; 8] {
    let density = development.clamp(0.0, 1.0);
    let natural = natural_ground(ground, forest);
    let park = ground == Some(GroundKind::Green);
    let paved = matches!(ground, Some(GroundKind::Paved | GroundKind::RunwayArea));
    // Asphalt, concrete, brick, setts, gravel, earth, cobbles, stone slabs.
    let mut weights = match detail {
        RoadDetail::Cycleway | RoadDetail::Crossing => [75.0, 25.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        RoadDetail::Sidewalk | RoadDetail::Corridor | RoadDetail::Steps => {
            [10.0, 65.0, 10.0, 0.0, 0.0, 0.0, 0.0, 15.0]
        }
        RoadDetail::Pedestrian | RoadDetail::Pier => [10.0, 35.0, 15.0, 20.0, 0.0, 0.0, 0.0, 20.0],
        RoadDetail::LivingStreet => [45.0, 25.0, 15.0, 15.0, 0.0, 0.0, 0.0, 0.0],
        RoadDetail::Track | RoadDetail::Bridleway => [0.0, 0.0, 0.0, 0.0, 60.0, 40.0, 0.0, 0.0],
        RoadDetail::Alley => [55.0, 25.0, 10.0, 5.0, 5.0, 0.0, 0.0, 0.0],
        RoadDetail::Driveway | RoadDetail::ParkingAisle => {
            [65.0, 30.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0]
        }
        _ if class == RoadClass::Service => [75.0, 20.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0],
        _ if natural => [0.0, 0.0, 0.0, 0.0, 45.0, 55.0, 0.0, 0.0],
        _ if park => [15.0, 10.0, 0.0, 0.0, 55.0, 20.0, 0.0, 0.0],
        _ => [
            5.0 + 25.0 * density,
            5.0 + 35.0 * density,
            5.0 * density,
            0.0,
            40.0 - 20.0 * density,
            50.0 - 50.0 * density,
            0.0,
            5.0 * density,
        ],
    };
    let unit_paving = matches!(
        detail,
        RoadDetail::LivingStreet
            | RoadDetail::Pedestrian
            | RoadDetail::Sidewalk
            | RoadDetail::Corridor
            | RoadDetail::Steps
            | RoadDetail::Alley
    ) || (!natural
        && !park
        && matches!(detail, RoadDetail::Footway | RoadDetail::Generic)
        && class == RoadClass::Path);
    if unit_paving {
        let strength = if matches!(detail, RoadDetail::Footway | RoadDetail::Generic) {
            density
        } else {
            1.0
        };
        for (weight, bonus) in weights.iter_mut().zip(recipe.paving) {
            *weight += bonus * strength;
        }
    }
    // Rough cobbles belong to alleys/shared streets, never an inferred accessible
    // sidewalk, ordinary footpath or dedicated cycleway. Local records can use
    // documented historic surfaces on specific pedestrian routes.
    if !matches!(detail, RoadDetail::Alley | RoadDetail::LivingStreet) {
        weights[Finish::Cobblestone.index()] = 0.0;
    }
    if paved {
        weights[Finish::Concrete.index()] +=
            weights[Finish::Gravel.index()] + weights[Finish::Earth.index()];
        weights[Finish::Gravel.index()] = 0.0;
        weights[Finish::Earth.index()] = 0.0;
    }
    weights
}

fn choose(weights: [f32; 8], seed: u64) -> Finish {
    if seed == 0 {
        // Merged unnamed features do not provide route identity. Use a coherent
        // contextual preference instead of rerolling each source tile fragment.
        let index = (0..8)
            .max_by(|&a, &b| weights[a].total_cmp(&weights[b]).then(b.cmp(&a)))
            .unwrap();
        return FINISHES[index];
    }
    let mut selector = (hash(seed ^ 0x5041_5649_4e47) >> 40) as f32 / (1u32 << 24) as f32
        * weights.iter().sum::<f32>();
    for (weight, finish) in weights.into_iter().zip(FINISHES) {
        if selector < weight {
            return finish;
        }
        selector -= weight;
    }
    Finish::Concrete
}

fn appearance(finish: Finish, base: [f32; 3], seed: u64) -> SurfaceAppearance {
    let material = match finish {
        Finish::Asphalt => material::ASPHALT,
        Finish::Concrete => material::CONCRETE_PATH,
        Finish::Brick => material::BRICK_PAVING,
        Finish::Stone => material::STONE_PAVING,
        Finish::Gravel => material::GRAVEL_PATH,
        Finish::Earth => material::PATH,
        Finish::Cobblestone => material::COBBLESTONE,
        Finish::StoneSlab => material::STONE_SLAB,
    };
    let variation = if seed == 0 {
        1.0
    } else {
        0.97 + (hash(seed ^ 0x5449_4e54) >> 40) as f32 / (1u32 << 24) as f32 * 0.06
    };
    SurfaceAppearance {
        material,
        tint: base.map(|v| (v * variation).round() as u8),
    }
}

/// Neutral specimens in asphalt, concrete, brick, setts, gravel, earth,
/// rounded cobbles and stone-slab order, rendered by the same production atlas.
pub fn samples() -> [SurfaceAppearance; 8] {
    let recipe = road_catalog::recipe(0.0, 0.0, None);
    FINISHES.map(|finish| appearance(finish, recipe.colors[finish.index()], 0))
}

/// A source-backed local paving specimen for the renderer's inspection catalog.
pub struct LocalSample {
    pub label: &'static str,
    pub region: &'static str,
    pub reference: &'static str,
    pub appearance: SurfaceAppearance,
}

pub fn local_samples() -> Vec<LocalSample> {
    road_catalog::local_examples()
        .map(|example| {
            let local = road_catalog::local_finish(
                example.lat_rad,
                example.lon_rad,
                example.detail,
                example.class,
                example.name_seed,
            )
            .expect("catalog example resolves");
            debug_assert_eq!(
                (local.label, local.finish, local.color),
                (example.label, example.finish, example.color)
            );
            let context = AdminRegionAtlas::bundled().context_at(example.lat_rad, example.lon_rad);
            let recipe = road_catalog::recipe(example.lat_rad, example.lon_rad, context);
            LocalSample {
                label: local.label,
                region: recipe.label,
                reference: example.source,
                appearance: appearance(local.finish, local.color, example.name_seed),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_data::admin_regions::AdminRegionContext;

    #[test]
    fn production_resolver_uses_local_catalog_across_render_lods_and_fragment_directions() {
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            ellipsoid::geodetic_to_ecef,
        };
        use vearth_data::road_profile::RoadProfile;
        for example in road_catalog::local_examples() {
            let expected = appearance(example.finish, example.color, example.name_seed);
            for level in [15, 17, 19] {
                let direction = geodetic_to_ecef(example.lat_rad, example.lon_rad, 0.0).normalize();
                let cell = CellId::from_dir(direction, level);
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                let mut pack = FeaturePack::empty();
                for pts in [
                    vec![[-20.0, 0.0], [20.0, 0.0]],
                    vec![[0.0, 0.0], [-20.0, 0.0]],
                    vec![[20.0, 0.0], [0.0, 0.0]],
                ] {
                    pack.roads.push(Road {
                        profile: RoadProfile {
                            detail: example.detail,
                            seed: example.name_seed,
                            location_rad: Some([example.lat_rad, example.lon_rad]),
                        },
                        pts,
                        class: example.class,
                        is_bridge: false,
                    });
                }
                let resolver = Resolver::new(&frame, &pack);
                for road in &pack.roads {
                    assert_eq!(
                        resolver.appearance(road),
                        expected,
                        "{} L{level}",
                        example.label
                    );
                }
                let mut unrelated = Road {
                    profile: pack.roads[0].profile,
                    pts: pack.roads[0].pts.clone(),
                    class: pack.roads[0].class,
                    is_bridge: false,
                };
                unrelated.class = RoadClass::Highway;
                unrelated.profile.detail = RoadDetail::Generic;
                assert_eq!(
                    resolver.appearance(&unrelated),
                    crate::road_appearance(RoadClass::Highway, 0.5, 1.0)
                );
                unrelated.class = RoadClass::Minor;
                unrelated.profile.seed = 0;
                assert_eq!(
                    resolver.appearance(&unrelated),
                    crate::road_appearance(RoadClass::Minor, 0.5, 1.0)
                );
            }
        }
    }

    fn recipes() -> impl Iterator<Item = Recipe> {
        AdminRegionAtlas::bundled()
            .contexts()
            .map(|context| road_catalog::recipe(0.5, 0.5, Some(context)))
    }

    #[test]
    fn every_region_respects_route_function_and_mapped_ground() {
        for recipe in recipes() {
            for density in [0.0, 0.4, 0.65, 0.85, 1.0] {
                for detail in [RoadDetail::Cycleway, RoadDetail::Crossing] {
                    let cycle = weights(detail, RoadClass::Path, density, None, false, &recipe);
                    assert!(cycle[0] > 0.0 && cycle[1] > 0.0);
                    assert_eq!(&cycle[2..], &[0.0; 6]);
                }
                for ground in [
                    GroundKind::Forest,
                    GroundKind::Urban,
                    GroundKind::Farmyard,
                    GroundKind::AirportArea,
                ] {
                    let woodland = weights(
                        RoadDetail::Footway,
                        RoadClass::Path,
                        density,
                        Some(ground),
                        true,
                        &recipe,
                    );
                    assert_eq!(&woodland[..4], &[0.0; 4]);
                    assert_eq!(&woodland[6..], &[0.0; 2]);
                    assert!(woodland[4] > 0.0 && woodland[5] > 0.0);
                }
                let paved = weights(
                    RoadDetail::Footway,
                    RoadClass::Path,
                    density,
                    Some(GroundKind::Paved),
                    false,
                    &recipe,
                );
                assert_eq!(&paved[4..6], &[0.0; 2]);
                let sidewalk = weights(
                    RoadDetail::Sidewalk,
                    RoadClass::Path,
                    density,
                    None,
                    false,
                    &recipe,
                );
                assert_eq!(&sidewalk[4..7], &[0.0; 3]);
            }
        }
    }

    #[test]
    fn names_supply_variety_and_unnamed_routes_use_a_coherent_preference() {
        let context = AdminRegionContext {
            id: 0,
            name: "",
            country_name: "Netherlands",
        };
        let recipe =
            road_catalog::recipe(52.37_f64.to_radians(), 4.9_f64.to_radians(), Some(context));
        let mix = weights(
            RoadDetail::LivingStreet,
            RoadClass::Minor,
            1.0,
            None,
            false,
            &recipe,
        );
        assert_eq!(
            choose(mix, 0).index(),
            (0..8)
                .max_by(|&a, &b| mix[a].total_cmp(&mix[b]).then(b.cmp(&a)))
                .unwrap()
        );
        let named: std::collections::HashSet<_> =
            (1..256).map(|seed| choose(mix, seed).index()).collect();
        assert!(named.len() >= 3);
        let rural = weights(
            RoadDetail::Footway,
            RoadClass::Path,
            0.0,
            None,
            false,
            &recipe,
        );
        let city = weights(
            RoadDetail::Footway,
            RoadClass::Path,
            1.0,
            None,
            false,
            &recipe,
        );
        assert!(rural[5] > city[5]);
        assert!(city[1] > rural[1]);
    }

    #[test]
    fn local_minor_ground_evidence_is_independent_of_unrelated_paths() {
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            ellipsoid::geodetic_to_ecef,
        };
        use vearth_data::road_profile::{RoadProfile, name_seed};
        let lat = 40.70335_f64.to_radians();
        let lon = -73.98957_f64.to_radians();
        let cell = CellId::from_dir(geodetic_to_ecef(lat, lon, 0.0).normalize(), 19);
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let mut pack = FeaturePack::empty();
        pack.roads.push(Road {
            profile: RoadProfile {
                detail: RoadDetail::Generic,
                seed: name_seed(Some("Washington Street")),
                location_rad: Some([lat, lon]),
            },
            pts: vec![[-20.0, 0.0], [20.0, 0.0]],
            class: RoadClass::Minor,
            is_bridge: false,
        });
        let documented = Resolver::new(&frame, &pack).appearance(&pack.roads[0]);
        assert_eq!(documented.material, material::STONE_PAVING);
        pack.forest.push(vec![vec![
            [-1000.0, -1000.0],
            [1000.0, -1000.0],
            [1000.0, 1000.0],
            [-1000.0, 1000.0],
        ]]);
        let alone = Resolver::new(&frame, &pack).appearance(&pack.roads[0]);
        assert_eq!(alone, crate::road_appearance(RoadClass::Minor, 0.5, 1.0));
        pack.roads.push(Road {
            profile: RoadProfile {
                detail: RoadDetail::Footway,
                seed: 0,
                location_rad: Some([lat, lon]),
            },
            pts: vec![[30.0, 0.0], [40.0, 0.0]],
            class: RoadClass::Path,
            is_bridge: false,
        });
        assert_eq!(
            Resolver::new(&frame, &pack).appearance(&pack.roads[0]),
            alone
        );
    }

    #[test]
    fn ground_query_distinguishes_forest_cultivation_and_explicit_pavement() {
        use vearth_data::vectors::{FloraRegion, FloraRegionKind};
        let rings = vec![vec![
            [-10.0, -10.0],
            [10.0, -10.0],
            [10.0, 10.0],
            [-10.0, 10.0],
        ]];
        let mut pack = FeaturePack::empty();
        pack.flora.push(FloraRegion {
            location_rad: None,
            rings: rings.clone(),
            kind: FloraRegionKind::Farmland,
            seed: 17,
        });
        let query = SourceSurfaceQuery::new(&pack);
        assert_eq!(query.at([0.0, 0.0]), (Some(GroundKind::Green), true));
        assert!(!query.contains_forest([0.0, 0.0]));
        pack.flora.clear();
        pack.forest.push(rings.clone());
        pack.ground.push((GroundKind::Urban, rings.clone()));
        let query = SourceSurfaceQuery::new(&pack);
        assert_eq!(query.at([0.0, 0.0]), (Some(GroundKind::Urban), false));
        assert!(query.contains_forest([0.0, 0.0]));
        assert!(natural_ground(
            query.at([0.0, 0.0]).0,
            query.contains_forest([0.0, 0.0])
        ));
        pack.ground.clear();
        pack.ground.push((GroundKind::Paved, rings));
        let query = SourceSurfaceQuery::new(&pack);
        assert!(!natural_ground(
            query.at([0.0, 0.0]).0,
            query.contains_forest([0.0, 0.0])
        ));
        assert!(!query.contains_forest([100.0, 100.0]));
    }
}
