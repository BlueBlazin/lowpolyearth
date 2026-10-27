//! WGS84 ellipsoid geometry in ECEF coordinates (meters, f64).
//!
//! ECEF axes: +X pierces (0°N, 0°E), +Y pierces (0°N, 90°E), +Z the north pole.
//!
//! Altitude zero is the ellipsoid surface, not a geoid model.

use glam::DVec3;

pub const WGS84_A: f64 = 6_378_137.0;
pub const WGS84_F: f64 = 1.0 / 298.257_223_563;
pub const WGS84_B: f64 = WGS84_A * (1.0 - WGS84_F);
/// First eccentricity squared.
pub const WGS84_E2: f64 = WGS84_F * (2.0 - WGS84_F);
/// Second eccentricity squared.
pub const WGS84_EP2: f64 = (WGS84_A * WGS84_A - WGS84_B * WGS84_B) / (WGS84_B * WGS84_B);
/// IUGG mean radius, used for nominal (not exact) cell/voxel sizing.
pub const MEAN_RADIUS: f64 = 6_371_008.8;

/// Geodetic (lat, lon in radians, ellipsoidal height in meters) → ECEF.
pub fn geodetic_to_ecef(lat: f64, lon: f64, height: f64) -> DVec3 {
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    DVec3::new(
        (n + height) * cos_lat * cos_lon,
        (n + height) * cos_lat * sin_lon,
        (n * (1.0 - WGS84_E2) + height) * sin_lat,
    )
}

/// ECEF → geodetic (lat, lon in radians, ellipsoidal height in meters).
///
/// Bowring's method as the seed, then a few fixed-point refinements — the
/// single-pass form drifts at high altitude, and the camera lives at up to
/// tens of thousands of km.
pub fn ecef_to_geodetic(p: DVec3) -> (f64, f64, f64) {
    let lon = p.y.atan2(p.x);
    let rho = (p.x * p.x + p.y * p.y).sqrt();
    let theta = (p.z * WGS84_A).atan2(rho * WGS84_B);
    let (sin_t, cos_t) = theta.sin_cos();
    let mut lat =
        (p.z + WGS84_EP2 * WGS84_B * sin_t.powi(3)).atan2(rho - WGS84_E2 * WGS84_A * cos_t.powi(3));
    for _ in 0..4 {
        let sin_lat = lat.sin();
        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
        lat = (p.z + WGS84_E2 * n * sin_lat).atan2(rho);
    }
    let sin_lat = lat.sin();
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    // Near the poles rho/cos(lat) degenerates; pick the better-conditioned form.
    let height = if lat.abs() < 1.2 {
        rho / lat.cos() - n
    } else {
        p.z / sin_lat - n * (1.0 - WGS84_E2)
    };
    (lat, lon, height)
}

/// Intersection of the geocentric ray `t * dir` (t > 0) with the ellipsoid.
/// `dir` need not be normalized.
pub fn surface_point_along(dir: DVec3) -> DVec3 {
    let scale = ((dir.x * dir.x + dir.y * dir.y) / (WGS84_A * WGS84_A)
        + (dir.z * dir.z) / (WGS84_B * WGS84_B))
        .sqrt();
    dir / scale
}

/// Geodetic latitude/longitude (radians) where a nonzero geocentric ray meets
/// the WGS84 surface. The ray need not be normalized, but must be finite with a
/// finite, nonzero squared length. This is a surface query, not the inverse
/// coordinate transform for an elevated ECEF position.
pub fn surface_latlon_along(dir: DVec3) -> (f64, f64) {
    debug_assert!(
        dir.is_finite() && dir.length_squared().is_finite() && dir.length_squared() > 0.0
    );
    // The intersection scale cancels from the ellipsoid gradient's direction.
    // Its horizontal components share 1/a²; the vertical component uses 1/b².
    let rho = (dir.x * dir.x + dir.y * dir.y).sqrt();
    (
        dir.z
            .atan2(rho * ((WGS84_B * WGS84_B) / (WGS84_A * WGS84_A))),
        dir.y.atan2(dir.x),
    )
}

