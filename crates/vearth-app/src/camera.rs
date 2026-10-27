//! Globe camera: geodetic position plus heading and pitch, with drag-to-pan,
//! cursor-anchored zoom, and modifier-drag orbit operations. Positions use
//! `f64` ECEF; the renderer works camera-relative.

use glam::{DMat3, DMat4, DVec3};
use vearth_core::ellipsoid::{self, MEAN_RADIUS};

#[derive(Clone, Debug)]
pub struct Camera {
    /// Geodetic latitude/longitude in radians.
    pub lat: f64,
    pub lon: f64,
    /// Height above the ellipsoid in meters.
    pub alt_m: f64,
    /// 0 = looking north; positive turns east.
    pub heading: f64,
    /// 0 = straight down; toward pi/2 tilts to the horizon.
    pub pitch: f64,
    pub fov_y: f64,
    /// Viewport aspect (w/h), refreshed each frame; bounds max zoom-out.
    pub aspect: f64,
}

fn wrap_pi(a: f64) -> f64 {
    let mut a = a % std::f64::consts::TAU;
    if a > std::f64::consts::PI {
        a -= std::f64::consts::TAU;
    } else if a < -std::f64::consts::PI {
        a += std::f64::consts::TAU;
    }
    a
}

impl Camera {
    pub fn start() -> Self {
        // Above the Alps, high enough to see the whole globe.
        Camera {
            lat: 46f64.to_radians(),
            lon: 8f64.to_radians(),
            alt_m: 16_000_000.0,
            heading: 0.0,
            pitch: 0.0,
            fov_y: 55f64.to_radians(),
            aspect: 1.6,
        }
    }

    /// Altitude at which the globe's diameter fills ~85% of the smaller
    /// viewport dimension — the Maps-style zoom-out stop.
    pub fn max_zoom_out_alt(&self) -> f64 {
        let half_fov_min = if self.aspect >= 1.0 {
            self.fov_y * 0.5
        } else {
            ((self.fov_y * 0.5).tan() * self.aspect).atan()
        };
        let theta = (0.85 * half_fov_min.tan()).atan();
        (MEAN_RADIUS / theta.sin() - MEAN_RADIUS).clamp(1_000_000.0, 40_000_000.0)
    }

    pub fn eye_ecef(&self) -> DVec3 {
        ellipsoid::geodetic_to_ecef(self.lat, self.lon, self.alt_m)
    }

