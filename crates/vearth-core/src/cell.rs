//! Quadtree cell and chunk addressing.
//!
//! A `CellId` names a square region of one cube face at some subdivision
//! level. A `ChunkAddr` adds a radial layer index `k`, naming one 32³ voxel
//! chunk. Addresses are canonical and global: the same address always
//! generates the same chunk.

use crate::cubesphere::{self, Face};
use crate::ellipsoid::MEAN_RADIUS;
use glam::DVec3;
use std::f64::consts::FRAC_PI_2;

/// Nominal arc length of one cube-face edge (quarter great circle).
pub const NOMINAL_FACE_METERS: f64 = FRAC_PI_2 * MEAN_RADIUS;

/// Coarsest level the renderer uses. Below L3 a flat chunk deviates from the
/// sphere by more than a voxel (sagitta/voxel ≈ 4·cell/R), so the quadtree
/// roots for rendering purposes are the 6×8×8 = 384 L3 cells.
pub const MIN_RENDER_LEVEL: u8 = 3;

/// Finest global level (≈0.6 m nominal sample spacing).
pub const MAX_LEVEL: u8 = 19;

/// Nominal cell edge length in meters at `level`.
pub fn cell_size_m(level: u8) -> f64 {
    NOMINAL_FACE_METERS / (1u64 << level) as f64
}

/// Nominal voxel edge length in meters at `level`.
pub fn sample_pitch_m(level: u8) -> f64 {
    cell_size_m(level) / crate::chunk::CHUNK_DIM as f64
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct CellId {
    pub face: Face,
    pub level: u8,
    pub i: u32,
    pub j: u32,
}

impl CellId {
    pub fn root(face: Face) -> Self {
        CellId {
            face,
            level: 0,
            i: 0,
            j: 0,
        }
    }

    /// Number of cells along one face edge at this cell's level.
    pub fn edge_cells(self) -> u32 {
        1u32 << self.level
    }

    /// (u_min, v_min, u_max, v_max) bounds in face coordinates [-1, 1].
    pub fn uv_bounds(self) -> (f64, f64, f64, f64) {
        let n = self.edge_cells() as f64;
        let w = 2.0 / n;
        let u0 = -1.0 + self.i as f64 * w;
        let v0 = -1.0 + self.j as f64 * w;
        (u0, v0, u0 + w, v0 + w)
    }

    pub fn center_uv(self) -> (f64, f64) {
        let (u0, v0, u1, v1) = self.uv_bounds();
        (0.5 * (u0 + u1), 0.5 * (v0 + v1))
    }

    /// Face coordinates at fractional position (fu, fv) ∈ \[0,1\]² within the cell.
    pub fn uv_at(self, fu: f64, fv: f64) -> (f64, f64) {
        let (u0, v0, u1, v1) = self.uv_bounds();
        (u0 + fu * (u1 - u0), v0 + fv * (v1 - v0))
    }

    pub fn center_dir(self) -> DVec3 {
        let (u, v) = self.center_uv();
        cubesphere::face_uv_to_dir(self.face, u, v)
    }

    /// The cell at `level` containing the given geocentric direction.
    pub fn from_dir(dir: DVec3, level: u8) -> Self {
        let (face, u, v) = cubesphere::dir_to_face_uv(dir);
        let n = 1u32 << level;
        let to_index = |c: f64| (((c + 1.0) * 0.5 * n as f64) as u32).min(n - 1);
        CellId {
            face,
            level,
            i: to_index(u),
            j: to_index(v),
        }
    }

    pub fn parent(self) -> Option<Self> {
        if self.level == 0 {
            return None;
        }
        Some(CellId {
            face: self.face,
            level: self.level - 1,
            i: self.i / 2,
            j: self.j / 2,
        })
    }

    pub fn children(self) -> [Self; 4] {
        let (f, l, i, j) = (self.face, self.level + 1, self.i * 2, self.j * 2);
        [
            CellId {
                face: f,
                level: l,
                i,
                j,
            },
            CellId {
                face: f,
                level: l,
                i: i + 1,
                j,
            },
            CellId {
                face: f,
                level: l,
                i,
                j: j + 1,
            },
            CellId {
                face: f,
                level: l,
                i: i + 1,
                j: j + 1,
            },
        ]
    }

    pub fn size_m(self) -> f64 {
        cell_size_m(self.level)
    }

    pub fn sample_pitch_m(self) -> f64 {
        sample_pitch_m(self.level)
    }

    /// Angle (radians) from cell center direction to its farthest corner.
    /// Exact via the corner directions; used for horizon/frustum culling.
    pub fn angular_radius(self) -> f64 {
        let c = self.center_dir();
        let (u0, v0, u1, v1) = self.uv_bounds();
        [(u0, v0), (u1, v0), (u0, v1), (u1, v1)]
            .into_iter()
            .map(|(u, v)| {
                c.dot(cubesphere::face_uv_to_dir(self.face, u, v))
                    .clamp(-1.0, 1.0)
            })
            // acos is decreasing: the farthest corner has the smallest dot.
            .fold(1.0, f64::min)
            .acos()
    }
}

/// One 32³ voxel chunk: a cell plus a radial layer. Layer `k` spans local
/// altitudes [k·size, (k+1)·size) meters relative to the ellipsoid, measured
/// along the chunk frame's up axis (see `ChunkFrame`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkAddr {
    pub cell: CellId,
    pub k: i32,
}

