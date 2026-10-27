//! The interactive native application: window, input, frame loop, streaming.
//!
//! `resumed` creates the window and initializes the GPU before assembling
//! application state.

use crate::admission::{IntentClass, UploadPriority, UploadStager, upload_fits_budget};
use crate::camera::Camera;
use crate::camera::FlyTo;
use crate::capture::{CaptureFrame, PhotoRecipe, PhotoSettings, PhotoStatus, still_render_samples};
#[cfg(not(target_arch = "wasm32"))]
use crate::capture::{render_rgba, write_photo_bundle, write_screenshot};
use crate::egui_input::UiEvent;
use crate::gfx::{DrawCell, Gfx, PlayerDraw};
use crate::host::Window;
use crate::input::{
    ElementState, KeyCode, ModifiersState, MouseButton, MouseScrollDelta, PhysicalKey, WindowEvent,
};
use crate::local_time::LocalTimeTracker;
use crate::location::LocationTracker;
use crate::lod_selection::{
    LodSelector, Selection, SelectionMode, SelectionRequest, TerrainMetric, compute_selection,
};
#[cfg(target_os = "macos")]
use crate::native_menu::{NativeMenu, NativeMenuAction, NativeMenuState};
use crate::platform_paths::{RuntimePaths, WINDOW_TITLE};
use crate::player::{PlayKeys, Player};
use crate::replay::{ReplayTrace, SearchBenchmarkPlayer, SearchBenchmarkSpec, TracePlayer};
use crate::residency::{LeaseBook, LeaseKind, ResidencyBudget, ResidentCell, eviction_plan};
use crate::settings::{
    CacheUsageTracker, PanSpeedPreference, PendingCacheClear, PreferencesStore,
    SolarSpeedPreference, UserPreferences, VehiclePreference, WeatherQualityPreference,
    WindPreference,
};
use crate::solar::{SolarClock, SolarStart};
use crate::streaming::{DrawableCutArena, LodView, ResidentStore, ViewLane, WantedCell};
use crate::telemetry::{Distribution, PipelineTotals, RollingHistogram, SmoothnessReport};
use crate::telemetry::{FrameMeasurement, SmoothnessTelemetry};
use crate::transition::{CoverageIndex, LodTransitions};
use crate::ui::{HudInfo, LaunchUi, PlayHud, StorageHud, SunHud, Ui, UiAction, WeatherHud};
use crate::vehicles::VehicleKind;
use crate::weather::{WeatherConfig, WeatherMode, WeatherModel, WeatherQuality, WindInfluence};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use vearth_core::cell::CellId;
use vearth_core::cell_key::CellKey;
use vearth_platform::time::{Duration, Instant};
#[cfg(not(target_arch = "wasm32"))]
use winit::application::ApplicationHandler;
#[cfg(not(target_arch = "wasm32"))]
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
#[cfg(not(target_arch = "wasm32"))]
use winit::window::WindowId;

#[cfg(target_arch = "wasm32")]
#[path = "app_web.rs"]
pub mod web;

type Pool = crate::streaming::GenPool;

const WANTED_REFRESH: Duration = Duration::from_millis(120);
/// LOD crossfade duration. Long enough to read as "resolving", short
/// enough that a dive never shows more than one transition per area.
const FADE_S: f64 = 0.30;
/// GPU uploads are the render-thread admission boundary. A single mesh may
/// exceed the byte budget and is still admitted to guarantee progress.
/// Dense city views hold gigabytes of prepared geometry and render slowly
/// while loading, so a per-frame bound sized for light scenes leaves CPU
/// workers idle behind the upload queue. Both bounds apply per frame.
const UPLOAD_BYTES_PER_FRAME: usize = 64 * 1024 * 1024;
const UPLOAD_TIME_PER_FRAME: Duration = Duration::from_millis(6);
/// High watermark for generated meshes waiting on GPU admission. Intake
/// stops at this point; one mesh may cross it so an oversized cell can still
/// make progress without ever being discarded and regenerated.
const READY_MESH_BYTES: usize = 256 * 1024 * 1024;
/// Bound completion classification work as well as bytes in a single frame.
const COMPLETION_DRAINS_PER_FRAME: usize = 128;
const PRESENTED_LEASE_FRAMES: u64 = 4;
const DESTINATION_LEASE_FRAMES: u64 = 60 * 15;
/// Match fullscreen media controls: leave enough time to acquire a HUD
/// target, then remove the pointer from the scene until it moves again.
const CURSOR_IDLE_TIMEOUT: Duration = Duration::from_secs(2);
/// Explicit native replay-benchmark cadence. The surface present mode alone
/// does not guarantee 60 Hz on macOS (FIFO may follow a 90/120 Hz display).
const REPLAY_BENCHMARK_FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
const OCCLUDED_FRAME_INTERVAL: Duration = Duration::from_millis(100);

fn redraw_deadline(unpresentable: bool, last_redraw: Option<Instant>, now: Instant) -> Instant {
    if unpresentable {
        last_redraw.map_or(now, |last| (last + OCCLUDED_FRAME_INTERVAL).max(now))
    } else {
        now
    }
}

fn lease_presented_cells(
    draw: &[DrawCell],
    index: &mut ResidentStore,
    leases: &mut LeaseBook,
    frame: u64,
) {
    // The drawable cut already contains every ancestor needed to cover gaps,
    // and transitions retain both endpoints. Leasing additional ancestors
    // would keep superseded geometry resident for as long as its leaves exist.
    for dc in draw {
        index.touch_id(dc.resident, frame);
        let kind = if dc.fade.abs() < 1.0 || dc.fade.is_sign_negative() {
            LeaseKind::Transition
        } else {
            LeaseKind::Presented
        };
        leases.protect(dc.cell, kind, frame.saturating_add(PRESENTED_LEASE_FRAMES));
    }
}

fn replay_benchmark_next_deadline(deadline: Instant, started: Instant) -> Instant {
    // Keep the scheduled phase despite timer overshoot. Under load, skip
    // expired ticks without delaying an already-late frame or making bursts.
    let interval_ns = REPLAY_BENCHMARK_FRAME_INTERVAL.as_nanos();
    let ticks = started.saturating_duration_since(deadline).as_nanos() / interval_ns + 1;
    deadline + Duration::from_nanos((ticks * interval_ns) as u64)
}

fn direct_vehicle_key(key: &PhysicalKey) -> Option<VehicleKind> {
    match key {
        PhysicalKey::Code(KeyCode::Digit1 | KeyCode::Numpad1) => Some(VehicleKind::Kestrel),
        PhysicalKey::Code(KeyCode::Digit2 | KeyCode::Numpad2) => Some(VehicleKind::PhaseManta),
        PhysicalKey::Code(KeyCode::Digit3 | KeyCode::Numpad3) => Some(VehicleKind::Sparrow),
        _ => None,
    }
}

#[cfg(test)]
mod replay_benchmark_pacing_tests {
    use super::*;

    #[test]
    fn occluded_redraws_retry_without_spinning_and_resume_immediately() {
        let now = Instant::now();
        assert_eq!(redraw_deadline(true, None, now), now);
        assert_eq!(
            redraw_deadline(true, Some(now), now + Duration::from_millis(5)),
            now + OCCLUDED_FRAME_INTERVAL
        );
        let due = now + OCCLUDED_FRAME_INTERVAL;
        assert_eq!(redraw_deadline(true, Some(now), due), due);
        assert_eq!(
            redraw_deadline(true, Some(now), due + Duration::from_secs(1)),
            due + Duration::from_secs(1)
        );
        assert_eq!(redraw_deadline(false, Some(now), now), now);
    }

    #[test]
    fn benchmark_cadence_does_not_accumulate_timer_overshoot() {
        let origin = Instant::now();
        let mut deadline = origin;
        for tick in 0..120 {
            let started = deadline + Duration::from_millis(2);
            deadline = replay_benchmark_next_deadline(deadline, started);
            assert_eq!(
                deadline,
                origin + REPLAY_BENCHMARK_FRAME_INTERVAL * (tick + 1)
            );
        }
    }

    #[test]
    fn benchmark_cadence_skips_missed_ticks_without_catchup_bursts() {
        let origin = Instant::now();
        let started = origin + Duration::from_millis(108);
        let next = replay_benchmark_next_deadline(origin, started);
        assert_eq!(next, origin + REPLAY_BENCHMARK_FRAME_INTERVAL * 7);
        assert!(next > started);
        assert!(next - started <= REPLAY_BENCHMARK_FRAME_INTERVAL);
        assert_eq!(
            replay_benchmark_next_deadline(next, next),
            next + REPLAY_BENCHMARK_FRAME_INTERVAL
        );
    }
}

fn legacy_residency_enabled() -> bool {
    vearth_platform::env::var("VEARTH_RESIDENCY")
        .is_ok_and(|value| value.eq_ignore_ascii_case("legacy"))
}

pub(crate) fn legacy_lod_enabled() -> bool {
    vearth_platform::env::var("VEARTH_LOD").is_ok_and(|value| value.eq_ignore_ascii_case("legacy"))
}

fn transition_validation_enabled() -> bool {
    vearth_platform::env::var("VEARTH_VALIDATE_TRANSITIONS")
        .is_ok_and(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
}

/// Replays remain synchronous by default so frame-indexed benchmark traces
/// are deterministic. This opt-in exists only to measure the interactive
/// worker path against the same recorded camera motion.
fn async_replay_lod_enabled() -> bool {
    vearth_platform::env::var("VEARTH_ASYNC_REPLAY_LOD")
        .is_ok_and(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
}

fn lod_worker_enabled() -> bool {
    !vearth_platform::env::var("VEARTH_LOD_WORKER").is_ok_and(|value| {
        value == "0"
            || value.eq_ignore_ascii_case("off")
            || value.eq_ignore_ascii_case("false")
            || value.eq_ignore_ascii_case("legacy")
    })
}

/// Starting camera position filled from CLI flags.
#[derive(Clone, Copy)]
pub struct StartPos {
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub alt_m: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
    /// Start directly in play mode with the selected vehicle at this position.
    pub play: bool,
    /// Initial Sun/time mode. Live astronomical time is the default.
    pub solar_start: SolarStart,
    /// Simulation seconds per real second; mode-specific default when None.
    pub time_scale: Option<f64>,
    /// Deterministic weather and wind controls.
    pub weather: WeatherConfig,
}

/// Opt-in lifecycle/report controls for a finite native replay benchmark.
/// Ordinary `--replay` runs keep their historical hold-at-final-pose behavior.
#[derive(Clone, Debug)]
pub struct ReplayBenchmarkOptions {
    pub report_path: PathBuf,
    pub trace_path: String,
    pub max_settle: Duration,
    pub idle_frames: u64,
    pub surface_size: Option<(u32, u32)>,
    pub measurement_range: Option<crate::replay_measurement::MeasurementRange>,
}

struct ReplayBenchmarkState {
    options: ReplayBenchmarkOptions,
    started_at: Instant,
    motion_finished_at: Option<Instant>,
    motion_report: Option<SmoothnessReport>,
    visible_idle_started_at: Option<Instant>,
    visible_idle_frames: u64,
    visible_converged_after_ms: Option<f64>,
    foreground_idle_started_at: Option<Instant>,
    foreground_idle_frames: u64,
    foreground_converged_after_ms: Option<f64>,
    lane_counts: [ReplayLaneCounts; 4],
    lane_series: [ReplayLaneSeries; 4],
    convergence: ReplayConvergenceDiagnostics,
    full_idle_started_at: Option<Instant>,
    full_idle_frames: u64,
    report_written: bool,
    last_triangles: [u64; 3],
    last_desired_missing: usize,
    last_wanted_presented_delta: usize,
    progress_checkpoint_at: Instant,
    progress_surface_frames: [u64; 2],
    progress_last_gpu_ms: Option<f64>,
    measurement: Option<crate::replay_measurement::ReplayMeasurement>,
}

const REPLAY_LANES: [&str; 4] = ["visible", "guard", "background", "legacy"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ReplayLaneCounts {
    desired: usize,
    missing: usize,
    presented: usize,
    opaque_presented: usize,
}

#[derive(Default)]
struct ReplayLaneSeries {
    missing: RollingHistogram,
    presented: RollingHistogram,
}

fn replay_lane_counts(
    wanted: &[WantedCell],
    presented: &HashMap<CellId, f32>,
    is_resident: impl Fn(CellId) -> bool,
) -> [ReplayLaneCounts; 4] {
    let mut lanes = [ReplayLaneCounts::default(); 4];
    for wanted in wanted {
        let lane = &mut lanes[match wanted.lane {
            ViewLane::Visible => 0,
            ViewLane::Guard => 1,
            ViewLane::Background => 2,
            ViewLane::Legacy => 3,
        }];
        lane.desired += 1;
        lane.missing += usize::from(!is_resident(wanted.cell));
        if let Some(&fade) = presented.get(&wanted.cell) {
            lane.presented += 1;
            lane.opaque_presented += usize::from(fade == 1.0);
        }
    }
    lanes
}

fn foreground_opaque_idle(lanes: &[ReplayLaneCounts; 4]) -> bool {
    [0, 3].into_iter().all(|i| {
        let lane = lanes[i];
        lane.missing == 0 && lane.opaque_presented == lane.desired
    })
}

// Records explain observer state; they never participate in readiness decisions.
const CONVERGENCE_RECORD_LIMIT: usize = 16;
const CONVERGENCE_CELL_LIMIT: usize = 16;
const CONVERGENCE_SAMPLE_INTERVAL_MS: f64 = 500.0;

struct ReplayPresentation {
    desired: HashSet<CellId>,
    presented: HashMap<CellId, f32>,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
struct ReplayConvergenceFrame {
    frame: u64,
    after_trace_ms: f64,
    foreground_idle_frames: u64,
    full_idle_frames: u64,
    foreground_missing: usize,
    foreground_unpresented: usize,
    foreground_nonopaque: usize,
    desired_missing: usize,
    wanted_presented_delta: usize,
    selection_ready: bool,
    selection_settled: bool,
    selected_topology_changes: u64,
    // Other work is evaluated by the existing full-idle predicate only when
    // the entire requested cut is present without extra transition draws.
    work_pending_after_cut_ready: Option<bool>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct ReplayConvergenceReset {
    observation: ReplayConvergenceFrame,
    foreground_streak_ended: Option<u64>,
    full_idle_streak_ended: Option<u64>,
    selected_topology_changed: bool,
    reasons: Vec<&'static str>,
}

#[derive(Debug, serde::Serialize)]
struct ReplayConvergenceCell {
    cell: [u32; 4],
    role: &'static str,
    lane: Option<&'static str>,
    resident: bool,
    fade: Option<f32>,
    fade_bits: Option<u32>,
    phase: Option<&'static str>,
}

#[derive(Debug, serde::Serialize)]
struct ReplayConvergenceSample {
    observation: ReplayConvergenceFrame,
    selected_topology_changed_since_previous_sample: bool,
    cells: Vec<ReplayConvergenceCell>,
}

#[derive(Default, serde::Serialize)]
struct ReplayConvergenceDiagnostics {
    longest_foreground_idle_frames: u64,
    longest_full_idle_frames: u64,
    foreground_resets: u64,
    full_idle_resets: u64,
    last_resets: std::collections::VecDeque<ReplayConvergenceReset>,
    near_idle_samples: std::collections::VecDeque<ReplayConvergenceSample>,
    #[serde(skip)]
    previous: Option<ReplayConvergenceFrame>,
    #[serde(skip)]
    last_sample_ms: Option<f64>,
    #[serde(skip)]
    last_sample_topology_changes: u64,
}

fn retain_convergence_record<T>(records: &mut std::collections::VecDeque<T>, record: T) {
    if records.len() == CONVERGENCE_RECORD_LIMIT {
        records.pop_front();
    }
    records.push_back(record);
}

impl ReplayConvergenceDiagnostics {
    fn observe(&mut self, frame: ReplayConvergenceFrame) -> bool {
        self.longest_foreground_idle_frames = self
            .longest_foreground_idle_frames
            .max(frame.foreground_idle_frames);
        self.longest_full_idle_frames = self.longest_full_idle_frames.max(frame.full_idle_frames);
        if let Some(previous) = self.previous {
            let foreground_streak_ended = (previous.foreground_idle_frames > 0
                && frame.foreground_idle_frames == 0)
                .then_some(previous.foreground_idle_frames);
            let full_idle_streak_ended = (previous.full_idle_frames > 0
                && frame.full_idle_frames == 0)
                .then_some(previous.full_idle_frames);
            self.foreground_resets += u64::from(foreground_streak_ended.is_some());
            self.full_idle_resets += u64::from(full_idle_streak_ended.is_some());
            if foreground_streak_ended.is_some() || full_idle_streak_ended.is_some() {
                let topology_changed =
                    frame.selected_topology_changes != previous.selected_topology_changes;
                let mut reasons = Vec::new();
                for (active, name) in [
                    (!frame.selection_ready, "selection_not_ready"),
                    (
                        foreground_streak_ended.is_some() && frame.foreground_missing > 0,
                        "foreground_missing",
                    ),
                    (
                        foreground_streak_ended.is_some() && frame.foreground_unpresented > 0,
                        "foreground_unpresented",
                    ),
                    (
                        foreground_streak_ended.is_some() && frame.foreground_nonopaque > 0,
                        "visible_nonopaque",
                    ),
                    (
                        full_idle_streak_ended.is_some() && frame.desired_missing > 0,
                        "desired_missing",
                    ),
                    (
                        full_idle_streak_ended.is_some() && frame.wanted_presented_delta > 0,
                        "presentation_difference",
                    ),
                    (
                        frame.work_pending_after_cut_ready == Some(true),
                        "work_pending",
                    ),
                    (topology_changed, "selected_topology_change"),
                ] {
                    if active {
                        reasons.push(name);
                    }
                }
                retain_convergence_record(
                    &mut self.last_resets,
                    ReplayConvergenceReset {
                        observation: frame,
                        foreground_streak_ended,
                        full_idle_streak_ended,
                        selected_topology_changed: topology_changed,
                        reasons,
                    },
                );
            }
        }
        self.previous = Some(frame);
        let near_idle = frame.desired_missing <= CONVERGENCE_CELL_LIMIT
            || (frame.foreground_missing <= CONVERGENCE_CELL_LIMIT
                && frame.foreground_unpresented <= CONVERGENCE_CELL_LIMIT
                && frame.foreground_nonopaque <= CONVERGENCE_CELL_LIMIT);
        near_idle
            && self
                .last_sample_ms
                .is_none_or(|last| frame.after_trace_ms - last >= CONVERGENCE_SAMPLE_INTERVAL_MS)
    }

    fn record_sample(&mut self, frame: ReplayConvergenceFrame, cells: Vec<ReplayConvergenceCell>) {
        let topology_changed = self.last_sample_ms.is_some()
            && frame.selected_topology_changes != self.last_sample_topology_changes;
        self.last_sample_ms = Some(frame.after_trace_ms);
        self.last_sample_topology_changes = frame.selected_topology_changes;
        retain_convergence_record(
            &mut self.near_idle_samples,
            ReplayConvergenceSample {
                observation: frame,
                selected_topology_changed_since_previous_sample: topology_changed,
                cells,
            },
        );
    }
}

fn convergence_cell_samples(
    wanted: &[WantedCell],
    presentation: &ReplayPresentation,
    is_resident: impl Fn(CellId) -> bool,
    phase: impl Fn(CellId) -> Option<crate::transition::TransitionPhase>,
) -> Vec<ReplayConvergenceCell> {
    // Keep deterministic representatives without sorting or retaining the
    // whole candidate cohort. Reserve room for both foreground and extra draws.
    fn insert_bounded(
        group: &mut Vec<(CellId, Option<ViewLane>)>,
        cell: CellId,
        lane: Option<ViewLane>,
    ) {
        let at = group.partition_point(|(existing, _)| *existing < cell);
        if at < 8 {
            group.insert(at, (cell, lane));
            group.truncate(8);
        }
    }
    let mut foreground = Vec::new();
    let mut other = Vec::new();
    let mut extra = Vec::new();
    for wanted in wanted {
        if is_resident(wanted.cell) && presentation.presented.get(&wanted.cell) == Some(&1.0) {
            continue;
        }
        let group = if matches!(wanted.lane, ViewLane::Visible | ViewLane::Legacy) {
            &mut foreground
        } else {
            &mut other
        };
        insert_bounded(group, wanted.cell, Some(wanted.lane));
    }
    for &cell in presentation.presented.keys() {
        if !presentation.desired.contains(&cell) {
            insert_bounded(&mut extra, cell, None);
        }
    }
    foreground
        .into_iter()
        .map(|item| (item, "foreground_target"))
        .chain(extra.into_iter().map(|item| (item, "extra_draw")))
        .chain(other.into_iter().map(|item| (item, "other_target")))
        .take(CONVERGENCE_CELL_LIMIT)
        .map(|((cell, lane), role)| {
            let fade = presentation.presented.get(&cell).copied();
            ReplayConvergenceCell {
                cell: [
                    u32::from(cell.face.0),
                    u32::from(cell.level),
                    cell.i,
                    cell.j,
                ],
                role,
                lane: lane.map(|lane| match lane {
                    ViewLane::Visible => "visible",
                    ViewLane::Guard => "guard",
                    ViewLane::Background => "background",
                    ViewLane::Legacy => "legacy",
                }),
                resident: is_resident(cell),
                fade,
                fade_bits: fade.map(f32::to_bits),
                phase: phase(cell).map(|phase| match phase {
                    crate::transition::TransitionPhase::Incoming => "incoming",
                    crate::transition::TransitionPhase::Stable => "stable",
                    crate::transition::TransitionPhase::Outgoing => "outgoing",
                }),
            }
        })
        .collect()
}

struct PhotoJob {
    settings: PhotoSettings,
    width: u32,
    height: u32,
    camera: Camera,
    solar: SolarClock,
    weather: WeatherModel,
}

impl ReplayBenchmarkState {
    fn new(options: ReplayBenchmarkOptions, started_at: Instant) -> Self {
        let measurement = options
            .measurement_range
            .map(crate::replay_measurement::ReplayMeasurement::new);
        Self {
            options,
            started_at,
            motion_finished_at: None,
            motion_report: None,
            visible_idle_started_at: None,
            visible_idle_frames: 0,
            visible_converged_after_ms: None,
            foreground_idle_started_at: None,
            foreground_idle_frames: 0,
            foreground_converged_after_ms: None,
            lane_counts: [ReplayLaneCounts::default(); 4],
            lane_series: std::array::from_fn(|_| ReplayLaneSeries::default()),
            convergence: ReplayConvergenceDiagnostics::default(),
            full_idle_started_at: None,
            full_idle_frames: 0,
            report_written: false,
            last_triangles: [0; 3],
            last_desired_missing: 0,
            last_wanted_presented_delta: 0,
            progress_checkpoint_at: started_at,
            progress_surface_frames: [0; 2],
            progress_last_gpu_ms: None,
            measurement,
        }
    }

    fn record_lanes(&mut self, counts: [ReplayLaneCounts; 4]) {
        self.lane_counts = counts;
        for (series, count) in self.lane_series.iter_mut().zip(counts) {
            series.missing.record(count.missing as f64);
            series.presented.record(count.presented as f64);
        }
    }
}

fn distribution_json(distribution: Distribution) -> serde_json::Value {
    serde_json::json!({
        "count": distribution.count,
        "p50": distribution.p50,
        "p95": distribution.p95,
        "p99": distribution.p99,
        "max": distribution.max,
    })
}

fn totals_json(totals: PipelineTotals) -> serde_json::Value {
    serde_json::json!({
        "frames": totals.frames,
        "missed_vsync": totals.missed_vsync,
        "uploaded_cells": totals.uploaded_cells,
        "uploaded_bytes": totals.uploaded_bytes,
        "evicted_cells": totals.evicted_cells,
        "evicted_bytes": totals.evicted_bytes,
        "stale_results": totals.stale_results,
        "stale_gpu_admissions": totals.stale_gpu_admissions,
        "ready_backpressure_drops": totals.ready_backpressure_drops,
        "draw_overflow": totals.draw_overflow,
        "sun_shadow_capacity_drops": totals.sun_shadow_capacity_drops,
        "transitions_started": totals.transitions_started,
        "transitions_completed": totals.transitions_completed,
        "transition_reversals": totals.transition_reversals,
        "uncovered_transitions": totals.uncovered_transitions,
        "wanted_budget_saturated": totals.wanted_budget_saturated,
        "wanted_topology_changes": totals.wanted_topology_changes,
        "protected_eviction_attempts": totals.protected_eviction_attempts,
    })
}

fn smoothness_json(report: SmoothnessReport) -> serde_json::Value {
    serde_json::json!({
        "frame_interval_ms": distribution_json(report.frame_interval_ms),
        "main_ms": distribution_json(report.main_ms),
        "wanted_ms": distribution_json(report.wanted_ms),
        "pump_ms": distribution_json(report.pump_ms),
        "upload_ms": distribution_json(report.upload_ms),
        "resolve_ms": distribution_json(report.resolve_ms),
        "transition_ms": distribution_json(report.transition_ms),
        "residency_ms": distribution_json(report.residency_ms),
        "eviction_ms": distribution_json(report.eviction_ms),
        "render_submit_ms": distribution_json(report.render_submit_ms),
        "cpu_visibility_ms": distribution_json(report.cpu_visibility_ms),
        "cpu_world_draw_construction_ms": distribution_json(report.cpu_world_draw_construction_ms),
        "cpu_sorting_ms": distribution_json(report.cpu_sorting_ms),
        "cpu_uniform_construction_ms": distribution_json(report.cpu_uniform_construction_ms),
        "cpu_grouping_ms": distribution_json(report.cpu_grouping_ms),
        "cpu_indirect_construction_ms": distribution_json(report.cpu_indirect_construction_ms),
        "cpu_sun_shadow_preparation_ms": distribution_json(report.cpu_sun_shadow_preparation_ms),
        "upload_preparation_ms": distribution_json(report.upload_preparation_ms),
        "queue_upload_ms": distribution_json(report.queue_upload_ms),
        "gpu_ms": distribution_json(report.gpu_ms),
        "gpu_shadow_ms": distribution_json(report.gpu_shadow_ms),
        "gpu_world_ms": distribution_json(report.gpu_world_ms),
        "gpu_flora_ms": distribution_json(report.gpu_flora_ms),
        "gpu_atmosphere_ms": distribution_json(report.gpu_atmosphere_ms),
        "gpu_cloud_ms": distribution_json(report.gpu_cloud_ms),
        "gpu_precipitation_ms": distribution_json(report.gpu_precipitation_ms),
        "gpu_tonemap_ms": distribution_json(report.gpu_tonemap_ms),
        "completion_backlog": distribution_json(report.completion_backlog),
        "oldest_ready_ms": distribution_json(report.oldest_ready_ms),
        "wanted_presented_delta": distribution_json(report.wanted_presented_delta),
        "world_draw_batches": distribution_json(report.world_draw_batches),
        "sun_shadow_caster_chunks": distribution_json(report.sun_shadow_caster_chunks),
        "sun_shadow_caster_indices": distribution_json(report.sun_shadow_caster_indices),
        "sun_shadow_batches": distribution_json(report.sun_shadow_batches),
        "submitted_world_vertices": distribution_json(report.submitted_world_vertices),
        "world_material_indices": report.world_material_indices,
        "opaque_world_indices": distribution_json(report.opaque_world_indices),
        "feedback_world_indices": distribution_json(report.feedback_world_indices),
        "triangles": distribution_json(report.triangles),
        "frame_upload_bytes": distribution_json(report.frame_upload_bytes),
        "resident_gpu_bytes": distribution_json(report.resident_gpu_bytes),
        "totals": totals_json(report.totals),
    })
}

fn measurement_smoothness_json(report: SmoothnessReport) -> serde_json::Value {
    let mut value = smoothness_json(report);
    // Window rows retain only these totals; other cumulative pipeline events
    // remain in the unchanged motion/final reports.
    value["totals"] = serde_json::json!({
        "frames": report.totals.frames,
        "missed_vsync": report.totals.missed_vsync,
        "uploaded_cells": report.totals.uploaded_cells,
        "uploaded_bytes": report.totals.uploaded_bytes,
    });
    value
}

fn roof_work_json(work: vearth_gen::contracts::RoofWorkCounters) -> serde_json::Value {
    let rejections: std::collections::BTreeMap<_, _> = vearth_gen::contracts::ROOF_PLANE_REJECTIONS
        .into_iter()
        .zip(work.plane_rejections)
        .collect();
    serde_json::json!({
        "roofs": work.roofs,
        "parent_roofs": work.parent_roofs,
        "simple_roofs": work.simple_roofs,
        "measured_roofs": work.measured_roofs,
        "elapsed_ns": work.elapsed_ns,
        "analytic_attempts": work.analytic_attempts,
        "analytic_accepted": work.analytic_accepted,
        "initial_cells": work.initial_cells,
        "coarse_grid_fallbacks": work.coarse_grid_fallbacks,
        "plane_attempts": work.plane_attempts,
        "plane_successes": work.plane_successes,
        "plane_concavity_rejections": work.plane_concavity_rejections,
        "plane_rejections": rejections,
        "refinement_visits_by_depth": work.refinement_visits,
        "fallback_depth": work.fallback_depth,
        "fallback_budget": work.fallback_budget,
        "fallback_invalid": work.fallback_invalid,
        "discarded_triangles": work.discarded_triangles,
        "triangles": work.triangles,
        "elapsed_buckets": work.elapsed_buckets,
    })
}

fn roof_diagnostics_json(
    stages: &vearth_gen::contracts::GenerationStageSnapshot,
) -> serde_json::Value {
    let by_shape: std::collections::BTreeMap<_, _> = vearth_gen::contracts::ROOF_DIAGNOSTIC_SHAPES
        .into_iter()
        .zip(&stages.roof_by_shape_and_level)
        .filter_map(|(name, levels)| {
            let observed: std::collections::BTreeMap<_, _> = levels
                .iter()
                .enumerate()
                .filter(|(_, work)| work.roofs != 0)
                .map(|(level, work)| (level, roof_work_json(*work)))
                .collect();
            (!observed.is_empty()).then_some((name, observed))
        })
        .collect();
    let slowest: Vec<_> = stages.slow_roofs.iter().flatten().map(|roof| {
        serde_json::json!({
            "cell": { "face": roof.cell_face, "level": roof.cell_level, "i": roof.cell_i, "j": roof.cell_j },
            "source_ordinal": roof.source_ordinal,
            "seed": roof.seed,
            "parent_body": roof.parent_body,
            "simple": roof.simple,
            "shape": vearth_gen::contracts::ROOF_DIAGNOSTIC_SHAPES.get(usize::from(roof.shape)).copied().unwrap_or("unknown"),
            "reentrant": roof.reentrant,
            "tiered": roof.tiered,
            "measured": roof.measured,
            "rings": roof.rings,
            "edges": roof.edges,
            "extent_m": roof.extent_m,
            "rise_m": roof.rise_m,
            "pitch": roof.pitch,
            "work": roof_work_json(roof.work),
        })
    }).collect();
    serde_json::json!({
        "by_shape_and_level": by_shape,
        "elapsed_bucket_upper_ns": vearth_gen::contracts::ROOF_ELAPSED_BUCKET_UPPER_NS,
        "slowest": slowest,
        "building_retries": {
            "count_by_level": stages.building_retry_count_by_level,
            "ns_by_level": stages.building_retry_ns_by_level,
            "discarded_bytes_by_level": stages.building_discarded_bytes_by_level,
        },
    })
}

fn measurement_generation_json(
    stages: &vearth_gen::contracts::GenerationStageSnapshot,
) -> serde_json::Value {
    let phases: std::collections::BTreeMap<_, _> = vearth_gen::contracts::GENERATION_PHASES
        .into_iter()
        .enumerate()
        .map(|(phase, name)| {
            (
                name,
                serde_json::json!({
                    "count_by_level": stages.phase_counts_by_level[phase],
                    "ns_by_level": stages.phase_ns_by_level[phase],
                }),
            )
        })
        .collect();
    serde_json::json!({
        "samples": stages.samples,
        "generation_ns": stages.generation_ns,
        "collision_build_ns": stages.collision_build_ns,
        "phases": phases,
        "roofs": roof_diagnostics_json(stages),
    })
}

impl Default for StartPos {
    fn default() -> Self {
        StartPos {
            lat_deg: 46.0,
            lon_deg: 8.0,
            alt_m: 16_000_000.0,
            heading_deg: 0.0,
            pitch_deg: 0.0,
            play: false,
            solar_start: SolarStart::Live,
            time_scale: None,
            weather: WeatherConfig::default(),
        }
    }
}

/// Exact generation targets and completion admission for one frame. Coverage
/// roots fill uncovered regions; requested leaves remain independently runnable.
/// Presented backdrops are retained without creating ancestor generation work.
#[derive(Default)]
struct IntentSnapshot {
    priorities: HashMap<CellKey, UploadPriority>,
    generation: HashMap<CellKey, GenerationIntent>,
    prefetch_leases: HashMap<CellKey, LeaseKind>,
    inputs: IntentInputs,
    presented_scratch: Vec<CellId>,
    #[cfg(test)]
    rebuilds: usize,
}

#[derive(Default)]
struct IntentInputs {
    store_identity: Option<Arc<()>>,
    membership_revision: u64,
    wanted: Vec<(CellId, u64, ViewLane)>,
    destination: Vec<(CellId, u64)>,
    predicted: Vec<(CellId, u64)>,
    presented: Vec<CellId>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GenerationIntent {
    priority: f64,
    view_weighted: bool,
}

// Wanted lanes are separated by 100 decimal exponents. Coverage precedes
// final detail inside its lane without crossing into a stronger view lane.
const BOOTSTRAP_PRIORITY_MULTIPLIER: f64 = 1e20;

impl IntentSnapshot {
    #[cfg(test)]
    fn build(
        wanted: &[WantedCell],
        index: &ResidentStore,
        presented: impl IntoIterator<Item = CellId>,
        destination: &[(CellId, f64)],
        predicted: &[(CellId, f64)],
    ) -> Self {
        let mut intents = Self::default();
        intents.rebuild(wanted, index, presented, destination, predicted);
        intents
    }

    fn rebuild(
        &mut self,
        wanted: &[WantedCell],
        index: &ResidentStore,
        presented_cells: impl IntoIterator<Item = CellId>,
        destination: &[(CellId, f64)],
        predicted: &[(CellId, f64)],
    ) {
        let mut presented = std::mem::take(&mut self.presented_scratch);
        presented.clear();
        presented.extend(presented_cells);
        let (identity, revision) = index.membership_stamp();
        let wanted_key =
            |wanted: &WantedCell| (wanted.cell, wanted.screen_px.to_bits(), wanted.lane);
        let prefetch_key = |&(cell, score): &(CellId, f64)| (cell, score.to_bits());
        let previous = &self.inputs;
        // Bootstrap coverage depends only on resident membership. Fades do
        // not enter admission, and generation's camera weights stay live in
        // generation_jobs_into even when the selected targets are unchanged.
        if previous.membership_revision == revision
            && previous
                .store_identity
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old, identity))
            && previous
                .wanted
                .iter()
                .copied()
                .eq(wanted.iter().map(wanted_key))
            && previous
                .destination
                .iter()
                .copied()
                .eq(destination.iter().map(prefetch_key))
            && previous
                .predicted
                .iter()
                .copied()
                .eq(predicted.iter().map(prefetch_key))
            && previous.presented == presented
        {
            self.presented_scratch = presented;
            return;
        }
        #[cfg(test)]
        {
            self.rebuilds += 1;
        }
        self.priorities.clear();
        self.generation.clear();
        self.prefetch_leases.clear();
        for wanted in wanted {
            let (quality_class, coverage_class) = match wanted.lane {
                ViewLane::Visible | ViewLane::Legacy => {
                    (IntentClass::VisibleQuality, IntentClass::VisibleCoverage)
                }
                ViewLane::Guard => (IntentClass::Guard, IntentClass::Guard),
                ViewLane::Background => (IntentClass::Background, IntentClass::Background),
            };
            self.insert_target(
                wanted.cell,
                UploadPriority::new(quality_class, wanted.screen_px),
                wanted.stream_priority(),
                true,
                None,
            );
            self.insert_bootstrap(
                wanted.cell,
                index,
                UploadPriority::new(coverage_class, wanted.screen_px),
                wanted.stream_priority(),
                true,
                None,
            );
        }
        for &(cell, score) in destination {
            self.insert_target(
                cell,
                UploadPriority::new(IntentClass::DestinationDetail, score),
                score,
                false,
                Some(LeaseKind::Destination),
            );
            self.insert_bootstrap(
                cell,
                index,
                UploadPriority::new(IntentClass::DestinationCoverage, score),
                score,
                false,
                Some(LeaseKind::Destination),
            );
        }
        for &(cell, score) in predicted {
            self.insert_target(
                cell,
                UploadPriority::new(IntentClass::Predicted, score),
                score,
                false,
                Some(LeaseKind::Predicted),
            );
            self.insert_bootstrap(
                cell,
                index,
                UploadPriority::new(IntentClass::Guard, score),
                score,
                false,
                Some(LeaseKind::Predicted),
            );
        }
        for &cell in &presented {
            self.insert(
                cell,
                UploadPriority::new(IntentClass::VisibleCoverage, f64::MAX),
            );
        }
        self.inputs.store_identity = Some(Arc::clone(identity));
        self.inputs.membership_revision = revision;
        self.inputs.wanted.clear();
        self.inputs.wanted.extend(wanted.iter().map(wanted_key));
        self.inputs.destination.clear();
        self.inputs
            .destination
            .extend(destination.iter().map(prefetch_key));
        self.inputs.predicted.clear();
        self.inputs
            .predicted
            .extend(predicted.iter().map(prefetch_key));
        std::mem::swap(&mut self.inputs.presented, &mut presented);
        self.presented_scratch = presented;
    }

    fn insert_bootstrap(
        &mut self,
        target: CellId,
        index: &ResidentStore,
        priority: UploadPriority,
        generation_priority: f64,
        view_weighted: bool,
        lease: Option<LeaseKind>,
    ) {
        if let Some(root) = crate::streaming::missing_coverage_root(target, index) {
            self.insert_target(
                root,
                UploadPriority::new(
                    priority.class,
                    priority.score * BOOTSTRAP_PRIORITY_MULTIPLIER,
                ),
                generation_priority * BOOTSTRAP_PRIORITY_MULTIPLIER,
                view_weighted,
                lease,
            );
        }
    }

    fn insert_target(
        &mut self,
        cell: CellId,
        priority: UploadPriority,
        generation_priority: f64,
        view_weighted: bool,
        lease: Option<LeaseKind>,
    ) {
        self.insert(cell, priority);
        let intent = GenerationIntent {
            priority: generation_priority,
            view_weighted,
        };
        self.generation
            .entry(CellKey::from(cell))
            .and_modify(|existing| {
                if intent.priority > existing.priority {
                    *existing = intent;
                }
            })
            .or_insert(intent);
        if let Some(kind) = lease {
            self.prefetch_leases
                .entry(CellKey::from(cell))
                .and_modify(|existing| {
                    if kind == LeaseKind::Destination {
                        *existing = kind;
                    }
                })
                .or_insert(kind);
        }
    }

    fn insert(&mut self, cell: CellId, priority: UploadPriority) {
        self.priorities
            .entry(CellKey::from(cell))
            .and_modify(|existing| {
                if priority.class > existing.class
                    || (priority.class == existing.class && priority.score > existing.score)
                {
                    *existing = priority;
                }
            })
            .or_insert(priority);
    }

    fn generation_jobs_into(
        &self,
        index: &ResidentStore,
        staged: impl Fn(CellId) -> bool,
        camera: DVec3,
        view_direction: DVec3,
        jobs: &mut Vec<(CellId, f64)>,
    ) {
        jobs.clear();
        for (&key, intent) in &self.generation {
            let cell = CellId::from(key);
            if index.contains(cell) || staged(cell) {
                continue;
            }
            let weight = if intent.view_weighted {
                let anchor = vearth_core::ellipsoid::surface_point_along(cell.center_dir());
                let to = (anchor - camera).normalize_or(view_direction);
                0.25 + 0.75 * ((to.dot(view_direction) + 1.0) * 0.5)
            } else {
                1.0
            };
            jobs.push((cell, intent.priority * weight));
        }
    }

    fn lease_prefetch(&self, leases: &mut LeaseBook, frame: u64) {
        for (&key, &kind) in &self.prefetch_leases {
            let duration = if kind == LeaseKind::Destination {
                DESTINATION_LEASE_FRAMES
            } else {
                120
            };
            leases.protect(CellId::from(key), kind, frame.saturating_add(duration));
        }
    }

    fn priority(&self, cell: CellId) -> Option<UploadPriority> {
        self.priorities.get(&CellKey::from(cell)).copied()
    }

    fn contains(&self, cell: CellId) -> bool {
        self.priorities.contains_key(&CellKey::from(cell))
    }
}

/// Which controller owns the camera: explore navigation or vehicle follow.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Explore,
    Play,
}

#[derive(Default)]
struct Mouse {
    left: bool,
    pos: Option<(f64, f64)>,
    /// ECEF surface point grabbed at the start of a Cmd/Ctrl tilt-drag; the
    /// orbit keeps it pinned to its pixel for the whole gesture.
    tilt_anchor: Option<DVec3>,
}

struct CursorIdleState {
    last_moved_at: Instant,
    inside_window: bool,
    window_focused: bool,
}

impl CursorIdleState {
    fn new(now: Instant) -> Self {
        Self {
            last_moved_at: now,
            // Winit sends leave/focus events when these assumptions change.
            inside_window: true,
            window_focused: true,
        }
    }

    fn moved(&mut self, now: Instant) {
        self.last_moved_at = now;
        self.inside_window = true;
    }

    fn entered(&mut self, now: Instant) {
        self.moved(now);
    }

    fn left(&mut self) {
        self.inside_window = false;
    }

    fn focused(&mut self, focused: bool, now: Instant) {
        self.window_focused = focused;
        if focused {
            self.last_moved_at = now;
        }
    }

    fn cursor_visible(&self, now: Instant) -> bool {
        !self.inside_window
            || !self.window_focused
            || now.duration_since(self.last_moved_at) < CURSOR_IDLE_TIMEOUT
    }
}

/// Held-key state for continuous Google-Earth-style keyboard navigation.
#[derive(Default)]
struct Keys {
    north: bool,     // W / ArrowUp
    south: bool,     // S / ArrowDown
    east: bool,      // D / ArrowRight
    west: bool,      // A / ArrowLeft
    zoom_in: bool,   // + / =
    zoom_out: bool,  // -
    rot_left: bool,  // Q
    rot_right: bool, // E
    tilt_up: bool,   // R (toward horizon)
    tilt_down: bool, // F (toward nadir)
}

/// User-selectable multiplier for explore-mode WASD / arrow-key panning.
/// The preset is independent of the native menu implementation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PanSpeedPreset {
    Slow,
    #[default]
    Standard,
    Fast,
    VeryFast,
}

// The browser has no native menu listing the presets.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
impl PanSpeedPreset {
    pub(crate) const ALL: [Self; 4] = [Self::Slow, Self::Standard, Self::Fast, Self::VeryFast];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Slow => "Slow (1x)",
            Self::Standard => "Standard (Default, 1.5x)",
            Self::Fast => "Fast (2x)",
            Self::VeryFast => "Very Fast (2.5x)",
        }
    }

    const fn multiplier(self) -> f64 {
        match self {
            Self::Slow => 1.0,
            Self::Standard => 1.5,
            Self::Fast => 2.0,
            Self::VeryFast => 2.5,
        }
    }
}

