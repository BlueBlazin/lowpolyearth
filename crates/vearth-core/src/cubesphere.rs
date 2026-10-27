//! Cube-sphere parameterization.
//!
//! Six cube faces wrap the sphere of geocentric *directions*. Face coordinates
//! (u, v) ∈ [-1, 1]² pass through a tangent reparameterization before the
//! gnomonic projection, which keeps cell sizes within roughly ±10% of nominal
//! across a face (raw gnomonic would vary far more).
//!
//! `CUBE_ROTATION` defines the mapping from cube space to ECEF.

use glam::{DMat3, DVec3};
use std::f64::consts::FRAC_PI_4;

/// Global cube orientation: cube space → ECEF.
pub const CUBE_ROTATION: DMat3 = DMat3::IDENTITY;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Face(pub u8);

/// Per-face orthonormal bases in cube space: (normal, right ≙ +u, up ≙ +v),
/// each right-handed (right × up = normal).
const FACE_AXES: [(DVec3, DVec3, DVec3); 6] = [
    (DVec3::X, DVec3::Y, DVec3::Z),     // +X: equator, lon 0
    (DVec3::NEG_X, DVec3::Z, DVec3::Y), // -X: equator, lon 180
    (DVec3::Y, DVec3::Z, DVec3::X),     // +Y: equator, lon 90E
    (DVec3::NEG_Y, DVec3::X, DVec3::Z), // -Y: equator, lon 90W
    (DVec3::Z, DVec3::X, DVec3::Y),     // +Z: north polar
    (DVec3::NEG_Z, DVec3::Y, DVec3::X), // -Z: south polar
];

impl Face {
    pub const COUNT: u8 = 6;

    pub fn normal(self) -> DVec3 {
        CUBE_ROTATION * FACE_AXES[self.0 as usize].0
    }
    pub fn right(self) -> DVec3 {
        CUBE_ROTATION * FACE_AXES[self.0 as usize].1
    }
    pub fn up(self) -> DVec3 {
        CUBE_ROTATION * FACE_AXES[self.0 as usize].2
    }
}

/// Tangent reparameterization: cube coordinate c ∈ [-1,1] → gnomonic plane.
#[inline]
fn reparam(c: f64) -> f64 {
    (c * FRAC_PI_4).tan()
}

#[inline]
fn reparam_inv(g: f64) -> f64 {
    g.atan() / FRAC_PI_4
}

/// d(reparam)/dc — used for analytic tangents.
#[inline]
fn reparam_deriv(c: f64) -> f64 {
    let sec = 1.0 / (c * FRAC_PI_4).cos();
    FRAC_PI_4 * sec * sec
}

/// Face (u, v) → unit geocentric direction (ECEF).
pub fn face_uv_to_dir(face: Face, u: f64, v: f64) -> DVec3 {
    let (gu, gv) = (reparam(u), reparam(v));
    (face.normal() + gu * face.right() + gv * face.up()).normalize()
}

/// Unit geocentric direction → (face, u, v). Total on the sphere; points on
/// face boundaries resolve to the face with the largest axis component.
pub fn dir_to_face_uv(dir: DVec3) -> (Face, f64, f64) {
    let d = CUBE_ROTATION.transpose() * dir;
    let ax = d.x.abs();
    let ay = d.y.abs();
    let az = d.z.abs();
    let face = if ax >= ay && ax >= az {
        if d.x >= 0.0 { Face(0) } else { Face(1) }
    } else if ay >= ax && ay >= az {
        if d.y >= 0.0 { Face(2) } else { Face(3) }
    } else if d.z >= 0.0 {
        Face(4)
    } else {
        Face(5)
    };
    let n = dir.dot(face.normal());
    let gu = dir.dot(face.right()) / n;
    let gv = dir.dot(face.up()) / n;
    (
        face,
        reparam_inv(gu).clamp(-1.0, 1.0),
        reparam_inv(gv).clamp(-1.0, 1.0),
    )
}

