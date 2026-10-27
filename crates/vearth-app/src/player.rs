//! Third-person vehicle controls and terrain collision for play mode.
//!
//! Arcade flight with light inertia — stick-to-velocity with a little
//! drift, auto-hover on release, hold-to-boost — over solid AABB voxel
//! collision (bump-and-slide, no damage state). All positions f64 ECEF;
//! the local east/north/up basis is recomputed every frame, so gravity/up
//! realignment is continuous and free. Works anywhere on Earth by
//! construction: a flyer needs no navmesh or ground-type logic.

use crate::streaming::ResidentStore;
use crate::vehicles::VehicleKind;
use crate::weather::{WindInfluence, WindSample};
use glam::{DMat4, DVec3};
use std::collections::HashMap;
use vearth_core::cell::CellId;
use vearth_core::ellipsoid;
use vearth_core::frame::ChunkFrame;

/// Held-key state for the active vehicle (and its follow cam).
#[derive(Default, Clone, Copy)]
pub struct PlayKeys {
    pub fwd: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub boost: bool,
    pub rot_left: bool,
    pub rot_right: bool,
    pub tilt_up: bool,
    pub tilt_down: bool,
}

/// Follow-cam pitch limits: near-overhead to looking up at the drone.
const CAM_PITCH_RANGE: (f64, f64) = (0.25, 2.75);
/// Chase-cam auto-follow: the camera swings in behind the drone's heading
/// while flying. Deliberately slower than the drone's own heading ease
/// (0.22 s) so turns read as arcs, not rigid rotation; tighter under boost.
const FOLLOW_TAU: f64 = 0.75;
const FOLLOW_TAU_BOOST: f64 = 0.45;
/// No auto-follow below this horizontal speed — a hovering drone can be
/// inspected from any side without the camera creeping.
const FOLLOW_MIN_SPEED: f64 = 3.0;
/// How long a manual orbit (drag or Q/E) overrides auto-follow.
const ORBIT_HOLD_S: f64 = 1.5;
const CRUISE_MS: f64 = 24.0;
const BOOST_MS: f64 = 76.0;
const VERTICAL_MS: f64 = 9.0;
const VERTICAL_BOOST_MS: f64 = 22.0;
/// Preserve the Kestrel's precise near-ground lift, then give it enough
/// vertical authority to cross the upper atmosphere without a long wait.
const KESTREL_VERTICAL_ALT_START_M: f64 = 2_500.0;
const KESTREL_VERTICAL_ALT_FULL_M: f64 = 25_000.0;
const KESTREL_MAX_VERTICAL_MS: f64 = 180.0;
const KESTREL_MAX_VERTICAL_BOOST_MS: f64 = 600.0;
const ACCEL_TAU: f64 = 0.30;
const SPARROW_CRUISE_MS: f64 = 30.0;
const SPARROW_BOOST_MS: f64 = 84.0;
const SPARROW_ACCEL_TAU: f64 = 0.65;
pub(crate) const SPARROW_HOVER_CLEARANCE_M: f64 = 3.0;
const SPARROW_JUMP_MS: f64 = 14.0;
const SPARROW_MAX_RISE_MS: f64 = 14.0;
const SPARROW_MAX_FALL_MS: f64 = 30.0;
const SPARROW_SPRING_STIFFNESS: f64 = 18.0;
const SPARROW_DESCENT_DAMPING: f64 = 0.65;
const SPARROW_SETTLE_DAMPING: f64 = 8.5;
const SPARROW_MAX_VERTICAL_ACCEL: f64 = 100.0;
/// A hoverbike can pivot briskly at low speed, with enough authority left at
/// cruise and boost speed to hold a useful arc.
const SPARROW_IDLE_STEER_RATE: f64 = 68.0_f64.to_radians();
const SPARROW_CRUISE_STEER_RATE: f64 = 90.0_f64.to_radians();
const SPARROW_BOOST_STEER_RATE: f64 = 70.0_f64.to_radians();
/// Blend out of the standing pivot quickly, then hold the requested moving
/// turn rate even when lateral grip scrubs a little speed in a hard corner.
const SPARROW_MOVING_STEER_SPEED_M: f64 = 3.0;
const SPARROW_STEER_INPUT_TAU: f64 = 0.09;
/// Side-slip is removed faster than longitudinal speed changes, making the
/// velocity describe the same arc as the bike without defeating acceleration.
const SPARROW_LATERAL_GRIP_TAU: f64 = 0.14;
const SPARROW_FORWARD_PITCH_MAX: f64 = 5.0_f64.to_radians();
/// Soft road guidance: nearby, directionally aligned roads influence the
/// bike, but full steering input suppresses the influence immediately so the
/// rider can leave the road at any point.
const SPARROW_ROAD_CAPTURE_MARGIN_M: f64 = 7.5;
const SPARROW_ROAD_ENGAGE_TAU: f64 = 0.10;
const SPARROW_ROAD_RELEASE_TAU: f64 = 0.06;
const SPARROW_ROAD_STEER_RATE: f64 = 110.0_f64.to_radians();
const SPARROW_ROAD_FULL_HEADING_ERROR: f64 = 22.0_f64.to_radians();
const SPARROW_ROAD_DIRECTION_BLEND: f64 = 0.90;
const SPARROW_ROAD_CENTER_GAIN: f64 = 4.5;
const SPARROW_ROAD_MAX_CENTER_MS: f64 = 14.0;
/// Begin blending toward the next authored segment far enough ahead for the
/// current ground speed. The cap prevents a distant corner from influencing
/// more than a bounded stretch of road.
const SPARROW_ROAD_TURN_LOOKAHEAD_BASE_M: f64 = 8.0;
const SPARROW_ROAD_TURN_LOOKAHEAD_S: f64 = 0.65;
const SPARROW_ROAD_TURN_LOOKAHEAD_MAX_M: f64 = 65.0;
const PHASE_CRUISE_MS: f64 = 320.0;
const PHASE_BOOST_MS: f64 = 12_000.0;
/// The Phase Manta keeps the shared phase-drive ceiling in the lower
/// atmosphere, then progressively unlocks its full envelope above it.
const PHASE_MANTA_BOOST_ALT_START_M: f64 = 2_500.0;
const PHASE_MANTA_BOOST_ALT_FULL_M: f64 = 50_000.0;
const PHASE_MANTA_MAX_BOOST_MS: f64 = 100_000.0;
const PHASE_VERTICAL_MS: f64 = 120.0;
const PHASE_VERTICAL_BOOST_MS: f64 = 1_200.0;
const PHASE_MANTA_MAX_VERTICAL_MS: f64 = 1_200.0;
const PHASE_MANTA_MAX_VERTICAL_BOOST_MS: f64 = 12_000.0;
const PHASE_ACCEL_TAU: f64 = 0.85;
const PHASE_MIN_CLEARANCE_M: f64 = 2.0;
// Horizontal half-widths bound each neutral hull's radius in metres: the
// collision box stays in ENU while the authored model can yaw freely.
// Cosmetic pitch and bank remain render-only.
const COLLISION_HALF_H: f64 = 2.10;
const COLLISION_HALF_V: f64 = 0.76;
const SPARROW_COLLISION_HALF_H: f64 = 1.65 * crate::vehicles::SPARROW_SCALE as f64;
const SPARROW_COLLISION_HALF_V: f64 = 0.72 * crate::vehicles::SPARROW_SCALE as f64;
const TURN_BANK_GAIN_S: f64 = 0.16;
const PHASE_TURN_BANK_MAX: f64 = 0.24;
const KESTREL_TURN_BANK_MAX: f64 = 0.16;
const SPARROW_TURN_BANK_MAX: f64 = 0.34;
const PRODUCTION_PHASE_BOOST_PITCH_MAX: f64 = 10.0 * std::f64::consts::PI / 180.0;
/// Hover strafing banks the Kestrel while preserving the nose direction.
/// A bounded yaw cue keeps steering legible from the chase camera.
const KESTREL_MAX_TRAVEL_YAW: f64 = 25.0 * std::f64::consts::PI / 180.0;

fn wrap_pi(a: f64) -> f64 {
    let mut a = a % std::f64::consts::TAU;
    if a > std::f64::consts::PI {
        a -= std::f64::consts::TAU;
    } else if a < -std::f64::consts::PI {
        a += std::f64::consts::TAU;
    }
    a
}

fn body_heading_target(travel_heading: f64, camera_heading: f64) -> f64 {
    let relative = wrap_pi(travel_heading - camera_heading)
        .clamp(-KESTREL_MAX_TRAVEL_YAW, KESTREL_MAX_TRAVEL_YAW);
    wrap_pi(camera_heading + relative)
}

fn turn_bank_target(yaw_rate: f64, vehicle: VehicleKind) -> f64 {
    let limit = match vehicle {
        VehicleKind::SplitArrow | VehicleKind::PhaseManta => PHASE_TURN_BANK_MAX,
        VehicleKind::Sparrow => SPARROW_TURN_BANK_MAX,
        VehicleKind::Kestrel => KESTREL_TURN_BANK_MAX,
    };
    (yaw_rate * TURN_BANK_GAIN_S).clamp(-limit, limit)
}

fn altitude_ease(altitude_m: f64, start_m: f64, full_m: f64) -> f64 {
    let progress = ((altitude_m - start_m) / (full_m - start_m)).clamp(0.0, 1.0);
    progress * progress * (3.0 - 2.0 * progress)
}

fn altitude_scaled_speed(
    altitude_m: f64,
    start_m: f64,
    full_m: f64,
    low_speed_ms: f64,
    high_speed_ms: f64,
) -> f64 {
    low_speed_ms + (high_speed_ms - low_speed_ms) * altitude_ease(altitude_m, start_m, full_m)
}

fn phase_manta_boost_speed(altitude_m: f64) -> f64 {
    // Smoothstep keeps both ends flat: crossing 2.5 km does not kick the
    // craft, and arriving at the true cap does not produce a hard shoulder.
    let eased = altitude_ease(
        altitude_m,
        PHASE_MANTA_BOOST_ALT_START_M,
        PHASE_MANTA_BOOST_ALT_FULL_M,
    );
    PHASE_BOOST_MS + (PHASE_MANTA_MAX_BOOST_MS - PHASE_BOOST_MS) * eased
}

fn kestrel_vertical_speeds(altitude_m: f64) -> (f64, f64) {
    (
        altitude_scaled_speed(
            altitude_m,
            KESTREL_VERTICAL_ALT_START_M,
            KESTREL_VERTICAL_ALT_FULL_M,
            VERTICAL_MS,
            KESTREL_MAX_VERTICAL_MS,
        ),
        altitude_scaled_speed(
            altitude_m,
            KESTREL_VERTICAL_ALT_START_M,
            KESTREL_VERTICAL_ALT_FULL_M,
            VERTICAL_BOOST_MS,
            KESTREL_MAX_VERTICAL_BOOST_MS,
        ),
    )
}

fn phase_manta_vertical_speeds(altitude_m: f64) -> (f64, f64) {
    (
        altitude_scaled_speed(
            altitude_m,
            PHASE_MANTA_BOOST_ALT_START_M,
            PHASE_MANTA_BOOST_ALT_FULL_M,
            PHASE_VERTICAL_MS,
            PHASE_MANTA_MAX_VERTICAL_MS,
        ),
        altitude_scaled_speed(
            altitude_m,
            PHASE_MANTA_BOOST_ALT_START_M,
            PHASE_MANTA_BOOST_ALT_FULL_M,
            PHASE_VERTICAL_BOOST_MS,
            PHASE_MANTA_MAX_VERTICAL_BOOST_MS,
        ),
    )
}

