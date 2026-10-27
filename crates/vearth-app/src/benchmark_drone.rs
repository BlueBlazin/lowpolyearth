//! Bounded pilot-input continuation of the native search benchmark.

use crate::player::PlayKeys;
use crate::replay::ReplayPose;
use serde::Serialize;

const WALL_LIMIT_MS: f64 = 240_000.0;
const CITY_TOUR_WALL_LIMIT_MS: f64 = 480_000.0;
const SAMPLE_INTERVAL_MS: f64 = 500.0;
const CHECKPOINT_INTERVAL_MS: f64 = 5_000.0;
const SAMPLE_LIMIT: usize = 512;
const TAIL_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    SearchReady,
    ZoomIn,
    ZoomReady,
    ChaseReady,
    Cruise,
    Boost,
    BrakeOutbound,
    Turn,
    Return,
    BrakeReturn,
    FinalReady,
    ZoomOut,
    GlobeRotate,
    GlobeReady,
    Complete,
}

impl Phase {
    fn duration_s(self) -> Option<f64> {
        match self {
            Self::ZoomIn => Some(6.0),
            Self::ZoomOut => Some(20.0),
            Self::GlobeRotate => Some(5.0),
            Self::Cruise | Self::Boost | Self::Return => Some(20.0),
            Self::BrakeOutbound | Self::BrakeReturn => Some(3.0),
            Self::Turn => Some(std::f64::consts::PI / 1.6),
            _ => None,
        }
    }

    fn next(self, city_tour: bool) -> Self {
        match self {
            Self::SearchReady if city_tour => Self::ZoomIn,
            Self::SearchReady => Self::ChaseReady,
            Self::ZoomIn => Self::ZoomReady,
            Self::ZoomReady => Self::ChaseReady,
            Self::ChaseReady => Self::Cruise,
            Self::Cruise => Self::Boost,
            Self::Boost if city_tour => Self::Turn,
            Self::Boost => Self::BrakeOutbound,
            Self::BrakeOutbound => Self::Turn,
            Self::Turn => Self::Return,
            Self::Return => Self::BrakeReturn,
            Self::BrakeReturn => Self::FinalReady,
            Self::FinalReady if city_tour => Self::ZoomOut,
            Self::ZoomOut => Self::GlobeRotate,
            Self::GlobeRotate => Self::GlobeReady,
            Self::FinalReady | Self::GlobeReady | Self::Complete => Self::Complete,
        }
    }

    fn explore(self) -> bool {
        matches!(
            self,
            Self::SearchReady
                | Self::ZoomIn
                | Self::ZoomReady
                | Self::ZoomOut
                | Self::GlobeRotate
                | Self::GlobeReady
        )
    }