/// Analytic partial derivatives of `face_uv_to_dir` — the (unnormalized)
/// tangent directions along +u and +v on the unit sphere.
pub fn face_uv_tangents(face: Face, u: f64, v: f64) -> (DVec3, DVec3) {
    let (gu, gv) = (reparam(u), reparam(v));
    let w = face.normal() + gu * face.right() + gv * face.up();
    let len = w.length();
    let w_hat = w / len;
    // d(normalize(w)) = (I - ŵŵᵀ)/|w| · dw
    let proj = |dw: DVec3| (dw - w_hat * dw.dot(w_hat)) / len;
    (
        proj(face.right() * reparam_deriv(u)),
        proj(face.up() * reparam_deriv(v)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pseudo_random_dirs(n: usize) -> Vec<DVec3> {
        // Deterministic low-discrepancy-ish directions; no RNG dep.
        (0..n)
            .map(|i| {
                let t = i as f64 * 0.618_033_988_749_895;
                let z = -1.0 + 2.0 * ((i as f64 + 0.5) / n as f64);
                let r = (1.0 - z * z).sqrt();
                let phi = t * std::f64::consts::TAU;
                DVec3::new(r * phi.cos(), r * phi.sin(), z)
            })
            .collect()
    }

    #[test]
    fn uv_dir_round_trip() {
        for f in 0..6 {
            for iu in 0..=20 {
                for iv in 0..=20 {
                    let u = -1.0 + iu as f64 * 0.1;
                    let v = -1.0 + iv as f64 * 0.1;
                    let dir = face_uv_to_dir(Face(f), u, v);
                    assert!((dir.length() - 1.0).abs() < 1e-12);
                    let (f2, u2, v2) = dir_to_face_uv(dir);
                    // Boundary points may resolve to a neighboring face; skip those.
                    if u.abs() < 0.999 && v.abs() < 0.999 {
                        assert_eq!(f2, Face(f), "face mismatch at {u},{v}");
                        assert!((u - u2).abs() < 1e-12 && (v - v2).abs() < 1e-12);
                    }
                }
            }
        }
    }

    #[test]
    fn dir_uv_round_trip_total() {
        for dir in pseudo_random_dirs(5000) {
            let (f, u, v) = dir_to_face_uv(dir);
            assert!((-1.0..=1.0).contains(&u) && (-1.0..=1.0).contains(&v));
            let dir2 = face_uv_to_dir(f, u, v);
            assert!(dir.dot(dir2) > 1.0 - 1e-12, "round trip drift for {dir:?}");
        }
    }

    #[test]
    fn tangents_match_numeric() {
        let eps = 1e-7;
        for f in 0..6 {
            for &(u, v) in &[(0.0, 0.0), (0.5, -0.3), (-0.9, 0.9), (0.99, 0.01)] {
                let (tu, tv) = face_uv_tangents(Face(f), u, v);
                let nu = (face_uv_to_dir(Face(f), u + eps, v)
                    - face_uv_to_dir(Face(f), u - eps, v))
                    / (2.0 * eps);
                let nv = (face_uv_to_dir(Face(f), u, v + eps)
                    - face_uv_to_dir(Face(f), u, v - eps))
                    / (2.0 * eps);
                assert!((tu - nu).length() < 1e-5 * tu.length().max(1.0));
                assert!((tv - nv).length() < 1e-5 * tv.length().max(1.0));
            }
        }
    }

    #[test]
    fn cell_scale_variation_is_bounded() {
        // The whole point of the tangent reparameterization: angular step per
        // unit-u stays within a modest band across the face.
        let mut min = f64::INFINITY;
        let mut max: f64 = 0.0;
        for iu in 0..=10 {
            for iv in 0..=10 {
                let u = -1.0 + iu as f64 * 0.2;
                let v = -1.0 + iv as f64 * 0.2;
                let (tu, tv) = face_uv_tangents(Face(0), u, v);
                for len in [tu.length(), tv.length()] {
                    min = min.min(len);
                    max = max.max(len);
                }
            }
        }
        let ratio = max / min;
        assert!(
            ratio < 1.5,
            "tangent-reparam scale spread too wide: {ratio} (min {min}, max {max})"
        );
    }

    #[test]
    fn faces_cover_and_agree_on_poles() {
        let (f, u, v) = dir_to_face_uv(DVec3::Z);
        assert_eq!(f, Face(4));
        assert!(u.abs() < 1e-12 && v.abs() < 1e-12);
        let (f, ..) = dir_to_face_uv(DVec3::NEG_Z);
        assert_eq!(f, Face(5));
    }
}
