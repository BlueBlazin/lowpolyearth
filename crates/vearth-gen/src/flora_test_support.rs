//! Shared collision invariants for visual flora tests.
use crate::{SurfaceCell, TerrainCollision, collision::SurfaceCollision};
use vearth_core::cell::CellId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualFloraControl {
    Disabled,
    Enabled,
}

/// Physical output, independent of surface materials and visual descriptors.
#[derive(Clone, Debug, PartialEq)]
pub struct SolidGeometry {
    pub cell: CellId,
    pub terrain: Option<TerrainCollision>,
    pub solids: SurfaceCollision,
}

pub fn solid_geometry(cell: &SurfaceCell) -> SolidGeometry {
    SolidGeometry {
        cell: cell.cell,
        terrain: cell.terrain_collision.clone(),
        solids: cell.solid_collision.clone(),
    }
}

#[track_caller]
pub fn assert_visual_flora_preserves_collision(
    mut generate: impl FnMut(VisualFloraControl) -> SurfaceCell,
) {
    assert_eq!(
        solid_geometry(&generate(VisualFloraControl::Disabled)),
        solid_geometry(&generate(VisualFloraControl::Enabled)),
        "visual flora changed physical geometry"
    );
}