impl From<PanSpeedPreference> for PanSpeedPreset {
    fn from(value: PanSpeedPreference) -> Self {
        match value {
            PanSpeedPreference::Slow => Self::Slow,
            PanSpeedPreference::Standard => Self::Standard,
            PanSpeedPreference::Fast => Self::Fast,
            PanSpeedPreference::VeryFast => Self::VeryFast,
        }
    }
}

impl From<PanSpeedPreset> for PanSpeedPreference {
    fn from(value: PanSpeedPreset) -> Self {
        match value {
            PanSpeedPreset::Slow => Self::Slow,
            PanSpeedPreset::Standard => Self::Standard,
            PanSpeedPreset::Fast => Self::Fast,
            PanSpeedPreset::VeryFast => Self::VeryFast,
        }
    }
}

/// Ground speed for explore-mode keyboard panning. When resident terrain is
/// available, clearance rather than ellipsoid altitude controls the close-
/// range response (otherwise elevated cities pan far too quickly). The 30 m
/// floor preserves the sea-level street response while removing the elevation
/// component. `Slow` retains that baseline;
/// the default and faster presets scale it upward.
fn keyboard_pan_speed_m_s(alt_m: f64, terrain_m: Option<f64>, preset: PanSpeedPreset) -> f64 {
    const PAN_RATE: f64 = 0.9;
    const STREET_CLEARANCE_M: f64 = 30.0;
    const ABSOLUTE_ALT_BLEND_START_M: f64 = 250.0;
    const TERRAIN_BLEND_END_M: f64 = 1_000.0;

    fn smoothstep(t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    let terrain_m = terrain_m.filter(|h| h.is_finite()).unwrap_or(0.0).max(0.0);
    let clearance_m = (alt_m - terrain_m).max(STREET_CLEARANCE_M);

    // Rejoin ellipsoid-altitude scaling smoothly once terrain height is
    // negligible compared with the camera's clearance.
    let terrain_t = smoothstep(
        (clearance_m - ABSOLUTE_ALT_BLEND_START_M)
            / (TERRAIN_BLEND_END_M - ABSOLUTE_ALT_BLEND_START_M),
    );
    let pan_alt_m = clearance_m + (alt_m.max(clearance_m) - clearance_m) * terrain_t;
    pan_alt_m * PAN_RATE * preset.multiplier()
}

fn initial_weather_restore_mode(mode: WeatherMode) -> WeatherMode {
    match mode {
        WeatherMode::Clear => WeatherMode::Procedural,
        active => active,
    }
}

fn set_weather_enabled(config: &mut WeatherConfig, restore_mode: &mut WeatherMode, enabled: bool) {
    if enabled {
        if config.mode == WeatherMode::Clear {
            config.mode = initial_weather_restore_mode(*restore_mode);
        }
    } else if config.mode != WeatherMode::Clear {
        *restore_mode = config.mode;
        config.mode = WeatherMode::Clear;
    }
}

fn vehicle_from_preference(value: VehiclePreference) -> VehicleKind {
    match value {
        VehiclePreference::Kestrel => VehicleKind::Kestrel,
        VehiclePreference::PhaseManta => VehicleKind::PhaseManta,
        VehiclePreference::Sparrow => VehicleKind::Sparrow,
    }
}

fn vehicle_preference(value: VehicleKind) -> VehiclePreference {
    match value {
        VehicleKind::PhaseManta | VehicleKind::SplitArrow => VehiclePreference::PhaseManta,
        VehicleKind::Kestrel => VehiclePreference::Kestrel,
        VehicleKind::Sparrow => VehiclePreference::Sparrow,
    }
}

fn weather_quality_from_preference(value: WeatherQualityPreference) -> WeatherQuality {
    match value {
        WeatherQualityPreference::Low => WeatherQuality::Low,
        WeatherQualityPreference::Standard => WeatherQuality::Standard,
        WeatherQualityPreference::High => WeatherQuality::High,
    }
}

fn weather_quality_preference(value: WeatherQuality) -> WeatherQualityPreference {
    match value {
        WeatherQuality::Low => WeatherQualityPreference::Low,
        WeatherQuality::Standard => WeatherQualityPreference::Standard,
        WeatherQuality::High => WeatherQualityPreference::High,
    }
}

fn wind_from_preference(value: WindPreference) -> WindInfluence {
    match value {
        WindPreference::Off => WindInfluence::Off,
        WindPreference::Assisted => WindInfluence::Assisted,
        WindPreference::Full => WindInfluence::Full,
    }
}

fn wind_preference(value: WindInfluence) -> WindPreference {
    match value {
        WindInfluence::Off => WindPreference::Off,
        WindInfluence::Assisted => WindPreference::Assisted,
        WindInfluence::Full => WindPreference::Full,
    }
}

struct State {
    window: Arc<Window>,
    gfx: Gfx,
    camera: Camera,
    pool: Pool,
    index: ResidentStore,
    lod_selector: LodSelector,
    wanted: Vec<WantedCell>,
    /// None → recompute immediately (also the first-frame state; web Instant
    /// starts near zero, so "now minus interval" would underflow there).
    wanted_at: Option<Instant>,
    frame_no: u64,
    mouse: Mouse,
    cursor_idle: CursorIdleState,
    mods: ModifiersState,
    keys: Keys,
    pan_speed: PanSpeedPreset,
    last_frame: Option<Instant>,
    benchmark_frame_deadline: Option<Instant>,
    surface_occluded: bool,
    /// When the page last became visible; `None` while it is hidden.
    visible_since: Option<Instant>,
    fps_window: (Instant, u32),
    fps: f64,
    /// North-up lock (Maps behavior): heading eases back to north at every
    /// altitude — released by a deliberate Cmd-drag rotation, re-engaged
    /// once the globe view has re-northed the camera.
    north_locked: bool,
    /// Results that arrived for already-resident cells (should stay ~0; a
    /// growing count means the pool is redoing finished work).
    dup_results: u64,
    /// (generated, cache_hits, http_reqs, http_bytes, dups) at last log.
    last_stats: (u64, u64, u64, u64, u64),
    /// Share of cells generated cold (no chunk-cache hit) in the last
    /// stats window; None when the window generated nothing.
    hud_cold_pct: Option<u32>,
    /// Split memory for the SSE merge hysteresis (fed back to
    /// `compute_wanted` on every refresh).
    split_set: HashSet<CellId>,
    /// Coverage-safe, non-interruptible LOD transition groups.
    transitions: LodTransitions,
    transition_coverage: CoverageIndex,
    /// Optional frame-indexed benchmark trace. Absolute poses are applied
    /// after interactive updates, so timing variance cannot change the path.
    replay: Option<TracePlayer>,
    search_benchmark: Option<SearchBenchmarkPlayer>,
    drone_benchmark: Option<crate::benchmark_drone::DroneBenchmark>,
    replay_finished_logged: bool,
    replay_benchmark: Option<ReplayBenchmarkState>,
    replay_exit_requested: bool,
    /// Generated meshes awaiting time/byte-budgeted GPU admission.
    ready_meshes: UploadStager,
    drawable_cut_arena: DrawableCutArena,
    drawable_cut_cells: Vec<CellId>,
    drawable_cut_missing: Vec<(CellId, f64)>,
    streaming_intents: IntentSnapshot,
    leases: LeaseBook,
    legacy_residency: bool,
    validate_transitions: bool,
    telemetry: SmoothnessTelemetry,
    prediction_anchor: Option<DVec3>,
    predicted_prefetch: Vec<(CellId, f64)>,
    mode: Mode,
    player: Option<Player>,
    location_tracker: LocationTracker,
    local_time: LocalTimeTracker,
    vehicle_parts: crate::vehicles::VehicleRenderParts,
    deferred_vehicles: crate::vehicles::DeferredVehicles,
    selected_vehicle: VehicleKind,
    play_keys: PlayKeys,
    /// Astronomical time. Lighting is render-only, while its calendar month
    /// selects the matching bundled surface-colour atlas.
    solar: SolarClock,
    /// Stateless climate/weather sampler. It never participates in chunk
    /// generation or the terrain provider queues.
    weather: WeatherModel,
    /// Last active weather mode. Turning the system off is reversible and
    /// never discards the selected live/procedural or authored preset mode.
    weather_restore_mode: WeatherMode,
    /// Model whose generated vertices carry the calibration tag. Ordinary
    /// transform edits stay render-only; changing this selection is the one
    /// operation that requires a fresh world preparation.
    calibration_landmark: Option<&'static vearth_gen::landmarks::Landmark>,
    /// Re-enter play mode when the in-flight teleport lands (the teleport
    /// was issued from play mode).
    resume_play: bool,
    ui: Ui,
    flyto: Option<FlyTo>,
    /// Destination-view cells streamed during a fly-to, refreshed with the
    /// wanted set (cell, priority).
    dest_prefetch: Vec<(CellId, f64)>,
    /// Cold-teleport stopwatch: (teleport started, arrived at destination).
    dive: Option<(Instant, Option<Instant>)>,
    /// A photo locks the authored scene while its output-resolution LOD set
    /// settles. The UI remains responsive, but navigation and simulation do
    /// not advance until the clean offscreen render has been written.
    photo_job: Option<PhotoJob>,
    #[cfg(target_arch = "wasm32")]
    pending_capture: Option<web::PendingWebCapture>,
    photo_status: PhotoStatus,
    preferences: UserPreferences,
    preferences_store: Option<PreferencesStore>,
    default_provider_controls: Vec<vearth_data::registry::ProviderControl>,
    runtime_paths: RuntimePaths,
    cache_usage: CacheUsageTracker,
    active_cache_limit_gb: u64,
    pending_cache_clear: Option<PendingCacheClear>,
}

struct PreparedWorld {
    pool: Pool,
    attributions: Vec<vearth_data::registry::AttributionEntry>,
    locations: Option<Arc<vearth_data::vectors::ProtomapsSource>>,
    default_provider_controls: Vec<vearth_data::registry::ProviderControl>,
}

impl Drop for State {
    fn drop(&mut self) {
        // GPU teardown can be slow; invalidate work before any fields drop.
        self.pool.shutdown();
    }
}

fn prepare_world(
    calibration_mode: bool,
    surface_month: u8,
    cache_root: &std::path::Path,
    preferences: &UserPreferences,
) -> PreparedWorld {
    let budgets = preferences.cache_budgets();
    let built =
        crate::sources::build_registry_with_budget(cache_root, surface_month, budgets.source_bytes);
    let default_provider_controls = built.registry.provider_controls();
    for control in &default_provider_controls {
        if let Some(enabled) = preferences.provider_enabled(&control.id) {
            built.registry.set_provider_enabled(&control.id, enabled);
        }
    }
    for line in built.registry.attributions() {
        log::info!("provider: {line}");
    }
    let mut attributions = built.registry.attribution_entries();
    attributions.extend(crate::sky::attribution_entries());
    attributions.push(vearth_data::registry::AttributionEntry {
        layer: "Landmarks",
        name: "Great Wall geographic network".into(),
        attribution: "© OpenStreetMap contributors; pinned nationwide Great Wall relation 318110"
            .into(),
        license: "ODbL 1.0".into(),
        url: "https://www.openstreetmap.org/relation/318110".into(),
    });
    attributions.push(vearth_data::registry::AttributionEntry {
        layer: "Location regions",
        name: "Bundled Natural Earth Admin-1 atlas".into(),
        attribution: vearth_data::admin_regions::ADMIN_REGIONS_PROVENANCE.into(),
        license: "Public domain".into(),
        url: "https://www.naturalearthdata.com/".into(),
    });
    attributions.push(vearth_data::registry::AttributionEntry {
        layer: "Settlement context",
        name: "Bundled GHS-SMOD 2020 settlement atlas".into(),
        attribution: vearth_data::settlement::SETTLEMENT_ATLAS_PROVENANCE.into(),
        license: "CC BY 4.0".into(),
        url: "https://doi.org/10.2905/A0DF7A6F-49DE-46EA-9BDE-563437A6E2BA".into(),
    });
    attributions.push(vearth_data::registry::AttributionEntry {
        layer: "Local time",
        name: "Bundled full-precision world timezone atlas".into(),
        attribution: vearth_data::timezones::TIMEZONE_ATLAS_PROVENANCE.into(),
        license: "ODbL 1.0".into(),
        url: "https://github.com/evansiroky/timezone-boundary-builder".into(),
    });
    attributions.push(vearth_data::registry::AttributionEntry {
        layer: "Map overlay",
        name: "Natural Earth 1:50m world map".into(),
        attribution: "Natural Earth".into(),
        license: "Public domain".into(),
        url: "https://www.naturalearthdata.com/".into(),
    });
    attributions.push(vearth_data::registry::AttributionEntry {
        layer: "Climate",
        name: "Bundled Climate Atlas v2".into(),
        attribution: vearth_data::climate::CLIMATE_ATLAS_PROVENANCE.into(),
        license: "WorldClim: CC BY 4.0; NOAA/NASA: public domain".into(),
        url: "https://www.worldclim.org/data/worldclim21.html".into(),
    });
    // Persistent generated-chunk cache: revisits skip generation entirely.
    // Calibration transforms are intentionally process-local and mutable;
    // never read or write canonical generated chunks while one is active.
    let cache = (!calibration_mode).then(|| {
        Arc::new(crate::chunk_cache::ChunkCache::with_configured_disk_budget(
            cache_root,
            &built.dataset_tag,
            budgets.generated_bytes,
        ))
    });
    // Provider-backed blocking workers can park on cache misses. In-flight
    // deduplication prevents duplicate fetches; oversubscription keeps other
    // cells progressing while those workers wait.
    let workers = vearth_platform::thread::available_parallelism()
        .map(|n| n.get() * 2 + 8)
        .unwrap_or(16)
        .clamp(16, 32);
    log::info!("gen pool: {workers} native workers");
    let locations = built.locations.clone();
    PreparedWorld {
        pool: crate::streaming::GenPool::with_async_acquisition_slots(
            built.registry,
            cache,
            workers,
            crate::streaming::ASYNC_ACQUISITION_CELLS.max(workers),
        ),
        attributions,
        locations,
        default_provider_controls,
    }
}

fn frame_calibration_camera(
    camera: &mut Camera,
    value: vearth_gen::landmarks::CalibrationOverride,
) {
    let landmark = vearth_gen::landmarks::LANDMARKS
        .iter()
        .find(|landmark| landmark.name == value.name)
        .expect("calibration landmark exists");
    let resolved = vearth_gen::landmarks::resolve_calibration(value);
    let width = landmark.bbox_max[0] - landmark.bbox_min[0];
    let height = landmark.bbox_max[1] - landmark.bbox_min[1];
    let depth = landmark.bbox_max[2] - landmark.bbox_min[2];
    let fixed_base = match landmark.base {
        vearth_gen::landmarks::Base::Fixed(altitude_m) => f64::from(altitude_m),
        vearth_gen::landmarks::Base::Terrain | vearth_gen::landmarks::Base::TerrainOffset(_) => 0.0,
    };
    camera.lat = resolved.lat_deg.to_radians();
    camera.lon = resolved.lon_deg.to_radians();
    camera.alt_m =
        (width.max(height).max(depth) * 1.45 + fixed_base + resolved.base_offset_m.max(0.0))
            .clamp(180.0, 12_000.0);
    // A top-down north-up reset makes footprint and heading comparisons
    // unambiguous. Cmd/Ctrl-drag remains available for base-height checks.
    camera.heading = 0.0;
    camera.pitch = 0.0;
}

impl State {
    #[allow(clippy::too_many_arguments)]
    fn new(
        window: Arc<Window>,
        mut gfx: Gfx,
        start: StartPos,
        replay: Option<ReplayTrace>,
        search_benchmark: Option<SearchBenchmarkSpec>,
        replay_benchmark: Option<ReplayBenchmarkOptions>,
        calibration_landmark: Option<&'static vearth_gen::landmarks::Landmark>,
        prepared_world: PreparedWorld,
        runtime_paths: RuntimePaths,
        preferences_store: Option<PreferencesStore>,
        preferences: UserPreferences,
    ) -> Self {
        let benchmark_started_at = Instant::now();
        gfx.renderer.set_gpu_timing_capture(
            replay_benchmark
                .as_ref()
                .is_some_and(|options| options.measurement_range.is_some()),
        );
        let mut camera = Camera::start();
        camera.lat = start.lat_deg.to_radians();
        camera.lon = start.lon_deg.to_radians();
        camera.alt_m = start.alt_m;
        camera.heading = start.heading_deg.to_radians();
        camera.pitch = start.pitch_deg.to_radians();
        if search_benchmark.is_some() {
            // Match the globe origin used by the search trace, at the actual
            // viewport-dependent zoom stop rather than an overridden pose.
            camera.lat = 0.0;
            camera.lon = 0.0;
            camera.heading = 0.0;
            camera.pitch = 0.0;
            camera.aspect = gfx.config.width as f64 / gfx.config.height.max(1) as f64;
            camera.alt_m = camera.max_zoom_out_alt();
        }
        if let Some(landmark) = calibration_landmark {
            let value = vearth_gen::landmarks::CalibrationOverride {
                name: landmark.name,
                east_m: 0.0,
                north_m: 0.0,
                up_m: 0.0,
                heading_deg: landmark.heading_deg,
            };
            vearth_gen::landmarks::set_calibration_override(Some(value));
            gfx.renderer.set_landmark_calibration(Some(value));
            frame_calibration_camera(&mut camera, value);
        } else {
            vearth_gen::landmarks::set_calibration_override(None);
            gfx.renderer.set_landmark_calibration(None);
        }
        let solar = SolarClock::new(start.solar_start, start.time_scale);
        let PreparedWorld {
            pool,
            attributions,
            locations,
            default_provider_controls,
        } = prepared_world;
        let _ = &attributions;
        let vehicle_parts = crate::vehicles::AUTHORED_PARTS;
        let ui = Ui::new(
            &window,
            &gfx.renderer.device,
            gfx.view_format(),
            attributions,
            calibration_landmark,
        );
        let weather_restore_mode = initial_weather_restore_mode(start.weather.mode);
        let mut state = State {
            window,
            gfx,
            camera,
            pool,
            index: ResidentStore::new(),
            lod_selector: LodSelector::new(),
            wanted: Vec::new(),
            wanted_at: None,
            frame_no: 0,
            mouse: Mouse::default(),
            cursor_idle: CursorIdleState::new(Instant::now()),
            mods: ModifiersState::default(),
            keys: Keys::default(),
            pan_speed: PanSpeedPreset::default(),
            last_frame: None,
            benchmark_frame_deadline: None,
            surface_occluded: false,
            visible_since: Some(Instant::now()),
            fps_window: (Instant::now(), 0),
            fps: 0.0,
            // A deep link that specifies a heading is a chosen orientation;
            // don't immediately unwind it back to north.
            north_locked: calibration_landmark.is_some() || start.heading_deg.abs() < 0.5,
            dup_results: 0,
            last_stats: (0, 0, 0, 0, 0),
            hud_cold_pct: None,
            split_set: HashSet::new(),
            transitions: LodTransitions::new(FADE_S as f32),
            transition_coverage: CoverageIndex::default(),
            replay: replay.map(TracePlayer::new),
            drone_benchmark: search_benchmark
                .as_ref()
                .filter(|spec| spec.drone_flight)
                .map(|spec| {
                    let options = replay_benchmark.as_ref().expect("drone benchmark report");
                    crate::benchmark_drone::DroneBenchmark::new(
                        options.idle_frames,
                        options.max_settle.as_secs_f64(),
                    )
                    .with_city_tour(spec.city_tour)
                }),
            search_benchmark: search_benchmark.map(SearchBenchmarkPlayer::new),
            replay_finished_logged: false,
            replay_benchmark: replay_benchmark
                .map(|options| ReplayBenchmarkState::new(options, benchmark_started_at)),
            replay_exit_requested: false,
            ready_meshes: UploadStager::new(),
            drawable_cut_arena: DrawableCutArena::new(),
            drawable_cut_cells: Vec::new(),
            drawable_cut_missing: Vec::new(),
            streaming_intents: IntentSnapshot::default(),
            leases: LeaseBook::default(),
            legacy_residency: legacy_residency_enabled(),
            validate_transitions: transition_validation_enabled(),
            telemetry: SmoothnessTelemetry::default(),
            prediction_anchor: None,
            predicted_prefetch: Vec::new(),
            mode: Mode::Explore,
            player: None,
            location_tracker: LocationTracker::new(locations),
            local_time: LocalTimeTracker::new(benchmark_started_at),
            vehicle_parts,
            deferred_vehicles: crate::vehicles::DeferredVehicles::Waiting,
            selected_vehicle: VehicleKind::Kestrel,
            play_keys: PlayKeys::default(),
            solar,
            weather: WeatherModel::new(start.weather),
            weather_restore_mode,
            calibration_landmark,
            resume_play: false,
            ui,
            flyto: None,
            dest_prefetch: Vec::new(),
            dive: None,
            photo_job: None,
            #[cfg(target_arch = "wasm32")]
            pending_capture: None,
            photo_status: PhotoStatus::Idle,
            cache_usage: CacheUsageTracker::new(runtime_paths.cache.clone()),
            pending_cache_clear: crate::settings::pending_cache_clear(
                &runtime_paths.application_support,
            ),
            active_cache_limit_gb: preferences.cache_limit_gb,
            preferences: UserPreferences::default(),
            preferences_store,
            default_provider_controls,
            runtime_paths,
        };
        if state.preferences_store.is_some() {
            state.apply_preferences(&preferences);
        } else {
            state.ui.set_debug_visible(preferences.show_debug);
            state.preferences = preferences;
        }
        if start.play {
            state.enter_play();
        }
        state
    }

    /// Any manual navigation cancels an in-progress teleport.
    fn cancel_flyto(&mut self) {
        let was_flying = self.flyto.take().is_some();
        self.dest_prefetch.clear();
        self.dive = None; // an aborted dive isn't a measurement
        self.resume_play = false;
        if was_flying {
            self.lod_selector.invalidate();
            self.wanted_at = None;
        }
    }

    /// Remove every generated artifact that may contain data from the old
    /// provider configuration. Source-tile caches remain warm; only composed
    /// world cells are rebuilt.
    fn invalidate_generated_world(&mut self) {
        let resident: Vec<_> = self.index.ids().collect();
        self.gfx.renderer.evict(&resident);
        self.index.clear();
        debug_assert_eq!(self.gfx.renderer.resident_count(), self.index.len());
        self.lod_selector.reset();
        self.ready_meshes.clear();
        self.drawable_cut_cells.clear();
        self.drawable_cut_missing.clear();
        self.drawable_cut_arena.invalidate();
        self.leases = LeaseBook::default();
        self.transitions.reset_opaque(&[]);
        self.transition_coverage.rebuild(std::iter::empty());
        self.split_set.clear();
        self.wanted.clear();
        self.wanted_at = None;
        self.prediction_anchor = None;
        self.predicted_prefetch.clear();
        self.dest_prefetch.clear();
    }

    fn resize(&mut self, width: u32, height: u32) {
        if self.gfx.config.width != width || self.gfx.config.height != height {
            self.lod_selector.invalidate();
            self.wanted_at = None;
        }
        self.gfx.resize(width, height);
    }

    fn selection_request(
        &mut self,
        cam_pos: DVec3,
        viewport_h: f64,
        photo_locked: bool,
    ) -> SelectionRequest {
        // Short-horizon camera prediction is request-only: it never changes
        // the desired or presented cut. Clamp displacement so a teleport
        // does not extrapolate to the far side of the planet.
        let predicted_cam = if photo_locked {
            None
        } else {
            self.prediction_anchor.and_then(|previous| {
                let delta = cam_pos - previous;
                let max_step = (self.camera.alt_m * 0.5).clamp(1_000.0, 250_000.0);
                let predicted = if delta.length() > max_step {
                    cam_pos + delta.normalize_or_zero() * max_step
                } else {
                    cam_pos + delta * 2.0
                };
                ((predicted - cam_pos).length() > 10.0).then_some(predicted)
            })
        };
        self.prediction_anchor = Some(cam_pos);

        // A fly-to destination or the drone's velocity vector supplies the
        // second speculative view. The worker filters it against its
        // incrementally maintained resident mirror.
        let destination_cam = if photo_locked {
            None
        } else if let Some(fly) = &self.flyto {
            Some(fly.end_pos_ecef())
        } else if self.mode == Mode::Play {
            self.player
                .as_ref()
                .filter(|player| player.speed_ms() > 12.0)
                .map(|player| player.pos + player.vel * 2.5)
        } else {
            None
        };

        if destination_cam.is_some()
            && let Some(search) = &mut self.search_benchmark
        {
            search.record_destination_request();
        }
        let mode = if legacy_lod_enabled() {
            SelectionMode::Legacy
        } else {
            // Drone flight retains surrounding detail: the angular guard alone
            // does not bound terrain acquisition latency during a turn.
            SelectionMode::View(
                LodView::new(
                    self.camera.view_rotation(),
                    self.camera.fov_y,
                    self.camera.aspect,
                )
                .with_background_coalescing(self.mode != Mode::Play),
            )
        };
        SelectionRequest {
            cam_ecef: cam_pos,
            viewport_h,
            fov_y: self.camera.fov_y,
            mode,
            prev_split: self.split_set.clone(),
            predicted_cam,
            destination_cam,
        }
    }

    fn adopt_selection(&mut self, selection: Selection) {
        self.index
            .protect_metric_dependencies(&selection.metric_dependencies);
        if let Some(search) = &mut self.search_benchmark {
            search.record_prefetch(selection.destination_prefetch.len());
        }
        self.telemetry.totals.wanted_budget_saturated += selection.stats.budget_saturated as u64;
        self.telemetry.totals.wanted_topology_changes += selection.stats.topology_changes as u64;
        self.wanted = selection.wanted;
        self.split_set = selection.split;
        self.predicted_prefetch = selection.predicted_prefetch;
        self.dest_prefetch = selection.destination_prefetch;
    }

    fn set_provider_enabled(&mut self, id: &vearth_data::registry::ProviderId, enabled: bool) {
        if self.pool.set_provider_enabled(id, enabled) {
            log::info!(
                "provider {} / {}: {}",
                id.layer.label(),
                id.name,
                if enabled { "enabled" } else { "disabled" }
            );
            self.invalidate_generated_world();
        }
    }

    fn preference_snapshot(&self) -> UserPreferences {
        let mut preferences = self.preferences.clone();
        let (search, debug, solar, compass, loading, attribution) = self.ui.panel_visibility();
        preferences.show_search = search;
        preferences.show_settings_button = self.ui.settings_button_visible();
        preferences.show_debug = debug;
        preferences.show_solar = solar;
        preferences.show_compass = compass;
        preferences.show_loading = loading;
        preferences.show_attribution = attribution;
        preferences.solar_speed = SolarSpeedPreference::from_scale(self.solar.time_scale());
        preferences.twenty_four_hour = self.local_time.twenty_four_hour();
        preferences.weather_enabled = self.weather_enabled();
        preferences.weather_quality = weather_quality_preference(self.weather.config.quality);
        preferences.wind_influence = wind_preference(self.weather.config.wind_influence);
        preferences.precipitation_camera_effects = self.weather.config.precipitation_camera_effects;
        preferences.clouds_visible = self.gfx.renderer.clouds_visible();
        preferences.precipitation_visible = self.gfx.renderer.precipitation_visible();
        preferences.building_night_lights = self.gfx.renderer.building_night_lights_enabled();
        preferences.shadows = self.gfx.renderer.shadows();
        let post_process = self.gfx.renderer.post_process_settings();
        preferences.color_grading = post_process.color_grading;
        preferences.filmic_saturation = post_process.filmic_saturation;
        preferences.pan_speed = self.pan_speed.into();
        if !self.selected_vehicle.is_phased() {
            preferences.selected_vehicle = vehicle_preference(self.selected_vehicle);
        }
        preferences.capture_providers(&self.pool.provider_controls());
        preferences
    }

    fn apply_preferences(&mut self, preferences: &UserPreferences) {
        self.ui.set_search_visible(preferences.show_search);
        self.ui
            .set_settings_button_visible(preferences.show_settings_button);
        self.ui.set_debug_visible(preferences.show_debug);
        self.ui.set_solar_visible(preferences.show_solar);
        self.ui.set_compass_visible(preferences.show_compass);
        self.ui.set_loading_visible(preferences.show_loading);
        self.ui
            .set_attribution_visible(preferences.show_attribution);
        let solar_scale = preferences.solar_speed.scale();
        if (self.solar.time_scale() - solar_scale).abs() > f64::EPSILON {
            self.solar.set_time_scale(solar_scale);
        }
        self.local_time
            .set_twenty_four_hour(preferences.twenty_four_hour);
        self.set_weather_enabled(preferences.weather_enabled);
        self.weather.config.quality = weather_quality_from_preference(preferences.weather_quality);
        self.weather.config.wind_influence = wind_from_preference(preferences.wind_influence);
        self.weather.config.precipitation_camera_effects = preferences.precipitation_camera_effects;
        self.gfx
            .renderer
            .set_clouds_visible(preferences.clouds_visible);
        self.gfx
            .renderer
            .set_precipitation_visible(preferences.precipitation_visible);
        self.gfx
            .renderer
            .set_building_night_lights_enabled(preferences.building_night_lights);
        self.gfx.renderer.set_shadows(preferences.shadows);
        #[cfg(target_arch = "wasm32")]
        crate::host::post_to_page(
            "maxPixelRatio",
            preferences
                .render_resolution
                .max_pixel_ratio()
                .unwrap_or(0.0)
                .into(),
        );
        #[cfg(not(target_arch = "wasm32"))]
        if self
            .window
            .set_max_pixel_ratio(preferences.render_resolution.max_pixel_ratio())
        {
            let size = self.window.inner_size();
            self.resize(size.width, size.height);
        }
        self.gfx
            .renderer
            .set_color_grading_enabled(preferences.color_grading);
        self.gfx
            .renderer
            .set_filmic_saturation_enabled(preferences.filmic_saturation);
        self.pan_speed = preferences.pan_speed.into();
        if preferences.selected_vehicle != self.preferences.selected_vehicle {
            let vehicle = vehicle_from_preference(preferences.selected_vehicle);
            self.selected_vehicle = vehicle;
            if let Some(player) = self.player.as_mut()
                && player.vehicle != vehicle
            {
                player.select_vehicle(vehicle);
            }
        }
        for control in self.pool.provider_controls() {
            if let Some(enabled) = preferences.provider_enabled(&control.id) {
                self.set_provider_enabled(&control.id, enabled);
            }
        }
        self.preferences = preferences.clone();
        self.preferences = self.preference_snapshot();
        if let Some(store) = &self.preferences_store
            && let Err(error) = store.save(&self.preferences)
        {
            log::error!("preferences: could not save: {error}");
            self.ui
                .set_settings_notice(format!("Could not save settings: {error}"));
        }
    }

    fn persist_preferences(&mut self) {
        let snapshot = self.preference_snapshot();
        if snapshot == self.preferences {
            return;
        }
        self.preferences = snapshot;
        if let Some(store) = &self.preferences_store
            && let Err(error) = store.save(&self.preferences)
        {
            log::error!("preferences: could not save: {error}");
            self.ui
                .set_settings_notice(format!("Could not save settings: {error}"));
        }
    }

    fn reset_preferences(&mut self) {
        let mut defaults = UserPreferences::default();
        defaults.capture_providers(&self.default_provider_controls);
        self.apply_preferences(&defaults);
        self.ui
            .set_settings_notice("Settings restored to their defaults".into());
    }

    fn schedule_cache_clear(&mut self, requested: PendingCacheClear) {
        match crate::settings::schedule_cache_clear(
            &self.runtime_paths.application_support,
            requested,
        ) {
            Ok(pending) => {
                self.pending_cache_clear = Some(pending);
                self.ui.set_settings_notice(format!(
                    "{} will be cleared the next time Low Poly Earth starts",
                    pending.label()
                ));
            }
            Err(error) => self
                .ui
                .set_settings_notice(format!("Could not schedule data removal: {error}")),
        }
    }

    fn cancel_cache_clear(&mut self) {
        match crate::settings::cancel_cache_clear(&self.runtime_paths.application_support) {
            Ok(()) => {
                self.pending_cache_clear = None;
                self.ui
                    .set_settings_notice("Scheduled data removal cancelled".into());
            }
            Err(error) => self
                .ui
                .set_settings_notice(format!("Could not cancel data removal: {error}")),
        }
    }

    fn weather_enabled(&self) -> bool {
        self.weather.config.mode != WeatherMode::Clear
    }

    fn set_weather_enabled(&mut self, enabled: bool) {
        set_weather_enabled(
            &mut self.weather.config,
            &mut self.weather_restore_mode,
            enabled,
        );
    }

    fn request_photo(&mut self, settings: PhotoSettings) {
        if self.photo_job.is_some() {
            return;
        }
        let (width, height) = settings.dimensions(self.gfx.config.width, self.gfx.config.height);
        let max_dimension = self.gfx.renderer.device.limits().max_texture_dimension_2d;
        if width > max_dimension || height > max_dimension {
            self.photo_status = PhotoStatus::Failed(format!(
                "{width}×{height} exceeds this GPU's {max_dimension} pixel texture limit"
            ));
            return;
        }

        self.cancel_flyto();
        self.keys = Keys::default();
        self.play_keys = PlayKeys::default();
        let mut camera = self.camera.clone();
        camera.aspect = width as f64 / height.max(1) as f64;
        self.photo_job = Some(PhotoJob {
            settings,
            width,
            height,
            camera,
            solar: self.solar,
            weather: self.weather,
        });
        self.photo_status = PhotoStatus::Preparing {
            width,
            height,
            remaining: self.wanted.len(),
        };
        self.wanted_at = None;
        self.split_set.clear();
        self.prediction_anchor = None;
        self.predicted_prefetch.clear();
        self.dest_prefetch.clear();
        self.lod_selector.invalidate();
        self.window.request_redraw();
    }

    /// Debug comparison of mountain-refinement evidence. The cut refreshes
    /// immediately; retained splits merge through the normal hysteresis.
    fn toggle_ruggedness_policy(&mut self) {
        use crate::streaming::{RuggednessPolicy, ruggedness_policy, set_ruggedness_policy};
        let next = match ruggedness_policy() {
            RuggednessPolicy::Broad => RuggednessPolicy::MaxStep,
            RuggednessPolicy::MaxStep => RuggednessPolicy::Broad,
        };
        set_ruggedness_policy(next);
        log::info!("terrain LOD ruggedness: {}", next.label());
        self.wanted_at = None;
        self.lod_selector.invalidate();
        self.window.request_redraw();
    }

    /// Spawn the selected aircraft hovering under the current camera and
    /// hand it the controls. Colliding craft use the streamed terrain floor;
    /// Phase craft deliberately remain outside that collision path.
    fn enter_play(&mut self) {
        let dir = self.camera.eye_ecef().normalize_or(DVec3::X);
        let terrain = self
            .index
            .terrain_top_under(dir)
            .map(|h| h.max(0.0) as f64)
            .unwrap_or(0.0);
        let alt = match self.selected_vehicle {
            VehicleKind::Sparrow => terrain + crate::player::SPARROW_HOVER_CLEARANCE_M,
            _ => self.camera.alt_m.clamp(terrain + 3.0, terrain + 300.0),
        };
        let pos = vearth_core::ellipsoid::geodetic_to_ecef(self.camera.lat, self.camera.lon, alt);
        self.player = Some(Player::spawn_as(
            pos,
            self.camera.heading,
            self.selected_vehicle,
        ));
        self.location_tracker.restart();
        self.play_keys = PlayKeys::default();
        self.keys = Keys::default();
        self.mode = Mode::Play;
        self.lod_selector.invalidate();
        self.wanted_at = None;
    }

    /// Back to the explore camera, wherever the follow cam was.
    fn exit_play(&mut self) {
        self.mode = Mode::Explore;
        self.player = None;
        self.play_keys = PlayKeys::default();
        // The follow cam may sit past the explore pitch limit or below its
        // altitude floor; pull the pose back into explore's envelope.
        self.camera.pitch = self.camera.pitch.clamp(0.0, 1.45);
        self.camera.alt_m = self.camera.alt_m.max(5.0);
        self.north_locked = false;
        self.lod_selector.invalidate();
        self.wanted_at = None;
    }

    /// The surface point under a screen position, terrain-aware: first hit
    /// the ellipsoid at sea level, then re-intersect at the resident
    /// terrain height there — so grabbing a mountainside anchors the ground
    /// you see, not sea level far beneath it.
    fn ground_anchor(&self, ndc: (f64, f64), aspect: f64) -> Option<DVec3> {
        let sea = self.camera.ray_ground_point(ndc.0, ndc.1, aspect, 0.0);
        let h = sea
            .and_then(|p| self.index.terrain_top_under(p.normalize_or(DVec3::X)))
            .map(|h| h.max(0.0) as f64)
            .unwrap_or(0.0);
        if h > 1.0 {
            self.camera
                .ray_ground_point(ndc.0, ndc.1, aspect, h)
                .or(sea)
        } else {
            sea
        }
    }

    /// Continuous keyboard navigation, frame-rate independent.
    fn apply_keys(&mut self, dt: f64) {
        let k = &self.keys;
        let (mut north, mut east) = (0.0_f64, 0.0_f64);
        if k.north {
            north += 1.0;
        }
        if k.south {
            north -= 1.0;
        }
        if k.east {
            east += 1.0;
        }
        if k.west {
            east -= 1.0;
        }
        if north != 0.0 || east != 0.0 {
            let terrain_m = self
                .index
                .terrain_top_under(self.camera.eye_ecef().normalize_or(DVec3::X))
                .map(f64::from);
            let pan = keyboard_pan_speed_m_s(self.camera.alt_m, terrain_m, self.pan_speed) * dt;
            self.camera.pan_ground(north * pan, east * pan);
        }
        if k.zoom_in {
            self.camera.zoom(4.0 * dt);
        }
        if k.zoom_out {
            self.camera.zoom(-4.0 * dt);
        }
        if k.rot_left {
            self.camera.heading -= 1.0 * dt;
        }
        if k.rot_right {
            self.camera.heading += 1.0 * dt;
        }
        if k.tilt_up {
            self.camera.pitch = (self.camera.pitch + 0.9 * dt).clamp(0.0, 1.45);
        }
        if k.tilt_down {
            self.camera.pitch = (self.camera.pitch - 0.9 * dt).clamp(0.0, 1.45);
        }
    }

    fn map_pose(&self) -> (f64, f64, f64) {
        self.player
            .as_ref()
            .filter(|_| self.mode == Mode::Play)
            .map(|player| {
                let (lat, lon, _) = vearth_core::ellipsoid::ecef_to_geodetic(player.pos);
                (
                    lat.to_degrees(),
                    lon.to_degrees(),
                    player.heading.to_degrees(),
                )
            })
            .unwrap_or_else(|| {
                (
                    self.camera.lat.to_degrees(),
                    self.camera.lon.to_degrees(),
                    self.camera.heading.to_degrees(),
                )
            })
    }

    fn frame_menu(&mut self) {
        let (lat_deg, lon_deg, heading_deg) = self.map_pose();
        let ui_paint = if self.ui.controls_visible() {
            self.ui.run_controls(&self.window)
        } else {
            self.ui.run_map(
                &self.window,
                lat_deg,
                lon_deg,
                heading_deg,
                self.location_tracker.current_name(),
            )
        };
        let result = (|| -> Result<(), crate::gfx::SurfaceFrameError> {
            let target = self.gfx.begin_map_frame()?;
            self.ui.paint(
                &self.gfx.renderer.device,
                &self.gfx.renderer.queue,
                &target.view,
                self.gfx.config.width,
                self.gfx.config.height,
                ui_paint,
            );
            target.present(&self.gfx.renderer.queue);
            Ok(())
        })();
        self.surface_occluded = matches!(&result, Err(crate::gfx::SurfaceFrameError::Occluded));
        match result {
            Ok(()) | Err(crate::gfx::SurfaceFrameError::Occluded) => {}
            Err(crate::gfx::SurfaceFrameError::Lost | crate::gfx::SurfaceFrameError::Outdated) => {
                let size = self.window.inner_size();
                self.resize(size.width, size.height);
            }
            Err(error) => log::warn!("menu frame error: {error:?}"),
        }
    }

    fn frame(&mut self) {
        self.frame_no += 1;
        // The browser downloads the assets, so it waits until they are due;
        // natively they are bundled and prepared from the first frame.
        #[cfg(target_arch = "wasm32")]
        let vehicles_due = self.player.is_some() || self.ui.first_view_settled();
        #[cfg(not(target_arch = "wasm32"))]
        let vehicles_due = true;
        if let Some(prepared) = self.deferred_vehicles.poll(vehicles_due) {
            let renderer = &mut self.gfx.renderer;
            renderer.upload_player_parts_with_uv(&prepared.set.meshes, &prepared.set.uvs);
            for (layer, levels) in (0..).zip(&prepared.texture_layers) {
                renderer.write_vehicle_texture_layer(layer, levels);
            }
        }
        if self.ui.controls_visible() || self.ui.map_visible() {
            self.last_frame = Some(Instant::now());
            self.benchmark_frame_deadline = None;
            self.frame_menu();
            return;
        }
        if self.replay_benchmark.is_some() {
            let now = Instant::now();
            let deadline = self.benchmark_frame_deadline.get_or_insert(now);
            if let Some(delay) = deadline.checked_duration_since(now) {
                vearth_platform::thread::sleep(delay);
            }
            *deadline = replay_benchmark_next_deadline(*deadline, Instant::now());
        }
        let t_start = Instant::now();
        let frame_interval_s = self
            .last_frame
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(1.0 / 60.0);
        let dt = frame_interval_s.min(0.1);
        let mut drone_player_dt = 0.0;
        self.last_frame = Some(t_start);
        let photo_locked = self.photo_job.is_some();
        let measurement_frame = (!photo_locked)
            .then(|| self.replay.as_ref().map(TracePlayer::next_frame))
            .flatten()
            .filter(|frame| {
                self.replay_benchmark
                    .as_ref()
                    .and_then(|b| b.measurement.as_ref())
                    .is_some_and(|measurement| measurement.contains(*frame))
            });
        let measurement_start = measurement_frame.map(|frame| {
            let (work, counters) = self.replay_measurement_work();
            let benchmark = self.replay_benchmark.as_mut().unwrap();
            benchmark.measurement.as_mut().unwrap().begin_frame(
                frame,
                t_start.duration_since(benchmark.started_at).as_secs_f64() * 1e3,
                work,
                counters,
                vearth_gen::contracts::generation_timing_snapshot,
            )
        });
        let mut measurement_result = None;
        if let Some(job) = &self.photo_job {
            self.camera = job.camera.clone();
        }
        // Replays advance an accelerated cycle by a fixed frame delta so an
        // explicit --time/--time-scale remains deterministic across runs.
        let solar_dt = if self.replay.is_some() {
            1.0 / 60.0
        } else {
            dt
        };
        self.solar.tick(if photo_locked { 0.0 } else { solar_dt });
        let surface_month = crate::solar::utc_month(self.solar.utc_seconds());
        if self.pool.set_imagery_month(surface_month) {
            log::info!("terrain colour: switched to Blue Marble month {surface_month:02}");
            self.invalidate_generated_world();
        }
        self.camera.aspect = self.photo_job.as_ref().map_or_else(
            || self.gfx.config.width as f64 / (self.gfx.config.height as f64).max(1.0),
            |job| job.width as f64 / (job.height as f64).max(1.0),
        );
        // Enforce the zoom-out stop continuously (covers resizes too).
        self.camera.alt_m = self.camera.alt_m.min(self.camera.max_zoom_out_alt());
        if self.mode == Mode::Explore && !photo_locked {
            if let Some(pilot) = self.drone_benchmark.as_ref()
                && self
                    .search_benchmark
                    .as_ref()
                    .is_some_and(|search| search.spec.city_tour)
            {
                let controls = pilot.explore_controls();
                self.keys = Keys {
                    zoom_in: controls.zoom_in,
                    zoom_out: controls.zoom_out,
                    east: controls.east,
                    tilt_down: controls.tilt_down,
                    ..Keys::default()
                };
            }
            self.apply_keys(dt);
            // Camera altitude is ellipsoidal. Keep an Explore camera above
            // resident terrain, including elevated ground; Play manages its
            // own drone and camera-boom collision response.
            let dir = self.camera.eye_ecef().normalize_or(glam::DVec3::X);
            let terrain_altitude_m = self.index.terrain_surface_at(dir).or_else(|| {
                self.index
                    .terrain_top_under(dir)
                    .map(|height| height as f64)
            });
            if let Some(top) = terrain_altitude_m {
                self.camera.alt_m = self.camera.alt_m.max(top.max(0.0) + 25.0);
            }
        }
        // Teleport animation owns the camera while active.
        let flying = if photo_locked {
            self.flyto.is_some()
        } else {
            let mut landed = false;
            if let Some(fly) = &mut self.flyto
                && !fly.update(&mut self.camera, dt)
            {
                self.flyto = None;
                self.dest_prefetch.clear();
                if let Some(search) = &mut self.search_benchmark {
                    let now = Instant::now();
                    let benchmark = self
                        .replay_benchmark
                        .as_mut()
                        .expect("search benchmark report");
                    if search.mark_arrived(
                        self.frame_no,
                        &self.camera,
                        now.duration_since(benchmark.started_at).as_secs_f64() * 1e3,
                    ) {
                        self.replay_finished_logged = true;
                        benchmark.motion_finished_at = Some(now);
                    }
                }
                self.lod_selector.invalidate();
                self.wanted_at = None;
                if let Some((_, arrived)) = &mut self.dive {
                    *arrived = Some(Instant::now());
                }
                landed = true;
            }
            // A teleport issued from play mode resumes play at the destination.
            if landed && self.resume_play {
                self.resume_play = false;
                self.enter_play();
            }
            self.flyto.is_some()
        };
        if let Some(drone) = &self.drone_benchmark {
            self.play_keys = drone.controls();
        }
        // Play mode: fly the drone, then drive the camera from its boom.
        if self.mode == Mode::Play
            && !flying
            && !photo_locked
            && let Some(p) = self.player.as_mut()
        {
            let local_weather = self.weather.sample_weather(p.pos, self.solar.utc_seconds());
            p.update_with_weather(
                dt,
                &self.play_keys,
                &self.index,
                local_weather.wind,
                self.weather.config.wind_influence,
            );
            drone_player_dt = dt;
            let (eye, fwd) = p.camera_pose(dt, &self.index);
            self.camera.set_pose_free(eye, fwd);
        }
        self.location_tracker.update(
            (self.mode == Mode::Play && !flying)
                .then_some(self.player.as_ref())
                .flatten(),
            &self.index,
            t_start,
        );
        self.local_time
            .update(self.camera.lat, self.camera.lon, t_start);
        // North-up behavior (Maps): while locked, heading eases to north at
        // every altitude — zooming from orbit to street keeps the map
        // north-up. A deliberate Cmd-drag rotation releases the lock; the
        // globe view's re-northing re-engages it. Pauses during a tilt drag
        // and in play mode (the follow cam owns heading there).
        if self.mode == Mode::Explore
            && self.mouse.tilt_anchor.is_none()
            && !flying
            && !photo_locked
        {
            if self.north_locked {
                self.camera.realign_north_always(dt);
            } else {
                self.camera.realign_north(dt);
                if self.camera.alt_m > 4_000_000.0 && self.camera.heading.abs() < 0.01 {
                    self.north_locked = true;
                }
            }
        }
        if !photo_locked && let Some(replay) = &mut self.replay {
            replay.next_pose().apply(&mut self.camera);
            self.north_locked = false;
            if replay.finished() && !self.replay_finished_logged {
                self.replay_finished_logged = true;
                if let Some(benchmark) = self.replay_benchmark.as_mut() {
                    // Start the settle wall clock when the trace reaches its
                    // final pose, not after the first final-pose frame has
                    // finished. That frame can legitimately be expensive
                    // while a cold destination cut is adopted.
                    benchmark.motion_finished_at = Some(Instant::now());
                }
                log::info!("camera replay reached its final frame; holding the final pose");
            }
        }
        let cam_pos = self.camera.eye_ecef();
        let capture_height = self
            .photo_job
            .as_ref()
            .map_or(self.gfx.config.height, |job| job.height);

        // Adopt completed interactive work in O(1). Replays and photos stay
        // synchronous by default: their frame-indexed/output-resolution LOD
        // must not depend on host scheduling.
        let async_lod = !photo_locked
            && lod_worker_enabled()
            && (self.replay.is_none() || async_replay_lod_enabled());
        if async_lod && let Some(completed) = self.lod_selector.take_latest() {
            log::trace!(
                "LOD worker request {} completed in {:.2} ms",
                completed.id,
                completed.compute_time.as_secs_f64() * 1e3
            );
            if let Some(search) = &mut self.search_benchmark {
                search.record_selection_adopted(completed.id);
            }
            self.adopt_selection(completed.selection);
        }

        // Refresh the wanted set on a timer. The first selection is kept
        // synchronous so startup never presents an empty world; subsequent
        // interactive refreshes only enqueue a camera snapshot.
        // Replays use a fixed seven-frame cadence (~117 ms at 60 Hz), so
        // their default synchronous path remains exactly reproducible.
        let wanted_due = if self.replay.is_some() {
            self.frame_no == 1 || self.frame_no.is_multiple_of(7)
        } else {
            self.wanted_at.is_none_or(|t| t.elapsed() >= WANTED_REFRESH)
        };
        if wanted_due {
            self.wanted_at = Some(Instant::now());
            let request = self.selection_request(cam_pos, capture_height as f64, photo_locked);
            if async_lod && !self.wanted.is_empty() {
                let id = self.lod_selector.submit(request);
                if let Some(search) = &mut self.search_benchmark {
                    search.record_selection_request(id);
                }
            } else {
                self.index.begin_metric_query();
                let selection = compute_selection(
                    request,
                    |cell| self.index.metric(cell),
                    |cell| self.index.contains(cell),
                );
                self.index.finish_metric_query();
                if let Some(search) = &mut self.search_benchmark {
                    search.record_synchronous_selection();
                }
                self.adopt_selection(selection);
            }
        }
        let t_wanted = t_start.elapsed();

        self.pool.pump();
        let t_pump = t_start.elapsed();

        let mut intents = std::mem::take(&mut self.streaming_intents);
        intents.rebuild(
            &self.wanted,
            &self.index,
            self.transitions.iter_draws().map(|draw| draw.cell),
            &self.dest_prefetch,
            &self.predicted_prefetch,
        );

        // Completion is intentionally separate from GPU admission. Shared
        // source tiles can release many workers together; stage the convoy,
        // reject work that is no longer useful, then check byte and elapsed-time
        // budgets between uploads. One oversized mesh can still make progress.
        // Completion intake stops at the CPU
        // staging high watermark, leaving overflow in the bounded worker
        // channel so useful finished meshes are never dropped and regenerated.
        let mut next_completion = if self.ready_meshes.bytes() < READY_MESH_BYTES {
            self.pool.drain_done_one()
        } else {
            None
        };
        if next_completion.is_some() || !self.ready_meshes.is_empty() {
            // Reclaim stale staged work before deciding whether the intake
            // watermark still has room for another completed mesh.
            self.ready_meshes
                .reprioritize(|cell| intents.priority(cell));
            let stale = self.ready_meshes.drop_stale(|cell| intents.contains(cell));
            self.telemetry.totals.stale_gpu_admissions += stale.cells as u64;
            if next_completion.is_none() && self.ready_meshes.bytes() < READY_MESH_BYTES {
                next_completion = self.pool.drain_done_one();
            }

            let mut drained = 0usize;
            while let Some(meshed) = next_completion {
                drained += 1;
                if self.index.contains(meshed.cell()) {
                    self.dup_results += 1;
                    self.index.touch(meshed.cell(), self.frame_no);
                } else if let Some(priority) = intents.priority(meshed.cell()) {
                    if !self.ready_meshes.push(meshed, priority) {
                        self.dup_results += 1;
                    }
                } else {
                    self.telemetry.totals.stale_results += 1;
                }

                if drained >= COMPLETION_DRAINS_PER_FRAME
                    || self.ready_meshes.bytes() >= READY_MESH_BYTES
                {
                    break;
                }
                next_completion = self.pool.drain_done_one();
            }
        }

        let upload_started = Instant::now();
        let mut uploaded_bytes = 0usize;
        let mut uploaded_cells = 0usize;
        let mut lod_metric_updates = Vec::new();
        while let Some(next_bytes) = self.ready_meshes.next_gpu_bytes() {
            if !upload_fits_budget(
                uploaded_cells,
                uploaded_bytes,
                next_bytes,
                UPLOAD_BYTES_PER_FRAME,
            ) || (uploaded_cells > 0 && upload_started.elapsed() >= UPLOAD_TIME_PER_FRAME)
            {
                break;
            }
            let Some(mut staged) = self.ready_meshes.pop_next() else {
                break;
            };
            let cell = staged.meshed.cell();
            if self.index.contains(cell) {
                self.dup_results += 1;
                continue;
            }
            let height_range = staged.meshed.height_range();
            let terrain_step_m = staged.meshed.terrain_step_m();
            let terrain_relief_m = staged.meshed.terrain_relief_m();
            let terrain_detail = staged.meshed.terrain_detail().cloned();
            let resident = self.index.insert(
                cell,
                self.frame_no,
                height_range,
                terrain_step_m,
                terrain_relief_m,
                terrain_detail.clone(),
                None,
                0,
            );
            self.index.set_metric_complete(cell, staged.meshed.complete);
            let exact_gpu_bytes =
                self.gfx
                    .renderer
                    .upload_prepared_cell(resident, &staged.meshed) as usize;
            let solid_collision = (cell.level >= crate::streaming::COLLISION_MIN_LEVEL)
                .then(|| staged.meshed.take_solid_collision());
            let terrain_collision = (cell.level >= crate::streaming::COLLISION_MIN_LEVEL)
                .then(|| staged.meshed.take_terrain_collision())
                .flatten();
            let water = staged.meshed.take_water();
            let road_guides = staged.meshed.take_road_guides();
            assert!(
                self.index.finish_upload(
                    resident,
                    solid_collision,
                    terrain_collision,
                    water,
                    road_guides,
                    exact_gpu_bytes,
                ),
                "newly inserted resident must still be live after GPU upload"
            );
            self.leases.register_resident(cell, self.frame_no);
            debug_assert!(self.gfx.renderer.contains_resident(resident));
            lod_metric_updates.push((
                cell,
                TerrainMetric {
                    height_range,
                    terrain_step_m,
                    terrain_relief_m,
                    terrain_detail,
                },
                staged.meshed.complete,
            ));
            uploaded_cells += 1;
            uploaded_bytes = uploaded_bytes.saturating_add(exact_gpu_bytes);
        }
        self.lod_selector.upsert(lod_metric_updates);
        self.telemetry.totals.uploaded_cells += uploaded_cells as u64;
        self.telemetry.totals.uploaded_bytes += uploaded_bytes as u64;
        let t_upload = t_start.elapsed();

        // Resolve what to draw; hand the pool the full missing set with
        // priorities. The pool's pending queue is *replaced* — anything the
        // camera no longer wants is dropped unstarted, so a fast pan or an
        // interrupted dive never queues stale work in front of the current
        // view.
        let mut cut = std::mem::take(&mut self.drawable_cut_cells);
        let mut missing = std::mem::take(&mut self.drawable_cut_missing);
        let cut_rebuilt = self.index.drawable_cut_into(
            &self.wanted,
            &mut self.drawable_cut_arena,
            &mut cut,
            &mut missing,
        );
        // Arrival convergence concerns the exact wanted cut; speculative
        // destination/prediction work is tracked separately by the pool.
        if let Some((t0, Some(arrived))) = self.dive
            && missing.is_empty()
        {
            log::info!(
                "cold-teleport: view resolved {:.1}s after teleport ({:.1}s after arrival)",
                t0.elapsed().as_secs_f64(),
                arrived.elapsed().as_secs_f64(),
            );
            self.dive = None;
        }
        let missing_now = missing.len();
        intents.generation_jobs_into(
            &self.index,
            |cell| self.ready_meshes.contains(cell),
            cam_pos,
            self.camera.forward_ecef(),
            &mut missing,
        );
        self.pool.sync_queue(&missing);
        let t_resolve = t_start.elapsed();

        // Retargeting never reverses a partial incoming cell. The previous
        // complete cover remains until every nested handoff reaches an opaque
        // endpoint, while newly arrived fine cells still draw immediately.
        if cut_rebuilt {
            self.transition_coverage
                .rebuild(self.transitions.iter_draws().map(|draw| draw.cell));
            let previously_drawn = &self.transition_coverage;
            self.transitions.retarget(&cut, |cell| {
                if previously_drawn.fully_covers(cell) {
                    0.0
                } else {
                    1.0
                }
            });
        }
        self.transitions.advance(dt as f32);
        let transition_stats = self.transitions.stats();
        self.telemetry.totals.transitions_started = transition_stats.started;
        self.telemetry.totals.transitions_completed = transition_stats.completed;
        self.telemetry.totals.transition_reversals = transition_stats.deferred_reversals;
        if self.validate_transitions
            && transition_stats.active > 0
            && self.frame_no.is_multiple_of(30)
        {
            let holes = self.transitions.uncovered_target_regions();
            self.telemetry.totals.uncovered_transitions += holes as u64;
            if holes > 0 {
                log::error!("LOD transition coverage invariant failed in {holes} target region(s)");
            }
        }
        let draw: Vec<DrawCell> = self
            .transitions
            .iter_draws()
            .filter_map(|transition| {
                self.index.id(transition.cell).map(|resident| DrawCell {
                    resident,
                    cell: transition.cell,
                    fade: transition.fade,
                })
            })
            .collect();
        let t_transition = t_start.elapsed();
        lease_presented_cells(&draw, &mut self.index, &mut self.leases, self.frame_no);

        intents.lease_prefetch(&mut self.leases, self.frame_no);

        // Crossing the residency watermark schedules a bounded LRU byte batch;
        // eviction remains incremental and respects active leases.
        let mesh_stats = self.gfx.renderer.mesh_stats();
        let resident_bytes = mesh_stats.total_bytes() as usize;
        debug_assert_eq!(mesh_stats.world_bytes() as usize, self.index.gpu_bytes());
        let budget = ResidencyBudget::for_host();
        let victims = if self.legacy_residency {
            self.index.eviction_victims(self.frame_no)
        } else {
            eviction_plan(
                self.index.len(),
                resident_bytes,
                self.frame_no,
                &mut self.leases,
                budget,
                |cell| {
                    let (_, record) = self
                        .index
                        .get_cell(cell)
                        .expect("registered eviction candidate must remain resident");
                    ResidentCell {
                        cell,
                        gpu_bytes: record.gpu_bytes,
                        last_used_frame: record.last_drawn_frame,
                    }
                },
            )
        };
        self.telemetry.totals.protected_eviction_attempts =
            self.leases.protected_eviction_attempts();
        if !victims.is_empty() {
            let evicted_bytes: usize = victims
                .iter()
                .filter_map(|cell| {
                    self.index
                        .get_cell(*cell)
                        .map(|(_, record)| record.gpu_bytes)
                })
                .sum();
            let resident_victims: Vec<_> = victims
                .iter()
                .filter_map(|cell| self.index.id(*cell))
                .collect();
            self.gfx.renderer.evict(&resident_victims);
            for v in &victims {
                self.index.remove(*v);
                self.leases.unregister_resident(*v);
            }
            self.index.prune_metric_history();
            debug_assert_eq!(self.gfx.renderer.resident_count(), self.index.len());
            self.lod_selector.remove(victims.clone());
            self.telemetry.totals.evicted_cells += victims.len() as u64;
            self.telemetry.totals.evicted_bytes += evicted_bytes as u64;
        }
        let t_evict = t_start.elapsed();

        let lighting = self.solar.lighting();
        let sky_visibility = crate::sky::SkyVisibility::at(cam_pos, lighting);

        let mut screenshot_requested = false;
        let ui_paint = {
            self.cache_usage.poll();
            let (queued, generating) = self.pool.counts();
            let provider_controls = self.pool.provider_controls();
            let weather_sample = self
                .weather
                .sample_weather(cam_pos, self.solar.utc_seconds());
            let (tick0, tick1, tick_alpha) = WeatherModel::tick_pair(self.solar.utc_seconds());
            let local_time = self.local_time.hud(self.solar.utc_seconds());
            let hud = HudInfo {
                visible_since: self.visible_since,
                lat_deg: self.camera.lat.to_degrees(),
                lon_deg: self.camera.lon.to_degrees(),
                alt_m: self.camera.alt_m,
                heading_deg: self.camera.heading.to_degrees(),
                pitch_deg: self.camera.pitch.to_degrees(),
                fps: self.fps,
                resident_cells: self.index.len(),
                queued,
                generating,
                cold_pct: self.hud_cold_pct,
                provider_status: self.pool.provider_status(),
                bortle_class: sky_visibility.bortle_class,
                limiting_magnitude: sky_visibility.limiting_magnitude,
                lod_ruggedness: crate::streaming::ruggedness_policy().label(),
                local_time,
                sun: SunHud {
                    time_scale: self.solar.time_scale(),
                    paused: self.solar.paused(),
                },
                weather: WeatherHud {
                    mode: self.weather.config.mode,
                    quality: self.weather.config.quality,
                    wind_influence: self.weather.config.wind_influence,
                    seed: self.weather.config.seed,
                    preset: self.weather.config.preset,
                    atlas_version: self.weather.atlas_version(),
                    model_version: self.weather.model_version(),
                    tick0,
                    tick1,
                    tick_alpha,
                    temperature_c: weather_sample.air_temperature_c,
                    humidity: weather_sample.relative_humidity,
                    precipitation_mm_h: weather_sample.precipitation_mm_h,
                    precipitation_wet_fraction: weather_sample.precipitation_wet_fraction,
                    precipitation_conditional_mm_h: weather_sample.precipitation_conditional_mm_h,
                    precipitation_kind: weather_sample.precipitation_kind,
                    cloud_coverage: weather_sample.cloud_coverage,
                    low_cloud_coverage: weather_sample.low_cloud_coverage,
                    mid_cloud_coverage: weather_sample.mid_cloud_coverage,
                    high_cloud_coverage: weather_sample.high_cloud_coverage,
                    cloud_optical_depth: weather_sample.cloud_optical_depth,
                    aerosol: weather_sample.aerosol,
                    aerosol_dust_fraction: weather_sample.aerosol_dust_fraction,
                    visibility_m: weather_sample.visibility_m,
                    wind_east_mps: weather_sample.wind.east_mps,
                    wind_north_mps: weather_sample.wind.north_mps,
                    wind_low: [
                        weather_sample.wind_low.east_mps,
                        weather_sample.wind_low.north_mps,
                    ],
                    wind_mid: [
                        weather_sample.wind_mid.east_mps,
                        weather_sample.wind_mid.north_mps,
                    ],
                    wind_upper: [
                        weather_sample.wind_upper.east_mps,
                        weather_sample.wind_upper.north_mps,
                    ],
                    wetness: weather_sample.wetness,
                    snow: weather_sample.snow_cover,
                    sea_state: weather_sample.sea_state,
                },
                photo: self.photo_status.clone(),
                play: self
                    .player
                    .as_ref()
                    .filter(|_| self.mode == Mode::Play)
                    .map(|p| PlayHud {
                        vehicle: p.vehicle,
                        vehicle_name: p.vehicle.name(),
                        phased: p.is_phased(),
                        speed_ms: p.speed_ms(),
                        airspeed_ms: p.airspeed_ms(),
                        altitude_m: vearth_core::ellipsoid::ecef_to_geodetic(p.pos).2,
                        boost: self.play_keys.boost,
                        boost_level: p.flame,
                    }),
                location_toast: self.location_tracker.toast(t_start),
                preferences: &self.preferences,
                provider_controls: &provider_controls,
                storage: StorageHud {
                    cache_path: &self.runtime_paths.cache,
                    usage: self.cache_usage.usage(),
                    scanning: self.cache_usage.scanning(),
                    error: self.cache_usage.error(),
                    active_limit_gb: self.active_cache_limit_gb,
                    pending_clear: self.pending_cache_clear,
                },
            };
            let (mut action, paint) = self.ui.run(&self.window, &hud);
            if let Some(search) = &mut self.search_benchmark {
                let elapsed_ms = self
                    .replay_benchmark
                    .as_ref()
                    .expect("search benchmark report")
                    .started_at
                    .elapsed()
                    .as_secs_f64()
                    * 1e3;
                if let Some(place) = search.take_action(self.frame_no, &self.camera, elapsed_ms) {
                    action = UiAction::FlyTo(place);
                }
            }
            match &action {
                UiAction::ResetNorth => {
                    // Let the existing north-lock easing animate the turn on
                    // subsequent frames; assigning zero here caused a snap.
                    self.north_locked = true;
                }
                UiAction::ToggleSunPause => self.solar.toggle_paused(),
                UiAction::ShiftSunHours(hours) => {
                    self.solar.shift_hours(*hours);
                }
                UiAction::SetSunTimeScale(scale) => {
                    self.solar.set_time_scale(*scale);
                }
                UiAction::SetSunUtc(utc) => self.solar.set_utc_seconds(*utc),
                UiAction::ApplyLandmarkCalibration { value, recenter } => {
                    let selected_changed = self
                        .calibration_landmark
                        .is_none_or(|landmark| landmark.name != value.name);
                    if selected_changed {
                        let landmark = vearth_gen::landmarks::LANDMARKS
                            .iter()
                            .find(|landmark| landmark.name == value.name)
                            .expect("selected calibration landmark exists");
                        // Generate the newly selected model once at its
                        // registry placement so those vertices receive the
                        // calibration tag. Subsequent edits are GPU-only.
                        let source = vearth_gen::landmarks::CalibrationOverride {
                            name: landmark.name,
                            east_m: 0.0,
                            north_m: 0.0,
                            up_m: 0.0,
                            heading_deg: landmark.heading_deg,
                        };
                        vearth_gen::landmarks::set_calibration_override(Some(source));
                        self.calibration_landmark = Some(landmark);
                        self.pool.invalidate_generation();
                        self.invalidate_generated_world();
                    }
                    self.gfx.renderer.set_landmark_calibration(Some(*value));
                    if *recenter {
                        frame_calibration_camera(&mut self.camera, *value);
                        self.north_locked = true;
                        self.lod_selector.invalidate();
                        self.wanted_at = None;
                    }
                }
                UiAction::CapturePhoto(settings) => self.request_photo(settings.clone()),
                UiAction::CaptureScreenshot => screenshot_requested = true,
                UiAction::ExitPlay => self.exit_play(),
                UiAction::ApplyPreferences(preferences) => self.apply_preferences(preferences),
                UiAction::ResetPreferences => self.reset_preferences(),
                UiAction::RefreshCacheUsage => self.cache_usage.refresh(),
                UiAction::ScheduleCacheClear(requested) => self.schedule_cache_clear(*requested),
                UiAction::CancelCacheClear => self.cancel_cache_clear(),
                #[cfg(target_arch = "wasm32")]
                UiAction::ClearBrowserData => {
                    crate::host::post_to_page("clearSiteData", wasm_bindgen::JsValue::NULL)
                }
                UiAction::None | UiAction::FlyTo(_) => {}
            }
            match action {
                UiAction::FlyTo(place) => {
                    let end_alt = place.view_altitude_m.clamp(200.0, 12_000_000.0);
                    log::info!(
                        "teleport: {} ({:.4}, {:.4}) → alt {:.0} m",
                        place.display_name,
                        place.lat_deg,
                        place.lon_deg,
                        end_alt
                    );
                    // Teleporting from play mode: the explore camera flies the
                    // cinematic, then the drone re-spawns at the destination.
                    if self.mode == Mode::Play {
                        self.mode = Mode::Explore;
                        self.player = None;
                        self.play_keys = PlayKeys::default();
                        self.camera.pitch = self.camera.pitch.clamp(0.0, 1.45);
                        self.resume_play = true;
                    }
                    self.flyto = Some(FlyTo::new(
                        &self.camera,
                        place.lat_deg,
                        place.lon_deg,
                        end_alt,
                    ));
                    self.lod_selector.invalidate();
                    self.wanted_at = None;
                    self.dive = Some((Instant::now(), None));
                }
                _ => {}
            }
            paint
        };

        let player_draw: Option<PlayerDraw> = self
            .player
            .as_ref()
            .filter(|_| self.mode == Mode::Play && !flying)
            .map(|p| {
                let (model, axes) = p.model_matrix();
                let layout = self.vehicle_parts;
                let parts = layout.draw_parts(p.vehicle);
                PlayerDraw {
                    model,
                    axes,
                    parts,
                    effects: Some(crate::gfx::VehicleEffects {
                        kind: p.vehicle,
                        throttle: (p.speed_ms() as f32
                            / if p.vehicle.is_phased() { 180.0 } else { 35.0 })
                        .clamp(0.0, 1.0),
                        boost: p.flame as f32,
                    }),
                }
            });

        if self.photo_job.is_some() {
            let photo_draw: Vec<DrawCell> = cut
                .iter()
                .filter_map(|&cell| {
                    self.index
                        .id(cell)
                        .map(|resident| DrawCell::opaque(resident, cell))
                })
                .collect();
            self.gfx
                .renderer
                .prepare_near_tree_assets(cam_pos, &photo_draw);
            let tree_remaining = self.gfx.renderer.pending_tree_assets();
            let remaining = missing_now + tree_remaining;
            if let Some(job) = &self.photo_job {
                self.photo_status = PhotoStatus::Preparing {
                    width: job.width,
                    height: job.height,
                    remaining,
                };
            }

            if remaining == 0 {
                let job = self.photo_job.take().expect("photo job exists");
                self.photo_status = PhotoStatus::Capturing {
                    width: job.width,
                    height: job.height,
                };
                let capture_format = self.gfx.view_format();
                let photo_cam_pos = job.camera.eye_ecef();
                let lighting = job.solar.lighting();
                let weather = job
                    .weather
                    .render_state(photo_cam_pos, job.solar.utc_seconds());
                let recipe = PhotoRecipe {
                    lat_deg: job.camera.lat.to_degrees(),
                    lon_deg: job.camera.lon.to_degrees(),
                    alt_m: job.camera.alt_m,
                    heading_deg: job.camera.heading.to_degrees(),
                    pitch_deg: job.camera.pitch.to_degrees(),
                    fov_y_deg: job.camera.fov_y.to_degrees(),
                    utc_seconds: job.solar.utc_seconds(),
                    weather: job.weather.config,
                };
                let capture_frame = CaptureFrame {
                    width: job.width,
                    height: job.height,
                    output_format: capture_format,
                    cam_pos: photo_cam_pos,
                    view_rot: job.camera.view_rotation(),
                    fov_y: job.camera.fov_y,
                    draw_cells: &photo_draw,
                    lighting,
                    weather,
                    player: player_draw.as_ref(),
                    render_samples: still_render_samples(),
                };
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let result =
                        render_rgba(&mut self.gfx.renderer, capture_frame).and_then(|capture| {
                            write_photo_bundle(
                                &job.settings,
                                job.width,
                                job.height,
                                &capture.rgba,
                                recipe,
                            )
                        });
                    self.photo_status = match result {
                        Ok(artifact) => {
                            log::info!(
                                "photo: wrote {} and {}",
                                artifact.image_path.display(),
                                artifact.recipe_path.display()
                            );
                            PhotoStatus::Saved(artifact)
                        }
                        Err(error) => {
                            log::error!("photo capture failed: {error}");
                            PhotoStatus::Failed(error.to_string())
                        }
                    };
                }
                #[cfg(target_arch = "wasm32")]
                self.submit_web_capture(
                    capture_frame,
                    web::CaptureTarget::Photo {
                        settings: job.settings,
                        recipe,
                    },
                );
                self.wanted_at = None;
                self.split_set.clear();
                self.prediction_anchor = None;
                self.lod_selector.invalidate();
                self.camera.aspect =
                    self.gfx.config.width as f64 / (self.gfx.config.height as f64).max(1.0);
            }
        }

        let frame_result = (|| -> Result<crate::gfx::FrameStats, crate::gfx::SurfaceFrameError> {
            let target = self.gfx.begin_frame()?;
            let weather_render = self.weather.render_state(cam_pos, self.solar.utc_seconds());
            if screenshot_requested {
                // Capture the current drawable scene at window resolution, before
                // the separate UI pass. Do not wait for a new photo LOD selection.
                let capture_frame = CaptureFrame {
                    width: self.gfx.config.width,
                    height: self.gfx.config.height,
                    output_format: self.gfx.view_format(),
                    cam_pos,
                    view_rot: self.camera.view_rotation(),
                    fov_y: self.camera.fov_y,
                    draw_cells: &draw,
                    lighting,
                    weather: weather_render,
                    player: player_draw.as_ref(),
                    render_samples: 1,
                };
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let result = (|| -> Result<_, Box<dyn std::error::Error>> {
                        let desktop = crate::platform_paths::desktop_directory()?;
                        let (width, height) = (capture_frame.width, capture_frame.height);
                        let capture = render_rgba(&mut self.gfx.renderer, capture_frame)?;
                        write_screenshot(&desktop, width, height, &capture.rgba)
                    })();
                    let notice = match result {
                        Ok(path) => {
                            log::info!("screenshot saved: {}", path.display());
                            format!(
                                "Saved to {}\nClick to take another screenshot",
                                path.display()
                            )
                        }
                        Err(error) => {
                            log::error!("screenshot failed: {error}");
                            format!("Screenshot failed: {error}\nClick to retry")
                        }
                    };
                    self.ui.set_screenshot_notice(notice);
                }
                #[cfg(target_arch = "wasm32")]
                self.submit_web_capture(capture_frame, web::CaptureTarget::Screenshot);
            }
            let stats = self.gfx.render_world(
                &target,
                cam_pos,
                self.camera.view_rotation(),
                self.camera.fov_y,
                &draw,
                lighting,
                weather_render,
                player_draw.as_ref(),
            );
            self.ui.paint(
                &self.gfx.renderer.device,
                &self.gfx.renderer.queue,
                &target.view,
                self.gfx.config.width,
                self.gfx.config.height,
                ui_paint,
            );
            target.present(&self.gfx.renderer.queue);
            Ok(stats)
        })();

        self.surface_occluded =
            matches!(&frame_result, Err(crate::gfx::SurfaceFrameError::Occluded));
        let (wanted_presented_delta, replay_presentation) = if self.replay_benchmark.is_some() {
            let desired: HashSet<_> = self.wanted.iter().map(|wanted| wanted.cell).collect();
            let presented: HashMap<_, _> = draw.iter().map(|draw| (draw.cell, draw.fade)).collect();
            let counts =
                replay_lane_counts(&self.wanted, &presented, |cell| self.index.contains(cell));
            self.replay_benchmark.as_mut().unwrap().record_lanes(counts);
            let delta = desired
                .iter()
                .filter(|cell| !presented.contains_key(*cell))
                .count()
                + presented
                    .keys()
                    .filter(|cell| !desired.contains(*cell))
                    .count();
            (delta, Some(ReplayPresentation { desired, presented }))
        } else {
            (0, None)
        };
        let drone_surface_presented = frame_result.is_ok();
        let mut wanted_cells_by_level = [0; 32];
        let mut drawn_cells_by_level = [0; 32];
        if self.drone_benchmark.is_some() {
            for wanted in &self.wanted {
                wanted_cells_by_level[usize::from(wanted.cell.level)] += 1;
            }
            for cell in &draw {
                drawn_cells_by_level[usize::from(cell.cell.level)] += 1;
            }
        }
        let drone_render = frame_result.as_ref().map_or_else(
            |_| crate::benchmark_drone::FlightRenderStats::default(),
            |stats| crate::benchmark_drone::FlightRenderStats {
                gpu_sample_id: stats.gpu_timing_sample_id,
                gpu_total_ms: stats.gpu_total_ms,
                gpu_world_ms: stats.gpu_world_ms,
                gpu_shadow_ms: stats.gpu_shadow_ms,
                resident_gpu_bytes: stats.mesh.total_bytes(),
                upload_bytes: stats.uploads.total_bytes(),
                world_triangles: stats.world_triangles,
                opaque_world_indices: stats.opaque_world_indices,
                feedback_world_indices: stats.feedback_world_indices,
                shadow_caster_indices: stats.sun_shadow_caster_indices,
                requested_draw_cells: stats.requested_cells,
                submitted_chunks: stats.submitted_chunks,
                wanted_cells_by_level,
                drawn_cells_by_level,
            },
        );
        if let Some(replay) = &self.replay
            && let Some(benchmark) = self.replay_benchmark.as_mut()
            && !benchmark.report_written
        {
            benchmark.progress_surface_frames[usize::from(frame_result.is_err())] += 1;
            if let Ok(stats) = &frame_result
                && let Some(gpu_ms) = stats.gpu_total_ms
            {
                benchmark.progress_last_gpu_ms = Some(gpu_ms);
            }
            let checkpoint_interval = benchmark.progress_checkpoint_at.elapsed();
            if checkpoint_interval >= Duration::from_secs(5) {
                // Count acquisition failures as progress: frame-indexed traces
                // continue at the bounded redraw cadence while unpresentable.
                let pending_jobs = self.pool.in_flight_count();
                let acquisition = vearth_data::metrics::acquisition_snapshots();
                let (queued, acquiring) =
                    acquisition.iter().fold((0u64, 0u64), |(q, a), series| {
                        (q + series.queued, a + series.in_flight)
                    });
                let fully_idle = missing_now == 0
                    && wanted_presented_delta == 0
                    && self.ready_meshes.is_empty()
                    && pending_jobs == 0
                    && queued == 0
                    && acquiring == 0;
                log::info!(
                    "native replay checkpoint: after-start-ms={:.3}; phase={}; next-trace-frame={}; window-ms={:.3}; presented={}; not-presented={}; surface={:?}; surface-occluded={}; last-observed-gpu-total-ms={:?}; pending-jobs={}; ready-meshes={}; acquisition-queued={}; acquiring={}; desired-missing={}; wanted-presented-delta={}; fully-idle={}; photo-locked={}",
                    benchmark.started_at.elapsed().as_secs_f64() * 1000.0,
                    if self.replay_finished_logged {
                        "settling"
                    } else {
                        "trace"
                    },
                    replay.next_frame(),
                    checkpoint_interval.as_secs_f64() * 1000.0,
                    benchmark.progress_surface_frames[0],
                    benchmark.progress_surface_frames[1],
                    frame_result.as_ref().map(|_| ()),
                    self.surface_occluded,
                    benchmark.progress_last_gpu_ms,
                    pending_jobs,
                    self.ready_meshes.len(),
                    queued,
                    acquiring,
                    missing_now,
                    wanted_presented_delta,
                    fully_idle,
                    photo_locked,
                );
                benchmark.progress_checkpoint_at = Instant::now();
                benchmark.progress_surface_frames = [0; 2];
            }
        }
        match frame_result {
            Ok(stats) => {
                if let Some(benchmark) = self.replay_benchmark.as_mut() {
                    benchmark.last_triangles = [
                        stats.world_triangles,
                        stats.tree_triangles,
                        stats.flora.triangles,
                    ];
                }
                let t_render = t_start.elapsed();
                self.telemetry.totals.draw_overflow +=
                    (stats.dropped_chunks + stats.dropped_player_parts) as u64;
                self.telemetry.totals.sun_shadow_capacity_drops +=
                    stats.sun_shadow_capacity_drops as u64;
                let measurement = FrameMeasurement {
                    interval_ms: frame_interval_s * 1e3,
                    main_ms: t_render.as_secs_f64() * 1e3,
                    wanted_ms: t_wanted.as_secs_f64() * 1e3,
                    pump_ms: (t_pump - t_wanted).as_secs_f64() * 1e3,
                    upload_ms: (t_upload - t_pump).as_secs_f64() * 1e3,
                    resolve_ms: (t_resolve - t_upload).as_secs_f64() * 1e3,
                    transition_ms: (t_transition - t_resolve).as_secs_f64() * 1e3,
                    residency_ms: (t_evict - t_transition).as_secs_f64() * 1e3,
                    eviction_ms: (t_evict - t_resolve).as_secs_f64() * 1e3,
                    render_submit_ms: (t_render - t_evict).as_secs_f64() * 1e3,
                    cpu_visibility_ms: stats.cpu_visibility_ms,
                    cpu_world_draw_construction_ms: stats.cpu_world_draw_construction_ms,
                    cpu_sorting_ms: stats.cpu_sorting_ms,
                    cpu_uniform_construction_ms: stats.cpu_uniform_construction_ms,
                    cpu_grouping_ms: stats.cpu_grouping_ms,
                    cpu_indirect_construction_ms: stats.cpu_indirect_construction_ms,
                    cpu_sun_shadow_preparation_ms: stats.cpu_sun_shadow_preparation_ms,
                    upload_preparation_ms: stats.upload_preparation_ms,
                    queue_upload_ms: stats.queue_upload_ms,
                    gpu_ms: stats.gpu_total_ms,
                    gpu_shadow_ms: stats.gpu_shadow_ms,
                    gpu_world_ms: stats.gpu_world_ms,
                    gpu_flora_ms: stats.gpu_flora_ms,
                    gpu_atmosphere_ms: stats.gpu_atmosphere_ms,
                    gpu_cloud_ms: stats.gpu_cloud_ms,
                    gpu_precipitation_ms: stats.gpu_precipitation_ms,
                    gpu_tonemap_ms: stats.gpu_tonemap_ms,
                    completion_backlog: self.ready_meshes.len(),
                    oldest_ready_ms: self.ready_meshes.oldest_ready_ms(),
                    wanted_presented_delta,
                    world_draw_batches: stats.submitted_world_batches
                        + stats.submitted_tree_batches
                        + stats.submitted_light_batches
                        + stats.flora.draw_calls,
                    sun_shadow_caster_chunks: stats.sun_shadow_caster_chunks,
                    sun_shadow_caster_indices: stats.sun_shadow_caster_indices,
                    sun_shadow_batches: stats.sun_shadow_batches,
                    submitted_world_vertices: stats.submitted_world_vertices,
                    world_material_indices: stats.world_material_indices,
                    opaque_world_indices: stats.opaque_world_indices,
                    feedback_world_indices: stats.feedback_world_indices,
                    triangles: stats.triangles,
                    frame_upload_bytes: stats.uploads.total_bytes()
                        + stats.world_draw_upload_bytes as u64
                        + stats.tree_instance_upload_bytes as u64
                        + stats.flora.instance_upload_bytes as u64,
                    resident_gpu_bytes: stats.mesh.total_bytes(),
                };
                self.telemetry.record_frame(measurement);
                if self
                    .replay_benchmark
                    .as_ref()
                    .is_some_and(|b| b.measurement.is_some())
                {
                    measurement_result = Some((
                        measurement,
                        stats.gpu_timing_issued_id,
                        stats.gpu_timing_completed,
                    ));
                }
                self.fps_window.1 += 1;
                if self.fps_window.0.elapsed().as_secs_f64() > 2.0 {
                    let window_s = self.fps_window.0.elapsed().as_secs_f64();
                    let fps = self.fps_window.1 as f64 / window_s;
                    self.fps = fps;
                    // Quantified pipeline health: per-window generation and
                    // network deltas. `dup` must stay 0; nonzero means the
                    // pool re-did finished work.
                    let (n_gen, n_hit) = self.pool.stats();
                    let (nreq, nbytes, nerr) = vearth_data::metrics::snapshot();
                    let (lg, lh, lr, lb, ld) = self.last_stats;
                    self.last_stats = (n_gen, n_hit, nreq, nbytes, self.dup_results);
                    // Cold share for the HUD: how much of this window's
                    // work skipped the chunk cache (i.e. paid the full
                    // sample+fetch path).
                    let (d_gen, d_hit) = (n_gen - lg, n_hit - lh);
                    // These counters are independent relaxed atomics; a
                    // snapshot can briefly observe a cache hit before the
                    // corresponding generated count. Clamp that transient
                    // skew instead of underflowing in debug builds.
                    self.hud_cold_pct =
                        (d_gen > 0).then(|| (100 * d_gen.saturating_sub(d_hit) / d_gen) as u32);
                    if log::log_enabled!(target: "vearth_app::diagnostics", log::Level::Debug) {
                        log::debug!(target: "vearth_app::diagnostics",
                            "fps {fps:5.1} | alt {:9.0} m | draws {:4} in {:2} batches (u16 {}, calibration {}, culled {:4}, horizon {:3}, dropped {}) | trees {} in {} batches | lights {} in {} batches | resident {:4} {:.1} MiB | q {:4} ready {} {:.1} MiB | gen {:4}/s (hit {:3}/s, dup {}) | net {:4} req/s {:5.2} MB/s (err {nerr}) | ms wanted {:.1} pump {:.1} upload {:.1} resolve {:.1} evict {:.1} render {:.1}",
                            self.camera.alt_m,
                            stats.draws,
                            stats.submitted_world_batches,
                            stats.submitted_u16_index_chunks,
                            stats.submitted_landmark_chunks,
                            stats.culled,
                            stats.culled_horizon,
                            stats.dropped_chunks + stats.dropped_player_parts,
                            stats.submitted_tree_instances,
                            stats.submitted_tree_batches,
                            stats.submitted_light_cells,
                            stats.submitted_light_batches,
                            self.index.len(),
                            stats.mesh.total_bytes() as f64 / (1024.0 * 1024.0),
                            self.pool.in_flight_count(),
                            self.ready_meshes.len(),
                            self.ready_meshes.bytes() as f64 / (1024.0 * 1024.0),
                            ((n_gen - lg) as f64 / window_s).round(),
                            ((n_hit - lh) as f64 / window_s).round(),
                            self.dup_results - ld,
                            ((nreq - lr) as f64 / window_s).round(),
                            (nbytes - lb) as f64 / window_s / 1e6,
                            t_wanted.as_secs_f64() * 1e3,
                            (t_pump - t_wanted).as_secs_f64() * 1e3,
                            (t_upload - t_pump).as_secs_f64() * 1e3,
                            (t_resolve - t_upload).as_secs_f64() * 1e3,
                            (t_evict - t_resolve).as_secs_f64() * 1e3,
                            (t_render - t_evict).as_secs_f64() * 1e3,
                        );
                        log::debug!(target: "vearth_app::diagnostics",
                            "flora resident {} visible {} lod {:?} archetype {:?} calendar {:?} registry {:?} inferred {:?} | {}/{} glyphs {:?} by LOD, {} tris {:?} by LOD in {} draws, {}/{} B upload ({} instances) | range {:.1}..{:.1} m estimated {:.0} occupied/{:.0} flower heads/{:.0} crop clusters ({:.0} plant equivalents) | suppression {} glyph/triangle/upload {}/{}/{} | curated {} | CPU prep {:.3} ms",
                            stats.flora.resident_cells,
                            stats.flora.visible_cells,
                            stats.flora.visible_by_lod,
                            stats.flora.visible_by_archetype,
                            stats.flora.visible_by_calendar,
                            stats.flora.visible_by_registry,
                            stats.flora.inferred_by_archetype,
                            stats.flora.glyphs,
                            stats.flora.glyph_budget,
                            stats.flora.glyphs_by_lod,
                            stats.flora.triangles,
                            stats.flora.triangles_by_lod,
                            stats.flora.draw_calls,
                            stats.flora.instance_upload_bytes,
                            stats.flora.instance_upload_budget_bytes,
                            stats.flora.admitted_instances,
                            stats.flora.nearest_visible_m,
                            stats.flora.farthest_visible_m,
                            stats.flora.estimated_occupied_glyphs,
                            stats.flora.estimated_flower_heads,
                            stats.flora.estimated_crop_clusters,
                            stats.flora.estimated_crop_plant_equivalents,
                            stats.flora.suppression_reason.as_str(),
                            stats.flora.cells_suppressed_by_glyph_budget,
                            stats.flora.cells_suppressed_by_triangle_budget,
                            stats.flora.cells_suppressed_by_upload_budget,
                            stats.flora.curated_destination_matches,
                            stats.flora.cpu_preparation_ms,
                        );
                        log::debug!(target: "vearth_app::diagnostics",
                            "Sun shadows configured {} active {} cascades {} | {} caster chunks / {} indices in {} batches | {} capacity drops | {:.1} MiB | CPU prep {:.3} ms",
                            stats.sun_shadow_configured,
                            stats.sun_shadow_active,
                            stats.sun_shadow_cascades,
                            stats.sun_shadow_caster_chunks,
                            stats.sun_shadow_caster_indices,
                            stats.sun_shadow_batches,
                            stats.sun_shadow_capacity_drops,
                            stats.sun_shadow_working_set_bytes as f64 / (1024.0 * 1024.0),
                            stats.cpu_sun_shadow_preparation_ms,
                        );
                        let smooth = self.telemetry.report();
                        log::debug!(target: "vearth_app::diagnostics",
                            "smooth ms frame p50 {:.1} p95 {:.1} p99 {:.1} max {:.1} | main p95 {:.1} p99 {:.1} | GPU p95 {:.2} (shadow {:.2}, world+flora {:.2}, atmo {:.2}, cloud {:.2}, precip {:.2}, tone {:.2}) | upload p95 {:.2} p99 {:.2} | ready-age p95 {:.0} | missed-vsync {} | stale result/GPU {}/{} pressure {} | transition/shadow drops {}/{} | uploaded {:.1} MiB evicted {:.1} MiB",
                            smooth.frame_interval_ms.p50,
                            smooth.frame_interval_ms.p95,
                            smooth.frame_interval_ms.p99,
                            smooth.frame_interval_ms.max,
                            smooth.main_ms.p95,
                            smooth.main_ms.p99,
                            smooth.gpu_ms.p95,
                            smooth.gpu_shadow_ms.p95,
                            smooth.gpu_world_ms.p95,
                            smooth.gpu_atmosphere_ms.p95,
                            smooth.gpu_cloud_ms.p95,
                            smooth.gpu_precipitation_ms.p95,
                            smooth.gpu_tonemap_ms.p95,
                            smooth.upload_ms.p95,
                            smooth.upload_ms.p99,
                            smooth.oldest_ready_ms.p95,
                            smooth.totals.missed_vsync,
                            smooth.totals.stale_results,
                            smooth.totals.stale_gpu_admissions,
                            smooth.totals.ready_backpressure_drops,
                            smooth.totals.uncovered_transitions,
                            smooth.totals.sun_shadow_capacity_drops,
                            smooth.totals.uploaded_bytes as f64 / (1024.0 * 1024.0),
                            smooth.totals.evicted_bytes as f64 / (1024.0 * 1024.0),
                        );
                        let acquisition = vearth_data::metrics::acquisition_snapshots();
                        let (acq_queued, acq_in_flight, acq_errors, acq_canceled, acq_cache_hits) =
                            acquisition.iter().fold(
                                (0u64, 0u64, 0u64, 0u64, 0u64),
                                |(queued, in_flight, errors, canceled, hits), series| {
                                    (
                                        queued + series.queued,
                                        in_flight + series.in_flight,
                                        errors + series.errors,
                                        canceled + series.canceled,
                                        hits + series.cache_hits,
                                    )
                                },
                            );
                        log::debug!(target: "vearth_app::diagnostics",
                            "acquisition series {} | queued {} in-flight {} | cache hits {} | errors {} canceled {}",
                            acquisition.len(),
                            acq_queued,
                            acq_in_flight,
                            acq_cache_hits,
                            acq_errors,
                            acq_canceled,
                        );
                        {
                            let stages = self.pool.stage_counts();
                            if let Some(cache) = self.pool.cache_stats() {
                                log::debug!(target: "vearth_app::diagnostics",
                                    "pipeline {} | cache-check {} acquire {} cpu {} | column RAM {} entries {:.1} MiB / {:.0} MiB (hit {} disk {} miss {})",
                                    if self.pool.staged_pipeline() {
                                        "staged"
                                    } else {
                                        "legacy"
                                    },
                                    stages.cache_check,
                                    stages.acquiring,
                                    stages.cpu,
                                    cache.resident_entries,
                                    cache.resident_bytes as f64 / (1024.0 * 1024.0),
                                    cache.budget_bytes as f64 / (1024.0 * 1024.0),
                                    cache.ram_hits,
                                    cache.disk_hits,
                                    cache.misses,
                                );
                            }
                        }
                    }
                    self.fps_window = (Instant::now(), 0);
                }
            }
            Err(crate::gfx::SurfaceFrameError::Lost | crate::gfx::SurfaceFrameError::Outdated) => {
                let size = self.window.inner_size();
                self.resize(size.width, size.height);
            }
            // The UI texture queue retains work while the surface is occluded.
            Err(crate::gfx::SurfaceFrameError::Occluded) => {}
            Err(e) => log::warn!("frame error: {e:?}"),
        }
        // Replay convergence and its wall-clock timeout describe the streaming
        // state, not swapchain visibility. Automated macOS capture runs may
        // legitimately occlude a later benchmark window; completion must still
        // advance instead of hanging forever on an unpresentable surface.
        self.drawable_cut_cells = cut;
        self.drawable_cut_missing = missing;
        self.streaming_intents = intents;
        self.persist_preferences();
        if let Some(start) = measurement_start {
            let (work, counters) = self.replay_measurement_work();
            let pose = crate::replay::ReplayPose {
                frame: measurement_frame.unwrap(),
                lat_deg: self.camera.lat.to_degrees(),
                lon_deg: self.camera.lon.to_degrees(),
                alt_m: self.camera.alt_m,
                heading_deg: self.camera.heading.to_degrees(),
                pitch_deg: self.camera.pitch.to_degrees(),
            };
            self.replay_benchmark
                .as_mut()
                .unwrap()
                .measurement
                .as_mut()
                .unwrap()
                .end_frame(
                    start,
                    pose,
                    work,
                    counters,
                    uploaded_cells,
                    uploaded_bytes,
                    measurement_result.map(|(frame, issued, _)| (frame, issued)),
                    vearth_gen::contracts::generation_timing_snapshot,
                );
        }
        if let Some((_, _, Some(completed))) = measurement_result {
            // Deliver every ready slot, including results collected during
            // later trace/settle frames, to its original issued frame ID.
            let collector = self
                .replay_benchmark
                .as_mut()
                .unwrap()
                .measurement
                .as_mut()
                .unwrap();
            for sample in completed.iter() {
                let timings = sample.timings;
                collector.observe_gpu(
                    Some(sample.id),
                    FrameMeasurement {
                        gpu_ms: Some(timings.total_ms),
                        gpu_shadow_ms: Some(timings.shadow_ms),
                        gpu_world_ms: Some(timings.world_ms),
                        gpu_flora_ms: timings.flora_ms,
                        gpu_atmosphere_ms: Some(timings.atmosphere_ms),
                        gpu_cloud_ms: Some(timings.cloud_ms),
                        gpu_precipitation_ms: Some(timings.precipitation_ms),
                        gpu_tonemap_ms: Some(timings.tonemap_ms),
                        ..FrameMeasurement::default()
                    },
                );
            }
        }
        self.update_replay_benchmark(
            missing_now,
            wanted_presented_delta,
            replay_presentation.as_ref(),
            self.drone_benchmark.as_ref().map(|_| {
                (
                    drone_player_dt,
                    frame_interval_s * 1000.0,
                    t_start.elapsed().as_secs_f64() * 1000.0,
                    drone_surface_presented,
                    drone_render,
                )
            }),
        );
    }
}