/// Outward geodetic surface normal ("true up") at an ellipsoid surface point.
pub fn geodetic_normal(surface_point: DVec3) -> DVec3 {
    DVec3::new(
        surface_point.x / (WGS84_A * WGS84_A),
        surface_point.y / (WGS84_A * WGS84_A),
        surface_point.z / (WGS84_B * WGS84_B),
    )
    .normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn surface_ray_coordinates_match_the_general_inverse_and_ray_intersection() {
        let mut worst_radians = 0.0_f64;
        let mut worst_metres = 0.0_f64;
        for face in 0..6 {
            for i in -2..=34 {
                for j in -2..=34 {
                    let ray = crate::cubesphere::face_uv_to_dir(
                        crate::cubesphere::Face(face),
                        i as f64 / 16.0 - 1.0,
                        j as f64 / 16.0 - 1.0,
                    );
                    for scale in [0.001, 1.0, WGS84_A, 1.0e12] {
                        let ray = ray * scale;
                        let point = surface_point_along(ray);
                        let (lat, lon) = surface_latlon_along(ray);
                        let (reference_lat, reference_lon, _) = ecef_to_geodetic(point);
                        let lon_error = (lon - reference_lon + PI).rem_euclid(2.0 * PI) - PI;
                        worst_radians = worst_radians
                            .max((lat - reference_lat).abs())
                            .max(lon_error.abs());
                        worst_metres =
                            worst_metres.max((geodetic_to_ecef(lat, lon, 0.0) - point).length());
                    }
                }
            }
        }
        assert!(worst_radians < 2.0e-15, "coordinate error {worst_radians}");
        assert!(worst_metres < 2.0e-8, "surface error {worst_metres}m");
    }

    #[test]
    fn surface_ray_coordinates_keep_poles_axes_and_signed_longitudes() {
        for (ray, expected) in [
            (DVec3::Z, (FRAC_PI_2, 0.0)),
            (-DVec3::Z, (-FRAC_PI_2, -PI)),
            (DVec3::X, (0.0, 0.0)),
            (DVec3::Y, (0.0, FRAC_PI_2)),
            (DVec3::new(-1.0, 0.0, 0.0), (0.0, PI)),
            (DVec3::new(-1.0, -0.0, 0.0), (0.0, -PI)),
        ] {
            let (lat, lon) = surface_latlon_along(ray);
            assert_eq!(lat, expected.0);
            assert_eq!(lon, expected.1);
        }
    }

    #[test]
    fn geodetic_ecef_round_trip() {
        let mut worst = 0.0f64;
        for lat_deg in -89..=89 {
            for lon_deg in (-180..180).step_by(15) {
                for h in [-400.0, 0.0, 8848.0, 400_000.0, 30_000_000.0] {
                    let lat = (lat_deg as f64).to_radians();
                    let lon = (lon_deg as f64).to_radians();
                    let p = geodetic_to_ecef(lat, lon, h);
                    let (lat2, lon2, h2) = ecef_to_geodetic(p);
                    let p2 = geodetic_to_ecef(lat2, lon2, h2);
                    worst = worst.max((p - p2).length());
                    assert!(
                        (lat - lat2).abs() < 1e-9,
                        "lat mismatch at {lat_deg},{lon_deg}"
                    );
                    assert!(
                        (h - h2).abs() < 1e-3,
                        "height mismatch at {lat_deg},{lon_deg},{h}"
                    );
                }
            }
        }
        assert!(worst < 1e-3, "worst ECEF round-trip error {worst} m");
    }

    #[test]
    fn poles_and_axes() {
        let north = geodetic_to_ecef(FRAC_PI_2, 0.0, 0.0);
        assert!((north.z - WGS84_B).abs() < 1e-6);
        assert!(north.x.abs() < 1e-6 && north.y.abs() < 1e-6);
        let (lat, _lon, h) = ecef_to_geodetic(DVec3::new(0.0, 0.0, WGS84_B + 100.0));
        assert!((lat - FRAC_PI_2).abs() < 1e-9);
        assert!((h - 100.0).abs() < 1e-6);
        let x0 = geodetic_to_ecef(0.0, 0.0, 0.0);
        assert!((x0.x - WGS84_A).abs() < 1e-6);
        let (_, lon, _) = ecef_to_geodetic(geodetic_to_ecef(0.0, PI * 0.75, 12.0));
        assert!((lon - PI * 0.75).abs() < 1e-12);
    }

    #[test]
    fn surface_point_is_on_ellipsoid() {
        for dir in [
            DVec3::new(1.0, 2.0, 3.0),
            DVec3::new(-0.3, 0.9, -0.1),
            DVec3::Z,
            DVec3::new(1.0, 0.0, 0.0),
        ] {
            let p = surface_point_along(dir);
            let f =
                (p.x * p.x + p.y * p.y) / (WGS84_A * WGS84_A) + (p.z * p.z) / (WGS84_B * WGS84_B);
            assert!((f - 1.0).abs() < 1e-12);
            // Normal must agree with the geodetic normal from lat/lon.
            let (lat, lon, _) = ecef_to_geodetic(p);
            let n_geo = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
            assert!(geodetic_normal(p).dot(n_geo) > 1.0 - 1e-10);
        }
    }
}