    fn enu(&self) -> (DVec3, DVec3, DVec3) {
        let (sin_lat, cos_lat) = self.lat.sin_cos();
        let (sin_lon, cos_lon) = self.lon.sin_cos();
        let east = DVec3::new(-sin_lon, cos_lon, 0.0);
        let north = DVec3::new(-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat);
        let up = DVec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat);
        (east, north, up)
    }

    /// View rotation (no translation — camera-relative rendering).
    pub fn view_rotation(&self) -> DMat4 {
        let (east, north, up) = self.enu();
        let horiz = north * self.heading.cos() + east * self.heading.sin();
        let (sin_p, cos_p) = self.pitch.sin_cos();
        let forward = -up * cos_p + horiz * sin_p;
        let cam_up = up * sin_p + horiz * cos_p;
        glam::dcamera::rh::view::look_to_mat4(DVec3::ZERO, forward, cam_up)
    }

    /// The view direction in ECEF (for view-weighted streaming priority).
    pub fn forward_ecef(&self) -> DVec3 {
        let (east, north, up) = self.enu();
        let horiz = north * self.heading.cos() + east * self.heading.sin();
        let (sin_p, cos_p) = self.pitch.sin_cos();
        -up * cos_p + horiz * sin_p
    }

    /// Move over the surface by ground meters (heading-relative north/east).
    pub fn pan_ground(&mut self, north_m: f64, east_m: f64) {
        let (dn, de) = (
            north_m * self.heading.cos() - east_m * self.heading.sin(),
            north_m * self.heading.sin() + east_m * self.heading.cos(),
        );
        self.lat = (self.lat + dn / MEAN_RADIUS).clamp(-1.55, 1.55);
        self.lon += de / (MEAN_RADIUS * self.lat.cos().max(0.05));
        if self.lon > std::f64::consts::PI {
            self.lon -= std::f64::consts::TAU;
        } else if self.lon < -std::f64::consts::PI {
            self.lon += std::f64::consts::TAU;
        }
    }

    /// Drag by screen pixels: grab-the-world, scaled by altitude.
    pub fn drag(&mut self, dx_px: f64, dy_px: f64, viewport_h: f64) {
        self.drag_over_ground(dx_px, dy_px, viewport_h, 0.0);
    }

    /// Drag using height above the visible terrain, rather than ellipsoid
    /// altitude. At high-elevation locations this preserves the exact same
    /// mouse sensitivity as an equivalent sea-level view.
    pub fn drag_over_ground(
        &mut self,
        dx_px: f64,
        dy_px: f64,
        viewport_h: f64,
        ground_altitude_m: f64,
    ) {
        let clearance_m = (self.alt_m - ground_altitude_m).max(25.0);
        let ground_per_px = 2.0 * clearance_m * (self.fov_y * 0.5).tan() / viewport_h.max(1.0);
        self.pan_ground(dy_px * ground_per_px, -dx_px * ground_per_px);
    }

    pub fn zoom(&mut self, scroll: f64) {
        self.alt_m = (self.alt_m * (-scroll * 0.13).exp()).clamp(30.0, self.max_zoom_out_alt());
    }

    /// NDC of a world point through the current pose (None if behind).
    fn ndc_of(&self, p: DVec3) -> Option<(f64, f64)> {
        let rel = p - self.eye_ecef();
        let v = DMat3::from_mat4(self.view_rotation()) * rel;
        if v.z >= -1e-9 {
            return None;
        }
        let th = (self.fov_y * 0.5).tan();
        Some((v.x / (-v.z) / (th * self.aspect), v.y / (-v.z) / th))
    }

    /// Approach or recede from a ground point while keeping it at the same
    /// pixel and preserving heading. Orbital views may flatten pitch after
    /// the anchored correction.
    ///
    /// Translating the eye with a world-fixed forward vector would pin the
    /// point exactly, but at globe scales one notch travels far enough that
    /// the local vertical changes several degrees — the camera then tilts,
    /// and near-nadir the re-derived heading is ill-conditioned and swings
    /// violently. Instead: move along the chord keeping the local-frame
    /// orientation, then re-pin the anchor with a few damped ground-pan
    /// corrections, which converge to sub-pixel error at every scale.
    pub fn zoom_towards(&mut self, scroll: f64, anchor: Option<DVec3>) {
        let Some(target) = anchor else {
            self.zoom(scroll);
            return;
        };
        let Some(ndc0) = self.ndc_of(target) else {
            self.zoom(scroll);
            return;
        };
        let eye = self.eye_ecef();
        let d = (eye - target).length();
        if d < 1.0 {
            return;
        }
        let d_new = (d * (-scroll * 0.13).exp()).max(35.0);
        let (lat, lon, alt) = ellipsoid::ecef_to_geodetic(target + (eye - target) * (d_new / d));
        self.lat = lat.clamp(-1.55, 1.55);
        self.lon = wrap_pi(lon);
        self.alt_m = alt.max(5.0).min(self.max_zoom_out_alt());

        // The local frame rotated under the fixed heading/pitch, so the
        // anchor drifted off its pixel; pan the ground position to put it
        // back. The x-gain is exact (camera-right pans are depth-neutral);
        // the y-gain weakens with tilt, compensated and iterated.
        let th = (self.fov_y * 0.5).tan();
        let y_comp = (1.0 / self.pitch.cos().max(0.35)).min(2.0);
        let mut last_err = f64::INFINITY;
        for _ in 0..6 {
            let Some(ndc) = self.ndc_of(target) else {
                break;
            };
            let (ex, ey) = (ndc.0 - ndc0.0, ndc.1 - ndc0.1);
            let err = ex.abs().max(ey.abs());
            if err < 5e-5 || err > last_err {
                break;
            }
            last_err = err;
            let d_now = (self.eye_ecef() - target).length();
            self.pan_ground(
                ey * th * d_now * 0.9 * y_comp,
                ex * th * self.aspect * d_now * 0.9,
            );
        }
        // Maps flattens the view as you leave the surface; fully top-down
        // (the fixed centered globe) by ~8,000 km.
        let max_pitch = 1.45 * ((8_000_000.0 - self.alt_m) / 6_000_000.0).clamp(0.0, 1.0);
        self.pitch = self.pitch.min(max_pitch);
    }

    /// ECEF intersection of the pixel ray with the WGS84 ellipsoid inflated
    /// by `h` meters (pass the local terrain height to anchor gestures to
    /// the *visible* surface). Must be the true ellipsoid, not the mean
    /// sphere: at mid-latitudes the ellipsoid lies ~1.5 km below the mean
    /// sphere, so a sphere test *misses entirely* from street-level
    /// altitudes — which silently disabled anchored gestures there.
    pub fn ray_ground_point(&self, nx: f64, ny: f64, aspect: f64, h: f64) -> Option<DVec3> {
        let eye = self.eye_ecef();
        let th = (self.fov_y * 0.5).tan();
        let view_dir = DVec3::new(nx * th * aspect, ny * th, -1.0);
        // view_rotation maps world→view (rotation only); transpose inverts.
        let m3 = DMat3::from_mat4(self.view_rotation()).transpose();
        let dir = (m3 * view_dir).normalize();
        // Scale space so the inflated ellipsoid becomes the unit sphere.
        let s = DVec3::new(
            1.0 / (ellipsoid::WGS84_A + h),
            1.0 / (ellipsoid::WGS84_A + h),
            1.0 / (ellipsoid::WGS84_B + h),
        );
        let o = eye * s;
        let d = dir * s;
        let a2 = d.dot(d);
        let b2 = o.dot(d);
        let c2 = o.dot(o) - 1.0;
        let disc = b2 * b2 - a2 * c2;
        if disc < 0.0 {
            return None;
        }
        let t = (-b2 - disc.sqrt()) / a2;
        if t <= 0.0 {
            return None;
        }
        Some(eye + dir * t)
    }

    /// Fallback tilt when there is no ground anchor (cursor off-planet).
    pub fn tilt(&mut self, dx_px: f64, dy_px: f64) {
        self.heading -= dx_px * 0.004;
        self.pitch = (self.pitch - dy_px * 0.004).clamp(0.0, 1.45);
    }

    /// Maps-style orbit: a **rigid rotation of the whole camera pose about
    /// the grabbed surface point** — horizontal drag revolves about the
    /// anchor's up axis, vertical drag tilts about the camera's right axis
    /// through the anchor. Because eye and orientation rotate together, the
    /// anchor's position in camera space is unchanged, so the grabbed point
    /// stays exactly on its pixel (wherever it was grabbed — not snapped to
    /// screen center), and a zero-length drag is a strict no-op.
    pub fn orbit_anchor(&mut self, anchor: DVec3, dx_px: f64, dy_px: f64) {
        use glam::DQuat;
        let eye = self.eye_ecef();
        let fwd = self.forward_ecef();
        let up_a = ellipsoid::geodetic_normal(anchor);

        // Horizontal: revolve about the anchor's local vertical. Dragging
        // right moves the camera eastward around the point.
        let r_az = DQuat::from_axis_angle(up_a, -dx_px * 0.005);
        // Vertical: tilt about the camera's right axis placed at the anchor
        // (drag up → toward the horizon). Reject the component if it would
        // push past zenith or below grazing elevation.
        let (east, north, up) = self.enu();
        let horiz = north * self.heading.cos() + east * self.heading.sin();
        let (sin_p, cos_p) = self.pitch.sin_cos();
        let cam_up = up * sin_p + horiz * cos_p;
        let right = fwd.cross(cam_up).normalize_or(east);
        if dy_px > 0.0 && self.pitch < 1e-6 && dx_px == 0.0 {
            // A held downward drag at nadir is a strict no-op. Avoid even a
            // harmless pose round-trip here: at the singularity its tiny
            // floating-point heading error can accumulate across events.
            return;
        }
        let rel = eye - anchor;
        let elevation_angle = -dy_px * 0.005;
        let pose_at = |fraction: f64| {
            let r_el = DQuat::from_axis_angle(right, elevation_angle * fraction);
            let rot = r_az * r_el;
            let rel_new = rot * rel;
            let eye_new = anchor + rel_new;
            let fwd_new = rot * fwd;
            let right_new = rot * right;
            let up_new = ellipsoid::geodetic_normal(eye_new);

            // Unlike acos(-fwd·up), this retains which side of nadir the
            // rigid orbit reached. Crossing nadir would otherwise be
            // re-encoded as a positive pitch with heading flipped by pi,
            // making a held drag bounce back and forth across top-down.
            let horizontal = up_new.cross(right_new).normalize_or(cam_up);
            let signed_pitch = fwd_new.dot(horizontal).atan2(-fwd_new.dot(up_new));
            let anchor_angle = up_a.dot(rel_new.normalize_or(up_a)).clamp(-1.0, 1.0).acos();
            (rot, rel_new, signed_pitch, anchor_angle)
        };
        let valid = |signed_pitch: f64, anchor_angle: f64| {
            (0.0..=1.45).contains(&signed_pitch) && anchor_angle <= 1.45
        };

        let requested = pose_at(1.0);
        let (rot, rel_new) = if valid(requested.2, requested.3) {
            (requested.0, requested.1)
        } else {
            // Stop at the first pitch/anchor limit instead of dropping the
            // whole pointer delta. The rigid partial rotation still keeps
            // the grabbed surface point pinned to exactly the same pixel.
            let mut lo = 0.0;
            let mut hi = 1.0;
            for _ in 0..52 {
                let mid = (lo + hi) * 0.5;
                let candidate = pose_at(mid);
                if valid(candidate.2, candidate.3) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let clamped = pose_at(lo);
            (clamped.0, clamped.1)
        };
        self.set_pose_oriented(anchor + rel_new, rot * fwd, rot * cam_up);
    }

    /// Place the orbit camera at `eye`, retaining camera-up as a stable
    /// heading reference when `fwd` reaches the nadir singularity.
    fn set_pose_oriented(&mut self, eye: DVec3, fwd: DVec3, view_up: DVec3) {
        self.set_pose_clamped(eye, fwd, Some(view_up), 5.0, 1.45);
    }

    /// Play-mode variant of `set_pose`: the follow cam may hug the ground
    /// (no explore-mode altitude floor) and look up past the horizon at a
    /// drone overhead.
    pub fn set_pose_free(&mut self, eye: DVec3, fwd: DVec3) {
        self.set_pose_clamped(eye, fwd, None, -450.0, 3.0);
    }

    fn set_pose_clamped(
        &mut self,
        eye: DVec3,
        fwd: DVec3,
        view_up: Option<DVec3>,
        min_alt: f64,
        max_pitch: f64,
    ) {
        let (lat, lon, alt) = ellipsoid::ecef_to_geodetic(eye);
        self.lat = lat.clamp(-1.55, 1.55);
        self.lon = wrap_pi(lon);
        self.alt_m = alt.max(min_alt);
        let (east, north, up) = self.enu();
        let cos_p = (-fwd.dot(up)).clamp(-1.0, 1.0);
        self.pitch = cos_p.acos().clamp(0.0, max_pitch);
        let horiz = view_up
            .map(|view_up| view_up - up * view_up.dot(up))
            .unwrap_or_else(|| fwd + up * cos_p);
        if horiz.length() > 1e-9 {
            let h = horiz.normalize();
            self.heading = h.dot(east).atan2(h.dot(north));
        }
    }

    /// Google-Maps globe behavior: the globe re-norths itself as you zoom
    /// out — you can never leave Australia upside-down at the top.
    pub fn realign_north(&mut self, dt: f64) {
        if self.alt_m > 4_000_000.0 {
            let strength = ((self.alt_m - 4_000_000.0) / 4_000_000.0).min(1.0);
            let k = (dt * 3.0 * strength).min(1.0);
            self.heading = wrap_pi(self.heading) * (1.0 - k);
        }
    }

    /// Hard north lock: ease heading to north at any altitude (active until
    /// the user rotates deliberately — Maps keeps the map north-up through
    /// a whole orbit-to-street dive by default).
    pub fn realign_north_always(&mut self, dt: f64) {
        let k = (dt * 5.0).min(1.0);
        self.heading = wrap_pi(self.heading) * (1.0 - k);
    }
}