/// Local east/north/up unit vectors at an ECEF position.
fn enu_at(p: DVec3) -> (DVec3, DVec3, DVec3) {
    let (lat, lon, _) = ellipsoid::ecef_to_geodetic(p);
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    let east = DVec3::new(-sin_lon, cos_lon, 0.0);
    let north = DVec3::new(-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat);
    let up = DVec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat);
    (east, north, up)
}

pub struct Player {
    /// Active aircraft. Phase craft are non-colliding.
    pub vehicle: VehicleKind,
    /// ECEF drone center.
    pub pos: DVec3,
    /// ECEF velocity (m/s).
    pub vel: DVec3,
    /// Pilot-controlled air-relative velocity. Ground velocity additionally
    /// contains the authoritative weather wind and assisted correction.
    pub air_vel: DVec3,
    /// Drone body yaw in the local frame (0 = north, +east), eased toward
    /// the direction of travel.
    pub heading: f64,
    /// Follow-cam orbit around the drone. Pitch follows the explore-camera
    /// convention (0 = looking straight down, π/2 = level).
    pub cam_heading: f64,
    pub cam_pitch: f64,
    /// Boom target length (scroll-adjustable) and its sprung current value.
    pub boom_len: f64,
    boom_cur: f64,
    /// Cosmetic lean angles (radians): nose-down into forward motion,
    /// banking into strafes. Render-only.
    pub lean_f: f64,
    pub lean_r: f64,
    /// Booster flame intensity 0..1, eased from the boost key.
    pub flame: f64,
    /// Smoothed Sparrow steering input. This keeps the useful turn rate from
    /// becoming a one-frame snap when A/D is pressed or released.
    sparrow_steer: f64,
    /// Smoothed strength of the optional road-centering tendency. Candidate
    /// geometry is queried afresh from resident cells; only the blend is
    /// stateful, which prevents capture flicker at cell and road boundaries.
    sparrow_road_assist: f64,
    /// Seconds of manual-orbit override left before the chase cam resumes
    /// auto-following the drone's heading.
    orbit_hold: f64,
    /// Auto-hover/crab correction applied in Assisted mode.
    wind_compensation: DVec3,
    /// Rising edge latch: Space is a jump impulse on the Sparrow rather than
    /// continuous drone-style vertical thrust.
    jump_held: bool,
    /// A descent uses one underdamped pass through the hover target, then
    /// switches to critical damping after the first upward rebound.
    hover_bounce_armed: bool,
    hover_rebounded: bool,
    /// Chunk-frame memo for the many voxel samples per frame.
    frames: HashMap<CellId, ChunkFrame>,
}

impl Player {
    /// Spawn hovering at `pos`, facing `heading` (camera behind).
    pub fn spawn(pos: DVec3, heading: f64) -> Self {
        Self::spawn_as(pos, heading, VehicleKind::Kestrel)
    }

    pub fn spawn_as(pos: DVec3, heading: f64, vehicle: VehicleKind) -> Self {
        let boom_len = vehicle.chase_boom_m();
        Player {
            vehicle,
            pos,
            vel: DVec3::ZERO,
            air_vel: DVec3::ZERO,
            heading,
            cam_heading: heading,
            cam_pitch: 1.15,
            boom_len,
            boom_cur: (boom_len * 0.5).max(4.0), // eases out on entry
            lean_f: 0.0,
            lean_r: 0.0,
            flame: 0.0,
            sparrow_steer: 0.0,
            sparrow_road_assist: 0.0,
            orbit_hold: 0.0,
            wind_compensation: DVec3::ZERO,
            jump_held: false,
            hover_bounce_armed: false,
            hover_rebounded: false,
            frames: HashMap::new(),
        }
    }

    /// Select a playable aircraft directly, applying the same safe transition
    /// as cycling through the vehicle list.
    pub fn select_vehicle(&mut self, vehicle: VehicleKind) {
        self.vehicle = vehicle;
        self.boom_len = self.vehicle.chase_boom_m();
        self.boom_cur = self.boom_cur.min(self.boom_len);
        self.vel = DVec3::ZERO;
        self.air_vel = DVec3::ZERO;
        self.wind_compensation = DVec3::ZERO;
        self.lean_f = 0.0;
        self.lean_r = 0.0;
        self.flame = 0.0;
        self.sparrow_steer = 0.0;
        self.sparrow_road_assist = 0.0;
        self.jump_held = false;
        self.hover_bounce_armed = false;
        self.hover_rebounded = false;
        self.frames.clear();
    }

    pub fn is_phased(&self) -> bool {
        self.vehicle.is_phased()
    }

    pub fn speed_ms(&self) -> f64 {
        self.vel.length()
    }

    pub fn airspeed_ms(&self) -> f64 {
        self.air_vel.length()
    }

    /// Mouse orbit of the follow cam (pixels). Holds off auto-follow for a
    /// grace period so a deliberate look-around isn't immediately undone.
    pub fn orbit(&mut self, dx_px: f64, dy_px: f64) {
        self.cam_heading += dx_px * 0.005;
        self.cam_pitch =
            (self.cam_pitch - dy_px * 0.005).clamp(CAM_PITCH_RANGE.0, CAM_PITCH_RANGE.1);
        self.orbit_hold = ORBIT_HOLD_S;
    }

    /// Scroll adjusts the boom length.
    pub fn zoom_boom(&mut self, scroll: f64) {
        self.boom_len = (self.boom_len * (-scroll * 0.15).exp()).clamp(3.5, 40.0);
    }

    pub fn update(&mut self, dt: f64, keys: &PlayKeys, index: &ResidentStore) {
        // Callers without weather input provide ground velocity through `vel`;
        // treat it as controlled air velocity.
        self.air_vel = self.vel;
        self.update_with_weather(dt, keys, index, WindSample::default(), WindInfluence::Off);
    }