    fn controls(self) -> PlayKeys {
        PlayKeys {
            fwd: matches!(self, Self::Cruise | Self::Boost | Self::Return),
            boost: matches!(self, Self::Boost | Self::Return),
            rot_right: self == Self::Turn,
            ..PlayKeys::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Directive {
    EnterPlay,
    ExitPlay,
    ResetSelectionFence,
    Finish,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExploreControls {
    pub zoom_in: bool,
    pub zoom_out: bool,
    pub east: bool,
    pub tilt_down: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct FlightFrame {
    pub frame: u64,
    pub after_start_ms: f64,
    pub player_dt_s: f64,
    pub interval_ms: f64,
    pub main_ms: f64,
    #[serde(flatten)]
    pub render: FlightRenderStats,
    pub surface_presented: bool,
    pub search_arrived: bool,
    pub selection_ready: bool,
    pub selection_settled: bool,
    pub fully_idle: bool,
    pub foreground_missing: usize,
    pub foreground_unpresented: usize,
    pub foreground_nonopaque: usize,
    pub guard_missing: usize,
    pub background_missing: usize,
    pub desired_missing: usize,
    pub destination_prefetch_cells: usize,
    pub predicted_prefetch_cells: usize,
    pub player_position_ecef: Option<[f64; 3]>,
    pub player_velocity_ecef: Option<[f64; 3]>,
    pub player_speed_ms: f64,
    pub player_airspeed_ms: f64,
    pub player_camera_heading_rad: Option<f64>,
    pub camera: ReplayPose,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct FlightRenderStats {
    pub gpu_sample_id: Option<u64>,
    pub gpu_total_ms: Option<f64>,
    pub gpu_world_ms: Option<f64>,
    pub gpu_shadow_ms: Option<f64>,
    pub resident_gpu_bytes: u64,
    pub upload_bytes: u64,
    pub world_triangles: u64,
    pub opaque_world_indices: u64,
    pub feedback_world_indices: u64,
    pub shadow_caster_indices: u64,
    pub requested_draw_cells: usize,
    pub submitted_chunks: usize,
    pub wanted_cells_by_level: [usize; 32],
    pub drawn_cells_by_level: [usize; 32],
}

impl FlightFrame {
    fn foreground_ready(self) -> bool {
        self.search_arrived
            && self.selection_ready
            && self.foreground_missing == 0
            && self.foreground_unpresented == 0
            && self.foreground_nonopaque == 0
    }
}

#[derive(Debug, Serialize)]
struct PhaseReport {
    phase: Phase,
    nominal_duration_s: Option<f64>,
    started_after_ms: f64,
    ended_after_ms: Option<f64>,
    ended_unix_ms: Option<f64>,
    player_dt_s: f64,
    navigation_dt_s: f64,
    frames: u64,
    surface_presented_frames: u64,
    foreground_ready_frames: u64,
    fully_idle_frames: u64,
    first_foreground_ready_after_ms: Option<f64>,
    first_fully_idle_after_ms: Option<f64>,
    foreground_missing_frames: u64,
    foreground_unpresented_frames: u64,
    foreground_nonopaque_frames: u64,
    background_missing_frames: u64,
    foreground_missing_peak: usize,
    background_missing_peak: usize,
    destination_prefetch_active_frames: u64,
    destination_prefetch_peak_cells: usize,
    predicted_prefetch_active_frames: u64,
    interval_ms_sum: f64,
    interval_ms_max: f64,
    main_ms_max: f64,
    intervals_over_16_67_ms: u64,
    intervals_over_33_33_ms: u64,
    intervals_over_50_ms: u64,
    maximum_ground_speed_ms: f64,
    maximum_airspeed_ms: f64,
    distance_travelled_m: f64,
    keyboard_orbit_input_rad: f64,
    first: Option<FlightFrame>,
    last: Option<FlightFrame>,
    worst_intervals: Vec<FlightFrame>,
}

impl PhaseReport {
    fn new(phase: Phase, started_after_ms: f64) -> Self {
        Self {
            phase,
            nominal_duration_s: phase.duration_s(),
            started_after_ms,
            ended_after_ms: None,
            ended_unix_ms: None,
            player_dt_s: 0.0,
            navigation_dt_s: 0.0,
            frames: 0,
            surface_presented_frames: 0,
            foreground_ready_frames: 0,
            fully_idle_frames: 0,
            first_foreground_ready_after_ms: None,
            first_fully_idle_after_ms: None,
            foreground_missing_frames: 0,
            foreground_unpresented_frames: 0,
            foreground_nonopaque_frames: 0,
            background_missing_frames: 0,
            foreground_missing_peak: 0,
            background_missing_peak: 0,
            destination_prefetch_active_frames: 0,
            destination_prefetch_peak_cells: 0,
            predicted_prefetch_active_frames: 0,
            interval_ms_sum: 0.0,
            interval_ms_max: 0.0,
            main_ms_max: 0.0,
            intervals_over_16_67_ms: 0,
            intervals_over_33_33_ms: 0,
            intervals_over_50_ms: 0,
            maximum_ground_speed_ms: 0.0,
            maximum_airspeed_ms: 0.0,
            distance_travelled_m: 0.0,
            keyboard_orbit_input_rad: 0.0,
            first: None,
            last: None,
            worst_intervals: Vec::new(),
        }
    }

    fn observe(&mut self, frame: FlightFrame, distance_m: f64) {
        self.frames += 1;
        self.player_dt_s += frame.player_dt_s;
        self.navigation_dt_s += (frame.interval_ms * 0.001).min(0.1);
        self.surface_presented_frames += u64::from(frame.surface_presented);
        self.foreground_ready_frames += u64::from(frame.foreground_ready());
        self.fully_idle_frames += u64::from(frame.fully_idle);
        if frame.foreground_ready() {
            self.first_foreground_ready_after_ms
                .get_or_insert(frame.after_start_ms);
        }
        if frame.fully_idle {
            self.first_fully_idle_after_ms
                .get_or_insert(frame.after_start_ms);
        }
        self.foreground_missing_frames += u64::from(frame.foreground_missing > 0);
        self.foreground_unpresented_frames += u64::from(frame.foreground_unpresented > 0);
        self.foreground_nonopaque_frames += u64::from(frame.foreground_nonopaque > 0);
        self.background_missing_frames += u64::from(frame.background_missing > 0);
        self.foreground_missing_peak = self.foreground_missing_peak.max(frame.foreground_missing);
        self.background_missing_peak = self.background_missing_peak.max(frame.background_missing);
        self.destination_prefetch_active_frames += u64::from(frame.destination_prefetch_cells > 0);
        self.destination_prefetch_peak_cells = self
            .destination_prefetch_peak_cells
            .max(frame.destination_prefetch_cells);
        self.predicted_prefetch_active_frames += u64::from(frame.predicted_prefetch_cells > 0);
        self.interval_ms_sum += frame.interval_ms;
        self.interval_ms_max = self.interval_ms_max.max(frame.interval_ms);
        self.main_ms_max = self.main_ms_max.max(frame.main_ms);
        self.intervals_over_16_67_ms += u64::from(frame.interval_ms > 1000.0 / 60.0);
        self.intervals_over_33_33_ms += u64::from(frame.interval_ms > 1000.0 / 30.0);
        self.intervals_over_50_ms += u64::from(frame.interval_ms > 50.0);
        self.maximum_ground_speed_ms = self.maximum_ground_speed_ms.max(frame.player_speed_ms);
        self.maximum_airspeed_ms = self.maximum_airspeed_ms.max(frame.player_airspeed_ms);
        self.distance_travelled_m += distance_m;
        if self.phase == Phase::Turn {
            self.keyboard_orbit_input_rad += 1.6 * frame.player_dt_s;
        }
        self.first.get_or_insert(frame);
        self.last = Some(frame);
        let at = self
            .worst_intervals
            .partition_point(|previous| previous.interval_ms >= frame.interval_ms);
        if at < TAIL_LIMIT {
            self.worst_intervals.insert(at, frame);
            self.worst_intervals.truncate(TAIL_LIMIT);
        }
    }
}

#[derive(Debug, Serialize)]
struct Sample {
    phase: Phase,
    observation: FlightFrame,
}

#[derive(Debug)]
pub(crate) struct DroneBenchmark {
    city_tour: bool,
    phase: Phase,
    phase_started_ms: f64,
    phase_dt_s: f64,
    ready_streak: u64,
    idle_frames: u64,
    wait_limit_ms: f64,
    reports: Vec<PhaseReport>,
    samples: Vec<Sample>,
    last_sample_ms: Option<f64>,
    last_checkpoint_ms: Option<f64>,
    last_position: Option<[f64; 3]>,
    flight_start_position: Option<[f64; 3]>,
}

impl DroneBenchmark {
    pub(crate) fn new(idle_frames: u64, wait_limit_s: f64) -> Self {
        Self {
            city_tour: false,
            phase: Phase::SearchReady,
            phase_started_ms: 0.0,
            phase_dt_s: 0.0,
            ready_streak: 0,
            idle_frames: idle_frames.max(1),
            wait_limit_ms: wait_limit_s * 1000.0,
            reports: vec![PhaseReport::new(Phase::SearchReady, 0.0)],
            samples: Vec::new(),
            last_sample_ms: None,
            last_checkpoint_ms: None,
            last_position: None,
            flight_start_position: None,
        }
    }

    pub(crate) fn awaiting_search(&self) -> bool {
        self.phase == Phase::SearchReady
    }

    pub(crate) fn with_city_tour(mut self, enabled: bool) -> Self {
        self.city_tour = enabled;
        self
    }

    pub(crate) fn explore_controls(&self) -> ExploreControls {
        ExploreControls {
            zoom_in: self.phase == Phase::ZoomIn,
            zoom_out: self.phase == Phase::ZoomOut,
            east: self.phase == Phase::GlobeRotate,
            tilt_down: self.phase == Phase::ZoomOut,
        }
    }

    fn wall_limit_ms(&self) -> f64 {
        if self.city_tour {
            CITY_TOUR_WALL_LIMIT_MS
        } else {
            WALL_LIMIT_MS
        }
    }

    fn sample_limit(&self) -> usize {
        if self.city_tour {
            SAMPLE_LIMIT * 2
        } else {
            SAMPLE_LIMIT
        }
    }

    pub(crate) fn controls(&self) -> PlayKeys {
        let mut controls = self.phase.controls();
        if self.city_tour && self.phase == Phase::Turn {
            controls.fwd = true;
            controls.boost = true;
        }
        controls
    }

    pub(crate) fn deadline_reason(&self, after_start_ms: f64) -> Option<&'static str> {
        if after_start_ms >= self.wall_limit_ms() {
            return Some("drone_global_timeout");
        }
        if after_start_ms - self.phase_started_ms < self.wait_limit_ms {
            return None;
        }
        match self.phase {
            Phase::ChaseReady => Some("drone_chase_ready_timeout"),
            Phase::FinalReady => Some("drone_final_ready_timeout"),
            Phase::ZoomReady => Some("city_zoom_ready_timeout"),
            Phase::GlobeReady => Some("city_globe_ready_timeout"),
            _ => None,
        }
    }

    pub(crate) fn observe(&mut self, frame: FlightFrame) -> Option<Directive> {
        if self.phase == Phase::Complete {
            return None;
        }
        let distance = self
            .last_position
            .zip(frame.player_position_ecef)
            .map_or(0.0, |(a, b)| {
                glam::DVec3::from_array(a).distance(glam::DVec3::from_array(b))
            });
        if frame.player_position_ecef.is_some() {
            self.last_position = frame.player_position_ecef;
        }
        self.reports.last_mut().unwrap().observe(frame, distance);
        if self.samples.len() < self.sample_limit()
            && self
                .last_sample_ms
                .is_none_or(|last| frame.after_start_ms - last >= SAMPLE_INTERVAL_MS)
        {
            self.samples.push(Sample {
                phase: self.phase,
                observation: frame,
            });
            self.last_sample_ms = Some(frame.after_start_ms);
        }
        let ready = frame.foreground_ready()
            && (self.phase.explore() || frame.player_position_ecef.is_some())
            && (!self.city_tour || (frame.surface_presented && frame.fully_idle));
        self.ready_streak = if ready {
            self.ready_streak.saturating_add(1)
        } else {
            0
        };
        self.phase_dt_s += if self.phase.explore() {
            (frame.interval_ms * 0.001).min(0.1)
        } else {
            frame.player_dt_s
        };
        let advance = if let Some(duration) = self.phase.duration_s() {
            self.phase_dt_s >= duration
        } else {
            self.phase != Phase::Complete
                && self.ready_streak >= self.idle_frames
                && frame.selection_settled
        };
        if self.city_tour
            && (advance
                || frame.after_start_ms - self.last_checkpoint_ms.unwrap_or(self.phase_started_ms)
                    >= CHECKPOINT_INTERVAL_MS)
        {
            let report = self.reports.last().unwrap();
            let presented_fps = if report.interval_ms_sum > 0.0 {
                report.surface_presented_frames as f64 * 1000.0 / report.interval_ms_sum
            } else {
                0.0
            };
            log::info!(
                "native city checkpoint: phase={:?}; after-start-ms={:.3}; phase-ms={:.3}; presented={}/{}; observed-presented-fps={:.2}; last-gpu-ms={:?}; last-world-triangles={}; fully-idle={}; missing=foreground:{},guard:{},background:{},desired:{}",
                self.phase,
                frame.after_start_ms,
                frame.after_start_ms - self.phase_started_ms,
                report.surface_presented_frames,
                report.frames,
                presented_fps,
                frame.render.gpu_total_ms,
                frame.render.world_triangles,
                frame.fully_idle,
                frame.foreground_missing,
                frame.guard_missing,
                frame.background_missing,
                frame.desired_missing,
            );
            self.last_checkpoint_ms = Some(frame.after_start_ms);
        }
        if !advance {
            return None;
        }
        let finished = self.phase;
        self.reports.last_mut().unwrap().ended_after_ms = Some(frame.after_start_ms);
        self.reports.last_mut().unwrap().ended_unix_ms = vearth_platform::time::SystemTime::now()
            .duration_since(vearth_platform::time::UNIX_EPOCH)
            .ok()
            .map(|elapsed| elapsed.as_secs_f64() * 1000.0);
        self.phase = self.phase.next(self.city_tour);
        log::info!(
            "native flight phase: {:?} -> {:?}; after-start-ms={:.3}; presented={}/{}",
            finished,
            self.phase,
            frame.after_start_ms,
            self.reports.last().unwrap().surface_presented_frames,
            self.reports.last().unwrap().frames,
        );
        self.phase_started_ms = frame.after_start_ms;
        self.phase_dt_s = 0.0;
        self.ready_streak = 0;
        if self.phase != Phase::Complete {
            self.reports
                .push(PhaseReport::new(self.phase, frame.after_start_ms));
        }
        if self.phase == Phase::Cruise {
            self.flight_start_position = frame.player_position_ecef;
        }
        match finished {
            Phase::SearchReady if self.city_tour => Some(Directive::ResetSelectionFence),
            Phase::SearchReady => Some(Directive::EnterPlay),
            Phase::ZoomReady => Some(Directive::EnterPlay),
            Phase::ZoomIn | Phase::GlobeRotate => Some(Directive::ResetSelectionFence),
            Phase::BrakeReturn => Some(Directive::ResetSelectionFence),
            Phase::FinalReady if self.city_tour => Some(Directive::ExitPlay),
            Phase::GlobeReady => Some(Directive::Finish),
            Phase::FinalReady => Some(Directive::Finish),
            _ => None,
        }
    }

    pub(crate) fn report(&self) -> serde_json::Value {
        let return_distance = self
            .flight_start_position
            .zip(self.last_position)
            .map(|(a, b)| glam::DVec3::from_array(a).distance(glam::DVec3::from_array(b)));
        serde_json::json!({
            "protocol": if self.city_tour { "city_zoom_kestrel_flight_globe_rotation_v2" } else { "kestrel_cruise_boost_keyboard_turn_return_v1" },
            "phase": self.phase,
            "completed": self.phase == Phase::Complete,
            "clock": if self.city_tour {
                "production_keyboard_navigation_and_player_dt_clamped_to_0.1s"
            } else {
                "production_player_dt_clamped_to_0.1s"
            },
            "phase_alignment": "ended_unix_ms_observed_immediately_after_ended_after_ms_for_native_profiler_clock_alignment",
            "wall_limit_ms_from_benchmark_start": self.wall_limit_ms(),
            "all_observed_frames_presented": self.reports.iter().all(|phase| phase.frames == phase.surface_presented_frames),
            "initial_and_final_readiness": if self.city_tour {
                "all_stationary_stops_require_fully_idle_foreground_opaque_and_presented_streak_with_current_selector_fence"
            } else {
                "foreground_opaque_streak_and_current_selector_fence"
            },
            "readiness_is_streaming_not_swapchain_visibility": !self.city_tour,
            "checkpoint_interval_ms": self.city_tour.then_some(CHECKPOINT_INTERVAL_MS),
            "absolute_pose_overrides": false,
            "input": "production_PlayKeys_and_Player_update_with_weather",
            "explore_input": self.city_tour.then_some("production_keyboard_zoom_tilt_and_ground_pan"),
            "turn_under_boost": self.city_tour,
            "sample_limit": self.sample_limit(),
            "sample_interval_ms": SAMPLE_INTERVAL_MS,
            "worst_intervals_per_phase": TAIL_LIMIT,
            "distance_from_flight_start_m": return_distance,
            "phases": self.reports,
            "samples": self.samples,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Player;
    use crate::streaming::ResidentStore;

    fn ready(frame: u64) -> FlightFrame {
        FlightFrame {
            frame,
            after_start_ms: frame as f64 * 100.0,
            player_dt_s: 0.1,
            interval_ms: 100.0,
            main_ms: 2.0,
            render: FlightRenderStats::default(),
            surface_presented: true,
            search_arrived: true,
            selection_ready: true,
            selection_settled: true,
            fully_idle: false,
            foreground_missing: 0,
            foreground_unpresented: 0,
            foreground_nonopaque: 0,
            guard_missing: 3,
            background_missing: 500,
            desired_missing: 503,
            destination_prefetch_cells: 0,
            predicted_prefetch_cells: 0,
            player_position_ecef: Some([6_378_137.0, 0.0, 300.0]),
            player_velocity_ecef: Some([0.0; 3]),
            player_speed_ms: 0.0,
            player_airspeed_ms: 0.0,
            player_camera_heading_rad: Some(0.0),
            camera: ReplayPose {
                frame,
                lat_deg: 51.9225,
                lon_deg: 4.47917,
                alt_m: 300.0,
                heading_deg: 0.0,
                pitch_deg: 65.9,
            },
        }
    }

    fn idle(frame: u64) -> FlightFrame {
        FlightFrame {
            fully_idle: true,
            guard_missing: 0,
            background_missing: 0,
            desired_missing: 0,
            ..ready(frame)
        }
    }

    #[test]
    fn city_tour_waits_for_full_loading_at_every_stationary_stop() {
        for (phase, next, directive) in [
            (
                Phase::SearchReady,
                Phase::ZoomIn,
                Some(Directive::ResetSelectionFence),
            ),
            (
                Phase::ZoomReady,
                Phase::ChaseReady,
                Some(Directive::EnterPlay),
            ),
            (Phase::ChaseReady, Phase::Cruise, None),
            (Phase::FinalReady, Phase::ZoomOut, Some(Directive::ExitPlay)),
            (Phase::GlobeReady, Phase::Complete, Some(Directive::Finish)),
        ] {
            let mut pilot = DroneBenchmark::new(2, 90.0).with_city_tour(true);
            pilot.phase = phase;
            pilot.reports = vec![PhaseReport::new(phase, 0.0)];
            for number in 1..5 {
                let frame = ready(number);
                assert!(frame.foreground_ready());
                assert!(frame.guard_missing > 0 && frame.background_missing > 0);
                assert_eq!(pilot.observe(frame), None);
                assert_eq!(pilot.phase, phase);
            }
            assert_eq!(pilot.observe(idle(5)), None);
            assert_eq!(pilot.observe(ready(6)), None);
            assert_eq!(pilot.ready_streak, 0);
            assert_eq!(pilot.observe(idle(7)), None);
            assert_eq!(pilot.phase, phase);
            assert_eq!(pilot.observe(idle(8)), directive);
            assert_eq!(pilot.phase, next);
            assert_eq!(pilot.reports[0].fully_idle_frames, 3);
            assert_eq!(
                pilot.report()["protocol"],
                "city_zoom_kestrel_flight_globe_rotation_v2"
            );
        }
    }

    #[test]
    fn drone_benchmark_requires_fresh_chase_and_final_foreground_fences() {
        let mut pilot = DroneBenchmark::new(2, 90.0);
        let mut frame = ready(1);
        frame.player_position_ecef = None;
        frame.player_dt_s = 0.0;
        frame.selection_ready = false;
        for _ in 0..8 {
            assert_eq!(pilot.observe(frame), None);
        }
        frame.selection_ready = true;
        frame.selection_settled = false;
        for _ in 0..4 {
            assert_eq!(pilot.observe(frame), None);
        }
        frame.selection_settled = true;
        assert_eq!(pilot.observe(frame), Some(Directive::EnterPlay));
        // A formerly ready aerial cut cannot start the chase-camera cruise.
        frame.selection_ready = false;
        for _ in 0..4 {
            assert_eq!(pilot.observe(frame), None);
        }
        frame = ready(2);
        assert_eq!(pilot.observe(frame), None);
        assert_eq!(pilot.observe(frame), None);
        assert_eq!(pilot.phase, Phase::Cruise);
        assert!(pilot.controls().fwd);
        assert!(!pilot.controls().boost);
        assert!(pilot.reports[1].background_missing_frames > 0);
        pilot.phase = Phase::FinalReady;
        pilot.ready_streak = 0;
        frame.selection_settled = false;
        for _ in 0..4 {
            assert_eq!(pilot.observe(frame), None);
        }
        frame.foreground_nonopaque = 1;
        frame.selection_settled = true;
        assert_eq!(pilot.observe(frame), None);
        frame.foreground_nonopaque = 0;
        assert_eq!(pilot.observe(frame), None);
        assert_eq!(pilot.observe(frame), Some(Directive::Finish));
        assert_eq!(pilot.observe(frame), None);
    }

    #[test]
    fn city_tour_zooms_through_camera_controls_and_waits_for_presentable_fresh_detail() {
        let mut pilot = DroneBenchmark::new(2, 90.0).with_city_tour(true);
        let mut camera = crate::camera::Camera {
            lat: 51.9225_f64.to_radians(),
            lon: 4.47917_f64.to_radians(),
            alt_m: 32_000.0,
            ..crate::camera::Camera::start()
        };
        let mut frame = idle(1);
        frame.player_position_ecef = None;
        frame.player_dt_s = 0.0;
        assert_eq!(pilot.observe(frame), None);
        assert_eq!(pilot.observe(frame), Some(Directive::ResetSelectionFence));
        assert!(pilot.explore_controls().zoom_in);
        for number in 2..100 {
            if pilot.explore_controls().zoom_in {
                camera.zoom(4.0 * 0.1);
            }
            frame.frame = number;
            frame.after_start_ms += 100.0;
            frame.camera = crate::replay::observed_pose(number, &camera);
            if pilot.observe(frame) == Some(Directive::ResetSelectionFence) {
                break;
            }
        }
        assert_eq!(pilot.phase, Phase::ZoomReady);
        assert!(camera.alt_m > 1_000.0 && camera.alt_m < 2_000.0);
        assert_eq!(pilot.explore_controls(), ExploreControls::default());
        frame.selection_ready = false;
        for _ in 0..4 {
            assert_eq!(pilot.observe(frame), None);
        }
        frame.selection_ready = true;
        frame.surface_presented = false;
        for _ in 0..4 {
            assert_eq!(pilot.observe(frame), None);
        }
        frame.surface_presented = true;
        assert_eq!(pilot.observe(frame), None);
        assert_eq!(pilot.observe(frame), Some(Directive::EnterPlay));
        assert_eq!(pilot.phase, Phase::ChaseReady);
        assert_eq!(pilot.report()["all_observed_frames_presented"], false);
        assert!(pilot.deadline_reason(240_000.0) != Some("drone_global_timeout"));
        assert_eq!(
            pilot.deadline_reason(480_000.0),
            Some("drone_global_timeout")
        );
    }

    #[test]
    fn city_tour_exits_play_before_zooming_out_and_retains_separate_idle_observations() {
        let mut pilot = DroneBenchmark::new(1, 90.0).with_city_tour(true);
        pilot.phase = Phase::FinalReady;
        pilot.reports = vec![PhaseReport::new(Phase::FinalReady, 0.0)];
        let mut frame = idle(1);
        assert_eq!(pilot.observe(frame), Some(Directive::ExitPlay));
        assert_eq!(pilot.phase, Phase::ZoomOut);
        assert!(pilot.explore_controls().zoom_out);
        assert!(pilot.explore_controls().tilt_down);
        frame.player_position_ecef = None;
        frame.player_dt_s = 0.0;
        frame.fully_idle = false;
        for _ in 0..201 {
            frame.after_start_ms += 100.0;
            pilot.observe(frame);
        }
        assert_eq!(pilot.phase, Phase::GlobeRotate);
        assert!(pilot.explore_controls().east);
        for _ in 0..51 {
            frame.after_start_ms += 100.0;
            if pilot.observe(frame) == Some(Directive::ResetSelectionFence) {
                break;
            }
        }
        assert_eq!(pilot.phase, Phase::GlobeReady);
        frame.fully_idle = true;
        assert_eq!(pilot.observe(frame), Some(Directive::Finish));
        let report = pilot.report();
        assert_eq!(report["phases"][0]["fully_idle_frames"], 1);
        assert_eq!(report["phases"][3]["fully_idle_frames"], 1);
        assert!(report["phases"][0]["first_fully_idle_after_ms"].is_number());
        assert!(report["phases"][3]["first_fully_idle_after_ms"].is_number());
    }

    #[test]
    fn city_tour_turns_the_real_player_at_boost_speed() {
        let index = ResidentStore::new();
        let start = vearth_core::ellipsoid::geodetic_to_ecef(
            51.9225_f64.to_radians(),
            4.47917_f64.to_radians(),
            300.0,
        );
        let mut player = Player::spawn(start, 0.0);
        let mut pilot = DroneBenchmark::new(1, 90.0).with_city_tour(true);
        pilot.phase = Phase::Boost;
        pilot.reports = vec![PhaseReport::new(Phase::Boost, 0.0)];
        let mut slowest_turn_speed = f64::INFINITY;
        for number in 1..300 {
            let controls = pilot.controls();
            player.update(0.1, &controls, &index);
            if controls.rot_right {
                assert!(controls.fwd && controls.boost);
                slowest_turn_speed = slowest_turn_speed.min(player.speed_ms());
            }
            let mut frame = ready(number);
            frame.player_position_ecef = Some(player.pos.to_array());
            frame.player_speed_ms = player.speed_ms();
            frame.player_camera_heading_rad = Some(player.cam_heading);
            pilot.observe(frame);
            if pilot.phase == Phase::Return {
                break;
            }
        }
        assert_eq!(pilot.phase, Phase::Return);
        assert!(slowest_turn_speed.is_finite() && slowest_turn_speed > 60.0);
        let turn = pilot
            .reports
            .iter()
            .find(|p| p.phase == Phase::Turn)
            .unwrap();
        assert!(turn.distance_travelled_m > 100.0);
        assert!(turn.keyboard_orbit_input_rad >= std::f64::consts::PI);
    }

    #[test]
    fn drone_benchmark_uses_player_dt_and_bounds_stalled_frame_reporting() {
        let mut pilot = DroneBenchmark::new(1, 90.0);
        pilot.observe(ready(1));
        pilot.observe(ready(2));
        assert_eq!(pilot.phase, Phase::Cruise);
        let mut frame = ready(3);
        frame.player_dt_s = 0.0;
        for number in 0..2_000 {
            frame.frame = number;
            frame.after_start_ms = number as f64 * 501.0;
            frame.interval_ms = number as f64;
            assert_eq!(pilot.observe(frame), None);
        }
        assert_eq!(pilot.phase_dt_s, 0.0);
        assert_eq!(pilot.samples.len(), SAMPLE_LIMIT);
        assert_eq!(
            pilot.reports.last().unwrap().worst_intervals.len(),
            TAIL_LIMIT
        );
        assert_eq!(pilot.reports.last().unwrap().interval_ms_max, 1999.0);
        assert_eq!(
            pilot.deadline_reason(240_000.0),
            Some("drone_global_timeout")
        );
        let mut chase = DroneBenchmark::new(1, 10.0);
        chase.observe(ready(1));
        assert_eq!(chase.deadline_reason(10_099.0), None);
        assert_eq!(
            chase.deadline_reason(10_100.0),
            Some("drone_chase_ready_timeout")
        );
        chase.phase = Phase::FinalReady;
        assert_eq!(
            chase.deadline_reason(10_100.0),
            Some("drone_final_ready_timeout")
        );
    }

    #[test]
    fn drone_benchmark_protocol_drives_real_player_translation_boost_and_orbit() {
        let index = ResidentStore::new();
        let start = vearth_core::ellipsoid::geodetic_to_ecef(
            51.9225_f64.to_radians(),
            4.47917_f64.to_radians(),
            300.0,
        );
        let mut player = Player::spawn(start, 0.0);
        let mut pilot = DroneBenchmark::new(1, 90.0);
        let mut actions = [0; 3];
        let mut camera = crate::camera::Camera {
            lat: 51.9225_f64.to_radians(),
            lon: 4.47917_f64.to_radians(),
            alt_m: 300.0,
            ..crate::camera::Camera::start()
        };
        for number in 1..2_000 {
            let controls = pilot.controls();
            if !pilot.awaiting_search() {
                player.update(0.1, &controls, &index);
                let (eye, forward) = player.camera_pose(0.1, &index);
                camera.set_pose_free(eye, forward);
            }
            let mut frame = ready(number);
            frame.player_dt_s = if pilot.awaiting_search() { 0.0 } else { 0.1 };
            frame.player_position_ecef = Some(player.pos.to_array());
            frame.player_velocity_ecef = Some(player.vel.to_array());
            frame.player_speed_ms = player.speed_ms();
            frame.player_airspeed_ms = player.airspeed_ms();
            frame.player_camera_heading_rad = Some(player.cam_heading);
            frame.camera = crate::replay::observed_pose(number, &camera);
            if let Some(action) = pilot.observe(frame) {
                let ordinal = match action {
                    Directive::EnterPlay => 0,
                    Directive::ResetSelectionFence => 1,
                    Directive::Finish => 2,
                    Directive::ExitPlay => panic!("legacy drone protocol does not exit Play"),
                };
                actions[ordinal] += 1;
                if action == Directive::Finish {
                    break;
                }
            }
        }
        assert_eq!(actions, [1, 1, 1]);
        assert_eq!(pilot.phase, Phase::Complete);
        let phase = |name| {
            pilot
                .reports
                .iter()
                .find(|report| report.phase == name)
                .unwrap()
        };
        let cruise = phase(Phase::Cruise);
        let boost = phase(Phase::Boost);
        assert!(cruise.maximum_airspeed_ms > 23.0 && cruise.maximum_airspeed_ms < 25.0);
        assert!(boost.maximum_airspeed_ms > 75.0 && boost.maximum_airspeed_ms < 77.0);
        assert!(cruise.distance_travelled_m > 450.0);
        assert!(boost.distance_travelled_m > 1400.0);
        let turn = phase(Phase::Turn);
        assert!(turn.keyboard_orbit_input_rad >= std::f64::consts::PI);
        assert!(turn.keyboard_orbit_input_rad < std::f64::consts::PI + 0.161);
        let headings = (
            turn.first.unwrap().player_camera_heading_rad.unwrap(),
            turn.last.unwrap().player_camera_heading_rad.unwrap(),
        );
        assert!(headings.1 - headings.0 > 2.9);
        let returning = phase(Phase::Return);
        assert!(returning.distance_travelled_m > 1400.0);
        let first_velocity =
            glam::DVec3::from_array(boost.last.unwrap().player_velocity_ecef.unwrap());
        let return_velocity =
            glam::DVec3::from_array(returning.last.unwrap().player_velocity_ecef.unwrap());
        assert!(first_velocity.normalize().dot(return_velocity.normalize()) < -0.98);
        let outbound = glam::DVec3::from_array(turn.last.unwrap().player_position_ecef.unwrap());
        assert!(player.pos.distance(start) < outbound.distance(start));
        assert!(
            pilot.report()["distance_from_flight_start_m"]
                .as_f64()
                .unwrap()
                < 700.0
        );
    }
}
