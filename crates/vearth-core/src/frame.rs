//! Local render frames on the cube-sphere's source sampling lattice.
//!
//! The +u and -v tangents retain the source grid's horizontal skew; the
//! ellipsoid's geodetic normal supplies vertical. Per-cell axis scales align
//! neighboring sample edges. Geometry uses continuous local coordinates and
//! the dual basis provides their inverse transform. Metric collision uses an
//! orthonormal frame instead of treating this sheared grid as Cartesian metres.

use crate::{cell::ChunkAddr, chunk::CHUNK_DIM, cubesphere, ellipsoid};
use glam::{DMat4, DVec3};

#[derive(Clone, Copy, Debug)]
pub struct ChunkFrame {
    pub addr: ChunkAddr,
    /// ECEF position of the chunk's local origin — local (0,0,0) corner.
    pub origin: DVec3,
    /// ECEF position of the cell-center surface anchor (altitude 0).
    pub anchor: DVec3,
    pub x_axis: DVec3,
    pub y_axis: DVec3,
    pub z_axis: DVec3,
    /// Per-axis sample spacing in meters for this cell.
    pub grid_scale: DVec3,
}

impl ChunkFrame {
    pub fn of(addr: ChunkAddr) -> Self {
        let cell = addr.cell;
        let (cu, cv) = cell.center_uv();
        let dir = cubesphere::face_uv_to_dir(cell.face, cu, cv);
        let anchor = ellipsoid::surface_point_along(dir);
        let up = ellipsoid::geodetic_normal(anchor);
        let (tu, tv) = cubesphere::face_uv_tangents(cell.face, cu, cv);
        let x_axis = (tu - up * tu.dot(up)).normalize();
        let minus_tv = -tv;
        let z_axis = (minus_tv - up * minus_tv.dot(up)).normalize();

        // Actual ground extent of this cell along u and v: angular rate of
        // the parameterization × uv width × geocentric radius.
        let (u0, v0, u1, v1) = cell.uv_bounds();
        let r_local = anchor.length();
        let sx = tu.length() * (u1 - u0) * r_local / CHUNK_DIM as f64;
        let sz = tv.length() * (v1 - v0) * r_local / CHUNK_DIM as f64;
        let sy = 0.5 * (sx + sz);
        let grid_scale = DVec3::new(sx, sy, sz);

        let half = CHUNK_DIM as f64 * 0.5;
        let origin = anchor - x_axis * (half * sx) - z_axis * (half * sz)
            + up * (addr.k as f64 * CHUNK_DIM as f64 * sy);
        ChunkFrame {
            addr,
            origin,
            anchor,
            x_axis,
            y_axis: up,
            z_axis,
            grid_scale,
        }
    }

    /// Mean sample spacing (for LOD metrics and diagnostics).
    pub fn sample_pitch_m(&self) -> f64 {
        self.grid_scale.y
    }

    /// Physical chunk height (the k-layer spacing for this cell).
    pub fn chunk_height_m(&self) -> f64 {
        self.grid_scale.y * CHUNK_DIM as f64
    }

    /// ECEF position of a point given in grid-unit local coordinates
    /// (32 units span the horizontal cell; coordinates are unrestricted).
    pub fn local_to_ecef(&self, local: DVec3) -> DVec3 {
        self.origin
            + self.x_axis * (local.x * self.grid_scale.x)
            + self.y_axis * (local.y * self.grid_scale.y)
            + self.z_axis * (local.z * self.grid_scale.z)
    }

    /// Map an ECEF point into grid-unit local coordinates. The horizontal
    /// cube-sphere basis is intentionally skewed, so dotting with the edge
    /// directions is not an inverse transform; use the dual basis instead.
    pub fn ecef_to_local(&self, point: DVec3) -> DVec3 {
        let bx = self.x_axis * self.grid_scale.x;
        let by = self.y_axis * self.grid_scale.y;
        let bz = self.z_axis * self.grid_scale.z;
        let rel = point - self.origin;
        let det = bx.dot(by.cross(bz));
        DVec3::new(
            rel.dot(by.cross(bz)) / det,
            rel.dot(bz.cross(bx)) / det,
            rel.dot(bx.cross(by)) / det,
        )
    }

