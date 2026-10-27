//! Level-of-detail selection and visibility helpers.

use crate::cell::CellId;
use crate::ellipsoid::MEAN_RADIUS;
use glam::DVec3;

/// Projected terrain sample spacing in pixels, used as the refinement metric.
pub fn sample_screen_px(
    sample_pitch_m: f64,
    distance_m: f64,
    viewport_h_px: f64,
    fov_y_rad: f64,
) -> f64 {
    let px_per_rad = viewport_h_px / fov_y_rad;
    (sample_pitch_m / distance_m.max(1.0)) * px_per_rad
}

/// Conservative behind-the-horizon test for a cell as seen from `cam_ecef`.
/// Returns true only when the whole cell is definitely hidden by the planet.
///
/// Uses a deliberately shrunken planet radius so terrain relief and the
/// cell's own extent can never cause false culling.
pub fn horizon_culled(cam_ecef: DVec3, cell: CellId) -> bool {
    HorizonCuller::new(cam_ecef).culled(cell)
}

/// Immutable spherical cap for a cell's conservative horizon test.
#[derive(Clone, Copy, Debug)]
pub struct CellHorizon {
    center_direction: DVec3,
    angular_radius: f64,
}

impl CellHorizon {
    pub fn new(cell: CellId) -> Self {
        Self {
            center_direction: cell.center_dir(),
            angular_radius: cell.angular_radius(),
        }
    }
}

/// Camera-invariant part of the conservative horizon test. Reuse one per
/// frame or LOD selection instead of recomputing the same view angles for
/// each candidate cell.
pub struct HorizonCuller {
    camera_direction: DVec3,
    camera_horizon_angle: f64,
    below_guard: bool,
}

impl HorizonCuller {
    pub fn new(cam_ecef: DVec3) -> Self {
        const R_SAFE: f64 = MEAN_RADIUS - 20_000.0;
        let r_cam = cam_ecef.length();
        let below_guard = r_cam <= R_SAFE;
        Self {
            camera_direction: if below_guard {
                DVec3::ZERO
            } else {
                cam_ecef.normalize()
            },
            camera_horizon_angle: if below_guard {
                0.0
            } else {
                (R_SAFE / r_cam).clamp(-1.0, 1.0).acos()
            },
            below_guard,
        }
    }

    pub fn culled(&self, cell: CellId) -> bool {
        if self.below_guard {
            return false;
        }
        self.culled_cap(CellHorizon::new(cell))
    }

