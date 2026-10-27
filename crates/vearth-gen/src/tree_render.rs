//! Shared transforms and low-detail surfaces for geographically owned trees.

use crate::{SurfaceBatch, tier0::TreeInstance};
use glam::{DMat3, DVec3};
use vearth_core::cell::CellId;
use vearth_core::{
    cell::ChunkAddr,
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex, decode_surface_normal, encode_surface_normal},
};

/// Right-handed east/up/south frame, rotated around the root normal.
pub fn instance_basis(lat: f64, lon: f64, yaw: f64) -> DMat3 {
    let (slat, clat) = lat.sin_cos();
    let (slon, clon) = lon.sin_cos();
    let east = DVec3::new(-slon, clon, 0.0);
    let north = DVec3::new(-slat * clon, -slat * slon, clat);
    let up = DVec3::new(clat * clon, clat * slon, slat);
    let (s, c) = yaw.sin_cos();
    DMat3::from_cols(east * c - north * s, up, -east * s - north * c)
}

/// All geometric LODs use the finest asset's height as their scale reference.
pub fn canonical_height(profile: crate::vegetation::TreeProfileId) -> f32 {
    crate::tree_mesh::canonical_height(profile)
}

pub fn instance_scale(tree: &TreeInstance, canonical_height: f32) -> f64 {
    (f64::from(tree.height_m) / f64::from(canonical_height.max(0.1))).clamp(0.35, 2.2)
}

pub(crate) fn append_surfaces(
    cell: CellId,
    trees: &[TreeInstance],
    chunks: &mut Vec<SurfaceBatch>,
) {
    append_surfaces_with_reuse::<true>(cell, trees, chunks);
}

fn shaded_channel(value: u8, color_scale: f32) -> u8 {
    let value = f32::from(value) / 255.0;
    let linear = if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    } * color_scale;
    let srgb = if linear <= 0.0031308 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round().clamp(0.0, 255.0) as u8
}

