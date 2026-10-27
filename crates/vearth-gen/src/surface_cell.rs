//! Source-generated world data. These records contain no hidden voxel volume.

use crate::{
    NightLightCluster, RoadGuide, TerrainCollision, built_surface::BuiltDetailBounds,
    collision::SurfaceCollision, flora::FloraCell, tier0::TreeInstance,
};
use vearth_core::{
    cell::{CellId, ChunkAddr},
    mesh::MeshBuffers,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceAppearance {
    pub material: u8,
    pub tint: [u8; 3],
}

#[derive(Clone)]
pub struct SurfaceBatch {
    pub addr: ChunkAddr,
    pub mesh: MeshBuffers,
    pub detail: Option<BuiltDetailBounds>,
    pub shadow_omissions: Vec<ShadowOmission>,
}

/// Triangle spans backed by an emitted opaque surface at a bounded distance.
/// A renderer may omit them from a shadow map only below its texel footprint.
/// Offsets address index-buffer elements; vertex compaction preserves them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowOmission {
    pub first_index: u32,
    pub index_count: u32,
    pub max_deviation_m: f32,
}

impl SurfaceBatch {
    pub fn validate_shadow_omissions(&self) -> bool {
        let mut end = 0u64;
        self.shadow_omissions.iter().all(|span| {
            let first = u64::from(span.first_index);
            let next = first + u64::from(span.index_count);
            let valid = first >= end
                && span.first_index.is_multiple_of(3)
                && span.index_count > 0
                && span.index_count.is_multiple_of(3)
                && next <= self.mesh.indices.len() as u64
                && span.max_deviation_m.is_finite()
                && span.max_deviation_m >= 0.0;
            end = next;
            valid
        })
    }
}

/// The generated cache and GPU preparation consume the same surface geometry.
/// Collision uses metric triangles/capsules and the shared ground height field.
#[derive(Clone)]
pub struct SurfaceCell {
    pub cell: CellId,
    pub batches: Vec<SurfaceBatch>,
    pub height_range: (f32, f32),
    /// Physical relief of source-owned terrain corners, in metres. Excludes
    /// visual seam skirts and the heights of separate structures or vegetation.
    pub terrain_relief_m: f32,
    pub terrain_step_m: f32,
    /// Physical terrain samples for geographically local descendant metrics.
    /// Absent when sampled elevation dependencies were unavailable.
    pub terrain_detail: Option<crate::TerrainDetailGrid>,
    pub terrain_collision: Option<TerrainCollision>,
    /// Solid surfaces and liquid contact tops share the gameplay collision index.
    pub solid_collision: SurfaceCollision,
    pub water: Vec<crate::water_surface::WaterTriangle>,
    pub road_guides: Vec<RoadGuide>,
    pub night_lights: Vec<NightLightCluster>,
    pub tree_instances: Vec<TreeInstance>,
    pub flora: Vec<FloraCell>,
    /// Incomplete source results may be displayed, but are never persisted.
    pub complete: bool,
}