    /// Advance air-relative controls and transform them through the shared
    /// physical wind field into ground-relative collision movement.
    pub fn update_with_weather(
        &mut self,
        dt: f64,
        keys: &PlayKeys,
        index: &ResidentStore,
        wind: WindSample,
        influence: WindInfluence,
    ) {
        // The frame memo only ever grows on long flights; keep it honest.
        if self.frames.len() > 512 {
            self.frames.clear();
        }
        // Keyboard camera orbit (mouse drag comes through `orbit`); like a
        // drag, it overrides auto-follow for the grace period.
        self.orbit_hold = (self.orbit_hold - dt).max(0.0);
        if keys.rot_left {
            self.cam_heading -= 1.6 * dt;
            self.orbit_hold = ORBIT_HOLD_S;
        }
        if keys.rot_right {
            self.cam_heading += 1.6 * dt;
            self.orbit_hold = ORBIT_HOLD_S;
        }
        if keys.tilt_up {
            self.cam_pitch = (self.cam_pitch + 1.2 * dt).min(CAM_PITCH_RANGE.1);
        }
        if keys.tilt_down {
            self.cam_pitch = (self.cam_pitch - 1.2 * dt).max(CAM_PITCH_RANGE.0);
        }

        let (east, north, up) = enu_at(self.pos);
        let heading_before_turn = self.heading;
        let mut sparrow_road_hit = None;
        let mut sparrow_road_effective_assist = 0.0;

        // Aircraft use camera-relative translation. The Sparrow instead has
        // bike controls: W/S drive along its nose and A/D steer the nose.
        // There is no lateral wish velocity, so a turn produces an arc
        // rather than a sideways hover-strafe.
        let h = if self.vehicle == VehicleKind::Sparrow {
            let horizontal_speed = (self.air_vel - up * self.air_vel.dot(up)).length();
            let speed_mix = (horizontal_speed / SPARROW_MOVING_STEER_SPEED_M).clamp(0.0, 1.0);
            let mut steer_rate = SPARROW_IDLE_STEER_RATE
                + (SPARROW_CRUISE_STEER_RATE - SPARROW_IDLE_STEER_RATE) * speed_mix;
            if keys.boost {
                steer_rate = SPARROW_BOOST_STEER_RATE;
            }
            let steer = (keys.right as i8 - keys.left as i8) as f64;
            let steer_k = 1.0 - (-dt / SPARROW_STEER_INPUT_TAU).exp();
            self.sparrow_steer += (steer - self.sparrow_steer) * steer_k;
            self.heading = wrap_pi(self.heading + self.sparrow_steer * steer_rate * dt);
            let throttle = (keys.fwd as i8 - keys.back as i8) as f64;
            let (sin_heading, cos_heading) = self.heading.sin_cos();
            let unassisted_nose = north * cos_heading + east * sin_heading;
            let turn_lookahead = (SPARROW_ROAD_TURN_LOOKAHEAD_BASE_M
                + horizontal_speed * SPARROW_ROAD_TURN_LOOKAHEAD_S)
                .min(SPARROW_ROAD_TURN_LOOKAHEAD_MAX_M);
            sparrow_road_hit = index.nearest_road_guide(
                self.pos,
                unassisted_nose,
                SPARROW_ROAD_CAPTURE_MARGIN_M,
                turn_lookahead,
            );
            let rider_override = (1.0 - steer.abs()).clamp(0.0, 1.0);
            let target_assist = sparrow_road_hit.map_or(0.0, |hit| {
                let capture = hit.width_m * 0.5 + SPARROW_ROAD_CAPTURE_MARGIN_M;
                let proximity = (1.0 - hit.distance_m / capture).clamp(0.0, 1.0);
                // Normalize cached or custom guide strengths so dirt roads
                // and trails engage as positively as paved roads.
                let surface_neutral_assist = hit.assist.max(0.9);
                proximity * proximity * (3.0 - 2.0 * proximity) * surface_neutral_assist
            });
            let assist_tau = if target_assist > self.sparrow_road_assist {
                SPARROW_ROAD_ENGAGE_TAU
            } else {
                SPARROW_ROAD_RELEASE_TAU
            };
            let assist_k = 1.0 - (-dt / assist_tau).exp();
            self.sparrow_road_assist += (target_assist - self.sparrow_road_assist) * assist_k;
            let motion_mix = (horizontal_speed / SPARROW_MOVING_STEER_SPEED_M)
                .clamp(0.0, 1.0)
                .max(throttle.abs());
            sparrow_road_effective_assist = self.sparrow_road_assist * rider_override * motion_mix;
            if let Some(hit) = sparrow_road_hit {
                let target_heading = hit.tangent.dot(east).atan2(hit.tangent.dot(north));
                let heading_error = wrap_pi(target_heading - self.heading);
                let auto_steer = (heading_error / SPARROW_ROAD_FULL_HEADING_ERROR).clamp(-1.0, 1.0)
                    * sparrow_road_effective_assist;
                self.heading = wrap_pi(self.heading + auto_steer * SPARROW_ROAD_STEER_RATE * dt);
            }
            let (sin_heading, cos_heading) = self.heading.sin_cos();
            (north * cos_heading + east * sin_heading) * throttle
        } else {
            let (sh, ch) = self.cam_heading.sin_cos();
            let fwd_h = north * ch + east * sh;
            let right_h = east * ch - north * sh;
            let mut wish = DVec3::ZERO;
            if keys.fwd {
                wish += fwd_h;
            }
            if keys.back {
                wish -= fwd_h;
            }
            if keys.right {
                wish += right_h;
            }
            if keys.left {
                wish -= right_h;
            }
            wish.normalize_or(DVec3::ZERO)
        };
        let vy = (keys.up as i8 - keys.down as i8) as f64;
        let (cruise, boost, vertical, vertical_boost, accel_tau) = match self.vehicle {
            VehicleKind::SplitArrow => (
                PHASE_CRUISE_MS,
                PHASE_BOOST_MS,
                PHASE_VERTICAL_MS,
                PHASE_VERTICAL_BOOST_MS,
                PHASE_ACCEL_TAU,
            ),
            VehicleKind::PhaseManta => {
                let altitude_m = ellipsoid::ecef_to_geodetic(self.pos).2;
                let (vertical, vertical_boost) = phase_manta_vertical_speeds(altitude_m);
                (
                    PHASE_CRUISE_MS,
                    phase_manta_boost_speed(altitude_m),
                    vertical,
                    vertical_boost,
                    PHASE_ACCEL_TAU,
                )
            }
            VehicleKind::Kestrel => {
                let altitude_m = ellipsoid::ecef_to_geodetic(self.pos).2;
                let (vertical, vertical_boost) = kestrel_vertical_speeds(altitude_m);
                (CRUISE_MS, BOOST_MS, vertical, vertical_boost, ACCEL_TAU)
            }
            VehicleKind::Sparrow => (
                SPARROW_CRUISE_MS,
                SPARROW_BOOST_MS,
                0.0,
                0.0,
                SPARROW_ACCEL_TAU,
            ),
        };
        let (sp_h, sp_v) = if keys.boost {
            (boost, vertical_boost)
        } else {
            (cruise, vertical)
        };
        // Light air-relative inertia. The Sparrow separates horizontal
        // response from its terrain-following lift controller: Space injects
        // one upward impulse, then the controller eases it back to the normal
        // low hover band.
        let k = 1.0 - (-dt / accel_tau).exp();
        if self.vehicle == VehicleKind::Sparrow {
            let horizontal = self.air_vel - up * self.air_vel.dot(up);
            let (sin_heading, cos_heading) = self.heading.sin_cos();
            let nose = north * cos_heading + east * sin_heading;
            let right = east * cos_heading - north * sin_heading;
            let grip_k = 1.0 - (-dt / SPARROW_LATERAL_GRIP_TAU).exp();
            let gripped_horizontal =
                nose * horizontal.dot(nose) + right * horizontal.dot(right) * (1.0 - grip_k);
            let mut target_horizontal = h * sp_h;
            if let Some(hit) = sparrow_road_hit
                && sparrow_road_effective_assist > 1.0e-4
            {
                let throttle_speed = target_horizontal.length();
                if throttle_speed > 1.0e-6 {
                    let drive_direction = target_horizontal / throttle_speed;
                    let road_direction = if hit.tangent.dot(drive_direction) < 0.0 {
                        -hit.tangent
                    } else {
                        hit.tangent
                    };
                    let guided = (drive_direction
                        * (1.0 - SPARROW_ROAD_DIRECTION_BLEND * sparrow_road_effective_assist)
                        + road_direction
                            * (SPARROW_ROAD_DIRECTION_BLEND * sparrow_road_effective_assist))
                        .normalize_or(drive_direction);
                    target_horizontal = guided * throttle_speed;
                }
                let center_speed = (hit.distance_m * SPARROW_ROAD_CENTER_GAIN)
                    .min(SPARROW_ROAD_MAX_CENTER_MS)
                    * sparrow_road_effective_assist;
                target_horizontal += hit.center_offset.normalize_or(DVec3::ZERO) * center_speed;
            }
            let mut controlled_horizontal =
                gripped_horizontal + (target_horizontal - gripped_horizontal) * k;
            if controlled_horizontal.length() < 1.0e-6 {
                controlled_horizontal = DVec3::ZERO;
            }

            let terrain = index
                .terrain_top_under(self.pos.normalize_or(DVec3::X))
                .map(|height| height.max(0.0) as f64)
                .unwrap_or(0.0);
            let altitude = ellipsoid::ecef_to_geodetic(self.pos).2;
            let height_error = terrain + SPARROW_HOVER_CLEARANCE_M - altitude;
            let jump_pressed = keys.up && !self.jump_held;
            let mut vertical_speed = self.air_vel.dot(up);
            if jump_pressed {
                vertical_speed = vertical_speed.max(SPARROW_JUMP_MS);
                self.hover_bounce_armed = true;
                self.hover_rebounded = false;
            }
            if height_error < -0.75 && !self.hover_bounce_armed && !self.hover_rebounded {
                self.hover_bounce_armed = true;
            }
            let descending_before = vertical_speed < 0.0;
            let damping = if self.hover_bounce_armed && !self.hover_rebounded {
                SPARROW_DESCENT_DAMPING
            } else {
                SPARROW_SETTLE_DAMPING
            };
            let acceleration = (height_error * SPARROW_SPRING_STIFFNESS - vertical_speed * damping)
                .clamp(-SPARROW_MAX_VERTICAL_ACCEL, SPARROW_MAX_VERTICAL_ACCEL);
            vertical_speed = (vertical_speed + acceleration * dt)
                .clamp(-SPARROW_MAX_FALL_MS, SPARROW_MAX_RISE_MS);
            if self.hover_bounce_armed
                && !self.hover_rebounded
                && descending_before
                && vertical_speed >= 0.0
            {
                self.hover_rebounded = true;
            }
            if self.hover_rebounded && height_error.abs() < 0.08 && vertical_speed.abs() < 0.08 {
                self.hover_bounce_armed = false;
                self.hover_rebounded = false;
            }
            self.air_vel = controlled_horizontal + up * vertical_speed;
        } else {
            let target_air = h * sp_h + up * (vy * sp_v);
            self.air_vel += (target_air - self.air_vel) * k;
        }
        self.jump_held = keys.up;

        let base_wind = wind.ecef(self.pos);
        let gust_direction = base_wind.normalize_or(east);
        let effective_wind = base_wind
            + gust_direction * f64::from(wind.gust_mps) * f64::from(wind.turbulence) * 0.45;
        let resisted_wind = effective_wind;
        let desired_compensation = match influence {
            WindInfluence::Off | WindInfluence::Full => DVec3::ZERO,
            WindInfluence::Assisted if h == DVec3::ZERO && vy == 0.0 => -resisted_wind * 0.95,
            WindInfluence::Assisted => -resisted_wind * 0.65,
        };
        // Bounded response time makes the correction readable as body tilt
        // without letting a gust instantly teleport the controlled velocity.
        let wind_k = 1.0 - (-dt / 0.80).exp();
        self.wind_compensation += (desired_compensation - self.wind_compensation) * wind_k;
        let applied_wind = if influence == WindInfluence::Off {
            DVec3::ZERO
        } else {
            resisted_wind
        };
        self.vel = self.air_vel + applied_wind + self.wind_compensation;

        // Body yaw follows deliberate pilot motion, not the automatic
        // station-keeping correction. Otherwise an idle drone eventually
        // turns into the wind and its chase camera is pulled around with it.
        let v_h = self.air_vel - up * self.air_vel.dot(up);
        let speed_h = v_h.length();
        if self.vehicle != VehicleKind::Sparrow && speed_h > 2.0 {
            let travel = v_h / speed_h;
            let travel_heading = travel.dot(east).atan2(travel.dot(north));
            let want = if self.is_phased() {
                travel_heading
            } else {
                body_heading_target(travel_heading, self.cam_heading)
            };
            let kh = 1.0 - (-dt / 0.22).exp();
            self.heading = wrap_pi(self.heading + wrap_pi(want - self.heading) * kh);
        }
        let yaw_rate = wrap_pi(self.heading - heading_before_turn) / dt.max(1.0e-4);

        // Chase cam: aircraft keep their existing free-look hover behavior.
        // The Sparrow also follows while its steering input is active, so a
        // standing turn rotates the view before the rider opens the throttle.
        // Manual orbit and reversing still suppress follow for every vehicle.
        let sparrow_is_steering =
            self.vehicle == VehicleKind::Sparrow && self.sparrow_steer.abs() > 0.02;
        let should_follow = speed_h > FOLLOW_MIN_SPEED || sparrow_is_steering;
        if should_follow && self.orbit_hold <= 0.0 && !keys.back {
            let tau = if self.vehicle == VehicleKind::Sparrow {
                if keys.boost { 0.12 } else { 0.20 }
            } else if keys.boost {
                FOLLOW_TAU_BOOST
            } else {
                FOLLOW_TAU
            };
            let kc = 1.0 - (-dt / tau).exp();
            self.cam_heading =
                wrap_pi(self.cam_heading + wrap_pi(self.heading - self.cam_heading) * kc);
        }

        // Cosmetic lean into motion (deeper under boost).
        let (sb, cb) = self.heading.sin_cos();
        let fwd_b = north * cb + east * sb;
        let right_b = east * cb - north * sb;
        let max_roll = if self.vehicle == VehicleKind::Sparrow {
            if keys.boost { 0.62 } else { 0.40 }
        } else if self.is_phased() {
            if keys.boost { 0.70 } else { 0.38 }
        } else if keys.boost {
            0.55
        } else {
            0.28
        };
        let max_forward = match (self.vehicle, keys.boost) {
            (VehicleKind::SplitArrow | VehicleKind::PhaseManta, true) => {
                PRODUCTION_PHASE_BOOST_PITCH_MAX
            }
            (VehicleKind::Sparrow, _) => SPARROW_FORWARD_PITCH_MAX,
            _ => max_roll,
        };
        let control_effort = self.air_vel + self.wind_compensation;
        let lf = (control_effort.dot(fwd_b) / boost * 1.4).clamp(-max_forward, max_forward);
        let lateral_bank = control_effort.dot(right_b) / boost * 1.4;
        let turn_bank = turn_bank_target(yaw_rate, self.vehicle);
        let bank_direction = if self.vehicle.is_phased() { -1.0 } else { 1.0 };
        let lr = ((lateral_bank + turn_bank) * bank_direction).clamp(-max_roll, max_roll);
        let lean_tau = if self.vehicle == VehicleKind::Sparrow {
            0.10
        } else {
            0.18
        };
        let kl = 1.0 - (-dt / lean_tau).exp();
        self.lean_f += (lf - self.lean_f) * kl;
        self.lean_r += (lr - self.lean_r) * kl;

        let kf = 1.0 - (-dt / 0.12).exp();
        self.flame += ((keys.boost as i8) as f64 - self.flame) * kf;

        self.step_collide(dt, index);
    }

