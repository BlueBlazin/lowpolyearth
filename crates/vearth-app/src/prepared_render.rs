//! Device-independent, transient renderer-ready terrain cells.
//!
//! Workers validate the settled mesh once, choose the exact
//! vertex and index formats, and hand byte-for-byte upload products directly
//! to the renderer. These values are never serialized or stored in the chunk
//! cache: source-generated surface geometry is the persistent terrain product.
//! GPU handles, arena offsets, and dynamic camera/environment uniforms never
//! cross this boundary.

use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use vearth_core::cell::{CellId, ChunkAddr};
use vearth_core::contracts::{Fingerprint, SemanticHasher};
use vearth_core::frame::ChunkFrame;
#[cfg(test)]
use vearth_core::mesh::MeshBuffers;
use vearth_core::mesh::{PackedSurfaceVertex, SurfaceVertex, pack_surface_vertices};
use vearth_gen::collision::SurfaceCollision;
use vearth_gen::flora::FloraCell;
use vearth_gen::tier0::TreeInstance;
use vearth_gen::{NightLightCluster, RoadGuide, SurfaceCell};

#[path = "render_index_ranges.rs"]
mod index_ranges;
pub(crate) use index_ranges::MAX_RENDER_INDEX_RANGES;

const CALIBRATION_TAG: u32 = (vearth_gen::landmarks::CALIBRATION_MATERIAL_FLAG as u32) << 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TerrainVertexFormat {
    Packed12 = 1,
    Full20 = 2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TerrainVertices {
    Packed(Vec<PackedSurfaceVertex>),
    Full(Vec<SurfaceVertex>),
}

impl TerrainVertices {
    pub fn format(&self) -> TerrainVertexFormat {
        match self {
            Self::Packed(_) => TerrainVertexFormat::Packed12,
            Self::Full(_) => TerrainVertexFormat::Full20,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Packed(vertices) => vertices.len(),
            Self::Full(vertices) => vertices.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Packed(vertices) => bytemuck::cast_slice(vertices),
            Self::Full(vertices) => bytemuck::cast_slice(vertices),
        }
    }

    #[cfg(test)]
    fn to_full_vec(&self) -> Vec<SurfaceVertex> {
        match self {
            Self::Packed(vertices) => vertices
                .iter()
                .copied()
                .map(PackedSurfaceVertex::to_full)
                .collect(),
            Self::Full(vertices) => vertices.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TerrainIndexFormat {
    Uint16 = 1,
    Uint32 = 2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerrainIndices {
    U16(Vec<u16>),
    U32(Vec<u32>),
}

impl TerrainIndices {
    pub fn format(&self) -> TerrainIndexFormat {
        match self {
            Self::U16(_) => TerrainIndexFormat::Uint16,
            Self::U32(_) => TerrainIndexFormat::Uint32,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::U16(indices) => indices.len(),
            Self::U32(indices) => indices.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::U16(indices) => bytemuck::cast_slice(indices),
            Self::U32(indices) => bytemuck::cast_slice(indices),
        }
    }

    fn index(&self, index: usize) -> Option<u32> {
        match self {
            Self::U16(indices) => indices.get(index).copied().map(u32::from),
            Self::U32(indices) => indices.get(index).copied(),
        }
    }

    #[cfg(test)]
    fn to_u32_vec(&self) -> Vec<u32> {
        match self {
            Self::U16(indices) => indices.iter().copied().map(u32::from).collect(),
            Self::U32(indices) => indices.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderBounds {
    pub center_ecef: [f64; 3],
    pub radius_m: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderLocalBox {
    /// Local-axis extents around the paired [`RenderBounds::center_ecef`].
    pub half_extents: [f64; 3],
    /// Isotropic allowance for rounded model products, in metres.
    pub padding_m: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderIndexRange {
    pub first_index: u32,
    pub index_count: u32,
    pub bounds: RenderBounds,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PackedShadowIndices {
    pub indices: TerrainIndices,
    pub ranges: Vec<RenderIndexRange>,
    pub max_deviation_m: f32,
}

fn pack_indices(indices: Vec<u32>, indices_u16: bool) -> TerrainIndices {
    if indices_u16 {
        TerrainIndices::U16(indices.into_iter().map(|index| index as u16).collect())
    } else {
        TerrainIndices::U32(indices)
    }
}

fn validate_index_ranges(ranges: &[RenderIndexRange], count: usize) -> Result<(), String> {
    if ranges.is_empty() {
        return Ok(());
    }
    if ranges.len() > MAX_RENDER_INDEX_RANGES {
        return Err("render chunk has too many index ranges".into());
    }
    let mut end = 0u64;
    for range in ranges {
        if u64::from(range.first_index) != end
            || range.index_count == 0
            || !range.index_count.is_multiple_of(3)
            || !range.bounds.radius_m.is_finite()
            || range.bounds.radius_m < 0.0
            || range.bounds.center_ecef.iter().any(|v| !v.is_finite())
        {
            return Err("render chunk has invalid index ranges".into());
        }
        end += u64::from(range.index_count);
    }
    if end != count as u64 {
        return Err("render chunk index ranges do not cover its indices".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct PackedRenderChunk {
    pub detail: Option<vearth_gen::built_surface::BuiltDetailBounds>,
    pub addr: ChunkAddr,
    pub vertices: TerrainVertices,
    pub indices: TerrainIndices,
    pub bounds: RenderBounds,
    pub local_bounds: RenderLocalBox,
    pub has_calibration_landmark: bool,
    /// Whether any stored vertex can enter the shader's tree-discard path.
    pub has_fragment_feedback: bool,
    /// Whether any stored vertex needs the world shader's rare-surface paths.
    pub has_rare_surface: bool,
    /// Empty means draw the entire batch. Nonempty ranges partition its
    /// indices and share the original vertices, transform and admission slot.
    pub ranges: Vec<RenderIndexRange>,
    pub shadow: Option<PackedShadowIndices>,
}

impl PackedRenderChunk {
    pub fn vertex_bytes(&self) -> &[u8] {
        self.vertices.bytes()
    }

    pub fn index_bytes(&self) -> &[u8] {
        self.indices.bytes()
    }

    pub fn combined_index_bytes(&self) -> usize {
        let world = self.index_bytes().len();
        self.shadow.as_ref().map_or(world, |shadow| {
            if shadow.indices.is_empty() {
                world
            } else {
                world.next_multiple_of(4) + shadow.indices.bytes().len()
            }
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(detail) = self.detail {
            if detail.center.iter().any(|v| !v.is_finite())
                || !detail.radius_m.is_finite()
                || detail.radius_m <= 0.0
                || !detail.min_screen_px.is_finite()
                || detail.min_screen_px <= 0.0
            {
                return Err("invalid building detail bounds".into());
            }
        }
        if self.addr.cell.level > 30 {
            return Err("render chunk has an invalid cell level".into());
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err("render chunk index buffer is not triangle aligned".into());
        }
        if !self.bounds.radius_m.is_finite() || self.bounds.radius_m < 0.0 {
            return Err("render chunk has an invalid bounding radius".into());
        }
        if self
            .local_bounds
            .half_extents
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
            || !self.local_bounds.padding_m.is_finite()
            || self.local_bounds.padding_m < 0.0
        {
            return Err("render chunk has invalid local bounds".into());
        }
        if self
            .bounds
            .center_ecef
            .iter()
            .any(|coordinate| !coordinate.is_finite())
        {
            return Err("render chunk has a non-finite bounding center".into());
        }
        for index in 0..self.indices.len() {
            if self.indices.index(index).unwrap() as usize >= self.vertices.len() {
                return Err(format!("render chunk index {index} is out of bounds"));
            }
        }
        let mut scanned_calibration = false;
        let mut scanned_feedback = false;
        let mut scanned_rare = false;
        let mut inspect = |face_ao: u32| {
            scanned_calibration |= face_ao & CALIBRATION_TAG != 0;
            scanned_feedback |= material_has_fragment_feedback(face_ao);
            scanned_rare |= material_has_rare_surface(face_ao);
        };
        match &self.vertices {
            TerrainVertices::Packed(vertices) => {
                for vertex in vertices {
                    inspect(vertex.to_full().face_ao);
                }
            }
            TerrainVertices::Full(vertices) => {
                for vertex in vertices {
                    if vertex.pos.iter().any(|v| !v.is_finite()) {
                        return Err("render chunk has a non-finite vertex position".into());
                    }
                    inspect(vertex.face_ao);
                }
            }
        }
        if scanned_calibration != self.has_calibration_landmark {
            return Err("render chunk calibration flag does not match its vertices".into());
        }
        if scanned_feedback != self.has_fragment_feedback {
            return Err("render chunk fragment-feedback flag does not match its vertices".into());
        }
        if scanned_rare != self.has_rare_surface {
            return Err("render chunk rare-surface flag does not match its vertices".into());
        }
        if !self.ranges.is_empty() && (self.detail.is_some() || self.has_calibration_landmark) {
            return Err("render chunk has unsupported index ranges".into());
        }
        validate_index_ranges(&self.ranges, self.indices.len())?;
        if let Some(shadow) = &self.shadow {
            if self.has_calibration_landmark
                || self.detail.is_some()
                || shadow.indices.format() != self.indices.format()
                || shadow.indices.len() >= self.indices.len()
                || !shadow.indices.len().is_multiple_of(3)
                || !shadow.max_deviation_m.is_finite()
                || shadow.max_deviation_m < 0.0
            {
                return Err("render chunk has invalid shadow indices".into());
            }
            for index in 0..shadow.indices.len() {
                if shadow.indices.index(index).unwrap() as usize >= self.vertices.len() {
                    return Err("render chunk shadow index is out of bounds".into());
                }
            }
            validate_index_ranges(&shadow.ranges, shadow.indices.len())?;
        }
        Ok(())
    }
}

/// Exact GPU layout used by the night-light vertex pipeline. Workers expand
/// descriptors into these bytes so upload never rebuilds billboard vertices.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct PackedNightLightVertex {
    pub center: [f32; 3],
    pub energy: f32,
    pub corner: [f32; 2],
    pub color: [u8; 4],
    pub bucket: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PackedNightLights {
    pub vertices: Vec<PackedNightLightVertex>,
    pub bounds: RenderBounds,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackedFloraBounds {
    pub center_ecef: [f64; 3],
    pub radius_m: f64,
    pub local_y_min: f64,
    pub local_y_max: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PackedRenderCell {
    pub cell: CellId,
    pub complete: bool,
    pub chunks: Vec<PackedRenderChunk>,
    pub night_lights: Option<PackedNightLights>,
    pub tree_instances: Vec<TreeInstance>,
    pub solid_collision: SurfaceCollision,
    pub water: Vec<vearth_gen::water_surface::WaterTriangle>,
    pub flora: Vec<FloraCell>,
    pub height_range: (f32, f32),
    pub terrain_step_m: f32,
    pub terrain_relief_m: f32,
    pub terrain_detail: Option<std::sync::Arc<vearth_gen::TerrainDetailSummary>>,
    pub terrain_collision: Option<vearth_gen::TerrainCollision>,
    pub road_guides: Vec<RoadGuide>,
    pub flora_bounds: Option<PackedFloraBounds>,
}

/// Worker-to-renderer handoff for the settled packed terrain layout.
pub type PreparedRenderCell = PackedRenderCell;

impl PackedRenderCell {
    pub fn cell(&self) -> CellId {
        self.cell
    }

    pub fn height_range(&self) -> (f32, f32) {
        self.height_range
    }

    pub fn terrain_step_m(&self) -> f32 {
        self.terrain_step_m
    }

    pub fn terrain_relief_m(&self) -> f32 {
        self.terrain_relief_m
    }

    pub fn terrain_detail(&self) -> Option<&std::sync::Arc<vearth_gen::TerrainDetailSummary>> {
        self.terrain_detail.as_ref()
    }

    pub fn take_terrain_collision(&mut self) -> Option<vearth_gen::TerrainCollision> {
        self.terrain_collision.take()
    }

    pub fn take_solid_collision(&mut self) -> SurfaceCollision {
        std::mem::take(&mut self.solid_collision)
    }

    pub fn take_water(&mut self) -> Vec<vearth_gen::water_surface::WaterTriangle> {
        std::mem::take(&mut self.water)
    }

    pub fn take_road_guides(&mut self) -> Vec<RoadGuide> {
        std::mem::take(&mut self.road_guides)
    }

    pub fn gpu_bytes(&self) -> usize {
        self.chunks
            .iter()
            .map(|chunk| chunk.vertex_bytes().len() + chunk.combined_index_bytes())
            .sum::<usize>()
            + self.night_lights.as_ref().map_or(0, |lights| {
                lights.vertices.len() * size_of::<PackedNightLightVertex>()
            })
    }

    pub fn owned_bytes(&self) -> usize {
        self.gpu_bytes()
            + self
                .terrain_detail
                .as_ref()
                .map_or(0, |detail| detail.heap_bytes())
            + self.chunks.len() * size_of::<PackedRenderChunk>()
            + self
                .chunks
                .iter()
                .map(|chunk| chunk.ranges.capacity() * size_of::<RenderIndexRange>())
                .sum::<usize>()
            + self
                .chunks
                .iter()
                .filter_map(|chunk| chunk.shadow.as_ref())
                .map(|shadow| shadow.ranges.capacity() * size_of::<RenderIndexRange>())
                .sum::<usize>()
            + self.tree_instances.len() * size_of::<TreeInstance>()
            + self.solid_collision.heap_bytes()
            + std::mem::size_of_val(self.water.as_slice())
            + self.flora.len() * size_of::<FloraCell>()
            + self.road_guides.len() * size_of::<RoadGuide>()
            + self
                .terrain_collision
                .as_ref()
                .map_or(0, |surface| surface.heap_bytes())
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.height_range.0.is_finite()
            || !self.height_range.1.is_finite()
            || self.height_range.0 > self.height_range.1
            || !self.terrain_step_m.is_finite()
            || self.terrain_step_m < 0.0
            || !self.terrain_relief_m.is_finite()
            || self.terrain_relief_m < 0.0
        {
            return Err("prepared render cell has invalid terrain metadata".into());
        }
        self.solid_collision.validate()?;
        if self.water.iter().any(|triangle| {
            !triangle.vertices.iter().flatten().all(|v| v.is_finite())
                || !triangle.depth_m.is_finite()
                || triangle.depth_m < 0.0
        }) {
            return Err("invalid continuous water metadata".into());
        }
        if let Some(surface) = &self.terrain_collision {
            surface.validate()?;
        }
        for chunk in &self.chunks {
            if chunk.addr.cell != self.cell() {
                return Err("prepared render cell contains a chunk for another cell".into());
            }
            chunk.validate()?;
        }
        if let Some(lights) = &self.night_lights {
            if lights.vertices.len() % 6 != 0 {
                return Err("night-light vertices are not billboard aligned".into());
            }
            if !lights.bounds.radius_m.is_finite() || lights.bounds.radius_m < 0.0 {
                return Err("night-light bounds are invalid".into());
            }
        }
        if let Some(bounds) = self.flora_bounds {
            if !bounds.radius_m.is_finite()
                || bounds.radius_m < 0.0
                || !bounds.local_y_min.is_finite()
                || !bounds.local_y_max.is_finite()
                || bounds.local_y_min > bounds.local_y_max
            {
                return Err("flora bounds are invalid".into());
            }
        } else if !self.flora.is_empty() {
            return Err("flora descriptors are missing their bounds".into());
        }
        Ok(())
    }

    /// Compute the offline contract identity on demand. Rendering and upload
    /// use validated geometry directly and do not need this whole-cell hash.
    pub fn compute_semantic_fingerprint(&self) -> Fingerprint {
        let mut hasher = SemanticHasher::new("vearth.prepared-render.surface.v1");
        let cell = self.cell();
        hasher.u8(cell.face.0);
        hasher.u8(cell.level);
        hasher.u32(cell.i);
        hasher.u32(cell.j);
        hasher.bool(self.complete);
        hasher.f32(self.height_range.0);
        hasher.f32(self.height_range.1);
        hasher.f32(self.terrain_step_m);
        hasher.f32(self.terrain_relief_m);
        hasher.bool(self.terrain_detail.is_some());
        if let Some(detail) = &self.terrain_detail {
            hasher.fingerprint(detail.fingerprint());
        }
        vearth_gen::collision::hash_collision(&mut hasher, &self.solid_collision);
        hasher.u64(self.water.len() as u64);
        for triangle in &self.water {
            for value in triangle.vertices.into_iter().flatten() {
                hasher.f32(value);
            }
            hasher.u8(triangle.kind as u8);
            hasher.f32(triangle.depth_m);
        }
        hasher.bool(self.terrain_collision.is_some());
        if let Some(surface) = &self.terrain_collision {
            for &height in surface.heights_local.iter() {
                hasher.f32(height);
            }
            for mask in surface.source_columns {
                hasher.u32(mask);
            }
        }
        let mut chunks: Vec<_> = self.chunks.iter().collect();
        chunks.sort_unstable_by_key(|chunk| chunk.addr.k);
        hasher.u64(chunks.len() as u64);
        for chunk in chunks {
            hasher.i32(chunk.addr.k);
            hasher.u8(chunk.vertices.format() as u8);
            hasher.bytes(chunk.vertex_bytes());
            hasher.u8(chunk.indices.format() as u8);
            hasher.bytes(chunk.index_bytes());
            hasher.bool(chunk.has_calibration_landmark);
            hasher.bool(chunk.has_fragment_feedback);
            hasher.bool(chunk.has_rare_surface);
            vearth_gen::built_surface::hash_detail(&mut hasher, chunk.detail);
            for value in chunk.bounds.center_ecef {
                hasher.f64(value);
            }
            hasher.f64(chunk.bounds.radius_m);
            for value in chunk.local_bounds.half_extents {
                hasher.f64(value);
            }
            hasher.f64(chunk.local_bounds.padding_m);
            hasher.u64(chunk.ranges.len() as u64);
            for range in &chunk.ranges {
                hasher.u32(range.first_index);
                hasher.u32(range.index_count);
                for value in range.bounds.center_ecef {
                    hasher.f64(value);
                }
                hasher.f64(range.bounds.radius_m);
            }
            hasher.bool(chunk.shadow.is_some());
            if let Some(shadow) = &chunk.shadow {
                hasher.bytes(shadow.indices.bytes());
                hasher.f32(shadow.max_deviation_m);
                hasher.u64(shadow.ranges.len() as u64);
                for range in &shadow.ranges {
                    hasher.u32(range.first_index);
                    hasher.u32(range.index_count);
                    for value in range.bounds.center_ecef {
                        hasher.f64(value);
                    }
                    hasher.f64(range.bounds.radius_m);
                }
            }
        }
        hasher.u64(self.road_guides.len() as u64);
        for guide in &self.road_guides {
            for value in guide
                .a_local
                .into_iter()
                .chain(guide.b_local)
                .chain(guide.a_exit)
                .chain(guide.b_exit)
                .chain([guide.width_m, guide.assist])
            {
                hasher.f32(value);
            }
        }
        hasher.u64(self.tree_instances.len() as u64);
        for tree in &self.tree_instances {
            hasher.u8(tree.profile as u8);
            hasher.u64(tree.seed);
            hasher.f64(tree.lat_rad);
            hasher.f64(tree.lon_rad);
            for value in [
                tree.elevation_m,
                tree.height_m,
                tree.yaw_rad,
                tree.cell_x,
                tree.cell_z,
            ] {
                hasher.f32(value);
            }
        }
        hasher.u64(self.flora.len() as u64);
        for flora in &self.flora {
            hasher.u8(flora.archetype as u8);
            hasher.u8(flora.calendar as u8);
            hasher.u64(flora.field_seed);
            hasher.f32(flora.row_heading_rad);
            hasher.u8(flora.palette);
            hasher.u8(flora.base_density);
            for word in flora.coverage {
                hasher.u32(word);
            }
            hasher.i32(flora.ground_base_cm);
            for value in flora.ground_cm {
                hasher.bytes(&value.to_le_bytes());
            }
            hasher.u8(flora.phenology_offset);
            hasher.u8(flora.curated_style);
        }
        if let Some(lights) = &self.night_lights {
            hasher.bool(true);
            hasher.bytes(bytemuck::cast_slice(&lights.vertices));
            for value in lights.bounds.center_ecef {
                hasher.f64(value);
            }
            hasher.f64(lights.bounds.radius_m);
        } else {
            hasher.bool(false);
        }
        if let Some(bounds) = self.flora_bounds {
            hasher.bool(true);
            for value in bounds.center_ecef {
                hasher.f64(value);
            }
            hasher.f64(bounds.radius_m);
            hasher.f64(bounds.local_y_min);
            hasher.f64(bounds.local_y_max);
        } else {
            hasher.bool(false);
        }
        hasher.finish()
    }
}

pub fn pack_surface_cell(meshed: SurfaceCell) -> Result<PackedRenderCell, String> {
    pack_surface_cell_with(meshed, |_| {})
}

/// Admit the source product to storage only after all fallible validation,
/// while retaining ownership for packing without cloning the mesh payload.
pub(crate) fn pack_surface_cell_with(
    mut meshed: SurfaceCell,
    on_validated: impl FnOnce(&SurfaceCell),
) -> Result<PackedRenderCell, String> {
    let cell = meshed.cell;
    if cell.face.0 >= 6
        || cell.level > 30
        || cell.i >= 1 << cell.level
        || cell.j >= 1 << cell.level
        || !meshed.height_range.0.is_finite()
        || !meshed.height_range.1.is_finite()
        || meshed.height_range.0 > meshed.height_range.1
        || !meshed.terrain_step_m.is_finite()
        || meshed.terrain_step_m < 0.0
        || !meshed.terrain_relief_m.is_finite()
        || meshed.terrain_relief_m < 0.0
    {
        return Err("surface cell has invalid address or height metadata".into());
    }
    let terrain_detail = meshed
        .terrain_detail
        .as_ref()
        .map(|grid| {
            grid.validate()?;
            vearth_gen::TerrainDetailSummary::from_grid(grid)
                .map(std::sync::Arc::new)
                .ok_or_else(|| "surface cell has invalid spatial terrain metadata".to_owned())
        })
        .transpose()?;
    meshed
        .solid_collision
        .limit_complexity(&mut meshed.terrain_collision);
    meshed.solid_collision.validate()?;
    if let Some(terrain) = &meshed.terrain_collision {
        terrain.validate()?;
    }
    if meshed.water.iter().any(|t| {
        t.vertices.iter().flatten().any(|v| !v.is_finite())
            || !t.depth_m.is_finite()
            || t.depth_m < 0.0
    }) {
        return Err("surface cell has invalid water metadata".into());
    }
    let mut batch_metadata = Vec::with_capacity(meshed.batches.len());
    for chunk in &mut meshed.batches {
        if !chunk.validate_shadow_omissions() {
            return Err("surface batch has invalid shadow omissions".into());
        }
        if chunk.addr.cell != cell
            || chunk.addr.k.checked_mul(32).is_none()
            || chunk.mesh.indices.len() % 3 != 0
        {
            return Err("surface batch has an invalid address or triangle index".into());
        }
        if chunk.detail.is_some_and(|d| {
            d.center.iter().any(|v| !v.is_finite())
                || !d.radius_m.is_finite()
                || d.radius_m <= 0.
                || !d.min_screen_px.is_finite()
                || d.min_screen_px <= 0.
        }) {
            return Err("surface batch has invalid detail bounds".into());
        }
        let mut indices_u16 = true;
        for &index in &chunk.mesh.indices {
            if index as usize >= chunk.mesh.vertices.len() {
                return Err("surface batch has an invalid address or triangle index".into());
            }
            indices_u16 &= u16::try_from(index).is_ok();
        }
        let MeshBounds {
            bounds,
            local_bounds,
            has_calibration_landmark,
            has_fragment_feedback,
            has_rare_surface,
        } = mesh_bounds(&ChunkFrame::of(chunk.addr), &chunk.mesh.vertices)?;
        if has_repeated_surface_vertices(&chunk.mesh.vertices) {
            // Surface emitters retain independent triangle corners. Share only
            // identical values before storage/upload; this changes neither
            // primitive order nor positions, material seams or hard normals.
            vearth_gen::built_surface::compact_mesh_preserving_vertices(&mut chunk.mesh);
            indices_u16 = chunk.mesh.vertices.len() <= usize::from(u16::MAX) + 1
                || chunk
                    .mesh
                    .indices
                    .iter()
                    .all(|&index| u16::try_from(index).is_ok());
        }
        batch_metadata.push((
            MeshBounds {
                bounds,
                local_bounds,
                has_calibration_landmark,
                has_fragment_feedback,
                has_rare_surface,
            },
            indices_u16,
        ));
    }
    on_validated(&meshed);
    let mut chunks = Vec::with_capacity(meshed.batches.len());
    for (mut chunk, (mesh_bounds, indices_u16)) in meshed.batches.into_iter().zip(batch_metadata) {
        let MeshBounds {
            bounds,
            local_bounds,
            has_calibration_landmark,
            has_fragment_feedback,
            has_rare_surface,
        } = mesh_bounds;
        let shadow = if chunk.shadow_omissions.is_empty()
            || chunk.detail.is_some()
            || has_calibration_landmark
        {
            None
        } else {
            let omitted = chunk
                .shadow_omissions
                .iter()
                .map(|span| span.index_count as usize)
                .sum::<usize>();
            let mut shadow_indices = Vec::with_capacity(chunk.mesh.indices.len() - omitted);
            let mut previous_end = 0;
            let mut max_deviation_m = 0.0f32;
            for span in &chunk.shadow_omissions {
                shadow_indices.extend_from_slice(
                    &chunk.mesh.indices[previous_end..span.first_index as usize],
                );
                previous_end = span.first_index as usize + span.index_count as usize;
                max_deviation_m = max_deviation_m.max(span.max_deviation_m);
            }
            shadow_indices.extend_from_slice(&chunk.mesh.indices[previous_end..]);
            let ranges = index_ranges::partition_indices(
                &ChunkFrame::of(chunk.addr),
                &chunk.mesh.vertices,
                &mut shadow_indices,
            );
            Some(PackedShadowIndices {
                indices: pack_indices(shadow_indices, indices_u16),
                ranges,
                max_deviation_m,
            })
        };
        let ranges = if chunk.detail.is_none()
            && !has_calibration_landmark
            && index_ranges::eligible(&chunk.mesh)
        {
            index_ranges::partition(&ChunkFrame::of(chunk.addr), &mut chunk.mesh)
        } else {
            Vec::new()
        };
        let vertices = match pack_surface_vertices(&chunk.mesh.vertices) {
            Some(vertices) => TerrainVertices::Packed(vertices),
            None => TerrainVertices::Full(chunk.mesh.vertices),
        };
        let indices = pack_indices(chunk.mesh.indices, indices_u16);
        chunks.push(PackedRenderChunk {
            detail: chunk.detail,
            addr: chunk.addr,
            vertices,
            indices,
            bounds,
            local_bounds,
            has_calibration_landmark,
            has_fragment_feedback,
            has_rare_surface,
            ranges,
            shadow,
        });
    }

    let night_lights = pack_night_lights(meshed.cell, &meshed.night_lights);
    let flora_bounds = pack_flora_bounds(meshed.cell, meshed.terrain_step_m, &meshed.flora);
    let prepared = PackedRenderCell {
        cell,
        complete: meshed.complete,
        chunks,
        night_lights,
        tree_instances: meshed.tree_instances,
        solid_collision: meshed.solid_collision,
        water: meshed.water,
        flora: meshed.flora,
        height_range: meshed.height_range,
        terrain_step_m: meshed.terrain_step_m,
        terrain_relief_m: meshed.terrain_relief_m,
        terrain_detail,
        terrain_collision: meshed.terrain_collision,
        road_guides: meshed.road_guides,
        flora_bounds,
    };
    Ok(prepared)
}

fn has_repeated_surface_vertices(vertices: &[SurfaceVertex]) -> bool {
    if vertices.len() < 4_096 {
        return false;
    }
    // Cached/indexed meshes normally have no duplicate vertex records. A
    // bounded prefix avoids hashing their entire payload again on every load.
    // Missing duplicates only skips an optional lossless memory optimization.
    let mut seen = std::collections::HashSet::with_capacity(128);
    vertices
        .iter()
        .take(128)
        .any(|vertex| !seen.insert((vertex.pos.map(f32::to_bits), vertex.face_ao, vertex.rgba)))
}

// Match surface_vertex's calibration-tag removal before fs_main decodes
// the flat material ID. Atlas layers and colour/alpha do not prove opacity.
fn material_has_fragment_feedback(face_ao: u32) -> bool {
    let material = ((face_ao & !CALIBRATION_TAG) >> 5) & 255;
    material == 13 || (22..=65).contains(&material)
}

// Materials behind the shader's RARE_SURFACES override: landmark lamps and
// sparkle fixtures, sculpture metal and highrise facade systems.
fn material_has_rare_surface(face_ao: u32) -> bool {
    let material = ((face_ao & !CALIBRATION_TAG) >> 5) & 255;
    matches!(material, 79 | 80 | 95 | 103..=118)
}

struct MeshBounds {
    bounds: RenderBounds,
    local_bounds: RenderLocalBox,
    has_calibration_landmark: bool,
    has_fragment_feedback: bool,
    has_rare_surface: bool,
}

fn mesh_bounds(frame: &ChunkFrame, vertices: &[SurfaceVertex]) -> Result<MeshBounds, String> {
    let mut lo = glam::DVec3::splat(f64::INFINITY);
    let mut hi = glam::DVec3::splat(f64::NEG_INFINITY);
    let mut has_calibration_landmark = false;
    let mut has_fragment_feedback = false;
    let mut has_rare_surface = false;
    for vertex in vertices {
        let local = glam::DVec3::from_array(vertex.pos.map(f64::from));
        if !local.is_finite() {
            return Err("render chunk has a non-finite vertex position".into());
        }
        lo = lo.min(local);
        hi = hi.max(local);
        has_calibration_landmark |= vertex.face_ao & CALIBRATION_TAG != 0;
        has_fragment_feedback |= material_has_fragment_feedback(vertex.face_ao);
        has_rare_surface |= material_has_rare_surface(vertex.face_ao);
    }
    let (center, radius, local_bounds) = if vertices.is_empty() {
        (
            frame.origin,
            0.0,
            RenderLocalBox {
                half_extents: [0.0; 3],
                padding_m: 0.0,
            },
        )
    } else {
        let center = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5;
        // Both world and shadow vertices use an affine model without
        // displacement. Include actual skirts and out-of-cell geometry;
        // animated instances have independently prepared bounds. The skewed
        // cube-face basis requires the transformed box's corner radius.
        let magnitude = (center.abs() + half) * frame.grid_scale.abs();
        let margin = 0.01 + magnitude.element_sum() * f64::from(f32::EPSILON) * 8.0;
        (
            frame.local_to_ecef(center),
            frame.local_box_radius(half) + margin,
            RenderLocalBox {
                half_extents: half.to_array(),
                padding_m: margin,
            },
        )
    };
    Ok(MeshBounds {
        bounds: RenderBounds {
            center_ecef: center.to_array(),
            radius_m: radius,
        },
        local_bounds,
        has_calibration_landmark,
        has_fragment_feedback,
        has_rare_surface,
    })
}

pub fn prepare_surface_cell(meshed: SurfaceCell) -> Result<PreparedRenderCell, String> {
    pack_surface_cell(meshed)
}

fn pack_night_lights(cell: CellId, clusters: &[NightLightCluster]) -> Option<PackedNightLights> {
    if clusters.is_empty() {
        return None;
    }
    const CORNERS: [[f32; 2]; 6] = [
        [-1.0, -1.0],
        [1.0, -1.0],
        [1.0, 1.0],
        [-1.0, -1.0],
        [1.0, 1.0],
        [-1.0, 1.0],
    ];
    let mut vertices = Vec::with_capacity(clusters.len() * CORNERS.len());
    for light in clusters {
        for corner in CORNERS {
            vertices.push(PackedNightLightVertex {
                center: light.local_pos,
                energy: light.energy,
                corner,
                color: [light.color[0], light.color[1], light.color[2], 255],
                bucket: u32::from(light.bucket),
            });
        }
    }
    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let max_y = clusters
        .iter()
        .map(|light| f64::from(light.local_pos[1]))
        .fold(0.0, f64::max);
    let center = frame.local_to_ecef(glam::DVec3::new(16.0, max_y * 0.5, 16.0));
    Some(PackedNightLights {
        vertices,
        bounds: RenderBounds {
            center_ecef: center.to_array(),
            radius_m: frame
                .local_box_radius(glam::DVec3::new(16.0, max_y * 0.5, 16.0))
                .max(20.0),
        },
    })
}

fn pack_flora_bounds(
    cell: CellId,
    _terrain_step_m: f32,
    flora: &[FloraCell],
) -> Option<PackedFloraBounds> {
    if flora.is_empty() {
        return None;
    }
    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let (ground_min_m, ground_max_m) = flora
        .iter()
        .flat_map(|descriptor| {
            descriptor
                .ground_cm
                .iter()
                .map(move |offset| (descriptor.ground_base_cm + i32::from(*offset)) as f64 * 0.01)
        })
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), value| {
            (lo.min(value), hi.max(value))
        });
    let bottom_y = ground_min_m / frame.grid_scale.y - 0.2;
    let top_y = (ground_max_m + 4.0) / frame.grid_scale.y;
    let center_y = (bottom_y + top_y) * 0.5;
    let center = frame.local_to_ecef(glam::DVec3::new(16.0, center_y, 16.0));
    Some(PackedFloraBounds {
        center_ecef: center.to_array(),
        radius_m: frame.local_box_radius(glam::DVec3::new(16.0, (top_y - bottom_y) * 0.5, 16.0)),
        local_y_min: bottom_y,
        local_y_max: top_y,
    })
}

#[cfg(test)]
pub(crate) fn test_surface(cell: CellId) -> SurfaceCell {
    use vearth_gen::collision::{Capsule, CollisionTriangle};
    use vearth_gen::{SurfaceBatch, TerrainCollision};
    let points = [
        [2.125, 4.25, 2.125],
        [6.375, 4.25, 2.125],
        [2.125, 4.25, 6.375],
    ];
    let vertices = points
        .map(|pos| SurfaceVertex {
            pos,
            face_ao: 2 | (3 << 3) | (4 << 5),
            rgba: [80, 90, 100, 255],
        })
        .to_vec();
    SurfaceCell {
        cell,
        batches: vec![SurfaceBatch {
            shadow_omissions: Vec::new(),
            addr: ChunkAddr { cell, k: 0 },
            mesh: MeshBuffers {
                vertices,
                indices: vec![0, 2, 1],
            },
            detail: None,
        }],
        height_range: (0.0, 20.0),
        terrain_step_m: 1.0,
        terrain_relief_m: 12.25,
        terrain_detail: Some(vearth_gen::TerrainDetailGrid {
            heights_m: Box::new([0.0; 33 * 33]),
            source_columns: [u32::MAX; 32],
        }),
        terrain_collision: Some(TerrainCollision {
            heights_local: Box::new([0.0; 33 * 33]),
            source_columns: [u32::MAX; 32],
        }),
        solid_collision: SurfaceCollision::from_parts(
            vec![CollisionTriangle { vertices: points }],
            vec![Capsule {
                a: [8.0, 0.0, 8.0],
                b: [8.0, 6.0, 8.0],
                radius_a: 0.25,
                radius_b: 0.15,
            }],
        ),
        water: vec![vearth_gen::water_surface::WaterTriangle {
            vertices: points,
            kind: vearth_data::vectors::WaterRegionKind::Lake,
            depth_m: 2.5,
        }],
        road_guides: Vec::new(),
        night_lights: Vec::new(),
        tree_instances: Vec::new(),
        flora: Vec::new(),
        complete: true,
    }
}

#[cfg(test)]
pub(crate) fn test_generated_water(kind: vearth_data::vectors::WaterRegionKind) -> SurfaceCell {
    use vearth_data::{
        FnElevation,
        vectors::{FeaturePack, VectorSource, Water, WaterRegionKind},
    };
    struct Lake;
    impl VectorSource for Lake {
        fn features(&self, _lat: f64, _lon: f64, _east: f64, _north: f64) -> FeaturePack {
            let mut pack = FeaturePack::empty();
            let mut water = Water::inland(vec![vec![
                [-1024., -1024.],
                [1024., -1024.],
                [1024., 1024.],
                [-1024., 1024.],
                [-1024., -1024.],
            ]]);
            water.kind = WaterRegionKind::Lake;
            pack.water.push(water);
            pack
        }
    }
    let cell = CellId {
        face: vearth_core::cubesphere::Face(0),
        level: 17,
        i: 1 << 16,
        j: 1 << 16,
    };
    let source = FnElevation(move |_: f64, _: f64| {
        if kind == WaterRegionKind::Ocean {
            -20.0
        } else {
            40.0
        }
    });
    let lake = Lake;
    let mut sources = vearth_gen::Sources::terrain_only(&source);
    if kind == WaterRegionKind::Lake {
        sources.vectors = Some(&lake);
    }
    let surface = vearth_gen::generate_surface_cell(cell, &sources);
    assert!(!surface.water.is_empty());
    assert!(surface.water.iter().all(|water| water.kind == kind));
    surface
}

#[cfg(test)]
pub(crate) fn test_water_contact_top(surface: &SurfaceCell) -> f64 {
    let frame = ChunkFrame::of(ChunkAddr {
        cell: surface.cell,
        k: 0,
    });
    let metric = vearth_gen::collision::MetricFrame::of(&frame);
    let triangles = surface
        .water
        .iter()
        .map(|water| vearth_gen::collision::CollisionTriangle {
            vertices: water.vertices.map(|point| metric.point_from_local(point)),
        })
        .collect();
    SurfaceCollision::from_triangles(triangles)
        .top(0., 0.)
        .expect("water covers fixture centre")
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::cubesphere::Face;
    fn cell() -> CellId {
        CellId {
            face: Face(0),
            level: 17,
            i: 65536,
            j: 65536,
        }
    }

    #[test]
    fn large_range_packing_is_transient_and_preserves_whole_detail_and_calibration() {
        let mut source = test_surface(cell());
        let original = source.batches[0].mesh.clone();
        let mesh = &mut source.batches[0].mesh;
        mesh.vertices.clear();
        mesh.indices.clear();
        for i in 0..32_768 {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend(original.vertices.iter().map(|vertex| {
                let mut vertex = *vertex;
                vertex.pos[0] += if i & 1 == 0 { -50.0 } else { 50.0 };
                vertex.pos[2] += if i & 2 == 0 { -50.0 } else { 50.0 };
                vertex
            }));
            mesh.indices
                .extend(original.indices.iter().map(|&id| id + base));
        }
        source.batches[0]
            .shadow_omissions
            .push(vearth_gen::ShadowOmission {
                first_index: 3,
                index_count: 9,
                max_deviation_m: 0.025,
            });
        let mut cached = None;
        let cold =
            pack_surface_cell_with(source.clone(), |validated| cached = Some(validated.clone()))
                .unwrap();
        let cached = cached.unwrap();
        let warm = pack_surface_cell(cached.clone()).unwrap();
        cold.validate().unwrap();
        assert_eq!(cold, warm);
        assert_eq!(
            cold.compute_semantic_fingerprint(),
            warm.compute_semantic_fingerprint()
        );
        assert_eq!(cold.chunks.len(), source.batches.len());
        assert_eq!(cold.chunks[0].ranges.len(), 4);
        let shadow = cold.chunks[0].shadow.as_ref().unwrap();
        assert_eq!(shadow.max_deviation_m, 0.025);
        assert_eq!(shadow.indices.len() + 9, cold.chunks[0].indices.len());
        assert_eq!(shadow.indices.format(), cold.chunks[0].indices.format());
        let mut expected_shadow: Vec<_> = cached.batches[0]
            .mesh
            .indices
            .chunks_exact(3)
            .enumerate()
            .filter(|(index, _)| !(1..4).contains(index))
            .map(|(_, triangle)| [triangle[0], triangle[1], triangle[2]])
            .collect();
        let shadow_indices = shadow.indices.to_u32_vec();
        let mut actual_shadow: Vec<_> = shadow_indices
            .chunks_exact(3)
            .map(|triangle| [triangle[0], triangle[1], triangle[2]])
            .collect();
        expected_shadow.sort_unstable();
        actual_shadow.sort_unstable();
        assert_eq!(actual_shadow, expected_shadow);
        assert_eq!(
            cold.chunks[0].vertices.to_full_vec(),
            cached.batches[0].mesh.vertices
        );
        assert_eq!(cold.solid_collision, cached.solid_collision);
        assert_eq!(cold.terrain_collision, cached.terrain_collision);
        assert_eq!(cold.water, cached.water);
        let parent_bounds = mesh_bounds(
            &ChunkFrame::of(cached.batches[0].addr),
            &cached.batches[0].mesh.vertices,
        )
        .unwrap()
        .bounds;
        assert_eq!(cold.chunks[0].bounds, parent_bounds);
        let mut detail = source.clone();
        detail.batches[0].detail = Some(vearth_gen::built_surface::BuiltDetailBounds {
            center: [0.0, 0.0, 0.0],
            radius_m: 10.0,
            min_screen_px: 72.0,
        });
        let detail = pack_surface_cell(detail).unwrap();
        assert!(detail.chunks[0].ranges.is_empty());
        assert!(detail.chunks[0].shadow.is_none());
        source.batches[0].mesh.vertices[0].face_ao |= CALIBRATION_TAG;
        let calibration = pack_surface_cell(source).unwrap();
        assert!(calibration.chunks[0].ranges.is_empty());
        assert!(calibration.chunks[0].has_calibration_landmark);
        assert!(calibration.chunks[0].shadow.is_none());

        for invalid in 0..3 {
            let mut chunk = cold.chunks[0].clone();
            match invalid {
                0 => chunk.ranges[0].first_index = 3,
                1 => chunk.ranges[0].index_count -= 3,
                _ => chunk.ranges[0].bounds.radius_m = f64::NAN,
            }
            assert!(chunk.validate().is_err());
        }
    }

    #[test]
    fn dense_surface_packing_shares_vertices_without_changing_drawn_triangles() {
        let mut source = test_surface(cell());
        let original = source.batches[0].mesh.clone();
        let mesh = &mut source.batches[0].mesh;
        mesh.vertices.clear();
        mesh.indices.clear();
        for _ in 0..1_500 {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend_from_slice(&original.vertices);
            mesh.indices
                .extend(original.indices.iter().map(|&i| i + base));
        }
        let expanded = |mesh: &vearth_core::mesh::MeshBuffers| {
            mesh.indices
                .iter()
                .map(|&index| mesh.vertices[index as usize])
                .collect::<Vec<_>>()
        };
        let source_triangles = expanded(mesh);
        let old_vertices = mesh.vertices.len();
        let packed = pack_surface_cell_with(source, |validated| {
            assert!(validated.batches[0].mesh.vertices.len() < old_vertices / 100);
            assert_eq!(source_triangles, expanded(&validated.batches[0].mesh));
        })
        .unwrap();
        let actual = vearth_core::mesh::MeshBuffers {
            vertices: packed.chunks[0].vertices.to_full_vec(),
            indices: packed.chunks[0].indices.to_u32_vec(),
        };
        assert_eq!(source_triangles, expanded(&actual));
        assert!(!has_repeated_surface_vertices(&actual.vertices));
        packed.validate().unwrap();
    }

    #[test]
    fn dense_compaction_preserves_unreferenced_calibration_and_cached_bounds() {
        let mut source = test_surface(cell());
        let original = source.batches[0].mesh.clone();
        let mesh = &mut source.batches[0].mesh;
        for _ in 0..1_500 {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend_from_slice(&original.vertices);
            mesh.indices
                .extend(original.indices.iter().map(|&i| i + base));
        }
        let mut unused = original.vertices[0];
        unused.pos = [28.0, 8.0, 28.0];
        unused.face_ao |= u32::from(vearth_gen::landmarks::CALIBRATION_MATERIAL_FLAG) << 5;
        mesh.vertices.push(unused);
        let mut cached = None;
        let cold =
            pack_surface_cell_with(source, |validated| cached = Some(validated.clone())).unwrap();
        let warm = pack_surface_cell(cached.unwrap()).unwrap();
        cold.validate().unwrap();
        warm.validate().unwrap();
        assert!(cold.chunks[0].has_calibration_landmark);
        assert_eq!(
            cold.compute_semantic_fingerprint(),
            warm.compute_semantic_fingerprint()
        );
        assert!(cold.chunks[0].vertices.to_full_vec().contains(&unused));
    }

    #[test]
    fn surface_packing_preserves_geometry_collision_and_water() {
        let source = test_surface(cell());
        let mut packed = pack_surface_cell(source.clone()).unwrap();
        assert_eq!(
            packed.chunks[0].vertices.to_full_vec(),
            source.batches[0].mesh.vertices
        );
        assert_eq!(
            packed.chunks[0].indices.to_u32_vec(),
            source.batches[0].mesh.indices
        );
        assert_eq!(packed.solid_collision, source.solid_collision);
        assert_eq!(packed.terrain_collision, source.terrain_collision);
        assert_eq!(packed.water, source.water);
        assert!(packed.validate().is_ok());
        let bytes = packed.owned_bytes();
        let collision = packed.take_solid_collision();
        assert!(packed.owned_bytes() + collision.heap_bytes() <= bytes);
        assert_eq!(collision, source.solid_collision);
        assert_eq!(packed.take_water(), source.water);
    }

    #[test]
    fn oversized_optional_collision_preserves_render_admission_and_ground() {
        let mut source = test_surface(cell());
        let capsule = source.solid_collision.capsules()[0];
        source.solid_collision = SurfaceCollision::from_parts(
            Vec::new(),
            vec![capsule; vearth_gen::collision::MAX_COLLISION_CAPSULES + 1],
        );
        source
            .terrain_collision
            .as_mut()
            .unwrap()
            .source_columns
            .fill(0);
        let expected_mesh = source.batches[0].mesh.clone();
        let mut invalid = source.clone();
        invalid.batches[0].mesh.vertices[0].pos[0] = f32::NAN;
        assert!(pack_surface_cell(invalid).is_err());
        let packed = pack_surface_cell_with(source, |validated| {
            assert!(validated.solid_collision.is_empty());
            assert_eq!(
                validated
                    .terrain_collision
                    .as_ref()
                    .unwrap()
                    .sample_local(16.0, 16.0),
                Some(0.0),
            );
        })
        .unwrap();
        assert_eq!(
            packed.chunks[0].vertices.to_full_vec(),
            expected_mesh.vertices
        );
        assert_eq!(packed.chunks[0].indices.to_u32_vec(), expected_mesh.indices);
        assert!(packed.solid_collision.is_empty());
        assert!(packed.validate().is_ok());
    }
    #[test]
    fn full_precision_detail_and_metadata_are_fingerprinted() {
        let mut source = test_surface(cell());
        source.batches[0].mesh.vertices[0].pos[0] = 2.123456;
        source.batches[0].detail = Some(vearth_gen::built_surface::BuiltDetailBounds {
            center: [4.0, 4.25, 4.0],
            radius_m: 6.0,
            min_screen_px: 12.0,
        });
        let mut packed = pack_surface_cell(source.clone()).unwrap();
        assert_eq!(
            packed.chunks[0].vertices.format(),
            TerrainVertexFormat::Full20
        );
        assert_eq!(
            packed.chunks[0].vertices.to_full_vec(),
            source.batches[0].mesh.vertices
        );
        assert!(packed.validate().is_ok());
        let identity = packed.compute_semantic_fingerprint();
        packed.water[0].depth_m += 1.0;
        assert_ne!(identity, packed.compute_semantic_fingerprint());
        let identity = packed.compute_semantic_fingerprint();
        packed.chunks[0].detail.as_mut().unwrap().radius_m += 1.0;
        assert_ne!(identity, packed.compute_semantic_fingerprint());
    }

    #[test]
    fn terrain_detail_packing_shares_summary_without_changing_render_data() {
        let mut source = test_surface(cell());
        let flat = pack_surface_cell(source.clone()).unwrap();
        source.terrain_detail.as_mut().unwrap().heights_m[0] = 1_000.0;
        let mut rugged = pack_surface_cell(source.clone()).unwrap();
        assert_ne!(
            flat.compute_semantic_fingerprint(),
            rugged.compute_semantic_fingerprint()
        );
        let cloned = rugged.clone();
        assert!(std::sync::Arc::ptr_eq(
            rugged.terrain_detail().unwrap(),
            cloned.terrain_detail().unwrap()
        ));
        rugged.terrain_detail = flat.terrain_detail.clone();
        assert_eq!(
            rugged, flat,
            "spatial metadata must not alter any GPU or collision payload"
        );
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            source.terrain_detail.as_mut().unwrap().heights_m[0] = invalid;
            let mut admitted = false;
            assert!(pack_surface_cell_with(source.clone(), |_| admitted = true).is_err());
            assert!(
                !admitted,
                "invalid spatial metadata must not enter persistent storage"
            );
        }
    }

    #[test]
    fn terrain_relief_is_validated_and_fingerprinted_without_changing_render_data() {
        let mut source = test_surface(cell());
        source.height_range = (-77_000.0, 20.0);
        source.terrain_relief_m = 0.0;
        let flat = pack_surface_cell(source.clone()).unwrap();
        assert_eq!(flat.terrain_relief_m(), 0.0);
        for relief in [0.25, 12.25, f32::MAX] {
            source.terrain_relief_m = relief;
            let packed = pack_surface_cell(source.clone()).unwrap();
            packed.validate().unwrap();
            assert_eq!(packed.terrain_relief_m().to_bits(), relief.to_bits());
            assert_ne!(
                flat.compute_semantic_fingerprint(),
                packed.compute_semantic_fingerprint()
            );
            let mut same = packed;
            same.terrain_relief_m = flat.terrain_relief_m;
            assert_eq!(
                same, flat,
                "physical relief must not change buffers or bounds"
            );
        }
        for relief in [-0.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            source.terrain_relief_m = relief;
            assert!(
                pack_surface_cell_with(source.clone(), |_| {
                    panic!("invalid terrain relief reached the cache callback")
                })
                .is_err()
            );
            let mut packed = flat.clone();
            packed.terrain_relief_m = relief;
            assert!(packed.validate().unwrap_err().contains("terrain metadata"));
        }
    }
    fn legacy_padded_mesh_bounds(frame: &ChunkFrame, vertices: &[SurfaceVertex]) -> RenderBounds {
        let center = frame.local_to_ecef(glam::DVec3::splat(16.0));
        let minimum = frame.local_box_radius(glam::DVec3::new(16.0, 22.0, 16.0));
        let mut radius = minimum;
        for vertex in vertices {
            let local = glam::DVec3::from_array(vertex.pos.map(f64::from));
            let offset = (local - glam::DVec3::splat(16.0)) * frame.grid_scale;
            radius = radius.max(
                (frame.x_axis * offset.x + frame.y_axis * offset.y + frame.z_axis * offset.z)
                    .length(),
            );
        }
        if radius > minimum {
            radius += 0.01;
        }
        RenderBounds {
            center_ecef: center.to_array(),
            radius_m: radius,
        }
    }

    #[test]
    fn tight_mesh_bounds_enclose_surface_vertices_across_skewed_frames() {
        for face in 0..6 {
            for level in [0, 10, 17, 24, 30] {
                let last = (1 << level) - 1;
                for (i, j) in [(0, 0), (last, last), (last / 3, last / 2)] {
                    for k in [-5, 0, 8] {
                        let cell = CellId {
                            face: Face(face),
                            level,
                            i,
                            j,
                        };
                        let frame = ChunkFrame::of(ChunkAddr { cell, k });
                        let vertices: Vec<_> = (0..128)
                            .map(|i| SurfaceVertex {
                                pos: [
                                    ((i * 17 % 113) as f32 - 20.0) * 1.271,
                                    ((i * 13 % 127) as f32 - 32.0) * 0.391,
                                    ((i * 23 % 137) as f32 - 42.0) * 0.843,
                                ],
                                face_ao: if i % 17 == 3 { CALIBRATION_TAG } else { 0 },
                                rgba: [0; 4],
                            })
                            .collect();
                        let extreme: Vec<_> = [
                            [-1.0e7, 2.0e7, -3.0e7],
                            [4.0e7, -5.0e7, 6.0e7],
                            [1.0, -1.0, 0.0],
                        ]
                        .into_iter()
                        .map(|pos| SurfaceVertex {
                            pos,
                            face_ao: 0,
                            rgba: [0; 4],
                        })
                        .collect();
                        for vertices in [&[][..], &vertices[..1], &vertices[..], &extreme[..]] {
                            let MeshBounds {
                                bounds,
                                local_bounds,
                                has_calibration_landmark: calibration,
                                ..
                            } = mesh_bounds(&frame, vertices).unwrap();
                            let center = glam::DVec3::from_array(bounds.center_ecef);
                            assert!(center.is_finite() && bounds.radius_m.is_finite());
                            for vertex in vertices {
                                let world = frame.local_to_ecef(glam::DVec3::from_array(
                                    vertex.pos.map(f64::from),
                                ));
                                assert!(
                                    world.distance(center) <= bounds.radius_m,
                                    "{cell:?} k={k} {vertex:?} escaped {bounds:?}"
                                );
                                for normal in [
                                    glam::DVec3::X,
                                    glam::DVec3::Y,
                                    glam::DVec3::Z,
                                    glam::DVec3::new(1.0, -2.0, 3.0).normalize(),
                                ] {
                                    let support = [frame.x_axis, frame.y_axis, frame.z_axis]
                                        .into_iter()
                                        .zip(frame.grid_scale.to_array())
                                        .zip(local_bounds.half_extents)
                                        .map(|((axis, scale), half)| {
                                            normal.dot(axis * (scale * half)).abs()
                                        })
                                        .sum::<f64>()
                                        + local_bounds.padding_m;
                                    assert!(normal.dot(world - center).abs() <= support);
                                }
                            }
                            for triangle in vertices.chunks_exact(3) {
                                for weights in [[0.2, 0.3, 0.5], [0.0, 0.5, 0.5]] {
                                    let local = triangle.iter().zip(weights).fold(
                                        glam::DVec3::ZERO,
                                        |sum, (vertex, weight)| {
                                            sum + glam::DVec3::from_array(vertex.pos.map(f64::from))
                                                * weight
                                        },
                                    );
                                    assert!(
                                        frame.local_to_ecef(local).distance(center)
                                            <= bounds.radius_m
                                    );
                                }
                            }
                            assert_eq!(
                                calibration,
                                vertices.iter().any(|v| v.face_ao & CALIBRATION_TAG != 0)
                            );
                            if vertices.is_empty() {
                                assert_eq!(bounds.radius_m, 0.0);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn tight_mesh_bounds_preserve_packed_positions_and_remove_empty_vertical_padding() {
        for face in 0..6 {
            let cell = CellId {
                face: Face(face),
                level: 12,
                i: 3000,
                j: 400,
            };
            let frame = ChunkFrame::of(ChunkAddr { cell, k: -2 });
            let vertices: Vec<_> = [
                [0.0, -0.0625, 0.0],
                [32.0, 0.0625, 0.0],
                [32.0, 0.0625, 32.0],
                [0.0, -0.0625, 32.0],
            ]
            .into_iter()
            .map(|pos| SurfaceVertex {
                pos,
                face_ao: 2,
                rgba: [30, 40, 50, 255],
            })
            .collect();
            let packed = vearth_core::mesh::pack_surface_vertices(&vertices).unwrap();
            let decoded: Vec<_> = packed.into_iter().map(|v| v.to_full()).collect();
            assert_eq!(decoded, vertices);
            let bounds = mesh_bounds(&frame, &vertices).unwrap().bounds;
            assert_eq!(mesh_bounds(&frame, &decoded).unwrap().bounds, bounds);
            let previous = legacy_padded_mesh_bounds(&frame, &vertices);
            assert!(bounds.radius_m < previous.radius_m * 0.8);
            assert!(
                glam::DVec3::from_array(bounds.center_ecef)
                    .distance(glam::DVec3::from_array(previous.center_ecef))
                    > frame.grid_scale.y * 15.0
            );
        }
    }

    #[test]
    fn invalid_surface_mesh_and_bounds_are_rejected() {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for axis in 0..3 {
                let mut source = test_surface(cell());
                source.batches[0].mesh.vertices[0].pos[axis] = invalid;
                assert!(pack_surface_cell(source).is_err());
            }
        }
        let mut source = test_surface(cell());
        source.batches[0].mesh.indices[2] = 999;
        assert!(pack_surface_cell(source).is_err());
        let mut packed = pack_surface_cell(test_surface(cell())).unwrap();
        packed.chunks[0].indices = TerrainIndices::U16(vec![0, 1, 999]);
        assert!(packed.validate().unwrap_err().contains("bounds"));
    }

    #[test]
    fn material_flags_match_shader_materials_in_both_vertex_formats() {
        for packed_vertices in [true, false] {
            for material in 0..256u32 {
                let mut source = test_surface(cell());
                for (index, vertex) in source.batches[0].mesh.vertices.iter_mut().enumerate() {
                    vertex.face_ao = 2 | (3 << 3) | (material << 5);
                    vertex.rgba[3] = if material & 1 == 0 { 0 } else { 255 };
                    if index == 2 {
                        vertex.face_ao = vearth_core::mesh::encode_surface_normal(
                            vertex.face_ao,
                            [0.1, 0.9, -0.2],
                        );
                        vertex.rgba[3] = vearth_core::mesh::TERRAIN_GEOMETRIC_NORMAL;
                    }
                    if !packed_vertices {
                        vertex.pos[1] += 0.123;
                    }
                }
                let expected_vertices = source.batches[0].mesh.vertices.clone();
                let packed = pack_surface_cell(source).unwrap();
                let chunk = &packed.chunks[0];
                let shader_material = ((material << 5) & !(128 << 5)) >> 5;
                let feedback = shader_material == 13 || (22..66).contains(&shader_material);
                assert_eq!(
                    chunk.has_fragment_feedback, feedback,
                    "source material {material}"
                );
                let rare = matches!(shader_material, 79 | 80 | 95 | 103..=118);
                assert_eq!(chunk.has_rare_surface, rare, "source material {material}");
                assert_eq!(chunk.has_calibration_landmark, material & 128 != 0);
                assert_eq!(
                    chunk.vertices.format() == TerrainVertexFormat::Packed12,
                    packed_vertices
                );
                assert_eq!(chunk.vertices.to_full_vec(), expected_vertices);
                packed.validate().unwrap();
                let mut invalid = packed.clone();
                invalid.chunks[0].has_fragment_feedback = !feedback;
                assert!(
                    invalid
                        .validate()
                        .unwrap_err()
                        .contains("fragment-feedback")
                );
                assert_ne!(
                    packed.compute_semantic_fingerprint(),
                    invalid.compute_semantic_fingerprint()
                );
                let mut invalid = packed.clone();
                invalid.chunks[0].has_rare_surface = !rare;
                assert!(invalid.validate().unwrap_err().contains("rare-surface"));
            }
        }
    }

    #[test]
    fn fragment_feedback_flag_retains_mixed_and_unreferenced_tree_materials() {
        let mut source = test_surface(cell());
        assert!(!pack_surface_cell(source.clone()).unwrap().chunks[0].has_fragment_feedback);
        let mut tree = source.batches[0].mesh.vertices[0];
        tree.face_ao = 2 | (13 << 5);
        // Classification covers the entire immutable payload, including a
        // vertex outside this batch's current index stream.
        source.batches[0].mesh.vertices.push(tree);
        let packed = pack_surface_cell(source.clone()).unwrap();
        assert!(packed.chunks[0].has_fragment_feedback);
        assert_eq!(packed.chunks[0].vertices.len(), 4);
        assert_eq!(
            packed.chunks[0].indices.to_u32_vec(),
            source.batches[0].mesh.indices
        );
        let mut missing = packed.clone();
        missing.chunks[0].has_fragment_feedback = false;
        assert!(missing.validate().is_err());
        source.batches[0].mesh.vertices.clear();
        source.batches[0].mesh.indices.clear();
        let empty = pack_surface_cell(source).unwrap();
        assert!(!empty.chunks[0].has_fragment_feedback);
        empty.validate().unwrap();
    }

    #[test]
    fn render_local_bounds_are_validated_and_fingerprinted() {
        let source = pack_surface_cell(test_surface(cell())).unwrap();
        assert!(source.validate().is_ok());
        for invalid in [f64::NAN, f64::INFINITY, -1.0] {
            let mut changed = source.clone();
            changed.chunks[0].local_bounds.padding_m = invalid;
            assert!(changed.validate().unwrap_err().contains("local bounds"));
            for axis in 0..3 {
                let mut changed = source.clone();
                changed.chunks[0].local_bounds.half_extents[axis] = invalid;
                assert!(changed.validate().unwrap_err().contains("local bounds"));
            }
        }
        let mut changed = source.clone();
        changed.chunks[0].local_bounds.half_extents[0] += 1.0;
        assert_ne!(
            source.compute_semantic_fingerprint(),
            changed.compute_semantic_fingerprint()
        );
        let mut changed = source.clone();
        changed.chunks[0].local_bounds.padding_m += 1.0;
        assert_ne!(
            source.compute_semantic_fingerprint(),
            changed.compute_semantic_fingerprint()
        );
    }
}

#[cfg(test)]
pub(crate) fn test_box_triangles(
    frame: &ChunkFrame,
    lo: [f32; 3],
    hi: [f32; 3],
) -> Vec<vearth_gen::collision::CollisionTriangle> {
    let metric = vearth_gen::collision::MetricFrame::of(frame);
    let points = [
        [lo[0], lo[1], lo[2]],
        [hi[0], lo[1], lo[2]],
        [hi[0], hi[1], lo[2]],
        [lo[0], hi[1], lo[2]],
        [lo[0], lo[1], hi[2]],
        [hi[0], lo[1], hi[2]],
        [hi[0], hi[1], hi[2]],
        [lo[0], hi[1], hi[2]],
    ]
    .map(|p| metric.point_from_local(p));
    [
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [3, 7, 6],
        [3, 6, 2],
        [0, 4, 7],
        [0, 7, 3],
        [1, 2, 6],
        [1, 6, 5],
    ]
    .map(|ids| vearth_gen::collision::CollisionTriangle {
        vertices: ids.map(|i| points[i]),
    })
    .to_vec()
}