fn append_surfaces_with_reuse<const REUSE: bool>(
    cell: CellId,
    trees: &[TreeInstance],
    chunks: &mut Vec<SurfaceBatch>,
) {
    if trees.is_empty() {
        return;
    }
    let zero = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    for tree in trees {
        let asset = crate::tree_mesh::canonical_mesh(tree.profile, 2);
        let basis = instance_basis(tree.lat_rad, tree.lon_rad, f64::from(tree.yaw_rad));
        let origin = vearth_core::ellipsoid::geodetic_to_ecef(
            tree.lat_rad,
            tree.lon_rad,
            f64::from(tree.elevation_m),
        ) + basis.y_axis * 0.04;
        let scale = instance_scale(tree, canonical_height(tree.profile));
        // Choose a nearby origin to keep GPU positions precise.
        let midpoint = origin + basis.y_axis * (f64::from(asset.height_m) * scale * 0.5);
        let k = (zero.ecef_to_local(midpoint).y / 32.0).floor() as i32;
        let index = chunks
            .iter()
            .position(|ch| ch.addr.k == k)
            .unwrap_or_else(|| {
                let addr = ChunkAddr { cell, k };
                chunks.push(SurfaceBatch {
                    shadow_omissions: Vec::new(),
                    detail: None,
                    addr,
                    mesh: MeshBuffers::default(),
                });
                chunks.len() - 1
            });
        let chunk = &mut chunks[index];
        let frame = ChunkFrame::of(chunk.addr);
        let local_to_world = DMat3::from_mat4(frame.model_matrix());
        let color_scale = crate::tier0::near_tree_color_scale(tree.profile, tree.seed);
        let first = chunk.mesh.vertices.len() as u32;
        chunk.mesh.vertices.reserve(asset.mesh.vertices.len());
        chunk.mesh.indices.reserve(asset.mesh.indices.len());
        // The scale is constant for one instance. Cache exact byte results;
        // neither a linear-light approximation nor quantized scale is needed.
        let mut channels = [256_u16; 256];
        let mut last_normal = None;
        for v in &asset.mesh.vertices {
            let point = origin + basis * DVec3::from_array(v.pos.map(f64::from)) * scale;
            let face_ao = if REUSE
                && let Some((source, encoded)) = last_normal
                && source == v.face_ao
            {
                encoded
            } else {
                let world_normal = basis
                    * DVec3::from_array(
                        decode_surface_normal(v.face_ao)
                            .expect("tree facet normal")
                            .map(f64::from),
                    );
                let normal = (local_to_world.transpose() * world_normal)
                    .normalize()
                    .as_vec3();
                let encoded = encode_surface_normal(v.face_ao, normal.to_array());
                last_normal = Some((v.face_ao, encoded));
                encoded
            };
            let rgb = std::array::from_fn::<_, 3, _>(|i| {
                let value = v.rgba[i];
                if !REUSE {
                    return shaded_channel(value, color_scale);
                }
                let cached = &mut channels[usize::from(value)];
                if *cached == 256 {
                    *cached = u16::from(shaded_channel(value, color_scale));
                }
                *cached as u8
            });
            let local = frame.ecef_to_local(point);
            chunk.mesh.vertices.push(SurfaceVertex {
                pos: local.to_array().map(|v| v as f32),
                face_ao,
                rgba: [rgb[0], rgb[1], rgb[2], 255],
            });
        }
        chunk
            .mesh
            .indices
            .extend(asset.mesh.indices.iter().map(|i| first + i));
    }
    chunks.sort_unstable_by_key(|ch| ch.addr.k);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reuse_fixture(level: u8) -> (CellId, Vec<TreeInstance>) {
        let cell = CellId::from_dir(
            vearth_core::ellipsoid::geodetic_to_ecef(0.71, -1.3, 0.0),
            level,
        );
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let trees = crate::vegetation::TREE_PROFILES
            .iter()
            .flat_map(|profile| {
                (0..5).map(move |variant| {
                    let x = 0.5 + variant as f32 * 5.5;
                    let z = 0.5 + profile.id as u8 as f32 % 30.0;
                    let (lat_rad, lon_rad) =
                        frame.column_latlon(f64::from(x) / 32.0, f64::from(z) / 32.0);
                    TreeInstance {
                        profile: profile.id,
                        seed: variant * 17 + 37,
                        lat_rad,
                        lon_rad,
                        elevation_m: variant as f32 * 120.0 - 60.0,
                        height_m: canonical_height(profile.id) * (0.5 + variant as f32 * 0.4),
                        yaw_rad: variant as f32 * 1.3,
                        cell_x: x,
                        cell_z: z,
                    }
                })
            })
            .collect();
        (cell, trees)
    }

    fn batch_fingerprints(
        batches: &[SurfaceBatch],
    ) -> Vec<(ChunkAddr, vearth_core::contracts::Fingerprint)> {
        batches
            .iter()
            .map(|batch| {
                (
                    batch.addr,
                    vearth_core::contracts::mesh_semantic_fingerprint(&batch.mesh),
                )
            })
            .collect()
    }

    #[test]
    fn streamed_tree_reuse_preserves_every_ordered_vertex_and_payload() {
        for level in [11, 15, 19] {
            let (cell, trees) = reuse_fixture(level);
            let mut original = Vec::new();
            append_surfaces_with_reuse::<false>(cell, &trees, &mut original);
            let mut reused = Vec::new();
            append_surfaces_with_reuse::<true>(cell, &trees, &mut reused);
            assert_eq!(batch_fingerprints(&reused), batch_fingerprints(&original));
        }
    }

    #[test]
    #[ignore = "paired tree streaming benchmark; run without concurrent renderer or build"]
    fn streamed_tree_reuse_benchmark() {
        let (cell, trees) = reuse_fixture(15);
        let mut original_ns = Vec::new();
        let mut reused_ns = Vec::new();
        for iteration in 0..13 {
            for reused in [iteration % 2 == 0, iteration % 2 != 0] {
                let started = vearth_platform::time::Instant::now();
                let mut batches = Vec::new();
                if reused {
                    append_surfaces_with_reuse::<true>(cell, &trees, &mut batches);
                } else {
                    append_surfaces_with_reuse::<false>(cell, &trees, &mut batches);
                }
                let elapsed = started.elapsed().as_nanos();
                std::hint::black_box(batches);
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
            "trees={} original_median_ns={} reused_median_ns={}",
            trees.len(),
            original_ns[original_ns.len() / 2],
            reused_ns[reused_ns.len() / 2]
        );
    }

    #[test]
    fn streamed_lod_two_matches_instanced_geometry_and_normals_in_skewed_frames() {
        use vearth_core::{cell::CellId, cubesphere::Face};

        let mut distinct_lod_heights = 0;
        for profile in crate::vegetation::TREE_PROFILES {
            let coarse = crate::tree_mesh::build_tree_mesh(profile.id, 2);
            let finest = crate::tree_mesh::build_tree_mesh(profile.id, 0);
            assert_eq!(canonical_height(profile.id), finest.height_m);
            distinct_lod_heights += usize::from((coarse.height_m - finest.height_m).abs() > 0.001);
            for (i, j, yaw_rad) in [(425_000, 83_000, 0.7), (46_000, 310_000, 3.9)] {
                let cell = CellId {
                    face: Face(0),
                    level: 19,
                    i,
                    j,
                };
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                let (lat_rad, lon_rad) = frame.column_latlon(0.3 / 32.0, 31.7 / 32.0);
                let tree = TreeInstance {
                    profile: profile.id,
                    seed: 37,
                    lat_rad,
                    lon_rad,
                    elevation_m: 123.25,
                    height_m: finest.height_m * 1.27,
                    yaw_rad,
                    cell_x: 0.3,
                    cell_z: 31.7,
                };
                let basis = instance_basis(lat_rad, lon_rad, f64::from(yaw_rad));
                let origin = vearth_core::ellipsoid::geodetic_to_ecef(lat_rad, lon_rad, 123.25)
                    + basis.y_axis * 0.04;
                // The instanced draw transforms every geometric LOD with the
                // finest asset's scale; the streamed copy must use that same
                // physical transform, even when LOD 2 has another tip height.
                let scale = instance_scale(&tree, finest.height_m);
                let model = glam::DMat4::from_cols(
                    (basis.x_axis * scale).extend(0.0),
                    (basis.y_axis * scale).extend(0.0),
                    (basis.z_axis * scale).extend(0.0),
                    origin.extend(1.0),
                );
                let mut chunks = Vec::new();
                append_surfaces(cell, &[tree], &mut chunks);
                assert_eq!(chunks.len(), 1);
                let chunk = &chunks[0];
                let frame = ChunkFrame::of(chunk.addr);
                assert_eq!(chunk.mesh.indices, coarse.mesh.indices);
                assert_eq!(chunk.mesh.vertices.len(), coarse.mesh.vertices.len());
                let normal_to_world = DMat3::from_mat4(frame.model_matrix()).inverse().transpose();
                for (streamed, object) in chunk.mesh.vertices.iter().zip(&coarse.mesh.vertices) {
                    let expected =
                        model.transform_point3(DVec3::from_array(object.pos.map(f64::from)));
                    let actual =
                        frame.local_to_ecef(DVec3::from_array(streamed.pos.map(f64::from)));
                    let expected_local = frame.ecef_to_local(expected);
                    let actual_local = DVec3::from_array(streamed.pos.map(f64::from));
                    assert!(
                        (actual_local - expected_local).abs().max_element() <= 1.0 / 128.0 + 1e-7,
                        "{} fallback exceeds half a position quantum",
                        profile.name
                    );
                    let bound_m = frame.grid_scale.element_sum() / 128.0;
                    assert!(
                        actual.distance(expected) <= bound_m + 1e-7,
                        "{} fallback changed scale or root position",
                        profile.name
                    );
                    let expected_normal = basis
                        * DVec3::from_array(
                            decode_surface_normal(object.face_ao)
                                .unwrap()
                                .map(f64::from),
                        );
                    let actual_normal = (normal_to_world
                        * DVec3::from_array(
                            decode_surface_normal(streamed.face_ao)
                                .unwrap()
                                .map(f64::from),
                        ))
                    .normalize();
                    assert!(
                        actual_normal.dot(expected_normal) > 0.998,
                        "{} fallback normal changed direction",
                        profile.name
                    );
                    assert_eq!(streamed.face_ao & 0xffff, object.face_ao & 0xffff);
                }
            }
        }
        assert!(
            distinct_lod_heights > 0,
            "the correspondence fixture must exercise different measured LOD heights"
        );
    }

    #[test]
    fn tree_frames_are_right_handed_and_geodetically_upright() {
        for lat in [-1.5, -0.7, 0.0, 0.8, 1.5] {
            for yaw in [0.0, 1.0, 4.5] {
                let b = instance_basis(lat, 2.3, yaw);
                assert!((b.determinant() - 1.0).abs() < 1e-12);
                let origin = vearth_core::ellipsoid::geodetic_to_ecef(lat, 2.3, 0.0);
                assert!(
                    b.y_axis
                        .dot(vearth_core::ellipsoid::geodetic_normal(origin))
                        > 0.999999
                );
            }
        }
    }
}
