//! Shared bounds, mesh compaction and source preparation for continuous structures.

use vearth_core::{cell::ChunkAddr, frame::ChunkFrame, mesh::MeshBuffers};

pub(crate) const BUILDING_BASE_BUDGET_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const ROAD_BASE_BUDGET_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const BRIDGE_BASE_BUDGET_BYTES: usize = 4 * 1024 * 1024;

/// Full-feature bounds, retained across cell clipping for consistent detail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuiltDetailBounds {
    pub center: [f32; 3],
    pub radius_m: f32,
    pub min_screen_px: f32,
}

pub(crate) fn mesh_bytes(mesh: &MeshBuffers) -> usize {
    mesh.vertices.len() * std::mem::size_of::<vearth_core::mesh::SurfaceVertex>()
        + mesh.indices.len() * std::mem::size_of::<u32>()
}

/// Merge only bit-identical vertices, retaining hard edges and material seams.
/// Triangle order and every indexed vertex value remain unchanged. Indices
/// must already have been validated against the source vertex buffer.
pub(crate) fn compact_mesh(mesh: &mut MeshBuffers) {
    let mut vertices = Vec::new();
    let mut unique = rustc_hash::FxHashMap::default();
    for index in &mut mesh.indices {
        let vertex = mesh.vertices[*index as usize];
        let key = (vertex.pos.map(f32::to_bits), vertex.face_ao, vertex.rgba);
        *index = *unique.entry(key).or_insert_with(|| {
            let index = vertices.len() as u32;
            vertices.push(vertex);
            index
        });
    }
    mesh.vertices = vertices;
}

/// Share identical vertex records while retaining every source value, including
/// unreferenced records that contribute to conservative bounds or calibration.
/// Indices must have been validated; primitive order is unchanged.
pub fn compact_mesh_preserving_vertices(mesh: &mut MeshBuffers) {
    let source = std::mem::take(&mut mesh.vertices);
    let mut remap = Vec::with_capacity(source.len());
    let mut unique = rustc_hash::FxHashMap::default();
    for vertex in source {
        let key = (vertex.pos.map(f32::to_bits), vertex.face_ao, vertex.rgba);
        remap.push(*unique.entry(key).or_insert_with(|| {
            let index = mesh.vertices.len() as u32;
            mesh.vertices.push(vertex);
            index
        }));
    }
    for index in &mut mesh.indices {
        *index = remap[*index as usize];
    }
}

pub fn hash_detail(
    hasher: &mut vearth_core::contracts::SemanticHasher,
    detail: Option<BuiltDetailBounds>,
) {
    hasher.bool(detail.is_some());
    if let Some(detail) = detail {
        for value in detail.center {
            hasher.f32(value);
        }
        hasher.f32(detail.radius_m);
        hasher.f32(detail.min_screen_px);
    }
}

/// Fixed-detail caps covering every building, bridge, and inland-water datum
/// read the cell can make. Building and water datums are covered per feature
/// (see `DatumCover`), so a coarse cell prepares full-detail tiles only near
/// what samples them.
pub fn geometry_source_footprints(
    cell: vearth_core::cell::CellId,
    pack: &vearth_data::vectors::FeaturePack,
) -> Vec<crate::SourceFootprint> {
    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let mut cover = crate::DatumCover::new(
        &frame,
        crate::datum_pitch_m(frame.column_latlon(0.5, 0.5).0),
    );
    crate::building_surface::add_datum_cover(&frame, pack, &mut cover);
    crate::water_surface::add_datum_cover(&frame, pack, &mut cover);
    let mut footprints = cover.footprints();
    footprints.extend(crate::bridges::source_footprint(&frame, pack));
    footprints
}

pub fn roof_source_footprint(
    cell: vearth_core::cell::CellId,
    pack: &vearth_data::vectors::FeaturePack,
) -> Option<crate::SourceFootprint> {
    crate::building_surface::roof_source_footprint(&ChunkFrame::of(ChunkAddr { cell, k: 0 }), pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::mesh::SurfaceVertex;
    #[test]
    fn vertex_compaction_retains_exact_triangles_and_hard_edges() {
        let mut mesh = MeshBuffers {
            vertices: [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
                .map(|pos| SurfaceVertex {
                    pos,
                    face_ao: 42,
                    rgba: [128; 4],
                })
                .to_vec(),
            indices: vec![0, 1, 2],
        };
        let original = mesh.clone();
        let mut hard = mesh.vertices[0];
        hard.face_ao ^= 1;
        mesh.vertices.extend(original.vertices.clone());
        mesh.vertices.push(hard);
        mesh.indices.extend([3, 4, 5, 6, 1, 2]);
        let triangles: Vec<_> = mesh
            .indices
            .iter()
            .map(|&i| mesh.vertices[i as usize])
            .collect();
        compact_mesh(&mut mesh);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(
            triangles,
            mesh.indices
                .iter()
                .map(|&i| mesh.vertices[i as usize])
                .collect::<Vec<_>>()
        );
    }
}