impl State {
    fn search_benchmark_deadline_reason(&self) -> Option<&'static str> {
        let (search, benchmark) = self
            .search_benchmark
            .as_ref()
            .zip(self.replay_benchmark.as_ref())?;
        if benchmark.report_written {
            return None;
        }
        if let Some(drone) = &self.drone_benchmark {
            let reason =
                drone.deadline_reason(benchmark.started_at.elapsed().as_secs_f64() * 1000.0);
            if reason.is_some() || !drone.awaiting_search() {
                return reason;
            }
        }
        search.deadline_reason(
            benchmark.started_at.elapsed(),
            benchmark.motion_finished_at.map(|at| at.elapsed()),
            benchmark.options.max_settle,
        )
    }

    fn update_replay_benchmark(
        &mut self,
        desired_missing: usize,
        wanted_presented_delta: usize,
        presentation: Option<&ReplayPresentation>,
        drone_timing: Option<(
            f64,
            f64,
            f64,
            bool,
            crate::benchmark_drone::FlightRenderStats,
        )>,
    ) {
        if let Some(benchmark) = &mut self.replay_benchmark {
            benchmark.last_desired_missing = desired_missing;
            benchmark.last_wanted_presented_delta = wanted_presented_delta;
        }
        if let Some(reason) = self.search_benchmark_deadline_reason()
            && (self.drone_benchmark.is_some() || reason == "search_motion_timeout")
        {
            self.finish_replay_benchmark(
                reason,
                desired_missing,
                wanted_presented_delta,
                vearth_data::metrics::acquisition_snapshots(),
            );
            return;
        }
        if self.replay_benchmark.is_none() || !self.replay_finished_logged {
            return;
        }

        let now = Instant::now();
        {
            let benchmark = self.replay_benchmark.as_mut().unwrap();
            if benchmark.motion_finished_at.is_none() {
                benchmark.motion_finished_at = Some(now);
            }
            if benchmark.motion_report.is_none() {
                benchmark.motion_report = Some(self.telemetry.report());
            }
        }

        let acquisition = vearth_data::metrics::acquisition_snapshots();
        // The existing "visible" metric covers the whole wanted cut, including
        // guard/background lanes. Keep it comparable with earlier reports;
        // foreground readiness below separately measures opaque in-view targets.
        let selection_ready = self
            .search_benchmark
            .as_ref()
            .is_none_or(SearchBenchmarkPlayer::can_count_idle);
        let selection_settled = self
            .search_benchmark
            .as_ref()
            .is_none_or(SearchBenchmarkPlayer::can_settle);
        let visible_idle = selection_ready && desired_missing == 0 && wanted_presented_delta == 0;
        let fully_idle = visible_idle
            && self.ready_meshes.is_empty()
            && self.pool.in_flight_count() == 0
            && acquisition
                .iter()
                .all(|series| series.queued == 0 && series.in_flight == 0);

        let mut terminal_reason = {
            let benchmark = self.replay_benchmark.as_mut().unwrap();
            let motion_finished_at = benchmark.motion_finished_at.unwrap();

            if selection_ready && foreground_opaque_idle(&benchmark.lane_counts) {
                let idle_started_at = *benchmark.foreground_idle_started_at.get_or_insert(now);
                benchmark.foreground_idle_frames =
                    benchmark.foreground_idle_frames.saturating_add(1);
                if benchmark.foreground_converged_after_ms.is_none()
                    && benchmark.foreground_idle_frames >= benchmark.options.idle_frames
                {
                    benchmark.foreground_converged_after_ms = Some(
                        idle_started_at
                            .duration_since(motion_finished_at)
                            .as_secs_f64()
                            * 1e3,
                    );
                }
            } else {
                benchmark.foreground_idle_started_at = None;
                benchmark.foreground_idle_frames = 0;
            }

            if visible_idle {
                let idle_started_at = *benchmark.visible_idle_started_at.get_or_insert(now);
                benchmark.visible_idle_frames = benchmark.visible_idle_frames.saturating_add(1);
                if benchmark.visible_converged_after_ms.is_none()
                    && benchmark.visible_idle_frames >= benchmark.options.idle_frames
                {
                    benchmark.visible_converged_after_ms = Some(
                        idle_started_at
                            .duration_since(motion_finished_at)
                            .as_secs_f64()
                            * 1e3,
                    );
                }
            } else {
                benchmark.visible_idle_started_at = None;
                benchmark.visible_idle_frames = 0;
            }

            if fully_idle {
                benchmark.full_idle_started_at.get_or_insert(now);
                benchmark.full_idle_frames = benchmark.full_idle_frames.saturating_add(1);
            } else {
                benchmark.full_idle_started_at = None;
                benchmark.full_idle_frames = 0;
            }

            if benchmark.full_idle_frames >= benchmark.options.idle_frames && selection_settled {
                Some("settled")
            } else if motion_finished_at.elapsed() >= benchmark.options.max_settle {
                Some("settle_timeout")
            } else {
                None
            }
        };

        if let Some(presentation) = presentation {
            let benchmark = self.replay_benchmark.as_mut().unwrap();
            let foreground = [benchmark.lane_counts[0], benchmark.lane_counts[3]];
            let observation = ReplayConvergenceFrame {
                frame: self.frame_no,
                after_trace_ms: now
                    .duration_since(benchmark.motion_finished_at.unwrap())
                    .as_secs_f64()
                    * 1e3,
                foreground_idle_frames: benchmark.foreground_idle_frames,
                full_idle_frames: benchmark.full_idle_frames,
                foreground_missing: foreground.iter().map(|lane| lane.missing).sum(),
                foreground_unpresented: foreground
                    .iter()
                    .map(|lane| lane.desired - lane.presented)
                    .sum(),
                foreground_nonopaque: foreground
                    .iter()
                    .map(|lane| lane.presented - lane.opaque_presented)
                    .sum(),
                desired_missing,
                wanted_presented_delta,
                selection_ready,
                selection_settled,
                selected_topology_changes: self.telemetry.totals.wanted_topology_changes,
                work_pending_after_cut_ready: visible_idle.then_some(!fully_idle),
            };
            if benchmark.convergence.observe(observation) {
                let cells = convergence_cell_samples(
                    &self.wanted,
                    presentation,
                    |cell| self.index.contains(cell),
                    |cell| self.transitions.phase(cell),
                );
                self.replay_benchmark
                    .as_mut()
                    .unwrap()
                    .convergence
                    .record_sample(observation, cells);
            }
        }

        if let Some(drone_timing) = drone_timing {
            terminal_reason = self.observe_drone_benchmark(
                desired_missing,
                selection_ready,
                selection_settled,
                fully_idle,
                drone_timing,
            );
        }
        if let Some(reason) = terminal_reason {
            self.finish_replay_benchmark(
                reason,
                desired_missing,
                wanted_presented_delta,
                acquisition,
            );
        }
    }

    fn observe_drone_benchmark(
        &mut self,
        desired_missing: usize,
        selection_ready: bool,
        selection_settled: bool,
        fully_idle: bool,
        timing: (
            f64,
            f64,
            f64,
            bool,
            crate::benchmark_drone::FlightRenderStats,
        ),
    ) -> Option<&'static str> {
        use crate::benchmark_drone::{Directive, FlightFrame};
        let benchmark = self.replay_benchmark.as_ref().unwrap();
        let foreground = [benchmark.lane_counts[0], benchmark.lane_counts[3]];
        let player = self.player.as_ref().filter(|_| self.mode == Mode::Play);
        let frame = FlightFrame {
            frame: self.frame_no,
            after_start_ms: benchmark.started_at.elapsed().as_secs_f64() * 1000.0,
            player_dt_s: timing.0,
            interval_ms: timing.1,
            main_ms: timing.2,
            surface_presented: timing.3,
            render: timing.4,
            search_arrived: self
                .search_benchmark
                .as_ref()
                .is_some_and(SearchBenchmarkPlayer::arrived),
            selection_ready,
            selection_settled,
            fully_idle,
            foreground_missing: foreground.iter().map(|lane| lane.missing).sum(),
            foreground_unpresented: foreground
                .iter()
                .map(|lane| lane.desired - lane.presented)
                .sum(),
            foreground_nonopaque: foreground
                .iter()
                .map(|lane| lane.presented - lane.opaque_presented)
                .sum(),
            guard_missing: benchmark.lane_counts[1].missing,
            background_missing: benchmark.lane_counts[2].missing,
            desired_missing,
            destination_prefetch_cells: self.dest_prefetch.len(),
            predicted_prefetch_cells: self.predicted_prefetch.len(),
            player_position_ecef: player.map(|p| p.pos.to_array()),
            player_velocity_ecef: player.map(|p| p.vel.to_array()),
            player_speed_ms: player.map_or(0.0, Player::speed_ms),
            player_airspeed_ms: player.map_or(0.0, Player::airspeed_ms),
            player_camera_heading_rad: player.map(|p| p.cam_heading),
            camera: crate::replay::observed_pose(self.frame_no, &self.camera),
        };
        match self.drone_benchmark.as_mut().unwrap().observe(frame) {
            Some(Directive::EnterPlay) => {
                // The same entry point as P: terrain floor, selected aircraft,
                // chase camera and asynchronous selection keep their normal lifecycle.
                self.enter_play();
                self.search_benchmark
                    .as_mut()
                    .unwrap()
                    .reset_selection_fence();
            }
            Some(Directive::ResetSelectionFence) => {
                self.search_benchmark
                    .as_mut()
                    .unwrap()
                    .reset_selection_fence();
                self.lod_selector.invalidate();
                self.wanted_at = None;
            }
            Some(Directive::ExitPlay) => {
                self.exit_play();
                self.search_benchmark
                    .as_mut()
                    .unwrap()
                    .reset_selection_fence();
            }
            Some(Directive::Finish) => {
                self.play_keys = PlayKeys::default();
                self.keys = Keys::default();
                return Some("drone_flight_settled");
            }
            None => {}
        }
        None
    }

    fn replay_measurement_work(
        &self,
    ) -> (
        crate::replay_measurement::WorkState,
        crate::replay_measurement::WorkCounters,
    ) {
        let counts = &self.replay_benchmark.as_ref().unwrap().lane_counts;
        let (queued_cells, running_cells) = self.pool.counts();
        let (started_cell_jobs, generated_cache_hits) = self.pool.stats();
        let (network_requests, network_bytes, network_errors) = vearth_data::metrics::snapshot();
        (
            crate::replay_measurement::WorkState {
                desired_cells: counts.iter().map(|c| c.desired).sum(),
                missing_desired: counts.iter().map(|c| c.missing).sum(),
                nonopaque_desired: counts
                    .iter()
                    .map(|c| c.desired.saturating_sub(c.opaque_presented))
                    .sum(),
                active_transitions: self.transitions.stats().active,
                queued_cells,
                running_cells,
                ready_cells: self.ready_meshes.len(),
            },
            crate::replay_measurement::WorkCounters {
                started_cell_jobs,
                generated_cache_hits,
                network_requests,
                network_bytes,
                network_errors,
            },
        )
    }

    fn finish_replay_benchmark(
        &mut self,
        terminal_reason: &str,
        desired_missing: usize,
        wanted_presented_delta: usize,
        acquisition: Vec<vearth_data::metrics::AcquisitionSnapshot>,
    ) {
        let current_report = self.telemetry.report();
        let (
            options,
            started_at,
            motion_finished_at,
            motion_report,
            visible_converged_after_ms,
            full_idle_after_ms,
            last_triangles,
        ) = {
            let benchmark = self.replay_benchmark.as_mut().unwrap();
            if benchmark.report_written {
                return;
            }
            benchmark.report_written = true;
            let motion_finished_at = benchmark.motion_finished_at.unwrap_or_else(Instant::now);
            (
                benchmark.options.clone(),
                benchmark.started_at,
                motion_finished_at,
                benchmark.motion_report.unwrap_or(current_report),
                benchmark.visible_converged_after_ms,
                (terminal_reason == "settled")
                    .then_some(benchmark.full_idle_started_at)
                    .flatten()
                    .map(|started| started.duration_since(motion_finished_at).as_secs_f64() * 1e3),
                benchmark.last_triangles,
            )
        };

        let final_report = self.telemetry.report();
        let benchmark = self.replay_benchmark.as_ref().unwrap();
        let foreground_converged_after_ms = benchmark.foreground_converged_after_ms;
        let lane_diagnostics = crate::streaming::view_lane_diagnostics(
            &self.wanted,
            self.camera.eye_ecef(),
            &LodView::new(
                self.camera.view_rotation(),
                self.camera.fov_y,
                self.camera.aspect,
            ),
            &|cell| self.index.metric(cell),
        );
        let wanted_by_lane_level: std::collections::BTreeMap<_, _> = REPLAY_LANES
            .into_iter()
            .zip(lane_diagnostics.wanted_by_lane_level)
            .collect();
        let lane_reclassification: std::collections::BTreeMap<_, _> = REPLAY_LANES
            .into_iter()
            .zip(lane_diagnostics.global_to_known_lane)
            .map(|(from, row)| {
                (
                    from,
                    REPLAY_LANES
                        .into_iter()
                        .zip(row)
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
            })
            .collect();
        let view_lanes: std::collections::BTreeMap<_, _> = REPLAY_LANES
            .into_iter()
            .zip(benchmark.lane_counts.iter().zip(&benchmark.lane_series))
            .map(|(name, (counts, series))| {
                (
                    name,
                    serde_json::json!({
                        "desired": counts.desired,
                        "missing_desired": counts.missing,
                        "presented_desired": counts.presented,
                        "opaque_presented_desired": counts.opaque_presented,
                        "missing_desired_samples": distribution_json(series.missing.summary()),
                        "presented_desired_samples": distribution_json(series.presented.summary()),
                    }),
                )
            })
            .collect();
        let (generated_cells, generated_cache_hits) = self.pool.stats();
        let generation_stages = vearth_gen::contracts::generation_timing_snapshot();
        let generation_phases: std::collections::BTreeMap<_, _> =
            vearth_gen::contracts::GENERATION_PHASES
                .into_iter()
                .enumerate()
                .map(|(phase, name)| {
                    (
                        name,
                        serde_json::json!({
                            "ns_by_level": generation_stages.phase_ns_by_level[phase],
                            "count_by_level": generation_stages.phase_counts_by_level[phase],
                        }),
                    )
                })
                .collect();
        let roof_diagnostics = roof_diagnostics_json(&generation_stages);
        let generation_incomplete: std::collections::BTreeMap<_, _> =
            vearth_gen::contracts::GENERATION_INCOMPLETE_STAGES
                .into_iter()
                .zip(generation_stages.incomplete_by_stage)
                .collect();
        let building_failures: std::collections::BTreeMap<_, _> =
            vearth_gen::contracts::BUILDING_FAILURE_REASONS
                .into_iter()
                .zip(generation_stages.building_failures_by_reason)
                .collect();
        let source_completeness = vearth_data::metrics::source_completeness_snapshot();
        let acquisition_by_stage: std::collections::BTreeMap<_, _> =
            vearth_data::metrics::ACQUISITION_COMPLETENESS_STAGES
                .into_iter()
                .zip(source_completeness.acquisition_by_stage)
                .collect();
        let required_query_caps: std::collections::BTreeMap<_, _> =
            vearth_data::metrics::REQUIRED_QUERY_CLASSES
                .into_iter()
                .zip(source_completeness.required_query_caps)
                .collect();
        let (network_requests, network_bytes, network_errors) = vearth_data::metrics::snapshot();
        let stages = self.pool.stage_counts();
        let chunk_cache = self
            .pool
            .cache_stats()
            .map_or(serde_json::Value::Null, |cache| {
                serde_json::json!({
                    "ram_hits": cache.ram_hits,
                    "disk_hits": cache.disk_hits,
                    "misses": cache.misses,
                    "resident_entries": cache.resident_entries,
                    "resident_bytes": cache.resident_bytes,
                    "resident_flora_cells": cache.resident_flora_cells,
                    "resident_flora_bytes": cache.resident_flora_bytes,
                    "budget_bytes": cache.budget_bytes,
                    "indexed_hits": cache.indexed_hits,
                    "legacy_hits": cache.legacy_hits,
                    "legacy_imports": cache.legacy_imports,
                    "writes": cache.writes,
                    "evictions": cache.evictions,
                    "corruptions": cache.corruptions,
                    "write_errors": cache.write_errors,
                    "dropped_writes": cache.dropped_writes,
                    "disk_entries": cache.disk_entries,
                    "logical_disk_bytes": cache.logical_disk_bytes,
                    "physical_disk_bytes": cache.physical_disk_bytes,
                    "disk_budget_bytes": cache.disk_budget_bytes,
                    "pinned_entries": cache.pinned_entries,
                    "pinned_bytes": cache.pinned_bytes,
                    "queued_writes": cache.queued_writes,
                })
            });
        let acquisition: Vec<_> = acquisition
            .into_iter()
            .map(|series| {
                serde_json::json!({
                    "name": series.name,
                    "cache_hits": series.cache_hits,
                    "cache_misses": series.cache_misses,
                    "cache_bytes": series.cache_bytes,
                    "shared_waits": series.shared_waits,
                    "shared_wait_us": series.shared_wait_us,
                    "shared_wait_us_max": series.shared_wait_us_max,
                    "queued": series.queued,
                    "queued_peak": series.queued_peak,
                    "queue_us": series.queue_us,
                    "queue_us_max": series.queue_us_max,
                    "in_flight": series.in_flight,
                    "in_flight_peak": series.in_flight_peak,
                    "successes": series.successes,
                    "not_found": series.not_found,
                    "errors": series.errors,
                    "canceled": series.canceled,
                    "bytes": series.bytes,
                    "latency_us": series.latency_us,
                    "latency_us_max": series.latency_us_max,
                })
            })
            .collect();
        #[cfg(feature = "async-providers")]
        let provider_mode = if self.pool.async_acquisition() {
            "async"
        } else {
            "legacy"
        };
        #[cfg(not(feature = "async-providers"))]
        let provider_mode = "legacy";
        let resident_gpu_bytes = self.index.gpu_bytes();
        let (weather_tick0, weather_tick1, weather_alpha) =
            WeatherModel::tick_pair(self.solar.utc_seconds());
        let report = serde_json::json!({
            "schema_version": 1,
            "gpu_timing_scope": {
                "gpu_ms": "sun_shadow_begin_to_tonemap_end",
                "gpu_world_ms": "sky_preparation_and_combined_world_procedural_flora",
                "gpu_flora_ms": "unavailable_separately_included_in_gpu_world_ms",
            },
            "flora_control": "procedural_gpu_flower_lod_coupling",
            "trace_path": options.trace_path,
            "search": self.search_benchmark.as_ref().map(|search| search.report(Some(crate::replay::observed_pose(self.frame_no, &self.camera)))),
            "drone_flight": self.drone_benchmark.as_ref().map(crate::benchmark_drone::DroneBenchmark::report),
            "provider_mode": provider_mode,
            "pipeline": if self.pool.staged_pipeline() { "staged" } else { "legacy" },
            "terminal_reason": terminal_reason,
            "weather": {
                "climate_atlas_version": self.weather.atlas_version(),
                "weather_model_version": self.weather.model_version(),
                "mode": self.weather.config.mode.to_string(),
                "quality": format!("{:?}", self.weather.config.quality).to_ascii_lowercase(),
                "wind_influence": format!("{:?}", self.weather.config.wind_influence).to_ascii_lowercase(),
                "seed": self.weather.config.seed,
                "preset": self.weather.config.preset,
                "utc_seconds": self.solar.utc_seconds(),
                "tick0": weather_tick0,
                "tick1": weather_tick1,
                "tick_alpha": weather_alpha,
            },
            "benchmark": {
                "max_settle_seconds": options.max_settle.as_secs_f64(),
                "idle_frames": options.idle_frames,
                "debug_panel_visible": self.ui.panel_visibility().1,
            },
            "surface": {
                "width": self.gfx.config.width,
                "height": self.gfx.config.height,
                "present_mode": format!("{:?}", self.gfx.config.present_mode),
            },
            "frame_pacing": {
                "target_hz": 60.0,
                "target_interval_ms": REPLAY_BENCHMARK_FRAME_INTERVAL.as_secs_f64() * 1e3,
            },
            "timing": {
                "trace_wall_ms": motion_finished_at.duration_since(started_at).as_secs_f64() * 1e3,
                "settle_wall_ms": motion_finished_at.elapsed().as_secs_f64() * 1e3,
                "total_wall_ms": started_at.elapsed().as_secs_f64() * 1e3,
                "visible_converged_after_trace_ms": visible_converged_after_ms,
                "foreground_opaque_converged_after_trace_ms": foreground_converged_after_ms,
                "full_idle_after_trace_ms": full_idle_after_ms,
            },
            "motion": smoothness_json(motion_report),
            "final": smoothness_json(final_report),
            "measurement": benchmark.measurement.as_ref().map(|m| {
                m.report(measurement_smoothness_json, measurement_generation_json)
            }),
            "view_lanes": view_lanes,
            "convergence_diagnostics": {
                "record_limit": CONVERGENCE_RECORD_LIMIT,
                "cell_sample_limit": CONVERGENCE_CELL_LIMIT,
                "cell_address_order": ["face", "level", "i", "j"],
                "sample_interval_ms": CONVERGENCE_SAMPLE_INTERVAL_MS,
                "state": benchmark.convergence,
            },
            "selector": {
                "visibility_policy": "global_0_to_9000m",
                "ruggedness_policy": crate::streaming::ruggedness_policy().label(),
                "wanted_by_lane_level": wanted_by_lane_level,
                "metric_provenance": {
                    "cell": lane_diagnostics.metric_provenance[0],
                    "ancestor": lane_diagnostics.metric_provenance[1],
                    "unknown_or_invalid": lane_diagnostics.metric_provenance[2],
                },
                "global_to_known_lane_at_terminal": lane_reclassification,
            },
            "generation": {
                "cells": generated_cells,
                "chunk_cache_hits": generated_cache_hits,
                "stages": {
                    "samples": generation_stages.samples,
                    "generation_ns": generation_stages.generation_ns,
                    "collision_build_ns": generation_stages.collision_build_ns,
                    "incomplete_by_stage": generation_incomplete,
                    "building_failures_by_reason": building_failures,
                    "phases": generation_phases,
                },
                "roofs": roof_diagnostics,
            },
            "source_completeness": {
                "acquisition_by_stage": acquisition_by_stage,
                "required_query_caps_by_zoom": required_query_caps,
            },
            "network": {
                "requests": network_requests,
                "bytes": network_bytes,
                "errors": network_errors,
            },
            "acquisition": acquisition,
            "chunk_cache": chunk_cache,
            "terminal": {
                "desired_missing": desired_missing,
                "wanted_presented_delta": wanted_presented_delta,
                "pool_in_flight": self.pool.in_flight_count(),
                "ready_cells": self.ready_meshes.len(),
                "ready_bytes": self.ready_meshes.bytes(),
                "stage_cache_check": stages.cache_check,
                "stage_acquiring": stages.acquiring,
                "stage_cpu": stages.cpu,
                "resident_cells": self.index.len(),
                "resident_gpu_bytes": resident_gpu_bytes,
                "arena_capacity_bytes": self.gfx.renderer.mesh_stats().arena_capacity_bytes,
                "world_triangles": last_triangles[0],
                "tree_triangles": last_triangles[1],
                "flora_triangles": last_triangles[2],
                "resident_flora_cells": self.gfx.renderer.mesh_stats().flora_cells,
                "resident_flora_descriptor_bytes": self
                    .gfx
                    .renderer
                    .mesh_stats()
                    .flora_descriptor_bytes,
            },
        });

        if let Some(parent) = options
            .report_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            log::error!(
                "could not create replay report directory {}: {error}",
                parent.display()
            );
        }
        match serde_json::to_vec_pretty(&report)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&options.report_path, bytes))
        {
            Ok(()) => log::info!(
                "replay-report {} mode={} terminal={} visible-converged-ms={:?}",
                options.report_path.display(),
                provider_mode,
                terminal_reason,
                visible_converged_after_ms,
            ),
            Err(error) => log::error!(
                "could not write replay report {}: {error}",
                options.report_path.display()
            ),
        }
        self.replay_exit_requested = true;
    }
}

