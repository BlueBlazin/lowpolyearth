//! Spatially clipped direct landmark meshes in the canonical geographic frame.
use super::{ActiveLandmark, CALIBRATION_MATERIAL_FLAG, assets, calibration_override};
use glam::{DMat3, DVec3, Vec2};
use vearth_core::{
    cubesphere,
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal},
};

pub struct LandmarkSurfaceData {
    pub mesh: MeshBuffers,
    pub collision: Vec<crate::collision::CollisionTriangle>,
}

/// An inspection mesh in the landmark's metre frame, before georeferencing.
pub fn canonical_mesh(name: &str, lod: usize) -> Option<MeshBuffers> {
    let asset = assets::get(name)?;
    Some(asset.level(lod).mesh.clone())
}

#[derive(Clone, Copy)]
struct Corner {
    position: DVec3,
    normal: DVec3,
}

#[derive(Clone, Copy)]
struct ClipPlane {
    normal: DVec3,
    offset: f64,
}

impl ClipPlane {
    fn distance(self, position: DVec3) -> f64 {
        self.normal.dot(position) + self.offset
    }
}

fn cell_planes(frame: &ChunkFrame) -> [ClipPlane; 4] {
    let cell = frame.addr.cell;
    let (u0, v0, u1, v1) = cell.uv_bounds();
    let normal = cell.face.normal();
    let right = cell.face.right();
    let up = cell.face.up();
    let tangent = |a: f64| (a * std::f64::consts::FRAC_PI_4).tan();
    // Neighboring cells share these ECEF planes at every altitude. Their
    // independent tangent-frame x/z bounds leave gaps above the ellipsoid.
    [
        right - normal * tangent(u0),
        normal * tangent(u1) - right,
        up - normal * tangent(v0),
        normal * tangent(v1) - up,
    ]
    .map(|n| {
        let n = n.normalize();
        ClipPlane {
            normal: DVec3::new(
                n.dot(frame.x_axis) * frame.grid_scale.x,
                n.dot(frame.y_axis) * frame.grid_scale.y,
                n.dot(frame.z_axis) * frame.grid_scale.z,
            ),
            offset: n.dot(frame.origin),
        }
    })
}

fn clip(polygon: &[Corner], plane: ClipPlane, result: &mut Vec<Corner>) {
    result.clear();
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let da = plane.distance(a.position);
        let db = plane.distance(b.position);
        if da >= 0.0 {
            result.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            let mut p = a.position.lerp(b.position, t);
            p -= plane.normal * (plane.distance(p) / plane.normal.length_squared());
            result.push(Corner {
                position: p,
                normal: a.normal.lerp(b.normal, t).normalize_or_zero(),
            });
        }
    }
}

fn world_point(active: &ActiveLandmark, point: DVec3) -> DVec3 {
    let e = point.x * active.cos_h + point.z * active.sin_h;
    let n = -point.x * active.sin_h + point.z * active.cos_h;
    active.anchor_ecef
        + active.anchor_east * e
        + active.anchor_north * n
        + active.anchor_up * (point.y + f64::from(active.model_base_geo(point.x, point.z)))
}

fn model_point(active: &ActiveLandmark, world: DVec3) -> DVec3 {
    let delta = world - active.anchor_ecef;
    let e = delta.dot(active.anchor_east);
    let n = delta.dot(active.anchor_north);
    DVec3::new(
        e * active.cos_h - n * active.sin_h,
        delta.dot(active.anchor_up),
        e * active.sin_h + n * active.cos_h,
    )
}

fn candidate_bounds(
    frame: &ChunkFrame,
    lm: &ActiveLandmark,
    level: &assets::Level,
) -> (Vec2, Vec2) {
    let mut low = Vec2::splat(f32::INFINITY);
    let mut high = Vec2::splat(f32::NEG_INFINITY);
    let (min_base, max_base) = lm
        .flatten_bases
        .iter()
        .map(|base| f64::from(*base + lm.calibration_base_offset))
        .fold(
            (f64::from(lm.base_geo), f64::from(lm.base_geo)),
            |(a, b), h| (a.min(h), b.max(h)),
        );
    let heights = [
        min_base + f64::from(level.low.y),
        max_base + f64::from(level.high.y),
    ];
    let (u0, v0, u1, v1) = frame.addr.cell.uv_bounds();
    for u in [u0, u1] {
        for v in [v0, v1] {
            let direction = cubesphere::face_uv_to_dir(frame.addr.cell.face, u, v);
            let denominator = direction.dot(lm.anchor_up);
            // A very coarse cell can cross the model tangent-plane horizon.
            // Its unbounded section must query the complete source mesh.
            if denominator <= 1e-8 {
                return (
                    Vec2::new(level.low.x, level.low.z),
                    Vec2::new(level.high.x, level.high.z),
                );
            }
            for height in heights {
                let distance = (lm.anchor_ecef.dot(lm.anchor_up) + height) / denominator;
                let p = model_point(lm, direction * distance);
                let p = Vec2::new(p.x as f32, p.z as f32);
                low = low.min(p);
                high = high.max(p);
            }
        }
    }
    (low - Vec2::splat(0.02), high + Vec2::splat(0.02))
}