    /// Reuse a resident cell's cap without recomputing its corner directions.
    pub fn culled_cap(&self, cap: CellHorizon) -> bool {
        if self.below_guard {
            return false;
        }
        // The cell radius and elevation allowance include terrain beyond its
        // center's horizon. Keep the operation order shared with scalar calls.
        let limit = self.camera_horizon_angle + cap.angular_radius + 0.06;
        let gamma = self
            .camera_direction
            .dot(cap.center_direction)
            .clamp(-1.0, 1.0)
            .acos();
        gamma > limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cubesphere::Face;

    #[test]
    fn sample_px_scales() {
        // A 1 m sample interval at 100 m, 1080p, 60° fov ≈ 10 px.
        let px = sample_screen_px(1.0, 100.0, 1080.0, 60f64.to_radians());
        assert!((px - 10.3).abs() < 0.5, "{px}");
        // Twice as far → half the pixels.
        let px2 = sample_screen_px(1.0, 200.0, 1080.0, 60f64.to_radians());
        assert!((px / px2 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn cached_camera_horizon_matches_scalar_reference() {
        for direction in [
            DVec3::X,
            DVec3::NEG_Y,
            DVec3::new(1.0, 2.0, -3.0).normalize(),
        ] {
            for altitude in [-30_000.0, 0.0, 100.0, 30_000.0, 1_000_000.0, 20_000_000.0] {
                let cam = direction * (MEAN_RADIUS + altitude);
                let culler = HorizonCuller::new(cam);
                for face in 0..6 {
                    for level in [0, 3, 8, 17, 24] {
                        let width = 1 << level;
                        for seed in 0..129 {
                            let cell = CellId {
                                face: Face(face),
                                level,
                                i: (seed * 193) % width,
                                j: (seed * 317) % width,
                            };
                            let r_safe = MEAN_RADIUS - 20_000.0;
                            let r_cam = cam.length();
                            let expected = if r_cam <= r_safe {
                                false
                            } else {
                                let limit = (r_safe / r_cam).clamp(-1.0, 1.0).acos()
                                    + cell.angular_radius()
                                    + 0.06;
                                let gamma = cam
                                    .normalize()
                                    .dot(cell.center_dir())
                                    .clamp(-1.0, 1.0)
                                    .acos();
                                gamma > limit
                            };
                            assert_eq!(culler.culled(cell), expected);
                            assert_eq!(culler.culled_cap(CellHorizon::new(cell)), expected);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn resident_horizon_cap_matches_scalar_at_moving_horizon_boundaries() {
        for face in 0..6 {
            for level in [0, 8, 24, 30] {
                let width = 1 << level;
                let cell = CellId {
                    face: Face(face),
                    level,
                    i: width / 3,
                    j: width / 5,
                };
                let cap = CellHorizon::new(cell);
                let tangent = cap
                    .center_direction
                    .cross(if cap.center_direction.x.abs() < 0.8 {
                        DVec3::X
                    } else {
                        DVec3::Y
                    })
                    .normalize();
                for altitude in [-30_000.0, 0.0, 30_000.0, 20_000_000.0] {
                    let distance = MEAN_RADIUS + altitude;
                    let safe = MEAN_RADIUS - 20_000.0;
                    let horizon_angle = (safe / distance).clamp(-1.0, 1.0).acos();
                    for offset in [-1e-9, 0.0, 1e-9] {
                        let angle = (horizon_angle + cell.angular_radius() + 0.06 + offset)
                            .min(std::f64::consts::PI);
                        let camera =
                            (cap.center_direction * angle.cos() + tangent * angle.sin()) * distance;
                        let actual_distance = camera.length();
                        let expected = actual_distance > safe
                            && camera
                                .normalize()
                                .dot(cell.center_dir())
                                .clamp(-1.0, 1.0)
                                .acos()
                                > (safe / actual_distance).clamp(-1.0, 1.0).acos()
                                    + cell.angular_radius()
                                    + 0.06;
                        assert_eq!(
                            HorizonCuller::new(camera).culled_cap(cap),
                            expected,
                            "cell={cell:?}, altitude={altitude}, boundary offset={offset}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn horizon_culls_far_side_only() {
        // Camera 1000 km above lon 0 on the equator (+X axis). Horizon sits
        // ~30.5° of center-angle from nadir.
        let cam = DVec3::new(MEAN_RADIUS + 1_000_000.0, 0.0, 0.0);
        // Cells safely inside the horizon must never be culled.
        let near = CellId::from_dir(DVec3::X, 10);
        assert!(!horizon_culled(cam, near));
        let inside = CellId::from_dir(
            DVec3::new(25f64.to_radians().cos(), 25f64.to_radians().sin(), 0.0),
            10,
        );
        assert!(!horizon_culled(cam, inside));
        // A fine cell on the exact far side must be culled...
        let far = CellId::from_dir(DVec3::NEG_X, 10);
        assert!(horizon_culled(cam, far));
        // ...as must the whole far-side root face: its nearest content is
        // 125° from nadir, far beyond any horizon.
        assert!(horizon_culled(cam, CellId::root(Face(1))));
        // From very high altitude the far hemisphere stays hidden (visibility
        // asymptotically approaches 90° + elevation margin).
        let cam_high = DVec3::new(MEAN_RADIUS + 20_000_000.0, 0.0, 0.0);
        assert!(!horizon_culled(cam_high, near));
        assert!(horizon_culled(cam_high, far));
        // From ground level the antipode is culled (correct), and anything
        // within a few degrees is not.
        let cam_ground = DVec3::new(MEAN_RADIUS + 2.0, 0.0, 0.0);
        assert!(horizon_culled(cam_ground, far));
        assert!(!horizon_culled(cam_ground, near));
        // Below the guard radius nothing is ever culled.
        let cam_deep = DVec3::new(MEAN_RADIUS - 30_000.0, 0.0, 0.0);
        assert!(!horizon_culled(cam_deep, far));
    }
}
