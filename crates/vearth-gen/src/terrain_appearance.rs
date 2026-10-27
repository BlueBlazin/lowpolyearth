//! Shared-corner source appearance independent of a cell's column ownership.

use crate::{
    Col, N, SAMPLE_ZOOM_FACTOR, Sources, TerrainCornerShading, art_direction, climate_at,
    ground_appearance, material, natural_cover_exposure, terrain_appearance,
    terrain_appearance_with_exposure, terrain_pigment, tier0,
};
use vearth_core::{ellipsoid::MEAN_RADIUS, frame::ChunkFrame};
use vearth_data::vectors::FeaturePack;

pub(crate) fn sample(
    frame: &ChunkFrame,
    sources: &Sources,
    pack: Option<&FeaturePack>,
    corner_heights: &[f32],
    margin: usize,
) -> Box<[TerrainCornerShading; (N + 1) * (N + 1)]> {
    assert!(
        margin >= 1,
        "terrain shading requires a source derivative halo"
    );
    let edge = N + 2 * margin + 1;
    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    let projection = tier0::GridProj::of(frame, lat0, lon0);
    let ground_query = pack
        .filter(|_| frame.addr.cell.level >= tier0::GROUND_MIN_LEVEL)
        .map(tier0::SourceSurfaceQuery::new);
    let normal_to_world = frame.model_matrix().inverse().transpose();
    let hint = frame.grid_scale.y * SAMPLE_ZOOM_FACTOR;
    let detail = (100.0 / frame.grid_scale.y).clamp(0.05, 1.0) as f32;
    let spacing_m = frame.grid_scale.x.max(frame.grid_scale.z);
    let geodetic_height = |x: i32, z: i32| {
        corner_heights[(z + margin as i32) as usize * edge + (x + margin as i32) as usize]
    };
    // The derivative stencil shares lattice points between adjacent corners.
    // Resolve each ellipsoid position once with the same arithmetic and reuse
    // its local height and geographic position for normals and source shading.
    let local_edge = N + 3;
    let local_samples: Vec<_> = (-1..=N as i32 + 1)
        .flat_map(|z| {
            (-1..=N as i32 + 1).map(move |x| {
                let (surface, offset) =
                    frame.column_surface(x as f64 / N as f64, z as f64 / N as f64);
                (
                    surface,
                    (f64::from(geodetic_height(x, z)) + offset) / frame.grid_scale.y,
                )
            })
        })
        .collect();
    let local_sample =
        |x: i32, z: i32| local_samples[(z + 1) as usize * local_edge + (x + 1) as usize];
    let local_height = |x: i32, z: i32| local_sample(x, z).1;

    let imagery_month = sources.imagery.and_then(|source| source.month());
    Box::new(std::array::from_fn(|index| {
        let (x, z) = ((index % (N + 1)) as i32, (index / (N + 1)) as i32);
        let height = geodetic_height(x, z);
        let normal = [
            (local_height(x - 1, z) - local_height(x + 1, z)) * 0.5,
            1.0,
            (local_height(x, z - 1) - local_height(x, z + 1)) * 0.5,
        ];
        let (mut low, mut high) = (height, height);
        for rz in z - 1..=z + 1 {
            for rx in x - 1..=x + 1 {
                let nearby = geodetic_height(rx, rz);
                low = low.min(nearby);
                high = high.max(nearby);
            }
        }
        let surface = local_sample(x, z).0;
        let (lat, lon) = vearth_core::ellipsoid::surface_latlon_along(surface);
        let world_normal = normal_to_world
            .transform_vector3(glam::DVec3::from_array(normal))
            .normalize();
        // Cube-face horizontal axes are skewed. Measuring inclination from
        // the transformed covector avoids orientation-dependent rock exposure.
        let slope = world_normal
            .dot(vearth_core::ellipsoid::geodetic_normal(surface))
            .clamp(-1.0, 1.0)
            .acos() as f32;
        let climate = climate_at(lat, lon, height);
        let mut col = Col {
            top: (local_height(x, z) * frame.grid_scale.y) as f32,
            sea: if height < 0.0 { 0.0 } else { f32::NEG_INFINITY },
            elev: height,
            bathymetry: 0.0,
            landscape_variation: art_direction::landscape_variation(
                lat.to_degrees(),
                lon.to_degrees(),
            ),
            slope,
            relief: high - low,
            climate,
            regional: crate::regional_landscape::resolve(lat, lon, height, climate, imagery_month),
            img: sources.imagery.and_then(|source| source.color(lat, lon)),
            img_detail: sources
                .imagery
                .and_then(|source| source.color_detail(lat, lon, hint)),
            img_hr: sources
                .imagery
                .and_then(|source| source.color_hr(lat, lon, hint)),
        };
        let (ground, cultivated) = if frame.addr.cell.level >= tier0::GROUND_MIN_LEVEL {
            ground_query.as_ref().map_or((None, false), |surfaces| {
                let delta_lon = (lon - lon0 + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
                let point = [
                    (delta_lon * lat0.cos() * MEAN_RADIUS) as f32,
                    ((lat - lat0) * MEAN_RADIUS) as f32,
                ];
                tier0::terrain_corner_surface(
                    surfaces,
                    &projection,
                    frame.addr.cell.level,
                    point,
                    [x, z],
                    &geodetic_height,
                )
            })
        } else {
            (None, false)
        };
        if cultivated {
            col.regional = crate::regional_landscape::Palette::EMPTY;
        }
        let mut appearance = ground.map_or_else(
            || terrain_appearance(&col, height, 0.5, detail),
            |ground| ground_appearance(ground, &col, 0.5, detail),
        );
        if matches!(
            ground,
            Some(
                tier0::GroundKind::Forest
                    | tier0::GroundKind::Scrub
                    | tier0::GroundKind::NaturalGrass
                    | tier0::GroundKind::RegionalForest
                    | tier0::GroundKind::RegionalScrub
                    | tier0::GroundKind::RegionalGrassland
            )
        ) && !cultivated
            && natural_cover_exposure(col.slope) > crate::ROCK_MATERIAL_EXPOSURE
        {
            // Column ownership remains mapped vegetation. Only this visible
            // response follows the substrate already exposed by its tint.
            appearance.material =
                terrain_appearance_with_exposure(&col, height, detail, 1.0).material;
        }
        let up = vearth_core::ellipsoid::geodetic_normal(surface);
        let corner = terrain_pigment::Corner {
            position: surface + up * f64::from(height),
            up,
            normal: world_normal,
            altitude_m: f64::from(height),
            spacing_m,
            // The 3x3 extent spans two corner steps.
            altitude_step_m: f64::from(high - low) * 0.5,
        };
        let mut pigment = terrain_pigment::NEUTRAL;
        if cultivated {
            // Cultivated source ownership is not a natural blend endpoint;
            // field shaders retain their own rows, palette and season state.
            // Their per-field colour receives only a softer ground ratio.
            pigment = terrain_pigment::multiplier(
                appearance.tint,
                terrain_pigment::Family::Vegetation,
                &corner,
                FIELD_PIGMENT,
            );
            appearance.material = material::FIELD_CROP;
        } else if let Some(family) = pigment_family(appearance.material) {
            appearance.tint = terrain_pigment::pigment(appearance.tint, family, &corner, 1.0);
        }
        let mut shading = TerrainCornerShading::new(
            normal.map(|component| component as f32),
            appearance.material,
            appearance.tint,
        );
        shading.pigment = pigment;
        shading
    }))
}

/// Cultivated fields keep their authored per-field colour; moisture patches
/// and gullies read through at reduced strength.
const FIELD_PIGMENT: f64 = 0.6;

fn pigment_family(mat: u8) -> Option<terrain_pigment::Family> {
    use terrain_pigment::Family;
    match mat {
        material::SAND | material::SOIL | material::SALT => Some(Family::Aggregate),
        material::GRASS | material::WETLAND => Some(Family::Vegetation),
        material::ROCK | material::VOLCANIC => Some(Family::Rock),
        material::SNOW | material::ICE => Some(Family::Snow),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        cubesphere::Face,
        mesh::{SURFACE_NORMAL_FLAG, decode_surface_normal},
    };
    use vearth_data::FnElevation;

    fn frame(cell: CellId) -> ChunkFrame {
        ChunkFrame::of(ChunkAddr { cell, k: 0 })
    }

    fn heights(frame: &ChunkFrame, sources: &Sources) -> Vec<f32> {
        let mut heights = Vec::new();
        for z in -1..=N as i32 + 1 {
            for x in -1..=N as i32 + 1 {
                let (lat, lon) = frame.column_latlon(x as f64 / N as f64, z as f64 / N as f64);
                heights.push(sources.elevation.elevation_m(
                    lat,
                    lon,
                    frame.grid_scale.y * SAMPLE_ZOOM_FACTOR,
                ));
            }
        }
        heights
    }

    fn world_normal(frame: &ChunkFrame, shading: TerrainCornerShading) -> DVec3 {
        let normal =
            decode_surface_normal(SURFACE_NORMAL_FLAG | (u32::from(shading.normal_oct) << 16))
                .unwrap();
        frame
            .model_matrix()
            .inverse()
            .transpose()
            .transform_vector3(DVec3::from_array(normal.map(f64::from)))
            .normalize()
    }

    #[test]
    fn neighboring_cells_sample_identical_edge_appearance_and_compatible_normals() {
        let elevation = FnElevation(|lat: f64, lon: f64| {
            (700.0 + 90.0 * (lat * 100.0).sin() + 50.0 * (lon * 200.0).cos()) as f32
        });
        let sources = Sources::terrain_only(&elevation);
        for face in 0..6 {
            let a = frame(CellId {
                face: Face(face),
                level: 15,
                i: 20_000,
                j: 13_000,
            });
            let b = frame(CellId {
                i: a.addr.cell.i + 1,
                ..a.addr.cell
            });
            let sa = sample(&a, &sources, None, &heights(&a, &sources), 1);
            let sb = sample(&b, &sources, None, &heights(&b, &sources), 1);
            for z in 0..=N {
                let (a_corner, b_corner) = (sa[z * (N + 1) + N], sb[z * (N + 1)]);
                assert_eq!(a_corner.material, b_corner.material);
                assert_eq!(a_corner.tint, b_corner.tint);
                assert!(world_normal(&a, a_corner).dot(world_normal(&b, b_corner)) > 0.9995);
            }
        }
    }

    #[test]
    fn orbital_corner_albedo_retains_measured_imagery_without_a_biome_grade() {
        use vearth_data::ColorSource;

        struct Observed;
        impl ColorSource for Observed {
            fn color(&self, lat: f64, lon: f64) -> Option<[u8; 3]> {
                Some([
                    (120.0 + 80.0 * (lat * 100.0).sin()) as u8,
                    (90.0 + 70.0 * (lon * 100.0).sin()) as u8,
                    (95.0 + 65.0 * ((lat + lon) * 100.0).cos()) as u8,
                ])
            }
        }
        let elevation = FnElevation(|lat: f64, lon: f64| {
            (1_500.0 + 400.0 * ((lat + lon) * 100.0).sin()) as f32
        });
        let sources = Sources {
            imagery: Some(&Observed),
            ..Sources::terrain_only(&elevation)
        };
        let frame = frame(CellId {
            face: Face(0),
            level: 5,
            i: 12,
            j: 20,
        });
        let shading = sample(&frame, &sources, None, &heights(&frame, &sources), 1);
        for z in 0..=N {
            for x in 0..=N {
                let (lat, lon) = frame.column_latlon(x as f64 / N as f64, z as f64 / N as f64);
                assert_eq!(
                    shading[z * (N + 1) + x].tint,
                    Observed.color(lat, lon).unwrap()
                );
            }
        }
    }

    #[test]
    fn source_appearance_and_normals_remain_continuous_across_cube_faces() {
        for face in 0..6 {
            let a = frame(CellId {
                face: Face(face),
                level: 15,
                i: 0,
                j: 10_000,
            });
            let neighbor = CellId::from_dir(a.column_dir(-0.01, 0.5), 15);
            assert_ne!(neighbor.face, a.addr.cell.face);
            let b = frame(neighbor);
            let anchor = a.column_surface(0.0, 0.5).0;
            let up = vearth_core::ellipsoid::geodetic_normal(anchor);
            let gradient = up.cross(DVec3::X).normalize() * 0.65;
            let elevation = FnElevation(|lat, lon| {
                let point = vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
                (900.0 + (point - anchor).dot(gradient)) as f32
            });
            let sources = Sources::terrain_only(&elevation);
            let sa = sample(&a, &sources, None, &heights(&a, &sources), 1);
            let sb = sample(&b, &sources, None, &heights(&b, &sources), 1);
            let boundary: Vec<_> = (0..=N)
                .flat_map(|q| [(0, q), (N, q), (q, 0), (q, N)])
                .collect();
            for z in 0..=N {
                let point = a.column_surface(0.0, z as f64 / N as f64).0;
                let &(bx, bz) = boundary
                    .iter()
                    .min_by(|&&(ax, az), &&(bx, bz)| {
                        let pa = b
                            .column_surface(ax as f64 / N as f64, az as f64 / N as f64)
                            .0;
                        let pb = b
                            .column_surface(bx as f64 / N as f64, bz as f64 / N as f64)
                            .0;
                        pa.distance_squared(point)
                            .total_cmp(&pb.distance_squared(point))
                    })
                    .unwrap();
                assert!(
                    b.column_surface(bx as f64 / N as f64, bz as f64 / N as f64)
                        .0
                        .distance(point)
                        < 0.001
                );
                let (ca, cb) = (sa[z * (N + 1)], sb[bz * (N + 1) + bx]);
                assert_eq!(ca.material, cb.material, "cube-face {face} source material");
                assert!(
                    ca.tint.iter().zip(cb.tint).all(|(a, b)| a.abs_diff(b) <= 1),
                    "cube-face {face}: {:?} versus {:?}",
                    ca.tint,
                    cb.tint
                );
                assert!(world_normal(&a, ca).dot(world_normal(&b, cb)) > 0.999);
            }
        }
    }

    #[test]
    fn mapped_cliff_corners_require_dem_support_and_preserve_stronger_ground() {
        use vearth_data::{
            protomaps_semantics::SourceFeatureIdentity,
            vectors::{CliffLine, GroundKind, Road, RoadClass, Water},
        };

        let frame = frame(CellId {
            face: Face(0),
            level: 16,
            i: 32_768,
            j: 32_768,
        });
        let elevation = FnElevation(|_, _| 600.0);
        let sources = Sources::terrain_only(&elevation);
        let mut slope = Vec::new();
        for _z in -1..=N as i32 + 1 {
            for x in -1..=N as i32 + 1 {
                slope.push(600.0 + x as f32 * frame.grid_scale.y as f32 * 0.65);
            }
        }
        let rings = vec![vec![
            [-5_000.0, -5_000.0],
            [5_000.0, -5_000.0],
            [5_000.0, 5_000.0],
            [-5_000.0, 5_000.0],
        ]];
        let mut pack = FeaturePack::empty();
        pack.forest.push(rings.clone());
        let center = (N / 2) * (N + 1) + N / 2;
        assert_eq!(
            sample(&frame, &sources, Some(&pack), &slope, 1)[center].material,
            material::GRASS
        );
        pack.cliffs.push(CliffLine {
            pts: vec![[-500.0, 0.0], [500.0, 0.0]],
            identity: SourceFeatureIdentity::Provided(17),
        });
        assert_eq!(
            sample(&frame, &sources, Some(&pack), &slope, 1)[center].material,
            material::ROCK
        );
        assert_eq!(
            sample(&frame, &sources, Some(&pack), &heights(&frame, &sources), 1)[center].material,
            material::GRASS
        );
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-500.0, 0.0], [500.0, 0.0]],
            class: RoadClass::Major,
            is_bridge: false,
        });
        assert_eq!(
            sample(&frame, &sources, Some(&pack), &slope, 1)[center].material,
            material::GRASS
        );
        pack.roads.clear();
        pack.water.push(Water::inland(rings.clone()));
        assert_eq!(
            sample(&frame, &sources, Some(&pack), &slope, 1)[center].material,
            material::GRASS
        );
        pack.water.clear();
        pack.forest.clear();
        for (ground, expected) in [
            (GroundKind::Glacier, material::ICE),
            (GroundKind::Sand, material::SAND),
            (GroundKind::Wetland, material::WETLAND),
        ] {
            pack.ground = vec![(ground, rings.clone())];
            assert_eq!(
                sample(&frame, &sources, Some(&pack), &slope, 1)[center].material,
                expected
            );
        }
    }

    #[test]
    fn steep_mapped_cover_uses_exposed_surface_response_without_changing_source_ownership() {
        use vearth_data::{
            protomaps_semantics::LandcoverKind,
            vectors::{FloraRegion, FloraRegionKind, GroundKind, RegionalLandcoverRegion},
        };

        let frame = frame(CellId {
            face: Face(0),
            level: 16,
            i: 32_768,
            j: 32_768,
        });
        let elevation = FnElevation(|_, _| 600.0);
        let sources = Sources::terrain_only(&elevation);
        let mut steep = Vec::new();
        for _z in -1..=N as i32 + 1 {
            for x in -1..=N as i32 + 1 {
                steep.push(600.0 + x as f32 * frame.grid_scale.y as f32 * 3.0);
            }
        }
        let gentle = heights(&frame, &sources);
        let rings = vec![vec![
            [-5_000.0, -5_000.0],
            [5_000.0, -5_000.0],
            [5_000.0, 5_000.0],
            [-5_000.0, 5_000.0],
        ]];
        let center = (N / 2) * (N + 1) + N / 2;
        for ground in [
            GroundKind::Forest,
            GroundKind::NaturalGrass,
            GroundKind::Scrub,
        ] {
            let mut pack = FeaturePack::empty();
            pack.ground.push((ground, rings.clone()));
            assert_eq!(
                sample(&frame, &sources, Some(&pack), &gentle, 1)[center].material,
                material::GRASS
            );
            assert_eq!(
                sample(&frame, &sources, Some(&pack), &steep, 1)[center].material,
                material::ROCK
            );
            assert_eq!(
                tier0::tree_source_surface(&pack, [0.0, 0.0]),
                (Some(ground), false)
            );
        }
        for (ground, expected) in [
            (GroundKind::Glacier, material::ICE),
            (GroundKind::Sand, material::SAND),
        ] {
            let mut pack = FeaturePack::empty();
            pack.ground.push((ground, rings.clone()));
            assert_eq!(
                sample(&frame, &sources, Some(&pack), &steep, 1)[center].material,
                expected
            );
        }
        let mut pack = FeaturePack::empty();
        pack.regional_landcover.push(RegionalLandcoverRegion {
            location_rad: None,
            rings: rings.clone(),
            kind: LandcoverKind::Grassland,
            source_id: Some(1),
            stable_identity: 1,
        });
        assert_eq!(
            sample(&frame, &sources, Some(&pack), &steep, 1)[center].material,
            material::ROCK
        );
        pack.flora.push(FloraRegion {
            location_rad: None,
            rings,
            kind: FloraRegionKind::Farmland,
            seed: 7,
        });
        assert_eq!(
            sample(&frame, &sources, Some(&pack), &steep, 1)[center].material,
            material::FIELD_CROP
        );
    }

    #[test]
    fn point_cover_preserves_mapped_authority_and_cultivated_boundaries_at_the_dateline() {
        use vearth_data::{
            protomaps_semantics::LandcoverKind,
            vectors::{GroundKind, RegionalLandcoverRegion},
        };

        let cell = CellId::from_dir(
            vearth_core::ellipsoid::geodetic_to_ecef(
                20.0_f64.to_radians(),
                std::f64::consts::PI,
                0.0,
            )
            .normalize(),
            15,
        );
        let frame = frame(cell);
        let elevation = FnElevation(|_, _| 600.0);
        let sources = Sources::terrain_only(&elevation);
        let heights = heights(&frame, &sources);
        let rings = vec![vec![
            [-5_000.0, -5_000.0],
            [5_000.0, -5_000.0],
            [5_000.0, 5_000.0],
            [-5_000.0, 5_000.0],
        ]];
        let mut pack = FeaturePack::empty();
        pack.regional_landcover.push(RegionalLandcoverRegion {
            location_rad: None,
            rings: rings.clone(),
            kind: LandcoverKind::Farmland,
            source_id: Some(1),
            stable_identity: 1,
        });
        assert!(
            sample(&frame, &sources, Some(&pack), &heights, 1)
                .iter()
                .all(|sample| sample.material == material::FIELD_CROP)
        );
        pack.ground.push((GroundKind::Glacier, rings.clone()));
        assert!(
            sample(&frame, &sources, Some(&pack), &heights, 1)
                .iter()
                .all(|sample| sample.material == material::ICE)
        );
        pack.forest.push(rings);
        assert!(
            sample(&frame, &sources, Some(&pack), &heights, 1)
                .iter()
                .all(|sample| sample.material == material::GRASS)
        );
    }

    #[test]
    fn imagery_failure_during_corner_sampling_marks_generated_cell_incomplete() {
        use std::sync::atomic::{AtomicU64, Ordering};

        struct CornerFailure(AtomicU64);
        impl vearth_data::ColorSource for CornerFailure {
            fn color(&self, _: f64, _: f64) -> Option<[u8; 3]> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Some([80, 113, 72])
            }

            fn sample_color(&self, lat: f64, lon: f64) -> vearth_data::SourceSample<[u8; 3]> {
                let value = self.color(lat, lon);
                vearth_data::SourceSample {
                    value,
                    complete: self.0.load(Ordering::Relaxed) <= (N * N) as u64,
                }
            }

            fn sample_color_detail(
                &self,
                lat: f64,
                lon: f64,
                _: f64,
            ) -> vearth_data::SourceSample<[u8; 3]> {
                self.sample_color(lat, lon)
            }
        }
        let elevation = FnElevation(|_, _| 600.0);
        let imagery = CornerFailure(AtomicU64::new(0));
        let cell = CellId {
            face: Face(0),
            level: 12,
            i: 2048,
            j: 2048,
        };
        let column = crate::generate_surface_cell(
            cell,
            &Sources {
                imagery: Some(&imagery),
                ..Sources::terrain_only(&elevation)
            },
        );
        assert!(imagery.0.load(Ordering::Relaxed) > (N * N) as u64);
        assert!(!column.complete);
        assert!(!column.batches.is_empty());
    }
}