    /// Integrate `vel` over `dt` with continuous swept boxes, resolved per
    /// local axis. A blocked axis zeroes
    /// its velocity component while the others keep sliding. Public for
    /// tests, which drive `vel` directly.
    pub fn step_collide(&mut self, dt: f64, index: &ResidentStore) {
        if self.is_phased() {
            self.pos += self.vel * dt;
            self.clamp_phase_elevation(index);
            return;
        }
        let (east, north, up) = enu_at(self.pos);

        // Depenetration: newly streamed (or coarse-ancestor) geometry can
        // swallow the drone; lift it free gradually rather than trap it.
        if self.box_hit(self.pos, east, north, up, index) == Some(true) {
            let mut lifted = 0.0;
            while lifted < 8.0 {
                self.pos += up * 0.4;
                lifted += 0.4;
                if self.box_hit(self.pos, east, north, up, index) != Some(true) {
                    break;
                }
            }
            let down = self.vel.dot(up).min(0.0);
            self.vel -= up * down;
            if down < 0.0 {
                self.settle_hover_after_contact();
            }
        }

        for (axis_index, axis) in [east, north, up].into_iter().enumerate() {
            let step = self.vel.dot(axis) * dt;
            if step.abs() < 1.0e-9 {
                continue;
            }
            let displacement = axis * step;
            let mut half_axes = [
                east * self.collision_half_h(),
                north * self.collision_half_h(),
                up * self.collision_half_v(),
            ];
            half_axes[axis_index] += axis * step.abs() * 0.5;
            if index.box_hit(self.pos + displacement * 0.5, half_axes, &mut self.frames)
                == Some(true)
            {
                // Continuous swept boxes cannot step over a thin wall. Find
                // the last clear fraction, then retain motion on free axes.
                let mut lo = 0.0;
                let mut hi = 1.0;
                for _ in 0..18 {
                    let t = (lo + hi) * 0.5;
                    let mut half_axes = [
                        east * self.collision_half_h(),
                        north * self.collision_half_h(),
                        up * self.collision_half_v(),
                    ];
                    half_axes[axis_index] += axis * step.abs() * t * 0.5;
                    if index.box_hit(
                        self.pos + displacement * t * 0.5,
                        half_axes,
                        &mut self.frames,
                    ) == Some(true)
                    {
                        hi = t;
                    } else {
                        lo = t;
                    }
                }
                // Keep a two-millimetre contact skin: rebuilding the local
                // ENU box next frame must not reinterpret a tangent contact
                // as penetration and lift the craft up the face of a wall.
                self.pos += displacement * (lo - 0.002 / step.abs()).max(0.0);
                self.vel -= axis * self.vel.dot(axis);
                if axis == up && step < 0.0 {
                    self.settle_hover_after_contact();
                }
            } else {
                self.pos += displacement;
            }
        }

        // No collision data under the drone at all (outflying every
        // occupancy-bearing level): the terrain-top floor is the backstop —
        // never clip into a mountain that hasn't streamed yet.
        if self.box_hit(self.pos, east, north, up, index).is_none()
            && let Some(top) = index.terrain_top_under(self.pos.normalize_or(DVec3::X))
        {
            let (lat, lon, alt) = ellipsoid::ecef_to_geodetic(self.pos);
            let floor = top.max(0.0) as f64 + self.collision_half_v() + 0.4;
            if alt < floor {
                self.pos = ellipsoid::geodetic_to_ecef(lat, lon, floor);
                let down = self.vel.dot(up).min(0.0);
                self.vel -= up * down;
                if down < 0.0 {
                    self.settle_hover_after_contact();
                }
            }
        }
    }

    fn settle_hover_after_contact(&mut self) {
        // Contact can stop descent before the next control tick observes its
        // velocity reversal. Finish that bounce here so recovery is damped.
        if self.vehicle == VehicleKind::Sparrow && self.hover_bounce_armed {
            self.hover_rebounded = true;
        }
    }

    fn clamp_phase_elevation(&mut self, index: &ResidentStore) {
        let direction = self.pos.normalize_or(DVec3::X);
        let terrain_top = index
            .terrain_top_under(direction)
            .map(|height| height.max(0.0) as f64)
            .unwrap_or(0.0);
        let floor = terrain_top + PHASE_MIN_CLEARANCE_M;
        let (lat, lon, altitude) = ellipsoid::ecef_to_geodetic(self.pos);
        if altitude >= floor {
            return;
        }

        self.pos = ellipsoid::geodetic_to_ecef(lat, lon, floor);
        let up = enu_at(self.pos).2;
        let ground_speed_down = self.vel.dot(up).min(0.0);
        self.vel -= up * ground_speed_down;
        let airspeed_down = self.air_vel.dot(up).min(0.0);
        self.air_vel -= up * airspeed_down;
    }

    /// Does the drone box intersect the continuous resident collision field?
    fn box_hit(
        &mut self,
        center: DVec3,
        east: DVec3,
        north: DVec3,
        up: DVec3,
        index: &ResidentStore,
    ) -> Option<bool> {
        index.box_hit(
            center,
            [
                east * self.collision_half_h(),
                north * self.collision_half_h(),
                up * self.collision_half_v(),
            ],
            &mut self.frames,
        )
    }

    fn collision_half_h(&self) -> f64 {
        match self.vehicle {
            VehicleKind::Sparrow => SPARROW_COLLISION_HALF_H,
            _ => COLLISION_HALF_H,
        }
    }

    fn collision_half_v(&self) -> f64 {
        match self.vehicle {
            VehicleKind::Sparrow => SPARROW_COLLISION_HALF_V,
            _ => COLLISION_HALF_V,
        }
    }

    /// Third-person follow cam: boom behind/above the drone, shortened by
    /// the first blocker along the ray (collision-aware), sprung so cuts
    /// snap in and relax out. Returns (eye, forward) for the camera.
    pub fn camera_pose(&mut self, dt: f64, index: &ResidentStore) -> (DVec3, DVec3) {
        let (east, north, up) = enu_at(self.pos);
        let look_height = match self.vehicle {
            VehicleKind::Sparrow => 0.32,
            _ => 0.8,
        };
        let look_at = self.pos + up * look_height;
        let (sh, ch) = self.cam_heading.sin_cos();
        let horiz = north * ch + east * sh;
        let (sp, cp) = self.cam_pitch.sin_cos();
        let fwd = -up * cp + horiz * sp;

        let mut clear = self.boom_len;
        if !self.is_phased() {
            if let Some(distance) = index.raycast(
                look_at - fwd * 0.9,
                -fwd,
                (self.boom_len - 0.9).max(0.0),
                &mut self.frames,
            ) {
                clear = (distance + 0.4).max(1.2);
            }
        }

        // Snap in (never rest inside a wall), relax out.
        let tau = if clear < self.boom_cur { 0.03 } else { 0.45 };
        let k = 1.0 - (-dt / tau).exp();
        self.boom_cur = (self.boom_cur + (clear - self.boom_cur) * k).min(clear);

        let mut eye = look_at - fwd * self.boom_cur;
        // Terrain backstop for the eye only where no voxel data exists.
        if !self.is_phased()
            && index.solid_at(eye, &mut self.frames).is_none()
            && let Some(top) = index.terrain_top_under(eye.normalize_or(DVec3::X))
        {
            let (lat, lon, alt) = ellipsoid::ecef_to_geodetic(eye);
            let floor = top.max(0.0) as f64 + 0.6;
            if alt < floor {
                eye = ellipsoid::geodetic_to_ecef(lat, lon, floor);
            }
        }
        (eye, (look_at - eye).normalize_or(fwd))
    }