/// Transform only indexed source triangles intersecting this terrain cell.
/// The shared landmark LOD locks keep adjacent fragments on one source mesh.
pub fn generate(frame: &ChunkFrame, active: &[ActiveLandmark]) -> LandmarkSurfaceData {
    generate_with_reuse::<true>(frame, active)
}

fn generate_with_reuse<const REUSE: bool>(
    frame: &ChunkFrame,
    active: &[ActiveLandmark],
) -> LandmarkSurfaceData {
    let mut result = LandmarkSurfaceData {
        mesh: MeshBuffers::default(),
        collision: Vec::new(),
    };
    let metric = crate::collision::MetricFrame::of(frame);
    let normal_to_local = DMat3::from_mat4(frame.model_matrix()).transpose();
    let planes = cell_planes(frame);
    // A triangle clipped against the cell stays a small convex polygon. Reuse
    // capacity across source triangles without imposing a new geometry limit.
    let mut polygon = Vec::new();
    let mut clipped = Vec::new();
    for lm in active {
        let Some(asset) = assets::get(lm.lm.name) else {
            continue;
        };
        let lod = if frame.sample_pitch_m() <= 1.4 {
            0
        } else if frame.sample_pitch_m() <= 5.6 {
            1
        } else {
            2
        };
        let level = asset.level(lod);
        let (low, high) = candidate_bounds(frame, lm, level);
        let basis = DMat3::from_cols(
            lm.anchor_east * lm.cos_h - lm.anchor_north * lm.sin_h,
            lm.anchor_up,
            lm.anchor_east * lm.sin_h + lm.anchor_north * lm.cos_h,
        );
        let tagged = calibration_override().is_some_and(|v| v.name == lm.lm.name);
        // Source triangles share indexed vertices. A direct-mapped scratch
        // cache has a fixed budget even for a very large landmark; collisions
        // simply repeat the identical transform instead of changing geometry.
        let cache_len = level.mesh.vertices.len().clamp(1, 4096).next_power_of_two();
        let mut transformed = if REUSE {
            vec![None; cache_len]
        } else {
            Vec::new()
        };
        for triangle in level.candidates(low, high) {
            let ids = &level.mesh.indices[triangle as usize * 3..triangle as usize * 3 + 3];
            let first = level.mesh.vertices[ids[0] as usize];
            // The model's east/up/north frame is left handed in ECEF.
            polygon.clear();
            polygon.extend(ids.iter().rev().map(|&i| {
                let slot = i as usize & (cache_len - 1);
                if REUSE
                    && let Some((id, corner)) = transformed[slot]
                    && id == i
                {
                    return corner;
                }
                let p = level.mesh.vertices[i as usize].pos.map(f64::from);
                let corner = Corner {
                    position: frame.ecef_to_local(world_point(lm, DVec3::from_array(p))),
                    normal: (normal_to_local
                        * basis
                        * DVec3::from_array(level.normals[i as usize].map(f64::from)))
                    .normalize(),
                };
                if REUSE {
                    transformed[slot] = Some((i, corner));
                }
                corner
            }));
            for plane in planes {
                if !REUSE {
                    clipped = Vec::new();
                }
                clip(&polygon, plane, &mut clipped);
                std::mem::swap(&mut polygon, &mut clipped);
                if polygon.len() < 3 {
                    break;
                }
            }
            if polygon.len() < 3 {
                continue;
            }
            let material = ((first.face_ao >> 5) & 255) as u8
                | if tagged { CALIBRATION_MATERIAL_FLAG } else { 0 };
            let base = result.mesh.vertices.len() as u32;
            result
                .mesh
                .vertices
                .extend(polygon.iter().map(|p| SurfaceVertex {
                    pos: p.position.as_vec3().to_array(),
                    face_ao: encode_surface_normal(
                        2 | (3 << 3) | (u32::from(material) << 5),
                        p.normal.as_vec3().to_array(),
                    ),
                    rgba: first.rgba,
                }));
            for i in 1..polygon.len().saturating_sub(1) {
                if (polygon[i].position - polygon[0].position)
                    .cross(polygon[i + 1].position - polygon[0].position)
                    .length_squared()
                    < 1e-14
                {
                    continue;
                }
                result
                    .mesh
                    .indices
                    .extend([base, base + i as u32, base + i as u32 + 1]);
                if material & !CALIBRATION_MATERIAL_FLAG != crate::material::WATER {
                    if let Some(triangle) = crate::collision::triangle_from_local(
                        [0, i, i + 1].map(|j| result.mesh.vertices[base as usize + j].pos),
                        &metric,
                    ) {
                        result.collision.push(triangle);
                    }
                }
            }
        }
    }
    crate::built_surface::compact_mesh(&mut result.mesh);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        cubesphere::Face,
        ellipsoid,
    };

    fn clipped_world(frame: &ChunkFrame, points: &[DVec3]) -> Vec<DVec3> {
        let mut polygon: Vec<_> = points
            .iter()
            .map(|&p| Corner {
                position: frame.ecef_to_local(p),
                normal: DVec3::Y,
            })
            .collect();
        let mut scratch = Vec::new();
        for plane in cell_planes(frame) {
            clip(&polygon, plane, &mut scratch);
            std::mem::swap(&mut polygon, &mut scratch);
        }
        polygon
            .into_iter()
            .map(|p| frame.local_to_ecef(p.position))
            .collect()
    }

    fn polygon_area(points: &[DVec3]) -> f64 {
        (1..points.len().saturating_sub(1))
            .map(|i| {
                (points[i] - points[0])
                    .cross(points[i + 1] - points[0])
                    .length()
                    * 0.5
            })
            .sum()
    }

    #[test]
    fn angular_cell_clips_partition_elevated_and_sea_level_surfaces() {
        // This boundary passes through Biete's far pit wall. Exercise equal
        // and mixed resolutions: no positive-area fragment may be owned twice.
        for altitude in [0.0, 2424.269, 5000.0] {
            for level in [16, 18] {
                let cell = CellId {
                    face: Face(0),
                    level,
                    i: 61196 << (level - 16),
                    j: 43870 << (level - 16),
                };
                let (_, v0, u1, v1) = cell.uv_bounds();
                let direction = cubesphere::face_uv_to_dir(cell.face, u1, (v0 + v1) * 0.5);
                let center = ellipsoid::surface_point_along(direction) + direction * altitude;
                let across = (cell.face.right()
                    - cell.face.normal() * (u1 * std::f64::consts::FRAC_PI_4).tan())
                .normalize();
                let triangle = [
                    center - across * 3.0 - direction * 3.0,
                    center + across * 3.0 - direction * 3.0,
                    center + direction * 3.0,
                ];
                let neighbor = CellId {
                    i: cell.i + 1,
                    ..cell
                };
                for mixed in [false, true] {
                    let mut cells = vec![cell];
                    if mixed {
                        cells.extend(
                            neighbor
                                .children()
                                .into_iter()
                                .filter(|child| child.i == neighbor.i * 2),
                        );
                    } else {
                        cells.push(neighbor);
                    }
                    let mut area = 0.0;
                    let mut seam = Vec::new();
                    for owner in cells {
                        let frame = ChunkFrame::of(ChunkAddr { cell: owner, k: 0 });
                        let polygon = clipped_world(&frame, &triangle);
                        area += polygon_area(&polygon);
                        for p in polygon {
                            assert!(
                                cell_planes(&frame)
                                    .iter()
                                    .all(|plane| plane.distance(frame.ecef_to_local(p)) >= -1e-7)
                            );
                            if across.dot(p - center).abs() < 1e-7 {
                                seam.push(p);
                            }
                        }
                    }
                    assert!(
                        (area - polygon_area(&triangle)).abs() < 2e-6,
                        "altitude={altitude} level={level} mixed={mixed}: area={area}"
                    );
                    assert!(seam.len() >= 4);
                    // Both owners recover the two source-edge intersections.
                    for expected in [center - direction * 3.0, center + direction * 3.0] {
                        assert!(seam.iter().filter(|p| p.distance(expected) < 1e-6).count() >= 2);
                    }
                }
            }
        }
    }

    fn ray_triangle(origin: DVec3, direction: DVec3, p: [DVec3; 3]) -> Option<f64> {
        let e1 = p[1] - p[0];
        let e2 = p[2] - p[0];
        let cross = direction.cross(e2);
        let determinant = e1.dot(cross);
        if determinant.abs() < 1e-12 {
            return None;
        }
        let delta = origin - p[0];
        let u = delta.dot(cross) / determinant;
        let q = delta.cross(e1);
        let v = direction.dot(q) / determinant;
        let t = e2.dot(q) / determinant;
        (u >= -1e-8 && v >= -1e-8 && u + v <= 1.0 + 1e-8 && t > 0.0).then_some(t)
    }

    #[test]
    fn biete_corner_survives_cell_clipping_and_candidate_search() {
        let asset = assets::get("biete-giyorgis").unwrap();
        let level = asset.level(1);
        let mut fragments = Vec::new();
        let mut canonical = Vec::new();
        let mut ray_origin = DVec3::ZERO;
        let mut ray_targets = Vec::new();
        for i in [61196, 61197] {
            let cell = CellId {
                face: Face(0),
                level: 16,
                i,
                j: 43870,
            };
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let active = super::super::active_for(cell, &frame, &|_, _| 2420.681);
            let lm = active
                .iter()
                .find(|lm| lm.lm.name == "biete-giyorgis")
                .unwrap();
            let (low, high) = candidate_bounds(&frame, lm, level);
            let candidates = level.candidates(low, high);
            for (id, ids) in level.mesh.indices.chunks_exact(3).enumerate() {
                let triangle = [0, 1, 2].map(|j| {
                    world_point(
                        lm,
                        DVec3::from_array(level.mesh.vertices[ids[j] as usize].pos.map(f64::from)),
                    )
                });
                let clipped = clipped_world(&frame, &triangle);
                if polygon_area(&clipped) > 1e-9 {
                    assert!(
                        candidates.binary_search(&(id as u32)).is_ok(),
                        "omitted source triangle {id}"
                    );
                }
                if i == 61196 {
                    canonical.push(triangle);
                }
            }
            let generated = generate(&frame, &active);
            let metric = crate::collision::MetricFrame::of(&frame);
            let expected_collision: Vec<_> = generated
                .mesh
                .indices
                .chunks_exact(3)
                .filter_map(|ids| {
                    crate::collision::triangle_from_local(
                        [0, 1, 2].map(|j| generated.mesh.vertices[ids[j] as usize].pos),
                        &metric,
                    )
                })
                .collect();
            assert_eq!(generated.collision, expected_collision);
            fragments.extend(generated.mesh.indices.chunks_exact(3).map(|ids| {
                [0, 1, 2].map(|j| {
                    frame.local_to_ecef(DVec3::from_array(
                        generated.mesh.vertices[ids[j] as usize].pos.map(f64::from),
                    ))
                })
            }));
            ray_origin = world_point(lm, DVec3::new(-27.24187259, 37.85283180, -38.90542604));
            ray_targets = [
                [14.37647205, 3.10136981, 12.09460662],
                [14.35883713, 2.73814928, 12.17002651],
                [14.32886753, 1.66598657, 12.30747623],
                [14.28885608, 0.52255693, 12.55929798],
            ]
            .map(|p| world_point(lm, DVec3::from_array(p)))
            .to_vec();
        }
        for target in ray_targets {
            let direction = (target - ray_origin).normalize();
            let nearest = |triangles: &[[DVec3; 3]]| {
                triangles
                    .iter()
                    .filter_map(|p| ray_triangle(ray_origin, direction, *p))
                    .reduce(f64::min)
                    .unwrap()
            };
            assert!(
                (nearest(&canonical) - nearest(&fragments)).abs() < 0.002,
                "pit wall lost along observed seam ray"
            );
        }
    }

    fn reuse_fixture(name: &str, level: u8) -> (ChunkFrame, Vec<ActiveLandmark>) {
        let lm = super::super::LANDMARKS
            .iter()
            .find(|lm| lm.name == name)
            .unwrap();
        let cell = CellId::from_dir(
            ellipsoid::geodetic_to_ecef(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), 0.0)
                .normalize(),
            level,
        );
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let active = super::super::active_for(cell, &frame, &|_, _| 10.0);
        (frame, active)
    }

    #[test]
    fn landmark_scratch_reuse_preserves_clipping_and_collision_exactly() {
        for name in [
            "washington-monument",
            "sydney-opera-house",
            "biete-giyorgis",
        ] {
            for level in [15, 16, 19] {
                let (frame, active) = reuse_fixture(name, level);
                let original = generate_with_reuse::<false>(&frame, &active);
                let reused = generate_with_reuse::<true>(&frame, &active);
                assert_eq!(
                    vearth_core::contracts::mesh_semantic_fingerprint(&original.mesh),
                    vearth_core::contracts::mesh_semantic_fingerprint(&reused.mesh),
                    "{name} L{level}"
                );
                assert_eq!(original.collision, reused.collision, "{name} L{level}");
            }
        }
    }

    #[test]
    #[ignore = "paired landmark streaming benchmark; run without concurrent renderer or build"]
    fn landmark_scratch_reuse_benchmark() {
        let (frame, active) = reuse_fixture("sydney-opera-house", 16);
        let mut original_ns = Vec::new();
        let mut reused_ns = Vec::new();
        for iteration in 0..13 {
            for reused in [iteration % 2 == 0, iteration % 2 != 0] {
                let started = vearth_platform::time::Instant::now();
                let result = if reused {
                    generate_with_reuse::<true>(&frame, &active)
                } else {
                    generate_with_reuse::<false>(&frame, &active)
                };
                let elapsed = started.elapsed().as_nanos();
                std::hint::black_box(result);
                if iteration > 0 {
                    if reused {
                        &mut reused_ns
                    } else {
                        &mut original_ns
                    }
                    .push(elapsed);
                }
            }
        }
        original_ns.sort_unstable();
        reused_ns.sort_unstable();
        eprintln!(
            "original_median_ns={} reused_median_ns={}",
            original_ns[original_ns.len() / 2],
            reused_ns[reused_ns.len() / 2]
        );
    }

    #[test]
    fn new_monuments_generate_clipped_collision_at_every_lod() {
        for lm in super::super::monument_generated::MONUMENT_MODELS {
            for level in [13, 17, 19] {
                let (_, initial) = reuse_fixture(lm.name, level);
                let owner = initial
                    .iter()
                    .find(|a| a.lm.name == lm.name)
                    .unwrap_or_else(|| panic!("{} inactive at L{level}", lm.name));
                let mesh = canonical_mesh(lm.name, 2).unwrap();
                let center = mesh.indices[..3]
                    .iter()
                    .map(|&i| DVec3::from_array(mesh.vertices[i as usize].pos.map(f64::from)))
                    .sum::<DVec3>()
                    / 3.0;
                let cell = CellId::from_dir(world_point(owner, center).normalize(), level);
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                let active = super::super::active_for(cell, &frame, &|_, _| 10.0);
                let result = generate(&frame, &active);
                assert!(!result.mesh.indices.is_empty(), "{} L{level}", lm.name);
                assert!(
                    !result.collision.is_empty(),
                    "{} collision L{level}",
                    lm.name
                );
                for vertex in &result.mesh.vertices {
                    assert!(
                        cell_planes(&frame).iter().all(|plane| {
                            plane.distance(DVec3::from_array(vertex.pos.map(f64::from))) >= -0.002
                        }),
                        "{} clipped bounds L{level}",
                        lm.name
                    );
                }
            }
        }
    }

    #[test]
    fn direct_monument_fragments_are_clipped_and_metric() {
        let lm = super::super::LANDMARKS
            .iter()
            .find(|l| l.name == "washington-monument")
            .unwrap();
        let cell = CellId::from_dir(
            ellipsoid::geodetic_to_ecef(lm.lat_deg.to_radians(), lm.lon_deg.to_radians(), 0.)
                .normalize(),
            19,
        );
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let active = super::super::active_for(cell, &frame, &|_, _| 10.);
        let result = generate(&frame, &active);
        assert!(!result.mesh.indices.is_empty());
        assert!(!result.collision.is_empty());
        for vertex in &result.mesh.vertices {
            assert!(cell_planes(&frame).iter().all(|plane| {
                plane.distance(DVec3::from_array(vertex.pos.map(f64::from))) >= -0.001
            }));
        }
        let metric = crate::collision::MetricFrame::of(&frame);
        let t = &result.collision[0];
        for (i, p) in t.vertices.iter().enumerate() {
            let q =
                metric.point_from_local(result.mesh.vertices[result.mesh.indices[i] as usize].pos);
            assert_eq!(*p, q);
        }
    }
}