/// Great-circle camera flight with a distance-scaled altitude arc and a gentle
/// arrival tilt. The app cancels it on manual camera input.
pub struct FlyTo {
    from: (f64, f64, f64, f64, f64), // lat, lon, alt, heading, pitch
    to: (f64, f64, f64),
    /// Peak of the altitude bump above the endpoints' lerp.
    bump_m: f64,
    end_pitch: f64,
    dur_s: f64,
    t_s: f64,
}

impl FlyTo {
    pub fn new(cam: &Camera, lat_deg: f64, lon_deg: f64, end_alt_m: f64) -> Self {
        // Arrive tilted toward the horizon at street scale, top-down when
        // the target is a whole region.
        let end_pitch = if end_alt_m < 3000.0 { 0.9 } else { 0.0 };
        let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
        let a = ellipsoid::geodetic_to_ecef(cam.lat, cam.lon, 0.0).normalize();
        let b = ellipsoid::geodetic_to_ecef(lat, lon, 0.0).normalize();
        let dist = a.dot(b).clamp(-1.0, 1.0).acos() * MEAN_RADIUS;
        // Rise high enough to see where you're going, never for short hops.
        let bump = (dist * 0.55).clamp(0.0, 13_000_000.0);
        let dur = (1.6 + (dist / 3_000_000.0).sqrt()).clamp(1.6, 4.5);
        FlyTo {
            from: (cam.lat, cam.lon, cam.alt_m, cam.heading, cam.pitch),
            to: (lat, lon, end_alt_m),
            bump_m: bump,
            end_pitch,
            dur_s: dur,
            t_s: 0.0,
        }
    }