pub struct App {
    state: Option<State>,
    boot: Option<BootState>,
    window: Option<Arc<Window>>,
    occluded: bool,
    zero_sized: bool,
    last_redraw: Option<Instant>,
    launched_at: Instant,
    start: StartPos,
    replay: Option<ReplayTrace>,
    search_benchmark: Option<SearchBenchmarkSpec>,
    replay_benchmark: Option<ReplayBenchmarkOptions>,
    calibration_landmark: Option<&'static vearth_gen::landmarks::Landmark>,
    runtime_paths: Option<RuntimePaths>,
    preferences_store: Option<PreferencesStore>,
    preferences: UserPreferences,
    /// Set when the application asks its host to exit (window closed,
    /// replay or benchmark finished).
    exit_requested: bool,
    #[cfg(target_os = "macos")]
    native_menu: Option<NativeMenu>,
}

struct BootState {
    gfx: Gfx,
    ui: LaunchUi,
    window: Arc<Window>,
    prepared_world: Receiver<Result<PreparedWorld, String>>,
    started_at: Instant,
    splash_started_at: Instant,
    failure: Option<String>,
    presented: bool,
}

impl BootState {
    fn render(&mut self) -> Result<(), crate::gfx::SurfaceFrameError> {
        let target = self.gfx.begin_launch_frame(self.failure.is_some())?;
        let frame = self.ui.run(
            &self.window,
            self.splash_started_at.elapsed(),
            self.failure.as_deref(),
        );
        self.ui.paint(
            &self.gfx.renderer.device,
            &self.gfx.renderer.queue,
            &target.view,
            self.gfx.config.width,
            self.gfx.config.height,
            frame,
        );
        target.present(&self.gfx.renderer.queue);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn native_menu_state(state: &State) -> NativeMenuState {
    NativeMenuState::new(
        state.ui.panel_visibility(),
        state.ui.panels_hidden(),
        state.ui.settings_button_visible(),
        state.local_time.twenty_four_hour(),
        state.gfx.renderer.clouds_visible(),
        state.gfx.renderer.precipitation_visible(),
        state.gfx.renderer.building_night_lights_enabled(),
        state.gfx.renderer.shadows(),
        state.gfx.renderer.post_process_settings(),
        state.weather_enabled(),
        state.pan_speed,
        state.pool.provider_controls(),
    )
}

impl App {
    pub fn new(start: StartPos) -> Self {
        App {
            state: None,
            boot: None,
            window: None,
            occluded: false,
            zero_sized: false,
            last_redraw: None,
            launched_at: Instant::now(),
            start,
            replay: None,
            search_benchmark: None,
            replay_benchmark: None,
            calibration_landmark: None,
            runtime_paths: None,
            preferences_store: None,
            preferences: UserPreferences::default(),
            exit_requested: false,
            #[cfg(target_os = "macos")]
            native_menu: None,
        }
    }

    pub fn with_replay(
        start: StartPos,
        replay: ReplayTrace,
        replay_benchmark: Option<ReplayBenchmarkOptions>,
    ) -> Self {
        if replay_benchmark.is_some() {
            // Benchmark construction precedes provider and worker startup.
            vearth_gen::contracts::reset_generation_timing();
            vearth_gen::contracts::set_generation_timing_enabled(true);
            vearth_data::metrics::reset_source_completeness();
            vearth_data::metrics::set_source_completeness_enabled(true);
        }
        App {
            state: None,
            boot: None,
            window: None,
            occluded: false,
            zero_sized: false,
            last_redraw: None,
            launched_at: Instant::now(),
            start,
            replay: Some(replay),
            search_benchmark: None,
            replay_benchmark,
            calibration_landmark: None,
            runtime_paths: None,
            preferences_store: None,
            preferences: UserPreferences::default(),
            exit_requested: false,
            #[cfg(target_os = "macos")]
            native_menu: None,
        }
    }

    pub fn with_search_benchmark(
        mut start: StartPos,
        search: SearchBenchmarkSpec,
        benchmark: ReplayBenchmarkOptions,
    ) -> Self {
        vearth_gen::contracts::reset_generation_timing();
        vearth_gen::contracts::set_generation_timing_enabled(true);
        vearth_data::metrics::reset_source_completeness();
        vearth_data::metrics::set_source_completeness_enabled(true);
        start.lat_deg = 0.0;
        start.lon_deg = 0.0;
        start.heading_deg = 0.0;
        start.pitch_deg = 0.0;
        start.play = false;
        let mut app = Self::new(start);
        app.search_benchmark = Some(search);
        app.replay_benchmark = Some(benchmark);
        app
    }

    fn advance_boot(&mut self) {
        if let Some(boot) = self.boot.as_mut()
            && (boot.failure.is_some() || !boot.presented)
        {
            // Present the launch screen at least once: building the
            // interactive state blocks this thread, and a surface with no
            // presented frame shows nothing meanwhile.
            boot.presented |= boot.render().is_ok();
            if boot.failure.is_some() || boot.presented {
                return;
            }
        }

        let result = {
            let Some(boot) = self.boot.as_mut() else {
                return;
            };
            match boot.prepared_world.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => {
                    let _ = boot.render();
                    None
                }
                Err(TryRecvError::Disconnected) => Some(Err(
                    "the startup worker stopped before initialization completed".into(),
                )),
            }
        };
        let Some(result) = result else {
            return;
        };
        let prepared_world = match result {
            Ok(prepared) => prepared,
            Err(error) => {
                log::error!("startup failed: {error}");
                if let Some(boot) = self.boot.as_mut() {
                    boot.failure = Some(error);
                    let _ = boot.render();
                }
                if let Some(window) = &self.window {
                    window.set_title(WINDOW_TITLE);
                }
                return;
            }
        };

        let boot = self.boot.take().expect("boot state exists");
        let elapsed = boot.started_at.elapsed();
        let window = self.window.as_ref().expect("window exists").clone();
        let state = State::new(
            window.clone(),
            boot.gfx,
            self.start,
            self.replay.clone(),
            self.search_benchmark.clone(),
            self.replay_benchmark.clone(),
            self.calibration_landmark,
            prepared_world,
            self.runtime_paths
                .clone()
                .expect("runtime paths discovered before startup"),
            self.preferences_store.take(),
            self.preferences.clone(),
        );
        window.set_title(WINDOW_TITLE);
        log::info!(
            "launch event: interactive-state-ready-ms={:.1}",
            elapsed.as_secs_f64() * 1e3
        );
        #[cfg(target_os = "macos")]
        match NativeMenu::new(native_menu_state(&state)) {
            Ok(menu) => self.native_menu = Some(menu),
            Err(error) => log::error!("could not create native menu bar: {error}"),
        }
        self.state = Some(state);
    }

    #[cfg(target_os = "macos")]
    fn handle_native_menu_actions(&mut self) {
        let actions = self
            .native_menu
            .as_ref()
            .map(NativeMenu::drain_actions)
            .unwrap_or_default();
        for action in actions {
            let Some(state) = self.state.as_mut() else {
                continue;
            };
            match action {
                NativeMenuAction::OpenSettings => state.ui.open_settings(),
                NativeMenuAction::OpenPrivacy => state.ui.open_privacy(),
                NativeMenuAction::ShowSettingsButton(visible) => {
                    state.ui.set_settings_button_visible(visible)
                }
                NativeMenuAction::Paste => state.ui.paste_into_search(),
                NativeMenuAction::HidePanels(hidden) => state.ui.set_panels_hidden(hidden),
                NativeMenuAction::ShowSearch(visible) => state.ui.set_search_visible(visible),
                NativeMenuAction::ShowDebug(visible) => state.ui.set_debug_visible(visible),
                NativeMenuAction::ShowSolar(visible) => state.ui.set_solar_visible(visible),
                NativeMenuAction::SetDate => state.ui.open_date_editor(state.solar.utc_seconds()),
                NativeMenuAction::SetTwentyFourHour(enabled) => {
                    state.local_time.set_twenty_four_hour(enabled)
                }
                NativeMenuAction::SetCloudsVisible(visible) => {
                    state.gfx.renderer.set_clouds_visible(visible)
                }
                NativeMenuAction::SetPrecipitationVisible(visible) => {
                    state.gfx.renderer.set_precipitation_visible(visible)
                }
                NativeMenuAction::SetBuildingNightLights(enabled) => state
                    .gfx
                    .renderer
                    .set_building_night_lights_enabled(enabled),
                NativeMenuAction::SetColorGrading(enabled) => {
                    state.gfx.renderer.set_color_grading_enabled(enabled)
                }
                NativeMenuAction::SetShadows(shadows) => state.gfx.renderer.set_shadows(shadows),
                NativeMenuAction::SetFilmicSaturation(enabled) => {
                    state.gfx.renderer.set_filmic_saturation_enabled(enabled)
                }
                NativeMenuAction::SetWeatherEnabled(enabled) => state.set_weather_enabled(enabled),
                NativeMenuAction::ShowCompass(visible) => state.ui.set_compass_visible(visible),
                NativeMenuAction::ShowLoading(visible) => state.ui.set_loading_visible(visible),
                NativeMenuAction::ShowAttribution(visible) => {
                    state.ui.set_attribution_visible(visible)
                }
                NativeMenuAction::SetPanSpeed(preset) => state.pan_speed = preset,
                NativeMenuAction::SetProviderEnabled { id, enabled } => {
                    state.set_provider_enabled(&id, enabled)
                }
            }
        }
    }
}

impl App {
    /// Starts the launch work that needs no GPU: runtime paths, preferences
    /// and the startup worker that prepares the world. Returns the receiver
    /// for [`Self::finish_launch`].
    fn begin_launch(&mut self, window: Arc<Window>) -> Receiver<Result<PreparedWorld, String>> {
        self.window = Some(window);
        let runtime_paths = RuntimePaths::discover();
        match crate::settings::apply_pending_cache_clear(
            &runtime_paths.cache,
            &runtime_paths.application_support,
        ) {
            Ok(true) => log::info!("storage: applied scheduled downloaded-data removal"),
            Ok(false) => {}
            Err(error) => log::error!("storage: could not clear downloaded data: {error}"),
        }
        if let Err(error) = runtime_paths.ensure_directories() {
            log::error!("could not create runtime directories: {error}");
        }
        let preferences_enabled = self.replay.is_none()
            && self.search_benchmark.is_none()
            && self.calibration_landmark.is_none();
        let preferences = if preferences_enabled {
            let (store, preferences) = PreferencesStore::load(&runtime_paths.application_support);
            self.preferences_store = Some(store);
            preferences
        } else {
            UserPreferences {
                show_debug: self.replay_benchmark.is_some()
                    && vearth_platform::env::var("VEARTH_BENCHMARK_DEBUG_PANEL")
                        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "on")),
                ..UserPreferences::default()
            }
        };
        // Before the surface is first configured, so it starts at this size.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(window) = &self.window {
            window.set_max_pixel_ratio(preferences.render_resolution.max_pixel_ratio());
        }
        self.preferences = preferences.clone();
        self.runtime_paths = Some(runtime_paths.clone());
        log::info!(
            "runtime paths: mode={} cache={} photos={}",
            if runtime_paths.packaged {
                "packaged"
            } else {
                "development"
            },
            runtime_paths.cache.display(),
            runtime_paths.photos.display(),
        );
        let surface_month = crate::solar::utc_month(
            SolarClock::new(self.start.solar_start, self.start.time_scale).utc_seconds(),
        );
        let cache_root = runtime_paths.cache.clone();
        let calibration_mode = self.calibration_landmark.is_some();
        let startup_preferences = preferences;
        let (tx, rx) = std::sync::mpsc::channel();
        vearth_platform::thread::Builder::new()
            .name("vearth-startup".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    prepare_world(
                        calibration_mode,
                        surface_month,
                        &cache_root,
                        &startup_preferences,
                    )
                }))
                .map_err(|payload| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|message| (*message).to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "startup worker panicked".into())
                });
                let _ = tx.send(result);
            })
            .expect("startup worker");
        rx
    }

    /// Completes launch once the GPU device exists: shows the launch screen
    /// while the startup worker finishes.
    fn finish_launch(&mut self, gfx: Gfx, prepared_world: Receiver<Result<PreparedWorld, String>>) {
        let window = self.window.clone().expect("window exists before launch");
        log::info!(
            "launch event: gpu-ready-ms={:.1}",
            self.launched_at.elapsed().as_secs_f64() * 1e3
        );
        let launch_ui = LaunchUi::new(&window, &gfx.renderer.device, gfx.view_format());
        self.boot = Some(BootState {
            gfx,
            ui: launch_ui,
            window: window.clone(),
            prepared_world,
            started_at: self.launched_at,
            splash_started_at: Instant::now(),
            failure: None,
            presented: false,
        });
        window.set_title(WINDOW_TITLE);
        window.request_redraw();
    }

    /// Handles one window event. `ui_event` is the host's form of the same
    /// event for egui, which also receives events the application ignores.
    fn handle_window_event(&mut self, event: WindowEvent, ui_event: &UiEvent) {
        match &event {
            WindowEvent::Occluded(occluded) => {
                self.occluded = *occluded;
                if let Some(state) = self.state.as_mut() {
                    state.visible_since = (!occluded).then(Instant::now);
                }
                if !occluded {
                    self.last_redraw = None;
                    if let Some(state) = self.state.as_mut() {
                        state.surface_occluded = false;
                        state.last_frame = None;
                    }
                }
            }
            WindowEvent::Resized(size) => {
                self.zero_sized = size.width == 0 || size.height == 0;
                // A minimized window has no new camera viewport. Retain its
                // last useful dimensions so background loading cannot select
                // a 1px terrain cut and evict the view needed on restoration.
                if self.zero_sized {
                    return;
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                if self.next_redraw(now) > now {
                    return;
                }
                self.last_redraw = Some(now);
            }
            _ => {}
        }
        if self.state.is_none() {
            match event {
                WindowEvent::CloseRequested => self.exit_requested = true,
                WindowEvent::Resized(size) => {
                    if let Some(boot) = self.boot.as_mut() {
                        boot.gfx.resize(size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => self.advance_boot(),
                _ => {}
            }
            return;
        }
        let Some(state) = self.state.as_mut() else {
            return;
        };
        // Cursor visibility is app-level rather than UI-level. Reveal it
        // before egui sees movement so controls are immediately discoverable.
        match &event {
            WindowEvent::CursorMoved { .. } => {
                state.cursor_idle.moved(Instant::now());
                state.window.set_cursor_visible(true);
            }
            WindowEvent::CursorEntered { .. } => {
                state.cursor_idle.entered(Instant::now());
                state.window.set_cursor_visible(true);
            }
            WindowEvent::CursorLeft { .. } => {
                state.cursor_idle.left();
                state.window.set_cursor_visible(true);
            }
            WindowEvent::Focused(focused) => {
                state.cursor_idle.focused(*focused, Instant::now());
                state.window.set_cursor_visible(true);
            }
            _ => {}
        }
        // Handle Escape before egui so a focused field cannot swallow the
        // toggle or receive the same press when the legend closes.
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
        {
            if event.state == ElementState::Pressed && !event.repeat {
                state.ui.toggle_controls();
                state.keys = Keys::default();
                state.play_keys = PlayKeys::default();
                state.mouse.left = false;
                state.mouse.tilt_anchor = None;
                state.window.request_redraw();
            }
            return;
        }
        // The overlay UI sees events first; when it consumes them (typing a
        // search query, clicking a result) they must not steer the camera.
        let ui_consumed = state.ui.on_event(&state.window, ui_event);
        match event {
            WindowEvent::CloseRequested => self.exit_requested = true,
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                let map_toggle = matches!(
                    event.physical_key,
                    PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter)
                );
                if state.ui.controls_visible() || state.ui.map_visible() {
                    state.keys = Keys::default();
                    state.play_keys = PlayKeys::default();
                    state.mouse.left = false;
                    state.mouse.tilt_anchor = None;
                    if !state.ui.controls_visible() && down && !event.repeat && map_toggle {
                        state.ui.close_map();
                    }
                    return;
                }
                if ui_consumed {
                    // Keyboard went to the UI: release any held nav keys so
                    // focusing the search box mid-flight doesn't keep panning.
                    state.keys = Keys::default();
                    state.play_keys = PlayKeys::default();
                    return;
                }
                if state.photo_job.is_some() {
                    state.keys = Keys::default();
                    state.play_keys = PlayKeys::default();
                    return;
                }
                if down && event.physical_key == PhysicalKey::Code(KeyCode::Slash) {
                    state.ui.focus_search();
                    return;
                }
                if event.physical_key == PhysicalKey::Code(KeyCode::KeyT) {
                    if down && !event.repeat {
                        let pos = state
                            .player
                            .as_ref()
                            .filter(|_| state.mode == Mode::Play)
                            .map_or_else(|| state.camera.eye_ecef(), |player| player.pos);
                        state.location_tracker.show_title(pos, Instant::now());
                        state.window.request_redraw();
                    }
                    return;
                }
                if down && !event.repeat && map_toggle {
                    state.keys = Keys::default();
                    state.play_keys = PlayKeys::default();
                    state.mouse.left = false;
                    state.mouse.tilt_anchor = None;
                    state.cancel_flyto();
                    let (lat_deg, lon_deg, _) = state.map_pose();
                    state
                        .ui
                        .open_map(lat_deg, lon_deg, state.mode == Mode::Play);
                    return;
                }
                if down {
                    state.cancel_flyto();
                }
                if down
                    && !event.repeat
                    && event.physical_key == PhysicalKey::Code(KeyCode::KeyL)
                    && state.ui.panel_visibility().1
                {
                    state.toggle_ruggedness_policy();
                    return;
                }
                if down && event.physical_key == PhysicalKey::Code(KeyCode::BracketLeft) {
                    state.solar.shift_hours(-1.0);
                    return;
                }
                if down && event.physical_key == PhysicalKey::Code(KeyCode::BracketRight) {
                    state.solar.shift_hours(1.0);
                    return;
                }
                if down && !event.repeat && event.physical_key == PhysicalKey::Code(KeyCode::KeyP) {
                    match state.mode {
                        Mode::Explore => state.enter_play(),
                        Mode::Play => state.exit_play(),
                    }
                    return;
                }
                if state.mode == Mode::Play {
                    if down
                        && !event.repeat
                        && let Some(vehicle) = direct_vehicle_key(&event.physical_key)
                    {
                        if let Some(player) = state.player.as_mut()
                            && player.vehicle != vehicle
                        {
                            player.select_vehicle(vehicle);
                            state.selected_vehicle = player.vehicle;
                        }
                        return;
                    }
                    if down
                        && !event.repeat
                        && event.physical_key == PhysicalKey::Code(KeyCode::KeyV)
                    {
                        let selected = state.player.as_ref().map(|player| player.vehicle.next());
                        if let (Some(player), Some(vehicle)) = (state.player.as_mut(), selected) {
                            player.select_vehicle(vehicle);
                            state.selected_vehicle = player.vehicle;
                        }
                        // Keep the live key state across the swap. Held keys
                        // do not reliably emit another press event, so clearing
                        // this would leave the new vehicle hovering until W or
                        // Shift was released and pressed again.
                        return;
                    }
                    let k = &mut state.play_keys;
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::KeyW | KeyCode::ArrowUp) => k.fwd = down,
                        PhysicalKey::Code(KeyCode::KeyS | KeyCode::ArrowDown) => k.back = down,
                        PhysicalKey::Code(KeyCode::KeyD | KeyCode::ArrowRight) => k.right = down,
                        PhysicalKey::Code(KeyCode::KeyA | KeyCode::ArrowLeft) => k.left = down,
                        PhysicalKey::Code(KeyCode::Space) => k.up = down,
                        PhysicalKey::Code(KeyCode::KeyC) => k.down = down,
                        PhysicalKey::Code(KeyCode::ShiftLeft | KeyCode::ShiftRight) => {
                            k.boost = down
                        }
                        PhysicalKey::Code(KeyCode::KeyQ) => k.rot_left = down,
                        PhysicalKey::Code(KeyCode::KeyE) => k.rot_right = down,
                        PhysicalKey::Code(KeyCode::KeyR) => k.tilt_up = down,
                        PhysicalKey::Code(KeyCode::KeyF) => k.tilt_down = down,
                        _ => {}
                    }
                    return;
                }
                let k = &mut state.keys;
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::KeyW | KeyCode::ArrowUp) => k.north = down,
                    PhysicalKey::Code(KeyCode::KeyS | KeyCode::ArrowDown) => k.south = down,
                    PhysicalKey::Code(KeyCode::KeyD | KeyCode::ArrowRight) => k.east = down,
                    PhysicalKey::Code(KeyCode::KeyA | KeyCode::ArrowLeft) => k.west = down,
                    PhysicalKey::Code(KeyCode::Equal | KeyCode::NumpadAdd) => k.zoom_in = down,
                    PhysicalKey::Code(KeyCode::Minus | KeyCode::NumpadSubtract) => {
                        k.zoom_out = down
                    }
                    PhysicalKey::Code(KeyCode::KeyQ) => k.rot_left = down,
                    PhysicalKey::Code(KeyCode::KeyE) => k.rot_right = down,
                    PhysicalKey::Code(KeyCode::KeyR) => k.tilt_up = down,
                    PhysicalKey::Code(KeyCode::KeyF) => k.tilt_down = down,
                    _ => {}
                }
            }
            WindowEvent::Resized(size) => state.resize(size.width, size.height),
            WindowEvent::ModifiersChanged(m) => state.mods = m,
            WindowEvent::MouseInput {
                state: s, button, ..
            } => {
                if state.ui.controls_visible() || state.ui.map_visible() {
                    state.mouse.left = false;
                    state.mouse.tilt_anchor = None;
                    return;
                }
                if ui_consumed || state.photo_job.is_some() {
                    return;
                }
                if button == MouseButton::Left {
                    state.mouse.left = s == ElementState::Pressed;
                    if state.mouse.left {
                        state.cancel_flyto();
                    }
                    if !state.mouse.left {
                        state.mouse.tilt_anchor = None;
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if state.ui.controls_visible() || state.ui.map_visible() {
                    state.mouse.pos = Some((position.x, position.y));
                    return;
                }
                if state.photo_job.is_some() {
                    return;
                }
                let p = (position.x, position.y);
                if let Some(last) = state.mouse.pos {
                    let (dx, dy) = (p.0 - last.0, p.1 - last.1);
                    let (w, h) = (
                        state.gfx.config.width as f64,
                        state.gfx.config.height as f64,
                    );
                    if state.mode == Mode::Play {
                        if state.mouse.left
                            && let Some(pl) = state.player.as_mut()
                        {
                            pl.orbit(dx, dy);
                        }
                        state.mouse.pos = Some(p);
                        return;
                    }
                    if state.mouse.left {
                        if state.mods.super_key() || state.mods.control_key() {
                            // Cmd/Ctrl + drag: rigid orbit about the grabbed
                            // surface point (Maps style) — the point stays
                            // pinned to the pixel where it was grabbed.
                            if state.mouse.tilt_anchor.is_none() {
                                let ndc = (last.0 / w * 2.0 - 1.0, 1.0 - last.1 / h * 2.0);
                                state.mouse.tilt_anchor = state.ground_anchor(ndc, w / h);
                            }
                            match state.mouse.tilt_anchor {
                                Some(anchor) => state.camera.orbit_anchor(anchor, dx, dy),
                                None => state.camera.tilt(dx, dy),
                            }
                            // A deliberate horizontal rotation releases the
                            // north lock (Maps: the compass persists until
                            // the globe view re-norths it).
                            if dx.abs() > 0.5 {
                                state.north_locked = false;
                            }
                        } else {
                            state.mouse.tilt_anchor = None;
                            let ground_altitude_m = state
                                .index
                                .terrain_surface_at(state.camera.eye_ecef().normalize_or(DVec3::X))
                                .or_else(|| {
                                    state
                                        .index
                                        .terrain_top_under(
                                            state.camera.eye_ecef().normalize_or(DVec3::X),
                                        )
                                        .map(|height| height as f64)
                                })
                                .map(|height| height.max(0.0))
                                .unwrap_or(0.0);
                            state.camera.drag_over_ground(dx, dy, h, ground_altitude_m);
                        }
                    }
                }
                state.mouse.pos = Some(p);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if state.ui.controls_visible() || state.ui.map_visible() {
                    return;
                }
                if ui_consumed || state.photo_job.is_some() {
                    return;
                }
                state.cancel_flyto();
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 60.0,
                };
                if state.mode == Mode::Play {
                    if let Some(pl) = state.player.as_mut() {
                        pl.zoom_boom(amount);
                    }
                    return;
                }
                // Zoom along the ray to the surface point under the cursor:
                // the point stays glued to its pixel at any tilt.
                let (w, h) = (
                    state.gfx.config.width as f64,
                    state.gfx.config.height as f64,
                );
                let anchor = state
                    .mouse
                    .pos
                    .map(|(x, y)| (x / w * 2.0 - 1.0, 1.0 - y / h * 2.0))
                    .and_then(|ndc| state.ground_anchor(ndc, w / h.max(1.0)));
                state.camera.zoom_towards(amount, anchor);
            }
            WindowEvent::RedrawRequested => {
                state.frame();
                #[cfg(target_os = "macos")]
                if let Some(menu) = self.native_menu.as_mut() {
                    menu.apply_state(native_menu_state(state));
                }
                if state.replay_exit_requested {
                    self.exit_requested = true;
                }
            }
            _ => {}
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window({
                    let mut attributes = winit::window::Window::default_attributes()
                        .with_title(WINDOW_TITLE)
                        .with_inner_size(winit::dpi::LogicalSize::new(1280, 800));
                    if self.replay_benchmark.is_some() {
                        // Native frame measurements require a visible drawable;
                        // an obscured Metal window follows the 100 ms idle cadence.
                        attributes = attributes
                            .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
                            .with_active(true);
                    }
                    if let Some((width, height)) = self
                        .replay_benchmark
                        .as_ref()
                        .and_then(|benchmark| benchmark.surface_size)
                    {
                        attributes.with_inner_size(winit::dpi::PhysicalSize::new(width, height))
                    } else {
                        attributes
                    }
                })
                .expect("window"),
        );
        let window = Arc::new(Window::new(window));
        self.window = Some(window.clone());
        if self.replay_benchmark.is_some() {
            window.focus_window();
        }
        let prepared_world = self.begin_launch(window.clone());
        // Provider discovery and disk-cache setup are independent of the GPU.
        // Keep graphics on the event thread while the startup worker proceeds.
        let gfx = pollster::block_on(Gfx::new(&window)).expect("gpu init");
        self.finish_launch(gfx, prepared_world);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        match WindowEvent::from_winit(&event) {
            Some(app_event) => {
                let app_event = match &self.window {
                    Some(window) => window.to_render(app_event),
                    None => app_event,
                };
                self.handle_window_event(app_event, &event)
            }
            // egui still needs events the application ignores (IME, touch).
            None => {
                if let Some(state) = self.state.as_mut() {
                    state.ui.on_event(&state.window, &event);
                }
            }
        }
        if self.exit_requested {
            event_loop.exit();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.shutdown();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.after_events();
        if self.exit_requested {
            event_loop.exit();
            return;
        }
        if let Some(window) = &self.window {
            // Occluded surfaces have no successful presentation to pace the
            // event loop. Keep loading/replay timeouts progressing at a bounded
            // cadence, including when the backend omits an occlusion event.
            let now = Instant::now();
            let deadline = self.next_redraw(now);
            if deadline > now {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
                window.request_redraw();
            }
        }
    }
}