    /// Model matrix mapping centered, metre-authored vehicle geometry to
    /// ECEF (f64; the renderer rebases it against the camera), plus the unit
    /// axes the shader resolves face normals through.
    pub fn model_matrix(&self) -> (DMat4, [DVec3; 3]) {
        let (east, north, up) = enu_at(self.pos);
        let (sb, cb) = self.heading.sin_cos();
        let f0 = north * cb + east * sb;
        let r0 = east * cb - north * sb;
        // Nose-down pitch about the right axis, then bank about forward.
        let (slf, clf) = self.lean_f.sin_cos();
        let fwd = f0 * clf - up * slf;
        let u1 = up * clf + f0 * slf;
        let (slr, clr) = self.lean_r.sin_cos();
        let right = r0 * clr + u1 * slr;
        let up_b = u1 * clr - r0 * slr;
        // With +Y up and +Z forward, an authored right-handed mesh's +X
        // points to port. East/up/north would mirror its geometry and lettering.
        let port = -right;
        let model = DMat4::from_cols(
            port.extend(0.0),
            up_b.extend(0.0),
            fwd.extend(0.0),
            self.pos.extend(1.0),
        );
        (model, [port, up_b, fwd])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::cell::ChunkAddr;
    use vearth_core::cubesphere::Face;

    fn mid_cell() -> CellId {
        let n = 1u32 << 19;
        CellId {
            face: Face(0),
            level: 19,
            i: n / 2,
            j: n / 2,
        }
    }

    fn road_assist_fixture(offset_m: f64) -> (ResidentStore, ChunkFrame, Player) {
        let cell = mid_cell();
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let center = frame.anchor + frame.y_axis * SPARROW_HOVER_CLEARANCE_M;
        let (east, north, _) = enu_at(center);
        let road_direction = frame.x_axis;
        let heading = road_direction.dot(east).atan2(road_direction.dot(north));
        let start = center + frame.z_axis * offset_m;
        let mut index = ResidentStore::new();
        index.set_height_for_test(cell, (0.0, 0.0));
        index.set_road_guides_for_test(
            cell,
            vec![vearth_gen::RoadGuide {
                a_local: [-16.0, 16.0],
                b_local: [48.0, 16.0],
                a_exit: [0.0; 2],
                b_exit: [0.0; 2],
                width_m: 6.5,
                assist: 0.9,
            }],
        );
        (
            index,
            frame,
            Player::spawn_as(start, heading, VehicleKind::Sparrow),
        )
    }

    #[test]
    fn sparrow_road_assist_centres_aligned_forward_travel() {
        let (index, frame, mut player) = road_assist_fixture(4.5);
        let initial_offset = (player.pos - frame.anchor).dot(frame.z_axis);
        let keys = PlayKeys {
            fwd: true,
            ..Default::default()
        };
        for _ in 0..24 {
            player.update(1.0 / 60.0, &keys, &index);
        }
        let final_offset = (player.pos - frame.anchor).dot(frame.z_axis);
        assert!(
            final_offset.abs() < initial_offset.abs() - 0.20,
            "road assist did not centre travel: {initial_offset} -> {final_offset}"
        );
        assert!(
            player.sparrow_road_assist > 0.50,
            "road capture never engaged: {}",
            player.sparrow_road_assist
        );
    }

    #[test]
    fn sparrow_road_assist_upgrades_cached_path_guides() {
        let (mut index, frame, mut player) = road_assist_fixture(2.0);
        index.set_road_guides_for_test(
            mid_cell(),
            vec![vearth_gen::RoadGuide {
                a_local: [-16.0, 16.0],
                b_local: [48.0, 16.0],
                a_exit: [0.0; 2],
                b_exit: [0.0; 2],
                width_m: 2.4,
                assist: 0.45,
            }],
        );
        let initial_offset = (player.pos - frame.anchor).dot(frame.z_axis);
        let keys = PlayKeys {
            fwd: true,
            ..Default::default()
        };
        for _ in 0..24 {
            player.update(1.0 / 60.0, &keys, &index);
        }
        let final_offset = (player.pos - frame.anchor).dot(frame.z_axis);
        assert!(
            final_offset.abs() < initial_offset.abs() - 0.15,
            "cached path guide remained too weak: {initial_offset} -> {final_offset}"
        );
        assert!(player.sparrow_road_assist > 0.65);
    }

    #[test]
    fn sparrow_road_assist_turns_into_authored_sharp_corner() {
        let cell = mid_cell();
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let start = frame.local_to_ecef(DVec3::new(4.0, 0.0, 16.0))
            + frame.y_axis * SPARROW_HOVER_CLEARANCE_M;
        let (east, north, _) = enu_at(start);
        let heading = frame.x_axis.dot(east).atan2(frame.x_axis.dot(north));
        let mut index = ResidentStore::new();
        index.set_height_for_test(cell, (0.0, 0.0));
        index.set_road_guides_for_test(
            cell,
            vec![
                vearth_gen::RoadGuide {
                    a_local: [-16.0, 16.0],
                    b_local: [16.0, 16.0],
                    a_exit: [0.0; 2],
                    b_exit: [0.0, 1.0],
                    width_m: 6.5,
                    assist: 0.9,
                },
                vearth_gen::RoadGuide {
                    a_local: [16.0, 16.0],
                    b_local: [16.0, 48.0],
                    a_exit: [-1.0, 0.0],
                    b_exit: [0.0; 2],
                    width_m: 6.5,
                    assist: 0.9,
                },
            ],
        );
        let mut player = Player::spawn_as(start, heading, VehicleKind::Sparrow);
        let keys = PlayKeys {
            fwd: true,
            ..Default::default()
        };

        for _ in 0..60 {
            player.update(1.0 / 60.0, &keys, &index);
        }

        let (east, north, _) = enu_at(player.pos);
        let nose = north * player.heading.cos() + east * player.heading.sin();
        assert!(
            nose.dot(frame.z_axis) > 0.55,
            "Sparrow failed to rotate into the 90-degree road corner"
        );
    }

    #[test]
    fn sparrow_full_steering_overrides_road_assist() {
        let (road_index, _, mut assisted) = road_assist_fixture(2.0);
        let mut free_index = ResidentStore::new();
        free_index.set_height_for_test(mid_cell(), (0.0, 0.0));
        let mut free = Player::spawn_as(assisted.pos, assisted.heading, VehicleKind::Sparrow);
        let keys = PlayKeys {
            fwd: true,
            right: true,
            ..Default::default()
        };
        for _ in 0..24 {
            assisted.update(1.0 / 60.0, &keys, &road_index);
            free.update(1.0 / 60.0, &keys, &free_index);
        }
        assert!(
            wrap_pi(assisted.heading - free.heading).abs() < 1.0e-10,
            "road assist resisted deliberate steering"
        );
        assert!(
            (assisted.pos - free.pos).length() < 1.0e-6,
            "road assist added lateral force during full steering"
        );
    }

    #[test]
    fn sparrow_road_assist_preserves_reverse_throttle() {
        let (index, _, mut player) = road_assist_fixture(2.0);
        let (east, north, _) = enu_at(player.pos);
        let (sin_heading, cos_heading) = player.heading.sin_cos();
        let nose = north * cos_heading + east * sin_heading;
        let keys = PlayKeys {
            back: true,
            ..Default::default()
        };
        for _ in 0..24 {
            player.update(1.0 / 60.0, &keys, &index);
        }
        assert!(
            player.air_vel.dot(nose) < -2.0,
            "road assist reversed backward throttle: {}",
            player.air_vel.dot(nose)
        );
    }

    /// Continuous floor slab and wall fixture in the cell's render coordinates.
    fn walled_index() -> (ResidentStore, ChunkFrame) {
        let cell = mid_cell();
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let mut triangles =
            crate::prepared_render::test_box_triangles(&frame, [0.0; 3], [32.0, 8.0, 32.0]);
        triangles.extend(crate::prepared_render::test_box_triangles(
            &frame,
            [26.0, 8.0, 0.0],
            [32.0; 3],
        ));
        let mut index = ResidentStore::new();
        index.set_collision_for_test(
            cell,
            vearth_gen::collision::SurfaceCollision::from_triangles(triangles),
        );
        (index, frame)
    }

    #[test]
    fn neutral_vehicle_meshes_fit_collision_for_every_yaw() {
        let position = ellipsoid::geodetic_to_ecef(0.5, 0.5, 500.0);
        for vehicle in [VehicleKind::Kestrel, VehicleKind::Sparrow] {
            let player = Player::spawn_as(position, 0.0, vehicle);
            let mesh = crate::vehicles::build_stage_mesh(vehicle);
            for vertex in mesh.vertices {
                let [x, y, z] = vertex.pos.map(f64::from);
                // The XZ radius bounds every yaw, including oblique walls.
                assert!(
                    x.hypot(z) <= player.collision_half_h(),
                    "{vehicle:?} hull exceeds collision half-width"
                );
                assert!(
                    y.abs() <= player.collision_half_v(),
                    "{vehicle:?} hull exceeds collision half-height"
                );
            }
        }
    }

    #[test]
    fn auto_hover_decays_velocity() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.5, 0.5, 500.0);
        let mut p = Player::spawn(pos, 0.0);
        p.vel = DVec3::new(30.0, 0.0, 0.0);
        for _ in 0..180 {
            p.update(1.0 / 60.0, &PlayKeys::default(), &idx);
        }
        assert!(p.speed_ms() < 0.2, "should hover: {}", p.speed_ms());
    }

    #[test]
    fn boost_outruns_cruise() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, 2000.0);
        let mut p = Player::spawn(pos, 0.0);
        let keys = PlayKeys {
            fwd: true,
            ..Default::default()
        };
        for _ in 0..240 {
            p.update(1.0 / 60.0, &keys, &idx);
        }
        let cruise = p.speed_ms();
        assert!((cruise - CRUISE_MS).abs() < 1.0, "cruise speed: {cruise}");
        let keys = PlayKeys {
            fwd: true,
            boost: true,
            ..Default::default()
        };
        for _ in 0..240 {
            p.update(1.0 / 60.0, &keys, &idx);
        }
        let boost = p.speed_ms();
        assert!((boost - BOOST_MS).abs() < 2.0, "boost speed: {boost}");
        // 4 s of full boost must not drift the position off the ellipsoid
        // frame (up realignment is continuous).
        let (_, _, alt) = ellipsoid::ecef_to_geodetic(p.pos);
        assert!((alt - 2000.0).abs() < 30.0, "altitude drifted: {alt}");
    }

    #[test]
    fn sparrow_tracks_the_low_terrain_hover_band() {
        let lat = 0.3;
        let lon = 0.3;
        let ground = 120.0;
        let mut idx = ResidentStore::new();
        let cell = CellId::from_dir(ellipsoid::geodetic_to_ecef(lat, lon, 0.0), 19);
        idx.set_height_for_test(cell, (ground as f32, ground as f32));
        let start = ellipsoid::geodetic_to_ecef(lat, lon, ground + 30.0);
        let mut player = Player::spawn_as(start, 0.0, VehicleKind::Sparrow);

        for _ in 0..600 {
            player.update(1.0 / 60.0, &PlayKeys::default(), &idx);
        }
        let altitude = ellipsoid::ecef_to_geodetic(player.pos).2;
        let clearance = altitude - ground;
        assert!(
            (clearance - SPARROW_HOVER_CLEARANCE_M).abs() < 0.15,
            "Sparrow did not settle at its hover target: {clearance}"
        );
        assert!(player.vel.dot(enu_at(player.pos).2).abs() < 0.25);
        assert!(player.collision_half_h() < COLLISION_HALF_H);
        assert!(player.collision_half_v() < COLLISION_HALF_V);
    }

    #[test]
    fn sparrow_space_jumps_then_settles_back_to_ground_following_height() {
        let lat = 0.25;
        let lon = 0.35;
        let ground = 80.0;
        let mut idx = ResidentStore::new();
        let cell = CellId::from_dir(ellipsoid::geodetic_to_ecef(lat, lon, 0.0), 19);
        idx.set_height_for_test(cell, (ground as f32, ground as f32));
        let start = ellipsoid::geodetic_to_ecef(lat, lon, ground + SPARROW_HOVER_CLEARANCE_M);
        let mut player = Player::spawn_as(start, 0.0, VehicleKind::Sparrow);

        player.update(
            1.0 / 60.0,
            &PlayKeys {
                up: true,
                ..Default::default()
            },
            &idx,
        );
        let mut peak_clearance = 0.0_f64;
        let mut minimum_after_peak = f64::INFINITY;
        let mut falling = false;
        let mut rebounds = 0_u32;
        let mut peak_frame = 0_usize;
        let mut target_cross_frame = None;
        let mut max_fall_speed = 0.0_f64;
        for frame in 0..600 {
            player.update(1.0 / 60.0, &PlayKeys::default(), &idx);
            let altitude = ellipsoid::ecef_to_geodetic(player.pos).2;
            let clearance = altitude - ground;
            if clearance > peak_clearance {
                peak_clearance = clearance;
                peak_frame = frame;
            } else if clearance < peak_clearance - 0.10 {
                minimum_after_peak = minimum_after_peak.min(clearance);
            }
            let vertical_speed = player.vel.dot(enu_at(player.pos).2);
            max_fall_speed = max_fall_speed.max(-vertical_speed);
            if frame > peak_frame
                && clearance <= SPARROW_HOVER_CLEARANCE_M
                && target_cross_frame.is_none()
            {
                target_cross_frame = Some(frame);
            }
            if vertical_speed < -0.15 {
                falling = true;
            } else if falling && vertical_speed > 0.15 {
                rebounds += 1;
                falling = false;
            }
        }
        let final_clearance = ellipsoid::ecef_to_geodetic(player.pos).2 - ground;
        assert!(
            peak_clearance > SPARROW_HOVER_CLEARANCE_M + 2.0,
            "Space did not produce a readable jump: {peak_clearance}"
        );
        // The terrain backstop permits contact at its floor; allow for the
        // geodetic/ECEF round trip when checking that clearance.
        assert!(
            minimum_after_peak < SPARROW_HOVER_CLEARANCE_M - 0.15
                && minimum_after_peak >= SPARROW_COLLISION_HALF_V + 0.4 - 1.0e-6,
            "descent should dip below target and maintain terrain clearance: {minimum_after_peak}"
        );
        assert_eq!(rebounds, 1, "hover spring should rebound exactly once");
        assert!(
            max_fall_speed > 10.0,
            "Sparrow still falls like a feather: {max_fall_speed} m/s"
        );
        assert!(
            target_cross_frame.unwrap() - peak_frame < 90,
            "Sparrow took too long to fall back through its hover target"
        );
        assert!(
            (final_clearance - SPARROW_HOVER_CLEARANCE_M).abs() < 0.15,
            "Sparrow did not settle back at its hover target: {final_clearance}"
        );
    }

    #[test]
    fn sparrow_steers_its_nose_without_lateral_glide() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, SPARROW_HOVER_CLEARANCE_M);
        let steer_only = PlayKeys {
            right: true,
            ..Default::default()
        };
        let mut sparrow = Player::spawn_as(pos, 0.0, VehicleKind::Sparrow);
        let mut max_sparrow_step = 0.0_f64;
        let mut previous_heading = sparrow.heading;
        let mut quarter_turn_frame = None;
        for frame in 0..120 {
            sparrow.update(1.0 / 60.0, &steer_only, &idx);
            max_sparrow_step =
                max_sparrow_step.max(wrap_pi(sparrow.heading - previous_heading).abs());
            previous_heading = sparrow.heading;
            if quarter_turn_frame.is_none() && sparrow.heading >= 90.0_f64.to_radians() {
                quarter_turn_frame = Some(frame);
            }
        }
        assert!(
            max_sparrow_step <= SPARROW_IDLE_STEER_RATE / 60.0 + 1.0e-6,
            "Sparrow snapped by {}° in one frame",
            max_sparrow_step.to_degrees()
        );
        assert!(
            (70..=105).contains(&quarter_turn_frame.unwrap()),
            "standing quarter-turn should take 1.2–1.75 s, took {:?} frames",
            quarter_turn_frame
        );
        let horizontal_displacement = {
            let up = enu_at(pos).2;
            let delta = sparrow.pos - pos;
            (delta - up * delta.dot(up)).length()
        };
        assert!(
            horizontal_displacement < 0.05,
            "steering-only input strafed the Sparrow by {horizontal_displacement} m"
        );

        let drive_and_steer = PlayKeys {
            fwd: true,
            right: true,
            ..Default::default()
        };
        for _ in 0..180 {
            sparrow.update(1.0 / 60.0, &drive_and_steer, &idx);
        }
        let (east, north, up) = enu_at(sparrow.pos);
        let (sin_heading, cos_heading) = sparrow.heading.sin_cos();
        let nose = north * cos_heading + east * sin_heading;
        let horizontal_velocity = sparrow.vel - up * sparrow.vel.dot(up);
        assert!(
            horizontal_velocity.normalize_or_zero().dot(nose) > 0.96,
            "Sparrow is side-slipping instead of following its steered nose"
        );
    }

    #[test]
    fn sparrow_keeps_useful_steering_authority_while_moving() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, SPARROW_HOVER_CLEARANCE_M);
        let mut sparrow = Player::spawn_as(pos, 0.0, VehicleKind::Sparrow);
        let forward = PlayKeys {
            fwd: true,
            ..Default::default()
        };
        for _ in 0..240 {
            sparrow.update(1.0 / 60.0, &forward, &idx);
        }
        assert!(sparrow.airspeed_ms() > SPARROW_CRUISE_MS - 1.0);

        let turn = PlayKeys {
            fwd: true,
            right: true,
            ..Default::default()
        };
        for _ in 0..30 {
            sparrow.update(1.0 / 60.0, &turn, &idx);
        }
        let mut accumulated_turn = 0.0;
        let mut previous_heading = sparrow.heading;
        for _ in 0..60 {
            sparrow.update(1.0 / 60.0, &turn, &idx);
            accumulated_turn += wrap_pi(sparrow.heading - previous_heading);
            previous_heading = sparrow.heading;
        }
        let cruise_turn_degrees = accumulated_turn.to_degrees();
        assert!(
            (cruise_turn_degrees - 90.0).abs() < 1.0,
            "90°/s cruise steering turned {cruise_turn_degrees}° in 1 s"
        );

        let boost_turn = PlayKeys {
            fwd: true,
            right: true,
            boost: true,
            ..Default::default()
        };
        accumulated_turn = 0.0;
        previous_heading = sparrow.heading;
        for _ in 0..120 {
            sparrow.update(1.0 / 60.0, &boost_turn, &idx);
            accumulated_turn += wrap_pi(sparrow.heading - previous_heading);
            previous_heading = sparrow.heading;
        }
        let boost_turn_degrees = accumulated_turn.to_degrees();
        assert!(
            (boost_turn_degrees - 140.0).abs() < 1.0,
            "70°/s boost steering turned {boost_turn_degrees}° in 2 s"
        );
    }

    #[test]
    fn sparrow_stationary_steering_turns_camera_without_changing_drone_hover() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, SPARROW_HOVER_CLEARANCE_M);
        let steer_only = PlayKeys {
            right: true,
            ..Default::default()
        };
        let mut sparrow = Player::spawn_as(pos, 0.0, VehicleKind::Sparrow);
        for _ in 0..60 {
            sparrow.update(1.0 / 60.0, &steer_only, &idx);
        }
        assert!(
            sparrow.cam_heading > 0.6,
            "stationary steering left the Sparrow camera behind at {}°",
            sparrow.cam_heading.to_degrees()
        );
        assert!(
            wrap_pi(sparrow.heading - sparrow.cam_heading).abs() < 0.35,
            "Sparrow camera lagged its standing turn by {}°",
            wrap_pi(sparrow.heading - sparrow.cam_heading)
                .abs()
                .to_degrees()
        );

        let mut drone = Player::spawn_as(pos, 1.0, VehicleKind::Kestrel);
        drone.cam_heading = 0.0;
        for _ in 0..60 {
            drone.update(1.0 / 60.0, &PlayKeys::default(), &idx);
        }
        assert_eq!(
            drone.cam_heading, 0.0,
            "stationary drone camera should retain its existing free-look behavior"
        );
    }

    #[test]
    fn sparrow_accelerates_progressively_to_lower_speed_caps() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, SPARROW_HOVER_CLEARANCE_M);
        let mut sparrow = Player::spawn_as(pos, 0.0, VehicleKind::Sparrow);
        let cruise = PlayKeys {
            fwd: true,
            ..Default::default()
        };
        for _ in 0..6 {
            sparrow.update(1.0 / 60.0, &cruise, &idx);
        }
        assert!(
            sparrow.airspeed_ms() < SPARROW_CRUISE_MS * 0.35,
            "Sparrow blasted to cruise immediately: {}",
            sparrow.airspeed_ms()
        );
        for _ in 0..240 {
            sparrow.update(1.0 / 60.0, &cruise, &idx);
        }
        assert!((sparrow.airspeed_ms() - SPARROW_CRUISE_MS).abs() < 1.0);

        let boost = PlayKeys {
            fwd: true,
            boost: true,
            ..Default::default()
        };
        for _ in 0..6 {
            sparrow.update(1.0 / 60.0, &boost, &idx);
        }
        assert!(
            sparrow.airspeed_ms() < SPARROW_BOOST_MS * 0.65,
            "Sparrow blasted to boost immediately: {}",
            sparrow.airspeed_ms()
        );
        for _ in 0..300 {
            sparrow.update(1.0 / 60.0, &boost, &idx);
        }
        assert!((sparrow.airspeed_ms() - SPARROW_BOOST_MS).abs() < 1.0);
    }

    #[test]
    fn sparrow_forward_lean_never_exceeds_five_degrees() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, SPARROW_HOVER_CLEARANCE_M);
        let mut sparrow = Player::spawn_as(pos, 0.0, VehicleKind::Sparrow);
        let keys = PlayKeys {
            fwd: true,
            boost: true,
            ..Default::default()
        };
        for _ in 0..180 {
            sparrow.update(1.0 / 60.0, &keys, &idx);
            assert!(
                sparrow.lean_f.abs() <= SPARROW_FORWARD_PITCH_MAX + 1.0e-6,
                "Sparrow pitched to {}°",
                sparrow.lean_f.to_degrees()
            );
        }
    }

    #[test]
    fn full_wind_changes_ground_speed_without_changing_airspeed_cap() {
        let idx = ResidentStore::new();
        let position = ellipsoid::geodetic_to_ecef(0.4, 0.2, 2_000.0);
        let keys = PlayKeys {
            fwd: true,
            ..Default::default()
        };
        let run = |north_mps| {
            let mut player = Player::spawn(position, 0.0);
            let wind = WindSample {
                north_mps,
                ..Default::default()
            };
            for _ in 0..240 {
                player.update_with_weather(1.0 / 60.0, &keys, &idx, wind, WindInfluence::Full);
            }
            (player.airspeed_ms(), player.speed_ms())
        };
        let (head_air, head_ground) = run(-8.0);
        let (tail_air, tail_ground) = run(8.0);
        assert!((head_air - CRUISE_MS).abs() < 1.0);
        assert!((tail_air - CRUISE_MS).abs() < 1.0);
        assert!(
            tail_ground > head_ground + 14.0,
            "{head_ground} vs {tail_ground}"
        );
    }

    #[test]
    fn assisted_hover_counters_sustained_crosswind_gradually() {
        let idx = ResidentStore::new();
        let position = ellipsoid::geodetic_to_ecef(0.4, 0.2, 2_000.0);
        let wind = WindSample {
            east_mps: 14.0,
            gust_mps: 2.0,
            turbulence: 0.2,
            ..Default::default()
        };
        let mut assisted = Player::spawn(position, 0.0);
        for _ in 0..300 {
            assisted.update_with_weather(
                1.0 / 60.0,
                &PlayKeys::default(),
                &idx,
                wind,
                WindInfluence::Assisted,
            );
        }
        let mut full = Player::spawn(position, 0.0);
        for _ in 0..300 {
            full.update_with_weather(
                1.0 / 60.0,
                &PlayKeys::default(),
                &idx,
                wind,
                WindInfluence::Full,
            );
        }
        assert!(
            assisted.speed_ms() < 1.0,
            "assisted hover: {}",
            assisted.speed_ms()
        );
        assert!(full.speed_ms() > 12.0, "full wind: {}", full.speed_ms());
    }

    #[test]
    fn assisted_station_keeping_does_not_steer_body_or_camera() {
        let idx = ResidentStore::new();
        let position = ellipsoid::geodetic_to_ecef(0.4, 0.2, 2_000.0);
        let mut player = Player::spawn(position, 0.8);
        player.cam_heading = -1.0;
        let wind = WindSample {
            east_mps: 14.0,
            gust_mps: 3.0,
            turbulence: 0.6,
            ..Default::default()
        };

        for _ in 0..360 {
            player.update_with_weather(
                1.0 / 60.0,
                &PlayKeys::default(),
                &idx,
                wind,
                WindInfluence::Assisted,
            );
        }

        assert!((player.heading - 0.8).abs() < 1.0e-9);
        assert!((player.cam_heading + 1.0).abs() < 1.0e-9);
    }

    #[test]
    fn floor_stops_descent_and_wall_slides() {
        let (idx, frame) = walled_index();
        // Start above the floor slab, well clear of the wall.
        let start = frame.local_to_ecef(DVec3::new(10.0, 20.0, 16.0));
        let mut p = Player::spawn(start, 0.0);
        let up = frame.y_axis;

        // Descend: must stop above the 8-voxel floor, never inside it.
        p.vel = -up * 12.0;
        for _ in 0..240 {
            p.step_collide(1.0 / 60.0, &idx);
            p.vel = -up * 12.0; // keep pushing down
        }
        let y = (p.pos - frame.origin).dot(up) / frame.grid_scale.y;
        assert!(y > 8.0 - 0.01, "sank into the floor: y = {y}");
        assert!(y < 10.5, "floated too high: y = {y}");

        // Push diagonally into the wall (+x) with a +z component: the x
        // motion must stop before the wall at x=26, the z motion continues.
        // (Slide speed kept low so the drone stays inside this one test
        // cell for the whole run.)
        let z0 = (p.pos - frame.origin).dot(frame.z_axis) / frame.grid_scale.z;
        for _ in 0..120 {
            p.vel = frame.x_axis * 10.0 + frame.z_axis * 1.5 - up * 2.0;
            p.step_collide(1.0 / 60.0, &idx);
        }
        let x = (p.pos - frame.origin).dot(frame.x_axis) / frame.grid_scale.x;
        let z1 = (p.pos - frame.origin).dot(frame.z_axis) / frame.grid_scale.z;
        assert!(x < 26.0, "clipped into the wall: x = {x}");
        assert!(x > 22.0, "never reached the wall: x = {x}");
        assert!(z1 > z0 + 2.0, "did not slide along the wall: {z0} -> {z1}");
    }

    #[test]
    fn fast_sweep_and_camera_stop_at_a_thin_continuous_wall() {
        let cell = mid_cell();
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let mut index = ResidentStore::new();
        index.set_collision_for_test(
            cell,
            vearth_gen::collision::SurfaceCollision::from_triangles(
                crate::prepared_render::test_box_triangles(
                    &frame,
                    [18.125, 0., 0.],
                    [18.135, 32., 32.],
                ),
            ),
        );
        let mut player = Player::spawn(frame.local_to_ecef(DVec3::new(3., 15., 16.)), 0.);
        player.vel = frame.x_axis * 80.;
        player.step_collide(1., &index);
        let local = frame.ecef_to_local(player.pos);
        assert!(
            local.x > 10. && local.x < 18.125,
            "swept past thin wall: {local:?}"
        );
        player.pos = frame.local_to_ecef(DVec3::new(12., 15., 16.));
        player.boom_len = 20.;
        let (east, north, _) = enu_at(player.pos);
        let toward = -frame.x_axis;
        player.cam_heading = toward.dot(east).atan2(toward.dot(north));
        player.cam_pitch = std::f64::consts::FRAC_PI_2;
        let (eye, _) = player.camera_pose(1., &index);
        assert!(
            frame.ecef_to_local(eye).x < 18.125,
            "camera crossed thin wall"
        );
    }

    #[test]
    fn generated_ocean_and_lake_stop_kestrel_sweeps_and_camera_at_the_visible_water_top() {
        use crate::prepared_render::{
            pack_surface_cell, test_generated_water, test_water_contact_top,
        };
        use vearth_data::vectors::WaterRegionKind;
        use vearth_gen::collision::MetricFrame;
        for kind in [WaterRegionKind::Ocean, WaterRegionKind::Lake] {
            let surface = test_generated_water(kind);
            let top = test_water_contact_top(&surface);
            let cell = surface.cell;
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let metric = MetricFrame::of(&frame);
            let mut packed = pack_surface_cell(surface).unwrap();
            let mut index = ResidentStore::new();
            let id = index.insert(
                cell,
                0,
                packed.height_range(),
                packed.terrain_step_m(),
                packed.terrain_relief_m(),
                packed.terrain_detail().cloned(),
                None,
                0,
            );
            index.finish_upload(
                id,
                Some(packed.take_solid_collision()),
                packed.take_terrain_collision(),
                packed.take_water(),
                packed.take_road_guides(),
                0,
            );
            let surface_point = metric.point_to_ecef([0., top, 0.]);
            let mut memo = HashMap::new();
            assert_eq!(
                index.solid_at(surface_point - frame.y_axis * 0.02, &mut memo),
                Some(true),
                "{kind:?} water interior lost contact"
            );
            assert!((index.terrain_surface_at(surface_point).unwrap() - top).abs() < 0.01);
            let mut player = Player::spawn(surface_point + frame.y_axis * 15., 0.);
            player.vel = -frame.y_axis * 80.;
            player.step_collide(1., &index);
            let clearance = metric.point_from_ecef(player.pos)[1] - top;
            assert!(
                (player.collision_half_v() - 0.01..player.collision_half_v() + 0.06)
                    .contains(&clearance),
                "{kind:?} descent crossed or floated above visible water: {clearance}"
            );
            player.pos = surface_point + frame.y_axis * 3.;
            player.boom_len = 20.;
            player.cam_pitch = std::f64::consts::PI;
            let (eye, _) = player.camera_pose(1., &index);
            let camera_clearance = metric.point_from_ecef(eye)[1] - top;
            assert!(
                (0.45..0.55).contains(&camera_clearance),
                "{kind:?} camera crossed water: {camera_clearance}"
            );
        }
    }

    #[test]
    fn phase_craft_cross_solid_geometry_without_collision_resolution() {
        let (idx, frame) = walled_index();
        let start = frame.local_to_ecef(DVec3::new(20.0, 20.0, 16.0));
        for vehicle in [VehicleKind::SplitArrow, VehicleKind::PhaseManta] {
            let mut player = Player::spawn_as(start, 0.0, vehicle);
            player.vel = frame.x_axis * (frame.grid_scale.x * 10.0);
            player.step_collide(1.0, &idx);
            player.camera_pose(1.0 / 60.0, &idx);
            let x = (player.pos - frame.origin).dot(frame.x_axis) / frame.grid_scale.x;
            assert!(x > 26.0, "{vehicle:?} was blocked by the wall: x = {x}");
            assert!(
                player.frames.is_empty(),
                "{vehicle:?} movement or camera performed a collision lookup"
            );
        }
    }

    #[test]
    fn phase_craft_boost_reaches_intercontinental_speed() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, 20_000.0);
        let keys = PlayKeys {
            fwd: true,
            boost: true,
            ..Default::default()
        };
        for vehicle in [VehicleKind::SplitArrow, VehicleKind::PhaseManta] {
            let mut player = Player::spawn_as(pos, 0.0, vehicle);
            for _ in 0..360 {
                player.update(1.0 / 60.0, &keys, &idx);
            }
            assert!(
                player.speed_ms() > 11_000.0,
                "{vehicle:?} phase boost too slow: {}",
                player.speed_ms()
            );
            match vehicle {
                VehicleKind::SplitArrow | VehicleKind::PhaseManta => assert!(
                    player.lean_f > 0.15
                        && player.lean_f <= PRODUCTION_PHASE_BOOST_PITCH_MAX + 1.0e-6,
                    "{vehicle:?} full-thrust pitch exceeds 10 degrees: {}",
                    player.lean_f
                ),
                _ => {}
            }
        }
    }

    #[test]
    fn phase_manta_boost_ceiling_rises_smoothly_with_altitude() {
        assert_eq!(PHASE_MANTA_BOOST_ALT_START_M, 2_500.0);
        assert_eq!(PHASE_MANTA_BOOST_ALT_FULL_M, 50_000.0);
        assert_eq!(PHASE_MANTA_MAX_BOOST_MS, 100_000.0);
        assert_eq!(
            phase_manta_boost_speed(PHASE_MANTA_BOOST_ALT_START_M - 1.0),
            PHASE_BOOST_MS
        );
        assert_eq!(
            phase_manta_boost_speed(PHASE_MANTA_BOOST_ALT_START_M),
            PHASE_BOOST_MS
        );
        assert!(phase_manta_boost_speed(PHASE_MANTA_BOOST_ALT_START_M + 1.0) > PHASE_BOOST_MS);
        assert_eq!(
            phase_manta_boost_speed(PHASE_MANTA_BOOST_ALT_FULL_M),
            PHASE_MANTA_MAX_BOOST_MS
        );
        assert_eq!(
            phase_manta_boost_speed(PHASE_MANTA_BOOST_ALT_FULL_M + 1.0),
            PHASE_MANTA_MAX_BOOST_MS
        );

        let quarter = phase_manta_boost_speed(
            PHASE_MANTA_BOOST_ALT_START_M
                + (PHASE_MANTA_BOOST_ALT_FULL_M - PHASE_MANTA_BOOST_ALT_START_M) * 0.25,
        );
        let halfway = phase_manta_boost_speed(
            (PHASE_MANTA_BOOST_ALT_START_M + PHASE_MANTA_BOOST_ALT_FULL_M) * 0.5,
        );
        let three_quarters = phase_manta_boost_speed(
            PHASE_MANTA_BOOST_ALT_START_M
                + (PHASE_MANTA_BOOST_ALT_FULL_M - PHASE_MANTA_BOOST_ALT_START_M) * 0.75,
        );
        assert!(PHASE_BOOST_MS < quarter);
        assert!(quarter < halfway);
        assert!(halfway < three_quarters);
        assert!(three_quarters < PHASE_MANTA_MAX_BOOST_MS);
        assert!((halfway - (PHASE_BOOST_MS + PHASE_MANTA_MAX_BOOST_MS) * 0.5).abs() < 1.0e-9);
    }

    #[test]
    fn vertical_authority_stays_precise_low_and_rises_smoothly_high() {
        assert_eq!(KESTREL_VERTICAL_ALT_START_M, 2_500.0);
        assert_eq!(KESTREL_VERTICAL_ALT_FULL_M, 25_000.0);
        assert_eq!(KESTREL_MAX_VERTICAL_MS, 180.0);
        assert_eq!(KESTREL_MAX_VERTICAL_BOOST_MS, 600.0);
        assert_eq!(PHASE_MANTA_MAX_VERTICAL_MS, 1_200.0);
        assert_eq!(PHASE_MANTA_MAX_VERTICAL_BOOST_MS, 12_000.0);

        assert_eq!(
            kestrel_vertical_speeds(KESTREL_VERTICAL_ALT_START_M - 1.0),
            (VERTICAL_MS, VERTICAL_BOOST_MS)
        );
        assert_eq!(
            kestrel_vertical_speeds(KESTREL_VERTICAL_ALT_FULL_M),
            (KESTREL_MAX_VERTICAL_MS, KESTREL_MAX_VERTICAL_BOOST_MS)
        );
        let kestrel_halfway = kestrel_vertical_speeds(
            (KESTREL_VERTICAL_ALT_START_M + KESTREL_VERTICAL_ALT_FULL_M) * 0.5,
        );
        assert!((kestrel_halfway.0 - (VERTICAL_MS + KESTREL_MAX_VERTICAL_MS) * 0.5).abs() < 1.0e-9);
        assert!(
            (kestrel_halfway.1 - (VERTICAL_BOOST_MS + KESTREL_MAX_VERTICAL_BOOST_MS) * 0.5).abs()
                < 1.0e-9
        );

        assert_eq!(
            phase_manta_vertical_speeds(PHASE_MANTA_BOOST_ALT_START_M - 1.0),
            (PHASE_VERTICAL_MS, PHASE_VERTICAL_BOOST_MS)
        );
        assert_eq!(
            phase_manta_vertical_speeds(PHASE_MANTA_BOOST_ALT_FULL_M),
            (
                PHASE_MANTA_MAX_VERTICAL_MS,
                PHASE_MANTA_MAX_VERTICAL_BOOST_MS
            )
        );
        let phase_halfway = phase_manta_vertical_speeds(
            (PHASE_MANTA_BOOST_ALT_START_M + PHASE_MANTA_BOOST_ALT_FULL_M) * 0.5,
        );
        assert!(
            (phase_halfway.0 - (PHASE_VERTICAL_MS + PHASE_MANTA_MAX_VERTICAL_MS) * 0.5).abs()
                < 1.0e-9
        );
        assert!(
            (phase_halfway.1 - (PHASE_VERTICAL_BOOST_MS + PHASE_MANTA_MAX_VERTICAL_BOOST_MS) * 0.5)
                .abs()
                < 1.0e-9
        );
    }

    #[test]
    fn high_altitude_vertical_controls_apply_the_expanded_ceiling_both_ways() {
        let idx = ResidentStore::new();
        let dt = 1.0 / 60.0;
        for (vehicle, altitude_m, ceiling_ms, accel_tau) in [
            (
                VehicleKind::Kestrel,
                KESTREL_VERTICAL_ALT_FULL_M,
                KESTREL_MAX_VERTICAL_BOOST_MS,
                ACCEL_TAU,
            ),
            (
                VehicleKind::PhaseManta,
                PHASE_MANTA_BOOST_ALT_FULL_M,
                PHASE_MANTA_MAX_VERTICAL_BOOST_MS,
                PHASE_ACCEL_TAU,
            ),
        ] {
            for descending in [false, true] {
                let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, altitude_m);
                let up = enu_at(pos).2;
                let mut player = Player::spawn_as(pos, 0.0, vehicle);
                let keys = PlayKeys {
                    up: !descending,
                    down: descending,
                    boost: true,
                    ..Default::default()
                };
                player.update(dt, &keys, &idx);

                let sign = if descending { -1.0 } else { 1.0 };
                let expected = sign * ceiling_ms * (1.0 - (-dt / accel_tau).exp());
                let actual = player.air_vel.dot(up);
                assert!(
                    (actual - expected).abs() < 1.0e-8,
                    "{vehicle:?} vertical control used {actual} m/s instead of {expected} m/s"
                );
            }
        }
    }

    #[test]
    fn only_phase_manta_unlocks_the_high_altitude_boost_ceiling() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, PHASE_MANTA_BOOST_ALT_FULL_M + 1_000.0);
        let keys = PlayKeys {
            fwd: true,
            boost: true,
            ..Default::default()
        };

        let mut split_arrow = Player::spawn_as(pos, 0.0, VehicleKind::SplitArrow);
        let mut phase_manta = Player::spawn_as(pos, 0.0, VehicleKind::PhaseManta);
        for _ in 0..480 {
            split_arrow.update(1.0 / 60.0, &keys, &idx);
            phase_manta.update(1.0 / 60.0, &keys, &idx);
        }

        assert!(
            (split_arrow.airspeed_ms() - PHASE_BOOST_MS).abs() < 2.0,
            "Split Arrow ceiling changed: {}",
            split_arrow.airspeed_ms()
        );
        assert!(
            phase_manta.airspeed_ms() > PHASE_MANTA_MAX_BOOST_MS - 50.0,
            "Phase Manta did not reach its high-altitude ceiling: {}",
            phase_manta.airspeed_ms()
        );
    }

    #[test]
    fn kestrel_strafe_yaw_is_bounded_and_does_not_snap_when_forward_is_added() {
        let pure_strafe = body_heading_target(std::f64::consts::FRAC_PI_2, 0.0);
        let forward_strafe = body_heading_target(std::f64::consts::FRAC_PI_4, 0.0);
        assert!((pure_strafe - KESTREL_MAX_TRAVEL_YAW).abs() < 1.0e-12);
        assert!((forward_strafe - pure_strafe).abs() < 1.0e-12);
    }

    #[test]
    fn phase_craft_bank_into_heading_changes() {
        assert!(turn_bank_target(1.0, VehicleKind::PhaseManta) > 0.0);
        assert!(turn_bank_target(-1.0, VehicleKind::PhaseManta) < 0.0);
        assert_eq!(
            turn_bank_target(100.0, VehicleKind::PhaseManta),
            PHASE_TURN_BANK_MAX
        );

        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.2, 0.2, 20_000.0);
        let keys = PlayKeys {
            fwd: true,
            right: true,
            ..Default::default()
        };
        for vehicle in [VehicleKind::SplitArrow, VehicleKind::PhaseManta] {
            let mut player = Player::spawn_as(pos, 0.0, vehicle);
            player.update(1.0 / 60.0, &keys, &idx);
            assert!(
                player.heading > 0.0 && player.lean_r < 0.0,
                "{vehicle:?} did not bank into a right turn"
            );
            let (_, axes) = player.model_matrix();
            assert!(
                (-axes[0]).dot(enu_at(player.pos).2) < 0.0,
                "{vehicle:?} right wing did not drop into the right turn"
            );
        }
    }

    #[test]
    fn chase_cam_follows_the_drones_turns() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.4, 0.4, 1000.0);
        let mut p = Player::spawn(pos, 0.0);
        // Holding strafe turns the drone; the chase cam must swing with it
        // (before auto-follow, cam_heading stayed bit-identical at 0).
        let keys = PlayKeys {
            right: true,
            ..Default::default()
        };
        for _ in 0..120 {
            p.update(1.0 / 60.0, &keys, &idx);
        }
        assert!(
            wrap_pi(p.cam_heading).abs() > 0.3,
            "camera never followed the turn: {}",
            p.cam_heading
        );
        // And it trails the body by a bounded lag — an arc, not a rigid
        // lock and not the unbounded 90° gap of a fixed camera.
        let lag = wrap_pi(p.heading - p.cam_heading).abs();
        assert!(lag < 1.2, "camera lag too large: {lag}");
    }

    #[test]
    fn manual_orbit_holds_then_hands_back() {
        let idx = ResidentStore::new();
        let pos = ellipsoid::geodetic_to_ecef(0.4, 0.4, 1000.0);
        let mut p = Player::spawn(pos, 0.0);
        let (_, north, _) = enu_at(p.pos);
        // Cruise north, then deliberately orbit the camera +1 rad.
        p.orbit(200.0, 0.0);
        assert!((p.cam_heading - 1.0).abs() < 1e-9);
        // While the grace period holds, flying must not touch the camera.
        for _ in 0..80 {
            p.vel = north * 30.0; // keep travel (and body heading) at north
            p.update(1.0 / 60.0, &PlayKeys::default(), &idx);
        }
        assert!(
            (p.cam_heading - 1.0).abs() < 1e-9,
            "auto-follow stole the camera during the hold: {}",
            p.cam_heading
        );
        // Past the hold, the camera eases back in behind the heading.
        for _ in 0..100 {
            p.vel = north * 30.0;
            p.update(1.0 / 60.0, &PlayKeys::default(), &idx);
        }
        assert!(
            p.cam_heading.abs() < 0.5,
            "camera never handed back to auto-follow: {}",
            p.cam_heading
        );
    }

    #[test]
    fn collides_with_generated_terrain() {
        // Exercise the same heightfield and solid handoff used by live uploads.
        use vearth_data::FnElevation;
        use vearth_gen::{Sources, generate_surface_cell};
        let n = 1u32 << 16;
        let cell = CellId {
            face: Face(0),
            level: 16,
            i: n / 2,
            j: n / 2,
        };
        let src = FnElevation(|_, _| 250.0);
        let surface = generate_surface_cell(cell, &Sources::terrain_only(&src));
        assert!(surface.terrain_collision.is_some());
        let mut idx = ResidentStore::new();
        let id = idx.insert(
            cell,
            0,
            surface.height_range,
            surface.terrain_step_m,
            surface.terrain_relief_m,
            surface
                .terrain_detail
                .as_ref()
                .and_then(vearth_gen::TerrainDetailSummary::from_grid)
                .map(std::sync::Arc::new),
            None,
            0,
        );
        idx.finish_upload(
            id,
            Some(surface.solid_collision),
            surface.terrain_collision,
            surface.water,
            surface.road_guides,
            0,
        );

        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let mut p = Player::spawn(ellipsoid::geodetic_to_ecef(lat, lon, 500.0), 0.0);
        for _ in 0..600 {
            p.vel = -enu_at(p.pos).2 * 25.0;
            p.step_collide(1.0 / 60.0, &idx);
        }
        // The drone must rest at the generated terrain elevation.
        let (_, _, alt) = ellipsoid::ecef_to_geodetic(p.pos);
        assert!(alt > 244.0, "sank into generated terrain: {alt}");
        assert!(alt < 262.0, "hovering far above generated terrain: {alt}");
    }

    #[test]
    fn terrain_floor_backstops_without_occupancy() {
        // Heights resident, occupancy not (cell coarser than the collision
        // gate): descending must stop at the terrain-top floor.
        let mut idx = ResidentStore::new();
        let cell = CellId::from_dir(ellipsoid::geodetic_to_ecef(0.3, 0.3, 0.0), 9);
        idx.set_height_for_test(cell, (120.0, 300.0));
        let pos = ellipsoid::geodetic_to_ecef(0.3, 0.3, 500.0);
        let mut p = Player::spawn(pos, 0.0);
        for _ in 0..600 {
            p.vel = -enu_at(p.pos).2 * 20.0;
            p.step_collide(1.0 / 60.0, &idx);
        }
        let (_, _, alt) = ellipsoid::ecef_to_geodetic(p.pos);
        assert!(alt >= 300.0, "sank below the terrain floor: {alt}");
        assert!(alt < 302.0, "hovering too high above the floor: {alt}");
    }

    #[test]
    fn phase_elevation_clamp_uses_height_index_without_collision_queries() {
        let mut idx = ResidentStore::new();
        let cell = CellId::from_dir(ellipsoid::geodetic_to_ecef(0.3, 0.3, 0.0), 9);
        idx.set_height_for_test(cell, (120.0, 300.0));
        let start = ellipsoid::geodetic_to_ecef(0.3, 0.3, 500.0);

        for vehicle in [VehicleKind::SplitArrow, VehicleKind::PhaseManta] {
            let mut player = Player::spawn_as(start, 0.0, vehicle);
            for _ in 0..120 {
                player.vel = -enu_at(player.pos).2 * 200.0;
                player.step_collide(1.0 / 60.0, &idx);
            }
            let (_, _, altitude) = ellipsoid::ecef_to_geodetic(player.pos);
            assert!(
                altitude >= 300.0 + PHASE_MIN_CLEARANCE_M - 1.0e-4,
                "{vehicle:?} sank below the height clamp: {altitude}"
            );
            assert!(
                player.frames.is_empty(),
                "{vehicle:?} elevation clamp performed occupancy checks"
            );
        }
    }

    #[test]
    fn boom_shortens_before_walls() {
        let (idx, frame) = walled_index();
        // Drone hovering just above the floor near the wall; camera orbited
        // so the boom points into the wall (+x side).
        let start = frame.local_to_ecef(DVec3::new(22.0, 10.0, 16.0));
        let mut p = Player::spawn(start, 0.0);
        p.boom_len = 20.0;
        // Point the camera forward = -x, so the boom (behind) extends +x
        // into the wall. heading 0 = north; find the heading whose horiz
        // matches -x_axis by construction instead: use atan2 on the frame.
        let (east, north, _) = enu_at(p.pos);
        let minus_x = -frame.x_axis;
        p.cam_heading = minus_x.dot(east).atan2(minus_x.dot(north));
        p.cam_pitch = std::f64::consts::FRAC_PI_2; // level boom
        let (eye, _) = p.camera_pose(1.0, &idx);
        let eye_x = (eye - frame.origin).dot(frame.x_axis) / frame.grid_scale.x;
        assert!(eye_x < 26.0, "camera ended inside the wall: x = {eye_x}");
        // And with a clear view behind (-x), the boom relaxes long.
        p.cam_heading = frame.x_axis.dot(east).atan2(frame.x_axis.dot(north));
        for _ in 0..600 {
            p.camera_pose(1.0 / 60.0, &idx);
        }
        let (eye2, _) = p.camera_pose(1.0 / 60.0, &idx);
        assert!(
            (eye2 - p.pos).length() > 15.0,
            "boom should relax to ~20 m in the open: {}",
            (eye2 - p.pos).length()
        );
    }
}