    /// Unit normals of the +X, +Y and +Z coordinate planes. These differ from the
    /// corresponding edge directions when the horizontal uv basis is skewed.
    pub fn face_normals(&self) -> [DVec3; 3] {
        [
            self.y_axis.cross(self.z_axis).normalize(),
            self.z_axis.cross(self.x_axis).normalize(),
            self.x_axis.cross(self.y_axis).normalize(),
        ]
    }

    /// Radius of a local box centered at the origin with the supplied half
    /// extents, after applying this frame's linear transform. A Euclidean
    /// `half_extents.length()` is not conservative for the intentionally
    /// skewed horizontal basis.
    pub fn local_box_radius(&self, half_extents: DVec3) -> f64 {
        let bx = self.x_axis * (self.grid_scale.x * half_extents.x);
        let by = self.y_axis * (self.grid_scale.y * half_extents.y);
        let bz = self.z_axis * (self.grid_scale.z * half_extents.z);
        let mut radius: f64 = 0.0;
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for sz in [-1.0, 1.0] {
                    radius = radius.max((bx * sx + by * sy + bz * sz).length());
                }
            }
        }
        radius
    }

    /// Model matrix mapping grid-unit local coordinates to ECEF (f64; the
    /// renderer re-bases it against the camera before converting to f32).
    pub fn model_matrix(&self) -> DMat4 {
        DMat4::from_cols(
            (self.x_axis * self.grid_scale.x).extend(0.0),
            (self.y_axis * self.grid_scale.y).extend(0.0),
            (self.z_axis * self.grid_scale.z).extend(0.0),
            self.origin.extend(1.0),
        )
    }

    /// Geocentric direction of the sample position at fractional grid position
    /// (fx, fz) ∈ \[0,1\]², resolved on the shared cell uv grid so that
    /// neighboring chunks sample identical positions on their shared edge.
    ///
    /// Local +z runs along −v, hence the flip of the v coordinate.
    pub fn column_dir(&self, fx: f64, fz: f64) -> DVec3 {
        let (u, v) = self.addr.cell.uv_at(fx, 1.0 - fz);
        cubesphere::face_uv_to_dir(self.addr.cell.face, u, v)
    }

    /// For the column at (fx, fz): the ellipsoid surface point, and its
    /// altitude in this chunk's frame measured in meters from the *cell
    /// anchor plane* (i.e. from local altitude zero, before the k offset).
    /// Terrain fill adds the sampled terrain height to this offset, which
    /// makes flat chunks follow the curved surface to subsample accuracy.
    pub fn column_surface(&self, fx: f64, fz: f64) -> (DVec3, f64) {
        let p = ellipsoid::surface_point_along(self.column_dir(fx, fz));
        let y_off = (p - self.anchor).dot(self.y_axis);
        (p, y_off)
    }

    /// Latitude/longitude (radians) of the column at (fx, fz) — the sampling
    /// key for geographic data providers.
    pub fn column_latlon(&self, fx: f64, fz: f64) -> (f64, f64) {
        ellipsoid::surface_latlon_along(self.column_dir(fx, fz))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellId;
    use crate::cubesphere::Face;

    fn frame(face: u8, level: u8, i: u32, j: u32, k: i32) -> ChunkFrame {
        ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face: Face(face),
                level,
                i,
                j,
            },
            k,
        })
    }

    #[test]
    fn edge_directions_are_tangent_unit_and_right_handed() {
        for f in 0..6 {
            for (level, i, j) in [(3, 0, 0), (3, 7, 7), (10, 511, 200), (19, 400_000, 1)] {
                let fr = frame(f, level, i, j, 0);
                assert!(fr.x_axis.dot(fr.y_axis).abs() < 1e-12);
                assert!(fr.y_axis.dot(fr.z_axis).abs() < 1e-12);
                assert!((fr.x_axis.length() - 1.0).abs() < 1e-12);
                assert!((fr.y_axis.length() - 1.0).abs() < 1e-12);
                assert!((fr.z_axis.length() - 1.0).abs() < 1e-12);
                assert!(fr.x_axis.cross(fr.y_axis).dot(fr.z_axis) > 0.0);
            }
        }

        // Away from a cube-face center line the uv lattice is genuinely
        // skewed. Re-orthogonalizing this pair caused the directional strip
        // displacement this test suite is guarding against.
        let la = crate::ellipsoid::geodetic_to_ecef(
            34.0522f64.to_radians(),
            -118.2437f64.to_radians(),
            0.0,
        );
        let cell = CellId::from_dir(la, 19);
        let fr = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        assert!(fr.x_axis.dot(fr.z_axis).abs() > 0.20);
    }

    #[test]
    fn sample_scale_tracks_local_cell_size() {
        // Tangent cube-sphere linear scale spans [~1/√2, 1]× nominal: 1.0 on
        // the face center lines, minimum transversally at face-edge midpoints.
        for level in [6u8, 12, 19] {
            let n = 1u32 << level;
            let nominal = crate::cell::sample_pitch_m(level);
            for (i, j) in [(n / 2, n / 2), (0, 0), (n - 1, n / 2)] {
                let fr = frame(0, level, i, j, 0);
                for s in [fr.grid_scale.x, fr.grid_scale.z] {
                    let ratio = s / nominal;
                    assert!(
                        (0.69..1.02).contains(&ratio),
                        "level {level} cell ({i},{j}): scale ratio {ratio}"
                    );
                }
            }
            // And the center cell really is ~nominal.
            let fr = frame(0, level, n / 2, n / 2, 0);
            assert!((fr.grid_scale.x / nominal - 1.0).abs() < 0.02);
        }
    }

    #[test]
    fn chunk_center_is_near_anchor() {
        let fr = frame(0, 10, 500, 500, 0);
        let center = fr.local_to_ecef(DVec3::new(16.0, 0.0, 16.0));
        assert!((center - fr.anchor).length() < 1.0);
    }

    #[test]
    fn neighbor_chunks_meet_to_subsample_in_both_directions() {
        // Adjacent rigid chunks must agree on
        // their shared edge to a small fraction of a sample interval wherever terrain
        // can actually exist (≤ ~9 km altitude), at fine and coarse levels,
        // including across the reparameterization's scale gradient. Both UV
        // directions are required to detect axis-specific displacement.
        for (level, bound) in [(19u8, 0.05), (12, 0.05), (9, 0.10), (6, 0.45), (3, 3.00)] {
            let n = 1u32 << level;
            for face in 0..6 {
                for (i, j) in [(n / 3, n / 2), (0, 0), (n - 2, n / 4)] {
                    let a = frame(face, level, i, j, 0);
                    let bi = frame(face, level, i + 1, j, 0);
                    let bj = frame(face, level, i, j + 1, 0);
                    let y_terrain = (9000.0 / a.grid_scale.y).min(CHUNK_DIM as f64);
                    // Regression table of measured worst gaps (+ margin):
                    // chord-vs-arc error halves per level and is worst near
                    // face edges. See module docs.
                    for q in [0.0, 16.0, 32.0] {
                        for y in [0.0, y_terrain] {
                            let pi_a = a.local_to_ecef(DVec3::new(32.0, y, q));
                            let pi_b = bi.local_to_ecef(DVec3::new(0.0, y, q));
                            let gap_i = (pi_a - pi_b).length() / a.grid_scale.x;
                            assert!(
                                gap_i < bound,
                                "face {face} level {level} cell ({i},{j}) i-edge q {q} y {y}: gap {gap_i} sample intervals (bound {bound})"
                            );

                            // j increases with +v, while local +z follows
                            // -v: a's z=0 boundary meets bj's z=32 boundary.
                            let pj_a = a.local_to_ecef(DVec3::new(q, y, 0.0));
                            let pj_b = bj.local_to_ecef(DVec3::new(q, y, 32.0));
                            let gap_j = (pj_a - pj_b).length() / a.grid_scale.z;
                            assert!(
                                gap_j < bound,
                                "face {face} level {level} cell ({i},{j}) j-edge q {q} y {y}: gap {gap_j} sample intervals (bound {bound})"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn skew_frame_round_trips_local_coordinates_and_normals() {
        let la = crate::ellipsoid::geodetic_to_ecef(
            34.0522f64.to_radians(),
            -118.2437f64.to_radians(),
            0.0,
        );
        let cell = CellId::from_dir(la, 19);
        let fr = ChunkFrame::of(ChunkAddr { cell, k: -2 });
        for local in [
            DVec3::ZERO,
            DVec3::new(32.0, 32.0, 32.0),
            DVec3::new(3.25, 17.5, 29.75),
        ] {
            let back = fr.ecef_to_local(fr.local_to_ecef(local));
            assert!((back - local).length() < 1e-8, "{local:?} -> {back:?}");
        }

        let [nx, ny, nz] = fr.face_normals();
        for normal in [nx, ny, nz] {
            assert!((normal.length() - 1.0).abs() < 1e-12);
        }
        assert!(nx.dot(fr.y_axis).abs() < 1e-12);
        assert!(nx.dot(fr.z_axis).abs() < 1e-12);
        assert!(nx.dot(fr.x_axis) > 0.0);
        assert!(ny.dot(fr.x_axis).abs() < 1e-12);
        assert!(ny.dot(fr.z_axis).abs() < 1e-12);
        assert!(ny.dot(fr.y_axis) > 0.0);
        assert!(nz.dot(fr.x_axis).abs() < 1e-12);
        assert!(nz.dot(fr.y_axis).abs() < 1e-12);
        assert!(nz.dot(fr.z_axis) > 0.0);

        let half_extents = DVec3::new(16.0, 22.0, 16.0);
        let radius = fr.local_box_radius(half_extents);
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for sz in [-1.0, 1.0] {
                    let local = DVec3::new(
                        sx * half_extents.x,
                        sy * half_extents.y,
                        sz * half_extents.z,
                    );
                    let center = fr.local_to_ecef(DVec3::ZERO);
                    let corner = fr.local_to_ecef(local);
                    assert!((corner - center).length() <= radius + 1e-6);
                }
            }
        }
    }

    #[test]
    fn column_surface_offset_matches_curvature() {
        // At the cell center the surface offset is ~0; at chunk corners it is
        // negative (sphere curves away below the tangent plane) and ~sagitta.
        let fr = frame(2, 6, 20, 20, 0);
        let (_, y_center) = fr.column_surface(0.5, 0.5);
        assert!(y_center.abs() < 1e-6);
        let (_, y_corner) = fr.column_surface(0.0, 0.0);
        let half_diag_sq = (fr.grid_scale.x * 16.0).powi(2) + (fr.grid_scale.z * 16.0).powi(2);
        let expected = -half_diag_sq / (2.0 * crate::ellipsoid::MEAN_RADIUS);
        assert!(y_corner < 0.0, "corner should sit below tangent plane");
        assert!(
            (y_corner - expected).abs() < expected.abs() * 0.5 + 1.0,
            "corner drop {y_corner} vs expected ~{expected}"
        );
    }

    #[test]
    fn column_latlon_is_seamless_across_neighbors() {
        let a = frame(1, 9, 100, 40, 0);
        let b = frame(1, 9, 101, 40, 0);
        for fz in [0.0, 0.25, 0.75, 1.0] {
            let (lat_a, lon_a) = a.column_latlon(1.0, fz);
            let (lat_b, lon_b) = b.column_latlon(0.0, fz);
            assert!((lat_a - lat_b).abs() < 1e-14 && (lon_a - lon_b).abs() < 1e-14);
        }
    }

    #[test]
    fn surface_coordinates_keep_shared_sample_keys_through_level_30() {
        for face in 0..6 {
            for level in [1, 5, 11, 19, 30] {
                let n = 1_u32 << level;
                for i in [0, n / 3, n - 2] {
                    let a = frame(face, level, i, n / 3, 0);
                    let b = frame(face, level, i + 1, n / 3, 0);
                    for sample in -1..=33 {
                        let z = sample as f64 / 32.0;
                        let actual = a.column_latlon(1.0, z);
                        assert_eq!(actual, b.column_latlon(0.0, z));
                        let (lat, lon, _) = ellipsoid::ecef_to_geodetic(a.column_surface(1.0, z).0);
                        assert!((actual.0 - lat).abs() < 2.0e-15);
                        assert!((actual.1 - lon).abs() < 2.0e-15);
                    }
                }
            }
        }
    }
}