impl App {
    /// Stops background work before the host exits.
    fn shutdown(&mut self) {
        if let Some(state) = &self.state {
            state.pool.shutdown();
        }
    }

    /// Housekeeping after a batch of events, before the host waits for the
    /// next redraw: benchmark deadlines, menu actions and cursor policy.
    fn after_events(&mut self) {
        if self.state.is_none()
            && let Some(search) = &self.search_benchmark
            && let Some(benchmark) = &self.replay_benchmark
            && self.launched_at.elapsed() >= benchmark.max_settle
        {
            if let Err(error) = crate::replay::write_search_startup_timeout(
                &benchmark.report_path,
                search,
                self.launched_at.elapsed().as_secs_f64() * 1e3,
            ) {
                log::error!("could not write search startup timeout: {error}");
            }
            self.exit_requested = true;
            return;
        }
        if let Some(state) = self.state.as_mut()
            && let Some(reason) = state.search_benchmark_deadline_reason()
        {
            let benchmark = state
                .replay_benchmark
                .as_ref()
                .expect("search benchmark report");
            let (missing, delta) = (
                benchmark.last_desired_missing,
                benchmark.last_wanted_presented_delta,
            );
            state.finish_replay_benchmark(
                reason,
                missing,
                delta,
                vearth_data::metrics::acquisition_snapshots(),
            );
            self.exit_requested = true;
            return;
        }
        #[cfg(target_os = "macos")]
        self.handle_native_menu_actions();
        if let Some(state) = self.state.as_ref() {
            // egui owns cursor *icons* and may make one visible while painting.
            // Reassert the inactivity policy after the frame has completed.
            state
                .window
                .set_cursor_visible(state.cursor_idle.cursor_visible(Instant::now()));
        }
    }