    /// ECEF position of the destination pose (for streaming the arrival
    /// view while the flight is still in the air).
    pub fn end_pos_ecef(&self) -> DVec3 {
        ellipsoid::geodetic_to_ecef(self.to.0, self.to.1, self.to.2)
    }

    /// Logical duration of the same smooth flight used by interactive
    /// navigation. Capture tooling uses this to sample the path at a fixed
    /// cadence without tying the result to wall-clock or render speed.
    pub fn duration_seconds(&self) -> f64 {
        self.dur_s
    }

    /// Advance the animation; returns false once finished.
    pub fn update(&mut self, cam: &mut Camera, dt: f64) -> bool {
        self.t_s += dt;
        let t = if self.t_s + 1.0e-9 >= self.dur_s {
            1.0
        } else {
            self.t_s / self.dur_s
        };
        let s = t * t * (3.0 - 2.0 * t); // smoothstep
        // Great-circle position.
        let a = ellipsoid::geodetic_to_ecef(self.from.0, self.from.1, 0.0).normalize();
        let b = ellipsoid::geodetic_to_ecef(self.to.0, self.to.1, 0.0).normalize();
        let omega = a.dot(b).clamp(-1.0, 1.0).acos();
        let p = if omega < 1e-9 {
            a
        } else {
            (a * ((1.0 - s) * omega).sin() + b * (s * omega).sin()) / omega.sin()
        };
        let (lat, lon, _) = ellipsoid::ecef_to_geodetic(p * MEAN_RADIUS);
        cam.lat = lat;
        cam.lon = lon;
        // Altitude: endpoint lerp plus a sine arc, clamped by the zoom stop.
        let base = self.from.2 + (self.to.2 - self.from.2) * s;
        cam.alt_m =
            (base + self.bump_m * (std::f64::consts::PI * s).sin()).min(cam.max_zoom_out_alt());
        // Heading unwinds to north; pitch flattens mid-flight, then eases to
        // the arrival tilt over the last quarter.
        cam.heading = wrap_pi(self.from.3) * (1.0 - s);
        let arrive = ((t - 0.75) / 0.25).clamp(0.0, 1.0);
        let arrive = arrive * arrive * (3.0 - 2.0 * arrive);
        cam.pitch = self.from.4 * (1.0 - s) * (1.0 - arrive) + self.end_pitch * arrive;
        if t >= 1.0 {
            // Avoid leaving a tiny floating-point remainder at the end of a
            // fixed-step capture (and at the end of interactive navigation).
            cam.lat = self.to.0;
            cam.lon = self.to.1;
            cam.alt_m = self.to.2;
            cam.heading = 0.0;
            cam.pitch = self.end_pitch;
        }
        t < 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn city_camera() -> Camera {
        Camera {
            lat: 40.75f64.to_radians(),
            lon: (-73.98f64).to_radians(),
            alt_m: 900.0,
            heading: 0.4,
            pitch: 0.9,
            fov_y: 55f64.to_radians(),
            aspect: 1.6,
        }
    }

    /// Project a world point to NDC through the camera (matches the
    /// renderer's perspective up to the depth term).
    fn ndc_of(cam: &Camera, p: DVec3) -> (f64, f64) {
        let rel = p - cam.eye_ecef();
        let v = DMat3::from_mat4(cam.view_rotation()) * rel;
        assert!(v.z < 0.0, "point behind camera");
        let th = (cam.fov_y * 0.5).tan();
        (v.x / (-v.z) / (th * cam.aspect), v.y / (-v.z) / th)
    }

    #[test]
    fn orbit_zero_drag_is_identity() {
        let mut cam = city_camera();
        let anchor = cam
            .ray_ground_point(0.35, -0.2, cam.aspect, 0.0)
            .expect("cursor on ground");
        let before = (cam.lat, cam.lon, cam.alt_m, cam.heading, cam.pitch);
        cam.orbit_anchor(anchor, 0.0, 0.0);
        let after = (cam.lat, cam.lon, cam.alt_m, cam.heading, cam.pitch);
        assert!((before.0 - after.0).abs() < 1e-12, "lat moved");
        assert!((before.1 - after.1).abs() < 1e-12, "lon moved");
        assert!((before.2 - after.2).abs() < 1e-6, "alt moved");
        assert!((before.3 - after.3).abs() < 1e-9, "heading moved");
        assert!((before.4 - after.4).abs() < 1e-9, "pitch moved");
    }

    #[test]
    fn orbit_pins_anchor_to_its_pixel() {
        let mut cam = city_camera();
        // Exercise an off-center anchor.
        let ndc0 = (0.42, -0.31);
        let anchor = cam
            .ray_ground_point(ndc0.0, ndc0.1, cam.aspect, 0.0)
            .expect("cursor on ground");
        for (dx, dy) in [
            (18.0, 0.0),
            (-7.0, 12.0),
            (25.0, -9.0),
            (0.0, 20.0),
            (-40.0, -15.0),
        ] {
            cam.orbit_anchor(anchor, dx, dy);
            let ndc = ndc_of(&cam, anchor);
            assert!(
                (ndc.0 - ndc0.0).abs() < 1e-4 && (ndc.1 - ndc0.1).abs() < 1e-4,
                "anchor drifted to {ndc:?} after drag ({dx},{dy})"
            );
        }
    }

    #[test]
    fn zoom_pins_anchor_and_preserves_city_orientation() {
        let mut cam = city_camera();
        let ndc0 = (-0.5, 0.22);
        let anchor = cam
            .ray_ground_point(ndc0.0, ndc0.1, cam.aspect, 0.0)
            .expect("cursor on ground");
        let d0 = (cam.eye_ecef() - anchor).length();
        let (h0, p0) = (cam.heading, cam.pitch);
        for _ in 0..5 {
            cam.zoom_towards(1.0, Some(anchor));
            let ndc = ndc_of(&cam, anchor);
            assert!(
                (ndc.0 - ndc0.0).abs() < 1e-3 && (ndc.1 - ndc0.1).abs() < 1e-3,
                "anchor drifted to {ndc:?} while zooming"
            );
            assert_eq!(cam.heading, h0);
            assert_eq!(cam.pitch, p0);
        }
        let d1 = (cam.eye_ecef() - anchor).length();
        assert!(d1 < d0 * 0.6, "zoom must approach the anchor: {d0} -> {d1}");
        cam.zoom_towards(-2.0, Some(anchor));
        let ndc = ndc_of(&cam, anchor);
        assert!((ndc.0 - ndc0.0).abs() < 1e-3 && (ndc.1 - ndc0.1).abs() < 1e-3);
    }

    #[test]
    fn globe_zoom_keeps_north_up_and_pins_anchor() {
        // Heading and pitch remain fixed through an off-center
        // orbit-to-regional zoom.
        let mut cam = Camera {
            lat: 30f64.to_radians(),
            lon: 10f64.to_radians(),
            alt_m: 12_000_000.0,
            heading: 0.0,
            pitch: 0.0,
            fov_y: 55f64.to_radians(),
            aspect: 1.6,
        };
        let ndc0 = (0.3, 0.2);
        for step in 0..24 {
            let anchor = cam
                .ray_ground_point(ndc0.0, ndc0.1, cam.aspect, 0.0)
                .expect("cursor on globe");
            cam.zoom_towards(1.0, Some(anchor));
            assert_eq!(cam.heading, 0.0, "heading rotated at step {step}");
            assert_eq!(cam.pitch, 0.0, "pitch tilted at step {step}");
            let ndc = ndc_of(&cam, anchor);
            assert!(
                (ndc.0 - ndc0.0).abs() < 5e-3 && (ndc.1 - ndc0.1).abs() < 5e-3,
                "anchor at {ndc:?} after step {step}"
            );
        }
        assert!(
            cam.alt_m < 700_000.0,
            "24 notches should reach regional: {}",
            cam.alt_m
        );
    }

    #[test]
    fn orbit_survives_extreme_drags() {
        let mut cam = city_camera();
        let anchor = cam
            .ray_ground_point(0.0, -0.4, cam.aspect, 0.0)
            .expect("cursor on ground");
        // Hammer the tilt limits both ways; pose must stay finite and the
        // eye must stay above the anchor's horizon plane.
        for _ in 0..200 {
            cam.orbit_anchor(anchor, 3.0, 40.0);
        }
        for _ in 0..200 {
            cam.orbit_anchor(anchor, -5.0, -40.0);
        }
        assert!(cam.lat.is_finite() && cam.lon.is_finite() && cam.alt_m.is_finite());
        assert!(cam.pitch >= 0.0 && cam.pitch <= 1.45);
        let up_a = ellipsoid::geodetic_normal(anchor);
        let theta = up_a
            .dot((cam.eye_ecef() - anchor).normalize())
            .clamp(-1.0, 1.0)
            .acos();
        assert!(theta <= 1.46, "eye slid below grazing elevation: {theta}");
    }

    #[test]
    fn orbit_holds_at_nadir_while_drag_continues() {
        let mut cam = city_camera();
        let ndc0 = (0.32, -0.24);
        let anchor = cam
            .ray_ground_point(ndc0.0, ndc0.1, cam.aspect, 0.0)
            .expect("cursor on ground");

        let mut reached_nadir = false;
        for step in 0..200 {
            let dx = if reached_nadir {
                if step % 2 == 0 { 0.1 } else { -0.1 }
            } else {
                0.0
            };
            cam.orbit_anchor(anchor, dx, 4.0);
            let ndc = ndc_of(&cam, anchor);
            assert!(
                (ndc.0 - ndc0.0).abs() < 5e-4 && (ndc.1 - ndc0.1).abs() < 5e-4,
                "anchor drifted to {ndc:?} at step {step}"
            );
            if reached_nadir {
                assert!(
                    cam.pitch < 1e-6,
                    "pitch bounced away from nadir at step {step}: {}",
                    cam.pitch
                );
            } else if cam.pitch < 1e-6 {
                reached_nadir = true;
            }
        }
        assert!(reached_nadir, "downward drag never reached nadir");
    }

    #[test]
    fn north_lock_realigns_at_low_altitude() {
        let mut cam = city_camera();
        cam.heading = 1.0;
        for _ in 0..300 {
            cam.realign_north_always(1.0 / 60.0);
        }
        assert!(cam.heading.abs() < 0.01, "heading should ease to north");
        // The globe-only variant must NOT touch heading at 900 m.
        let mut cam2 = city_camera();
        cam2.heading = 1.0;
        cam2.realign_north(1.0 / 60.0);
        assert!((cam2.heading - 1.0).abs() < 1e-9);
    }

    #[test]
    fn mouse_drag_sensitivity_depends_on_ground_clearance() {
        let mut sea_level = city_camera();
        sea_level.alt_m = 500.0;
        let mut high_terrain = sea_level.clone();
        high_terrain.alt_m = 1_500.0;
        sea_level.drag_over_ground(24.0, -11.0, 900.0, 0.0);
        high_terrain.drag_over_ground(24.0, -11.0, 900.0, 1_000.0);
        assert!((sea_level.lat - high_terrain.lat).abs() < 1.0e-12);
        assert!((sea_level.lon - high_terrain.lon).abs() < 1.0e-12);
    }
}