impl ChunkAddr {
    /// Radial layer range covering local altitudes [min_m, max_m], given the
    /// cell's actual chunk height (`ChunkFrame::chunk_height_m`, which varies
    /// per cell — always use the frame's value so the enumerator and the
    /// generator agree).
    pub fn k_range_for_altitudes(
        chunk_height_m: f64,
        min_m: f64,
        max_m: f64,
    ) -> std::ops::RangeInclusive<i32> {
        let k0 = (min_m / chunk_height_m).floor() as i32;
        let k1 = (max_m / chunk_height_m).floor() as i32;
        k0..=k1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ladder_matches_expected_scales() {
        // Nominal level-to-voxel-size rungs.
        assert!((cell_size_m(19) - 19.1).abs() < 0.1);
        assert!((sample_pitch_m(19) - 0.60).abs() < 0.01);
        assert!((cell_size_m(15) - 305.5).abs() < 1.0);
        assert!((cell_size_m(0) / 1000.0 - 10_007.0).abs() < 10.0);
    }

    #[test]
    fn from_dir_center_round_trip() {
        for f in 0..6 {
            for level in [0u8, 1, 3, 7, 12, 19] {
                let n = 1u32 << level;
                // Sample a diagonal of cells across the face.
                for t in 0..8 {
                    let i = (t * n.max(1) / 8).min(n - 1);
                    let cell = CellId {
                        face: Face(f),
                        level,
                        i,
                        j: n - 1 - i,
                    };
                    let back = CellId::from_dir(cell.center_dir(), level);
                    assert_eq!(cell, back);
                }
            }
        }
    }

    #[test]
    fn angular_radius_matches_corner_angle_reference() {
        let mut state = 0x674ab3599120e1b3u64;
        for face in 0..6 {
            for level in [0, 1, 3, 7, 12, 19, 24, 30, 31] {
                let edge = 1u32 << level;
                for sample in 0..256 {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    let (i, j) = match sample {
                        0 => (0, 0),
                        1 => (edge - 1, 0),
                        2 => (0, edge - 1),
                        3 => (edge - 1, edge - 1),
                        _ => (state as u32 % edge, (state >> 32) as u32 % edge),
                    };
                    let cell = CellId {
                        face: Face(face),
                        level,
                        i,
                        j,
                    };
                    let center = cell.center_dir();
                    let (u0, v0, u1, v1) = cell.uv_bounds();
                    let reference = [(u0, v0), (u1, v0), (u0, v1), (u1, v1)]
                        .into_iter()
                        .map(|(u, v)| {
                            center
                                .dot(cubesphere::face_uv_to_dir(cell.face, u, v))
                                .clamp(-1.0, 1.0)
                                .acos()
                        })
                        .fold(0.0, f64::max);
                    assert_eq!(
                        cell.angular_radius().to_bits(),
                        reference.to_bits(),
                        "{cell:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn parent_child_consistency() {
        let cell = CellId {
            face: Face(2),
            level: 10,
            i: 123,
            j: 987,
        };
        for ch in cell.children() {
            assert_eq!(ch.parent(), Some(cell));
            // Child center must be inside parent uv bounds.
            let (u, v) = ch.center_uv();
            let (u0, v0, u1, v1) = cell.uv_bounds();
            assert!(u > u0 && u < u1 && v > v0 && v < v1);
        }
    }

    #[test]
    fn neighbor_cells_share_boundary_direction() {
        // Adjacent cells on the same face must agree exactly on their shared
        // boundary — this is what makes per-cell sampling seam-free.
        let a = CellId {
            face: Face(1),
            level: 8,
            i: 100,
            j: 40,
        };
        let b = CellId {
            face: Face(1),
            level: 8,
            i: 101,
            j: 40,
        };
        let (_, _, ua1, _) = a.uv_bounds();
        let (ub0, ..) = b.uv_bounds();
        assert_eq!(ua1, ub0, "uv bounds must be bit-identical");
    }

    #[test]
    fn angular_radius_sane() {
        let root = CellId::root(Face(0));
        // Root face corner is at atan(sqrt(2)) ≈ 54.7° from center.
        assert!((root.angular_radius().to_degrees() - 54.7356).abs() < 0.01);
        let fine = CellId {
            face: Face(0),
            level: 10,
            i: 512,
            j: 512,
        };
        assert!(fine.angular_radius() < 0.01);
    }

    #[test]
    fn k_range() {
        let s = cell_size_m(10);
        let r = ChunkAddr::k_range_for_altitudes(s, -10.0, s * 2.5);
        assert_eq!(r, -1..=2);
    }
}