    fn next_redraw(&self, now: Instant) -> Instant {
        redraw_deadline(
            self.occluded
                || self.zero_sized
                || self
                    .state
                    .as_ref()
                    .is_some_and(|state| state.surface_occluded),
            self.last_redraw,
            now,
        )
    }
}

#[cfg(target_os = "macos")]
struct NativeBenchmarkActivity {
    process: objc2::rc::Retained<objc2_foundation::NSProcessInfo>,
    activity: objc2::rc::Retained<objc2_foundation::NSObject>,
}

#[cfg(target_os = "macos")]
impl NativeBenchmarkActivity {
    fn begin() -> Self {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
        let process = NSProcessInfo::processInfo();
        let reason = NSString::from_str("Low Poly Earth native performance benchmark");
        let options = NSActivityOptions::NSActivityUserInitiated
            | NSActivityOptions::NSActivityIdleDisplaySleepDisabled;
        // The retained token brackets asynchronous startup and the entire
        // event loop. It does not alter window visibility or drawable errors.
        let activity = unsafe { process.beginActivityWithOptions_reason(options, &reason) };
        log::info!("native benchmark activity requested (user-initiated, display-awake)");
        Self { process, activity }
    }
}

#[cfg(target_os = "macos")]
impl Drop for NativeBenchmarkActivity {
    fn drop(&mut self) {
        // This guard uniquely owns the activity token and ends it exactly once,
        // before releasing either the token or its originating process object.
        unsafe { self.process.endActivity(&self.activity) };
        log::info!("native benchmark activity ended");
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native_search_benchmark(
    start: StartPos,
    search: SearchBenchmarkSpec,
    benchmark: ReplayBenchmarkOptions,
) {
    #[cfg(target_os = "macos")]
    let _benchmark_activity = NativeBenchmarkActivity::begin();
    let event_loop = EventLoop::new().expect("event loop");
    event_loop
        .run_app(&mut App::with_search_benchmark(start, search, benchmark))
        .expect("run");
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native(start: StartPos) {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut App::new(start)).expect("run");
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native_landmark_calibrator(
    start: StartPos,
    landmark: &'static vearth_gen::landmarks::Landmark,
) {
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App::new(start);
    app.calibration_landmark = Some(landmark);
    event_loop.run_app(&mut app).expect("run");
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native_replay_with_benchmark(
    start: StartPos,
    replay: ReplayTrace,
    benchmark: Option<ReplayBenchmarkOptions>,
) {
    if let Some(range) = benchmark.as_ref().and_then(|b| b.measurement_range) {
        range
            .validate_trace(replay.last_frame())
            .expect("valid replay measurement range");
    }
    #[cfg(target_os = "macos")]
    let _benchmark_activity = benchmark.is_some().then(NativeBenchmarkActivity::begin);
    let event_loop = EventLoop::new().expect("event loop");
    event_loop
        .run_app(&mut App::with_replay(start, replay, benchmark))
        .expect("run");
}

#[cfg(test)]
mod tests {
    use super::{
        CURSOR_IDLE_TIMEOUT, CursorIdleState, IntentSnapshot, PanSpeedPreset, direct_vehicle_key,
        initial_weather_restore_mode, keyboard_pan_speed_m_s, set_weather_enabled,
    };
    use crate::input::{KeyCode, PhysicalKey};
    use crate::streaming::{ResidentStore, ViewLane, WantedCell};
    use crate::vehicles::VehicleKind;
    use crate::weather::{WeatherConfig, WeatherMode};
    use vearth_core::{cell::CellId, cubesphere::Face};
    use vearth_platform::time::{Duration, Instant};

    #[test]
    fn roof_diagnostics_json_preserves_nonzero_levels_and_source_descriptors() {
        use vearth_gen::contracts::{GenerationStageSnapshot, RoofWorkCounters, SlowRoofRecord};
        let mut snapshot = GenerationStageSnapshot::default();
        let mut work = RoofWorkCounters {
            roofs: 2,
            elapsed_ns: 123,
            triangles: 456,
            ..Default::default()
        };
        work.plane_rejections[3] = 7;
        work.refinement_visits[2] = 8;
        work.elapsed_buckets[7] = 1;
        snapshot.roof_by_shape_and_level[3][14] = work;
        snapshot.slow_roofs[0] = Some(SlowRoofRecord {
            cell_face: 4,
            cell_level: 14,
            cell_i: 10,
            cell_j: 20,
            shape: 3,
            source_ordinal: 3,
            seed: 42,
            extent_m: [50.0, 25.0],
            work,
            ..Default::default()
        });
        snapshot.building_retry_count_by_level[14] = 9;
        let report = super::roof_diagnostics_json(&snapshot);
        assert_eq!(report["by_shape_and_level"].as_object().unwrap().len(), 1);
        assert_eq!(
            report["by_shape_and_level"]["hipped"]
                .as_object()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            report["by_shape_and_level"]["hipped"]["14"]["plane_rejections"]["one_plane"],
            7
        );
        assert_eq!(
            report["by_shape_and_level"]["hipped"]["14"]["refinement_visits_by_depth"][2],
            8
        );
        assert_eq!(report["slowest"].as_array().unwrap().len(), 1);
        assert_eq!(report["slowest"][0]["cell"]["i"], 10);
        assert_eq!(report["slowest"][0]["seed"], 42);
        assert_eq!(report["building_retries"]["count_by_level"][14], 9);
        assert_eq!(
            report["elapsed_bucket_upper_ns"].as_array().unwrap().len(),
            7
        );
    }

    fn convergence_observation(frame: u64) -> super::ReplayConvergenceFrame {
        super::ReplayConvergenceFrame {
            frame,
            after_trace_ms: frame as f64 * 20.0,
            foreground_idle_frames: frame,
            full_idle_frames: frame,
            foreground_missing: 0,
            foreground_unpresented: 0,
            foreground_nonopaque: 0,
            desired_missing: 0,
            wanted_presented_delta: 0,
            selection_ready: true,
            selection_settled: true,
            selected_topology_changes: 0,
            work_pending_after_cut_ready: Some(false),
        }
    }

    #[test]
    fn replay_convergence_diagnostics_preserve_streaks_across_pending_fences_and_background() {
        let mut diagnostics = super::ReplayConvergenceDiagnostics::default();
        for frame in 1..=30 {
            let mut observation = convergence_observation(frame);
            observation.selection_settled = frame % 7 == 0;
            diagnostics.observe(observation);
        }
        assert_eq!(diagnostics.longest_foreground_idle_frames, 30);
        assert_eq!(diagnostics.longest_full_idle_frames, 30);
        assert!(diagnostics.last_resets.is_empty());
        let mut background = convergence_observation(31);
        background.full_idle_frames = 0;
        background.desired_missing = 1;
        background.wanted_presented_delta = 1;
        background.work_pending_after_cut_ready = None;
        diagnostics.observe(background);
        assert_eq!(diagnostics.foreground_resets, 0);
        assert_eq!(diagnostics.full_idle_resets, 1);
        assert_eq!(diagnostics.last_resets[0].full_idle_streak_ended, Some(30));
        assert!(
            diagnostics.last_resets[0]
                .reasons
                .contains(&"desired_missing")
        );
        let mut foreground = background;
        foreground.frame = 32;
        foreground.foreground_idle_frames = 0;
        foreground.foreground_nonopaque = 1;
        foreground.selected_topology_changes = 3;
        diagnostics.observe(foreground);
        let reset = diagnostics.last_resets.back().unwrap();
        assert_eq!(reset.foreground_streak_ended, Some(31));
        assert_eq!(reset.full_idle_streak_ended, None);
        assert!(reset.reasons.contains(&"visible_nonopaque"));
        assert!(reset.reasons.contains(&"selected_topology_change"));
        assert_eq!(diagnostics.longest_foreground_idle_frames, 31);
        let mut opaque = convergence_observation(33);
        opaque.selected_topology_changes = 6;
        diagnostics.observe(opaque);
        assert_eq!(
            diagnostics.foreground_resets, 1,
            "topology alone never invents a readiness reset"
        );
    }

    #[test]
    fn replay_convergence_diagnostics_bound_records_and_throttle_near_idle_sampling() {
        let mut diagnostics = super::ReplayConvergenceDiagnostics::default();
        let mut busy = convergence_observation(1);
        busy.desired_missing = 100;
        busy.foreground_missing = 100;
        busy.foreground_unpresented = 100;
        assert!(!diagnostics.observe(busy));
        assert!(diagnostics.near_idle_samples.is_empty());
        let sample = convergence_observation(2);
        assert!(diagnostics.observe(sample));
        diagnostics.record_sample(sample, Vec::new());
        assert!(!diagnostics.observe(convergence_observation(3)));
        for frame in 1..=40 {
            let mut observation = convergence_observation(frame);
            observation.after_trace_ms = 1000.0 * frame as f64;
            assert!(diagnostics.observe(observation));
            diagnostics.record_sample(observation, Vec::new());
            observation.foreground_idle_frames = 0;
            observation.full_idle_frames = 0;
            observation.foreground_nonopaque = 1;
            observation.wanted_presented_delta = 1;
            assert!(!diagnostics.observe(observation));
        }
        assert_eq!(
            diagnostics.near_idle_samples.len(),
            super::CONVERGENCE_RECORD_LIMIT
        );
        assert_eq!(
            diagnostics.last_resets.len(),
            super::CONVERGENCE_RECORD_LIMIT
        );
        assert_eq!(diagnostics.foreground_resets, 40);
        assert_eq!(
            diagnostics
                .near_idle_samples
                .back()
                .unwrap()
                .observation
                .frame,
            40
        );
        assert_eq!(
            diagnostics.last_resets.back().unwrap().observation.frame,
            40
        );
    }

    #[test]
    fn replay_convergence_cell_samples_keep_exact_ids_fades_and_extra_draws_bounded() {
        use super::{ReplayPresentation, convergence_cell_samples};
        use crate::transition::TransitionPhase;
        let cell = |i| CellId {
            face: Face(4),
            level: 8,
            i,
            j: 50,
        };
        let wanted: Vec<_> = (0..24)
            .map(|i| WantedCell {
                cell: cell(i),
                screen_px: 1.0,
                lane: if i < 12 {
                    ViewLane::Visible
                } else {
                    ViewLane::Background
                },
            })
            .collect();
        let presentation = ReplayPresentation {
            desired: wanted.iter().map(|wanted| wanted.cell).collect(),
            presented: (1..24)
                .map(|i| (cell(i), if i == 1 { f32::NAN } else { 0.5 }))
                .chain((100..124).map(|i| (cell(i), -0.25)))
                .collect(),
        };
        let samples = convergence_cell_samples(
            &wanted,
            &presentation,
            |cell| cell.i != 0,
            |cell| {
                Some(if cell.i >= 100 {
                    TransitionPhase::Outgoing
                } else {
                    TransitionPhase::Incoming
                })
            },
        );
        assert_eq!(samples.len(), super::CONVERGENCE_CELL_LIMIT);
        assert_eq!(samples[0].cell, [4, 8, 0, 50]);
        assert!(!samples[0].resident);
        assert_eq!(samples[0].fade, None);
        assert_eq!(samples[1].fade_bits, Some(f32::NAN.to_bits()));
        assert_eq!(samples[7].cell, [4, 8, 7, 50]);
        assert_eq!(samples[8].cell, [4, 8, 100, 50]);
        assert_eq!(samples[15].cell, [4, 8, 107, 50]);
        assert_eq!(samples[8].role, "extra_draw");
        assert_eq!(samples[8].lane, None);
        assert_eq!(samples[8].phase, Some("outgoing"));
        assert_eq!(samples[8].fade, Some(-0.25));
        let json = serde_json::to_value(&samples).unwrap();
        assert!(json[1]["fade"].is_null());
        assert_eq!(json[1]["fade_bits"], f32::NAN.to_bits());
    }

    #[test]
    fn replay_lane_progress_separates_foreground_from_pending_guard_and_background() {
        use super::{foreground_opaque_idle, replay_lane_counts};
        use std::collections::HashMap;

        let wanted: Vec<_> = [
            ViewLane::Visible,
            ViewLane::Guard,
            ViewLane::Background,
            ViewLane::Legacy,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, lane)| WantedCell {
            cell: CellId {
                face: Face(0),
                level: 8,
                i: i as u32,
                j: 50,
            },
            lane,
            screen_px: 100.0,
        })
        .collect();
        let presented = HashMap::from([(wanted[0].cell, 1.0), (wanted[3].cell, 1.0)]);
        let counts = replay_lane_counts(&wanted, &presented, |cell| presented.contains_key(&cell));
        assert_eq!(counts.map(|lane| lane.desired), [1, 1, 1, 1]);
        assert_eq!(counts.map(|lane| lane.missing), [0, 1, 1, 0]);
        assert_eq!(counts.map(|lane| lane.presented), [1, 0, 0, 1]);
        assert!(foreground_opaque_idle(&counts));

        for missing in [wanted[0].cell, wanted[3].cell] {
            let counts = replay_lane_counts(&wanted, &presented, |cell| cell != missing);
            assert!(!foreground_opaque_idle(&counts));
        }
    }

    #[test]
    fn replay_lane_progress_requires_opaque_exact_targets_not_fading_or_ancestor_cover() {
        use super::{foreground_opaque_idle, replay_lane_counts};
        use std::collections::HashMap;

        let cell = CellId {
            face: Face(0),
            level: 8,
            i: 100,
            j: 50,
        };
        let wanted = [WantedCell {
            cell,
            lane: ViewLane::Visible,
            screen_px: 100.0,
        }];
        let parent = cell.parent().unwrap();
        let mut presented = HashMap::from([(parent, 1.0)]);
        assert!(!foreground_opaque_idle(&replay_lane_counts(
            &wanted,
            &presented,
            |_| true
        )));
        for fade in [-1.0, -0.5, 0.0, 0.5, f32::NAN] {
            presented.insert(cell, fade);
            let counts = replay_lane_counts(&wanted, &presented, |_| true);
            assert_eq!(counts[0].presented, 1);
            assert_eq!(counts[0].opaque_presented, 0);
            assert!(!foreground_opaque_idle(&counts));
        }
        presented.insert(cell, 1.0);
        let counts = replay_lane_counts(&wanted, &presented, |_| true);
        assert_eq!(counts[0].opaque_presented, 1);
        assert!(foreground_opaque_idle(&counts));
    }

    #[test]
    fn residency_keeps_actual_fallbacks_and_releases_superseded_ancestors() {
        use super::{
            DrawCell, LeaseBook, ResidencyBudget, ResidentCell, eviction_plan,
            lease_presented_cells,
        };

        for (child_count, fade) in [3, 4]
            .into_iter()
            .flat_map(|count| [-1.0, -0.5, 0.0, 0.5, 1.0].map(|fade| (count, fade)))
        {
            let parent = CellId {
                face: Face(0),
                level: 10,
                i: 100,
                j: 200,
            };
            let grandparent = parent.parent().unwrap();
            let children = parent.children();
            let mut index = ResidentStore::new();
            for cell in [grandparent, parent]
                .into_iter()
                .chain(children[..child_count].iter().copied())
            {
                index.insert(cell, 0, (0.0, 0.0), 0.0, 0.0, None, None, 100);
            }
            let wanted: Vec<_> = children
                .iter()
                .map(|cell| WantedCell {
                    cell: *cell,
                    screen_px: 2.0,
                    lane: ViewLane::Legacy,
                })
                .collect();
            let before = index.drawable_cut(&wanted);
            let draw: Vec<_> = before
                .0
                .iter()
                .map(|cell| DrawCell {
                    cell: *cell,
                    resident: index.id(*cell).unwrap(),
                    fade,
                })
                .collect();
            let mut leases = LeaseBook::default();
            for (_, record) in index.iter() {
                leases.register_resident(record.cell, 10);
            }
            lease_presented_cells(&draw, &mut index, &mut leases, 10);
            let victims = eviction_plan(
                index.len(),
                index.gpu_bytes(),
                10,
                &mut leases,
                ResidencyBudget {
                    hard_bytes: 250,
                    low_bytes: 200,
                    evict_bytes_per_frame: 1000,
                },
                |cell| {
                    let (_, record) = index.get_cell(cell).unwrap();
                    ResidentCell {
                        cell,
                        gpu_bytes: record.gpu_bytes,
                        last_used_frame: record.last_drawn_frame,
                    }
                },
            );
            assert!(victims.contains(&grandparent));
            assert_eq!(victims.contains(&parent), child_count == 4);
            for cell in victims {
                index.remove(cell);
                leases.unregister_resident(cell);
            }
            assert_eq!(
                index.drawable_cut(&wanted),
                before,
                "eviction changed displayed coverage"
            );
        }
    }

    #[test]
    fn child_prefetch_requires_a_predicted_or_destination_view() {
        for lane in [
            ViewLane::Visible,
            ViewLane::Legacy,
            ViewLane::Guard,
            ViewLane::Background,
        ] {
            let parent = CellId {
                face: Face(0),
                level: 10,
                i: 0,
                j: 0,
            };
            let wanted = [WantedCell {
                cell: parent,
                screen_px: 3.0,
                lane,
            }];
            let children = parent.children();
            let mut index = ResidentStore::new();
            index.insert(parent, 0, (0.0, 0.0), 0.0, 0.0, None, None, 0);
            let stationary = IntentSnapshot::build(&wanted, &index, [], &[], &[]);
            assert!(stationary.contains(parent));
            for child in children {
                assert!(!stationary.contains(child));
            }
            let moving = IntentSnapshot::build(
                &wanted,
                &index,
                [],
                &[(children[0], 2.0)],
                &[(children[1], 1.0)],
            );
            assert!(moving.contains(children[0]));
            assert!(moving.contains(children[1]));
            assert!(!moving.contains(children[2]));
            assert!(!moving.contains(children[3]));
        }
    }

    fn generation_targets(intents: &IntentSnapshot, index: &ResidentStore) -> Vec<(CellId, f64)> {
        let mut jobs = Vec::new();
        intents.generation_jobs_into(
            index,
            |_| false,
            glam::DVec3::ZERO,
            glam::DVec3::X,
            &mut jobs,
        );
        jobs.sort_by_key(|(cell, _)| *cell);
        jobs
    }

    #[test]
    fn intent_cache_preserves_priorities_through_input_and_membership_changes() {
        let parent = CellId {
            face: Face(2),
            level: 16,
            i: 1821,
            j: 9082,
        };
        let leaves = parent.children();
        let mut wanted: Vec<_> = leaves
            .into_iter()
            .enumerate()
            .map(|(i, cell)| WantedCell {
                cell,
                screen_px: 3.0 + i as f64,
                lane: ViewLane::Visible,
            })
            .collect();
        let mut destination = vec![(leaves[0], 10.0)];
        let mut predicted = vec![(leaves[1], 20.0)];
        let mut presented = vec![parent, leaves[2]];
        let mut index = ResidentStore::new();
        let mut cached = IntentSnapshot::default();
        for step in 0..14 {
            match step {
                1 => wanted[0].screen_px = f64::from_bits(wanted[0].screen_px.to_bits() + 1),
                2 => wanted[1].lane = ViewLane::Guard,
                3 => destination[0].1 += 1.0,
                4 => predicted[0].0 = leaves[3],
                5 => presented.reverse(),
                6 => {
                    index.insert(parent, 0, (0.0, 0.0), 1.0, 0.0, None, None, 0);
                }
                7 => {
                    let mut replacement = ResidentStore::new();
                    replacement.insert(leaves[0], 0, (0.0, 0.0), 1.0, 0.0, None, None, 0);
                    assert_eq!(index.membership_stamp().1, replacement.membership_stamp().1);
                    index = replacement;
                }
                8 => {
                    index.insert(leaves[1], 0, (0.0, 0.0), 1.0, 0.0, None, None, 0);
                }
                9 => {
                    index.remove(leaves[0]);
                }
                10 => wanted.reverse(),
                11 => wanted.clear(),
                12 => {
                    destination.clear();
                    predicted.clear();
                }
                13 => presented.clear(),
                _ => {}
            }
            let reference = IntentSnapshot::build(
                &wanted,
                &index,
                presented.iter().copied(),
                &destination,
                &predicted,
            );
            let before = cached.rebuilds;
            cached.rebuild(
                &wanted,
                &index,
                presented.iter().copied(),
                &destination,
                &predicted,
            );
            assert_eq!(cached.rebuilds, before + 1, "input change {step}");
            assert_eq!(cached.priorities, reference.priorities);
            assert_eq!(cached.generation, reference.generation);
            assert_eq!(cached.prefetch_leases, reference.prefetch_leases);
            assert_eq!(
                generation_targets(&cached, &index),
                generation_targets(&reference, &index)
            );
            index.touch(leaves[1], 100 + step);
            cached.rebuild(
                &wanted,
                &index,
                presented.iter().copied(),
                &destination,
                &predicted,
            );
            assert_eq!(cached.rebuilds, before + 1, "stable inputs {step}");
        }
    }

    #[test]
    fn intent_cache_tracks_last_descendant_arrival_and_coverage_removal() {
        let parent = CellId {
            face: Face(1),
            level: 13,
            i: 1821,
            j: 3082,
        };
        let children = parent.children();
        let root = coverage_root(parent);
        let wanted = [WantedCell {
            cell: parent,
            screen_px: 5.0,
            lane: ViewLane::Visible,
        }];
        let mut index = ResidentStore::new();
        for child in &children[..3] {
            index.insert_for_test(*child, 0);
        }
        let mut cached = IntentSnapshot::build(&wanted, &index, [], &[], &[]);
        assert!(cached.contains(root));
        cached.rebuild(&wanted, &index, [], &[], &[]);
        assert_eq!(cached.rebuilds, 1);

        // Completion happens after the frame's intent snapshot was built.
        index.insert_for_test(children[3], 1);
        cached.rebuild(&wanted, &index, [], &[], &[]);
        assert_eq!(cached.rebuilds, 2);
        assert!(!cached.contains(root));
        assert_eq!(
            generation_targets(&cached, &index)
                .iter()
                .map(|(cell, _)| *cell)
                .collect::<Vec<_>>(),
            [parent]
        );
        cached.rebuild(&wanted, &index, [], &[], &[]);
        assert_eq!(cached.rebuilds, 2);

        index.remove(children[3]).unwrap();
        cached.rebuild(&wanted, &index, [], &[], &[]);
        assert_eq!(cached.rebuilds, 3);
        assert!(cached.contains(root));
        assert!(
            generation_targets(&cached, &index)
                .iter()
                .any(|(cell, _)| *cell == root)
        );
    }

    #[test]
    fn intent_cache_hits_keep_camera_staging_and_lease_frames_live() {
        use super::{DESTINATION_LEASE_FRAMES, LeaseBook, LeaseKind};
        use glam::DVec3;

        let parent = CellId {
            face: Face(2),
            level: 14,
            i: 1000,
            j: 2000,
        };
        let leaves = parent.children();
        let wanted = [
            WantedCell {
                cell: leaves[0],
                screen_px: 4.0,
                lane: ViewLane::Legacy,
            },
            WantedCell {
                cell: leaves[1],
                screen_px: 3.0,
                lane: ViewLane::Legacy,
            },
        ];
        let destination = [(leaves[2], 2.0)];
        let predicted = [(leaves[3], 1.0)];
        let mut index = ResidentStore::new();
        index.insert_for_test(parent, 0);
        let mut cached = IntentSnapshot::build(&wanted, &index, [], &destination, &predicted);
        let anchor = vearth_core::ellipsoid::surface_point_along(leaves[0].center_dir());
        let mut jobs = Vec::new();
        cached.generation_jobs_into(
            &index,
            |_| false,
            anchor + DVec3::X * 1_000.0,
            DVec3::X,
            &mut jobs,
        );
        let original: std::collections::BTreeMap<_, _> = jobs.iter().copied().collect();
        assert_eq!(original[&leaves[0]], 1.0);

        cached.rebuild(&wanted, &index, [], &destination, &predicted);
        assert_eq!(cached.rebuilds, 1);
        cached.generation_jobs_into(
            &index,
            |cell| cell == leaves[1],
            anchor - DVec3::X * 1_000.0,
            DVec3::X,
            &mut jobs,
        );
        let moved: std::collections::BTreeMap<_, _> = jobs.iter().copied().collect();
        assert_eq!(moved.len(), 3);
        assert!(!moved.contains_key(&leaves[1]));
        assert_eq!(moved[&leaves[0]], 4.0);
        assert_eq!(moved[&leaves[2]], original[&leaves[2]]);
        assert_eq!(moved[&leaves[3]], original[&leaves[3]]);

        let mut leases = LeaseBook::default();
        cached.lease_prefetch(&mut leases, 10);
        assert_eq!(leases.kind(leaves[2], 11 + DESTINATION_LEASE_FRAMES), None);
        assert_eq!(leases.kind(leaves[3], 131), None);
        cached.rebuild(&wanted, &index, [], &destination, &predicted);
        assert_eq!(cached.rebuilds, 1);
        cached.lease_prefetch(&mut leases, 20);
        assert_eq!(
            leases.kind(leaves[2], 11 + DESTINATION_LEASE_FRAMES),
            Some(LeaseKind::Destination)
        );
        assert_eq!(leases.kind(leaves[3], 131), Some(LeaseKind::Predicted));
        assert_eq!(leases.kind(leaves[2], 21 + DESTINATION_LEASE_FRAMES), None);
        assert_eq!(leases.kind(leaves[3], 141), None);
    }

    #[test]
    #[ignore = "manual release-mode frame intent benchmark"]
    fn benchmark_stable_frame_intents() {
        let wanted: Vec<_> = (0..2200)
            .map(|i| WantedCell {
                cell: CellId {
                    face: Face(2),
                    level: 16,
                    i: 1800 + i % 50,
                    j: 9000 + i / 50,
                },
                screen_px: 2.0 + f64::from(i % 20),
                lane: ViewLane::Visible,
            })
            .collect();
        let mut index = ResidentStore::new();
        for w in &wanted {
            index.insert(w.cell, 0, (0.0, 0.0), 1.0, 0.0, None, None, 0);
        }
        let mut snapshots: [IntentSnapshot; 2] = std::array::from_fn(|_| IntentSnapshot::default());
        let mut samples: [Vec<f64>; 2] = std::array::from_fn(|_| Vec::new());
        for iteration in 0..202 {
            for cached in [iteration % 2 == 0, iteration % 2 != 0] {
                let slot = usize::from(cached);
                let snapshot = &mut snapshots[slot];
                if !cached {
                    snapshot.inputs.store_identity = None;
                }
                let started = Instant::now();
                snapshot.rebuild(&wanted, &index, wanted.iter().map(|w| w.cell), &[], &[]);
                std::hint::black_box(&snapshot.priorities);
                if iteration >= 2 {
                    samples[slot].push(started.elapsed().as_secs_f64() * 1e6);
                }
            }
        }
        for values in &mut samples {
            values.sort_by(f64::total_cmp);
        }
        eprintln!(
            "stable intents uncached_us={:.3} cached_us={:.3}",
            samples[0][100], samples[1][100]
        );
    }

    fn coverage_root(mut cell: CellId) -> CellId {
        while cell.level > vearth_core::cell::MIN_RENDER_LEVEL {
            cell = cell.parent().unwrap();
        }
        cell
    }

    #[test]
    fn explicit_streaming_targets_share_admission_without_implicit_ancestor_jobs() {
        let parent = CellId {
            face: Face(2),
            level: 16,
            i: 1821,
            j: 9082,
        };
        let leaves = parent.children();
        let root = coverage_root(parent);
        let wanted: Vec<_> = leaves
            .into_iter()
            .enumerate()
            .map(|(i, cell)| WantedCell {
                cell,
                screen_px: 3.0 + i as f64,
                lane: ViewLane::Visible,
            })
            .collect();
        let mut index = ResidentStore::new();
        let plan = IntentSnapshot::build(&wanted, &index, [], &[], &[]);
        let jobs = generation_targets(&plan, &index);
        assert_eq!(jobs.len(), leaves.len() + 1);
        assert_eq!(
            jobs.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0,
            root
        );
        assert!(jobs.iter().all(|(cell, _)| plan.contains(*cell)));
        assert!(plan.contains(root));
        assert!(!plan.contains(parent));
        for leaf in leaves {
            assert!(plan.contains(leaf));
        }
        index.insert(parent, 0, (0.0, 0.0), 1.0, 0.0, None, None, 0);
        let plan = IntentSnapshot::build(&wanted, &index, [parent], &[], &[]);
        assert!(
            plan.contains(parent),
            "a presented backdrop remains useful for admission"
        );
        assert!(
            !plan.contains(root),
            "an existing backdrop already covers every target"
        );
        let mut sorted_leaves = leaves;
        sorted_leaves.sort();
        assert_eq!(
            generation_targets(&plan, &index)
                .iter()
                .map(|(cell, _)| *cell)
                .collect::<Vec<_>>(),
            sorted_leaves
        );
        let mut staged_jobs = Vec::new();
        plan.generation_jobs_into(
            &index,
            |cell| cell == leaves[0],
            glam::DVec3::ZERO,
            glam::DVec3::X,
            &mut staged_jobs,
        );
        assert_eq!(staged_jobs.len(), 3);
        assert!(!staged_jobs.iter().any(|(cell, _)| *cell == leaves[0]));
    }

    #[test]
    fn direct_final_arrivals_preserve_coverage_and_draw_before_siblings() {
        use crate::transition::CoverageIndex;
        let root = CellId {
            face: Face(4),
            level: vearth_core::cell::MIN_RENDER_LEVEL,
            i: 3,
            j: 4,
        };
        let leaves: Vec<_> = root
            .children()
            .into_iter()
            .flat_map(CellId::children)
            .flat_map(CellId::children)
            .collect();
        let wanted: Vec<_> = leaves
            .iter()
            .copied()
            .map(|cell| WantedCell {
                cell,
                screen_px: 4.0,
                lane: ViewLane::Visible,
            })
            .collect();
        let mut index = ResidentStore::new();
        index.insert(root, 0, (0.0, 0.0), 1.0, 0.0, None, None, 0);
        let mut presented = vec![root];
        // Visit different quadrants before completing any sibling group.
        for step in 0..leaves.len() {
            let cell = leaves[(step * 17) % leaves.len()];
            let plan = IntentSnapshot::build(&wanted, &index, presented.iter().copied(), &[], &[]);
            let jobs = generation_targets(&plan, &index);
            assert!(jobs.iter().any(|(target, _)| *target == cell));
            assert!(
                jobs.iter()
                    .all(|(target, _)| wanted.iter().any(|w| w.cell == *target))
            );
            index.insert(cell, step as u64, (0.0, 0.0), 1.0, 0.0, None, None, 0);
            let (draw, _) = index.drawable_cut(&wanted);
            assert!(
                draw.contains(&cell),
                "fine arrival waited for siblings at step {step}"
            );
            let mut coverage = CoverageIndex::default();
            coverage.rebuild(draw.iter().copied());
            assert!(
                wanted.iter().all(|w| coverage.fully_covers(w.cell)),
                "coverage hole at step {step}"
            );
            assert_eq!(draw.contains(&root), step + 1 != leaves.len());
            presented = draw;
        }
        let plan = IntentSnapshot::build(&wanted, &index, presented, &[], &[]);
        assert!(generation_targets(&plan, &index).is_empty());
        let (draw, missing) = index.drawable_cut(&wanted);
        assert_eq!(draw, leaves);
        assert!(missing.is_empty());
    }

    #[test]
    fn covered_merge_requests_only_its_final_target_and_ready_obsolete_root_drops() {
        use crate::{
            admission::{IntentClass, UploadPriority, UploadStager},
            prepared_render::{pack_surface_cell, test_surface},
        };
        let parent = CellId {
            face: Face(1),
            level: 13,
            i: 1821,
            j: 3082,
        };
        let children = parent.children();
        let root = coverage_root(parent);
        let wanted = [WantedCell {
            cell: parent,
            screen_px: 5.0,
            lane: ViewLane::Visible,
        }];
        let mut index = ResidentStore::new();
        let empty_plan = IntentSnapshot::build(&wanted, &index, [], &[], &[]);
        assert!(empty_plan.contains(root));
        let mut ready = UploadStager::new();
        ready.push(
            pack_surface_cell(test_surface(root)).unwrap(),
            UploadPriority::new(IntentClass::VisibleCoverage, 5.0),
        );
        for cell in children {
            index.insert(cell, 0, (0.0, 0.0), 1.0, 0.0, None, None, 0);
        }
        let plan = IntentSnapshot::build(&wanted, &index, children, &[], &[]);
        assert_eq!(
            generation_targets(&plan, &index)
                .iter()
                .map(|(cell, _)| *cell)
                .collect::<Vec<_>>(),
            [parent]
        );
        assert!(!plan.contains(root));
        assert_eq!(ready.drop_stale(|cell| plan.contains(cell)).cells, 1);
        assert!(ready.is_empty());
        let no_targets = IntentSnapshot::build(&[], &index, [root], &[], &[]);
        assert!(no_targets.contains(root));
        assert!(
            generation_targets(&no_targets, &index).is_empty(),
            "presentation must not regenerate cells"
        );
    }

    #[test]
    fn prefetch_leases_and_generation_cover_only_explicit_targets_and_bootstraps() {
        use super::{LeaseBook, LeaseKind};
        let destination = CellId {
            face: Face(0),
            level: 15,
            i: 1000,
            j: 2000,
        };
        let predicted = CellId {
            face: Face(5),
            level: 18,
            i: 22000,
            j: 85000,
        };
        let mut index = ResidentStore::new();
        let parent = destination.parent().unwrap();
        index.insert(parent, 0, (0.0, 0.0), 1.0, 0.0, None, None, 0);
        let root = coverage_root(predicted);
        let plan =
            IntentSnapshot::build(&[], &index, [], &[(destination, 2.0)], &[(predicted, 1.0)]);
        let jobs = generation_targets(&plan, &index);
        assert_eq!(jobs.len(), 3);
        assert!(
            jobs.iter()
                .all(|(cell, _)| [destination, predicted, root].contains(cell))
        );
        let mut leases = LeaseBook::default();
        plan.lease_prefetch(&mut leases, 10);
        assert_eq!(leases.kind(destination, 10), Some(LeaseKind::Destination));
        assert_eq!(leases.kind(predicted, 10), Some(LeaseKind::Predicted));
        assert_eq!(leases.kind(root, 10), Some(LeaseKind::Predicted));
        assert_eq!(
            leases.kind(parent, 10),
            None,
            "an unused resident ancestor was implicitly leased"
        );
        assert_eq!(leases.len(), 3);
        let changed = IntentSnapshot::build(&[], &index, [], &[], &[]);
        assert!(generation_targets(&changed, &index).is_empty());
        assert!(!changed.contains(root));
        let combined =
            IntentSnapshot::build(&[], &index, [], &[(predicted, 2.0)], &[(predicted, 1.0)]);
        let mut combined_leases = LeaseBook::default();
        combined.lease_prefetch(&mut combined_leases, 20);
        assert_eq!(
            combined_leases.kind(predicted, 200),
            Some(LeaseKind::Destination)
        );
        assert_eq!(
            combined_leases.kind(root, 200),
            Some(LeaseKind::Destination)
        );
    }

    #[test]
    fn cursor_hides_after_inactivity_and_movement_reveals_it() {
        let started_at = Instant::now();
        let mut cursor = CursorIdleState::new(started_at);

        assert!(cursor.cursor_visible(started_at));
        assert!(cursor.cursor_visible(started_at + CURSOR_IDLE_TIMEOUT - Duration::from_nanos(1)));
        assert!(!cursor.cursor_visible(started_at + CURSOR_IDLE_TIMEOUT));

        let moved_at = started_at + CURSOR_IDLE_TIMEOUT + Duration::from_secs(1);
        cursor.moved(moved_at);
        assert!(cursor.cursor_visible(moved_at));
        assert!(!cursor.cursor_visible(moved_at + CURSOR_IDLE_TIMEOUT));
    }

    #[test]
    fn cursor_stays_visible_outside_or_while_unfocused() {
        let started_at = Instant::now();
        let idle_at = started_at + CURSOR_IDLE_TIMEOUT;
        let mut cursor = CursorIdleState::new(started_at);

        cursor.left();
        assert!(cursor.cursor_visible(idle_at));

        cursor.entered(idle_at);
        cursor.focused(false, idle_at);
        assert!(cursor.cursor_visible(idle_at + CURSOR_IDLE_TIMEOUT));

        let focused_at = idle_at + CURSOR_IDLE_TIMEOUT;
        cursor.focused(true, focused_at);
        assert!(cursor.cursor_visible(focused_at));
        assert!(!cursor.cursor_visible(focused_at + CURSOR_IDLE_TIMEOUT));
    }

    #[test]
    fn number_keys_select_vehicles_in_cycle_order() {
        for (key, vehicle) in [
            (KeyCode::Digit1, VehicleKind::Kestrel),
            (KeyCode::Digit2, VehicleKind::PhaseManta),
            (KeyCode::Digit3, VehicleKind::Sparrow),
            (KeyCode::Numpad1, VehicleKind::Kestrel),
            (KeyCode::Numpad2, VehicleKind::PhaseManta),
            (KeyCode::Numpad3, VehicleKind::Sparrow),
        ] {
            assert_eq!(direct_vehicle_key(&PhysicalKey::Code(key)), Some(vehicle));
        }
        assert_eq!(
            direct_vehicle_key(&PhysicalKey::Code(KeyCode::Digit4)),
            None
        );
    }

    #[test]
    fn weather_disable_restores_the_previous_mode_and_preset() {
        let mut config = WeatherConfig {
            mode: WeatherMode::Preset,
            preset: 42,
            ..WeatherConfig::default()
        };
        let mut restore_mode = initial_weather_restore_mode(config.mode);

        set_weather_enabled(&mut config, &mut restore_mode, false);
        assert_eq!(config.mode, WeatherMode::Clear);
        assert_eq!(config.preset, 42);

        set_weather_enabled(&mut config, &mut restore_mode, true);
        assert_eq!(config.mode, WeatherMode::Preset);
        assert_eq!(config.preset, 42);
    }

    #[test]
    fn initially_disabled_weather_enables_as_procedural() {
        let mut config = WeatherConfig {
            mode: WeatherMode::Clear,
            ..WeatherConfig::default()
        };
        let mut restore_mode = initial_weather_restore_mode(config.mode);

        set_weather_enabled(&mut config, &mut restore_mode, true);
        assert_eq!(config.mode, WeatherMode::Procedural);
    }

    #[test]
    fn keyboard_pan_slow_preserves_the_original_sea_level_response() {
        let close = keyboard_pan_speed_m_s(30.0, Some(0.0), PanSpeedPreset::Slow);
        assert!((close - 30.0 * 0.9).abs() < 1e-9);

        // At 250 m and above on low terrain, retain the original response.
        for alt_m in [250.0, 1_000.0, 50_000.0] {
            let speed = keyboard_pan_speed_m_s(alt_m, Some(0.0), PanSpeedPreset::Slow);
            assert!((speed - alt_m * 0.9).abs() < 1e-9, "altitude {alt_m}");
        }
    }

    #[test]
    fn keyboard_pan_uses_clearance_on_high_terrain() {
        let sea_level = keyboard_pan_speed_m_s(30.0, Some(0.0), PanSpeedPreset::default());
        let mountain = keyboard_pan_speed_m_s(2_030.0, Some(2_000.0), PanSpeedPreset::default());
        assert!((mountain - sea_level).abs() < 1e-9);

        // A newly resident cell can temporarily report a conservative
        // terrain maximum above the camera. It must not collapse movement
        // below the normal street-level response.
        let conservative =
            keyboard_pan_speed_m_s(2_030.0, Some(2_050.0), PanSpeedPreset::default());
        assert!((conservative - sea_level).abs() < 1e-9);

        // Far enough above the terrain, the existing ellipsoid-altitude
        // response is restored exactly.
        let regional = keyboard_pan_speed_m_s(3_000.0, Some(2_000.0), PanSpeedPreset::default());
        assert!((regional - 3_000.0 * 0.9 * 1.5).abs() < 1e-9);
    }

    #[test]
    fn keyboard_pan_speed_increases_smoothly_with_flat_ground_clearance() {
        let mut previous = 0.0;
        for alt_m in (5..=1_000).map(f64::from) {
            let speed = keyboard_pan_speed_m_s(alt_m, Some(0.0), PanSpeedPreset::default());
            assert!(speed >= previous, "speed fell at {alt_m} m");
            previous = speed;
        }
    }

    #[test]
    fn keyboard_pan_presets_shift_the_old_default_to_slow() {
        assert_eq!(PanSpeedPreset::default(), PanSpeedPreset::Standard);

        let old_default = keyboard_pan_speed_m_s(30.0, Some(0.0), PanSpeedPreset::Slow);
        for (preset, multiplier) in [
            (PanSpeedPreset::Slow, 1.0),
            (PanSpeedPreset::Standard, 1.5),
            (PanSpeedPreset::Fast, 2.0),
            (PanSpeedPreset::VeryFast, 2.5),
        ] {
            let speed = keyboard_pan_speed_m_s(30.0, Some(0.0), preset);
            assert!((speed - old_default * multiplier).abs() < 1e-9);
        }
    }
}
