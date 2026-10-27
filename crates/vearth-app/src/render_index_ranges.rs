use glam::DVec3;
use vearth_core::{
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex},
};

use super::{RenderBounds, RenderIndexRange};

const MAX_GRID_SIDE: usize = 8;
pub(crate) const MAX_RENDER_INDEX_RANGES: usize = MAX_GRID_SIDE * MAX_GRID_SIDE;
const MIN_TRIANGLES: usize = 32_768;
const TARGET_TRIANGLES: usize = 16_384;

pub(super) fn eligible(mesh: &MeshBuffers) -> bool {
    mesh.indices.len() / 3 >= MIN_TRIANGLES && mesh.indices.len() <= u32::MAX as usize
}

// This is a transient upload layout. Each oriented triangle belongs to one
// range; the grid groups complete triangles and never clips their geometry.
pub(super) fn partition(frame: &ChunkFrame, mesh: &mut MeshBuffers) -> Vec<RenderIndexRange> {
    partition_indices(frame, &mesh.vertices, &mut mesh.indices)
}

pub(super) fn partition_indices(
    frame: &ChunkFrame,
    vertices: &[SurfaceVertex],
    indices: &mut Vec<u32>,
) -> Vec<RenderIndexRange> {
    let triangles = indices.len() / 3;
    if triangles < MIN_TRIANGLES || indices.len() > u32::MAX as usize {
        return Vec::new();
    }
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    for vertex in vertices {
        let point = DVec3::from_array(vertex.pos.map(f64::from));
        lo = lo.min(point);
        hi = hi.max(point);
    }
    let extent = hi - lo;
    if !extent.is_finite() || (extent.x <= 1e-9 && extent.z <= 1e-9) {
        return Vec::new();
    }
    let desired = triangles.div_ceil(TARGET_TRIANGLES);
    let side = (2..=MAX_GRID_SIDE)
        .find(|side| side * side >= desired)
        .unwrap_or(MAX_GRID_SIDE);
    let bin_for = |point: DVec3| {
        let axis = |component: usize| {
            if extent[component] <= 1e-9 {
                0
            } else {
                (((point[component] - lo[component]) / extent[component] * side as f64) as usize)
                    .min(side - 1)
            }
        };
        axis(2) * side + axis(0)
    };
    let mut bins = vec![Vec::<u32>::new(); side * side];
    let mut bin_lo = vec![DVec3::splat(f64::INFINITY); bins.len()];
    let mut bin_hi = vec![DVec3::splat(f64::NEG_INFINITY); bins.len()];
    for ids in indices.chunks_exact(3) {
        let points = [ids[0], ids[1], ids[2]]
            .map(|id| DVec3::from_array(vertices[id as usize].pos.map(f64::from)));
        let bin = bin_for((points[0] + points[1] + points[2]) / 3.0);
        bins[bin].extend_from_slice(ids);
        for point in points {
            bin_lo[bin] = bin_lo[bin].min(point);
            bin_hi[bin] = bin_hi[bin].max(point);
        }
    }
    if bins.iter().filter(|bin| !bin.is_empty()).count() < 2 {
        return Vec::new();
    }
    let mut ranges = Vec::with_capacity(bins.len());
    indices.clear();
    for (id, bin) in bins.into_iter().enumerate() {
        if bin.is_empty() {
            continue;
        }
        let center = (bin_lo[id] + bin_hi[id]) * 0.5;
        let half = (bin_hi[id] - bin_lo[id]) * 0.5;
        // ChunkFrame is sheared on cube faces: its transformed box radius,
        // rather than the length of scaled extents, encloses every corner.
        // The numeric margin covers f32 model/projection roundoff, including
        // large out-of-cell vertices and low-level world geometry.
        let magnitude = (center.abs() + half) * frame.grid_scale.abs();
        let margin = 0.01 + magnitude.element_sum() * f64::from(f32::EPSILON) * 8.0;
        ranges.push(RenderIndexRange {
            first_index: indices.len() as u32,
            index_count: bin.len() as u32,
            bounds: RenderBounds {
                center_ecef: frame.local_to_ecef(center).to_array(),
                radius_m: frame.local_box_radius(half) + margin,
            },
        });
        indices.extend(bin);
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        cubesphere::Face,
        mesh::SurfaceVertex,
    };

    fn scattered() -> MeshBuffers {
        let mut mesh = MeshBuffers::default();
        for i in 0..MIN_TRIANGLES {
            let group = i % 4;
            let x = if group & 1 == 0 { -100.0 } else { 100.0 };
            let z = if group & 2 == 0 { -100.0 } else { 100.0 };
            let base = mesh.vertices.len() as u32;
            for pos in [[x, 1.0, z], [x + 1.0, 1.0, z], [x, 2.0, z + 1.0]] {
                mesh.vertices.push(SurfaceVertex {
                    pos,
                    face_ao: 7 | ((i as u32 % 255) << 5),
                    rgba: [i as u8, 20, 60, 255],
                });
            }
            mesh.indices.extend([base, base + 1, base + 2]);
        }
        mesh
    }

    fn frame(face: u8, level: u8, k: i32) -> ChunkFrame {
        ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face: Face(face),
                level,
                i: (1 << level) - 1,
                j: (1 << level) - 1,
            },
            k,
        })
    }

    #[test]
    fn ranges_preserve_oriented_triangles_vertices_and_conservative_bounds() {
        let mut source = scattered();
        let base = source.vertices.len() as u32;
        for pos in [
            [-200.0, -4.0, -200.0],
            [200.0, 30.0, -200.0],
            [0.0, 70.0, 200.0],
        ] {
            source.vertices.push(SurfaceVertex {
                pos,
                face_ao: 991,
                rgba: [10, 20, 30, 255],
            });
        }
        source.indices.extend([base, base + 1, base + 2]);
        // Unreferenced source records remain untouched in the parent allocation.
        source.vertices.push(SurfaceVertex {
            pos: [300.0, 100.0, 300.0],
            face_ao: 44,
            rgba: [1, 2, 3, 255],
        });
        let mut expected: Vec<_> = source
            .indices
            .chunks_exact(3)
            .map(|ids| [ids[0], ids[1], ids[2]])
            .collect();
        expected.sort_unstable();
        for face in 0..6 {
            for level in [0, 7, 18] {
                for k in [-12, 0, 37] {
                    let frame = frame(face, level, k);
                    let mut mesh = source.clone();
                    let ranges = partition(&frame, &mut mesh);
                    assert!((2..=MAX_RENDER_INDEX_RANGES).contains(&ranges.len()));
                    assert_eq!(mesh.vertices, source.vertices);
                    let mut actual: Vec<_> = mesh
                        .indices
                        .chunks_exact(3)
                        .map(|ids| [ids[0], ids[1], ids[2]])
                        .collect();
                    actual.sort_unstable();
                    assert_eq!(actual, expected);
                    let mut end = 0;
                    for range in ranges {
                        assert_eq!(range.first_index, end);
                        end += range.index_count;
                        let center = DVec3::from_array(range.bounds.center_ecef);
                        for &id in &mesh.indices[range.first_index as usize..end as usize] {
                            let local =
                                DVec3::from_array(mesh.vertices[id as usize].pos.map(f64::from));
                            assert!(
                                frame.local_to_ecef(local).distance(center)
                                    <= range.bounds.radius_m
                            );
                        }
                    }
                    assert_eq!(end as usize, mesh.indices.len());
                }
            }
        }
    }

    #[test]
    fn local_ranges_reject_other_clusters_without_splitting_long_triangles() {
        let mut mesh = scattered();
        let frame = frame(4, 18, -1);
        let ranges = partition(&frame, &mut mesh);
        assert_eq!(ranges.len(), 4);
        for a in 0..ranges.len() {
            for b in a + 1..ranges.len() {
                assert!(
                    DVec3::from_array(ranges[a].bounds.center_ecef)
                        .distance(DVec3::from_array(ranges[b].bounds.center_ecef))
                        > ranges[a].bounds.radius_m + ranges[b].bounds.radius_m
                );
            }
        }
        let mut small = mesh.clone();
        small.indices.truncate(3);
        let before = small.clone();
        assert!(partition(&frame, &mut small).is_empty());
        assert_eq!(small.vertices, before.vertices);
        assert_eq!(small.indices, before.indices);
    }

    #[test]
    fn large_batches_refine_spatial_ranges_without_rewriting_geometry() {
        let mut source = MeshBuffers::default();
        for z in 0..8 {
            for x in 0..8 {
                let base = source.vertices.len() as u32;
                let x = x as f32 * 4.0 + 2.0;
                let z = z as f32 * 4.0 + 2.0;
                for pos in [[x, 1.0, z], [x + 1.0, 1.0, z], [x, 2.0, z + 1.0]] {
                    source.vertices.push(SurfaceVertex {
                        pos,
                        face_ao: base * 17,
                        rgba: [base as u8, 80, 30, 255],
                    });
                }
            }
        }
        // Interleave all clusters so grouping has to move complete triangles.
        for _ in 0..TARGET_TRIANGLES {
            for cluster in 0..64 {
                let base = cluster * 3;
                source.indices.extend([base, base + 1, base + 2]);
            }
        }
        let frame = frame(4, 18, -1);
        let mut mesh = source.clone();
        let ranges = partition(&frame, &mut mesh);
        assert_eq!(ranges.len(), 64);
        assert_eq!(mesh.vertices, source.vertices);
        let mut occurrences = [0usize; 64];
        let mut end = 0;
        for range in &ranges {
            assert_eq!(range.first_index, end);
            assert_eq!(range.index_count as usize, TARGET_TRIANGLES * 3);
            end += range.index_count;
            let center = DVec3::from_array(range.bounds.center_ecef);
            for ids in mesh.indices[range.first_index as usize..end as usize].chunks_exact(3) {
                assert_eq!(ids, [ids[0] / 3 * 3, ids[0] + 1, ids[0] + 2]);
                occurrences[ids[0] as usize / 3] += 1;
                for &id in ids {
                    let point = DVec3::from_array(mesh.vertices[id as usize].pos.map(f64::from));
                    assert!(frame.local_to_ecef(point).distance(center) <= range.bounds.radius_m);
                }
            }
        }
        assert_eq!(occurrences, [TARGET_TRIANGLES; 64]);
        assert_eq!(end as usize, source.indices.len());
        // A small view at one corner admits only its cluster; the other 63
        // remain resident and can be submitted by a different view/cascade.
        let corner = DVec3::from_array(ranges[0].bounds.center_ecef);
        let selected: u32 = ranges
            .iter()
            .filter(|range| {
                DVec3::from_array(range.bounds.center_ecef).distance(corner)
                    <= range.bounds.radius_m + frame.grid_scale.abs().min_element() * 0.1
            })
            .map(|range| range.index_count)
            .sum();
        assert_eq!(selected as usize, TARGET_TRIANGLES * 3);
    }
}
