//! Streaming controller: computes the screen-error-driven wanted
//! cell set, feeds a generate+mesh worker pool, and maintains the resident
//! cache with deepest-resident-ancestor fallback so refinement never leaves
//! holes.

use glam::{DMat3, DMat4, DVec3};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use vearth_core::cell::{CellId, ChunkAddr, MIN_RENDER_LEVEL};
use vearth_core::cell_key::{CellKey, CellRange};
use vearth_core::cubesphere::Face;
use vearth_core::ellipsoid;
use vearth_core::frame::ChunkFrame;
use vearth_core::lod;
use vearth_gen::collision::{MetricFrame, SurfaceCollision};
use {
    crate::chunk_cache::ChunkCache,
    crate::prepared_render::{PreparedRenderCell, pack_surface_cell, pack_surface_cell_with},
    std::sync::Arc,
    std::sync::atomic::Ordering,
    std::sync::mpsc::{Receiver, SyncSender, sync_channel},
    vearth_data::registry::Registry,
    vearth_data::vectors::FeaturePack,
    vearth_gen::{
        RoadGuide, Sources, SurfaceCell, TerrainCollision, generate_surface_cell,
        generate_surface_cell_with_features,
    },
};

/// Voxel screen-size tolerance in pixels: refine while voxels are bigger.
pub const TAU_PX: f64 = 3.0;
/// Merge hysteresis: a cell that split keeps its children until its error
/// drops well below the split threshold. Without the deadband the cut
/// oscillates when the camera rests near a boundary — harmless before LOD
/// fades, a visible dither pulse after them.
const TAU_MERGE_RATIO: f64 = 0.7;
/// Stream the ordinary refinement ladder to street level. Terrain below ~5 m/px
/// is upsampled from the finest elevation level (coarser outside regional
/// models); buildings/roads/trees carry the detail.
const STANDARD_MAX_STREAM_LEVEL: u8 = vearth_core::cell::MAX_LEVEL;
/// One extra local level is available only inside explicitly biased landmark
/// footprints. It halves their visible voxel pitch without multiplying the
/// global resident-cell budget.
pub const MAX_STREAM_LEVEL: u8 = STANDARD_MAX_STREAM_LEVEL + 1;
/// Refinement budget per wanted-set computation.
const MAX_WANTED: usize = 3600;
/// Cell plans that may acquire at once when every provider is async. Remote
/// tiles can take seconds to first byte, so this follows the provider socket
/// quotas (which still bound requests per origin), not CPU parallelism.
pub const ASYNC_ACQUISITION_CELLS: usize = 96;
/// Leave a small part of the hard budget for the post-pass that enforces a
/// 2:1 neighbor relationship. The balanced cut still never exceeds
/// `MAX_WANTED`.
const BALANCE_RESERVE: usize = 256;
/// Reallocate the existing leaf budget toward the view; do not increase the
/// resident-cell envelope merely to improve LOD selection.
const MAX_VIEW_WANTED: usize = MAX_WANTED;
/// Extra field of view retained at useful quality so a small turn does not
/// immediately expose the coarse background cut.
const VIEW_GUARD_SCALE: f64 = 1.35;
/// Keep guard cells close to visible quality (and within roughly one LOD)
/// rather than starving the guard whenever the visible budget fills.
const GUARD_REFINEMENT_WEIGHT: f64 = 0.75;
/// Rugged visible terrain receives up to one extra quadtree level under the
/// existing leaf budget. A neighboring cell inherits part of that preference
/// so the LOD boundary falls outside the mountain silhouette rather than
/// cutting straight through it.
const MOUNTAIN_EXTRA_LEVEL_FACTOR: f64 = 2.0;
const MOUNTAIN_HALO_WEIGHT: f64 = 0.65;

fn smoothstep(lo: f64, hi: f64, value: f64) -> f64 {
    let t = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mountain_detail_multiplier(
    terrain_relief_m: f32,
    terrain_step_m: f32,
    source_sample_pitch_m: f64,
) -> f64 {
    let relief_m = f64::from(terrain_relief_m.max(0.0));
    let slope = f64::from(terrain_step_m.max(0.0)) / source_sample_pitch_m.max(1.0);
    let relief_score = smoothstep(180.0, 700.0, relief_m);
    let slope_score = smoothstep(0.15, 0.65, slope);
    let ruggedness = relief_score.max(slope_score);
    1.0 + (MOUNTAIN_EXTRA_LEVEL_FACTOR - 1.0) * ruggedness
}

/// Camera projection separates view/turn-guard quality from background
/// coverage. Off-screen regions remain represented by a compatible coarse cut.
#[derive(Clone, Copy, Debug)]
pub struct LodView {
    world_to_view: DMat3,
    tan_half_fov_y: f64,
    aspect: f64,
    coalesce_background: bool,
}

impl LodView {
    pub fn new(view_rotation: DMat4, fov_y: f64, aspect: f64) -> Self {
        LodView {
            world_to_view: DMat3::from_mat4(view_rotation),
            tan_half_fov_y: (fov_y * 0.5).tan().max(1e-6),
            aspect: aspect.max(1e-3),
            coalesce_background: true,
        }
    }

    pub(crate) fn with_background_coalescing(mut self, enabled: bool) -> Self {
        self.coalesce_background = enabled;
        self
    }

    #[cfg(test)]
    fn lane(self, cam_ecef: DVec3, cell: CellId) -> ViewLane {
        let anchor = ellipsoid::surface_point_along(cell.center_dir());
        self.lane_at_anchor(cam_ecef, cell, anchor)
    }

    fn lane_at_anchor(self, cam_ecef: DVec3, cell: CellId, anchor: DVec3) -> ViewLane {
        // Bound the same 0–9 km terrain volume used by the SSE distance
        // estimate. Coarse cells deliberately get a large sphere so a child
        // that might enter the frustum is never prematurely deprioritized.
        let up = anchor.normalize();
        let center = anchor + up * 4_500.0;
        // Cube-sphere scale varies across a face; 0.75× is only the planar
        // half-diagonal and can under-bound coarse edge cells. A conservative
        // 1.1× span prevents terrain that reaches the real frustum from being
        // mislabeled as coarse background (the source of dark vertical
        // curtains at the view boundary).
        let radius = (cell.size_m() * 1.1).hypot(4_500.0);
        let view_center = self.world_to_view * (center - cam_ecef);
        if self.intersects(view_center, radius, 1.0) {
            ViewLane::Visible
        } else if self.intersects(view_center, radius, VIEW_GUARD_SCALE) {
            ViewLane::Guard
        } else {
            ViewLane::Background
        }
    }

    fn intersects(self, center: DVec3, radius: f64, scale: f64) -> bool {
        let depth = -center.z;
        if depth + radius < 0.0 {
            return false;
        }
        let tan_y = self.tan_half_fov_y * scale;
        let tan_x = tan_y * self.aspect;
        depth * tan_x - center.x.abs() + radius * (1.0 + tan_x * tan_x).sqrt() >= 0.0
            && depth * tan_y - center.y.abs() + radius * (1.0 + tan_y * tan_y).sqrt() >= 0.0
    }
}

impl LodView {
    fn content_bound(
        cell: CellId,
        anchor: DVec3,
        height_range: Option<(f64, f64)>,
    ) -> (DVec3, f64) {
        let (up, mid, half_height) = match height_range {
            Some((lo, hi)) if lo.is_finite() && hi.is_finite() && lo <= hi => (
                ellipsoid::geodetic_normal(anchor),
                lo * 0.5 + hi * 0.5,
                hi * 0.5 - lo * 0.5,
            ),
            // Unloaded mountains retain the original global envelope. The
            // lower unknown ceiling used for SSE is not a visibility bound.
            _ => (anchor.normalize(), 4_500.0, 4_500.0),
        };
        // Cube-sphere scale varies across a face; 0.75× is only the planar
        // half-diagonal and can under-bound coarse edge cells. A conservative
        // 1.1× span prevents terrain that reaches the real frustum from being
        // mislabeled as coarse background (the source of dark vertical
        // curtains at the view boundary).
        let radius = (cell.size_m() * 1.1).hypot(half_height);
        (anchor + up * mid, radius)
    }

    fn diagnostic_lane_at_anchor(
        self,
        cam_ecef: DVec3,
        cell: CellId,
        anchor: DVec3,
        height_range: Option<(f64, f64)>,
    ) -> ViewLane {
        let (center, radius) = Self::content_bound(cell, anchor, height_range);
        let view_center = self.world_to_view * (center - cam_ecef);
        if self.intersects(view_center, radius, 1.0) {
            ViewLane::Visible
        } else if self.intersects(view_center, radius, VIEW_GUARD_SCALE) {
            ViewLane::Guard
        } else {
            ViewLane::Background
        }
    }
}

/// Borrowed selector metadata; source summaries are shared only at ownership
/// transitions, never cloned by the many target and halo lookups in a cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainMetricRef<'a> {
    pub height_range: (f32, f32),
    pub terrain_step_m: f32,
    pub terrain_relief_m: f32,
    pub terrain_detail: Option<&'a vearth_gen::TerrainDetailSummary>,
}

impl<'a> From<((f32, f32), f32, f32)> for TerrainMetricRef<'a> {
    fn from((height_range, terrain_step_m, terrain_relief_m): ((f32, f32), f32, f32)) -> Self {
        Self {
            height_range,
            terrain_step_m,
            terrain_relief_m,
            terrain_detail: None,
        }
    }
}

fn terrain_metric_for<'a>(
    cell: CellId,
    metric_lookup: Option<&TerrainMetricLookup<'a>>,
) -> Option<(TerrainMetricRef<'a>, CellId)> {
    let metric_lookup = metric_lookup?;
    let mut source = cell;
    loop {
        if let Some(metric) = metric_lookup(source) {
            return Some((metric, source));
        }
        source = source.parent()?;
    }
}

#[cfg(test)]
fn terrain_metrics_for(
    cell: CellId,
    metric_lookup: Option<&TerrainMetricLookup<'_>>,
) -> Option<((f32, f32), f32, f32, CellId)> {
    terrain_metric_for(cell, metric_lookup).map(|(metric, source)| {
        (
            metric.height_range,
            metric.terrain_step_m,
            metric.terrain_relief_m,
            source,
        )
    })
}

/// Terrain evidence that grants mountain refinement its extra level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuggednessPolicy {
    /// Steps covering a broad share of the region. Isolated quay walls,
    /// cuttings and embankments in otherwise gentle terrain do not qualify.
    Broad,
    /// The single largest sample step anywhere in the region.
    MaxStep,
}

impl RuggednessPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Broad => "broad",
            Self::MaxStep => "max-step",
        }
    }
}

const RUGGEDNESS_UNSET: u8 = 0;
const RUGGEDNESS_BROAD: u8 = 1;
const RUGGEDNESS_MAX_STEP: u8 = 2;
static RUGGEDNESS_POLICY: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(RUGGEDNESS_UNSET);

/// Process-wide selector policy. `VEARTH_LOD_RUGGEDNESS=max-step` selects the
/// previous response for comparisons; the debug panel can switch it live.
pub fn ruggedness_policy() -> RuggednessPolicy {
    let value = match RUGGEDNESS_POLICY.load(Ordering::Relaxed) {
        RUGGEDNESS_UNSET => {
            let initial = match vearth_platform::env::var("VEARTH_LOD_RUGGEDNESS").as_deref() {
                Ok("max-step") => RUGGEDNESS_MAX_STEP,
                _ => RUGGEDNESS_BROAD,
            };
            match RUGGEDNESS_POLICY.compare_exchange(
                RUGGEDNESS_UNSET,
                initial,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => initial,
                Err(current) => current,
            }
        }
        value => value,
    };
    if value == RUGGEDNESS_MAX_STEP {
        RuggednessPolicy::MaxStep
    } else {
        RuggednessPolicy::Broad
    }
}

pub fn set_ruggedness_policy(policy: RuggednessPolicy) {
    RUGGEDNESS_POLICY.store(
        match policy {
            RuggednessPolicy::Broad => RUGGEDNESS_BROAD,
            RuggednessPolicy::MaxStep => RUGGEDNESS_MAX_STEP,
        },
        Ordering::Relaxed,
    );
}

#[cfg(test)]
fn terrain_mountain_multiplier(
    cell: CellId,
    metric_lookup: Option<&TerrainMetricLookup<'_>>,
) -> f64 {
    terrain_mountain_multiplier_with(cell, metric_lookup, RuggednessPolicy::Broad)
}

fn terrain_mountain_multiplier_with(
    cell: CellId,
    metric_lookup: Option<&TerrainMetricLookup<'_>>,
    policy: RuggednessPolicy,
) -> f64 {
    let Some((metric, source)) = terrain_metric_for(cell, metric_lookup) else {
        return 1.0;
    };
    // The broad step comes from the same owned lattice as the maximum. It is
    // unavailable without spatial metadata, which keeps the scalar fallback.
    let broad_step = |target: CellId| {
        (policy == RuggednessPolicy::Broad)
            .then(|| metric.terrain_detail?.broad_step(source, target))
            .flatten()
            .filter(|step| step.is_finite() && *step >= 0.0)
    };
    if source != cell
        && let Some(range) = metric
            .terrain_detail
            .and_then(|detail| detail.query(source, cell))
        && range.min_m.is_finite()
        && range.max_m.is_finite()
        && range.step_m.is_finite()
        && range.step_m >= 0.0
    {
        if range.min_m > range.max_m {
            return 1.0;
        }
        let relief =
            (f64::from(range.max_m) - f64::from(range.min_m)).min(f64::from(f32::MAX)) as f32;
        let step = broad_step(cell).unwrap_or(range.step_m);
        return mountain_detail_multiplier(relief, step, source.sample_pitch_m());
    }
    // Missing or invalid spatial metadata keeps that same ancestor's
    // conservative scalar fallback.
    mountain_detail_multiplier(
        metric.terrain_relief_m,
        broad_step(source).unwrap_or(metric.terrain_step_m),
        source.sample_pitch_m(),
    )
}

fn padded_height_range(metric: Option<((f32, f32), f32, f32, CellId)>) -> Option<(f64, f64)> {
    let ((lo, hi), _, _, source) = metric?;
    // Own-cell records and unavailable spatial metadata keep the established
    // source-pitch heuristic; coarse expansion is still bounded below.
    height_range_with_padding((lo, hi), (source.sample_pitch_m() * 2.0).max(100.0))
}

fn padded_height_range_for(
    cell: CellId,
    metric: Option<(TerrainMetricRef<'_>, CellId)>,
) -> Option<(f64, f64)> {
    let (metric, source) = metric?;
    if source != cell
        && let Some(range) = metric
            .terrain_detail
            .and_then(|detail| detail.query(source, cell))
        && range.min_m.is_finite()
        && range.max_m.is_finite()
        && range.step_m.is_finite()
        && range.step_m >= 0.0
    {
        // Measured variation over one source interval is a vertical uncertainty
        // heuristic, not a bound on unobserved terrain. A known-empty region has
        // no measured terrain variation. Keep the entire recorded content range
        // so local terrain metadata cannot erase structures or remote extrema.
        let step = if range.min_m > range.max_m {
            0.0
        } else {
            f64::from(range.step_m)
        };
        return height_range_with_padding(metric.height_range, (step * 2.0).max(100.0));
    }
    padded_height_range(Some((
        metric.height_range,
        metric.terrain_step_m,
        metric.terrain_relief_m,
        source,
    )))
}

fn height_range_with_padding((lo, hi): (f32, f32), pad: f64) -> Option<(f64, f64)> {
    if !lo.is_finite() || !hi.is_finite() || lo > hi {
        return None;
    }
    // Bound only inferred expansion. Recorded extrema and their local safety
    // margin remain authoritative even outside the physical uncertainty limits.
    let lo = f64::from(lo);
    let hi = f64::from(hi);
    Some((
        (lo - 100.0).min((lo - pad).max(-500.0)),
        (hi + 100.0).max((hi + pad).min(9_000.0)) + 250.0,
    ))
}

#[derive(Default)]
pub(crate) struct ViewLaneDiagnostics {
    pub wanted_by_lane_level: [[u64; 32]; 4],
    pub metric_provenance: [u64; 3],
    pub global_to_known_lane: [[u64; 4]; 4],
}

/// Report-time observations on the same target IDs, without running another
/// selector or changing its cut. Lane order is visible, guard, background,
/// legacy; provenance order is the cell itself, an ancestor, unknown/invalid.
pub(crate) fn view_lane_diagnostics(
    wanted: &[WantedCell],
    cam_ecef: DVec3,
    view: &LodView,
    metric_lookup: &TerrainMetricLookup<'_>,
) -> ViewLaneDiagnostics {
    let lane_index = |lane| match lane {
        ViewLane::Visible => 0,
        ViewLane::Guard => 1,
        ViewLane::Background => 2,
        ViewLane::Legacy => 3,
    };
    let mut result = ViewLaneDiagnostics::default();
    for target in wanted {
        result.wanted_by_lane_level[lane_index(target.lane)]
            [usize::from(target.cell.level).min(31)] += 1;
        let metric = terrain_metric_for(target.cell, Some(metric_lookup));
        let envelope = padded_height_range_for(target.cell, metric);
        let provenance = match (metric, envelope) {
            (Some((_, source)), Some(_)) => usize::from(source != target.cell),
            _ => 2,
        };
        result.metric_provenance[provenance] += 1;
        let anchor = ellipsoid::surface_point_along(target.cell.center_dir());
        let (global, known) = if target.lane == ViewLane::Legacy {
            (ViewLane::Legacy, ViewLane::Legacy)
        } else {
            (
                view.diagnostic_lane_at_anchor(cam_ecef, target.cell, anchor, None),
                view.diagnostic_lane_at_anchor(cam_ecef, target.cell, anchor, envelope),
            )
        };
        result.global_to_known_lane[lane_index(global)][lane_index(known)] += 1;
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ViewLane {
    Background,
    Guard,
    Visible,
    /// Position-only selection and hand-authored tests retain their original
    /// scheduling semantics rather than receiving view-lane priority bands.
    Legacy,
}

impl ViewLane {
    fn refinement_class(self) -> u8 {
        u8::from(self != ViewLane::Background)
    }

    fn refinement_weight(self) -> f64 {
        match self {
            ViewLane::Guard => GUARD_REFINEMENT_WEIGHT,
            ViewLane::Background | ViewLane::Visible | ViewLane::Legacy => 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WantedCell {
    pub cell: CellId,
    pub screen_px: f64,
    pub(crate) lane: ViewLane,
}

impl WantedCell {
    /// Desired loading order, including the lane before raw screen error.
    pub(crate) fn stream_order(a: &Self, b: &Self) -> std::cmp::Ordering {
        b.lane
            .cmp(&a.lane)
            .then_with(|| b.screen_px.total_cmp(&a.screen_px))
            .then_with(|| a.cell.cmp(&b.cell))
    }

    /// Keep lane precedence intact in the existing scalar worker priority.
    /// The exponent gaps leave ample room for coverage-bootstrap multipliers.
    pub(crate) fn stream_priority(&self) -> f64 {
        let multiplier = match self.lane {
            ViewLane::Visible => 1e200,
            ViewLane::Guard => 1e100,
            ViewLane::Background => 1e-100,
            ViewLane::Legacy => 1.0,
        };
        self.screen_px * multiplier
    }
}

#[derive(Clone, Copy)]
struct LandmarkLock {
    region: vearth_gen::landmarks::LodRegion,
    anchor_dir: DVec3,
    target_level: u8,
}

fn landmark_locks(cam_ecef: DVec3, viewport_h: f64, fov_y: f64) -> Vec<LandmarkLock> {
    vearth_gen::landmarks::lod_regions()
        .filter_map(|region| {
            let anchor = ellipsoid::geodetic_to_ecef(region.lat_rad, region.lon_rad, 0.0);
            let up = ellipsoid::geodetic_normal(anchor);
            let center = anchor + up * region.height_m * 0.5;
            let distance = ((cam_ecef - center).length() - region.radius_m).max(1.0);
            let footprint_px =
                region.radius_m * viewport_h / (2.0 * distance * (fov_y * 0.5).tan().max(1.0e-6));
            if footprint_px < 2.0 {
                return None;
            }
            let mut target_level = vearth_gen::landmarks::LANDMARK_MIN_LEVEL;
            let region_max = STANDARD_MAX_STREAM_LEVEL + region.detail_level_bias;
            while target_level < region_max
                && lod::sample_screen_px(
                    vearth_core::cell::sample_pitch_m(target_level),
                    distance,
                    viewport_h,
                    fov_y,
                ) > TAU_PX
            {
                target_level += 1;
            }
            Some(LandmarkLock {
                region,
                anchor_dir: anchor.normalize(),
                target_level,
            })
        })
        .collect()
}

/// The coherent landmark level intersecting `cell`, if any. The angular
/// test is exact and cheap; the oriented EN rectangle prevents a long bridge
/// from locking a kilometer-radius disk of unrelated terrain.
fn landmark_level(cell: CellId, locks: &[LandmarkLock]) -> Option<u8> {
    let cell_dir = cell.center_dir();
    let cell_radius = cell.angular_radius();
    locks
        .iter()
        .filter(|lock| {
            let angular = cell_dir.dot(lock.anchor_dir).clamp(-1.0, 1.0).acos();
            if angular > cell_radius + lock.region.radius_m / ellipsoid::MEAN_RADIUS {
                return false;
            }

            let center = ellipsoid::surface_point_along(cell_dir);
            let (lat, lon, _) = ellipsoid::ecef_to_geodetic(center);
            let mut dlon = lon - lock.region.lon_rad;
            if dlon > std::f64::consts::PI {
                dlon -= std::f64::consts::TAU;
            } else if dlon < -std::f64::consts::PI {
                dlon += std::f64::consts::TAU;
            }
            let east = dlon * lock.region.lat_rad.cos() * ellipsoid::MEAN_RADIUS;
            let north = (lat - lock.region.lat_rad) * ellipsoid::MEAN_RADIUS;
            let (sin_h, cos_h) = lock.region.heading_rad.sin_cos();
            let mx = east * cos_h - north * sin_h;
            let mz = east * sin_h + north * cos_h;
            let pad = cell.size_m() * 0.8;
            mx >= lock.region.bbox_min_xz[0] - pad
                && mx <= lock.region.bbox_max_xz[0] + pad
                && mz >= lock.region.bbox_min_xz[1] - pad
                && mz <= lock.region.bbox_max_xz[1] + pad
        })
        .map(|lock| lock.target_level)
        .max()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WantedStats {
    /// Whether quality selection hit its budget before background coalescing.
    pub budget_saturated: bool,
    pub split_cells: usize,
    pub retained_splits: usize,
    pub topology_changes: usize,
}

/// Priority-driven SSE refinement from the 6 face roots: always split the
/// cell with the worst screen error first, so when the budget runs out the
/// remaining error is spread evenly instead of leaving giant coarse slabs
/// wherever a depth-first walk happened not to reach.
///
/// `prev_split` is the split set returned by the previous call (empty on
/// the first); cells in it merge back only below `TAU_PX * TAU_MERGE_RATIO`.
pub fn compute_wanted(
    cam_ecef: DVec3,
    viewport_h: f64,
    fov_y: f64,
    prev_split: &HashSet<CellId>,
) -> (Vec<WantedCell>, HashSet<CellId>) {
    let (wanted, split, _) = compute_wanted_internal(
        cam_ecef, viewport_h, fov_y, prev_split, None, None, MAX_WANTED,
    );
    (wanted, split)
}

/// View-aware variant used for the presented cut. Visible error receives full
/// weight and the turn guard stays close behind. Background then coalesces to
/// compatible coverage without reallocating quality to visible targets.
pub fn compute_wanted_for_view(
    cam_ecef: DVec3,
    viewport_h: f64,
    view: &LodView,
    prev_split: &HashSet<CellId>,
) -> (Vec<WantedCell>, HashSet<CellId>) {
    let (wanted, split, _) =
        compute_wanted_for_view_with_stats(cam_ecef, viewport_h, view, prev_split);
    (wanted, split)
}

pub fn compute_wanted_for_view_with_stats(
    cam_ecef: DVec3,
    viewport_h: f64,
    view: &LodView,
    prev_split: &HashSet<CellId>,
) -> (Vec<WantedCell>, HashSet<CellId>, WantedStats) {
    let fov_y = 2.0 * view.tan_half_fov_y.atan();
    compute_wanted_internal(
        cam_ecef,
        viewport_h,
        fov_y,
        prev_split,
        Some(view),
        None,
        MAX_VIEW_WANTED,
    )
}

pub fn compute_wanted_for_view_with_stats_and_metric_lookup(
    cam_ecef: DVec3,
    viewport_h: f64,
    view: &LodView,
    prev_split: &HashSet<CellId>,
    metric: &TerrainMetricLookup<'_>,
) -> (Vec<WantedCell>, HashSet<CellId>, WantedStats) {
    let fov_y = 2.0 * view.tan_half_fov_y.atan();
    compute_wanted_internal(
        cam_ecef,
        viewport_h,
        fov_y,
        prev_split,
        Some(view),
        Some(metric),
        MAX_VIEW_WANTED,
    )
}

pub fn compute_wanted_with_stats(
    cam_ecef: DVec3,
    viewport_h: f64,
    fov_y: f64,
    prev_split: &HashSet<CellId>,
) -> (Vec<WantedCell>, HashSet<CellId>, WantedStats) {
    compute_wanted_internal(
        cam_ecef, viewport_h, fov_y, prev_split, None, None, MAX_WANTED,
    )
}

pub fn compute_wanted_with_stats_and_metric_lookup(
    cam_ecef: DVec3,
    viewport_h: f64,
    fov_y: f64,
    prev_split: &HashSet<CellId>,
    metric: &TerrainMetricLookup<'_>,
) -> (Vec<WantedCell>, HashSet<CellId>, WantedStats) {
    compute_wanted_internal(
        cam_ecef,
        viewport_h,
        fov_y,
        prev_split,
        None,
        Some(metric),
        MAX_WANTED,
    )
}

/// Mutable leaf lookup used while the topology balancer splits coarse cells.
///
/// Exact-address lookup makes ancestor coverage cheap. The ordered range
/// index also finds finer leaves along a neighboring edge, avoiding a full
/// leaf-set rebuild and rescan after every balancing round.
struct LodLeafIndex {
    by_cell: HashMap<CellKey, usize>,
    by_range_start: BTreeMap<u64, CellKey>,
}

impl LodLeafIndex {
    fn new(leaves: &[WantedCell]) -> Self {
        let mut index = Self {
            by_cell: HashMap::with_capacity(leaves.len()),
            by_range_start: BTreeMap::new(),
        };
        for (position, wanted) in leaves.iter().enumerate() {
            index.insert(wanted.cell, position);
        }
        index
    }

    fn contains(&self, cell: CellId) -> bool {
        self.by_cell.contains_key(&CellKey::from(cell))
    }

    fn insert(&mut self, cell: CellId, position: usize) {
        let key = CellKey::from(cell);
        let previous_position = self.by_cell.insert(key, position);
        let previous_range = self.by_range_start.insert(key.range().start(), key);
        debug_assert!(previous_position.is_none());
        debug_assert!(previous_range.is_none());
    }

    fn remove(&mut self, cell: CellId) -> Option<usize> {
        let key = CellKey::from(cell);
        let position = self.by_cell.remove(&key)?;
        let range_entry = self.by_range_start.remove(&key.range().start());
        debug_assert_eq!(range_entry, Some(key));
        Some(position)
    }

    fn update_position(&mut self, cell: CellId, position: usize) {
        *self
            .by_cell
            .get_mut(&CellKey::from(cell))
            .expect("moved LOD leaf must remain indexed") = position;
    }

    /// Leaves covering any part of `cell`. A valid cut has either one
    /// covering ancestor or a set of non-overlapping descendants.
    fn covering_leaves(&self, cell: CellId, out: &mut Vec<CellId>) {
        out.clear();
        let mut ancestor = Some(cell);
        while let Some(candidate) = ancestor {
            if self.contains(candidate) {
                out.push(candidate);
                return;
            }
            ancestor = candidate.parent();
        }

        let range = CellKey::from(cell).range();
        out.extend(
            self.by_range_start
                .range(range.start()..range.end())
                .map(|(_, key)| key.cell()),
        );
    }

    fn adjacent_leaves(&self, cell: CellId, di: i64, dj: i64, out: &mut Vec<CellId>) {
        let edge = 1i64 << cell.level;
        let i = i64::from(cell.i) + di;
        let j = i64::from(cell.j) + dj;
        if i < 0 || j < 0 || i >= edge || j >= edge {
            out.clear();
            return;
        }
        let neighbor = CellId {
            face: cell.face,
            level: cell.level,
            i: i as u32,
            j: j as u32,
        };
        self.covering_leaves(neighbor, out);
        out.retain(|leaf| {
            if leaf.level <= neighbor.level {
                return true;
            }
            let shift = leaf.level - neighbor.level;
            match (di, dj) {
                (-1, 0) => leaf.i + 1 == (neighbor.i + 1) << shift,
                (1, 0) => leaf.i == neighbor.i << shift,
                (0, -1) => leaf.j + 1 == (neighbor.j + 1) << shift,
                (0, 1) => leaf.j == neighbor.j << shift,
                _ => unreachable!("only cardinal LOD neighbors are queried"),
            }
        });
    }
}

fn collect_balance_candidates(
    index: &LodLeafIndex,
    dirty: impl IntoIterator<Item = CellId>,
    candidates: &mut BTreeSet<CellId>,
    adjacent: &mut Vec<CellId>,
) {
    for cell in dirty {
        if !index.contains(cell) {
            continue;
        }
        for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            index.adjacent_leaves(cell, di, dj, adjacent);
            for &neighbor in adjacent.iter() {
                let (coarse, fine) = if cell.level <= neighbor.level {
                    (cell, neighbor)
                } else {
                    (neighbor, cell)
                };
                if coarse.level + 1 < fine.level {
                    candidates.insert(coarse);
                }
            }
        }
    }
}

/// Keep cube-face edge leaves unchanged: the current adjacency index is
/// same-face, so it cannot certify a new level difference across a cube seam.
fn background_merge_preserves_balance(cell: CellId, index: &LodLeafIndex) -> bool {
    let edge = cell.edge_cells();
    if cell.i == 0 || cell.j == 0 || cell.i + 1 == edge || cell.j + 1 == edge {
        return false;
    }
    for (di, dj, touching) in [
        (-1_i64, 0_i64, [1, 3]),
        (1, 0, [0, 2]),
        (0, -1, [2, 3]),
        (0, 1, [0, 1]),
    ] {
        let neighbor = CellId {
            i: (i64::from(cell.i) + di) as u32,
            j: (i64::from(cell.j) + dj) as u32,
            ..cell
        };
        let children = neighbor.children();
        for child in touching.map(|i| children[i]) {
            let range = CellKey::from(child).range();
            // A leaf at this half-edge's level or a covering ancestor is safe.
            // Any descendant in its range conservatively blocks merging. This
            // also refuses partially represented horizon regions without
            // scanning all of their descendants to infer edge coverage.
            if index
                .by_range_start
                .range(range.start()..range.end())
                .next()
                .is_some_and(|(_, key)| key.cell().level > cell.level + 1)
            {
                return false;
            }
        }
    }
    true
}

/// Background provides coverage and compatible boundaries, rather than
/// consuming spare quality budget. Complete sibling unions preserve the
/// original geographic footprint; omitted horizon quadrants never get filled.
fn coalesce_background_cut(
    measure: &impl Fn(CellId) -> (f64, ViewLane, Option<u8>),
    out: &mut Vec<WantedCell>,
    split: &mut HashSet<CellId>,
) -> usize {
    let mut index = LodLeafIndex::new(out);
    let mut slots: Vec<_> = out.iter().copied().map(Some).collect();
    let mut candidates: BTreeSet<_> = out
        .iter()
        .filter_map(|wanted| wanted.cell.parent())
        .filter(|parent| parent.level >= MIN_RENDER_LEVEL)
        .map(|parent| (std::cmp::Reverse(parent.level), parent))
        .collect();
    let mut merged = 0;
    // Deepest first means any neighbor fine enough to block this merge has
    // already had its own opportunity to merge. Each original leaf proposes
    // one parent and each successful merge proposes one more: at most N+N/3
    // candidates, each with four child lookups and eight ordered range probes.
    while let Some((_, parent)) = candidates.pop_first() {
        let children = parent.children();
        let positions = children.map(|child| index.by_cell.get(&CellKey::from(child)).copied());
        let [Some(a), Some(b), Some(c), Some(d)] = positions else {
            continue;
        };
        let positions = [a, b, c, d];
        if positions.iter().any(|&position| {
            slots[position].expect("indexed leaf owns a live slot").lane != ViewLane::Background
        }) {
            continue;
        }
        let (screen_px, lane, landmark) = measure(parent);
        if lane != ViewLane::Background
            || landmark.is_some()
            || children.iter().any(|&child| measure(child).2.is_some())
            || !background_merge_preserves_balance(parent, &index)
        {
            continue;
        }
        let first = *positions.iter().min().unwrap();
        for (child, position) in children.into_iter().zip(positions) {
            index.remove(child).expect("merged child remains indexed");
            slots[position] = None;
        }
        // Stable slots retain every untouched target's relative draw/priority
        // order. The replacement occupies its earliest child's position.
        slots[first] = Some(WantedCell {
            cell: parent,
            screen_px,
            lane,
        });
        index.insert(parent, first);
        split.remove(&parent);
        merged += 1;
        if let Some(ancestor) = parent
            .parent()
            .filter(|cell| cell.level >= MIN_RENDER_LEVEL)
        {
            candidates.insert((std::cmp::Reverse(ancestor.level), ancestor));
        }
    }
    out.clear();
    out.extend(slots.into_iter().flatten());
    merged
}

/// Split coarse same-face neighbors until every represented edge differs by
/// at most one level. This is a topology-only pass: it consumes the reserved
/// part of the wanted budget and never refines a cell pinned to a coherent
/// landmark presentation level.
fn balance_cut(
    cam_ecef: DVec3,
    measure: &impl Fn(CellId) -> (f64, ViewLane, Option<u8>),
    out: &mut Vec<WantedCell>,
    split: &mut HashSet<CellId>,
    max_wanted: usize,
) {
    let mut index = LodLeafIndex::new(out);
    let horizon = lod::HorizonCuller::new(cam_ecef);
    let mut dirty: VecDeque<_> = out.iter().map(|wanted| wanted.cell).collect();
    let mut candidates = BTreeSet::new();
    let mut adjacent = Vec::new();
    loop {
        collect_balance_candidates(&index, dirty.drain(..), &mut candidates, &mut adjacent);
        if candidates.is_empty() {
            break;
        }

        let mut changed = false;
        let round = std::mem::take(&mut candidates);
        for coarse in round {
            if !index.contains(coarse) {
                continue;
            }
            let (_, _, landmark_level) = measure(coarse);
            if landmark_level.is_some_and(|target| coarse.level >= target) {
                dirty.push_back(coarse);
                continue;
            }
            let children: Vec<_> = coarse
                .children()
                .into_iter()
                .filter(|child| !horizon.culled(*child))
                .collect();
            if children.is_empty() {
                dirty.push_back(coarse);
                continue;
            }
            let growth = children.len().saturating_sub(1);
            if out.len() + growth > max_wanted {
                dirty.push_back(coarse);
                continue;
            }
            let Some(position) = index.remove(coarse) else {
                continue;
            };
            out.swap_remove(position);
            if position < out.len() {
                index.update_position(out[position].cell, position);
            }
            split.insert(coarse);
            for child in children {
                let (screen_px, lane, _) = measure(child);
                let position = out.len();
                out.push(WantedCell {
                    cell: child,
                    screen_px,
                    lane,
                });
                index.insert(child, position);
                dirty.push_back(child);
            }
            changed = true;
        }
        if !changed {
            break;
        }
    }
}

#[cfg(test)]
fn balance_cut_reference(
    cam_ecef: DVec3,
    measure: &impl Fn(CellId) -> (f64, ViewLane, Option<u8>),
    out: &mut Vec<WantedCell>,
    split: &mut HashSet<CellId>,
    max_wanted: usize,
) {
    loop {
        let leaves: HashSet<CellId> = out.iter().map(|wanted| wanted.cell).collect();
        let mut candidates = BTreeSet::new();
        for fine in leaves.iter().copied() {
            if fine.level <= MIN_RENDER_LEVEL + 1 {
                continue;
            }
            let n = 1i64 << fine.level;
            for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let i = fine.i as i64 + di;
                let j = fine.j as i64 + dj;
                if i < 0 || j < 0 || i >= n || j >= n {
                    continue;
                }
                let mut probe = CellId {
                    face: fine.face,
                    level: fine.level,
                    i: i as u32,
                    j: j as u32,
                };
                loop {
                    if leaves.contains(&probe) {
                        if probe.level + 1 < fine.level {
                            candidates.insert(probe);
                        }
                        break;
                    }
                    let Some(parent) = probe.parent() else {
                        break;
                    };
                    probe = parent;
                }
            }
        }
        if candidates.is_empty() {
            break;
        }

        let mut changed = false;
        for coarse in candidates {
            let (_, _, landmark_level) = measure(coarse);
            if landmark_level.is_some_and(|target| coarse.level >= target) {
                continue;
            }
            let children: Vec<_> = coarse
                .children()
                .into_iter()
                .filter(|child| !lod::horizon_culled(cam_ecef, *child))
                .collect();
            if children.is_empty() {
                continue;
            }
            let growth = children.len().saturating_sub(1);
            if out.len() + growth > max_wanted {
                continue;
            }
            let Some(index) = out.iter().position(|wanted| wanted.cell == coarse) else {
                continue;
            };
            out.swap_remove(index);
            split.insert(coarse);
            for child in children {
                let (screen_px, lane, _) = measure(child);
                out.push(WantedCell {
                    cell: child,
                    screen_px,
                    lane,
                });
            }
            changed = true;
        }
        if !changed {
            break;
        }
    }
}

fn compute_wanted_internal(
    cam_ecef: DVec3,
    viewport_h: f64,
    fov_y: f64,
    prev_split: &HashSet<CellId>,
    view: Option<&LodView>,
    metric_lookup: Option<&TerrainMetricLookup<'_>>,
    max_wanted: usize,
) -> (Vec<WantedCell>, HashSet<CellId>, WantedStats) {
    compute_wanted_internal_with_background_policy(
        cam_ecef,
        viewport_h,
        fov_y,
        prev_split,
        view,
        metric_lookup,
        max_wanted,
        view.is_none_or(|view| view.coalesce_background),
    )
}

fn compute_wanted_internal_with_background_policy(
    cam_ecef: DVec3,
    viewport_h: f64,
    fov_y: f64,
    prev_split: &HashSet<CellId>,
    view: Option<&LodView>,
    metric_lookup: Option<&TerrainMetricLookup<'_>>,
    max_wanted: usize,
    coalesce_background: bool,
) -> (Vec<WantedCell>, HashSet<CellId>, WantedStats) {
    compute_wanted_with_measurement_cache::<true>(
        cam_ecef,
        viewport_h,
        fov_y,
        prev_split,
        view,
        metric_lookup,
        max_wanted,
        coalesce_background,
    )
}

fn compute_wanted_with_measurement_cache<const CACHED: bool>(
    cam_ecef: DVec3,
    viewport_h: f64,
    fov_y: f64,
    prev_split: &HashSet<CellId>,
    view: Option<&LodView>,
    metric_lookup: Option<&TerrainMetricLookup<'_>>,
    max_wanted: usize,
    coalesce_background: bool,
) -> (Vec<WantedCell>, HashSet<CellId>, WantedStats) {
    struct Entry {
        px: f64,
        rank_px: f64,
        retained: bool,
        lane: ViewLane,
        landmark_level: Option<u8>,
        cell: CellId,
    }
    impl PartialEq for Entry {
        fn eq(&self, other: &Self) -> bool {
            self.rank_px.total_cmp(&other.rank_px) == std::cmp::Ordering::Equal
                && self.retained == other.retained
                && self.lane == other.lane
                && self.landmark_level == other.landmark_level
                && self.cell == other.cell
        }
    }
    impl Eq for Entry {}
    impl PartialOrd for Entry {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Entry {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.lane
                .refinement_class()
                .cmp(&other.lane.refinement_class())
                .then_with(|| self.rank_px.total_cmp(&other.rank_px))
                .then_with(|| self.lane.cmp(&other.lane))
                .then_with(|| self.retained.cmp(&other.retained))
                .then_with(|| other.cell.cmp(&self.cell))
        }
    }

    let locks = landmark_locks(cam_ecef, viewport_h, fov_y);

    let height_range = |cell: CellId| {
        padded_height_range_for(cell, terrain_metric_for(cell, metric_lookup))
            .unwrap_or((-500.0, 1_500.0))
    };
    // Evidence and camera state are immutable for one selection. Neighbor
    // halos repeatedly request the same source chains and spatial summaries;
    // balance and background merging also revisit already measured cells.
    // Keep these caches local so later source arrivals cannot reuse old data.
    let mountain_cache = std::cell::RefCell::new(HashMap::new());
    let ruggedness = ruggedness_policy();
    let local_mountain_multiplier = |cell: CellId| {
        if CACHED && metric_lookup.is_some() {
            *mountain_cache
                .borrow_mut()
                .entry(CellKey::from(cell))
                .or_insert_with(|| {
                    terrain_mountain_multiplier_with(cell, metric_lookup, ruggedness)
                })
        } else {
            terrain_mountain_multiplier_with(cell, metric_lookup, ruggedness)
        }
    };
    let mountain_multiplier = |cell: CellId| {
        let mut multiplier = local_mountain_multiplier(cell);
        let n = 1i64 << cell.level;
        for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
            let i = i64::from(cell.i) + di;
            let j = i64::from(cell.j) + dj;
            if i < 0 || j < 0 || i >= n || j >= n {
                continue;
            }
            let neighbor = CellId {
                face: cell.face,
                level: cell.level,
                i: i as u32,
                j: j as u32,
            };
            let neighbor_multiplier = local_mountain_multiplier(neighbor);
            multiplier = multiplier.max(1.0 + (neighbor_multiplier - 1.0) * MOUNTAIN_HALO_WEIGHT);
        }
        multiplier
    };

    let measure_uncached = |cell: CellId| {
        let anchor = ellipsoid::surface_point_along(cell.center_dir());
        // Measure distance to the cell's known vertical content range so
        // aerial views do not collapse to horizontal distance.
        let up = anchor.normalize();
        let (height_lo, height_hi) = height_range(cell);
        let alt_along = (cam_ecef - anchor).dot(up).clamp(height_lo, height_hi);
        let content = anchor + up * alt_along;
        let half_diag = cell.size_m() * 0.75;
        let d = ((cam_ecef - content).length() - half_diag).max(1.0);
        let base_px = lod::sample_screen_px(cell.sample_pitch_m(), d, viewport_h, fov_y);
        let lane = view.map_or(ViewLane::Legacy, |view| {
            view.lane_at_anchor(cam_ecef, cell, anchor)
        });
        // Spend the existing screen-error budget on mountain shape. Background
        // terrain stays cheap; visible and turn-guard cells gain up to one
        // extra level, with a softer one-cell halo around rugged metrics.
        let px = if lane == ViewLane::Background {
            base_px
        } else {
            base_px * mountain_multiplier(cell)
        };
        (px, lane, landmark_level(cell, &locks))
    };
    let measurement_cache = std::cell::RefCell::new(HashMap::new());
    let measure = |cell: CellId| {
        if CACHED && metric_lookup.is_some() {
            *measurement_cache
                .borrow_mut()
                .entry(CellKey::from(cell))
                .or_insert_with(|| measure_uncached(cell))
        } else {
            measure_uncached(cell)
        }
    };

    let horizon = lod::HorizonCuller::new(cam_ecef);
    let mut heap = std::collections::BinaryHeap::new();
    let seed = |cell: CellId, heap: &mut std::collections::BinaryHeap<Entry>| {
        if !horizon.culled(cell) {
            let (px, lane, landmark_level) = measure(cell);
            let retained = prev_split.contains(&cell);
            let forced_landmark = landmark_level.is_some_and(|target| cell.level < target);
            heap.push(Entry {
                px,
                rank_px: if forced_landmark {
                    f64::INFINITY
                } else {
                    px * lane.refinement_weight()
                },
                retained,
                lane,
                landmark_level,
                cell,
            });
        }
    };
    for f in 0..6 {
        let root = CellId::root(Face(f));
        // Descend unconditionally to the minimum render level.
        let mut stack = vec![root];
        while let Some(c) = stack.pop() {
            if c.level < MIN_RENDER_LEVEL {
                stack.extend(c.children());
            } else {
                seed(c, &mut heap);
            }
        }
    }

    let mut out = Vec::new();
    let mut split = HashSet::new();
    let mut budget_saturated = false;
    let mut retained_splits = 0usize;
    while let Some(Entry {
        px,
        retained,
        lane,
        landmark_level,
        cell,
        ..
    }) = heap.pop()
    {
        let tau = if prev_split.contains(&cell) {
            TAU_PX * TAU_MERGE_RATIO
        } else {
            TAU_PX
        };
        let forced_landmark = landmark_level.is_some_and(|target| cell.level < target);
        let splittable = if let Some(target) = landmark_level {
            cell.level < target
        } else {
            px > tau && cell.level < STANDARD_MAX_STREAM_LEVEL
        };
        let refinement_limit = max_wanted.saturating_sub(BALANCE_RESERVE);
        if splittable && (forced_landmark || out.len() + heap.len() + 4 <= refinement_limit) {
            split.insert(cell);
            retained_splits += retained as usize;
            for ch in cell.children() {
                seed(ch, &mut heap);
            }
        } else {
            budget_saturated |= splittable && !forced_landmark;
            out.push(WantedCell {
                cell,
                screen_px: px,
                lane,
            });
        }
    }
    balance_cut(cam_ecef, &measure, &mut out, &mut split, max_wanted);
    if coalesce_background && view.is_some() {
        coalesce_background_cut(&measure, &mut out, &mut split);
        retained_splits = split.intersection(prev_split).count();
    }
    let topology_changes = split.symmetric_difference(prev_split).count();
    let stats = WantedStats {
        budget_saturated,
        split_cells: split.len(),
        retained_splits,
        topology_changes,
    };
    (out, split, stats)
}

/// Generate+mesh worker pool with a **replaceable priority queue**.
///
/// The app calls `sync_queue` every frame with the currently-missing cells
/// (priority = screen-space error). The desired set is reconciled by revision:
/// work that stopped being relevant — intermediate views from a fast pan or
/// an interrupted dive — is removed before admission or canceled while it
/// awaits providers. A later high-priority cell can overtake queued work.
///
/// With `async-providers`, a bounded Tokio lane prepares memory caches before
/// the unchanged synchronous generator runs on bounded CPU threads. The
/// legacy/no-default path retains blocking acquisition for rollback.
#[derive(Clone, Debug, PartialEq)]
pub struct CellPlan {
    pub cell: CellId,
    pub terrain_tiles: Vec<(u8, u32, u32)>,
    pub vector_bounds: Option<(f64, f64, f64, f64)>,
    pub night_building_bounds: Option<(f64, f64, f64, f64)>,
    pub footprint: vearth_gen::SourceFootprint,
    pub priority: f64,
}

impl CellPlan {
    pub fn for_cell(cell: CellId, has_vectors: bool, priority: f64) -> Self {
        CellPlan {
            cell,
            terrain_tiles: vearth_gen::required_tiles(cell),
            vector_bounds: (has_vectors && cell.level >= vearth_gen::tier0::WATER_MIN_LEVEL)
                .then(|| vearth_gen::vector_query_bounds(cell)),
            night_building_bounds: None,
            footprint: vearth_gen::source_query_footprint(cell),
            priority: normalized_priority(priority),
        }
    }

    /// Warm known dependencies for blocking providers. Generation checks the
    /// status of each sample, including failures already held as tombstones.
    fn acquire(&self, registry: &Registry) {
        use vearth_data::ElevationSource as _;
        use vearth_data::vectors::VectorSource as _;
        vearth_platform::thread::scope(|scope| {
            scope.spawn(|| registry.prefetch(&self.terrain_tiles));
            if let Some((lat0, lon0, half_e, half_n)) = self.vector_bounds {
                scope.spawn(move || registry.prefetch_vectors(lat0, lon0, half_e, half_n));
            }
            if let Some((lat0, lon0, half_e, half_n)) = self.night_building_bounds {
                scope.spawn(move || registry.prefetch_night_buildings(lat0, lon0, half_e, half_n));
            }
        });
    }

    #[cfg(feature = "async-providers")]
    async fn acquire_async(&self, registry: &Registry) -> AcquiredSources {
        let request = vearth_data::async_prep::PrepareRequest {
            terrain_tiles: self.terrain_tiles.clone(),
            footprint: vearth_data::async_prep::SourceFootprint {
                center_lat: self.footprint.center_lat,
                center_lon: self.footprint.center_lon,
                radius_rad: self.footprint.radius_rad,
                sample_hint_m: self.footprint.sample_hint_m,
            },
            vector_bounds: self.vector_bounds,
            night_building_bounds: self.night_building_bounds,
            priority: self.priority,
        };
        let mut raster_complete = true;
        let mut vector_complete = true;
        let mut projection_complete = true;
        let mut geometry_complete = [true; 2];
        let mut features = None;
        // Vector projection needs only vector dependencies. Discover and warm
        // geometry datums while the independent initial rasters are pending;
        // the join keeps every completed branch's leases until CPU handoff.
        let raster = Box::pin(async {
            let prepared = registry.prepare_raster_retained_async(&request).await;
            raster_complete = prepared.summary.complete();
            prepared
        }) as vearth_data::async_prep::RetainedPrepareFuture<'_>;
        let vector = Box::pin(async {
            let mut prepared = registry.prepare_vector_retained_async(&request).await;
            vector_complete = prepared.summary.complete();
            if let Some((lat, lon, half_e, half_n)) = self.vector_bounds {
                use vearth_data::vectors::VectorSource as _;
                let pack = registry.features(lat, lon, half_e, half_n);
                projection_complete = pack.complete;
                let (geometry, complete) = self.acquire_geometry_async(registry, &pack).await;
                geometry_complete = complete;
                prepared.combine(geometry);
                // An incomplete projection must still be retried on the CPU
                // lane: another source flight may recover its tiles meanwhile.
                if pack.complete {
                    features = Some(pack);
                }
            }
            prepared
        }) as vearth_data::async_prep::RetainedPrepareFuture<'_>;
        let prepared = vearth_data::async_prep::join_retained(vec![raster, vector]).await;
        let complete = prepared.summary.complete() && projection_complete;
        let stage_complete = [
            raster_complete && vector_complete,
            projection_complete,
            geometry_complete[0],
            geometry_complete[1],
        ];
        vearth_data::metrics::record_preparation_completeness(self.cell.level, stage_complete);
        AcquiredSources {
            complete,
            features,
            retained: prepared,
        }
    }

    #[cfg(feature = "async-providers")]
    async fn acquire_geometry_async(
        &self,
        registry: &Registry,
        pack: &FeaturePack,
    ) -> (vearth_data::async_prep::PreparedSources, [bool; 2]) {
        use std::sync::atomic::{AtomicBool, Ordering};
        // Datum footprints depend only on the projected vectors. Poll every
        // cap together, retaining completed leases while others still wait
        // and preserving each stage result. Geometry datums arrive as many
        // small caps near the features that sample them.
        let geometry_complete = AtomicBool::new(true);
        let roof_complete = AtomicBool::new(true);
        let geometry = vearth_gen::built_surface::geometry_source_footprints(self.cell, pack)
            .into_iter()
            .map(|footprint| (footprint, false, &geometry_complete));
        let roof = vearth_gen::built_surface::roof_source_footprint(self.cell, pack)
            .into_iter()
            .map(|footprint| (footprint, true, &roof_complete));
        let futures = geometry
            .chain(roof)
            .map(|(footprint, surface_only, stage_complete)| {
                Box::pin(async move {
                    let request = vearth_data::async_prep::PrepareRequest {
                        terrain_tiles: Vec::new(),
                        footprint: vearth_data::async_prep::SourceFootprint {
                            center_lat: footprint.center_lat,
                            center_lon: footprint.center_lon,
                            radius_rad: footprint.radius_rad,
                            sample_hint_m: footprint.sample_hint_m,
                        },
                        vector_bounds: None,
                        night_building_bounds: None,
                        priority: self.priority,
                    };
                    let prepared = registry
                        .prepare_geometry_retained_async(&request, surface_only)
                        .await;
                    if !prepared.summary.complete() {
                        stage_complete.store(false, Ordering::Relaxed);
                    }
                    prepared
                }) as vearth_data::async_prep::RetainedPrepareFuture<'_>
            })
            .collect();
        let prepared = vearth_data::async_prep::join_retained(futures).await;
        (
            prepared,
            [geometry_complete.into_inner(), roof_complete.into_inner()],
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStageCounts {
    pub cache_check: usize,
    pub acquiring: usize,
    pub cpu: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellStage {
    CacheCheck,
    Acquiring,
    Ready,
    Cpu,
}

pub struct GenPool {
    shared: Arc<PoolShared>,
    rx_done: Receiver<Completion>,
    /// Kept for HUD health queries (workers hold their own clones).
    registry: Arc<Registry>,
    cache: Option<Arc<ChunkCache>>,
    staged_pipeline: bool,
    #[cfg(feature = "async-providers")]
    _acquisition_runtime: Option<vearth_platform::rt::runtime::Runtime>,
    #[cfg(feature = "async-providers")]
    async_acquisition: bool,
}

struct PoolShared {
    state: std::sync::Mutex<PoolState>,
    cv: std::sync::Condvar,
    ready_cv: std::sync::Condvar,
    #[cfg(feature = "async-providers")]
    desired_version: tokio::sync::watch::Sender<u64>,
    stop: std::sync::atomic::AtomicBool,
    generated: std::sync::atomic::AtomicU64,
    cache_hits: std::sync::atomic::AtomicU64,
}

#[derive(Default)]
struct PoolState {
    pending: std::collections::BinaryHeap<Job>,
    /// Latest live revision per queued cell. Older heap entries are lazily
    /// discarded, allowing O(changed intents) queue updates.
    pending_set: HashMap<CellId, (f64, u64)>,
    /// Active generation token per cell. Carrying the pending revision into
    /// every staged message prevents a canceled run from touching a newer run
    /// of the same cell (the classic cell-id ABA race).
    running: HashMap<CellId, u64>,
    desired: HashSet<CellId>,
    desired_priorities: HashMap<CellId, f64>,
    ready: Vec<CpuJob>,
    /// Physical pre-CPU ownership outlives intent invalidation. An old task's
    /// reservation can only retire its own revision, never a newer cell run.
    staging: HashSet<(CellId, u64)>,
    staging_limit: Option<usize>,
    stages: HashMap<CellId, CellStage>,
    /// Bound deterministic geometry failures while an intent remains live.
    /// Leaving the desired set or explicitly invalidating generation resets it.
    failures: HashMap<CellId, u8>,
    next_revision: u64,
}

const MAX_GEOMETRY_ATTEMPTS: u8 = 2;

struct Job {
    priority: f64,
    cell: CellId,
    revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorkItem {
    cell: CellId,
    revision: u64,
    priority: f64,
}

struct AcquireJob {
    plan: CellPlan,
    work: WorkItem,
}

struct AcquiredSources {
    complete: bool,
    #[cfg(feature = "async-providers")]
    retained: vearth_data::async_prep::PreparedSources,
    // Retain the exact projected source window used to prepare geometry datums.
    // Generation applies its normal admission and landmark suppression once.
    features: Option<FeaturePack>,
}

enum CpuJob {
    Cached {
        column: SurfaceCell,
        work: WorkItem,
    },
    Prepared {
        work: WorkItem,
        acquired: AcquiredSources,
    },
}

impl CpuJob {
    fn work(&self) -> WorkItem {
        match self {
            CpuJob::Cached { work, .. } | CpuJob::Prepared { work, .. } => *work,
        }
    }
}

struct Completion {
    work: WorkItem,
    prepared: PreparedRenderCell,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.priority.total_cmp(&other.priority) == std::cmp::Ordering::Equal
            && self.cell == other.cell
            && self.revision == other.revision
    }
}
impl Eq for Job {}
impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Job {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .total_cmp(&other.priority)
            // Stable address tie-break avoids scheduler changes from input or
            // HashMap iteration order.
            .then_with(|| other.cell.cmp(&self.cell))
            .then_with(|| self.revision.cmp(&other.revision))
    }
}

impl GenPool {
    pub fn new(registry: Arc<Registry>, cache: Option<Arc<ChunkCache>>, workers: usize) -> Self {
        Self::with_async_acquisition_slots(registry, cache, workers, workers)
    }

    /// `async_slots` bounds cell plans acquiring concurrently when every
    /// provider is async; blocking acquisition always uses `workers` threads.
    pub fn with_async_acquisition_slots(
        registry: Arc<Registry>,
        cache: Option<Arc<ChunkCache>>,
        workers: usize,
        async_slots: usize,
    ) -> Self {
        let workers = workers.max(1);
        // Bounded handoff: when rendering cannot admit meshes quickly enough,
        // generation workers pause here instead of growing an unbounded
        // completion convoy in process memory.
        let (tx_done, rx_done) = sync_channel::<Completion>(workers * 2);
        #[cfg(feature = "async-providers")]
        let (desired_version, _) = tokio::sync::watch::channel(0u64);
        let shared = Arc::new(PoolShared {
            state: std::sync::Mutex::new(PoolState::default()),
            cv: std::sync::Condvar::new(),
            ready_cv: std::sync::Condvar::new(),
            #[cfg(feature = "async-providers")]
            desired_version,
            stop: std::sync::atomic::AtomicBool::new(false),
            generated: std::sync::atomic::AtomicU64::new(0),
            cache_hits: std::sync::atomic::AtomicU64::new(0),
        });
        let staged_pipeline = vearth_platform::env::var("VEARTH_PIPELINE")
            .map(|value| !value.eq_ignore_ascii_case("legacy"))
            .unwrap_or(true);
        #[cfg(feature = "async-providers")]
        let mut acquisition_runtime = None;
        #[cfg(feature = "async-providers")]
        let mut async_acquisition_enabled = false;
        if staged_pipeline {
            let cpu_workers = vearth_platform::thread::available_parallelism()
                .map(|parallelism| parallelism.get().saturating_sub(2))
                .unwrap_or(4)
                .clamp(2, 8);
            let cache_workers = cpu_workers.min(4);
            #[cfg(feature = "async-providers")]
            let async_acquisition = registry.has_async_preparers();
            #[cfg(not(feature = "async-providers"))]
            let async_acquisition = false;
            #[cfg(feature = "async-providers")]
            {
                async_acquisition_enabled = async_acquisition;
            }
            let acquisition_slots = if async_acquisition {
                async_slots.max(1)
            } else {
                workers
            };
            shared.state.lock().unwrap().staging_limit = Some(acquisition_slots + cpu_workers * 2);

            #[cfg(feature = "async-providers")]
            if async_acquisition {
                let runtime = vearth_platform::rt::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .max_blocking_threads(8)
                    .thread_name("vearth-acquisition")
                    .enable_time()
                    .build()
                    .expect("async provider acquisition runtime");
                let handle = runtime.handle().clone();
                let shared = shared.clone();
                let registry = registry.clone();
                let cache = cache.clone();
                vearth_platform::thread::Builder::new()
                    .name("vearth-acquisition-dispatch".into())
                    .spawn(move || {
                        async_acquisition_dispatch_loop(
                            shared,
                            registry,
                            cache,
                            handle,
                            acquisition_slots,
                        )
                    })
                    .expect("async acquisition dispatcher");
                acquisition_runtime = Some(runtime);
            }
            if !async_acquisition {
                let (tx_acquire, rx_acquire) = sync_channel::<AcquireJob>(workers * 2);
                let rx_acquire = Arc::new(std::sync::Mutex::new(rx_acquire));
                for i in 0..cache_workers {
                    let shared = shared.clone();
                    let registry = registry.clone();
                    let cache = cache.clone();
                    let tx_acquire = tx_acquire.clone();
                    vearth_platform::thread::Builder::new()
                        .name(format!("vearth-cache-{i}"))
                        .spawn(move || {
                            cache_check_loop(&shared, &registry, cache.as_deref(), &tx_acquire)
                        })
                        .expect("cache worker");
                }
                for i in 0..workers {
                    let shared = shared.clone();
                    let registry = registry.clone();
                    let rx_acquire = rx_acquire.clone();
                    vearth_platform::thread::Builder::new()
                        .name(format!("vearth-io-{i}"))
                        .spawn(move || acquisition_loop(&shared, &registry, &rx_acquire))
                        .expect("acquisition worker");
                }
            }
            for i in 0..cpu_workers {
                let shared = shared.clone();
                let registry = registry.clone();
                let cache = cache.clone();
                let tx_done = tx_done.clone();
                vearth_platform::thread::Builder::new()
                    .name(format!("vearth-cpu-{i}"))
                    .spawn(move || cpu_loop(&shared, &registry, cache.as_deref(), &tx_done))
                    .expect("CPU worker");
            }
            if async_acquisition {
                log::info!(
                    "stream pipeline: priority async acquisition+cache (limit {acquisition_slots}), {cpu_workers} CPU workers"
                );
            } else {
                log::info!(
                    "stream pipeline: {cache_workers} cache, {workers} blocking acquisition, {cpu_workers} CPU workers"
                );
            }
        } else {
            for i in 0..workers {
                let shared = shared.clone();
                let registry = registry.clone();
                let cache = cache.clone();
                let tx = tx_done.clone();
                vearth_platform::thread::Builder::new()
                    .name(format!("vearth-gen-{i}"))
                    .spawn(move || worker_loop(&shared, &registry, cache.as_deref(), &tx))
                    .expect("gen worker");
            }
            log::info!("stream pipeline: legacy blocking pool ({workers} workers)");
        }
        GenPool {
            shared,
            rx_done,
            registry,
            cache,
            staged_pipeline,
            #[cfg(feature = "async-providers")]
            _acquisition_runtime: acquisition_runtime,
            #[cfg(feature = "async-providers")]
            async_acquisition: async_acquisition_enabled,
        }
    }

    /// (queued, actively generating) — the two halves of `in_flight_count`.
    pub fn counts(&self) -> (usize, usize) {
        let st = self.shared.state.lock().unwrap();
        (st.pending_set.len(), st.running.len())
    }

    pub fn stage_counts(&self) -> PoolStageCounts {
        let state = self.shared.state.lock().unwrap();
        let mut counts = PoolStageCounts::default();
        for stage in state.stages.values() {
            match stage {
                CellStage::CacheCheck => counts.cache_check += 1,
                CellStage::Acquiring => counts.acquiring += 1,
                CellStage::Ready | CellStage::Cpu => counts.cpu += 1,
            }
        }
        counts
    }

    pub fn staged_pipeline(&self) -> bool {
        self.staged_pipeline
    }

    #[cfg(feature = "async-providers")]
    pub fn async_acquisition(&self) -> bool {
        self.async_acquisition
    }

    pub fn cache_stats(&self) -> Option<crate::chunk_cache::ChunkCacheStats> {
        self.cache.as_ref().map(|cache| cache.stats())
    }

    /// Non-nominal provider health notes for the HUD (empty = all good).
    pub fn provider_status(&self) -> Vec<String> {
        self.registry.provider_status()
    }

    /// Snapshot of the shared provider controls. The registry owns the
    /// state, keeping this independent of the UI implementation.
    pub fn provider_controls(&self) -> Vec<vearth_data::registry::ProviderControl> {
        self.registry.provider_controls()
    }

    /// Apply a provider control and cancel every generation revision that
    /// may have sampled the previous configuration. The app separately
    /// evicts resident meshes so the next queue sync rebuilds the view.
    pub fn set_provider_enabled(
        &self,
        id: &vearth_data::registry::ProviderId,
        enabled: bool,
    ) -> bool {
        if !self.registry.set_provider_enabled(id, enabled) {
            return false;
        }

        self.invalidate_generation();
        true
    }

    /// Switch time-varying imagery to a new calendar month and cancel every
    /// generation revision that may have baked colours from the prior atlas.
    pub fn set_imagery_month(&self, month: u8) -> bool {
        if !self.registry.set_imagery_month(month) {
            return false;
        }
        self.invalidate_generation();
        true
    }

    /// Cancel every queued/running generation revision without changing the
    /// provider configuration. The landmark calibrator uses this after a
    /// transform edit so no cell generated with the previous placement can
    /// land in the refreshed world.
    pub fn invalidate_generation(&self) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.pending.clear();
            state.pending_set.clear();
            state.running.clear();
            state.desired.clear();
            state.desired_priorities.clear();
            purge_ready(&mut state);
            state.stages.clear();
            state.failures.clear();
        }
        #[cfg(feature = "async-providers")]
        self.shared
            .desired_version
            .send_modify(|version| *version = version.wrapping_add(1));
        self.shared.cv.notify_all();
        self.shared.ready_cv.notify_all();
    }

    /// Stop admission and cancel staged work before the renderer is destroyed.
    /// A CPU operation already executing may finish, but cannot advance or publish.
    pub fn shutdown(&self) {
        if !self.shared.stop.swap(true, Ordering::SeqCst) {
            self.invalidate_generation();
        }
    }

    /// Diff the pending queue against the current intents. Removed cells are
    /// invalidated in the revision map and disappear lazily from the heap;
    /// unchanged priorities allocate nothing. A periodic compaction bounds
    /// stale heap entries after sustained camera motion.
    pub fn sync_queue(&mut self, jobs: &[(CellId, f64)]) {
        let mut st = self.shared.state.lock().unwrap();
        if self.shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let changes = sync_pending_state(&mut st, jobs);
        drop(st);
        #[cfg(feature = "async-providers")]
        if changes.desired_changed {
            self.shared
                .desired_version
                .send_modify(|version| *version = version.wrapping_add(1));
        }
        // Wake enough workers for the new work (notify_all is fine too; this
        // just avoids a thundering herd on tiny queues).
        if changes.desired_changed {
            self.shared.cv.notify_all();
        } else {
            for _ in 0..changes.queued_changes.min(changes.pending_len).min(64) {
                self.shared.cv.notify_one();
            }
        }
    }

    pub fn queued(&self) -> usize {
        self.shared.state.lock().unwrap().pending_set.len()
    }

    /// Take at most one current completion from the bounded worker handoff.
    ///
    /// The render loop uses this incremental form to stop draining when its
    /// CPU staging backlog reaches the high watermark. Leaving completions in
    /// the bounded channel blocks producers naturally, preserving completed
    /// meshes instead of dropping and regenerating them.
    pub fn drain_done_one(&mut self) -> Option<PreparedRenderCell> {
        loop {
            let completion = self.rx_done.try_recv().ok()?;
            let mut state = self.shared.state.lock().unwrap();
            let current =
                state.running.get(&completion.work.cell).copied() == Some(completion.work.revision);
            if !current {
                continue;
            }
            state.running.remove(&completion.work.cell);
            state.stages.remove(&completion.work.cell);
            state.failures.remove(&completion.work.cell);
            return Some(completion.prepared);
        }
    }

    pub fn drain_done(&mut self) -> Vec<PreparedRenderCell> {
        let mut out = Vec::new();
        while let Some(completion) = self.drain_done_one() {
            out.push(completion);
        }
        out
    }

    pub fn in_flight_count(&self) -> usize {
        let st = self.shared.state.lock().unwrap();
        st.pending_set.len() + st.running.len()
    }

    /// (cells generated, of which persistent-cache hits) since startup.
    pub fn stats(&self) -> (u64, u64) {
        (
            self.shared.generated.load(Ordering::Relaxed),
            self.shared.cache_hits.load(Ordering::Relaxed),
        )
    }

    /// Per-frame progress hook; generation happens on worker threads, so this
    /// is a no-op.
    pub fn pump(&mut self) {}
}

impl Drop for GenPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn normalized_priority(priority: f64) -> f64 {
    if !priority.is_finite() || priority == 0.0 {
        0.0
    } else {
        priority
    }
}

#[derive(Debug, PartialEq, Eq)]
struct QueueSync {
    pending_len: usize,
    queued_changes: usize,
    desired_changed: bool,
}

/// Apply one frame's desired intents without rebuilding the heap. The heap
/// may contain old revisions, but `pending_set` names the single live entry
/// for every cell and bounds that stale tail through periodic compaction.
fn sync_pending_state(state: &mut PoolState, jobs: &[(CellId, f64)]) -> QueueSync {
    let mut desired_priorities = std::mem::take(&mut state.desired_priorities);
    desired_priorities.clear();
    for &(cell, priority) in jobs {
        let priority = normalized_priority(priority);
        desired_priorities
            .entry(cell)
            .and_modify(|current| {
                if priority.total_cmp(current).is_gt() {
                    *current = priority;
                }
            })
            .or_insert(priority);
    }

    let desired_changed = desired_priorities.len() != state.desired.len()
        || desired_priorities
            .keys()
            .any(|cell| !state.desired.contains(cell));
    if desired_changed {
        state
            .desired
            .retain(|cell| desired_priorities.contains_key(cell));
        state.desired.extend(desired_priorities.keys().copied());
        state
            .failures
            .retain(|cell, _| state.desired.contains(cell));
    }
    purge_ready(state);
    state.pending_set.retain(|cell, _| {
        desired_priorities.contains_key(cell) && !state.running.contains_key(cell)
    });

    let mut changed = Vec::new();
    for (&cell, &priority) in &desired_priorities {
        if state.running.contains_key(&cell)
            || state.failures.get(&cell).copied().unwrap_or(0) >= MAX_GEOMETRY_ATTEMPTS
        {
            continue;
        }
        let unchanged = state
            .pending_set
            .get(&cell)
            .is_some_and(|(old_priority, _)| old_priority.to_bits() == priority.to_bits());
        if unchanged {
            continue;
        }
        changed.push((cell, priority));
    }
    // Only changed priorities need revisions. Preserve their CellId order so
    // the exact revision and priority sequence remains deterministic.
    changed.sort_unstable_by_key(|(cell, _)| *cell);
    let queued_changes = changed.len();
    for (cell, priority) in changed {
        state.next_revision = state.next_revision.wrapping_add(1);
        let revision = state.next_revision;
        state.pending_set.insert(cell, (priority, revision));
        state.pending.push(Job {
            priority,
            cell,
            revision,
        });
    }

    let live = state.pending_set.len();
    if state.pending.len() > live.saturating_mul(2).saturating_add(128) {
        let old_heap = std::mem::take(&mut state.pending);
        state.pending = old_heap
            .into_iter()
            .filter(|job| {
                state
                    .pending_set
                    .get(&job.cell)
                    .is_some_and(|(priority, revision)| {
                        priority.to_bits() == job.priority.to_bits() && *revision == job.revision
                    })
            })
            .collect();
    }
    state.desired_priorities = desired_priorities;
    QueueSync {
        pending_len: live,
        queued_changes,
        desired_changed,
    }
}

/// Ready payloads are owned here, so obsolete work can release its source
/// leases and staging reservation immediately instead of waiting in a FIFO.
fn purge_ready(state: &mut PoolState) {
    let mut index = 0;
    while index < state.ready.len() {
        let work = state.ready[index].work();
        if state.running.get(&work.cell).copied() == Some(work.revision)
            && state.desired.contains(&work.cell)
        {
            index += 1;
            continue;
        }
        let job = state.ready.swap_remove(index);
        state.staging.remove(&(work.cell, work.revision));
        if state.running.get(&work.cell).copied() == Some(work.revision) {
            state.running.remove(&work.cell);
            state.stages.remove(&work.cell);
        }
        drop(job);
    }
}

fn publish_ready(shared: &PoolShared, job: CpuJob) -> bool {
    let work = job.work();
    let mut state = shared.state.lock().unwrap();
    if shared.stop.load(Ordering::SeqCst)
        || state.running.get(&work.cell).copied() != Some(work.revision)
        || !state.desired.contains(&work.cell)
    {
        drop(state);
        cancel_running(shared, work);
        return false;
    }
    // A reservation has exactly one physical owner until CPU claim. Duplicate
    // publication must not create two consumers or retire the first owner.
    if !state.staging.contains(&(work.cell, work.revision))
        || state.stages.get(&work.cell) == Some(&CellStage::Ready)
    {
        return false;
    }
    state.stages.insert(work.cell, CellStage::Ready);
    state.ready.push(job);
    drop(state);
    shared.ready_cv.notify_one();
    true
}

fn pop_current_ready(state: &mut PoolState) -> Option<CpuJob> {
    purge_ready(state);
    // This collection is bounded by staging reservations. Looking up current
    // intent scores at claim time also handles priority-only camera changes,
    // without another heap of stale payloads or reacquiring source data.
    let index = (0..state.ready.len()).max_by(|&a, &b| {
        let a = &state.ready[a];
        let b = &state.ready[b];
        let priority = |job: &CpuJob| {
            let work = job.work();
            let score = state
                .desired_priorities
                .get(&work.cell)
                .copied()
                .unwrap_or(work.priority);
            // Cached packing is cheaper than generation and frees a staging
            // reservation. This bias stays below the lane and bootstrap gaps.
            if matches!(job, CpuJob::Cached { .. }) && score.is_finite() && score > 0.0 {
                (score * 64.0).min(f64::MAX)
            } else {
                score
            }
        };
        priority(a)
            .total_cmp(&priority(b))
            .then_with(|| b.work().cell.cmp(&a.work().cell))
    })?;
    let job = state.ready.swap_remove(index);
    let accepted = transition_stage(state, job.work(), CellStage::Cpu);
    debug_assert!(accepted);
    Some(job)
}

fn take_ready(shared: &PoolShared) -> Option<CpuJob> {
    let mut state = shared.state.lock().unwrap();
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return None;
        }
        let reservations = state.staging.len();
        let job = pop_current_ready(&mut state);
        if state.staging.len() < reservations {
            shared.cv.notify_all();
        }
        if job.is_some() {
            return job;
        }
        state = shared.ready_cv.wait(state).unwrap();
    }
}

fn pop_current_pending(state: &mut PoolState) -> Option<WorkItem> {
    if state
        .staging_limit
        .is_some_and(|limit| state.staging.len() >= limit)
    {
        return None;
    }
    while let Some(job) = state.pending.pop() {
        let current = state
            .pending_set
            .get(&job.cell)
            .is_some_and(|(priority, revision)| {
                priority.to_bits() == job.priority.to_bits() && *revision == job.revision
            });
        if !current {
            continue;
        }

        state.pending_set.remove(&job.cell);
        state.running.insert(job.cell, job.revision);
        state.staging.insert((job.cell, job.revision));
        state.stages.insert(job.cell, CellStage::CacheCheck);
        return Some(WorkItem {
            cell: job.cell,
            revision: job.revision,
            priority: job.priority,
        });
    }
    None
}

fn take_pending(shared: &PoolShared) -> Option<WorkItem> {
    let mut state = shared.state.lock().unwrap();
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return None;
        }
        if let Some(work) = pop_current_pending(&mut state) {
            return Some(work);
        }
        state = shared.cv.wait(state).unwrap();
    }
}

fn transition_stage(state: &mut PoolState, work: WorkItem, stage: CellStage) -> bool {
    let current = state.running.get(&work.cell).copied() == Some(work.revision);
    if !current {
        state.staging.remove(&(work.cell, work.revision));
        return false;
    }
    if !state.desired.contains(&work.cell) {
        state.staging.remove(&(work.cell, work.revision));
        state.running.remove(&work.cell);
        state.stages.remove(&work.cell);
        return false;
    }
    if stage == CellStage::Cpu {
        state.staging.remove(&(work.cell, work.revision));
    }
    state.stages.insert(work.cell, stage);
    true
}

fn advance_stage(shared: &PoolShared, work: WorkItem, stage: CellStage) -> bool {
    let mut state = shared.state.lock().unwrap();
    let before = state.staging.len();
    let result = transition_stage(&mut state, work, stage);
    let released = state.staging.len() < before;
    drop(state);
    if released {
        shared.cv.notify_all();
    }
    result
}

fn cancel_running(shared: &PoolShared, work: WorkItem) {
    let mut state = shared.state.lock().unwrap();
    let released = state.staging.remove(&(work.cell, work.revision));
    if state.running.get(&work.cell).copied() == Some(work.revision) {
        state.running.remove(&work.cell);
        state.stages.remove(&work.cell);
    }
    drop(state);
    if released {
        shared.cv.notify_all();
    }
}

fn fail_geometry(shared: &PoolShared, work: WorkItem, error: &str) {
    let mut state = shared.state.lock().unwrap();
    if state.staging.remove(&(work.cell, work.revision)) {
        shared.cv.notify_all();
    }
    if state.running.get(&work.cell).copied() != Some(work.revision) {
        return;
    }
    state.running.remove(&work.cell);
    state.stages.remove(&work.cell);
    if !state.desired.contains(&work.cell) {
        return;
    }
    let attempts = state.failures.entry(work.cell).or_default();
    *attempts = attempts.saturating_add(1);
    let attempts = *attempts;
    drop(state);
    let disposition = if attempts < MAX_GEOMETRY_ATTEMPTS {
        "retry on the next queue update"
    } else {
        "retry suspended until a new intent or generation invalidation"
    };
    log::warn!(
        "surface preparation failed for {:?} revision {} (attempt {attempts}/{MAX_GEOMETRY_ATTEMPTS}): {error}; {disposition}",
        work.cell,
        work.revision,
    );
}

fn run_geometry(
    shared: &PoolShared,
    work: WorkItem,
    operation: &str,
    generate: impl FnOnce() -> Result<PreparedRenderCell, String>,
) -> Option<PreparedRenderCell> {
    // No pool-state lock spans cell generation. Unwinding drops the cell's
    // partial geometry; the exact revision is then retired just like a checked
    // validation failure. The panic hook still records its original location.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(generate));
    let result = match result {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            Err(format!("caught panic during {operation}: {message}"))
        }
    };
    match result {
        Ok(prepared) => Some(prepared),
        Err(error) => {
            fail_geometry(shared, work, &error);
            None
        }
    }
}

#[cfg(feature = "async-providers")]
fn work_is_desired(shared: &PoolShared, work: WorkItem) -> bool {
    let state = shared.state.lock().unwrap();
    state.running.get(&work.cell).copied() == Some(work.revision)
        && state.desired.contains(&work.cell)
}

/// Revision ownership for a detached Tokio acquisition task. Any panic,
/// runtime shutdown, canceled plan, or failed handoff removes the exact
/// revision from `running`; successful CPU handoff explicitly disarms it.
#[cfg(feature = "async-providers")]
struct AsyncRunningGuard {
    shared: Arc<PoolShared>,
    work: WorkItem,
    armed: bool,
}

#[cfg(feature = "async-providers")]
impl AsyncRunningGuard {
    fn new(shared: Arc<PoolShared>, work: WorkItem) -> Self {
        Self {
            shared,
            work,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(feature = "async-providers")]
impl Drop for AsyncRunningGuard {
    fn drop(&mut self) {
        if self.armed {
            cancel_running(&self.shared, self.work);
        }
    }
}

fn cache_check_loop(
    shared: &PoolShared,
    registry: &Arc<Registry>,
    cache: Option<&ChunkCache>,
    tx_acquire: &SyncSender<AcquireJob>,
) {
    while let Some(work) = take_pending(shared) {
        let cell = work.cell;
        shared.generated.fetch_add(1, Ordering::Relaxed);
        if !advance_stage(shared, work, CellStage::CacheCheck) {
            continue;
        }
        if registry.generated_cache_allowed()
            && let Some(column) = cache.and_then(|cache| cache.load(cell))
        {
            shared.cache_hits.fetch_add(1, Ordering::Relaxed);
            publish_ready(shared, CpuJob::Cached { column, work });
            continue;
        }

        let plan = CellPlan::for_cell(cell, registry.has_vectors(), work.priority);
        if !advance_stage(shared, work, CellStage::Acquiring) {
            continue;
        }
        if tx_acquire.send(AcquireJob { plan, work }).is_err() {
            cancel_running(shared, work);
            return;
        }
    }
}

fn acquisition_loop(
    shared: &PoolShared,
    registry: &Arc<Registry>,
    rx: &std::sync::Mutex<Receiver<AcquireJob>>,
) {
    loop {
        let job = {
            let receiver = rx.lock().unwrap();
            receiver.recv()
        };
        let Ok(job) = job else { return };
        let work = job.work;
        if !advance_stage(shared, work, CellStage::Acquiring) {
            continue;
        }
        job.plan.acquire(registry);
        publish_ready(
            shared,
            CpuJob::Prepared {
                work,
                acquired: AcquiredSources {
                    #[cfg(feature = "async-providers")]
                    retained: Default::default(),
                    complete: true,
                    features: None,
                },
            },
        );
    }
}

/// Admit the highest-priority live revision directly into provider futures.
/// The semaphore is acquired *before* the replaceable priority heap is
/// popped. Consequently a cell stays reprioritizable/cancelable until a real
/// network slot is available: a later high-priority cell can overtake every
/// queued low-priority cell instead of sitting behind a FIFO handoff.
///
/// At most `max_active` cell plans acquire concurrently. Provider
/// transports apply their own lower per-origin limits, and cache-level keyed
/// locks collapse shared dependencies across these plans. A staging reservation
/// covers cache, acquisition and ready ownership until a CPU worker claims it;
/// completed acquisition never waits for a downstream sender.
#[cfg(feature = "async-providers")]
fn async_acquisition_dispatch_loop(
    shared: Arc<PoolShared>,
    registry: Arc<Registry>,
    cache: Option<Arc<ChunkCache>>,
    handle: vearth_platform::rt::runtime::Handle,
    max_active: usize,
) {
    let admission = Arc::new(tokio::sync::Semaphore::new(max_active.max(1)));
    loop {
        // Do not remove a cell from the replaceable heap while every network
        // slot is occupied. This ordering is the priority-admission contract.
        let permit = match handle.block_on(admission.clone().acquire_owned()) {
            Ok(permit) => permit,
            Err(_) => return,
        };
        let Some(work) = take_pending(&shared) else {
            return;
        };
        let shared = shared.clone();
        let registry = registry.clone();
        let cache = cache.clone();
        // Construct the guard before handing the future to Tokio. Runtime
        // shutdown then releases the exact running revision even if this
        // future is dropped before its first poll.
        let running = AsyncRunningGuard::new(shared.clone(), work);
        handle.spawn(async move {
            let _permit = permit;
            let mut running = running;
            shared.generated.fetch_add(1, Ordering::Relaxed);
            if !advance_stage(&shared, work, CellStage::CacheCheck) {
                return;
            }
            let cached = if registry.generated_cache_allowed()
                && let Some(cache) = cache
            {
                let cell = work.cell;
                match vearth_platform::rt::task::spawn_blocking(move || cache.load(cell)).await {
                    Ok(hit) => hit,
                    Err(error) => {
                        log::warn!("cache check task failed for {cell:?}: {error}");
                        None
                    }
                }
            } else {
                None
            };
            if let Some(column) = cached {
                shared.cache_hits.fetch_add(1, Ordering::Relaxed);
                if publish_ready(&shared, CpuJob::Cached { column, work }) {
                    running.disarm();
                }
                return;
            }

            let plan = CellPlan::for_cell(work.cell, registry.has_vectors(), work.priority);
            if !advance_stage(&shared, work, CellStage::Acquiring) {
                return;
            }
            let mut desired_version = shared.desired_version.subscribe();
            let mut acquisition = Box::pin(plan.acquire_async(&registry));
            let acquired = loop {
                if !work_is_desired(&shared, work) {
                    // Dropping the preparation cancels unstarted/unique raw
                    // requests. Provider-owned detached single-flights that
                    // already have other waiters remain free to finish into
                    // cache.
                    return;
                }
                enum AcquisitionEvent {
                    Complete(AcquiredSources),
                    DesiredChanged(Result<(), tokio::sync::watch::error::RecvError>),
                }
                let mut changed = Box::pin(desired_version.changed());
                let event = std::future::poll_fn(|cx| {
                    if let std::task::Poll::Ready(result) =
                        std::future::Future::poll(acquisition.as_mut(), cx)
                    {
                        return std::task::Poll::Ready(AcquisitionEvent::Complete(result));
                    }
                    if let std::task::Poll::Ready(result) =
                        std::future::Future::poll(changed.as_mut(), cx)
                    {
                        return std::task::Poll::Ready(AcquisitionEvent::DesiredChanged(result));
                    }
                    std::task::Poll::Pending
                })
                .await;
                match event {
                    AcquisitionEvent::Complete(result) => break result,
                    AcquisitionEvent::DesiredChanged(Ok(())) => {}
                    AcquisitionEvent::DesiredChanged(Err(_)) => return,
                }
            };
            if publish_ready(&shared, CpuJob::Prepared { work, acquired }) {
                running.disarm();
            }
        });
    }
}

fn cpu_loop(
    shared: &PoolShared,
    registry: &Arc<Registry>,
    cache: Option<&ChunkCache>,
    tx_done: &SyncSender<Completion>,
) {
    while let Some(job) = take_ready(shared) {
        let work = job.work();
        let operation = match &job {
            CpuJob::Cached { .. } => "cached surface packing",
            CpuJob::Prepared { .. } => "surface generation and packing",
        };
        let Some(prepared) = run_geometry(shared, work, operation, || match job {
            CpuJob::Cached { column, .. } => pack_surface_cell(column),
            CpuJob::Prepared { work, acquired } => {
                generate_prepared(work.cell, registry, cache, acquired)
            }
        }) else {
            continue;
        };
        // Generation can be long enough for several camera revisions. Check
        // the token again before occupying bounded completion capacity.
        if !advance_stage(shared, work, CellStage::Cpu) {
            continue;
        }
        // The completion channel is bounded. Backpressure here prevents the
        // CPU lane from outrunning render-thread upload admission.
        if tx_done.send(Completion { work, prepared }).is_err() {
            cancel_running(shared, work);
            return;
        }
    }
}

fn worker_loop(
    shared: &PoolShared,
    registry: &Arc<Registry>,
    cache: Option<&ChunkCache>,
    tx: &SyncSender<Completion>,
) {
    while let Some(work) = take_pending(shared) {
        if !advance_stage(shared, work, CellStage::Cpu) {
            continue;
        }
        let Some(prepared) = run_geometry(shared, work, "blocking surface generation", || {
            generate_one(work.cell, registry, cache, shared)
        }) else {
            continue;
        };
        if !advance_stage(shared, work, CellStage::Cpu) {
            continue;
        }
        // The cell stays in `running` until the app drains the result:
        // between send and drain it is neither resident nor pending, and
        // removing it here would let the per-frame sync_queue re-enqueue a
        // finished cell — 32 workers then regenerate already-done cells in
        // a tight loop (the "fan blazing, nothing loads" failure mode).
        if tx.send(Completion { work, prepared }).is_err() {
            cancel_running(shared, work);
            return; // pool dropped
        }
    }
}

fn prepare_worker_cell(
    surface: SurfaceCell,
    registry: &Registry,
    cache: Option<&ChunkCache>,
    acquisition_complete: bool,
) -> Result<PreparedRenderCell, String> {
    pack_surface_cell_with(surface, |validated| {
        // A provider may have changed while this CPU job was sampling. The
        // revision check rejects its completion; this gate also protects cache.
        if registry.generated_cache_allowed()
            && let Some(cache) = cache
        {
            persist_complete_column(cache, validated, acquisition_complete);
        }
    })
}

fn generate_prepared(
    cell: CellId,
    registry: &Arc<Registry>,
    cache: Option<&ChunkCache>,
    acquired: AcquiredSources,
) -> Result<PreparedRenderCell, String> {
    let sources = Sources {
        elevation: registry.as_ref(),
        surface: registry.surface(),
        imagery: Some(registry.as_ref()),
        vectors: registry.has_vectors().then_some(registry.as_ref() as _),
    };
    let cache = registry
        .generated_cache_allowed()
        .then_some(cache)
        .flatten();
    let surface = match acquired.features {
        Some(features) => generate_surface_cell_with_features(cell, &sources, features),
        None => generate_surface_cell(cell, &sources),
    };
    #[cfg(feature = "async-providers")]
    drop(acquired.retained);
    prepare_worker_cell(surface, registry, cache, acquired.complete)
}

fn generate_one(
    cell: CellId,
    registry: &Arc<Registry>,
    cache: Option<&ChunkCache>,
    shared: &PoolShared,
) -> Result<PreparedRenderCell, String> {
    shared.generated.fetch_add(1, Ordering::Relaxed);
    // Persistent-cache hit: restore final surfaces without meshing or source reads.
    if registry.generated_cache_allowed()
        && let Some(cache) = cache
        && let Some(col) = cache.load(cell)
    {
        shared.cache_hits.fetch_add(1, Ordering::Relaxed);
        return pack_surface_cell(col);
    }
    // Warm every tile this cell reads — elevation and vectors — with all
    // fetches in flight concurrently. Cold generation is otherwise
    // serialized on a dozen round-trips.
    CellPlan::for_cell(cell, registry.has_vectors(), 0.0).acquire(registry);
    generate_prepared(
        cell,
        registry,
        cache,
        AcquiredSources {
            #[cfg(feature = "async-providers")]
            retained: Default::default(),
            complete: true,
            features: None,
        },
    )
}

fn persist_complete_column(
    cache: &ChunkCache,
    column: &SurfaceCell,
    acquisition_complete: bool,
) -> bool {
    if acquisition_complete && column.complete {
        cache.store(column);
        true
    } else {
        false
    }
}

/// Coarse bootstrap needed to cover a missing target. Existing ancestors and
/// complete finer cuts already provide coverage; neither needs intermediate
/// generations before the exact target can be requested.
pub fn missing_coverage_root(cell: CellId, store: &ResidentStore) -> Option<CellId> {
    if store.resident_subtree_ready(cell) {
        return None;
    }
    let mut current = cell;
    loop {
        if store.contains(current) {
            return None;
        }
        if current.level <= MIN_RENDER_LEVEL {
            return Some(current);
        }
        current = current.parent().unwrap();
    }
}

/// Is `cell`'s area already covered by something being drawn — an ancestor
/// (refine/backdrop pairing) or a drawn descendant (merge pairing)? Decides
/// whether a cell entering the cut dithers in (covered: the fade blends two
/// surfaces) or appears instantly (uncovered: dithering against bare sky
/// reads as a flicker at silhouettes).
pub fn covered_by_drawn(cell: CellId, drawn: &HashSet<CellId>) -> bool {
    let mut cur = cell;
    while let Some(p) = cur.parent() {
        if p.level < MIN_RENDER_LEVEL {
            break;
        }
        if drawn.contains(&p) {
            return true;
        }
        cur = p;
    }
    drawn.iter().any(|d| {
        d.face == cell.face && d.level > cell.level && {
            let s = d.level - cell.level;
            (d.i >> s) == cell.i && (d.j >> s) == cell.j
        }
    })
}

/// Stronger coverage predicate used when a new transition decides whether it
/// may dither from zero. One drawn descendant is not sufficient: every one of
/// the cell's four quadrants must recursively resolve to a drawn descendant,
/// unless a drawn ancestor already covers the whole area.
#[cfg(test)]
pub(crate) fn fully_covered_by_drawn(cell: CellId, drawn: &HashSet<CellId>) -> bool {
    let mut cur = cell;
    loop {
        if drawn.contains(&cur) {
            return true;
        }
        let Some(parent) = cur.parent() else {
            break;
        };
        cur = parent;
    }

    let mut prefixes = HashSet::new();
    for &descendant in drawn {
        if descendant.face != cell.face || descendant.level <= cell.level {
            continue;
        }
        let shift = descendant.level - cell.level;
        if (descendant.i >> shift) != cell.i || (descendant.j >> shift) != cell.j {
            continue;
        }
        let mut prefix = descendant;
        while prefix.level > cell.level {
            prefix = prefix.parent().unwrap();
            prefixes.insert(prefix);
        }
    }

    fn covered(node: CellId, drawn: &HashSet<CellId>, prefixes: &HashSet<CellId>) -> bool {
        drawn.contains(&node)
            || (prefixes.contains(&node)
                && node
                    .children()
                    .into_iter()
                    .all(|child| covered(child, drawn, prefixes)))
    }
    covered(cell, drawn, &prefixes)
}

/// Stable handle joining one CPU resident record to its GPU allocation.
///
/// Removing a record increments its slot generation before that slot can be
/// reused, so a delayed draw or eviction command can never resolve to the
/// next cell that happens to occupy the same slot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResidentId {
    slot: u32,
    generation: u32,
}

impl ResidentId {
    pub(crate) fn slot(self) -> usize {
        self.slot as usize
    }

    pub(crate) fn generation(self) -> u32 {
        self.generation
    }

    #[cfg(test)]
    pub(crate) const fn for_test(slot: u32, generation: u32) -> Self {
        Self { slot, generation }
    }
}

/// The authoritative CPU record for one resident cell. Mutations are routed
/// through `ResidentStore` so cell lookup, slot generations, collision data,
/// LOD metrics, and byte accounting cannot drift apart.
#[derive(Debug)]
pub struct ResidentRecord {
    pub cell: CellId,
    pub last_drawn_frame: u64,
    pub height_range: (f32, f32),
    pub terrain_step_m: f32,
    /// Physical terrain relief excludes visual skirts and elevated content.
    pub terrain_relief_m: f32,
    pub terrain_detail: Option<Arc<vearth_gen::TerrainDetailSummary>>,
    /// Structure proxies, carved terrain and water tops complement
    /// the ground heightfield in vehicle and camera contact queries.
    pub solid_collision: Option<SurfaceCollision>,
    pub water: Vec<vearth_gen::water_surface::WaterTriangle>,
    pub terrain_collision: Option<TerrainCollision>,
    pub road_guides: Vec<RoadGuide>,
    pub gpu_bytes: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RoadGuideHit {
    pub tangent: DVec3,
    pub center_offset: DVec3,
    pub distance_m: f64,
    pub width_m: f64,
    pub assist: f64,
}

#[derive(Debug)]
struct ResidentSlot {
    generation: u32,
    record: Option<ResidentRecord>,
}

/// Reusable scratch state and the most recent drawable-cut result.
/// Membership and ordered wanted cells determine coverage; missing priorities
/// are read afresh on every call. Storage retains at most one resolved cut.
pub struct DrawableCutArena {
    is_leaf: HashMap<CellKey, bool>,
    topology: Vec<CellId>,
    roots: Vec<CellId>,
    ready_memo: HashMap<CellKey, bool>,
    seen: HashSet<CellKey>,
    resolved: ResolvedCut,
    #[cfg(test)]
    rebuilds: usize,
    #[cfg(test)]
    topology_rebuilds: usize,
}

#[derive(Default)]
struct ResolvedCut {
    store_identity: Option<Arc<()>>,
    membership_revision: u64,
    wanted: Vec<CellId>,
    draw: Vec<CellId>,
    missing_indices: Vec<usize>,
}

impl Default for DrawableCutArena {
    fn default() -> Self {
        Self::new()
    }
}

impl DrawableCutArena {
    pub fn new() -> Self {
        Self {
            is_leaf: HashMap::with_capacity(MAX_WANTED * 2),
            topology: Vec::with_capacity(MAX_WANTED),
            roots: Vec::with_capacity(64),
            ready_memo: HashMap::with_capacity(MAX_WANTED),
            seen: HashSet::with_capacity(MAX_WANTED),
            resolved: ResolvedCut::default(),
            #[cfg(test)]
            rebuilds: 0,
            #[cfg(test)]
            topology_rebuilds: 0,
        }
    }

    /// Forget the resolved cut while retaining scratch and output allocations.
    pub fn invalidate(&mut self) {
        self.resolved.store_identity = None;
    }

    fn prepare(&mut self, wanted: &[WantedCell]) {
        self.ready_memo.clear();
        self.seen.clear();
        if self
            .topology
            .iter()
            .copied()
            .eq(wanted.iter().map(|wanted| wanted.cell))
        {
            return;
        }
        self.is_leaf.clear();
        self.topology.clear();
        self.roots.clear();
        for w in wanted {
            self.topology.push(w.cell);
            self.is_leaf.insert(CellKey::from(w.cell), true);
            let mut current = w.cell;
            while current.level > MIN_RENDER_LEVEL {
                let parent = current.parent().unwrap();
                if self.is_leaf.insert(CellKey::from(parent), false).is_some() {
                    break;
                }
                current = parent;
            }
            // Ancestors already recorded have a root in the same wanted-order
            // prefix. Only a new path reaches its root on this traversal.
            if current.level <= MIN_RENDER_LEVEL && self.seen.insert(CellKey::from(current)) {
                self.roots.push(current);
            }
        }
        self.seen.clear();
        #[cfg(test)]
        {
            self.topology_rebuilds += 1;
        }
    }

    #[cfg(test)]
    fn capacities(&self) -> [usize; 8] {
        [
            self.is_leaf.capacity(),
            self.topology.capacity(),
            self.roots.capacity(),
            self.ready_memo.capacity(),
            self.seen.capacity(),
            self.resolved.wanted.capacity(),
            self.resolved.draw.capacity(),
            self.resolved.missing_indices.capacity(),
        ]
    }
}

#[derive(Debug, Default)]
struct ResidentCoverage {
    members: usize,
    complete: bool,
}

/// Generational resident arena with a compact `CellKey` lookup.
///
/// This replaces four independently mutated maps plus the app's separate
/// byte map. The renderer is keyed only by `ResidentId`; `CellId` ownership
/// and all CPU metadata live here.
#[derive(Debug)]
pub struct ResidentStore {
    by_cell: HashMap<CellKey, ResidentId>,
    slots: Vec<ResidentSlot>,
    free: Vec<u32>,
    len: usize,
    membership_identity: Arc<()>,
    membership_revision: u64,
    coverage: HashMap<CellKey, ResidentCoverage>,
    terrain_metrics: crate::lod_selection::TerrainMetricStore,
    collision_len: usize,
    collision_regions: std::cell::RefCell<Option<CollisionRegionCache>>,
}

#[derive(Debug)]
struct CollisionRegionCache {
    cover: Option<CellId>,
    regions: Vec<CollisionRegion>,
}

#[derive(Clone, Debug)]
struct CollisionRegion {
    owner: ResidentId,
    range: CellRange,
    metric: MetricFrame,
    planes: [[f64; 4]; 4],
}

fn collision_region_normals(cell: CellId) -> [DVec3; 4] {
    let (u0, v0, u1, v1) = cell.uv_bounds();
    let n = cell.face.normal();
    let r = cell.face.right();
    let u = cell.face.up();
    let tan = |v: f64| (v * std::f64::consts::FRAC_PI_4).tan();
    [
        r - n * tan(u0),
        n * tan(u1) - r,
        u - n * tan(v0),
        n * tan(v1) - u,
    ]
    .map(DVec3::normalize)
}

fn collision_region_planes(cell: CellId, frame: &ChunkFrame) -> [[f64; 4]; 4] {
    let metric = MetricFrame::of(frame);
    collision_region_normals(cell).map(|normal| {
        let local = metric.direction_from_ecef(normal);
        [local[0], local[1], local[2], normal.dot(frame.anchor)]
    })
}

fn collision_query_cell(points: &[DVec3]) -> Option<CellId> {
    let first = *points.first()?;
    if points.iter().any(|point| !point.is_finite()) {
        return None;
    }
    let mut cover = CellId::from_dir(first, MAX_STREAM_LEVEL);
    for &point in &points[1..] {
        let mut cell = CellId::from_dir(point, cover.level);
        if cell.face != cover.face {
            return None;
        }
        while cell != cover {
            cell = cell.parent()?;
            cover = cover.parent()?;
        }
    }
    // Cell cones are convex, so enclosing every box corner or ray endpoint
    // encloses the entire query. The margin exceeds narrow-phase clipping's
    // 1e-7 metre tolerance and ECEF dot-product roundoff at the query's scale.
    let scale = points
        .iter()
        .map(|point| point.abs().max_element())
        .fold(1.0, f64::max);
    let margin = 1e-4 + scale * (64.0 * f64::EPSILON);
    loop {
        if collision_region_normals(cover)
            .iter()
            .all(|normal| points.iter().all(|point| normal.dot(*point) > margin))
        {
            return Some(cover);
        }
        // Tangent queries may need a larger cover; cube-face seams and
        // uncertifiable inputs retain the full scan.
        cover = cover.parent()?;
    }
}

fn collision_query_range(points: &[DVec3]) -> Option<CellRange> {
    collision_query_cell(points).map(|cell| CellKey::from(cell).range())
}

fn collision_cache_cover(points: &[DVec3]) -> Option<CellId> {
    let mut cell = collision_query_cell(points)?;
    // A local neighbourhood amortizes ownership work across nearby player and
    // camera queries without retaining unrelated city-wide collision regions.
    while cell.level > 16 {
        cell = cell.parent()?;
    }
    Some(cell)
}

fn collision_region_candidates<'a>(
    regions: &'a [CollisionRegion],
    points: &[DVec3],
) -> &'a [CollisionRegion] {
    let Some(cover) = collision_query_range(points) else {
        return regions;
    };
    // These are disjoint coverage leaves, including clipped pieces inherited
    // from a coarser owner. Their sorted ends are therefore monotonic too.
    let start = regions.partition_point(|region| region.range.end() <= cover.start());
    let end = regions.partition_point(|region| region.range.start() < cover.end());
    &regions[start..end]
}

pub const RESIDENT_CAP: usize = 9000;

/// Coarsest level whose continuous collision is retained. Matches the vector
/// feature gate: below L11 nothing but terrain exists, and the terrain-top
/// floor covers that.
pub const COLLISION_MIN_LEVEL: u8 = 11;
type TerrainMetricLookup<'a> = dyn Fn(CellId) -> Option<TerrainMetricRef<'a>> + 'a;

impl Default for ResidentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ResidentStore {
    pub fn new() -> Self {
        ResidentStore {
            by_cell: HashMap::new(),
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
            membership_identity: Arc::new(()),
            membership_revision: 0,
            coverage: HashMap::new(),
            terrain_metrics: crate::lod_selection::TerrainMetricStore::default(),
            collision_len: 0,
            collision_regions: std::cell::RefCell::new(None),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn membership_stamp(&self) -> (&Arc<()>, u64) {
        (&self.membership_identity, self.membership_revision)
    }

    fn membership_changed(&mut self) {
        if let Some(next) = self.membership_revision.checked_add(1) {
            self.membership_revision = next;
        } else {
            self.membership_identity = Arc::new(());
            self.membership_revision = 0;
        }
    }

    pub fn contains(&self, cell: CellId) -> bool {
        self.by_cell.contains_key(&CellKey::from(cell))
    }

    // Membership is counted once per cell, independently of its vertical mesh
    // batches. A node remains indexed while any resident descendant exists;
    // completeness includes its own resident or all four covered quadrants.
    fn update_coverage(&mut self, cell: CellId, inserted: bool) {
        let mut current = cell;
        loop {
            let key = CellKey::from(current);
            let complete = self.contains(current)
                || (current.level < MAX_STREAM_LEVEL
                    && current
                        .children()
                        .into_iter()
                        .all(|child| self.resident_subtree_ready(child)));
            if inserted {
                let node = self.coverage.entry(key).or_default();
                node.members += 1;
                node.complete = complete;
            } else {
                let node = self
                    .coverage
                    .get_mut(&key)
                    .expect("resident path is indexed");
                node.members -= 1;
                node.complete = complete;
                if node.members == 0 {
                    self.coverage.remove(&key);
                }
            }
            if current.level <= MIN_RENDER_LEVEL {
                break;
            }
            current = current.parent().unwrap();
        }
    }

    pub fn id(&self, cell: CellId) -> Option<ResidentId> {
        self.by_cell.get(&CellKey::from(cell)).copied()
    }

    pub fn get(&self, id: ResidentId) -> Option<&ResidentRecord> {
        let slot = self.slots.get(id.slot())?;
        if slot.generation == id.generation {
            slot.record.as_ref()
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, id: ResidentId) -> Option<&mut ResidentRecord> {
        *self.collision_regions.get_mut() = None;
        self.record_mut(id)
    }

    fn invalidate_collision_owner(&mut self, cell: CellId) {
        let cache = self.collision_regions.get_mut();
        if cache.as_ref().is_some_and(|cache| {
            cache.cover.is_none_or(|cover| {
                CellKey::from(cover)
                    .range()
                    .overlaps(CellKey::from(cell).range())
            })
        }) {
            *cache = None;
        }
    }

    // Controlled metadata updates preserve collision ownership. Arbitrary
    // public record mutation above must still invalidate the derived regions.
    fn record_mut(&mut self, id: ResidentId) -> Option<&mut ResidentRecord> {
        let slot = self.slots.get_mut(id.slot())?;
        if slot.generation == id.generation {
            slot.record.as_mut()
        } else {
            None
        }
    }

    pub fn get_cell(&self, cell: CellId) -> Option<(ResidentId, &ResidentRecord)> {
        let id = self.id(cell)?;
        self.get(id).map(|record| (id, record))
    }

    /// Insert a complete CPU-side resident record and return its stable ID.
    /// Re-inserting an existing cell updates its metadata without changing
    /// the ID, which keeps duplicate completion handling deterministic.
    pub fn insert(
        &mut self,
        cell: CellId,
        last_drawn_frame: u64,
        height_range: (f32, f32),
        terrain_step_m: f32,
        terrain_relief_m: f32,
        terrain_detail: Option<Arc<vearth_gen::TerrainDetailSummary>>,
        solid_collision: Option<SurfaceCollision>,
        gpu_bytes: usize,
    ) -> ResidentId {
        self.terrain_metrics.upsert(
            cell,
            crate::lod_selection::TerrainMetric {
                height_range,
                terrain_step_m,
                terrain_relief_m,
                terrain_detail: terrain_detail.clone(),
            },
            false,
        );
        if let Some(id) = self.id(cell) {
            let had_collision = self.get(id).unwrap().solid_collision.is_some();
            let has_collision = solid_collision.is_some();
            if had_collision != has_collision {
                self.invalidate_collision_owner(cell);
            }
            let record = self
                .record_mut(id)
                .expect("resident cell lookup must resolve its own ID");
            record.last_drawn_frame = last_drawn_frame;
            record.height_range = height_range;
            record.terrain_step_m = terrain_step_m;
            record.terrain_relief_m = terrain_relief_m;
            record.terrain_detail = terrain_detail;
            record.solid_collision = solid_collision;
            record.water.clear();
            record.terrain_collision = None;
            record.road_guides.clear();
            record.gpu_bytes = gpu_bytes;
            self.collision_len =
                self.collision_len + usize::from(has_collision) - usize::from(had_collision);
            return id;
        }

        let (slot_index, generation) = if let Some(slot_index) = self.free.pop() {
            let slot = &self.slots[slot_index as usize];
            debug_assert!(slot.record.is_none());
            (slot_index, slot.generation)
        } else {
            let slot_index =
                u32::try_from(self.slots.len()).expect("resident slot arena exceeded u32 capacity");
            self.slots.push(ResidentSlot {
                generation: 1,
                record: None,
            });
            (slot_index, 1)
        };
        let id = ResidentId {
            slot: slot_index,
            generation,
        };
        let record = ResidentRecord {
            cell,
            last_drawn_frame,
            height_range,
            terrain_step_m,
            terrain_relief_m,
            terrain_detail,
            solid_collision,
            water: Vec::new(),
            terrain_collision: None,
            road_guides: Vec::new(),
            gpu_bytes,
        };
        if record.solid_collision.is_some() {
            self.invalidate_collision_owner(cell);
        }
        self.slots[slot_index as usize].record = Some(record);
        let previous = self.by_cell.insert(CellKey::from(cell), id);
        debug_assert!(previous.is_none());
        self.len += 1;
        self.update_coverage(cell, true);
        self.membership_changed();
        self.collision_len += usize::from(
            self.slots[slot_index as usize]
                .record
                .as_ref()
                .is_some_and(|record| record.solid_collision.is_some()),
        );
        self.debug_assert_consistent();
        id
    }

    pub fn touch(&mut self, cell: CellId, frame: u64) -> Option<ResidentId> {
        let id = self.id(cell)?;
        self.touch_id(id, frame)
    }

    pub(crate) fn touch_id(&mut self, id: ResidentId, frame: u64) -> Option<ResidentId> {
        self.record_mut(id)?.last_drawn_frame = frame;
        Some(id)
    }

    pub fn finish_upload(
        &mut self,
        id: ResidentId,
        solid_collision: Option<SurfaceCollision>,
        terrain_collision: Option<TerrainCollision>,
        water: Vec<vearth_gen::water_surface::WaterTriangle>,
        road_guides: Vec<RoadGuide>,
        gpu_bytes: usize,
    ) -> bool {
        let Some(record) = self.get(id) else {
            return false;
        };
        let had_collision = record.solid_collision.is_some();
        let has_collision = solid_collision.is_some();
        let cell = record.cell;
        if had_collision != has_collision {
            self.invalidate_collision_owner(cell);
        }
        let record = self.record_mut(id).unwrap();
        record.solid_collision = solid_collision;
        record.terrain_collision = terrain_collision;
        record.water = water;
        record.road_guides = road_guides;
        record.gpu_bytes = gpu_bytes;
        self.collision_len =
            self.collision_len + usize::from(has_collision) - usize::from(had_collision);
        true
    }

    pub fn gpu_bytes(&self) -> usize {
        self.iter().map(|(_, record)| record.gpu_bytes).sum()
    }

    #[cfg(test)]
    pub(crate) fn insert_for_test(&mut self, cell: CellId, last_drawn_frame: u64) -> ResidentId {
        self.insert(cell, last_drawn_frame, (0.0, 0.0), 0.0, 0.0, None, None, 0)
    }

    #[cfg(test)]
    pub(crate) fn set_height_for_test(&mut self, cell: CellId, height_range: (f32, f32)) {
        if let Some((id, _)) = self.get_cell(cell) {
            self.get_mut(id).unwrap().height_range = height_range;
            let record = self.get(id).unwrap();
            let metric = crate::lod_selection::TerrainMetric {
                height_range,
                terrain_step_m: record.terrain_step_m,
                terrain_relief_m: record.terrain_relief_m,
                terrain_detail: record.terrain_detail.clone(),
            };
            self.terrain_metrics.upsert(cell, metric, false);
        } else {
            self.insert(cell, 0, height_range, 0.0, 0.0, None, None, 0);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_collision_for_test(&mut self, cell: CellId, collision: SurfaceCollision) {
        if let Some((id, _)) = self.get_cell(cell) {
            let record = self.get_mut(id).unwrap();
            let had_collision = record.solid_collision.is_some();
            record.solid_collision = Some(collision);
            self.collision_len += usize::from(!had_collision);
        } else {
            self.insert(cell, 0, (0.0, 0.0), 0.0, 0.0, None, Some(collision), 0);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_road_guides_for_test(&mut self, cell: CellId, road_guides: Vec<RoadGuide>) {
        if let Some((id, _)) = self.get_cell(cell) {
            self.get_mut(id)
                .expect("resident cell lookup must resolve its own ID")
                .road_guides = road_guides;
        } else {
            let id = self.insert(cell, 0, (0.0, 0.0), 0.0, 0.0, None, None, 0);
            self.get_mut(id).unwrap().road_guides = road_guides;
        }
    }

    pub fn metric(&self, cell: CellId) -> Option<TerrainMetricRef<'_>> {
        self.terrain_metrics.get(cell)
    }

    /// Only complete accepted source results survive mesh eviction. The store
    /// is cleared together with resident state when providers or world change.
    pub(crate) fn set_metric_complete(&mut self, cell: CellId, complete: bool) {
        self.terrain_metrics.set_complete(cell, complete);
    }

    pub(crate) fn begin_metric_query(&mut self) {
        self.terrain_metrics.begin_query();
    }

    pub(crate) fn finish_metric_query(&mut self) {
        let _ = self.terrain_metrics.finish_query();
    }

    pub(crate) fn protect_metric_dependencies(&mut self, cells: &[CellId]) {
        self.terrain_metrics.protect_dependencies(cells);
    }

    pub(crate) fn prune_metric_history(&mut self) {
        self.terrain_metrics.prune();
    }

    pub fn iter(&self) -> impl Iterator<Item = (ResidentId, &ResidentRecord)> {
        self.slots.iter().enumerate().filter_map(|(slot, entry)| {
            let record = entry.record.as_ref()?;
            Some((
                ResidentId {
                    slot: slot as u32,
                    generation: entry.generation,
                },
                record,
            ))
        })
    }

    pub fn ids(&self) -> impl Iterator<Item = ResidentId> + '_ {
        self.iter().map(|(id, _)| id)
    }

    pub fn cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.iter().map(|(_, record)| record.cell)
    }

    pub fn remove(&mut self, cell: CellId) -> Option<(ResidentId, ResidentRecord)> {
        let id = self.id(cell)?;
        self.remove_id(id).map(|record| (id, record))
    }

    pub fn remove_id(&mut self, id: ResidentId) -> Option<ResidentRecord> {
        let slot = self.slots.get_mut(id.slot())?;
        if slot.generation != id.generation {
            return None;
        }
        let record = slot.record.take()?;
        if record.solid_collision.is_some() {
            let cache = self.collision_regions.get_mut();
            if cache.as_ref().is_some_and(|cache| {
                cache.cover.is_none_or(|cover| {
                    CellKey::from(cover)
                        .range()
                        .overlaps(CellKey::from(record.cell).range())
                })
            }) {
                *cache = None;
            }
        }
        self.terrain_metrics.remove_resident(record.cell);
        let removed = self.by_cell.remove(&CellKey::from(record.cell));
        debug_assert_eq!(removed, Some(id));
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(id.slot);
        self.len -= 1;
        self.update_coverage(record.cell, false);
        self.membership_changed();
        self.collision_len -= usize::from(record.solid_collision.is_some());
        self.debug_assert_consistent();
        Some(record)
    }

    pub fn clear(&mut self) -> Vec<(ResidentId, ResidentRecord)> {
        let ids: Vec<_> = self.ids().collect();
        let removed = ids
            .into_iter()
            .filter_map(|id| self.remove_id(id).map(|record| (id, record)))
            .collect();
        self.terrain_metrics.clear();
        removed
    }

    fn debug_assert_consistent(&self) {
        debug_assert_eq!(self.by_cell.len(), self.len);
        debug_assert_eq!(
            self.slots
                .iter()
                .filter(|slot| slot.record.is_some())
                .count(),
            self.len
        );
        debug_assert_eq!(
            self.slots
                .iter()
                .filter(|slot| {
                    slot.record
                        .as_ref()
                        .is_some_and(|record| record.solid_collision.is_some())
                })
                .count(),
            self.collision_len
        );
        debug_assert!(self.by_cell.iter().all(|(key, id)| {
            self.get(*id)
                .is_some_and(|record| CellKey::from(record.cell) == *key)
        }));
    }

    /// Max terrain elevation of the finest resident cell containing `dir`
    /// (a geocentric direction): the camera's ground-collision reference.
    pub fn terrain_top_under(&self, dir: DVec3) -> Option<f32> {
        for level in (MIN_RENDER_LEVEL..=MAX_STREAM_LEVEL).rev() {
            let cell = CellId::from_dir(dir, level);
            if let Some((_, record)) = self.get_cell(cell) {
                return Some(record.height_range.1);
            }
        }
        None
    }

    /// Height of the rendered ground or highest continuous solid, in metres
    /// above the owning cell anchor.
    pub fn terrain_surface_at(&self, dir: DVec3) -> Option<f64> {
        for level in (COLLISION_MIN_LEVEL..=MAX_STREAM_LEVEL).rev() {
            let cell = CellId::from_dir(dir, level);
            let Some((_, record)) = self.get_cell(cell) else {
                continue;
            };
            let Some(collision) = &record.solid_collision else {
                continue;
            };
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let local = frame.ecef_to_local(dir.normalize() * frame.anchor.length());
            let metric =
                MetricFrame::of(&frame).point_from_ecef(dir.normalize() * frame.anchor.length());
            let ground = record
                .terrain_collision
                .as_ref()
                .and_then(|surface| surface.sample_local(local.x, local.z))
                .map(|height| height * frame.grid_scale.y);
            return ground
                .into_iter()
                .chain(collision.top(metric[0], metric[2]))
                .reduce(f64::max);
        }
        None
    }

    /// Nearest road centerline in the finest resident street-level cell.
    /// Candidate direction is oriented with `forward`; strongly transverse
    /// crossings are rejected so riding off-road across a street does not
    /// unexpectedly capture the Sparrow.
    pub(crate) fn nearest_road_guide(
        &self,
        p: DVec3,
        forward: DVec3,
        capture_margin_m: f64,
        turn_lookahead_m: f64,
    ) -> Option<RoadGuideHit> {
        let dir = p.normalize_or(DVec3::X);
        for level in (vearth_gen::ROAD_GUIDE_MIN_LEVEL..=MAX_STREAM_LEVEL).rev() {
            let cell = CellId::from_dir(dir, level);
            let Some((_, record)) = self.get_cell(cell) else {
                continue;
            };
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let up = frame.y_axis;
            let forward = (forward - up * forward.dot(up)).normalize_or(frame.z_axis);
            let mut best: Option<(f64, RoadGuideHit)> = None;
            for guide in &record.road_guides {
                let a = frame.local_to_ecef(DVec3::new(
                    f64::from(guide.a_local[0]),
                    0.0,
                    f64::from(guide.a_local[1]),
                ));
                let b = frame.local_to_ecef(DVec3::new(
                    f64::from(guide.b_local[0]),
                    0.0,
                    f64::from(guide.b_local[1]),
                ));
                let segment = b - a;
                let tangent = segment - up * segment.dot(up);
                let length_sq = tangent.length_squared();
                if length_sq < 0.25 {
                    continue;
                }
                let rel = p - a;
                let rel_h = rel - up * rel.dot(up);
                let t = (rel_h.dot(tangent) / length_sq).clamp(0.0, 1.0);
                let nearest = a + tangent * t;
                let mut center_offset = nearest - p;
                center_offset -= up * center_offset.dot(up);
                let distance_m = center_offset.length();
                let width_m = f64::from(guide.width_m);
                if distance_m > width_m * 0.5 + capture_margin_m {
                    continue;
                }
                let segment_length = length_sq.sqrt();
                let mut tangent = tangent / segment_length;
                let alignment = tangent.dot(forward);
                if alignment.abs() < 70.0_f64.to_radians().cos() {
                    continue;
                }
                let (exit_local, distance_to_exit) = if alignment < 0.0 {
                    tangent = -tangent;
                    (guide.a_exit, t * segment_length)
                } else {
                    (guide.b_exit, (1.0 - t) * segment_length)
                };
                let alignment = tangent.dot(forward).clamp(0.0, 1.0);
                let exit = (frame.x_axis * f64::from(exit_local[0])
                    + frame.z_axis * f64::from(exit_local[1]))
                .normalize_or(DVec3::ZERO);
                if exit != DVec3::ZERO && turn_lookahead_m > 1.0e-3 {
                    let proximity = (1.0 - distance_to_exit / turn_lookahead_m).clamp(0.0, 1.0);
                    let turn_mix = proximity * proximity * (3.0 - 2.0 * proximity);
                    tangent = (tangent * (1.0 - turn_mix) + exit * turn_mix).normalize_or(tangent);
                }
                let assist = f64::from(guide.assist);
                let score = (distance_m - width_m * 0.5).max(0.0)
                    + distance_m * 0.1
                    + (1.0 - alignment) * 8.0
                    + (1.0 - assist) * 2.0;
                let hit = RoadGuideHit {
                    tangent,
                    center_offset,
                    distance_m,
                    width_m,
                    assist,
                };
                if best
                    .as_ref()
                    .is_none_or(|(best_score, _)| score < *best_score)
                {
                    best = Some((score, hit));
                }
            }
            return best.map(|(_, hit)| hit);
        }
        None
    }

    /// Solidity in the finest available continuous collision field.
    pub fn solid_at(&self, p: DVec3, memo: &mut HashMap<CellId, ChunkFrame>) -> Option<bool> {
        for level in (COLLISION_MIN_LEVEL..=MAX_STREAM_LEVEL).rev() {
            let cell = CellId::from_dir(p, level);
            let Some((_, record)) = self.get_cell(cell) else {
                continue;
            };
            let Some(collision) = &record.solid_collision else {
                continue;
            };
            let frame = memo
                .entry(cell)
                .or_insert_with(|| ChunkFrame::of(ChunkAddr { cell, k: 0 }));
            let metric = MetricFrame::of(frame).point_from_ecef(p);
            if collision.contains(metric[0], metric[1], metric[2]) {
                return Some(true);
            }
            let local = frame.ecef_to_local(p);
            return Some(
                record
                    .terrain_collision
                    .as_ref()
                    .and_then(|surface| surface.sample_local(local.x, local.z))
                    .is_some_and(|height| local.y < height),
            );
        }
        None
    }

    #[cfg(test)]
    fn ensure_collision_regions(&self) {
        self.ensure_collision_regions_for_cover(None);
    }

    fn ensure_collision_regions_for_cover(&self, cover: Option<CellId>) {
        if self
            .collision_regions
            .borrow()
            .as_ref()
            .is_some_and(|cache| cache.cover == cover)
        {
            return;
        }
        let regions = self.build_collision_regions(cover);
        *self.collision_regions.borrow_mut() = Some(CollisionRegionCache { cover, regions });
    }

    fn build_collision_regions(&self, cover: Option<CellId>) -> Vec<CollisionRegion> {
        let owners: HashMap<_, _> = self
            .iter()
            .filter(|(_, r)| r.solid_collision.is_some())
            .filter(|(_, r)| {
                cover.is_none_or(|cover| {
                    CellKey::from(cover)
                        .range()
                        .overlaps(CellKey::from(r.cell).range())
                })
            })
            .map(|(id, r)| (r.cell, id))
            .collect();
        let mut split = HashSet::new();
        let mut roots = BTreeSet::new();
        for &cell in owners.keys() {
            let mut ancestor = cell;
            while ancestor.level > COLLISION_MIN_LEVEL {
                ancestor = ancestor.parent().unwrap();
                split.insert(ancestor);
            }
            roots.insert(ancestor);
        }
        fn walk(
            store: &ResidentStore,
            owners: &HashMap<CellId, ResidentId>,
            split: &HashSet<CellId>,
            cell: CellId,
            inherited: Option<ResidentId>,
            cover: Option<CellId>,
            out: &mut Vec<CollisionRegion>,
        ) {
            if cover.is_some_and(|cover| {
                !CellKey::from(cover)
                    .range()
                    .overlaps(CellKey::from(cell).range())
            }) {
                return;
            }
            let owner = owners.get(&cell).copied().or(inherited);
            if split.contains(&cell) {
                for child in cell.children() {
                    walk(store, owners, split, child, owner, cover, out);
                }
            } else if let Some(owner) = owner {
                // An inherited coarse owner may extend outside the cached
                // neighbourhood. Its queried piece retains the same owner.
                let cell = cover
                    .filter(|cover| cover.level > cell.level)
                    .unwrap_or(cell);
                let frame = ChunkFrame::of(ChunkAddr {
                    cell: store.get(owner).unwrap().cell,
                    k: 0,
                });
                out.push(CollisionRegion {
                    owner,
                    range: CellKey::from(cell).range(),
                    metric: MetricFrame::of(&frame),
                    planes: collision_region_planes(cell, &frame),
                });
            }
        }
        let mut regions = Vec::new();
        for root in roots {
            walk(self, &owners, &split, root, None, cover, &mut regions);
        }
        regions.sort_unstable_by_key(|region| region.range.start());
        debug_assert!(
            regions
                .windows(2)
                .all(|pair| pair[0].range.end() <= pair[1].range.start())
        );
        regions
    }

    /// Test the swept ENU box in disjoint finest-resident coverage regions.
    /// Parent triangles are clipped away exactly where finer data is resident.
    pub fn box_hit(
        &self,
        center: DVec3,
        half_axes: [DVec3; 3],
        memo: &mut HashMap<CellId, ChunkFrame>,
    ) -> Option<bool> {
        self.box_hit_with_broad_phase::<true>(center, half_axes, memo)
    }

    fn box_hit_with_broad_phase<const ENABLED: bool>(
        &self,
        center: DVec3,
        half_axes: [DVec3; 3],
        memo: &mut HashMap<CellId, ChunkFrame>,
    ) -> Option<bool> {
        let corners: [DVec3; 8] = std::array::from_fn(|i| {
            center
                + half_axes[0] * if i & 1 == 0 { -1. } else { 1. }
                + half_axes[1] * if i & 2 == 0 { -1. } else { 1. }
                + half_axes[2] * if i & 4 == 0 { -1. } else { 1. }
        });
        let mut any = false;
        // Closed solid or ground interiors may contain the whole box without
        // crossing its boundary; point lookup already selects the finest cell.
        for point in corners.into_iter().chain([center]) {
            match self.solid_at(point, memo) {
                Some(true) => return Some(true),
                Some(false) => any = true,
                None => {}
            }
        }
        // The narrow phase encloses the swept box in each owner's orthogonal
        // metric AABB. This ECEF cube contains every such AABB, including the
        // volume beyond the original corners introduced by that enclosure.
        let radius = half_axes.iter().map(|axis| axis.length()).sum::<f64>() * 3.0f64.sqrt();
        let envelope: [DVec3; 8] = std::array::from_fn(|i| {
            center
                + DVec3::new(
                    if i & 1 == 0 { -radius } else { radius },
                    if i & 2 == 0 { -radius } else { radius },
                    if i & 4 == 0 { -radius } else { radius },
                )
        });
        let cover = ENABLED.then(|| collision_cache_cover(&envelope)).flatten();
        self.ensure_collision_regions_for_cover(cover);
        let regions = self.collision_regions.borrow();
        let regions = &regions.as_ref().unwrap().regions;
        let candidates = if ENABLED {
            collision_region_candidates(regions, &envelope)
        } else {
            regions
        };
        for region in candidates {
            let points = corners.map(|point| region.metric.point_from_ecef(point));
            if region.planes.iter().any(|p| {
                points
                    .iter()
                    .all(|q| p[0] * q[0] + p[1] * q[1] + p[2] * q[2] + p[3] < -1e-7)
            }) {
                continue;
            }
            any = true;
            let mut min = [f64::INFINITY; 3];
            let mut max = [f64::NEG_INFINITY; 3];
            for point in points {
                for axis in 0..3 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
            let record = self.get(region.owner).unwrap();
            let collision = record.solid_collision.as_ref().unwrap();
            if collision.intersects_region(min, max, &region.planes)
                || record.terrain_collision.as_ref().is_some_and(|terrain| {
                    let frame = memo.entry(record.cell).or_insert_with(|| {
                        ChunkFrame::of(ChunkAddr {
                            cell: record.cell,
                            k: 0,
                        })
                    });
                    terrain.intersects_region(frame, min, max, &region.planes)
                })
            {
                return Some(true);
            }
        }
        any.then_some(false)
    }

    /// Clip the ray to disjoint finest-resident cell cones before querying its
    /// collider, so a superseded first parent hit cannot hide a later valid hit.
    pub fn raycast(
        &self,
        origin: DVec3,
        direction: DVec3,
        max_t: f64,
        memo: &mut HashMap<CellId, ChunkFrame>,
    ) -> Option<f64> {
        self.raycast_with_broad_phase::<true>(origin, direction, max_t, memo)
    }

    fn raycast_with_broad_phase<const ENABLED: bool>(
        &self,
        origin: DVec3,
        direction: DVec3,
        max_t: f64,
        memo: &mut HashMap<CellId, ChunkFrame>,
    ) -> Option<f64> {
        let endpoints = [origin, origin + direction * max_t];
        let cover = (ENABLED && max_t.is_finite() && max_t >= 0.0)
            .then(|| collision_cache_cover(&endpoints))
            .flatten();
        self.ensure_collision_regions_for_cover(cover);
        let regions = self.collision_regions.borrow();
        let regions = &regions.as_ref().unwrap().regions;
        let candidates = if ENABLED && max_t.is_finite() && max_t >= 0.0 {
            collision_region_candidates(regions, &endpoints)
        } else {
            regions
        };
        let mut nearest: Option<f64> = None;
        for region in candidates {
            let start = region.metric.point_from_ecef(origin);
            let dir = region.metric.direction_from_ecef(direction);
            let mut low = 0f64;
            let mut high = nearest.unwrap_or(max_t);
            let mut outside = false;
            for p in region.planes {
                let value = p[0] * start[0] + p[1] * start[1] + p[2] * start[2] + p[3];
                let slope = p[0] * dir[0] + p[1] * dir[1] + p[2] * dir[2];
                if slope.abs() < 1e-15 {
                    if value < -1e-7 {
                        outside = true;
                        break;
                    }
                } else if slope > 0. {
                    low = low.max(-value / slope);
                } else {
                    high = high.min(-value / slope);
                }
            }
            if outside || low > high {
                continue;
            }
            let start = std::array::from_fn(|i| start[i] + dir[i] * low);
            let record = self.get(region.owner).unwrap();
            let solid_hit =
                record
                    .solid_collision
                    .as_ref()
                    .unwrap()
                    .raycast(start, dir, high - low);
            let terrain_hit = record.terrain_collision.as_ref().and_then(|terrain| {
                let frame = memo.entry(record.cell).or_insert_with(|| {
                    ChunkFrame::of(ChunkAddr {
                        cell: record.cell,
                        k: 0,
                    })
                });
                terrain.raycast(frame, start, dir, solid_hit.unwrap_or(high - low))
            });
            if let Some(t) = solid_hit.into_iter().chain(terrain_hit).reduce(f64::min) {
                nearest = Some(low + t);
            }
        }
        nearest
    }

    /// Resolve the wanted cut into the set of cells to draw. Two rules:
    ///
    /// 1. **Fine content always draws.** Every resident cell on the path
    ///    from the roots to the wanted leaves is reachable; arriving leaves
    ///    show up (and dither in) the frame they land — refinement is
    ///    progressive, never gated on siblings.
    /// 2. **Backdrops leave only when fully covered.** A resident cell
    ///    whose child subtrees aren't all *ready* (resident, or covered by
    ///    ready descendants) stays in the cut underneath the finer content.
    ///    It exits on exactly the frame its last child subtree becomes
    ///    ready, so its fade-out pairs with the final arrival's fade-in and
    ///    the swap stays seamless.
    ///
    /// Missing leaves use a complete finer resident cut during merge when one
    /// exists; otherwise their deepest resident ancestor covers any gaps while
    /// partial finer arrivals still draw. Missing cells come back with their
    /// generation priority — view lane first, then screen error — in
    /// `wanted` order.
    pub fn drawable_cut(&self, wanted: &[WantedCell]) -> (Vec<CellId>, Vec<(CellId, f64)>) {
        let mut arena = DrawableCutArena::new();
        let mut draw = Vec::new();
        let mut missing = Vec::new();
        self.resolve_drawable_cut_into(wanted, &mut arena, &mut draw, &mut missing);
        (draw, missing)
    }

    /// Reuse coverage while ordered wanted cells and resident membership match.
    /// Missing priorities always reflect the current wanted cells; caller edits
    /// to either output buffer cannot alter the cached result.
    /// Returns whether resolution was rebuilt. State derived from the cut can
    /// reuse this result until the next rebuild; resetting such state must also
    /// invalidate the arena.
    pub fn drawable_cut_into(
        &self,
        wanted: &[WantedCell],
        arena: &mut DrawableCutArena,
        draw: &mut Vec<CellId>,
        missing: &mut Vec<(CellId, f64)>,
    ) -> bool {
        let resolved = &arena.resolved;
        if resolved.membership_revision == self.membership_revision
            && resolved
                .store_identity
                .as_ref()
                .is_some_and(|identity| Arc::ptr_eq(identity, &self.membership_identity))
            && resolved
                .wanted
                .iter()
                .copied()
                .eq(wanted.iter().map(|wanted| wanted.cell))
        {
            draw.clear();
            draw.extend_from_slice(&resolved.draw);
            missing.clear();
            missing.extend(resolved.missing_indices.iter().map(|&index| {
                let wanted = &wanted[index];
                (wanted.cell, wanted.stream_priority())
            }));
            return false;
        }

        self.resolve_drawable_cut_into(wanted, arena, draw, missing);
        #[cfg(test)]
        {
            arena.rebuilds += 1;
        }
        let resolved = &mut arena.resolved;
        resolved.store_identity = Some(self.membership_identity.clone());
        resolved.membership_revision = self.membership_revision;
        resolved.wanted.clear();
        resolved
            .wanted
            .extend(wanted.iter().map(|wanted| wanted.cell));
        resolved.draw.clone_from(draw);
        resolved.missing_indices.clear();
        // Missing leaves are an ordered subsequence of wanted, so match their
        // indices once without another resident lookup for every wanted cell.
        let mut next_missing = 0;
        for (index, wanted) in wanted.iter().enumerate() {
            if missing
                .get(next_missing)
                .is_some_and(|(cell, _)| *cell == wanted.cell)
            {
                resolved.missing_indices.push(index);
                next_missing += 1;
            }
        }
        debug_assert_eq!(next_missing, missing.len());
        true
    }

    fn resolve_drawable_cut_into(
        &self,
        wanted: &[WantedCell],
        arena: &mut DrawableCutArena,
        draw: &mut Vec<CellId>,
        missing: &mut Vec<(CellId, f64)>,
    ) {
        arena.prepare(wanted);
        draw.clear();
        missing.clear();
        if draw.capacity() < wanted.len() {
            draw.reserve(wanted.len());
        }
        if missing.capacity() < wanted.len() {
            missing.reserve(wanted.len());
        }
        missing.extend(
            wanted
                .iter()
                .filter(|w| !self.contains(w.cell))
                .map(|w| (w.cell, w.stream_priority())),
        );
        for &root in &arena.roots {
            self.descend(
                root,
                &arena.is_leaf,
                &mut arena.ready_memo,
                &mut arena.seen,
                draw,
            );
        }
    }

    /// Is this node of the wanted tree coverable without drawing anything
    /// coarser: resident itself, or (internal) all its in-tree children
    /// ready? Children absent from the tree are horizon-culled — vacuously
    /// ready, they are not drawn at all.
    fn subtree_ready(
        &self,
        cell: CellId,
        is_leaf: &HashMap<CellKey, bool>,
        memo: &mut HashMap<CellKey, bool>,
    ) -> bool {
        let key = CellKey::from(cell);
        if let Some(&v) = memo.get(&key) {
            return v;
        }
        let v = if self.contains(cell) {
            true
        } else {
            match is_leaf.get(&key) {
                Some(&true) if cell.level < MAX_STREAM_LEVEL => cell
                    .children()
                    .into_iter()
                    .all(|child| self.resident_subtree_ready(child)),
                Some(&false) => cell.children().into_iter().all(|child| {
                    !is_leaf.contains_key(&CellKey::from(child))
                        || self.subtree_ready(child, is_leaf, memo)
                }),
                _ => false,
            }
        };
        memo.insert(key, v);
        v
    }

    fn descend(
        &self,
        cell: CellId,
        is_leaf: &HashMap<CellKey, bool>,
        ready_memo: &mut HashMap<CellKey, bool>,
        seen: &mut HashSet<CellKey>,
        draw: &mut Vec<CellId>,
    ) {
        let key = CellKey::from(cell);
        match is_leaf.get(&key) {
            None => {} // horizon-culled subtree: nothing wanted here
            Some(&true) => {
                if self.contains(cell) {
                    if seen.insert(key) {
                        draw.push(cell);
                    }
                    return;
                }
                self.draw_missing_leaf(cell, seen, draw);
            }
            Some(&false) => {
                let children = cell.children();
                let kids_ready = children.iter().all(|ch| {
                    !is_leaf.contains_key(&CellKey::from(*ch))
                        || self.subtree_ready(*ch, is_leaf, ready_memo)
                });
                // Keep the backdrop while any child subtree is unready, but
                // continue descending so resident detail appears progressively.
                if !kids_ready && self.contains(cell) && seen.insert(key) {
                    draw.push(cell);
                }
                for ch in children {
                    self.descend(ch, is_leaf, ready_memo, seen, draw);
                }
            }
        }
    }

    /// Draw the best resident representation of a missing wanted leaf.
    ///
    /// Refinement already has ancestor fallback; merge needs the symmetric
    /// case. Use the nearest resident descendants while the wanted parent
    /// streams; finer cached meshes below a covering resident are outside the
    /// requested cut. If even one quadrant has a gap, retain the deepest
    /// resident ancestor as backdrop and draw available descendant coverage
    /// over it. One arbitrary descendant never covers the whole leaf.
    fn draw_missing_leaf(&self, cell: CellId, seen: &mut HashSet<CellKey>, draw: &mut Vec<CellId>) {
        let descendants_cover = cell.level < MAX_STREAM_LEVEL
            && cell
                .children()
                .into_iter()
                .all(|child| self.resident_subtree_ready(child));

        if !descendants_cover {
            self.draw_deepest_resident_ancestor(cell, seen, draw);
        }

        if cell.level < MAX_STREAM_LEVEL {
            for child in cell.children() {
                self.descend_resident_subtree(child, seen, draw);
            }
        }
    }

    fn draw_deepest_resident_ancestor(
        &self,
        cell: CellId,
        seen: &mut HashSet<CellKey>,
        draw: &mut Vec<CellId>,
    ) {
        let mut cur = cell;
        while let Some(parent) = cur.parent() {
            cur = parent;
            if self.contains(cur) {
                if seen.insert(CellKey::from(cur)) {
                    draw.push(cur);
                }
                break;
            }
            if cur.level < MIN_RENDER_LEVEL {
                break;
            }
        }
    }

    fn resident_subtree_ready(&self, cell: CellId) -> bool {
        self.coverage
            .get(&CellKey::from(cell))
            .is_some_and(|node| node.complete)
    }

    /// Descend below a missing wanted leaf only until a resident covers each
    /// branch. This preserves coverage without resurrecting finer cached cuts
    /// that the camera has already merged past. Progressive refinement above
    /// wanted leaves remains the responsibility of `descend`.
    fn descend_resident_subtree(
        &self,
        cell: CellId,
        seen: &mut HashSet<CellKey>,
        draw: &mut Vec<CellId>,
    ) {
        let key = CellKey::from(cell);
        if !self.coverage.contains_key(&key) {
            return;
        }

        if self.contains(cell) {
            if seen.insert(key) {
                draw.push(cell);
            }
            return;
        }

        if cell.level < MAX_STREAM_LEVEL {
            for child in cell.children() {
                self.descend_resident_subtree(child, seen, draw);
            }
        }
    }

    /// Cells to evict once over capacity: oldest-drawn first.
    pub fn eviction_victims(&self, now_frame: u64) -> Vec<CellId> {
        if self.len() <= RESIDENT_CAP {
            return Vec::new();
        }
        let mut by_age: Vec<_> = self
            .iter()
            .map(|(_, record)| record)
            .filter(|record| record.last_drawn_frame + 2 < now_frame) // never evict just-drawn
            // Pin coarse levels: they're few, cheap, cover the whole planet,
            // and evicting them is what made zooming out reload the globe.
            .filter(|record| record.cell.level > 6)
            .map(|record| (record.last_drawn_frame, record.cell))
            .collect();
        by_age.sort_unstable();
        by_age
            .into_iter()
            .take(self.len() - RESIDENT_CAP + RESIDENT_CAP / 10)
            .map(|(_, c)| c)
            .collect()
    }
}

#[cfg(test)]
#[derive(Default)]
struct ReferenceDrawableCutArena {
    is_leaf: HashMap<CellKey, bool>,
    resident_tree: HashSet<CellKey>,
    missing_cells: HashSet<CellKey>,
    ready_memo: HashMap<CellKey, bool>,
    resident_ready_memo: HashMap<CellKey, bool>,
    seen: HashSet<CellKey>,
    visited_roots: HashSet<CellKey>,
}
#[cfg(test)]
impl ReferenceDrawableCutArena {
    fn clear(&mut self) {
        self.is_leaf.clear();
        self.resident_tree.clear();
        self.missing_cells.clear();
        self.ready_memo.clear();
        self.resident_ready_memo.clear();
        self.seen.clear();
        self.visited_roots.clear();
    }
}

#[cfg(test)]
impl ResidentStore {
    // This independent full-scan resolver is the ordering and coverage oracle.
    fn reference_drawable_cut(&self, wanted: &[WantedCell]) -> (Vec<CellId>, Vec<(CellId, f64)>) {
        let mut arena = ReferenceDrawableCutArena::default();
        let mut draw = Vec::new();
        let mut missing = Vec::new();
        self.reference_resolve_drawable_cut_into(wanted, &mut arena, &mut draw, &mut missing);
        (draw, missing)
    }
    fn reference_resolve_drawable_cut_into(
        &self,
        wanted: &[WantedCell],
        arena: &mut ReferenceDrawableCutArena,
        draw: &mut Vec<CellId>,
        missing: &mut Vec<(CellId, f64)>,
    ) {
        arena.clear();
        draw.clear();
        missing.clear();
        if draw.capacity() < wanted.len() {
            draw.reserve(wanted.len());
        }
        if missing.capacity() < wanted.len() {
            missing.reserve(wanted.len());
        }
        missing.extend(
            wanted
                .iter()
                .filter(|w| !self.contains(w.cell))
                .map(|w| (w.cell, w.stream_priority())),
        );

        for w in wanted {
            arena.is_leaf.insert(CellKey::from(w.cell), true);
            let mut cur = w.cell;
            while cur.level > MIN_RENDER_LEVEL {
                let parent = cur.parent().unwrap();
                if arena.is_leaf.insert(CellKey::from(parent), false).is_some() {
                    break; // ancestors above are already recorded
                }
                cur = parent;
            }
        }

        if !missing.is_empty() {
            arena
                .missing_cells
                .extend(missing.iter().map(|(cell, _)| CellKey::from(*cell)));
            for resident in self.cells() {
                let mut cur = resident;
                let missing_ancestor = loop {
                    if arena.missing_cells.contains(&CellKey::from(cur)) {
                        break Some(cur);
                    }
                    if cur.level <= MIN_RENDER_LEVEL {
                        break None;
                    }
                    cur = cur.parent().unwrap();
                };
                let Some(stop) = missing_ancestor else {
                    continue;
                };
                let mut path = resident;
                loop {
                    arena.resident_tree.insert(CellKey::from(path));
                    if path == stop {
                        break;
                    }
                    path = path.parent().unwrap();
                }
            }
        }

        for w in wanted {
            let mut root = w.cell;
            while root.level > MIN_RENDER_LEVEL {
                root = root.parent().unwrap();
            }
            if arena.visited_roots.insert(CellKey::from(root)) {
                self.reference_descend(
                    root,
                    &arena.is_leaf,
                    &arena.resident_tree,
                    &mut arena.ready_memo,
                    &mut arena.resident_ready_memo,
                    &mut arena.seen,
                    draw,
                );
            }
        }
    }

    fn reference_subtree_ready(
        &self,
        cell: CellId,
        is_leaf: &HashMap<CellKey, bool>,
        resident_tree: &HashSet<CellKey>,
        memo: &mut HashMap<CellKey, bool>,
        resident_memo: &mut HashMap<CellKey, bool>,
    ) -> bool {
        let key = CellKey::from(cell);
        if let Some(&v) = memo.get(&key) {
            return v;
        }
        let v = if self.contains(cell) {
            true
        } else {
            match is_leaf.get(&key) {
                Some(&true) if cell.level < MAX_STREAM_LEVEL => {
                    cell.children().into_iter().all(|child| {
                        self.reference_resident_subtree_ready(child, resident_tree, resident_memo)
                    })
                }
                Some(&false) => cell.children().into_iter().all(|child| {
                    !is_leaf.contains_key(&CellKey::from(child))
                        || self.reference_subtree_ready(
                            child,
                            is_leaf,
                            resident_tree,
                            memo,
                            resident_memo,
                        )
                }),
                _ => false,
            }
        };
        memo.insert(key, v);
        v
    }

    fn reference_descend(
        &self,
        cell: CellId,
        is_leaf: &HashMap<CellKey, bool>,
        resident_tree: &HashSet<CellKey>,
        ready_memo: &mut HashMap<CellKey, bool>,
        resident_ready_memo: &mut HashMap<CellKey, bool>,
        seen: &mut HashSet<CellKey>,
        draw: &mut Vec<CellId>,
    ) {
        let key = CellKey::from(cell);
        match is_leaf.get(&key) {
            None => {} // horizon-culled subtree: nothing wanted here
            Some(&true) => {
                if self.contains(cell) {
                    if seen.insert(key) {
                        draw.push(cell);
                    }
                    return;
                }
                self.reference_draw_missing_leaf(
                    cell,
                    resident_tree,
                    resident_ready_memo,
                    seen,
                    draw,
                );
            }
            Some(&false) => {
                let children = cell.children();
                let kids_ready = children.iter().all(|ch| {
                    !is_leaf.contains_key(&CellKey::from(*ch))
                        || self.reference_subtree_ready(
                            *ch,
                            is_leaf,
                            resident_tree,
                            ready_memo,
                            resident_ready_memo,
                        )
                });
                if !kids_ready && self.contains(cell) && seen.insert(key) {
                    draw.push(cell);
                }
                for ch in children {
                    self.reference_descend(
                        ch,
                        is_leaf,
                        resident_tree,
                        ready_memo,
                        resident_ready_memo,
                        seen,
                        draw,
                    );
                }
            }
        }
    }

    fn reference_draw_missing_leaf(
        &self,
        cell: CellId,
        resident_tree: &HashSet<CellKey>,
        descendant_ready: &mut HashMap<CellKey, bool>,
        seen: &mut HashSet<CellKey>,
        draw: &mut Vec<CellId>,
    ) {
        let descendants_cover = cell.level < MAX_STREAM_LEVEL
            && cell.children().into_iter().all(|child| {
                self.reference_resident_subtree_ready(child, resident_tree, descendant_ready)
            });

        if !descendants_cover {
            self.reference_draw_deepest_resident_ancestor(cell, seen, draw);
        }

        if cell.level < MAX_STREAM_LEVEL {
            for child in cell.children() {
                self.reference_descend_resident_subtree(child, resident_tree, seen, draw);
            }
        }
    }

    fn reference_draw_deepest_resident_ancestor(
        &self,
        cell: CellId,
        seen: &mut HashSet<CellKey>,
        draw: &mut Vec<CellId>,
    ) {
        let mut cur = cell;
        while let Some(parent) = cur.parent() {
            cur = parent;
            if self.contains(cur) {
                if seen.insert(CellKey::from(cur)) {
                    draw.push(cur);
                }
                break;
            }
            if cur.level < MIN_RENDER_LEVEL {
                break;
            }
        }
    }

    fn reference_resident_subtree_ready(
        &self,
        cell: CellId,
        resident_tree: &HashSet<CellKey>,
        memo: &mut HashMap<CellKey, bool>,
    ) -> bool {
        let key = CellKey::from(cell);
        if let Some(&ready) = memo.get(&key) {
            return ready;
        }
        let ready = if self.contains(cell) {
            true
        } else if cell.level >= MAX_STREAM_LEVEL || !resident_tree.contains(&key) {
            false
        } else {
            cell.children()
                .into_iter()
                .all(|child| self.reference_resident_subtree_ready(child, resident_tree, memo))
        };
        memo.insert(key, ready);
        ready
    }

    fn reference_descend_resident_subtree(
        &self,
        cell: CellId,
        resident_tree: &HashSet<CellKey>,
        seen: &mut HashSet<CellKey>,
        draw: &mut Vec<CellId>,
    ) {
        let key = CellKey::from(cell);
        if !resident_tree.contains(&key) {
            return;
        }

        if self.contains(cell) {
            if seen.insert(key) {
                draw.push(cell);
            }
            return;
        }

        if cell.level < MAX_STREAM_LEVEL {
            for child in cell.children() {
                self.reference_descend_resident_subtree(child, resident_tree, seen, draw);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_leaf_index_insertions_are_not_debug_only() {
        let cell = CellId {
            face: Face(2),
            level: MIN_RENDER_LEVEL,
            i: 3,
            j: 5,
        };
        let leaves = [WantedCell {
            cell,
            screen_px: 1.0,
            lane: ViewLane::Visible,
        }];
        let index = LodLeafIndex::new(&leaves);
        assert!(index.contains(cell));
        assert_eq!(index.by_cell.get(&CellKey::from(cell)), Some(&0));
        assert_eq!(
            index
                .by_range_start
                .get(&CellKey::from(cell).range().start()),
            Some(&CellKey::from(cell))
        );
    }

    fn resident_test_cell(index: u32) -> CellId {
        CellId {
            face: Face((index % u32::from(Face::COUNT)) as u8),
            level: 8,
            i: (index / u32::from(Face::COUNT)) & 0xff,
            j: ((index * 37) / u32::from(Face::COUNT)) & 0xff,
        }
    }

    #[test]
    fn resident_ids_are_compact_and_stale_ids_never_resolve() {
        assert_eq!(std::mem::size_of::<ResidentId>(), 8);
        let mut store = ResidentStore::new();
        let first_cell = resident_test_cell(1);
        let first = store.insert(first_cell, 7, (10.0, 20.0), 3.0, 0.0, None, None, 400);
        assert_eq!(store.id(first_cell), Some(first));
        assert_eq!(store.get(first).unwrap().gpu_bytes, 400);
        assert_eq!(store.touch_id(first, 8), Some(first));
        assert_eq!(store.get(first).unwrap().last_drawn_frame, 8);

        let removed = store.remove_id(first).unwrap();
        assert_eq!(removed.cell, first_cell);
        assert!(store.get(first).is_none());
        assert!(store.id(first_cell).is_none());
        assert_eq!(store.touch_id(first, 90), None);

        let second_cell = resident_test_cell(2);
        let second = store.insert(second_cell, 9, (30.0, 40.0), 5.0, 0.0, None, None, 600);
        assert_eq!(second.slot, first.slot, "freed slot should be reused");
        assert_ne!(second.generation, first.generation);
        assert!(store.get(first).is_none());
        assert_eq!(store.get(second).unwrap().cell, second_cell);
        assert_eq!(store.touch_id(first, 100), None);
        assert_eq!(store.get(second).unwrap().last_drawn_frame, 9);
        assert_eq!(store.touch_id(second, 3), Some(second));
        assert_eq!(store.get(second).unwrap().last_drawn_frame, 3);
        store.clear();
        assert_eq!(store.touch_id(second, 101), None);
    }

    #[test]
    fn resident_relief_replacement_keeps_height_and_ownership_independent() {
        let cell = resident_test_cell(1);
        let mut store = ResidentStore::new();
        let range = (-80_000.0, 1_000.0);
        let id = store.insert(cell, 7, range, 0.0, 0.0, None, None, 400);
        assert_eq!(store.metric(cell), Some((range, 0.0, 0.0).into()));
        assert_eq!(store.terrain_top_under(cell.center_dir()), Some(range.1));
        assert_eq!(
            store.insert(cell, 8, range, 10.0, 700.0, None, None, 400),
            id
        );
        assert_eq!(store.metric(cell), Some((range, 10.0, 700.0).into()));
        assert_eq!(store.get(id).unwrap().terrain_relief_m, 700.0);
        assert_eq!(store.coverage[&CellKey::from(cell)].members, 1);
        let inherited = terrain_metrics_for(cell.children()[0], Some(&|cell| store.metric(cell)));
        assert_eq!(inherited, Some((range, 10.0, 700.0, cell)));
        assert_eq!(
            padded_height_range(inherited),
            padded_height_range(Some((range, 10.0, 0.0, cell))),
        );
        store.remove_id(id).unwrap();
        assert_eq!(store.metric(cell), None);
    }

    #[test]
    fn resident_generation_wrap_skips_zero_and_invalidates_the_old_id() {
        let mut store = ResidentStore::new();
        let cell = resident_test_cell(3);
        let id = store.insert_for_test(cell, 0);
        store.slots[id.slot()].generation = u32::MAX;
        let wrapped = ResidentId {
            slot: id.slot,
            generation: u32::MAX,
        };
        store.by_cell.insert(CellKey::from(cell), wrapped);
        store.remove_id(wrapped).unwrap();
        let replacement = store.insert_for_test(resident_test_cell(4), 0);
        assert_eq!(replacement.generation, 1);
        assert!(store.get(wrapped).is_none());
    }

    #[test]
    fn randomized_resident_lifecycle_matches_reference_ownership() {
        let mut store = ResidentStore::new();
        let mut reference: HashMap<CellId, (ResidentId, u64, usize)> = HashMap::new();
        let mut stale = Vec::new();
        let mut state = 0x243f_6a88_85a3_08d3u64;

        for step in 0..50_000u64 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let cell = resident_test_cell((state as u32) % 512);
            match (state >> 32) % 3 {
                0 => {
                    let bytes = ((state >> 16) as usize & 0xffff) + 1;
                    let id = store.insert(
                        cell,
                        step,
                        (step as f32, step as f32 + 1.0),
                        (step % 17) as f32,
                        0.0,
                        None,
                        None,
                        bytes,
                    );
                    if let Some((old_id, _, _)) = reference.insert(cell, (id, step, bytes)) {
                        assert_eq!(id, old_id, "duplicate insert changed a live ID");
                    }
                }
                1 => {
                    let touched = store.touch(cell, step);
                    match reference.get_mut(&cell) {
                        Some((id, frame, _)) => {
                            *frame = step;
                            assert_eq!(touched, Some(*id));
                        }
                        None => assert_eq!(touched, None),
                    }
                }
                _ => {
                    let removed = store.remove(cell);
                    match reference.remove(&cell) {
                        Some((id, _, _)) => {
                            assert_eq!(removed.as_ref().map(|entry| entry.0), Some(id));
                            stale.push(id);
                        }
                        None => assert!(removed.is_none()),
                    }
                }
            }

            if step.is_multiple_of(257) {
                assert_eq!(store.len(), reference.len());
                assert_eq!(
                    store.gpu_bytes(),
                    reference
                        .values()
                        .map(|(_, _, bytes)| *bytes)
                        .sum::<usize>()
                );
                for (&cell, &(id, frame, bytes)) in &reference {
                    assert_eq!(store.id(cell), Some(id));
                    let record = store.get(id).unwrap();
                    assert_eq!(record.cell, cell);
                    assert_eq!(record.last_drawn_frame, frame);
                    assert_eq!(record.gpu_bytes, bytes);
                }
                for &id in stale.iter().rev().take(64) {
                    assert!(store.get(id).is_none());
                }
                store.debug_assert_consistent();
            }
        }
    }

    fn descendants_at(root: CellId, level: u8) -> Vec<CellId> {
        let mut cells = vec![root];
        while cells[0].level < level {
            cells = cells
                .into_iter()
                .flat_map(CellId::children)
                .collect::<Vec<_>>();
        }
        cells
    }

    fn assert_reference_resident_coverage(store: &ResidentStore) {
        let mut counts: HashMap<CellKey, usize> = HashMap::new();
        let mut complete: HashSet<_> = store.by_cell.keys().copied().collect();
        for resident in store.cells() {
            let mut path = resident;
            loop {
                *counts.entry(CellKey::from(path)).or_default() += 1;
                if path.level <= MIN_RENDER_LEVEL {
                    break;
                }
                path = path.parent().unwrap();
            }
        }
        for level in (MIN_RENDER_LEVEL..MAX_STREAM_LEVEL).rev() {
            for &key in counts
                .keys()
                .filter(|key| CellId::from(**key).level == level)
            {
                let cell = CellId::from(key);
                if cell
                    .children()
                    .into_iter()
                    .all(|child| complete.contains(&CellKey::from(child)))
                {
                    complete.insert(key);
                }
            }
        }
        assert_eq!(counts.len(), store.coverage.len());
        for (key, members) in counts {
            let node = &store.coverage[&key];
            assert_eq!(node.members, members, "count at {:?}", CellId::from(key));
            assert_eq!(
                node.complete,
                complete.contains(&key),
                "coverage at {:?}",
                CellId::from(key)
            );
        }
    }

    #[test]
    fn incremental_resident_coverage_matches_full_scan_through_mixed_level_churn() {
        let roots = [
            CellId {
                face: Face(0),
                level: MIN_RENDER_LEVEL,
                i: 2,
                j: 3,
            },
            CellId {
                face: Face(2),
                level: 14,
                i: 3812,
                j: 9752,
            },
            CellId {
                face: Face(5),
                level: 16,
                i: 59012,
                j: 63421,
            },
        ];
        let mut candidates = Vec::new();
        for root in roots {
            for level in root.level..=root.level + 4 {
                candidates.extend(descendants_at(root, level));
            }
        }
        let mut store = ResidentStore::new();
        let mut arena = DrawableCutArena::new();
        let mut draw = Vec::new();
        let mut missing = Vec::new();
        let mut state = 0xa409_3822_299f_31d0u64;
        let mut wanted = Vec::new();
        let mut stale = Vec::new();
        for iteration in 0..2_000u64 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let cell = candidates[state as usize % candidates.len()];
            match (state >> 32) % 5 {
                0 | 1 => {
                    store.insert_for_test(cell, iteration);
                }
                2 => {
                    if let Some((id, _)) = store.remove(cell) {
                        stale.push(id);
                    }
                }
                3 if cell.level < MAX_STREAM_LEVEL => {
                    for child in cell.children() {
                        store.insert_for_test(child, iteration);
                    }
                }
                _ => {
                    store.touch(cell, iteration);
                }
            }
            if iteration.is_multiple_of(37) || wanted.is_empty() {
                wanted.clear();
                for root in roots {
                    wanted.extend(randomized_unbalanced_cut(root, &mut state, 24));
                }
                let shift = state as usize % wanted.len();
                wanted.rotate_left(shift);
            }
            if iteration.is_multiple_of(101) {
                if let Some(id) = stale.last() {
                    assert!(store.remove_id(*id).is_none());
                }
                arena.invalidate();
            }
            if iteration.is_multiple_of(257) && iteration != 0 {
                store.clear();
            }
            if iteration.is_multiple_of(4) {
                assert_reference_resident_coverage(&store);
                let expected = store.reference_drawable_cut(&wanted);
                store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing);
                assert_eq!(draw, expected.0, "draw order at iteration {iteration}");
                assert_eq!(
                    missing, expected.1,
                    "missing order at iteration {iteration}"
                );
                draw.reverse();
                missing.push((roots[0], -1.0));
                store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing);
                assert_eq!((draw.clone(), missing.clone()), expected);
            }
        }
        store.clear();
        assert!(store.coverage.is_empty());
        wanted.clear();
        store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing);
        assert!(draw.is_empty() && missing.is_empty());
    }

    #[test]
    fn incremental_coverage_counts_cells_independently_of_vertical_or_empty_meshes() {
        use crate::prepared_render::test_surface;
        let cell = CellId {
            face: Face(3),
            level: 17,
            i: 69211,
            j: 43118,
        };
        let mut surface = test_surface(cell);
        let batch = surface.batches[0].clone();
        surface.batches = [-2, 0, 3]
            .into_iter()
            .map(|k| {
                let mut batch = batch.clone();
                batch.addr.k = k;
                batch
            })
            .collect();
        let packed = pack_surface_cell(surface).unwrap();
        assert_eq!(packed.chunks.len(), 3);
        let mut store = ResidentStore::new();
        let id = store.insert(
            cell,
            0,
            packed.height_range(),
            packed.terrain_step_m(),
            packed.terrain_relief_m(),
            None,
            None,
            packed.gpu_bytes(),
        );
        for _ in &packed.chunks {
            assert_eq!(
                store.insert(
                    cell,
                    1,
                    (-100.0, 200.0),
                    1.0,
                    0.0,
                    None,
                    None,
                    packed.gpu_bytes()
                ),
                id
            );
        }
        assert_eq!(store.coverage[&CellKey::from(cell)].members, 1);
        let empty = cell.parent().unwrap();
        let mut surface = test_surface(empty);
        surface.batches.clear();
        surface.night_lights.clear();
        let packed = pack_surface_cell(surface).unwrap();
        assert!(packed.chunks.is_empty());
        let empty_id = store.insert(
            empty,
            2,
            packed.height_range(),
            0.0,
            0.0,
            None,
            None,
            packed.gpu_bytes(),
        );
        assert!(store.resident_subtree_ready(empty));
        assert_eq!(store.coverage[&CellKey::from(empty)].members, 2);
        assert!(store.finish_upload(empty_id, None, None, Vec::new(), Vec::new(), 0));
        assert_reference_resident_coverage(&store);
        store.remove_id(empty_id).unwrap();
        assert!(
            !store.resident_subtree_ready(empty),
            "one fine quadrant cannot cover its parent"
        );
        assert_eq!(store.coverage[&CellKey::from(empty)].members, 1);
        store.remove_id(id).unwrap();
        assert!(store.coverage.is_empty());
        assert!(store.remove_id(id).is_none());
    }

    #[test]
    fn drawable_topology_survives_arrivals_evictions_and_store_replacement() {
        let root = CellId {
            face: Face(2),
            level: 12,
            i: 1212,
            j: 1901,
        };
        let mut wanted: Vec<_> = descendants_at(root, 15)
            .into_iter()
            .enumerate()
            .map(|(index, cell)| WantedCell {
                cell,
                screen_px: 100.0 - index as f64 * 0.01,
                lane: ViewLane::Visible,
            })
            .collect();
        let mut store = ResidentStore::new();
        let mut arena = DrawableCutArena::new();
        let mut draw = Vec::new();
        let mut missing = Vec::new();
        for index in 0..wanted.len() {
            store.insert_for_test(wanted[index].cell, 0);
            if index % 2 == 0 {
                store.remove(wanted[index / 2].cell);
            }
            if index == wanted.len() / 2 {
                store = ResidentStore::new();
            }
            store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing);
            assert_eq!(
                (draw.clone(), missing.clone()),
                store.reference_drawable_cut(&wanted)
            );
            assert_eq!(
                arena.topology_rebuilds, 1,
                "membership rebuilt wanted topology"
            );
        }
        wanted.reverse();
        store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing);
        assert_eq!(arena.topology_rebuilds, 2);
        assert_eq!((draw, missing), store.reference_drawable_cut(&wanted));
    }

    #[test]
    fn reused_drawable_cut_arena_matches_fresh_state_through_randomized_churn() {
        let root = CellId {
            face: Face(2),
            level: MIN_RENDER_LEVEL,
            i: 3,
            j: 5,
        };
        let leaves = descendants_at(root, MIN_RENDER_LEVEL + 4);
        let mut wanted: Vec<_> = leaves
            .iter()
            .copied()
            .enumerate()
            .map(|(index, cell)| WantedCell {
                cell,
                screen_px: 10_000.0 - index as f64,
                lane: ViewLane::Visible,
            })
            .collect();
        let candidates = descendants_at(root, MIN_RENDER_LEVEL + 5);
        let mut store = ResidentStore::new();
        store.insert_for_test(root, 0);
        let mut arena = DrawableCutArena::new();
        let mut draw = Vec::new();
        let mut missing = Vec::new();
        let mut state = 0x1319_8a2e_0370_7344u64;

        for iteration in 0..2_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let cell = candidates[(state as usize) % candidates.len()];
            if state & 1 == 0 {
                store.insert_for_test(cell, iteration);
            } else {
                store.remove(cell);
            }
            if iteration % 7 == 0 {
                for (index, wanted) in wanted.iter_mut().enumerate() {
                    wanted.screen_px = 2.0 + (iteration as usize + index) as f64 * 0.001;
                    wanted.lane = if (index + iteration as usize) % 3 == 0 {
                        ViewLane::Guard
                    } else {
                        ViewLane::Visible
                    };
                }
            }
            if iteration % 13 == 0 {
                wanted.rotate_left(1);
            }
            if iteration % 97 == 0 {
                arena.invalidate();
            }

            let expected = store.drawable_cut(&wanted);
            store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing);
            assert_eq!(draw, expected.0);
            assert_eq!(missing, expected.1);

            let rebuilds = arena.rebuilds;
            // The app reweights and extends missing with ladder/prefetch work;
            // its output buffers must never serve as the cached source.
            draw.reverse();
            missing.fill((root, -1.0));
            missing.push((root, 999.0));
            store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing);
            assert_eq!(draw, expected.0);
            assert_eq!(missing, expected.1);
            assert_eq!(arena.rebuilds, rebuilds);
        }

        let capacities = arena.capacities();
        for _ in 0..100 {
            store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing);
        }
        assert_eq!(
            arena.capacities(),
            capacities,
            "steady-state cut rebuild grew scratch storage"
        );
    }

    #[test]
    fn drawable_cut_cache_refreshes_priorities_without_rebuilding_coverage() {
        let root = CellId {
            face: Face(2),
            level: MIN_RENDER_LEVEL,
            i: 3,
            j: 5,
        };
        let mut wanted: Vec<_> = root
            .children()
            .into_iter()
            .map(|cell| WantedCell {
                cell,
                screen_px: 3.0,
                lane: ViewLane::Visible,
            })
            .collect();
        let mut store = ResidentStore::new();
        store.insert_for_test(root, 0);
        store.insert_for_test(wanted[0].cell, 0);
        let mut arena = DrawableCutArena::new();
        let mut draw = Vec::new();
        let mut missing = Vec::new();
        assert!(store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing));
        let rebuilds = arena.rebuilds;
        for frame in 1..20 {
            for (index, wanted) in wanted.iter_mut().enumerate() {
                wanted.screen_px = frame as f64 + index as f64 * 0.25;
                wanted.lane = [
                    ViewLane::Visible,
                    ViewLane::Guard,
                    ViewLane::Background,
                    ViewLane::Legacy,
                ][(index + frame) % 4];
            }
            store.touch(root, frame as u64);
            store.insert(
                root,
                frame as u64,
                (-2.0, frame as f32),
                1.0,
                0.0,
                None,
                None,
                frame,
            );
            let expected = store.drawable_cut(&wanted);
            assert!(!store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing));
            assert_eq!((draw.clone(), missing.clone()), expected);
            assert_eq!(
                arena.rebuilds, rebuilds,
                "priority/metadata-only change rebuilt coverage"
            );
        }
    }

    #[test]
    fn drawable_cut_cache_tracks_store_replacement_clear_topology_and_invalidation() {
        let roots = [
            CellId {
                face: Face(1),
                level: MIN_RENDER_LEVEL,
                i: 3,
                j: 5,
            },
            CellId {
                face: Face(2),
                level: MIN_RENDER_LEVEL,
                i: 3,
                j: 5,
            },
        ];
        let mut wanted: Vec<_> = roots
            .into_iter()
            .map(|cell| WantedCell {
                cell,
                screen_px: 3.0,
                lane: ViewLane::Visible,
            })
            .collect();
        let mut a = ResidentStore::new();
        let mut b = ResidentStore::new();
        a.insert_for_test(roots[0], 0);
        b.insert_for_test(roots[1], 0);
        assert_eq!(a.membership_revision, b.membership_revision);
        let mut arena = DrawableCutArena::new();
        let mut draw = Vec::new();
        let mut missing = Vec::new();
        let check = |store: &ResidentStore,
                     wanted: &[WantedCell],
                     arena: &mut DrawableCutArena,
                     draw: &mut Vec<CellId>,
                     missing: &mut Vec<(CellId, f64)>| {
            let expected = store.drawable_cut(wanted);
            assert!(store.drawable_cut_into(wanted, arena, draw, missing));
            assert_eq!((draw.clone(), missing.clone()), expected);
        };
        check(&a, &wanted, &mut arena, &mut draw, &mut missing);
        check(&b, &wanted, &mut arena, &mut draw, &mut missing);
        assert_eq!(arena.rebuilds, 2, "different stores shared a cached cut");
        b.insert_for_test(roots[0], 0);
        check(&b, &wanted, &mut arena, &mut draw, &mut missing);
        assert_eq!(draw, roots);
        wanted.reverse();
        check(&b, &wanted, &mut arena, &mut draw, &mut missing);
        assert_eq!(
            draw,
            [roots[1], roots[0]],
            "root order must match the wanted order"
        );
        wanted.pop();
        wanted.extend(roots[0].children().into_iter().map(|cell| WantedCell {
            cell,
            screen_px: 3.0,
            lane: ViewLane::Visible,
        }));
        check(&b, &wanted, &mut arena, &mut draw, &mut missing);
        b.clear();
        check(&b, &wanted, &mut arena, &mut draw, &mut missing);
        assert!(draw.is_empty());
        let rebuilds = arena.rebuilds;
        arena.invalidate();
        check(&b, &wanted, &mut arena, &mut draw, &mut missing);
        assert_eq!(arena.rebuilds, rebuilds + 1);
        wanted.clear();
        check(&b, &wanted, &mut arena, &mut draw, &mut missing);
        assert!(draw.is_empty() && missing.is_empty());
    }

    #[test]
    #[ignore = "timed loading-coverage benchmark; run without concurrent game workloads"]
    fn benchmark_incremental_drawable_cut_loading() {
        use std::{hint::black_box, time::Instant};
        let root = CellId {
            face: Face(1),
            level: 13,
            i: 2800,
            j: 3500,
        };
        let leaves = descendants_at(root, 19);
        let wanted: Vec<_> = leaves
            .iter()
            .copied()
            .map(|cell| WantedCell {
                cell,
                screen_px: 4.0,
                lane: ViewLane::Visible,
            })
            .collect();
        let mut store = ResidentStore::new();
        store.insert_for_test(root, 0);
        for (index, &cell) in leaves.iter().enumerate() {
            if index % 3 != 0 {
                store.insert_for_test(cell, 0);
            }
        }
        for cell in descendants_at(root, 17) {
            store.insert_for_test(cell, 0);
        }
        let mut arena = DrawableCutArena::new();
        let mut reference = ReferenceDrawableCutArena::default();
        let mut draw = Vec::new();
        let mut missing = Vec::new();
        let mut reference_draw = Vec::new();
        let mut reference_missing = Vec::new();
        let mut old = Vec::new();
        let mut new = Vec::new();
        for (iteration, &cell) in leaves.iter().step_by(3).take(240).enumerate() {
            store.insert_for_test(cell, iteration as u64);
            let started = Instant::now();
            store.reference_resolve_drawable_cut_into(
                black_box(&wanted),
                &mut reference,
                &mut reference_draw,
                &mut reference_missing,
            );
            old.push(started.elapsed().as_secs_f64() * 1e6);
            let started = Instant::now();
            store.drawable_cut_into(black_box(&wanted), &mut arena, &mut draw, &mut missing);
            new.push(started.elapsed().as_secs_f64() * 1e6);
            assert_eq!(draw, reference_draw);
            assert_eq!(missing, reference_missing);
            store.remove(cell);
        }
        old.sort_by(f64::total_cmp);
        new.sort_by(f64::total_cmp);
        eprintln!(
            "loading cut old p50 {:.1} us p95 {:.1} us; incremental p50 {:.1} us p95 {:.1} us",
            old[old.len() / 2],
            old[old.len() * 95 / 100],
            new[new.len() / 2],
            new[new.len() * 95 / 100]
        );
    }

    #[test]
    #[ignore = "release-mode performance benchmark"]
    fn benchmark_fresh_vs_reused_drawable_cut_arena() {
        use std::hint::black_box;
        use vearth_platform::time::Instant;

        let root = CellId {
            face: Face(1),
            level: MIN_RENDER_LEVEL,
            i: 4,
            j: 6,
        };
        let leaves = descendants_at(root, MIN_RENDER_LEVEL + 6);
        let wanted: Vec<_> = leaves
            .iter()
            .copied()
            .enumerate()
            .map(|(index, cell)| WantedCell {
                cell,
                screen_px: 20_000.0 - index as f64,
                lane: ViewLane::Visible,
            })
            .collect();
        let mut store = ResidentStore::new();
        store.insert_for_test(root, 0);
        for (index, &cell) in leaves.iter().enumerate() {
            if index % 3 != 0 {
                store.insert_for_test(cell, 0);
            }
        }

        let samples = 30;
        let iterations = 8;
        let mut fresh_us = Vec::with_capacity(samples);
        let mut reused_us = Vec::with_capacity(samples);
        let mut arena = DrawableCutArena::new();
        let mut draw = Vec::new();
        let mut missing = Vec::new();
        store.drawable_cut_into(&wanted, &mut arena, &mut draw, &mut missing);
        for _ in 0..samples {
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(store.drawable_cut(black_box(&wanted)));
            }
            fresh_us.push(started.elapsed().as_secs_f64() * 1e6 / iterations as f64);

            let started = Instant::now();
            for _ in 0..iterations {
                store.drawable_cut_into(black_box(&wanted), &mut arena, &mut draw, &mut missing);
                black_box((&draw, &missing));
            }
            reused_us.push(started.elapsed().as_secs_f64() * 1e6 / iterations as f64);
        }
        fresh_us.sort_by(f64::total_cmp);
        reused_us.sort_by(f64::total_cmp);
        let percentile =
            |values: &[f64], percent: usize| values[(values.len() - 1) * percent / 100];
        eprintln!(
            "drawable cut fresh p50 {:.1} us p95 {:.1} us; reused p50 {:.1} us p95 {:.1} us",
            percentile(&fresh_us, 50),
            percentile(&fresh_us, 95),
            percentile(&reused_us, 50),
            percentile(&reused_us, 95),
        );
    }

    fn randomized_unbalanced_cut(root: CellId, state: &mut u64, splits: usize) -> Vec<WantedCell> {
        let mut out = vec![WantedCell {
            cell: root,
            screen_px: 1.0,
            lane: ViewLane::Visible,
        }];
        for _ in 0..splits {
            *state ^= *state << 13;
            *state ^= *state >> 7;
            *state ^= *state << 17;
            let index = (*state as usize) % out.len();
            if out[index].cell.level >= MAX_STREAM_LEVEL.min(root.level + 7) {
                continue;
            }
            let parent = out.swap_remove(index).cell;
            out.extend(parent.children().map(|cell| WantedCell {
                cell,
                screen_px: f64::from(cell.level),
                lane: ViewLane::Visible,
            }));
        }
        out
    }

    fn sorted_wanted_signature(wanted: &[WantedCell]) -> Vec<(CellId, u64, ViewLane)> {
        let mut signature: Vec<_> = wanted
            .iter()
            .map(|wanted| (wanted.cell, wanted.screen_px.to_bits(), wanted.lane))
            .collect();
        signature.sort_unstable();
        signature
    }

    #[test]
    fn incremental_lod_leaf_index_matches_rebuild_reference() {
        let root = CellId {
            face: Face(0),
            level: MIN_RENDER_LEVEL,
            i: 4,
            j: 4,
        };
        let direction = root.center_dir();
        let cam_ecef = ellipsoid::surface_point_along(direction) + direction * 2_000_000.0;
        let measure = |cell: CellId| {
            (
                f64::from(cell.level) + f64::from(cell.i & 7) * 0.01,
                ViewLane::Visible,
                None,
            )
        };
        let mut state = 0xa409_3822_299f_31d0u64;

        for _ in 0..1_000 {
            let initial = randomized_unbalanced_cut(root, &mut state, 300);
            let extra_budget = (state as usize) % 97;
            let max_wanted = initial.len() + extra_budget;
            let mut expected = initial.clone();
            let mut actual = initial;
            let mut expected_split = HashSet::new();
            let mut actual_split = HashSet::new();
            balance_cut_reference(
                cam_ecef,
                &measure,
                &mut expected,
                &mut expected_split,
                max_wanted,
            );
            balance_cut(
                cam_ecef,
                &measure,
                &mut actual,
                &mut actual_split,
                max_wanted,
            );
            assert_eq!(
                sorted_wanted_signature(&actual),
                sorted_wanted_signature(&expected)
            );
            assert_eq!(actual_split, expected_split);
        }
    }

    #[test]
    #[ignore = "release-mode performance benchmark"]
    fn benchmark_rebuild_vs_incremental_lod_leaf_index() {
        use std::hint::black_box;
        use vearth_platform::time::Instant;

        let root = CellId {
            face: Face(0),
            level: MIN_RENDER_LEVEL,
            i: 4,
            j: 4,
        };
        let direction = root.center_dir();
        let cam_ecef = ellipsoid::surface_point_along(direction) + direction * 1_000_000.0;
        let measure = |cell: CellId| (f64::from(cell.level), ViewLane::Visible, None);
        let mut state = 0x082e_fa98_ec4e_6c89u64;
        let initial = randomized_unbalanced_cut(root, &mut state, 1_100);
        let max_wanted = (initial.len() + 256).min(MAX_WANTED);
        let samples = 40;
        let mut rebuild_us = Vec::with_capacity(samples);
        let mut incremental_us = Vec::with_capacity(samples);

        for _ in 0..samples {
            let mut wanted = initial.clone();
            let mut split = HashSet::new();
            let started = Instant::now();
            balance_cut_reference(
                cam_ecef,
                &measure,
                black_box(&mut wanted),
                &mut split,
                max_wanted,
            );
            rebuild_us.push(started.elapsed().as_secs_f64() * 1e6);

            let mut wanted = initial.clone();
            let mut split = HashSet::new();
            let started = Instant::now();
            balance_cut(
                cam_ecef,
                &measure,
                black_box(&mut wanted),
                &mut split,
                max_wanted,
            );
            incremental_us.push(started.elapsed().as_secs_f64() * 1e6);
        }
        rebuild_us.sort_by(f64::total_cmp);
        incremental_us.sort_by(f64::total_cmp);
        let percentile =
            |values: &[f64], percent: usize| values[(values.len() - 1) * percent / 100];
        eprintln!(
            "LOD balance rebuild p50 {:.1} us p95 {:.1} us; incremental p50 {:.1} us p95 {:.1} us",
            percentile(&rebuild_us, 50),
            percentile(&rebuild_us, 95),
            percentile(&incremental_us, 50),
            percentile(&incremental_us, 95),
        );
    }

    fn level_at(wanted: &[WantedCell], lat: f64, lon: f64) -> Option<u8> {
        let dir = ellipsoid::geodetic_to_ecef(lat, lon, 0.0).normalize();
        wanted
            .iter()
            .find(|wanted| CellId::from_dir(dir, wanted.cell.level) == wanted.cell)
            .map(|wanted| wanted.cell.level)
    }

    fn assert_same_face_cut_is_balanced(wanted: &[WantedCell]) {
        let leaves: HashSet<_> = wanted.iter().map(|wanted| wanted.cell).collect();
        for fine in leaves.iter().copied() {
            let n = 1i64 << fine.level;
            for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let i = fine.i as i64 + di;
                let j = fine.j as i64 + dj;
                if i < 0 || j < 0 || i >= n || j >= n {
                    continue;
                }
                let mut probe = CellId {
                    face: fine.face,
                    level: fine.level,
                    i: i as u32,
                    j: j as u32,
                };
                loop {
                    if leaves.contains(&probe) {
                        assert!(
                            probe.level + 1 >= fine.level,
                            "unbalanced edge between {fine:?} and {probe:?}"
                        );
                        break;
                    }
                    let Some(parent) = probe.parent() else { break };
                    probe = parent;
                }
            }
        }
    }

    fn coalescing_signature(cut: &[WantedCell]) -> Vec<(CellId, u64, ViewLane)> {
        cut.iter()
            .map(|w| (w.cell, w.screen_px.to_bits(), w.lane))
            .collect()
    }

    fn coalescing_coverage(cut: &[WantedCell]) -> Vec<(u64, u64)> {
        let mut intervals: Vec<_> = cut
            .iter()
            .map(|w| {
                let range = CellKey::from(w.cell).range();
                (range.start(), range.end())
            })
            .collect();
        intervals.sort_unstable();
        let mut union: Vec<(u64, u64)> = Vec::new();
        for (start, end) in intervals {
            if let Some(last) = union.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
            } else {
                union.push((start, end));
            }
        }
        union
    }

    fn coalescing_splits(cut: &[WantedCell]) -> HashSet<CellId> {
        let mut split = HashSet::new();
        for wanted in cut {
            let mut cell = wanted.cell;
            while cell.level > MIN_RENDER_LEVEL {
                cell = cell.parent().unwrap();
                split.insert(cell);
            }
        }
        split
    }

    #[test]
    fn background_coalescing_preserves_quality_order_coverage_and_balance() {
        let root = CellId {
            face: Face(4),
            level: 3,
            i: 3,
            j: 3,
        };
        let cells = descendants_at(root, 7);
        let protected: HashSet<_> = [cells[3], cells[19], cells[70], cells[120]]
            .into_iter()
            .collect();
        let locked = cells[120];
        let mut cut: Vec<_> = cells
            .into_iter()
            .enumerate()
            .map(|(i, cell)| WantedCell {
                cell,
                screen_px: i as f64 + 0.25,
                lane: match i {
                    3 => ViewLane::Visible,
                    19 => ViewLane::Guard,
                    70 => ViewLane::Legacy,
                    _ => ViewLane::Background,
                },
            })
            .collect();
        // Interleaved source order must not be changed by sibling removal.
        cut.rotate_left(51);
        let before = cut.clone();
        let mut split = coalescing_splits(&cut);
        let measure = |cell: CellId| {
            (
                1.0,
                ViewLane::Background,
                (cell == locked).then_some(cell.level),
            )
        };
        let merged = coalesce_background_cut(&measure, &mut cut, &mut split);
        assert!(merged > 0);
        assert_eq!(cut.len() + 3 * merged, before.len());
        let frozen = |cut: &[WantedCell]| {
            coalescing_signature(
                &cut.iter()
                    .copied()
                    .filter(|w| protected.contains(&w.cell))
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(frozen(&cut), frozen(&before));
        assert_eq!(coalescing_coverage(&cut), coalescing_coverage(&before));
        assert_same_face_cut_is_balanced(&cut);
        assert_eq!(split, coalescing_splits(&cut));
        let stable = coalescing_signature(&cut);
        assert_eq!(coalesce_background_cut(&measure, &mut cut, &mut split), 0);
        assert_eq!(coalescing_signature(&cut), stable);
    }

    #[test]
    fn background_coalescing_refuses_horizon_gaps_parent_bounds_and_locks() {
        let parent = CellId {
            face: Face(0),
            level: 5,
            i: 10,
            j: 11,
        };
        let siblings: Vec<_> = parent
            .children()
            .into_iter()
            .map(|cell| WantedCell {
                cell,
                screen_px: 4.0,
                lane: ViewLane::Background,
            })
            .collect();
        for missing in 0..4 {
            let mut cut = siblings.clone();
            cut.remove(missing);
            let before = coalescing_signature(&cut);
            let mut split = coalescing_splits(&cut);
            assert_eq!(
                coalesce_background_cut(
                    &|_| (1.0, ViewLane::Background, None),
                    &mut cut,
                    &mut split
                ),
                0
            );
            assert_eq!(coalescing_signature(&cut), before);
        }
        for lane in [ViewLane::Visible, ViewLane::Guard, ViewLane::Legacy] {
            let mut cut = siblings.clone();
            let mut split = coalescing_splits(&cut);
            let measure = |cell| {
                (
                    1.0,
                    if cell == parent {
                        lane
                    } else {
                        ViewLane::Background
                    },
                    None,
                )
            };
            assert_eq!(coalesce_background_cut(&measure, &mut cut, &mut split), 0);
            assert_eq!(coalescing_signature(&cut), coalescing_signature(&siblings));
        }
        for locked in std::iter::once(parent).chain(parent.children()) {
            let mut cut = siblings.clone();
            let mut split = coalescing_splits(&cut);
            let measure = |cell: CellId| {
                (
                    1.0,
                    ViewLane::Background,
                    (cell == locked).then_some(cell.level),
                )
            };
            assert_eq!(coalesce_background_cut(&measure, &mut cut, &mut split), 0);
            assert_eq!(coalescing_signature(&cut), coalescing_signature(&siblings));
        }
    }

    #[test]
    fn background_coalescing_keeps_all_cube_face_edge_leaves() {
        for face in 0..6 {
            for level in [3, 7, 16] {
                let edge = 1 << level;
                for (i, j) in [
                    (0, edge / 2),
                    (edge - 1, edge / 2),
                    (edge / 2, 0),
                    (edge / 2, edge - 1),
                ] {
                    let parent = CellId {
                        face: Face(face),
                        level,
                        i,
                        j,
                    };
                    let mut cut: Vec<_> = parent
                        .children()
                        .into_iter()
                        .map(|cell| WantedCell {
                            cell,
                            screen_px: 5.0,
                            lane: ViewLane::Background,
                        })
                        .collect();
                    let before = coalescing_signature(&cut);
                    let mut split = coalescing_splits(&cut);
                    assert_eq!(
                        coalesce_background_cut(
                            &|_| (1.0, ViewLane::Background, None),
                            &mut cut,
                            &mut split
                        ),
                        0
                    );
                    assert_eq!(coalescing_signature(&cut), before);
                }
            }
        }
    }

    #[test]
    fn background_coalescing_keeps_two_to_one_at_every_neighbor_edge() {
        let parent = CellId {
            face: Face(2),
            level: 5,
            i: 10,
            j: 11,
        };
        for (di, dj) in [(-1_i64, 0_i64), (1, 0), (0, -1), (0, 1)] {
            let neighbor = CellId {
                i: (i64::from(parent.i) + di) as u32,
                j: (i64::from(parent.j) + dj) as u32,
                ..parent
            };
            let mut cut: Vec<_> = parent
                .children()
                .into_iter()
                .map(|cell| WantedCell {
                    cell,
                    screen_px: 4.0,
                    lane: ViewLane::Background,
                })
                .chain(
                    descendants_at(neighbor, parent.level + 2)
                        .into_iter()
                        .map(|cell| WantedCell {
                            cell,
                            screen_px: 2.0,
                            lane: ViewLane::Visible,
                        }),
                )
                .collect();
            assert_same_face_cut_is_balanced(&cut);
            let before = coalescing_signature(&cut);
            let mut split = coalescing_splits(&cut);
            assert_eq!(
                coalesce_background_cut(
                    &|_| (1.0, ViewLane::Background, None),
                    &mut cut,
                    &mut split
                ),
                0
            );
            assert_eq!(coalescing_signature(&cut), before);
            assert_same_face_cut_is_balanced(&cut);
        }
    }

    #[test]
    fn background_coalescing_reuses_resident_cover_through_merge_and_turn() {
        let parent = CellId {
            face: Face(4),
            level: 5,
            i: 10,
            j: 11,
        };
        let ancestor = parent.parent().unwrap();
        let children = parent.children();
        let mut cut: Vec<_> = children
            .into_iter()
            .map(|cell| WantedCell {
                cell,
                screen_px: 4.0,
                lane: ViewLane::Background,
            })
            .collect();
        let mut split = coalescing_splits(&cut);
        assert_eq!(
            coalesce_background_cut(&|_| (1.0, ViewLane::Background, None), &mut cut, &mut split),
            1
        );
        let mut store = ResidentStore::new();
        store.insert_for_test(ancestor, 0);
        for child in children {
            store.insert_for_test(child, 0);
        }
        let (draw, missing) = store.drawable_cut(&cut);
        assert_eq!(
            draw.into_iter().collect::<HashSet<_>>(),
            children.into_iter().collect()
        );
        assert_eq!(missing, [(parent, cut[0].stream_priority())]);
        assert_eq!(missing_coverage_root(parent, &store), None);
        store.remove(children[0]);
        let (draw, _) = store.drawable_cut(&cut);
        assert!(draw.contains(&ancestor));
        assert!(children[1..].iter().all(|cell| draw.contains(cell)));
        store.insert_for_test(parent, 1);
        assert_eq!(store.drawable_cut(&cut).0, [parent]);
        let turned: Vec<_> = children
            .into_iter()
            .map(|cell| WantedCell {
                cell,
                screen_px: 4.0,
                lane: ViewLane::Visible,
            })
            .collect();
        let (draw, _) = store.drawable_cut(&turned);
        assert!(draw.contains(&parent));
        assert!(children[1..].iter().all(|cell| draw.contains(cell)));
        store.insert_for_test(children[0], 2);
        let (draw, missing) = store.drawable_cut(&turned);
        assert_eq!(
            draw.into_iter().collect::<HashSet<_>>(),
            children.into_iter().collect()
        );
        assert!(missing.is_empty());
    }

    #[test]
    fn background_coalescing_has_bounded_candidate_work_and_minimum_level() {
        let root = CellId {
            face: Face(4),
            level: 3,
            i: 3,
            j: 3,
        };
        let mut cut: Vec<_> = descendants_at(root, 8)
            .into_iter()
            .map(|cell| WantedCell {
                cell,
                screen_px: 8.0,
                lane: ViewLane::Background,
            })
            .collect();
        let input_count = cut.len();
        let calls = std::cell::Cell::new(0usize);
        let measure = |_: CellId| {
            calls.set(calls.get() + 1);
            (1.0, ViewLane::Background, None)
        };
        let mut split = coalescing_splits(&cut);
        let merged = coalesce_background_cut(&measure, &mut cut, &mut split);
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].cell, root);
        assert!(split.is_empty());
        assert_eq!(merged, (input_count - 1) / 3);
        assert!(calls.get() <= 5 * (input_count + merged));
    }

    #[test]
    fn background_coalescing_production_selector_keeps_visible_and_guard_targets() {
        let mut camera = crate::camera::Camera::start();
        camera.lat = 51.9225_f64.to_radians();
        camera.lon = 4.47917_f64.to_radians();
        camera.alt_m = 32_000.0;
        camera.aspect = 1600.0 / 900.0;
        camera.heading = 0.0;
        camera.pitch = 0.0;
        let detail = spatial_summary(|_, _| 0.0, [u32::MAX; 32]);
        let metrics = |cell: CellId| {
            (cell.level == MIN_RENDER_LEVEL).then_some(TerrainMetricRef {
                height_range: (-80_000.0, 0.0),
                terrain_step_m: 0.0,
                terrain_relief_m: 0.0,
                terrain_detail: Some(&detail),
            })
        };
        let mut prior = HashSet::new();
        for heading in [0.0, std::f64::consts::FRAC_PI_2, 0.0] {
            camera.heading = heading;
            let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
            let run = |policy| {
                compute_wanted_internal_with_background_policy(
                    camera.eye_ecef(),
                    900.0,
                    camera.fov_y,
                    &prior,
                    Some(&view),
                    Some(&metrics),
                    MAX_VIEW_WANTED,
                    policy,
                )
            };
            let (reference, reference_split, _) = run(false);
            let (cut, split, stats) = run(true);
            let quality = |cut: &[WantedCell]| {
                coalescing_signature(
                    &cut.iter()
                        .copied()
                        .filter(|w| w.lane != ViewLane::Background)
                        .collect::<Vec<_>>(),
                )
            };
            assert_eq!(quality(&cut), quality(&reference));
            assert_eq!(coalescing_coverage(&cut), coalescing_coverage(&reference));
            assert!(
                cut.len() < reference.len(),
                "offscreen quality must not refill spare budget"
            );
            assert_same_face_cut_is_balanced(&cut);
            assert_eq!(stats.split_cells, split.len());
            assert_eq!(stats.retained_splits, split.intersection(&prior).count());
            assert!(split.is_subset(&reference_split));
            prior = split;
        }
    }

    #[test]
    fn background_coalescing_view_opt_out_preserves_the_original_selector() {
        let mut camera = crate::camera::Camera::start();
        camera.lat = 51.9225_f64.to_radians();
        camera.lon = 4.47917_f64.to_radians();
        camera.aspect = 1600.0 / 900.0;
        let detail = spatial_summary(|_, _| 0.0, [u32::MAX; 32]);
        let metrics = |cell: CellId| {
            (cell.level == MIN_RENDER_LEVEL).then_some(TerrainMetricRef {
                height_range: (-80_000.0, 0.0),
                terrain_step_m: 0.0,
                terrain_relief_m: 0.0,
                terrain_detail: Some(&detail),
            })
        };
        let mut changed_background = false;
        for (altitude, pitch) in [(29.476, 1.15), (304.476, 1.15), (32_000.0, 0.0)] {
            camera.alt_m = altitude;
            camera.pitch = pitch;
            let mut prior = HashSet::new();
            for heading in [0.0, std::f64::consts::PI, 0.0] {
                camera.heading = heading;
                let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
                let motion_view = view.with_background_coalescing(false);
                let original = compute_wanted_internal_with_background_policy(
                    camera.eye_ecef(),
                    900.0,
                    2.0 * view.tan_half_fov_y.atan(),
                    &prior,
                    Some(&view),
                    Some(&metrics),
                    MAX_VIEW_WANTED,
                    false,
                );
                let disabled = compute_wanted_for_view_with_stats_and_metric_lookup(
                    camera.eye_ecef(),
                    900.0,
                    &motion_view,
                    &prior,
                    &metrics,
                );
                assert_eq!(
                    coalescing_signature(&disabled.0),
                    coalescing_signature(&original.0)
                );
                assert_eq!(disabled.1, original.1);
                assert_eq!(disabled.2, original.2);

                let enabled = compute_wanted_for_view_with_stats_and_metric_lookup(
                    camera.eye_ecef(),
                    900.0,
                    &view,
                    &prior,
                    &metrics,
                );
                let reference_enabled = compute_wanted_internal_with_background_policy(
                    camera.eye_ecef(),
                    900.0,
                    2.0 * view.tan_half_fov_y.atan(),
                    &prior,
                    Some(&view),
                    Some(&metrics),
                    MAX_VIEW_WANTED,
                    true,
                );
                assert_eq!(
                    coalescing_signature(&enabled.0),
                    coalescing_signature(&reference_enabled.0)
                );
                assert_eq!(enabled.1, reference_enabled.1);
                assert_eq!(enabled.2, reference_enabled.2);
                changed_background |= enabled.0.len() < disabled.0.len();
                prior = disabled.1;
            }
        }
        assert!(
            changed_background,
            "the opt-out must exercise a nonempty postpass"
        );
    }

    #[test]
    fn selection_measurement_cache_preserves_cut_scores_and_metric_dependencies() {
        use std::cell::{Cell, RefCell};

        let detail = spatial_summary(
            |x, z| if x < 16 { 0.0 } else { (x + z) as f32 * 31.0 },
            [u32::MAX; 32],
        );
        let mut camera = crate::camera::Camera::start();
        camera.aspect = 1600.0 / 900.0;
        let mut prior = HashSet::new();
        let mut total_reference_lookups = 0;
        let mut total_cached_lookups = 0;
        for (lat, lon, altitude, pitch, legacy, coalesce) in [
            (51.9225_f64, 4.47917_f64, 32_000.0, 0.0, false, true),
            (51.9225, 4.47917, 304.476, 1.15, false, false),
            (40.015, -105.2705, 2_000.0, 1.0, false, true),
            (-33.45, -70.67, 6_000.0, 1.1, false, true),
            (0.0, 45.0, 1_000_000.0, 0.0, true, false),
        ] {
            camera.lat = lat.to_radians();
            camera.lon = lon.to_radians();
            camera.alt_m = altitude;
            camera.pitch = pitch;
            for heading in [0.0, std::f64::consts::PI] {
                camera.heading = heading;
                let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
                let dependencies = RefCell::new(BTreeSet::new());
                let lookups = Cell::new(0usize);
                let metrics = |cell: CellId| {
                    lookups.set(lookups.get() + 1);
                    let present = cell.level == MIN_RENDER_LEVEL
                        || ((5..=14).contains(&cell.level)
                            && (cell.i * 17 + cell.j * 31) % 11 == 0);
                    if !present {
                        return None;
                    }
                    dependencies.borrow_mut().insert(cell);
                    Some(TerrainMetricRef {
                        height_range: (-80_000.0, 2_000.0),
                        terrain_step_m: 31.0,
                        terrain_relief_m: 900.0,
                        terrain_detail: Some(&detail),
                    })
                };
                let run = |cached| {
                    let compute = if cached {
                        compute_wanted_with_measurement_cache::<true>
                    } else {
                        compute_wanted_with_measurement_cache::<false>
                    };
                    compute(
                        camera.eye_ecef(),
                        900.0,
                        camera.fov_y,
                        &prior,
                        (!legacy).then_some(&view),
                        Some(&metrics),
                        MAX_VIEW_WANTED,
                        coalesce,
                    )
                };
                let reference = run(false);
                let reference_dependencies = dependencies.take();
                let reference_lookups = lookups.replace(0);
                let cached = run(true);
                let signature = |cut: &[WantedCell]| {
                    cut.iter()
                        .map(|w| (w.cell, w.screen_px.to_bits(), w.lane))
                        .collect::<Vec<_>>()
                };
                assert_eq!(signature(&cached.0), signature(&reference.0));
                assert_eq!(cached.1, reference.1);
                assert_eq!(cached.2, reference.2);
                assert_eq!(*dependencies.borrow(), reference_dependencies);
                assert!(lookups.get() <= reference_lookups);
                total_reference_lookups += reference_lookups;
                total_cached_lookups += lookups.get();
                prior = reference.1;
            }
        }
        assert!(total_cached_lookups * 2 < total_reference_lookups);
    }

    #[test]
    #[ignore = "manual release-mode selector benchmark"]
    fn benchmark_selection_measurement_cache() {
        use vearth_platform::time::Instant;

        let detail = Arc::new(spatial_summary(
            |x, z| (x + z) as f32 * 31.0,
            [u32::MAX; 32],
        ));
        let mut store = crate::lod_selection::TerrainMetricStore::default();
        for face in 0..6 {
            for cell in descendants_at(CellId::root(Face(face)), MIN_RENDER_LEVEL) {
                store.upsert(
                    cell,
                    crate::lod_selection::TerrainMetric {
                        height_range: (-80_000.0, 2_000.0),
                        terrain_step_m: 31.0,
                        terrain_relief_m: 900.0,
                        terrain_detail: Some(detail.clone()),
                    },
                    true,
                );
            }
        }
        let mut camera = crate::camera::Camera::start();
        camera.lat = 51.9225_f64.to_radians();
        camera.lon = 4.47917_f64.to_radians();
        camera.aspect = 1600.0 / 900.0;
        for (altitude, pitch, known) in [
            (32_000.0, 0.0, true),
            (304.476, 1.15, true),
            (304.476, 1.15, false),
        ] {
            camera.alt_m = altitude;
            camera.pitch = pitch;
            let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
            let prior = HashSet::new();
            let mut samples = [Vec::new(), Vec::new()];
            for iteration in 0..42 {
                for cached in [iteration % 2 == 0, iteration % 2 != 0] {
                    let compute = if cached {
                        compute_wanted_with_measurement_cache::<true>
                    } else {
                        compute_wanted_with_measurement_cache::<false>
                    };
                    store.begin_query();
                    let metrics = |cell| store.get(cell);
                    let start = Instant::now();
                    std::hint::black_box(compute(
                        camera.eye_ecef(),
                        900.0,
                        camera.fov_y,
                        &prior,
                        Some(&view),
                        known.then_some(&metrics as &TerrainMetricLookup<'_>),
                        MAX_VIEW_WANTED,
                        true,
                    ));
                    if iteration >= 2 {
                        samples[usize::from(cached)].push(start.elapsed().as_secs_f64() * 1_000.0);
                    }
                    store.finish_query();
                }
            }
            for values in &mut samples {
                values.sort_by(f64::total_cmp);
            }
            eprintln!(
                "selection altitude={altitude} known={known}: uncached={:.3}ms cached={:.3}ms ({:.2}x)",
                samples[0][samples[0].len() / 2],
                samples[1][samples[1].len() / 2],
                samples[0][samples[0].len() / 2] / samples[1][samples[1].len() / 2],
            );
        }
    }

    #[test]
    fn wanted_set_from_orbit_is_coarse_and_bounded() {
        let cam = DVec3::new(ellipsoid::MEAN_RADIUS + 16_000_000.0, 0.0, 0.0);
        let (wanted, _) = compute_wanted(cam, 1080.0, 1.0, &HashSet::new());
        assert!(!wanted.is_empty());
        assert!(wanted.len() < 1500, "orbit wanted set: {}", wanted.len());
        // From 16,000 km everything visible should be L3–L6.
        for w in &wanted {
            assert!(
                w.cell.level <= 6,
                "unexpectedly fine cell from orbit: {:?}",
                w.cell
            );
        }
    }

    #[test]
    fn wanted_set_refines_when_low() {
        let cam = DVec3::new(ellipsoid::MEAN_RADIUS + 5_000.0, 0.0, 0.0);
        let (wanted, _) = compute_wanted(cam, 1080.0, 1.0, &HashSet::new());
        let max_level = wanted.iter().map(|w| w.cell.level).max().unwrap();
        assert!(
            max_level >= 12,
            "at 5 km altitude expected fine cells, max level {max_level}"
        );
        assert!(wanted.len() <= MAX_WANTED + 8);
    }

    #[test]
    fn sse_height_envelope_caps_only_coarse_uncertainty() {
        for level in [3, 4] {
            let source = CellId::from_dir(DVec3::X, level);
            assert!(source.sample_pitch_m() * 2.0 > 32_000.0);
            assert_eq!(
                padded_height_range(Some(((0.0, 0.0), 0.0, 0.0, source))),
                Some((-500.0, 9_250.0)),
            );
            // Real extrema outside the inferred envelope retain their local
            // margin; neither a high peak nor deep bathymetry is clipped.
            assert_eq!(
                padded_height_range(Some(((-12_000.0, 10_500.0), 0.0, 0.0, source))),
                Some((-12_100.0, 10_850.0)),
            );
        }
        let source = CellId::from_dir(DVec3::X, 3);
        for range in [
            (f32::NAN, 0.0),
            (0.0, f32::INFINITY),
            (f32::NEG_INFINITY, 0.0),
            (10.0, 0.0),
        ] {
            assert_eq!(padded_height_range(Some((range, 0.0, 0.0, source))), None);
        }
    }

    #[test]
    fn sse_height_envelope_keeps_fine_metric_arithmetic() {
        for level in 11..=MAX_STREAM_LEVEL {
            let source = CellId::from_dir(DVec3::X, level);
            let pad = (source.sample_pitch_m() * 2.0).max(100.0);
            for (lo, hi) in [(-5.0_f32, 50.0_f32), (1_500.0, 3_000.0), (8_000.0, 8_600.0)] {
                let expected = (f64::from(lo) - pad, f64::from(hi) + pad + 250.0);
                let actual = padded_height_range(Some(((lo, hi), 0.0, 0.0, source))).unwrap();
                assert_eq!(actual.0.to_bits(), expected.0.to_bits());
                assert_eq!(actual.1.to_bits(), expected.1.to_bits());
            }
        }
    }

    #[test]
    fn rotterdam_search_coarse_metrics_keep_aerial_screen_error() {
        let mut camera = crate::camera::Camera::start();
        camera.lat = 51.9225_f64.to_radians();
        camera.lon = 4.47917_f64.to_radians();
        camera.alt_m = 32_000.0;
        let eye = camera.eye_ecef();
        for source_level in [3, 4] {
            let source = CellId::from_dir(eye, source_level);
            let metric = |cell| (cell == source).then_some(((0.0, 0.0), 0.0, 0.0).into());
            for level in [12, 19] {
                let cell = CellId::from_dir(eye, level);
                let inherited = terrain_metrics_for(cell, Some(&metric));
                assert_eq!(inherited.unwrap().3, source);
                let (lo, hi) = padded_height_range(inherited).unwrap();
                let anchor = ellipsoid::surface_point_along(cell.center_dir());
                let up = anchor.normalize();
                let height = (eye - anchor).dot(up).clamp(lo, hi);
                let distance =
                    ((eye - (anchor + up * height)).length() - cell.size_m() * 0.75).max(1.0);
                assert!(distance > 20_000.0, "{source:?} -> {cell:?}: {distance}");
                if level == 19 {
                    let screen =
                        lod::sample_screen_px(cell.sample_pitch_m(), distance, 900.0, camera.fov_y);
                    assert!(screen < 0.03, "street-scale sample occupied {screen} px");
                }
            }
        }
    }

    #[test]
    fn rotterdam_search_from_loaded_globe_stays_aerial_after_leaf_completion() {
        let mut globe = crate::camera::Camera::start();
        globe.lat = 0.0;
        globe.lon = 0.0;
        globe.aspect = 1600.0 / 900.0;
        globe.alt_m = globe.alt_m.min(globe.max_zoom_out_alt());
        let globe_view = LodView::new(globe.view_rotation(), globe.fov_y, globe.aspect);
        let (globe_wanted, globe_split) =
            compute_wanted_for_view(globe.eye_ecef(), 900.0, &globe_view, &HashSet::new());
        assert!(globe_wanted.iter().all(|target| target.cell.level <= 4));
        let globe_metrics: HashMap<_, _> = globe_wanted
            .iter()
            .map(|target| (target.cell, ((0.0, 0.0), 0.0, 0.0)))
            .collect();

        let mut camera = globe;
        camera.lat = 51.9225_f64.to_radians();
        camera.lon = 4.47917_f64.to_radians();
        camera.alt_m = 32_000.0;
        camera.pitch = 0.0;
        camera.heading = 0.0;
        let eye = camera.eye_ecef();
        let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (cold, _) = compute_wanted_for_view(eye, 900.0, &view, &HashSet::new());
        let cold_max = cold.iter().map(|target| target.cell.level).max().unwrap();
        // Cube Houses use the same coherent L13 footprint as other landmarks.
        assert_eq!(cold_max, vearth_gen::landmarks::LANDMARK_MIN_LEVEL);
        let locks = landmark_locks(eye, 900.0, camera.fov_y);
        for target in cold.iter().filter(|target| target.cell.level > 12) {
            assert!(
                landmark_level(target.cell.parent().unwrap(), &locks).is_some(),
                "aerial refinement escaped its landmark footprint"
            );
        }

        // Completed leaves do not supply metrics to their missing parents.
        // Coarse uncertainty must therefore stay physically meaningful even
        // after all targets have arrived and the camera remains stationary.
        for source_level in [None, Some(3), Some(4)] {
            let mut metrics = globe_metrics.clone();
            let mut previous = globe_split.clone();
            for _ in 0..4 {
                let metric = |cell: CellId| {
                    metrics.get(&cell).copied().map(Into::into).or_else(|| {
                        (Some(cell.level) == source_level).then_some(((0.0, 0.0), 0.0, 0.0).into())
                    })
                };
                let (wanted, split, _) = compute_wanted_for_view_with_stats_and_metric_lookup(
                    eye, 900.0, &view, &previous, &metric,
                );
                let maximum = wanted.iter().map(|target| target.cell.level).max().unwrap();
                assert!(
                    maximum <= cold_max + 1,
                    "coarse source {source_level:?}: L{maximum}"
                );
                assert!(wanted.len() <= MAX_VIEW_WANTED);
                assert!(level_at(&wanted, camera.lat, camera.lon).unwrap() >= cold_max);
                assert_same_face_cut_is_balanced(&wanted);
                for target in wanted {
                    metrics.insert(target.cell, ((0.0, 0.0), 0.0, 0.0));
                }
                previous = split;
            }
        }
    }

    #[test]
    fn rotterdam_search_arrival_floor_uses_physical_coarse_height() {
        let place = vearth_data::geocode::Geocoder::new()
            .search("Rotterdam")
            .unwrap()
            .remove(0);
        for level in [3, 4] {
            let mut camera = crate::camera::Camera::start();
            camera.lat = 0.0;
            camera.lon = 0.0;
            let mut flight = crate::camera::FlyTo::new(
                &camera,
                place.lat_deg,
                place.lon_deg,
                place.view_altitude_m,
            );
            assert!(!flight.update(&mut camera, flight.duration_seconds()));
            assert_eq!(camera.alt_m, place.view_altitude_m);
            assert_eq!(camera.pitch, 0.0);

            let direction = camera.eye_ecef().normalize_or(DVec3::X);
            let cell = CellId::from_dir(direction, level);
            let surface = generate_surface_cell(
                cell,
                &Sources::terrain_only(&vearth_data::FnElevation(|_, _| 0.0)),
            );
            assert!(surface.complete);
            assert!(surface.terrain_collision.is_none());
            assert!(surface.height_range.1 < 0.25, "{:?}", surface.height_range);
            let mut index = ResidentStore::new();
            index.insert(
                cell,
                0,
                surface.height_range,
                surface.terrain_step_m,
                surface.terrain_relief_m,
                None,
                None,
                0,
            );
            assert!(index.terrain_surface_at(direction).is_none());

            // This is the Explore floor operation after the flight releases
            // the camera. Coarse cells use the metric fallback because no
            // fine collision heightfield has arrived yet.
            let floor = |camera: &mut crate::camera::Camera, index: &ResidentStore| {
                let direction = camera.eye_ecef().normalize_or(DVec3::X);
                let terrain = index
                    .terrain_surface_at(direction)
                    .or_else(|| index.terrain_top_under(direction).map(f64::from));
                if let Some(top) = terrain {
                    camera.alt_m = camera.alt_m.max(top.max(0.0) + 25.0);
                }
            };
            for _ in 0..3 {
                floor(&mut camera, &index);
                assert_eq!(camera.alt_m, place.view_altitude_m);
            }

            // A synthetic high from a globe mesh's bounding-box corner can
            // raise the same stationary camera without any input.
            index.set_height_for_test(cell, (0.0, 55_000.0));
            floor(&mut camera, &index);
            assert_eq!(camera.alt_m, 55_025.0);
        }
    }

    #[test]
    fn rotterdam_search_flat_sea_skirts_do_not_request_mountain_detail() {
        let mut camera = crate::camera::Camera::start();
        camera.lat = 0.0;
        camera.lon = 0.0;
        camera.aspect = 1600.0 / 900.0;
        camera.alt_m = camera.alt_m.min(camera.max_zoom_out_alt());
        let globe_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (globe, initial_split) =
            compute_wanted_for_view(camera.eye_ecef(), 900.0, &globe_view, &HashSet::new());
        let elevation = vearth_data::FnElevation(|_, _| 0.0);
        let sources = Sources::terrain_only(&elevation);
        let mut metrics = HashMap::new();
        let insert_actual_metric = |cell, metrics: &mut HashMap<_, _>| {
            metrics.entry(cell).or_insert_with(|| {
                let surface = generate_surface_cell(cell, &sources);
                assert!(surface.complete);
                assert_eq!(surface.terrain_relief_m, 0.0);
                assert_eq!(surface.terrain_step_m, 0.0);
                assert!(surface.height_range.0.is_finite());
                assert!(surface.height_range.1.is_finite());
                assert!(surface.height_range.0 <= surface.height_range.1);
                if cell.level < COLLISION_MIN_LEVEL {
                    assert!(
                        surface.height_range.1 < 0.25,
                        "{cell:?}: {:?}",
                        surface.height_range,
                    );
                }
                // Fine cells retain their conservative content AABB height
                // range; its curvature allowance is not physical relief.
                (
                    surface.height_range,
                    surface.terrain_step_m,
                    surface.terrain_relief_m,
                )
            });
        };
        for target in &globe {
            insert_actual_metric(target.cell, &mut metrics);
        }
        assert!(metrics.values().any(|(range, _, _)| range.0 < -700.0));

        camera.lat = 51.9225_f64.to_radians();
        camera.lon = 4.47917_f64.to_radians();
        camera.alt_m = 32_000.0;
        camera.pitch = 0.0;
        camera.heading = 0.0;
        let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let mut split = initial_split;
        let mut prior = None;
        let mut stable = false;
        for iteration in 0..4 {
            let metric = |cell| metrics.get(&cell).copied().map(Into::into);
            let (wanted, next_split, _) = compute_wanted_for_view_with_stats_and_metric_lookup(
                camera.eye_ecef(),
                900.0,
                &view,
                &split,
                &metric,
            );
            let ids: HashSet<_> = wanted.iter().map(|target| target.cell).collect();
            assert!(wanted.iter().all(|target| target.cell.level <= 13));
            assert_same_face_cut_is_balanced(&wanted);
            if iteration == 0 {
                // Derive the control's relief from the content range while
                // keeping camera, recorded heights, steps and split history
                // identical to isolate the skirt-driven mountain boost.
                let old_metric = |cell| {
                    metrics
                        .get(&cell)
                        .map(|&(range, step, _)| (range, step, (range.1 - range.0).max(0.0)).into())
                };
                let (old, _, _) = compute_wanted_for_view_with_stats_and_metric_lookup(
                    camera.eye_ecef(),
                    900.0,
                    &view,
                    &split,
                    &old_metric,
                );
                let fine_count = |cut: &[WantedCell]| {
                    cut.iter().filter(|target| target.cell.level >= 13).count()
                };
                assert!(fine_count(&wanted) < fine_count(&old));
            }
            if prior.as_ref() == Some(&ids) {
                stable = true;
                break;
            }
            for target in &wanted {
                insert_actual_metric(target.cell, &mut metrics);
            }
            prior = Some(ids);
            split = next_split;
        }
        assert!(
            stable,
            "stationary sea cut did not converge after leaf completion"
        );
    }

    #[test]
    fn known_flat_envelope_does_not_prioritize_cells_behind_amsterdam_camera() {
        let camera = crate::camera::Camera {
            lat: 52.369_f64.to_radians(),
            lon: 4.908_f64.to_radians(),
            alt_m: 250.0,
            heading: 90_f64.to_radians(),
            pitch: 55_f64.to_radians(),
            fov_y: 55_f64.to_radians(),
            aspect: 1600.0 / 909.0,
        };
        let cell = CellId {
            face: Face(4),
            level: 17,
            i: 120463,
            j: 71052,
        };
        let eye = camera.eye_ecef();
        let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let anchor = ellipsoid::surface_point_along(cell.center_dir());
        assert_eq!(view.lane(eye, cell), ViewLane::Visible);
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        for x in [0.0, 16.0, 32.0] {
            for z in [0.0, 16.0, 32.0] {
                for h in [0.0, 9_000.0] {
                    let p = frame.local_to_ecef(DVec3::new(x, h / frame.grid_scale.y, z));
                    assert!((p - eye).dot(camera.forward_ecef()) < -600.0);
                }
            }
        }
        let known = padded_height_range(Some(((-5.0, 50.0), 0.0, 0.0, cell)));
        assert_eq!(
            view.diagnostic_lane_at_anchor(eye, cell, anchor, known),
            ViewLane::Background,
        );
    }

    #[test]
    fn known_content_bounds_contain_generated_frame_and_curved_source_extents() {
        for face in 0..6 {
            for level in [3, 5, 9, 13, 17, 19, 20] {
                let n = 1_u32 << level;
                for i in [0, n / 2, n - 1] {
                    for j in [0, n / 2, n - 1] {
                        let cell = CellId {
                            face: Face(face),
                            level,
                            i,
                            j,
                        };
                        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                        for range in [(-500.0_f32, 50.0_f32), (8_000.0, 10_000.0)] {
                            let envelope = padded_height_range(Some((range, 0.0, 0.0, cell)));
                            let (center, radius) =
                                LodView::content_bound(cell, frame.anchor, envelope);
                            for x in [0.0, 0.5, 1.0] {
                                for z in [0.0, 0.5, 1.0] {
                                    let (source, offset) = frame.column_surface(x, z);
                                    for height in [range.0, range.1] {
                                        let flat = frame.local_to_ecef(DVec3::new(
                                            x * 32.0,
                                            f64::from(height) / frame.grid_scale.y,
                                            z * 32.0,
                                        ));
                                        let curved = flat + frame.y_axis * offset;
                                        let geodetic = source
                                            + ellipsoid::geodetic_normal(source)
                                                * f64::from(height);
                                        for p in [flat, curved, geodetic] {
                                            assert!(
                                                p.distance(center) <= radius + 1e-6,
                                                "{cell:?} {range:?} radius={radius} point={p:?}"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn unknown_or_invalid_content_bounds_keep_original_global_envelope() {
        for face in 0..6 {
            for level in [3, 9, 17, 20] {
                let n = 1_u32 << level;
                let cell = CellId {
                    face: Face(face),
                    level,
                    i: n - 1,
                    j: n / 2,
                };
                let anchor = ellipsoid::surface_point_along(cell.center_dir());
                let original = (
                    anchor + anchor.normalize() * 4_500.0,
                    (cell.size_m() * 1.1).hypot(4_500.0),
                );
                for range in [
                    None,
                    Some((f64::NAN, 10.0)),
                    Some((0.0, f64::INFINITY)),
                    Some((10.0, 0.0)),
                ] {
                    assert_eq!(LodView::content_bound(cell, anchor, range), original);
                }
            }
        }
        let child = CellId::from_dir(DVec3::X, 17);
        let ancestor = child.parent().unwrap().parent().unwrap();
        let metric =
            |cell| (cell == ancestor).then_some(((8_000.0, 10_000.0), 100.0, 2_000.0).into());
        let inherited = terrain_metrics_for(child, Some(&metric));
        assert_eq!(inherited.unwrap().3, ancestor);
        let range = padded_height_range(inherited).unwrap();
        assert!(range.0 < 8_000.0 && range.1 > 10_000.0);
        assert_eq!(
            padded_height_range(Some(((f32::NAN, 0.0), 0.0, 0.0, child))),
            None
        );
    }

    #[test]
    fn known_tall_content_remains_visible_above_global_ceiling() {
        let cell = CellId::from_dir(DVec3::X, 19);
        let anchor = ellipsoid::surface_point_along(cell.center_dir());
        let up = ellipsoid::geodetic_normal(anchor);
        let eye = anchor + up * 10_000.0 + DVec3::Y * 1_000.0;
        let target = anchor + up * 10_000.0;
        let rotation =
            glam::dcamera::rh::view::look_to_mat4(DVec3::ZERO, (target - eye).normalize(), up);
        let view = LodView::new(rotation, 20_f64.to_radians(), 1.0);
        let range = padded_height_range(Some(((9_900.0, 10_100.0), 0.0, 0.0, cell)));
        assert_eq!(
            view.diagnostic_lane_at_anchor(eye, cell, anchor, range),
            ViewLane::Visible
        );
    }

    #[test]
    fn terminal_lane_diagnostics_keep_requested_cut_and_metric_provenance_distinct() {
        let base = CellId::from_dir(DVec3::X, 17);
        let parent = base.parent().unwrap();
        let [a, b, ..] = parent.children();
        let c = CellId::from_dir(DVec3::Y, 12);
        let wanted = [
            WantedCell {
                cell: a,
                screen_px: 2.0,
                lane: ViewLane::Visible,
            },
            WantedCell {
                cell: b,
                screen_px: 2.0,
                lane: ViewLane::Guard,
            },
            WantedCell {
                cell: c,
                screen_px: 2.0,
                lane: ViewLane::Background,
            },
        ];
        let camera = crate::camera::Camera {
            lat: 0.0,
            lon: 0.0,
            alt_m: 250.0,
            heading: 0.0,
            pitch: 0.0,
            fov_y: 55_f64.to_radians(),
            aspect: 1.6,
        };
        let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let metric =
            |cell| (cell == a || cell == parent).then_some(((0.0, 20.0), 0.0, 20.0).into());
        let result = view_lane_diagnostics(&wanted, camera.eye_ecef(), &view, &metric);
        assert_eq!(result.metric_provenance, [1, 1, 1]);
        assert_eq!(result.wanted_by_lane_level[0][17], 1);
        assert_eq!(result.wanted_by_lane_level[1][17], 1);
        assert_eq!(result.wanted_by_lane_level[2][12], 1);
        assert_eq!(result.wanted_by_lane_level.iter().flatten().sum::<u64>(), 3);
        assert_eq!(result.global_to_known_lane.iter().flatten().sum::<u64>(), 3);
        assert_eq!(wanted[1].lane, ViewLane::Guard);
    }

    fn spatial_descendant(source: CellId, depth: u8, x: u32, z: u32) -> CellId {
        let side = 1u32 << depth;
        CellId {
            face: source.face,
            level: source.level + depth,
            i: (source.i << depth) + x,
            j: (source.j << depth) + side - 1 - z,
        }
    }

    fn spatial_summary(
        height: impl Fn(usize, usize) -> f32,
        owned: [u32; 32],
    ) -> vearth_gen::TerrainDetailSummary {
        vearth_gen::TerrainDetailSummary::from_grid(&vearth_gen::TerrainDetailGrid {
            heights_m: Box::new(std::array::from_fn(|i| height(i % 33, i / 33))),
            source_columns: owned,
        })
        .unwrap()
    }

    #[test]
    fn broad_ruggedness_ignores_isolated_steps_that_max_step_promotes() {
        let source = CellId {
            face: Face(4),
            level: 15,
            i: 16_600,
            j: 11_300,
        };
        // A 20 m wall down one lattice column: the cell's maximum step, but a
        // thirty-second of its quads. A uniform 30% grade covers every quad.
        let wall = spatial_summary(|x, _| if x > 16 { 20.0 } else { 0.0 }, [u32::MAX; 32]);
        let pitch = source.sample_pitch_m() as f32;
        let slope = spatial_summary(|_, z| z as f32 * pitch * 0.9, [u32::MAX; 32]);
        for (detail, broad, max_step) in [(&wall, 1.0, 2.0), (&slope, 2.0, 2.0)] {
            let data = TerrainMetricRef {
                height_range: (0.0, 20.0),
                terrain_step_m: detail.query(source, source).unwrap().step_m,
                terrain_relief_m: 20.0,
                terrain_detail: Some(detail),
            };
            let metric = |cell| (cell == source).then_some(data);
            let child = spatial_descendant(source, 1, 1, 0);
            for cell in [source, child] {
                assert_eq!(
                    terrain_mountain_multiplier_with(cell, Some(&metric), RuggednessPolicy::Broad),
                    broad,
                    "{cell:?}"
                );
                assert_eq!(
                    terrain_mountain_multiplier_with(
                        cell,
                        Some(&metric),
                        RuggednessPolicy::MaxStep
                    ),
                    max_step,
                    "{cell:?}"
                );
            }
        }
        // Without spatial metadata both policies keep the recorded scalar.
        let scalar = |cell| {
            (cell == source).then_some(TerrainMetricRef {
                height_range: (0.0, 20.0),
                terrain_step_m: 20.0,
                terrain_relief_m: 20.0,
                terrain_detail: None,
            })
        };
        for policy in [RuggednessPolicy::Broad, RuggednessPolicy::MaxStep] {
            assert_eq!(
                terrain_mountain_multiplier_with(source, Some(&scalar), policy),
                2.0
            );
        }
    }

    #[test]
    fn spatial_terrain_metrics_localize_relief_without_changing_content_or_own_scalar() {
        let detail = spatial_summary(
            |x, z| if x <= 4 && z <= 4 { 1_000.0 } else { 0.0 },
            [u32::MAX; 32],
        );
        for face in 0..6 {
            let source = CellId {
                face: Face(face),
                level: 3,
                i: 2,
                j: 3,
            };
            let data = TerrainMetricRef {
                height_range: (-80_000.0, 3_000.0),
                terrain_step_m: 1_000.0,
                terrain_relief_m: 1_000.0,
                terrain_detail: Some(&detail),
            };
            let metric = |cell| (cell == source).then_some(data);
            let legacy = |cell| {
                (cell == source).then_some(TerrainMetricRef {
                    terrain_detail: None,
                    ..data
                })
            };
            let mountain = spatial_descendant(source, 2, 0, 0);
            let plain = spatial_descendant(source, 2, 3, 3);
            assert_eq!(
                terrain_mountain_multiplier(source, Some(&metric)).to_bits(),
                terrain_mountain_multiplier(source, Some(&legacy)).to_bits()
            );
            assert_eq!(terrain_mountain_multiplier(mountain, Some(&metric)), 2.0);
            assert_eq!(terrain_mountain_multiplier(plain, Some(&metric)), 1.0);
            assert_eq!(terrain_mountain_multiplier(plain, Some(&legacy)), 2.0);
            for cell in [source, mountain, plain] {
                assert_eq!(
                    terrain_metrics_for(cell, Some(&metric)),
                    terrain_metrics_for(cell, Some(&legacy)),
                    "localized uncertainty must not replace recorded content extrema"
                );
            }
            assert_eq!(
                padded_height_range_for(source, terrain_metric_for(source, Some(&metric))),
                padded_height_range_for(source, terrain_metric_for(source, Some(&legacy)))
            );
            let own = |cell| {
                if cell == plain {
                    Some(TerrainMetricRef {
                        terrain_relief_m: 700.0,
                        ..data
                    })
                } else {
                    metric(cell)
                }
            };
            assert_eq!(
                terrain_mountain_multiplier(plain, Some(&own)),
                2.0,
                "an exact measured cell keeps its original scalar preference"
            );
        }
    }

    #[test]
    fn spatial_terrain_metrics_keep_source_pitch_and_conservative_unavailable_fallback() {
        let source = CellId {
            face: Face(0),
            level: 12,
            i: 1_234,
            j: 2_345,
        };
        let pitch = source.sample_pitch_m();
        let detail = spatial_summary(|x, _| (x as f64 * pitch * 0.1) as f32, [u32::MAX; 32]);
        let data = TerrainMetricRef {
            height_range: (-500.0, 1_000.0),
            terrain_step_m: pitch as f32 * 0.1,
            terrain_relief_m: 1_000.0,
            terrain_detail: Some(&detail),
        };
        let metric = |cell| (cell == source).then_some(data);
        for depth in 5..=7 {
            let scale = 1 << (depth - 5);
            let target = spatial_descendant(source, depth, 10 * scale, 12 * scale);
            assert_eq!(
                terrain_mountain_multiplier(target, Some(&metric)),
                1.0,
                "slope must use the measured source pitch at depth {depth}"
            );
        }
        let target = spatial_descendant(source, 7, 40, 48);
        let unavailable = |cell| {
            (cell == source).then_some(TerrainMetricRef {
                terrain_detail: None,
                ..data
            })
        };
        assert_eq!(terrain_mountain_multiplier(target, Some(&unavailable)), 2.0);
        let empty = spatial_summary(|_, _| 9_000.0, [0; 32]);
        let empty_metric = |cell| {
            (cell == source).then_some(TerrainMetricRef {
                terrain_detail: Some(&empty),
                ..data
            })
        };
        assert_eq!(
            terrain_mountain_multiplier(target, Some(&empty_metric)),
            1.0
        );
    }

    #[test]
    fn spatial_terrain_height_uncertainty_preserves_extrema_and_exact_fallbacks() {
        let flat = spatial_summary(|_, _| 0.0, [u32::MAX; 32]);
        let empty = spatial_summary(|_, _| 0.0, [0; 32]);
        for level in [3, 4, 12] {
            let source = CellId::from_dir(DVec3::X, level);
            let target = spatial_descendant(source, 1, 0, 0);
            for (content, expected) in [
                ((0.0, 0.0), (-100.0, 350.0)),
                ((-40_000.0, 3_000.0), (-40_100.0, 3_350.0)),
                ((-12_000.0, 10_500.0), (-12_100.0, 10_850.0)),
            ] {
                for detail in [&flat, &empty] {
                    let metric = TerrainMetricRef {
                        height_range: content,
                        terrain_step_m: 3_000.0,
                        terrain_relief_m: 3_000.0,
                        terrain_detail: Some(detail),
                    };
                    assert_eq!(
                        padded_height_range_for(target, Some((metric, source))),
                        Some(expected)
                    );
                    let original = padded_height_range(Some((content, 3_000.0, 3_000.0, source)));
                    assert_eq!(
                        padded_height_range_for(source, Some((metric, source))),
                        original
                    );
                    assert_eq!(
                        padded_height_range_for(
                            target,
                            Some((
                                TerrainMetricRef {
                                    terrain_detail: None,
                                    ..metric
                                },
                                source
                            ))
                        ),
                        original
                    );
                    let unrelated = CellId {
                        face: Face((source.face.0 + 1) % 6),
                        ..target
                    };
                    assert_eq!(
                        padded_height_range_for(unrelated, Some((metric, source))),
                        original
                    );
                }
            }
            for content in [(f32::NAN, 0.0), (0.0, f32::INFINITY), (10.0, 0.0)] {
                let metric = TerrainMetricRef {
                    height_range: content,
                    terrain_step_m: 0.0,
                    terrain_relief_m: 0.0,
                    terrain_detail: Some(&flat),
                };
                assert_eq!(
                    padded_height_range_for(target, Some((metric, source))),
                    None
                );
            }
            assert_eq!(padded_height_range_for(target, None), None);
        }
    }

    #[test]
    fn spatial_terrain_height_uncertainty_keeps_measured_step_below_grid_resolution() {
        let source = CellId::from_dir(DVec3::X, 3);
        let detail = spatial_summary(|x, _| x as f32 * 400.0, [u32::MAX; 32]);
        let metric = TerrainMetricRef {
            height_range: (-30.0, 12_800.0),
            terrain_step_m: 400.0,
            terrain_relief_m: 12_800.0,
            terrain_detail: Some(&detail),
        };
        for depth in 5..=12 {
            let scale = 1 << (depth - 5);
            let target = spatial_descendant(source, depth, 10 * scale, 12 * scale);
            assert_eq!(
                padded_height_range_for(target, Some((metric, source))),
                Some((-500.0, 13_150.0)),
                "the recorded high peak remains, and source-step uncertainty cannot shrink with target pitch"
            );
            let modest_content = TerrainMetricRef {
                height_range: (-30.0, 500.0),
                ..metric
            };
            assert_eq!(
                padded_height_range_for(target, Some((modest_content, source))),
                Some((-500.0, 1_550.0)),
                "400m measured step requires 800m uncertainty at depth {depth}"
            );
        }
    }

    #[test]
    fn spatial_terrain_flat_globe_descent_keeps_rotterdam_at_aerial_resolution() {
        let mut camera = crate::camera::Camera::start();
        camera.lat = 0.0;
        camera.lon = 0.0;
        camera.aspect = 1600.0 / 900.0;
        camera.alt_m = camera.alt_m.min(camera.max_zoom_out_alt());
        let globe_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (globe, mut split) =
            compute_wanted_for_view(camera.eye_ecef(), 900.0, &globe_view, &HashSet::new());
        let flat = spatial_summary(|_, _| 0.0, [u32::MAX; 32]);
        let metrics: HashMap<_, _> = globe
            .iter()
            .map(|target| {
                (
                    target.cell,
                    TerrainMetricRef {
                        height_range: (-80_000.0, 0.0),
                        terrain_step_m: 0.0,
                        terrain_relief_m: 0.0,
                        terrain_detail: Some(&flat),
                    },
                )
            })
            .collect();
        camera.lat = 51.9225_f64.to_radians();
        camera.lon = 4.47917_f64.to_radians();
        camera.heading = 0.0;
        camera.pitch = 0.0;
        for altitude in [128_000.0, 64_000.0, 32_000.0] {
            camera.alt_m = altitude;
            let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
            let (wanted, next_split, _) = compute_wanted_for_view_with_stats_and_metric_lookup(
                camera.eye_ecef(),
                900.0,
                &view,
                &split,
                &|cell| metrics.get(&cell).copied(),
            );
            let center_level = level_at(&wanted, camera.lat, camera.lon).unwrap();
            let ceiling = if altitude == 32_000.0 {
                vearth_gen::landmarks::LANDMARK_MIN_LEVEL
            } else {
                12
            };
            assert!(
                center_level <= ceiling,
                "{altitude}m flat descent requested L{center_level}"
            );
            if altitude == 32_000.0 {
                assert_eq!(
                    center_level, ceiling,
                    "only the coherent Cube Houses footprint refines beyond aerial terrain"
                );
            }
            assert!(wanted.len() <= MAX_VIEW_WANTED);
            assert_same_face_cut_is_balanced(&wanted);
            split = next_split;
        }
    }

    fn metric_cycle_selection(
        store: &mut ResidentStore,
        split: &HashSet<CellId>,
    ) -> (Vec<WantedCell>, HashSet<CellId>) {
        let mut camera = crate::camera::Camera::start();
        camera.lat = 51.9225_f64.to_radians();
        camera.lon = 4.47917_f64.to_radians();
        camera.alt_m = 32_000.0;
        camera.heading = 0.0;
        camera.pitch = 0.0;
        camera.aspect = 1600.0 / 900.0;
        camera.fov_y = 55.0_f64.to_radians();
        let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        store.begin_metric_query();
        let (wanted, next, _) = compute_wanted_for_view_with_stats_and_metric_lookup(
            camera.eye_ecef(),
            900.0,
            &view,
            split,
            &|cell| store.metric(cell),
        );
        store.finish_metric_query();
        (wanted, next)
    }

    fn metric_cycle_parents() -> [CellId; 5] {
        [
            (3790, 2198),
            (3791, 2197),
            (3791, 2198),
            (3791, 2199),
            (3792, 2198),
        ]
        .map(|(i, j)| CellId {
            face: Face(4),
            level: 12,
            i,
            j,
        })
    }

    fn assert_metric_mesh_cycle_stable(metrics: &[(CellId, crate::lod_selection::TerrainMetric)]) {
        assert_metric_mesh_cycle_stable_with_status(metrics, true);
    }

    fn assert_metric_mesh_cycle_stable_with_status(
        metrics: &[(CellId, crate::lod_selection::TerrainMetric)],
        complete: bool,
    ) {
        let populate = || {
            let mut store = ResidentStore::new();
            for (cell, metric) in metrics {
                store.insert(
                    *cell,
                    0,
                    metric.height_range,
                    metric.terrain_step_m,
                    metric.terrain_relief_m,
                    metric.terrain_detail.clone(),
                    None,
                    64,
                );
                store.set_metric_complete(*cell, complete);
            }
            store
        };
        let signature = |wanted: &[WantedCell]| {
            wanted
                .iter()
                .map(|target| (target.cell, target.screen_px.to_bits(), target.lane))
                .collect::<Vec<_>>()
        };
        let mut store = populate();
        let mut split = HashSet::new();
        let mut before = Vec::new();
        for _ in 0..3 {
            (before, split) = metric_cycle_selection(&mut store, &split);
        }
        let parents = metric_cycle_parents();
        assert!(
            parents.iter().all(|parent| split.contains(parent)),
            "source slope and its four-cell halo must refine the observed parent cross"
        );
        let expected = signature(&before);
        let old_bytes = store.gpu_bytes();
        for parent in parents {
            store.remove(parent).unwrap();
        }
        assert_eq!(old_bytes - store.gpu_bytes(), 5 * 64);
        assert!(
            parents
                .iter()
                .all(|parent| !store.contains(*parent) && store.metric(*parent).is_some())
        );
        for _ in 0..4 {
            let (wanted, next) = metric_cycle_selection(&mut store, &split);
            assert_eq!(
                signature(&wanted),
                expected,
                "mesh eviction changed retained LOD evidence"
            );
            assert_eq!(next, split);
        }
        // The old residency-only lookup is an independent failure control.
        let mut old = populate();
        for parent in parents {
            old.set_metric_complete(parent, false);
            old.remove(parent).unwrap();
        }
        let (changed, _) = metric_cycle_selection(&mut old, &split);
        assert_ne!(
            signature(&changed),
            expected,
            "fixture must reproduce the erased-parent selection change"
        );
        // Reloading identical data restores only residency, never a new cut.
        for parent in parents {
            let metric = &metrics.iter().find(|(cell, _)| *cell == parent).unwrap().1;
            store.insert(
                parent,
                1,
                metric.height_range,
                metric.terrain_step_m,
                metric.terrain_relief_m,
                metric.terrain_detail.clone(),
                None,
                64,
            );
            store.set_metric_complete(parent, complete);
        }
        let (reloaded, _) = metric_cycle_selection(&mut store, &split);
        assert_eq!(signature(&reloaded), expected);
        store.clear();
        assert!(
            metrics
                .iter()
                .all(|(cell, _)| store.metric(*cell).is_none())
        );
    }

    #[test]
    fn retained_metrics_keep_stationary_parent_and_halo_cut_across_mesh_eviction() {
        let parents = metric_cycle_parents();
        let ranges = [
            (-157.09995, 7.756959),
            (-157.13931, 1.5005343),
            (-156.13591, 2.3989596),
            (-155.74423, 7.801166),
            (-155.84059, 3.3788812),
        ];
        let steps = [2.1155887, 2.5485537, 61.563374, 3.9306645, 2.7978697];
        let reliefs = [6.6046677, 4.8475003, 64.97048, 7.23431, 5.5149484];
        let mut metrics: Vec<_> = (0..5)
            .map(|i| {
                (
                    parents[i],
                    crate::lod_selection::TerrainMetric {
                        height_range: ranges[i],
                        terrain_step_m: steps[i],
                        terrain_relief_m: reliefs[i],
                        terrain_detail: None,
                    },
                )
            })
            .collect();
        metrics.push((
            parents[2].parent().unwrap().parent().unwrap(),
            crate::lod_selection::TerrainMetric {
                height_range: (-614.0448, 2.4217167),
                terrain_step_m: 7.0254416,
                terrain_relief_m: 8.569208,
                terrain_detail: Some(Arc::new(spatial_summary(|_, _| 0.0, [u32::MAX; 32]))),
            },
        ));
        assert_metric_mesh_cycle_stable(&metrics);
    }

    #[test]
    fn active_partial_metrics_keep_stationary_parent_and_halo_cut_across_mesh_eviction() {
        let parents = metric_cycle_parents();
        let ranges = [
            (-157.09995, 7.756959),
            (-157.13931, 1.5005343),
            (-156.13591, 2.3989596),
            (-155.74423, 7.801166),
            (-155.84059, 3.3788812),
        ];
        let steps = [2.1155887, 2.5485537, 61.563374, 3.9306645, 2.7978697];
        let reliefs = [6.6046677, 4.8475003, 64.97048, 7.23431, 5.5149484];
        // Each lattice alternates by the recorded step, so every owned quad
        // carries it and the spatial summary agrees with the scalar metrics.
        let zigzag = |step: f32| {
            Arc::new(spatial_summary(
                move |x, _| if x % 2 == 1 { step } else { 0.0 },
                [u32::MAX; 32],
            ))
        };
        let mut metrics: Vec<_> = (0..5)
            .map(|i| {
                (
                    parents[i],
                    crate::lod_selection::TerrainMetric {
                        height_range: ranges[i],
                        terrain_step_m: steps[i],
                        terrain_relief_m: reliefs[i],
                        terrain_detail: Some(zigzag(steps[i])),
                    },
                )
            })
            .collect();
        metrics.push((
            parents[2].parent().unwrap().parent().unwrap(),
            crate::lod_selection::TerrainMetric {
                height_range: (-614.0448, 2.4217167),
                terrain_step_m: 7.0254416,
                terrain_relief_m: 8.569208,
                terrain_detail: Some(zigzag(7.0254416)),
            },
        ));
        assert_metric_mesh_cycle_stable_with_status(&metrics, false);
    }

    #[test]
    #[ignore = "requires a bounded export of complete cached source metrics"]
    fn retained_metrics_cached_rotterdam_stationary_cycle() {
        let path = vearth_platform::env::var("VEARTH_RETAINED_METRIC_FIXTURE")
            .expect("set VEARTH_RETAINED_METRIC_FIXTURE");
        let bytes = std::fs::read(path).unwrap();
        let records: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let rows = records.as_array().unwrap();
        assert_eq!(
            rows.len(),
            16,
            "the bounded source cohort must remain unchanged"
        );
        let metrics: Vec<_> = rows
            .iter()
            .map(|row| {
                assert_eq!(row["complete"].as_u64(), Some(1));
                let id = row["cell"].as_array().unwrap();
                let cell = CellId {
                    face: Face(id[0].as_u64().unwrap() as u8),
                    level: id[1].as_u64().unwrap() as u8,
                    i: id[2].as_u64().unwrap() as u32,
                    j: id[3].as_u64().unwrap() as u32,
                };
                let heights: Vec<f32> = row["grid"]["heights"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap() as f32)
                    .collect();
                let columns: Vec<u32> = row["grid"]["source_columns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u32)
                    .collect();
                let grid = vearth_gen::TerrainDetailGrid {
                    heights_m: heights.into_boxed_slice().try_into().unwrap(),
                    source_columns: columns.try_into().unwrap(),
                };
                let detail = vearth_gen::TerrainDetailSummary::from_grid(&grid).unwrap();
                (
                    cell,
                    crate::lod_selection::TerrainMetric {
                        height_range: (
                            row["height_range"][0].as_f64().unwrap() as f32,
                            row["height_range"][1].as_f64().unwrap() as f32,
                        ),
                        terrain_step_m: row["step"].as_f64().unwrap() as f32,
                        terrain_relief_m: row["relief"].as_f64().unwrap() as f32,
                        terrain_detail: Some(Arc::new(detail)),
                    },
                )
            })
            .collect();
        let center = &metrics
            .iter()
            .find(|(cell, _)| *cell == metric_cycle_parents()[2])
            .unwrap()
            .1;
        assert_eq!(
            center.terrain_step_m.to_bits(),
            61.56337356567383_f32.to_bits()
        );
        assert_eq!(
            center.terrain_relief_m.to_bits(),
            64.9704818725586_f32.to_bits()
        );
        assert_metric_mesh_cycle_stable(&metrics);
    }

    #[test]
    fn spatial_terrain_resident_replacement_releases_old_summary_without_changing_identity() {
        let cell = resident_test_cell(3);
        let detail = Arc::new(spatial_summary(|x, _| x as f32, [u32::MAX; 32]));
        let mut store = ResidentStore::new();
        let id = store.insert(
            cell,
            1,
            (-100.0, 100.0),
            1.0,
            32.0,
            Some(detail.clone()),
            None,
            50,
        );
        assert_eq!(Arc::strong_count(&detail), 3);
        assert!(std::ptr::eq(
            store.metric(cell).unwrap().terrain_detail.unwrap(),
            detail.as_ref()
        ));
        assert_eq!(
            store.insert(cell, 2, (-100.0, 100.0), 1.0, 32.0, None, None, 50),
            id
        );
        assert_eq!(Arc::strong_count(&detail), 1);
        assert!(store.metric(cell).unwrap().terrain_detail.is_none());
        assert_eq!(store.coverage[&CellKey::from(cell)].members, 1);
        store.remove_id(id).unwrap();
        assert!(store.metric(cell).is_none());
    }

    #[test]
    fn spatial_terrain_rotterdam_globe_history_does_not_import_a_remote_ridge() {
        let mut camera = crate::camera::Camera::start();
        camera.lat = 0.0;
        camera.lon = 0.0;
        camera.aspect = 1600.0 / 900.0;
        camera.alt_m = camera.alt_m.min(camera.max_zoom_out_alt());
        let globe_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (globe, initial_split) =
            compute_wanted_for_view(camera.eye_ecef(), 900.0, &globe_view, &HashSet::new());
        camera.lat = 51.9225_f64.to_radians();
        camera.lon = 4.47917_f64.to_radians();
        camera.alt_m = 32_000.0;
        camera.pitch = 0.0;
        camera.heading = 0.0;
        let source = CellId::from_dir(camera.eye_ecef(), 4);
        let fine = CellId::from_dir(camera.eye_ecef(), source.level + 5);
        let local_x = fine.i - (source.i << 5);
        let local_z = 31 - (fine.j - (source.j << 5));
        // A measured mountain in the opposite source quadrant must not grant
        // the flat destination an extra level merely because parents are absent.
        let ridge_x = if local_x < 16 { 28 } else { 4 };
        let ridge_z = if local_z < 16 { 28 } else { 4 };
        let detail = spatial_summary(
            |x, z| {
                if x.abs_diff(ridge_x) <= 2 && z.abs_diff(ridge_z) <= 2 {
                    3_000.0
                } else {
                    0.0
                }
            },
            [u32::MAX; 32],
        );
        let mut metrics: HashMap<CellId, TerrainMetricRef<'_>> = globe
            .iter()
            .map(|target| (target.cell, ((0.0, 0.0), 0.0, 0.0).into()))
            .collect();
        metrics.insert(
            source,
            TerrainMetricRef {
                height_range: (-40_000.0, 3_000.0),
                terrain_step_m: 3_000.0,
                terrain_relief_m: 3_000.0,
                terrain_detail: Some(&detail),
            },
        );
        let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let mut prior = initial_split;
        let mut old_final_level = 0;
        for _ in 0..3 {
            let old = |cell| {
                metrics.get(&cell).copied().map(|metric| TerrainMetricRef {
                    terrain_detail: None,
                    ..metric
                })
            };
            let (wanted, split, _) = compute_wanted_for_view_with_stats_and_metric_lookup(
                camera.eye_ecef(),
                900.0,
                &view,
                &prior,
                &old,
            );
            old_final_level = level_at(&wanted, camera.lat, camera.lon).unwrap();
            for target in wanted {
                metrics
                    .entry(target.cell)
                    .or_insert(((0.0, 0.0), 0.0, 0.0).into());
            }
            prior = split;
        }
        let localized = |cell| metrics.get(&cell).copied();
        let (wanted, _, _) = compute_wanted_for_view_with_stats_and_metric_lookup(
            camera.eye_ecef(),
            900.0,
            &view,
            &prior,
            &localized,
        );
        let new_level = level_at(&wanted, camera.lat, camera.lon).unwrap();
        assert!(
            new_level <= old_final_level,
            "global L{old_final_level}, spatial L{new_level}"
        );
        let missing_parent = CellId::from_dir(camera.eye_ecef(), old_final_level - 1);
        assert!(!metrics.contains_key(&missing_parent));
        let old = |cell| {
            metrics.get(&cell).copied().map(|metric| TerrainMetricRef {
                terrain_detail: None,
                ..metric
            })
        };
        assert_eq!(terrain_mountain_multiplier(missing_parent, Some(&old)), 2.0);
        assert_eq!(
            terrain_mountain_multiplier(missing_parent, Some(&localized)),
            1.0
        );
        assert!(
            new_level >= 11,
            "terrain still follows the aerial screen-error demand"
        );
        assert!(wanted.len() <= MAX_VIEW_WANTED);
        assert_same_face_cut_is_balanced(&wanted);
    }

    #[test]
    fn mountain_metric_is_flat_neutral_and_caps_at_one_extra_level() {
        assert_eq!(mountain_detail_multiplier(20.0, 2.0, 75.0), 1.0);
        let rugged = mountain_detail_multiplier(1_000.0, 60.0, 75.0);
        assert!(
            (rugged - MOUNTAIN_EXTRA_LEVEL_FACTOR).abs() < f64::EPSILON,
            "rugged multiplier {rugged}"
        );
    }

    #[test]
    fn mountain_relief_retains_original_response_without_content_height() {
        for relief in [0.0, 20.0, 180.0, 350.0, 700.0, 1_000.0] {
            for (step, pitch) in [(0.0, 75.0), (2.0, 75.0), (60.0, 75.0)] {
                let old = 1.0
                    + (MOUNTAIN_EXTRA_LEVEL_FACTOR - 1.0)
                        * smoothstep(180.0, 700.0, f64::from(relief)).max(smoothstep(
                            0.15,
                            0.65,
                            f64::from(step) / pitch,
                        ));
                assert_eq!(
                    mountain_detail_multiplier(relief, step, pitch).to_bits(),
                    old.to_bits()
                );
            }
        }
        assert_eq!(mountain_detail_multiplier(700.0, 0.0, 75.0), 2.0);
    }

    #[test]
    fn boulder_like_rugged_terrain_receives_a_finer_visible_leaf() {
        let camera = crate::camera::Camera {
            lat: 40.015f64.to_radians(),
            lon: (-105.2705f64).to_radians(),
            alt_m: 4_000.0,
            heading: (-90f64).to_radians(),
            pitch: 70f64.to_radians(),
            fov_y: 55f64.to_radians(),
            aspect: 1.6,
        };
        let eye = camera.eye_ecef();
        let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let mountain_lat = 40.02f64.to_radians();
        let mountain_lon = (-105.55f64).to_radians();
        let mountain_dir = ellipsoid::geodetic_to_ecef(mountain_lat, mountain_lon, 0.0).normalize();
        let metric_cell = CellId::from_dir(mountain_dir, 9);
        let heights = HashMap::from([(metric_cell, (1_500.0, 3_000.0))]);
        let terrain_steps = HashMap::from([(metric_cell, 90.0)]);

        let (flat, _) = compute_wanted_for_view(eye, 1_280.0, &view, &HashSet::new());
        let metric = |cell: CellId| {
            heights.get(&cell).copied().map(|height| {
                (
                    height,
                    terrain_steps.get(&cell).copied().unwrap_or_default(),
                    height.1 - height.0,
                )
                    .into()
            })
        };
        let (rugged, _, _) = compute_wanted_for_view_with_stats_and_metric_lookup(
            eye,
            1_280.0,
            &view,
            &HashSet::new(),
            &metric,
        );
        let flat_level = level_at(&flat, mountain_lat, mountain_lon).unwrap();
        let rugged_level = level_at(&rugged, mountain_lat, mountain_lon).unwrap();
        assert!(
            rugged_level > flat_level,
            "rugged terrain stayed at L{rugged_level}; flat terrain was L{flat_level}"
        );
        assert!(rugged.len() <= MAX_VIEW_WANTED);
        assert_same_face_cut_is_balanced(&rugged);
    }

    #[test]
    fn golden_gate_uses_one_landmark_lod_across_the_full_span() {
        let region = vearth_gen::landmarks::lod_regions()
            .find(|region| region.name == "golden-gate-bridge")
            .unwrap();
        let camera = crate::camera::Camera {
            lat: region.lat_rad,
            lon: region.lon_rad,
            alt_m: 6_000.0,
            heading: region.heading_rad,
            pitch: 0.0,
            fov_y: 55f64.to_radians(),
            aspect: 1.28,
        };
        let (wanted, _) = compute_wanted(camera.eye_ecef(), 1_000.0, camera.fov_y, &HashSet::new());
        let (sin_h, cos_h) = region.heading_rad.sin_cos();
        let mut levels = std::collections::BTreeSet::new();
        for mz in (-1_000..=1_000).step_by(25) {
            let east = mz as f64 * sin_h;
            let north = mz as f64 * cos_h;
            let lat = region.lat_rad + north / ellipsoid::MEAN_RADIUS;
            let lon = region.lon_rad + east / (ellipsoid::MEAN_RADIUS * region.lat_rad.cos());
            levels.insert(level_at(&wanted, lat, lon).expect("bridge point remains in wanted cut"));
        }
        assert_eq!(levels.len(), 1, "bridge crossed terrain LODs: {levels:?}");
        assert_same_face_cut_is_balanced(&wanted);
    }

    #[test]
    fn merlion_alone_can_request_the_half_pitch_detail_level() {
        let region = vearth_gen::landmarks::lod_regions()
            .find(|region| region.name == "singapore-merlion")
            .unwrap();
        assert_eq!(region.detail_level_bias, 1);
        let eye = ellipsoid::geodetic_to_ecef(region.lat_rad, region.lon_rad, 30.0);
        let lock = landmark_locks(eye, 1_000.0, 55f64.to_radians())
            .into_iter()
            .find(|lock| lock.region.name == region.name)
            .unwrap();
        assert_eq!(lock.target_level, MAX_STREAM_LEVEL);

        let ordinary = vearth_gen::landmarks::lod_regions()
            .find(|region| region.name == "chrysler-building")
            .unwrap();
        assert_eq!(ordinary.detail_level_bias, 0);
        let eye = ellipsoid::geodetic_to_ecef(ordinary.lat_rad, ordinary.lon_rad, 100.0);
        let lock = landmark_locks(eye, 1_000.0, 55f64.to_radians())
            .into_iter()
            .find(|lock| lock.region.name == ordinary.name)
            .unwrap();
        assert_eq!(lock.target_level, STANDARD_MAX_STREAM_LEVEL);
    }

    #[test]
    fn hysteresis_holds_split_cells_near_the_threshold() {
        // A camera position produces some cut; feeding the split set back
        // at the same position must reproduce the same cut (stable), and
        // the split memory must keep cells split when the camera backs off
        // slightly past the split threshold.
        let cam = DVec3::new(ellipsoid::MEAN_RADIUS + 50_000.0, 0.0, 0.0);
        let (a, split_a) = compute_wanted(cam, 1080.0, 1.0, &HashSet::new());
        let (b, split_b) = compute_wanted(cam, 1080.0, 1.0, &split_a);
        assert_eq!(a.len(), b.len(), "same pose + memory must be stable");
        assert_eq!(split_a.len(), split_b.len());

        // Back off by ~15%: without memory some boundary cells merge;
        // with memory the cut must be at least as fine (no merges yet).
        let cam_back = DVec3::new(ellipsoid::MEAN_RADIUS + 57_500.0, 0.0, 0.0);
        let (fresh, _) = compute_wanted(cam_back, 1080.0, 1.0, &HashSet::new());
        let (held, _) = compute_wanted(cam_back, 1080.0, 1.0, &split_a);
        assert!(
            held.len() >= fresh.len(),
            "hysteresis must hold splits: held {} < fresh {}",
            held.len(),
            fresh.len()
        );
    }

    #[test]
    fn legacy_wanted_selection_is_deterministic_and_orientation_independent() {
        // Heading and pitch are intentionally absent from compute_wanted: a
        // fixed eye must retain the exact desired cut while request priority
        // changes around it.
        let eye = DVec3::new(ellipsoid::MEAN_RADIUS + 12_000.0, 500.0, -250.0);
        let (first, split) = compute_wanted(eye, 1080.0, 1.0, &HashSet::new());
        let (second, split_second) = compute_wanted(eye, 1080.0, 1.0, &split);
        let signature = |wanted: &[WantedCell]| {
            wanted
                .iter()
                .map(|wanted| (wanted.cell, wanted.screen_px.to_bits()))
                .collect::<Vec<_>>()
        };
        assert_eq!(signature(&first), signature(&second));
        assert_eq!(split, split_second);
    }

    #[test]
    fn stream_priority_preserves_view_lanes_and_legacy_scores() {
        let cell = CellId {
            face: Face(0),
            level: 8,
            i: 100,
            j: 50,
        };
        let wanted = |lane| WantedCell {
            cell,
            screen_px: 4.0,
            lane,
        };
        let visible = wanted(ViewLane::Visible).stream_priority();
        let guard = wanted(ViewLane::Guard).stream_priority();
        let background = wanted(ViewLane::Background).stream_priority();
        assert!(visible > guard && guard > background);
        assert!(background < 1.0, "background must trail speculative work");
        assert_eq!(wanted(ViewLane::Legacy).stream_priority(), 4.0);

        let mut ordered = [
            wanted(ViewLane::Background),
            wanted(ViewLane::Visible),
            wanted(ViewLane::Guard),
        ];
        ordered.sort_by(WantedCell::stream_order);
        assert_eq!(
            ordered.iter().map(|wanted| wanted.lane).collect::<Vec<_>>(),
            vec![ViewLane::Visible, ViewLane::Guard, ViewLane::Background]
        );
    }

    #[test]
    fn view_aware_selection_tracks_heading_aspect_and_guard_turns() {
        let mut camera = crate::camera::Camera {
            lat: (-31.8787f64).to_radians(),
            lon: (-70.0182f64).to_radians(),
            alt_m: 3_900.0,
            heading: 0.0,
            pitch: 70f64.to_radians(),
            fov_y: 55f64.to_radians(),
            aspect: 1.6,
        };
        let eye = camera.eye_ecef();
        let north_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (north, north_split) =
            compute_wanted_for_view(eye, 1_280.0, &north_view, &HashSet::new());
        let (north_again, north_split_again) =
            compute_wanted_for_view(eye, 1_280.0, &north_view, &north_split);
        let signature = |wanted: &[WantedCell]| {
            wanted
                .iter()
                .map(|wanted| wanted.cell)
                .collect::<HashSet<_>>()
        };
        assert_eq!(signature(&north), signature(&north_again));
        assert_eq!(north_split, north_split_again);

        camera.heading = 90f64.to_radians();
        let east_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (east, _) = compute_wanted_for_view(eye, 1_280.0, &east_view, &HashSet::new());
        assert!(
            signature(&north)
                .symmetric_difference(&signature(&east))
                .count()
                > 100,
            "a quarter turn must move detailed selection into the new view"
        );

        camera.heading = 3f64.to_radians();
        let guard_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (_, guard_split) = compute_wanted_for_view(eye, 1_280.0, &guard_view, &north_split);
        assert!(
            north_split.symmetric_difference(&guard_split).count() < MAX_VIEW_WANTED / 3,
            "a small turn inside the guard band changed too much topology"
        );

        camera.heading = 0.0;
        camera.aspect = 0.7;
        let portrait_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (portrait, _) = compute_wanted_for_view(eye, 1_280.0, &portrait_view, &HashSet::new());
        assert_ne!(
            signature(&north),
            signature(&portrait),
            "selection must account for viewport aspect"
        );
    }

    #[test]
    fn reported_andes_view_gives_visible_terrain_first_claim_on_budget() {
        let mut camera = crate::camera::Camera {
            lat: (-31.8787f64).to_radians(),
            lon: (-70.0182f64).to_radians(),
            alt_m: 3_900.0,
            heading: 0.0,
            pitch: 70f64.to_radians(),
            fov_y: 55f64.to_radians(),
            aspect: 1.6,
        };
        let eye = camera.eye_ecef();
        let view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (legacy, _) = compute_wanted(eye, 1_280.0, camera.fov_y, &HashSet::new());
        let (wanted, _, stats) =
            compute_wanted_for_view_with_stats(eye, 1_280.0, &view, &HashSet::new());

        let summarize = |mut cells: Vec<&WantedCell>| {
            cells.sort_by(|a, b| a.screen_px.total_cmp(&b.screen_px));
            let p95 = cells[cells.len() * 95 / 100].screen_px;
            let max_px = cells.last().unwrap().screen_px;
            (cells.len(), p95, max_px)
        };
        let legacy_summary = summarize(
            legacy
                .iter()
                .filter(|wanted| {
                    wanted.cell.level < MAX_STREAM_LEVEL
                        && view.lane(eye, wanted.cell) == ViewLane::Visible
                })
                .collect(),
        );
        let view_summary = summarize(
            wanted
                .iter()
                .filter(|wanted| {
                    wanted.cell.level < MAX_STREAM_LEVEL && wanted.lane == ViewLane::Visible
                })
                .collect(),
        );
        let guard_summary = summarize(
            wanted
                .iter()
                .filter(|wanted| {
                    wanted.cell.level < MAX_STREAM_LEVEL && wanted.lane == ViewLane::Guard
                })
                .collect(),
        );

        assert!(stats.budget_saturated);
        assert!(wanted.len() <= MAX_VIEW_WANTED);
        assert!(
            legacy_summary.2 > 8.0,
            "fixture no longer exposes the regression"
        );
        assert!(
            view_summary.1 < legacy_summary.1 * 0.80,
            "visible p95 error did not materially improve: legacy {legacy_summary:?}, view {view_summary:?}"
        );
        assert!(
            view_summary.1 <= 7.5,
            "visible p95 error exceeds the fixture quality bound: {view_summary:?}"
        );
        assert!(
            view_summary.2 <= TAU_PX * 2.7,
            "visible splittable terrain exceeds the guarded quality bound: {view_summary:?}"
        );
        assert!(
            guard_summary.2 <= view_summary.2 * 1.5,
            "guard must remain approximately one LOD behind visible terrain: visible {view_summary:?}, guard {guard_summary:?}"
        );

        camera.heading = 3f64.to_radians();
        let turned_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let turned_from_guard = summarize(
            wanted
                .iter()
                .filter(|wanted| {
                    wanted.cell.level < MAX_STREAM_LEVEL
                        && wanted.lane == ViewLane::Guard
                        && turned_view.lane(eye, wanted.cell) == ViewLane::Visible
                })
                .collect(),
        );
        assert!(
            turned_from_guard.2 <= view_summary.2 * 1.5,
            "a small guarded turn exposes terrain too coarse for the prior view: visible {view_summary:?}, turned {turned_from_guard:?}"
        );

        let edge_adjacent = |a: CellId, b: CellId| {
            if a.face != b.face {
                return false;
            }
            let level = a.level.max(b.level);
            let a_scale = 1u32 << (level - a.level);
            let b_scale = 1u32 << (level - b.level);
            let (ai0, aj0) = (a.i * a_scale, a.j * a_scale);
            let (bi0, bj0) = (b.i * b_scale, b.j * b_scale);
            let (ai1, aj1) = ((a.i + 1) * a_scale, (a.j + 1) * a_scale);
            let (bi1, bj1) = ((b.i + 1) * b_scale, (b.j + 1) * b_scale);
            ((ai1 == bi0 || bi1 == ai0) && aj0 < bj1 && bj0 < aj1)
                || ((aj1 == bj0 || bj1 == aj0) && ai0 < bi1 && bi0 < ai1)
        };
        let visible: Vec<_> = wanted
            .iter()
            .filter(|wanted| wanted.lane == ViewLane::Visible)
            .collect();
        let guard: Vec<_> = wanted
            .iter()
            .filter(|wanted| wanted.lane == ViewLane::Guard)
            .collect();
        let max_boundary_delta = visible
            .iter()
            .flat_map(|visible| {
                guard.iter().filter_map(|guard| {
                    edge_adjacent(visible.cell, guard.cell)
                        .then_some(visible.cell.level.abs_diff(guard.cell.level))
                })
            })
            .max()
            .unwrap();
        assert!(
            max_boundary_delta <= 1,
            "visible/guard boundary exceeds the L-1 continuity contract: {max_boundary_delta}"
        );

        let leaves: HashSet<_> = wanted.iter().map(|wanted| wanted.cell).collect();
        assert_eq!(leaves.len(), wanted.len(), "wanted cut contains duplicates");
        for leaf in &leaves {
            let mut ancestor = leaf.parent();
            while let Some(cell) = ancestor {
                assert!(!leaves.contains(&cell), "wanted cut is not an antichain");
                ancestor = cell.parent();
            }
        }
    }

    #[test]
    fn budget_saturated_sub_meter_jitter_has_bounded_topology_change() {
        let eye = DVec3::new(ellipsoid::MEAN_RADIUS + 2_000.0, 0.0, 0.0);
        let (_, split, first_stats) = compute_wanted_with_stats(eye, 1080.0, 1.0, &HashSet::new());
        assert!(first_stats.budget_saturated);
        let (_, _, jitter_stats) =
            compute_wanted_with_stats(eye + DVec3::new(0.25, -0.25, 0.1), 1080.0, 1.0, &split);
        assert!(jitter_stats.budget_saturated);
        assert!(
            jitter_stats.topology_changes <= MAX_WANTED / 8,
            "tiny motion changed {} split decisions",
            jitter_stats.topology_changes
        );
    }

    fn queue_cell(i: u32, j: u32) -> CellId {
        CellId {
            face: Face(0),
            level: MIN_RENDER_LEVEL,
            i,
            j,
        }
    }

    fn test_pool_shared() -> Arc<PoolShared> {
        Arc::new(PoolShared {
            state: std::sync::Mutex::new(PoolState::default()),
            cv: std::sync::Condvar::new(),
            ready_cv: std::sync::Condvar::new(),
            #[cfg(feature = "async-providers")]
            desired_version: tokio::sync::watch::channel(0).0,
            stop: std::sync::atomic::AtomicBool::new(false),
            generated: std::sync::atomic::AtomicU64::new(0),
            cache_hits: std::sync::atomic::AtomicU64::new(0),
        })
    }

    fn idle_test_pool(staging_limit: usize) -> GenPool {
        let shared = test_pool_shared();
        shared.state.lock().unwrap().staging_limit = Some(staging_limit);
        GenPool {
            shared,
            rx_done: sync_channel(1).1,
            registry: Arc::new(Registry::new()),
            cache: None,
            staged_pipeline: true,
            #[cfg(feature = "async-providers")]
            _acquisition_runtime: None,
            #[cfg(feature = "async-providers")]
            async_acquisition: false,
        }
    }

    fn prepared_test_job(work: WorkItem) -> CpuJob {
        CpuJob::Prepared {
            work,
            acquired: AcquiredSources {
                #[cfg(feature = "async-providers")]
                retained: Default::default(),
                complete: true,
                features: None,
            },
        }
    }

    #[test]
    fn ready_queue_cached_bias_preserves_priority_bands_and_large_score_gaps() {
        let cached = queue_cell(0, 0);
        let cold = queue_cell(1, 0);
        for (cached_score, cold_score, expected) in [
            (1.0e200, 32.0e200, cached),
            (1.0e100, 1.0e200, cold),
            (1.0e-100, 1.0e100, cold),
            (1.0e200, 1.0e220, cold),
            (1.0, 65.0, cold),
        ] {
            let mut pool = idle_test_pool(2);
            pool.sync_queue(&[(cached, cached_score), (cold, cold_score)]);
            for _ in 0..2 {
                let work = take_pending(&pool.shared).unwrap();
                let job = if work.cell == cached {
                    CpuJob::Cached {
                        column: crate::prepared_render::test_surface(cached),
                        work,
                    }
                } else {
                    prepared_test_job(work)
                };
                assert!(publish_ready(&pool.shared, job));
            }
            assert_eq!(
                take_ready(&pool.shared).unwrap().work().cell,
                expected,
                "cached score {cached_score}, cold score {cold_score}"
            );
        }
    }

    #[test]
    fn ready_queue_cached_bias_uses_live_scores_and_cell_ties() {
        let mut pool = idle_test_pool(3);
        let cells = [queue_cell(0, 0), queue_cell(1, 0), queue_cell(2, 0)];
        pool.sync_queue(&[(cells[0], 1.0), (cells[1], 100.0), (cells[2], 4.0)]);
        for _ in 0..3 {
            let work = take_pending(&pool.shared).unwrap();
            let job = if work.cell == cells[1] {
                prepared_test_job(work)
            } else {
                CpuJob::Cached {
                    column: crate::prepared_render::test_surface(work.cell),
                    work,
                }
            };
            assert!(publish_ready(&pool.shared, job));
        }
        let revisions = pool.shared.state.lock().unwrap().running.clone();
        pool.sync_queue(&[(cells[0], 2.0), (cells[1], 128.0), (cells[2], 1.0)]);
        assert_eq!(pool.shared.state.lock().unwrap().running, revisions);
        for cell in cells {
            assert_eq!(take_ready(&pool.shared).unwrap().work().cell, cell);
        }
        let state = pool.shared.state.lock().unwrap();
        assert!(state.staging.is_empty());
        assert_eq!(state.running, revisions);
    }

    #[test]
    fn ready_queue_cached_bias_preserves_cancellation_and_staging_capacity() {
        let mut pool = idle_test_pool(2);
        let cold = queue_cell(0, 0);
        let cached = queue_cell(1, 0);
        let pending = queue_cell(2, 0);
        pool.sync_queue(&[(cold, 3.0), (cached, 2.0), (pending, 1.0)]);
        let cold_work = take_pending(&pool.shared).unwrap();
        let cached_work = take_pending(&pool.shared).unwrap();
        assert_eq!(cold_work.cell, cold, "pending ordering is unchanged");
        assert_eq!(cached_work.cell, cached);
        assert!(publish_ready(&pool.shared, prepared_test_job(cold_work)));
        assert!(publish_ready(
            &pool.shared,
            CpuJob::Cached {
                column: crate::prepared_render::test_surface(cached),
                work: cached_work,
            },
        ));
        {
            let mut state = pool.shared.state.lock().unwrap();
            assert!(pop_current_pending(&mut state).is_none());
            assert_eq!(state.staging.len(), 2);
        }
        pool.sync_queue(&[(cold, 3.0), (pending, 1.0)]);
        assert_eq!(pool.shared.state.lock().unwrap().staging.len(), 1);
        let next = take_pending(&pool.shared).unwrap();
        assert_eq!(next.cell, pending);
        assert!(!publish_ready(&pool.shared, prepared_test_job(cached_work)));
        assert!(publish_ready(
            &pool.shared,
            CpuJob::Cached {
                column: crate::prepared_render::test_surface(pending),
                work: next,
            },
        ));
        assert_eq!(pool.shared.state.lock().unwrap().staging.len(), 2);
        assert_eq!(take_ready(&pool.shared).unwrap().work(), next);
        assert_eq!(take_ready(&pool.shared).unwrap().work(), cold_work);
        let mut state = pool.shared.state.lock().unwrap();
        assert!(pop_current_ready(&mut state).is_none());
        assert!(state.staging.is_empty());
        assert!(!state.running.contains_key(&cached));
        assert_eq!(state.running.len(), 2);
    }

    #[test]
    fn ready_queue_late_visible_work_overtakes_cached_background_work() {
        let mut pool = idle_test_pool(3);
        let low = queue_cell(0, 0);
        let high = queue_cell(1, 0);
        pool.sync_queue(&[(low, 1.0)]);
        let low_work = take_pending(&pool.shared).unwrap();
        assert!(publish_ready(
            &pool.shared,
            CpuJob::Cached {
                column: crate::prepared_render::test_surface(low),
                work: low_work,
            },
        ));
        pool.sync_queue(&[(low, 1.0), (high, 100.0)]);
        let high_work = take_pending(&pool.shared).unwrap();
        assert!(publish_ready(&pool.shared, prepared_test_job(high_work)));
        assert_eq!(take_ready(&pool.shared).unwrap().work(), high_work);
        assert_eq!(take_ready(&pool.shared).unwrap().work(), low_work);
        let state = pool.shared.state.lock().unwrap();
        assert!(state.ready.is_empty());
        assert!(state.staging.is_empty());
        assert_eq!(
            state.running.len(),
            2,
            "CPU claims retain completion ownership"
        );
    }

    #[test]
    fn ready_queue_uses_latest_scores_without_reacquiring_or_duplicating_jobs() {
        let mut pool = idle_test_pool(3);
        let cells = [queue_cell(0, 0), queue_cell(1, 0), queue_cell(2, 0)];
        pool.sync_queue(&[(cells[0], 1.0), (cells[1], 2.0), (cells[2], 3.0)]);
        let mut works = Vec::new();
        for _ in 0..3 {
            let work = take_pending(&pool.shared).unwrap();
            assert!(publish_ready(&pool.shared, prepared_test_job(work)));
            works.push(work);
        }
        let revisions = pool.shared.state.lock().unwrap().running.clone();
        pool.sync_queue(&[(cells[0], 100.0), (cells[1], 100.0), (cells[2], 0.0)]);
        assert_eq!(pool.shared.state.lock().unwrap().running, revisions);
        assert!(!publish_ready(&pool.shared, prepared_test_job(works[0])));
        assert_eq!(pool.shared.state.lock().unwrap().ready.len(), 3);
        for cell in cells {
            let work = take_ready(&pool.shared).unwrap().work();
            assert_eq!(
                work.cell, cell,
                "new priorities and CellId ties control claim order"
            );
            assert!(!publish_ready(&pool.shared, prepared_test_job(work)));
        }
        pool.sync_queue(&[(cells[0], 200.0), (cells[1], 1.0), (cells[2], 3.0)]);
        let mut state = pool.shared.state.lock().unwrap();
        assert!(pop_current_pending(&mut state).is_none());
        assert!(pop_current_ready(&mut state).is_none());
        assert_eq!(
            state.running, revisions,
            "completed handoff owns each revision until drain"
        );
    }

    #[test]
    fn staging_capacity_bounds_all_pre_cpu_jobs_and_keeps_waiting_intents_replaceable() {
        let mut pool = idle_test_pool(2);
        let cells = [queue_cell(0, 0), queue_cell(1, 0), queue_cell(2, 0)];
        pool.sync_queue(&[(cells[0], 3.0), (cells[1], 2.0), (cells[2], 1.0)]);
        let first = take_pending(&pool.shared).unwrap();
        let second = take_pending(&pool.shared).unwrap();
        assert!(publish_ready(&pool.shared, prepared_test_job(first)));
        assert!(publish_ready(&pool.shared, prepared_test_job(second)));
        {
            let mut state = pool.shared.state.lock().unwrap();
            assert!(pop_current_pending(&mut state).is_none());
            assert_eq!(state.staging.len(), 2);
            assert_eq!(state.pending_set.len(), 1);
        }
        let high = queue_cell(3, 0);
        pool.sync_queue(&[
            (cells[0], 3.0),
            (cells[1], 2.0),
            (cells[2], 1.0),
            (high, 100.0),
        ]);
        assert_eq!(take_ready(&pool.shared).unwrap().work(), first);
        let next = take_pending(&pool.shared).unwrap();
        assert_eq!(
            next.cell, high,
            "capacity waits must not freeze a low-priority pending plan"
        );
        let mut state = pool.shared.state.lock().unwrap();
        assert_eq!(state.staging.len(), 2);
        assert!(state.staging.contains(&(second.cell, second.revision)));
        assert!(state.staging.contains(&(next.cell, next.revision)));
        assert!(pop_current_pending(&mut state).is_none());
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn removing_ready_work_releases_source_leases_and_wakes_capacity_waiters() {
        let mut pool = idle_test_pool(1);
        let obsolete = queue_cell(0, 0);
        let next = queue_cell(1, 0);
        pool.sync_queue(&[(obsolete, 3.0), (next, 1.0)]);
        let work = take_pending(&pool.shared).unwrap();
        let lease = Arc::new(());
        let weak = Arc::downgrade(&lease);
        let mut job = prepared_test_job(work);
        let CpuJob::Prepared { acquired, .. } = &mut job else {
            unreachable!()
        };
        acquired.retained.retain(lease);
        assert!(publish_ready(&pool.shared, job));
        assert!(weak.upgrade().is_some());
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = pool.shared.clone();
        let waiter =
            vearth_platform::thread::spawn(move || tx.send(take_pending(&shared)).unwrap());
        pool.sync_queue(&[(next, 1.0)]);
        assert!(
            weak.upgrade().is_none(),
            "removed prepared sources must drop during sync"
        );
        let admitted = rx
            .recv_timeout(vearth_platform::time::Duration::from_secs(1))
            .unwrap()
            .unwrap();
        waiter.join().unwrap();
        assert_eq!(admitted.cell, next);
        let state = pool.shared.state.lock().unwrap();
        assert!(state.ready.is_empty());
        assert_eq!(state.staging.len(), 1);
        assert!(!state.running.contains_key(&obsolete));
    }

    #[test]
    fn invalidation_keeps_physical_reservations_until_their_exact_owner_retires() {
        let mut pool = idle_test_pool(1);
        let cell = queue_cell(0, 0);
        pool.sync_queue(&[(cell, 1.0)]);
        let old = take_pending(&pool.shared).unwrap();
        pool.invalidate_generation();
        pool.sync_queue(&[(cell, 100.0)]);
        {
            let mut state = pool.shared.state.lock().unwrap();
            assert_eq!(
                state.staging.len(),
                1,
                "invalidating an active acquisition cannot free its memory credit"
            );
            assert!(pop_current_pending(&mut state).is_none());
        }
        cancel_running(&pool.shared, old);
        let current = take_pending(&pool.shared).unwrap();
        assert_ne!(old.revision, current.revision);
        assert!(!publish_ready(&pool.shared, prepared_test_job(old)));
        cancel_running(&pool.shared, old);
        {
            let state = pool.shared.state.lock().unwrap();
            assert_eq!(state.running.get(&cell), Some(&current.revision));
            assert_eq!(state.staging.len(), 1);
            assert!(state.staging.contains(&(cell, current.revision)));
        }
        assert!(publish_ready(&pool.shared, prepared_test_job(current)));
        assert_eq!(take_ready(&pool.shared).unwrap().work(), current);
    }

    #[test]
    fn ready_queue_shutdown_wakes_both_worker_classes_and_drops_waiting_payloads() {
        let mut pool = idle_test_pool(1);
        pool.sync_queue(&[(queue_cell(0, 0), 2.0)]);
        let ready = take_pending(&pool.shared).unwrap();
        assert!(publish_ready(&pool.shared, prepared_test_job(ready)));
        pool.shutdown();
        let state = pool.shared.state.lock().unwrap();
        assert!(state.ready.is_empty());
        assert!(state.staging.is_empty());
        drop(state);

        let mut pool = idle_test_pool(1);
        pool.sync_queue(&[(queue_cell(0, 0), 2.0), (queue_cell(1, 0), 1.0)]);
        let acquiring = take_pending(&pool.shared).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let pending_shared = pool.shared.clone();
        let pending_tx = tx.clone();
        let pending_worker = vearth_platform::thread::spawn(move || {
            pending_tx
                .send(take_pending(&pending_shared).is_none())
                .unwrap();
        });
        let ready_shared = pool.shared.clone();
        let ready_worker = vearth_platform::thread::spawn(move || {
            tx.send(take_ready(&ready_shared).is_none()).unwrap();
        });
        pool.shutdown();
        for _ in 0..2 {
            assert!(
                rx.recv_timeout(vearth_platform::time::Duration::from_secs(1))
                    .unwrap()
            );
        }
        pending_worker.join().unwrap();
        ready_worker.join().unwrap();
        assert!(!publish_ready(&pool.shared, prepared_test_job(acquiring)));
        assert!(pool.shared.state.lock().unwrap().staging.is_empty());
    }

    #[test]
    fn geometry_failure_keeps_cpu_worker_alive_for_the_next_cell() {
        let shared = test_pool_shared();
        let bad_cell = queue_cell(0, 0);
        let good_cell = queue_cell(1, 0);
        let (bad, good) = {
            let mut state = shared.state.lock().unwrap();
            sync_pending_state(&mut state, &[(bad_cell, 2.0), (good_cell, 1.0)]);
            (
                pop_current_pending(&mut state).unwrap(),
                pop_current_pending(&mut state).unwrap(),
            )
        };
        let mut malformed = crate::prepared_render::test_surface(bad_cell);
        malformed.solid_collision =
            SurfaceCollision::from_triangles(vec![vearth_gen::collision::CollisionTriangle {
                vertices: [[1.0, 2.0, 3.0]; 3],
            }]);
        let (tx_done, rx_done) = sync_channel(2);
        let worker_shared = shared.clone();
        let worker = vearth_platform::thread::spawn(move || {
            cpu_loop(&worker_shared, &Arc::new(Registry::new()), None, &tx_done);
        });
        assert!(publish_ready(
            &shared,
            CpuJob::Cached {
                column: malformed,
                work: bad,
            }
        ));
        assert!(publish_ready(
            &shared,
            CpuJob::Cached {
                column: crate::prepared_render::test_surface(good_cell),
                work: good,
            }
        ));
        let completed = rx_done
            .recv_timeout(vearth_platform::time::Duration::from_secs(2))
            .expect("the same worker must finish the good cell after rejecting malformed geometry");
        shared.stop.store(true, Ordering::SeqCst);
        shared.ready_cv.notify_all();
        worker.join().unwrap();
        assert_eq!(completed.work, good);
        assert!(completed.prepared.validate().is_ok());
        assert!(rx_done.try_recv().is_err());
        let state = shared.state.lock().unwrap();
        assert!(!state.running.contains_key(&bad_cell));
        assert!(!state.stages.contains_key(&bad_cell));
        assert_eq!(state.failures.get(&bad_cell), Some(&1));
        assert_eq!(state.running.get(&good_cell), Some(&good.revision));
    }

    #[test]
    fn geometry_panic_keeps_generator_alive_and_never_caches_the_failed_cell() {
        let shared = test_pool_shared();
        let bad_cell = queue_cell(0, 0);
        let good_cell = queue_cell(1, 0);
        let (bad, good) = {
            let mut state = shared.state.lock().unwrap();
            sync_pending_state(&mut state, &[(bad_cell, 2.0), (good_cell, 1.0)]);
            (
                pop_current_pending(&mut state).unwrap(),
                pop_current_pending(&mut state).unwrap(),
            )
        };
        let panic_once = std::sync::atomic::AtomicBool::new(true);
        let mut registry = Registry::new();
        registry.add_elevation(
            vearth_data::registry::ProviderInfo {
                name: "panicking generation sample".into(),
                resolution_m: 1.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(vearth_data::FnElevation(move |_: f64, _: f64| {
                assert!(
                    !panic_once.swap(false, Ordering::SeqCst),
                    "cell-local generation failure"
                );
                12.0
            })),
        );
        let dir = std::env::temp_dir().join(format!(
            "vearth-generation-panic-{}-{:?}",
            std::process::id(),
            vearth_platform::thread::current().id()
        ));
        let cache = Arc::new(ChunkCache::with_ram_budget(
            dir.to_str().unwrap(),
            "test",
            1024 * 1024,
        ));
        let (tx_done, rx_done) = sync_channel(2);
        let worker_shared = shared.clone();
        let worker_cache = cache.clone();
        let worker = vearth_platform::thread::spawn(move || {
            cpu_loop(
                &worker_shared,
                &Arc::new(registry),
                Some(&worker_cache),
                &tx_done,
            );
        });
        for work in [bad, good] {
            assert!(publish_ready(
                &shared,
                CpuJob::Prepared {
                    work,
                    acquired: AcquiredSources {
                        #[cfg(feature = "async-providers")]
                        retained: Default::default(),
                        complete: true,
                        features: None,
                    },
                }
            ));
        }
        let completed = rx_done
            .recv_timeout(vearth_platform::time::Duration::from_secs(2))
            .expect("a generator panic must leave the same worker able to generate the next cell");
        shared.stop.store(true, Ordering::SeqCst);
        shared.ready_cv.notify_all();
        worker.join().unwrap();
        assert_eq!(completed.work, good);
        assert!(completed.prepared.validate().is_ok());
        assert!(rx_done.try_recv().is_err());
        cache.flush();
        assert!(cache.load(bad_cell).is_none());
        assert!(cache.load(good_cell).is_some());
        let state = shared.state.lock().unwrap();
        assert!(!state.running.contains_key(&bad_cell));
        assert!(!state.stages.contains_key(&bad_cell));
        assert_eq!(state.failures.get(&bad_cell), Some(&1));
        assert_eq!(state.running.get(&good_cell), Some(&good.revision));
        drop(state);
        drop(cache);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn geometry_panic_retries_are_bounded_without_retiring_newer_or_canceled_intents() {
        let shared = test_pool_shared();
        let cell = queue_cell(0, 0);
        let first = {
            let mut state = shared.state.lock().unwrap();
            sync_pending_state(&mut state, &[(cell, 1.0)]);
            pop_current_pending(&mut state).unwrap()
        };
        assert!(
            run_geometry(&shared, first, "test generation", || {
                panic!("first generation panic")
            })
            .is_none()
        );
        let retry = {
            let mut state = shared.state.lock().unwrap();
            assert_eq!(
                sync_pending_state(&mut state, &[(cell, 2.0)]).pending_len,
                1
            );
            pop_current_pending(&mut state).unwrap()
        };
        assert!(
            run_geometry(&shared, first, "stale generation", || {
                panic!("late panic from the previous revision")
            })
            .is_none()
        );
        {
            let state = shared.state.lock().unwrap();
            assert_eq!(state.failures.get(&cell), Some(&1));
            assert_eq!(state.running.get(&cell), Some(&retry.revision));
        }
        assert!(
            run_geometry(&shared, retry, "test generation", || {
                std::panic::panic_any(String::from("repeated generation panic"))
            })
            .is_none()
        );
        let canceled = {
            let mut state = shared.state.lock().unwrap();
            for priority in 0..100 {
                assert_eq!(
                    sync_pending_state(&mut state, &[(cell, priority as f64)]).pending_len,
                    0
                );
                assert!(pop_current_pending(&mut state).is_none());
            }
            assert!(state.running.is_empty());
            assert!(state.stages.is_empty());
            assert_eq!(state.failures.get(&cell), Some(&MAX_GEOMETRY_ATTEMPTS));
            sync_pending_state(&mut state, &[]);
            sync_pending_state(&mut state, &[(cell, 1.0)]);
            let canceled = pop_current_pending(&mut state).unwrap();
            sync_pending_state(&mut state, &[]);
            canceled
        };
        assert!(
            run_geometry(&shared, canceled, "canceled generation", || {
                std::panic::panic_any(17u32)
            })
            .is_none()
        );
        let state = shared.state.lock().unwrap();
        assert!(state.running.is_empty());
        assert!(state.stages.is_empty());
        assert!(state.failures.is_empty());
    }

    #[test]
    fn geometry_failure_retries_are_bounded_and_revision_scoped() {
        let shared = test_pool_shared();
        let cell = queue_cell(0, 0);
        let first = {
            let mut state = shared.state.lock().unwrap();
            sync_pending_state(&mut state, &[(cell, 1.0)]);
            pop_current_pending(&mut state).unwrap()
        };
        fail_geometry(&shared, first, "invalid surface");
        let retry = {
            let mut state = shared.state.lock().unwrap();
            assert_eq!(
                sync_pending_state(&mut state, &[(cell, 2.0)]).pending_len,
                1
            );
            pop_current_pending(&mut state).unwrap()
        };
        fail_geometry(&shared, first, "late failure from the previous revision");
        {
            let state = shared.state.lock().unwrap();
            assert_eq!(state.failures.get(&cell), Some(&1));
            assert_eq!(state.running.get(&cell), Some(&retry.revision));
        }
        fail_geometry(&shared, retry, "invalid surface again");
        let mut state = shared.state.lock().unwrap();
        for priority in 0..100 {
            assert_eq!(
                sync_pending_state(&mut state, &[(cell, priority as f64)]).pending_len,
                0
            );
            assert!(pop_current_pending(&mut state).is_none());
        }
        assert!(state.running.is_empty());
        assert!(state.stages.is_empty());
        assert_eq!(state.failures.get(&cell), Some(&MAX_GEOMETRY_ATTEMPTS));
        sync_pending_state(&mut state, &[]);
        assert!(state.failures.is_empty());
        assert_eq!(
            sync_pending_state(&mut state, &[(cell, 1.0)]).pending_len,
            1
        );
        assert_ne!(
            pop_current_pending(&mut state).unwrap().revision,
            retry.revision
        );
    }

    #[test]
    fn geometry_failure_never_admits_an_invalid_complete_cell_to_cache() {
        let cell = queue_cell(0, 0);
        let dir = std::env::temp_dir().join(format!(
            "vearth-geometry-rejection-{}-{:?}",
            std::process::id(),
            vearth_platform::thread::current().id()
        ));
        let cache = ChunkCache::with_ram_budget(dir.to_str().unwrap(), "test", 1024 * 1024);
        let registry = Registry::new();
        let valid = crate::prepared_render::test_surface(cell);
        let mut malformed = valid.clone();
        malformed.batches[0].mesh.vertices[0].pos[0] = f32::NAN;
        assert!(prepare_worker_cell(malformed, &registry, Some(&cache), true).is_err());
        assert_eq!(cache.stats().resident_entries, 0);
        assert!(cache.load(cell).is_none());

        let prepared = prepare_worker_cell(valid, &registry, Some(&cache), true).unwrap();
        cache.flush();
        let restored = cache
            .load(cell)
            .expect("validated complete cells remain cacheable");
        assert_eq!(
            pack_surface_cell(restored)
                .unwrap()
                .compute_semantic_fingerprint(),
            prepared.compute_semantic_fingerprint()
        );
        drop(cache);
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(feature = "async-providers")]
    struct PriorityGatedElevation {
        started: std::sync::mpsc::Sender<f64>,
        release: Arc<tokio::sync::Semaphore>,
    }

    #[cfg(feature = "async-providers")]
    impl vearth_data::ElevationSource for PriorityGatedElevation {
        fn elevation_m(&self, _lat: f64, _lon: f64, _sample_pitch_m: f64) -> f32 {
            0.0
        }
    }

    #[cfg(feature = "async-providers")]
    impl vearth_data::async_prep::AsyncSourcePreparer for PriorityGatedElevation {
        fn prepare<'a>(
            &'a self,
            request: &'a vearth_data::async_prep::PrepareRequest,
        ) -> vearth_data::async_prep::PrepareFuture<'a> {
            Box::pin(async move {
                self.started.send(request.priority).unwrap();
                let permit = self.release.acquire().await.unwrap();
                permit.forget();
                let mut summary = vearth_data::async_prep::PrepareSummary::default();
                summary.record(vearth_data::async_prep::DependencyOutcome::Ready);
                summary
            })
        }
    }

    #[cfg(feature = "async-providers")]
    fn priority_gated_registry(
        started: std::sync::mpsc::Sender<f64>,
        release: Arc<tokio::sync::Semaphore>,
    ) -> Registry {
        let mut registry = Registry::new();
        registry.add_elevation_async(
            vearth_data::registry::ProviderInfo {
                name: "priority gate".into(),
                resolution_m: 30.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(PriorityGatedElevation { started, release }),
        );
        registry
    }

    #[test]
    fn queue_diff_preserves_unchanged_revisions_and_tie_order() {
        let cells = [queue_cell(2, 0), queue_cell(0, 0), queue_cell(1, 0)];
        let mut state = PoolState::default();
        let first = cells.map(|cell| (cell, 7.0));
        assert_eq!(
            sync_pending_state(&mut state, &first).pending_len,
            cells.len()
        );
        let revisions = state.pending_set.clone();
        let heap_len = state.pending.len();

        let reversed = [first[2], first[1], first[0]];
        assert_eq!(
            sync_pending_state(&mut state, &reversed).pending_len,
            cells.len()
        );
        assert_eq!(state.pending_set, revisions);
        assert_eq!(state.pending.len(), heap_len);

        let mut popped = Vec::new();
        while let Some(work) = pop_current_pending(&mut state) {
            popped.push(work.cell);
        }
        let mut expected = cells.to_vec();
        expected.sort_unstable();
        assert_eq!(popped, expected, "equal priorities use CellId order");
    }

    #[test]
    fn queue_sync_only_wakes_changed_work_and_only_cancels_on_membership_changes() {
        let a = queue_cell(0, 0);
        let b = queue_cell(1, 0);
        let mut state = PoolState::default();
        assert_eq!(
            sync_pending_state(&mut state, &[(a, 7.0), (b, 3.0)]),
            QueueSync {
                pending_len: 2,
                queued_changes: 2,
                desired_changed: true,
            }
        );
        let revisions = state.pending_set.clone();
        // Duplicate/order variation is common when desired, ladder and
        // destination intents converge on the same cells.
        assert_eq!(
            sync_pending_state(&mut state, &[(b, 1.0), (a, 7.0), (b, 3.0)]),
            QueueSync {
                pending_len: 2,
                queued_changes: 0,
                desired_changed: false,
            }
        );
        assert_eq!(state.pending_set, revisions);
        let active = pop_current_pending(&mut state).unwrap();
        assert_eq!(active.cell, a);
        assert_eq!(
            sync_pending_state(&mut state, &[(a, 9.0), (b, 5.0)]),
            QueueSync {
                pending_len: 1,
                queued_changes: 1,
                desired_changed: false,
            }
        );
        assert_eq!(state.running.get(&a), Some(&active.revision));
        assert_eq!(state.pending_set[&b].1, 3);
        assert_eq!(
            sync_pending_state(&mut state, &[(b, 5.0)]),
            QueueSync {
                pending_len: 1,
                queued_changes: 0,
                desired_changed: true,
            }
        );
        assert!(!transition_stage(&mut state, active, CellStage::Cpu));
        assert!(state.running.is_empty());
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn unchanged_queue_does_not_broadcast_async_cancellation_checks() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let registry = priority_gated_registry(started_tx, release);
        let mut pool = GenPool::new(Arc::new(registry), None, 1);
        let mut changes = pool.shared.desired_version.subscribe();
        let a = queue_cell(0, 0);
        pool.sync_queue(&[(a, 7.0)]);
        started_rx
            .recv_timeout(vearth_platform::time::Duration::from_secs(1))
            .expect("admission");
        assert!(changes.has_changed().unwrap());
        changes.borrow_and_update();
        for _ in 0..10 {
            pool.sync_queue(&[(a, 8.0), (a, 1.0)]);
            assert!(!changes.has_changed().unwrap());
        }
        pool.sync_queue(&[]);
        assert!(
            changes.has_changed().unwrap(),
            "removed active intent must wake cancellation"
        );
    }

    #[test]
    fn queue_diff_uses_max_duplicate_priority_and_skips_stale_revision() {
        let cell = queue_cell(0, 0);
        let mut state = PoolState::default();
        sync_pending_state(&mut state, &[(cell, 1.0), (cell, 5.0)]);
        let first_revision = state.pending_set[&cell].1;
        assert_eq!(state.pending_set[&cell].0, 5.0);

        sync_pending_state(&mut state, &[(cell, 2.0)]);
        let live_revision = state.pending_set[&cell].1;
        assert_ne!(first_revision, live_revision);
        assert_eq!(state.pending.len(), 2, "old revision remains lazy");

        let work = pop_current_pending(&mut state).expect("live queue entry");
        assert_eq!(work.cell, cell);
        assert_eq!(work.revision, live_revision);
        assert!(pop_current_pending(&mut state).is_none());
    }

    #[test]
    fn cache_lookup_admission_preserves_near_job_priority() {
        // Payload kind is discovered only after this shared priority pop, so
        // a queued render hit cannot jump ahead of a nearer column hit or
        // cold miss. Bounded cache workers may finish in either order once
        // admitted, but they cannot starve the live high-priority revision.
        let far_render_hit = queue_cell(0, 0);
        let near_cold_miss = queue_cell(1, 0);
        let mut state = PoolState::default();
        sync_pending_state(
            &mut state,
            &[(far_render_hit, 1.0), (near_cold_miss, 100.0)],
        );

        assert_eq!(
            pop_current_pending(&mut state).unwrap().cell,
            near_cold_miss
        );
        assert_eq!(
            pop_current_pending(&mut state).unwrap().cell,
            far_render_hit
        );
    }

    #[test]
    fn stale_stage_token_cannot_cancel_newer_run() {
        let cell = queue_cell(0, 0);
        let stale = WorkItem {
            cell,
            revision: 3,
            priority: 1.0,
        };
        let current = WorkItem {
            cell,
            revision: 4,
            priority: 2.0,
        };
        let mut state = PoolState::default();
        state.desired.insert(cell);
        state.running.insert(cell, current.revision);
        state.stages.insert(cell, CellStage::Acquiring);

        assert!(!transition_stage(&mut state, stale, CellStage::Cpu));
        assert_eq!(state.running.get(&cell), Some(&current.revision));
        assert_eq!(state.stages.get(&cell), Some(&CellStage::Acquiring));

        state.desired.clear();
        assert!(!transition_stage(&mut state, current, CellStage::Cpu));
        assert!(!state.running.contains_key(&cell));
        assert!(!state.stages.contains_key(&cell));
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn async_admission_late_high_priority_overtakes_queued_low_priority() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let registry = priority_gated_registry(started_tx, release.clone());
        let mut pool = GenPool::new(Arc::new(registry), None, 1);
        let active = queue_cell(0, 0);
        let low = queue_cell(1, 0);
        let high = queue_cell(2, 0);

        pool.sync_queue(&[(active, 10.0), (low, 1.0)]);
        assert_eq!(
            started_rx
                .recv_timeout(vearth_platform::time::Duration::from_secs(1))
                .expect("first acquisition started"),
            10.0
        );
        assert_eq!(pool.counts(), (1, 1));

        // This intent arrives after `low` is already queued. It must remain
        // in the replaceable heap until the sole admission slot opens.
        pool.sync_queue(&[(active, 10.0), (low, 1.0), (high, 100.0)]);
        assert_eq!(pool.counts(), (2, 1));
        release.add_permits(1);
        assert_eq!(
            started_rx
                .recv_timeout(vearth_platform::time::Duration::from_secs(1))
                .expect("next priority admitted"),
            100.0,
            "the late high-priority cell must overtake the queued low cell"
        );

        // Cancel the gated high-priority request and every still-queued
        // revision so teardown never needs to release another test permit.
        pool.sync_queue(&[]);
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn ready_publication_releases_async_permits_without_unbounded_preparation() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let registry = Arc::new(priority_gated_registry(started_tx, release.clone()));
        let mut pool = idle_test_pool(2);
        let runtime = vearth_platform::rt::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_time()
            .build()
            .unwrap();
        let shared = pool.shared.clone();
        let handle = runtime.handle().clone();
        let dispatcher = vearth_platform::thread::spawn(move || {
            async_acquisition_dispatch_loop(shared, registry, None, handle, 1);
        });
        let cells = [queue_cell(0, 0), queue_cell(1, 0), queue_cell(2, 0)];
        pool.sync_queue(&[(cells[0], 30.0), (cells[1], 20.0), (cells[2], 1.0)]);
        assert_eq!(
            started_rx
                .recv_timeout(vearth_platform::time::Duration::from_secs(1))
                .unwrap(),
            30.0
        );
        release.add_permits(1);
        assert_eq!(
            started_rx
                .recv_timeout(vearth_platform::time::Duration::from_secs(1))
                .unwrap(),
            20.0
        );
        assert_eq!(
            pool.shared.state.lock().unwrap().ready.len(),
            1,
            "completed acquisition must release its permit while still waiting for CPU"
        );
        release.add_permits(1);
        let deadline =
            vearth_platform::time::Instant::now() + vearth_platform::time::Duration::from_secs(1);
        while pool.shared.state.lock().unwrap().ready.len() != 2 {
            assert!(vearth_platform::time::Instant::now() < deadline);
            vearth_platform::thread::yield_now();
        }
        assert!(
            started_rx.try_recv().is_err(),
            "full staging must stop further acquisition"
        );
        assert_eq!(pool.counts(), (1, 2));
        let high = queue_cell(3, 0);
        pool.sync_queue(&[
            (cells[0], 30.0),
            (cells[1], 20.0),
            (cells[2], 1.0),
            (high, 100.0),
        ]);
        assert_eq!(take_ready(&pool.shared).unwrap().work().cell, cells[0]);
        assert_eq!(
            started_rx
                .recv_timeout(vearth_platform::time::Duration::from_secs(1))
                .unwrap(),
            100.0
        );
        pool.shutdown();
        dispatcher.join().unwrap();
        drop(runtime);
        assert!(pool.shared.state.lock().unwrap().staging.is_empty());
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn async_priority_admission_never_exceeds_cell_limit() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let registry = priority_gated_registry(started_tx, release);
        let mut pool = GenPool::new(Arc::new(registry), None, 2);
        let jobs: Vec<_> = (0..8)
            .map(|i| (queue_cell(i, 0), f64::from(i + 1)))
            .collect();

        pool.sync_queue(&jobs);
        let mut admitted = [
            started_rx
                .recv_timeout(vearth_platform::time::Duration::from_secs(1))
                .expect("first admission"),
            started_rx
                .recv_timeout(vearth_platform::time::Duration::from_secs(1))
                .expect("second admission"),
        ];
        admitted.sort_by(f64::total_cmp);
        assert_eq!(admitted, [7.0, 8.0]);
        assert!(
            started_rx
                .recv_timeout(vearth_platform::time::Duration::from_millis(100))
                .is_err(),
            "a third cell escaped the two-cell admission bound"
        );
        assert_eq!(pool.counts(), (6, 2));
        assert_eq!(
            pool.stage_counts(),
            PoolStageCounts {
                cache_check: 0,
                acquiring: 2,
                cpu: 0,
            }
        );

        pool.sync_queue(&[]);
        let deadline =
            vearth_platform::time::Instant::now() + vearth_platform::time::Duration::from_secs(1);
        while pool.counts().1 != 0 {
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "canceling bounded admissions stranded a running revision"
            );
            vearth_platform::thread::yield_now();
        }
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn staged_pipeline_moves_one_cell_through_bounded_lanes() {
        let registry = Arc::new(Registry::new());
        let mut pool = GenPool::new(registry, None, 2);
        assert!(pool.staged_pipeline());
        let cell = CellId {
            face: Face(0),
            level: MIN_RENDER_LEVEL,
            i: 0,
            j: 0,
        };
        pool.sync_queue(&[(cell, 1.0)]);
        let deadline =
            vearth_platform::time::Instant::now() + vearth_platform::time::Duration::from_secs(2);
        let result = loop {
            if let Some(result) = pool.drain_done().pop() {
                break result;
            }
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "staged pipeline stalled"
            );
            vearth_platform::thread::sleep(vearth_platform::time::Duration::from_millis(1));
        };
        assert_eq!(result.cell(), cell);
        assert_eq!(pool.counts(), (0, 0));
        assert_eq!(pool.stage_counts(), PoolStageCounts::default());
    }

    #[test]
    fn column_hit_precedes_generation_and_duplicate_intents_build_once() {
        use vearth_data::FnElevation;

        let dir =
            std::env::temp_dir().join(format!("vearth-stream-column-cache-{}", std::process::id()));
        let cell = CellId {
            face: Face(0),
            level: MIN_RENDER_LEVEL,
            i: 0,
            j: 0,
        };
        let elevation = FnElevation(|_: f64, _: f64| 80.0);
        let column = generate_surface_cell(cell, &Sources::terrain_only(&elevation));
        let cache = Arc::new(ChunkCache::with_ram_budget(
            dir.to_str().unwrap(),
            "stream-test",
            32 * 1024 * 1024,
        ));
        cache.store(&column);
        cache.flush();

        let registry = Arc::new(Registry::new());
        let mut pool = GenPool::new(registry, Some(cache.clone()), 2);
        pool.sync_queue(&[(cell, 1.0), (cell, 9.0)]);
        // A worker may take the queued key before this thread observes it.
        assert_eq!(
            pool.in_flight_count(),
            1,
            "duplicate intent has one live key"
        );
        let deadline =
            vearth_platform::time::Instant::now() + vearth_platform::time::Duration::from_secs(2);
        let result = loop {
            if let Some(result) = pool.drain_done_one() {
                break result;
            }
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "column-cache completion stalled"
            );
            vearth_platform::thread::sleep(vearth_platform::time::Duration::from_millis(1));
        };
        assert_eq!(result.cell(), cell);
        assert_eq!(result.cell(), cell);
        assert_eq!(pool.stats(), (1, 1));
        assert!(pool.drain_done().is_empty());
        let stats = cache.stats();
        assert_eq!(stats.ram_hits, 1);
        drop(pool);
        drop(cache);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn incremental_completion_drain_leaves_other_meshes_in_flight() {
        let registry = Arc::new(Registry::new());
        let mut pool = GenPool::new(registry, None, 2);
        let cells = [queue_cell(0, 0), queue_cell(0, 1), queue_cell(1, 0)];
        let jobs: Vec<_> = cells
            .iter()
            .enumerate()
            .map(|(index, &cell)| (cell, index as f64 + 1.0))
            .collect();
        pool.sync_queue(&jobs);

        let deadline =
            vearth_platform::time::Instant::now() + vearth_platform::time::Duration::from_secs(2);
        while pool.counts() != (0, cells.len()) {
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "generation jobs were not all admitted"
            );
            vearth_platform::thread::sleep(vearth_platform::time::Duration::from_millis(1));
        }

        let first = loop {
            if let Some(result) = pool.drain_done_one() {
                break result;
            }
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "first incremental completion never arrived"
            );
            vearth_platform::thread::sleep(vearth_platform::time::Duration::from_millis(1));
        };
        assert!(cells.contains(&first.cell()));
        assert_eq!(
            pool.counts(),
            (0, cells.len() - 1),
            "one drain must retire exactly one running revision"
        );

        let mut completed = vec![first.cell()];
        while completed.len() < cells.len() {
            completed.extend(pool.drain_done().into_iter().map(|mesh| mesh.cell()));
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "remaining incremental completions never arrived"
            );
            vearth_platform::thread::sleep(vearth_platform::time::Duration::from_millis(1));
        }
        completed.sort();
        let mut expected = cells.to_vec();
        expected.sort();
        assert_eq!(completed, expected);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn polar_cell_preparation_keeps_vector_window_bilinear_dependencies() {
        use vearth_data::terrarium::{TerrariumSource, sampling_tiles};

        #[derive(Default)]
        struct RecordingFetcher(std::sync::Mutex<HashSet<(u8, u32, u32)>>);

        impl vearth_data::async_http::AsyncTileFetcher for RecordingFetcher {
            fn fetch<'a>(&'a self, url: &'a str) -> vearth_data::async_http::FetchFuture<'a> {
                Box::pin(async move {
                    let path = url.strip_prefix("http://polar/").unwrap();
                    let mut components = path.trim_end_matches(".webp").split('/');
                    let key = (
                        components.next().unwrap().parse().unwrap(),
                        components.next().unwrap().parse().unwrap(),
                        components.next().unwrap().parse().unwrap(),
                    );
                    assert!(components.next().is_none());
                    self.0.lock().unwrap().insert(key);
                    Err(vearth_data::DataError::NotAvailable)
                })
            }
        }

        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let mut nonlegacy = HashSet::new();
        for latitude in [-89.0_f64, 89.0] {
            for longitude in [5.0_f64, 179.0] {
                for level in [18, 19] {
                    let direction = vearth_core::ellipsoid::geodetic_to_ecef(
                        latitude.to_radians(),
                        longitude.to_radians(),
                        0.0,
                    )
                    .normalize();
                    let cell = CellId::from_dir(direction, level);
                    let plan = CellPlan::for_cell(cell, false, 731.25);
                    let fetcher = Arc::new(RecordingFetcher::default());
                    let mut registry = Registry::new();
                    registry.add_elevation_async(
                        vearth_data::registry::ProviderInfo {
                            name: "polar terrarium".into(),
                            resolution_m: 30.0,
                            coverage: vearth_data::registry::Coverage::Global,
                            attribution: String::new(),
                            license: String::new(),
                            url: String::new(),
                        },
                        Arc::new(
                            TerrariumSource::new_async(fetcher.clone(), "http://polar", None)
                                .with_coverage(None),
                        ),
                    );
                    let acquired = runtime.block_on(plan.acquire_async(&registry));
                    assert!(acquired.features.is_none());
                    let requested = fetcher.0.lock().unwrap();
                    assert!(requested.len() <= 1024, "polar fixture must stay bounded");
                    let available: HashSet<_> = requested
                        .iter()
                        .copied()
                        .chain(plan.terrain_tiles.iter().copied())
                        .collect();
                    let legacy: HashSet<_> = plan.terrain_tiles.iter().copied().collect();
                    let (lat0, lon0, half_e, half_n) = vearth_gen::vector_query_bounds(cell);
                    let radius = vearth_core::ellipsoid::MEAN_RADIUS;
                    for step in 0..=8 {
                        let t = step as f64 / 4.0 - 1.0;
                        for (east, north) in [
                            (-half_e, t * half_n),
                            (half_e, t * half_n),
                            (t * half_e, -half_n),
                            (t * half_e, half_n),
                        ] {
                            let lat = lat0 + north / radius;
                            let lon = lon0 + east / (radius * lat0.cos());
                            let zoom = TerrariumSource::zoom_for(plan.footprint.sample_hint_m, lat);
                            for key in sampling_tiles(lat, lon, zoom) {
                                assert!(available.contains(&key), "polar {cell:?} missed {key:?}");
                                if !legacy.contains(&key) {
                                    nonlegacy.insert(key);
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            !nonlegacy.is_empty(),
            "fixture must exercise non-lattice samples"
        );
    }

    #[cfg(feature = "async-providers")]
    mod geometry_acquisition {
        use super::*;
        use std::future::Future as _;
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        use std::task::{Context, Poll, Waker};
        use vearth_data::async_prep::{
            AsyncSourcePreparer, DependencyOutcome, PreparationCoverage, PrepareFuture,
            PrepareRequest, PrepareSummary, PreparedSources, RetainedPrepareFuture,
        };

        struct Lease(Arc<AtomicUsize>);

        impl Drop for Lease {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::Relaxed);
            }
        }

        struct GatedSource {
            outcome: DependencyOutcome,
            release: tokio::sync::Semaphore,
            initial_release: tokio::sync::Semaphore,
            initial_failure: AtomicBool,
            initial_completed: AtomicBool,
            projection_complete: AtomicBool,
            projections: AtomicUsize,
            requests: std::sync::Mutex<Vec<PrepareRequest>>,
            leases: Arc<AtomicUsize>,
        }

        impl vearth_data::ElevationSource for GatedSource {
            fn elevation_m(&self, _: f64, _: f64, _: f64) -> f32 {
                0.0
            }
        }

        impl vearth_data::vectors::VectorSource for GatedSource {
            fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
                assert!(self.initial_completed.load(Ordering::Relaxed));
                self.projections.fetch_add(1, Ordering::Relaxed);
                let mut pack = building_pack();
                pack.complete = self.projection_complete.load(Ordering::Relaxed);
                pack
            }
        }

        impl AsyncSourcePreparer for GatedSource {
            fn prepare<'a>(&'a self, request: &'a PrepareRequest) -> PrepareFuture<'a> {
                Box::pin(async move {
                    self.prepare_retained_for_coverage(request, PreparationCoverage::Global)
                        .await
                        .summary
                })
            }

            fn prepare_retained_for_coverage<'a>(
                &'a self,
                request: &'a PrepareRequest,
                _: PreparationCoverage,
            ) -> RetainedPrepareFuture<'a> {
                Box::pin(async move {
                    self.requests.lock().unwrap().push(request.clone());
                    self.leases.fetch_add(1, Ordering::Relaxed);
                    let lease = Arc::new(Lease(self.leases.clone()));
                    let outcome = if request.vector_bounds.is_some() {
                        self.initial_release.acquire().await.unwrap().forget();
                        self.initial_completed.store(true, Ordering::Relaxed);
                        if self.initial_failure.load(Ordering::Relaxed) {
                            DependencyOutcome::TransientFailure
                        } else {
                            DependencyOutcome::Ready
                        }
                    } else {
                        self.release.acquire().await.unwrap().forget();
                        self.outcome
                    };
                    let mut prepared = PreparedSources::default();
                    prepared.summary.record(outcome);
                    prepared.retain(lease);
                    prepared
                })
            }
        }

        struct BuildingVectors;

        impl vearth_data::vectors::VectorSource for BuildingVectors {
            fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
                building_pack()
            }
        }

        fn building_pack() -> FeaturePack {
            let mut pack = FeaturePack::empty();
            pack.buildings.push(vearth_data::vectors::Building {
                location_rad: None,
                roof_shape: None,
                use_kind: Default::default(),
                rings: vec![vec![
                    [-20.0, -20.0],
                    [20.0, -20.0],
                    [20.0, 20.0],
                    [-20.0, 20.0],
                ]],
                height_m: 24.0,
                min_height_m: 0.0,
                seed: 41,
                kind: vearth_data::vectors::BuildingKind::Building,
            });
            pack
        }

        fn gated_source(outcome: DependencyOutcome) -> Arc<GatedSource> {
            Arc::new(GatedSource {
                outcome,
                release: tokio::sync::Semaphore::new(0),
                initial_release: tokio::sync::Semaphore::new(1),
                initial_failure: AtomicBool::new(false),
                initial_completed: AtomicBool::new(false),
                projection_complete: AtomicBool::new(true),
                projections: AtomicUsize::new(0),
                requests: Default::default(),
                leases: Default::default(),
            })
        }

        fn fixture(outcomes: [DependencyOutcome; 2]) -> (Registry, [Arc<GatedSource>; 2]) {
            let sources = outcomes.map(gated_source);
            let info = vearth_data::registry::ProviderInfo {
                name: "gated datum".into(),
                resolution_m: 1.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            };
            let mut registry = Registry::new();
            registry.add_elevation_async(info.clone(), sources[0].clone());
            registry.add_surface_async(info.clone(), sources[1].clone());
            registry.add_vectors(info, Arc::new(BuildingVectors));
            (registry, sources)
        }

        fn plan(level: u8) -> CellPlan {
            CellPlan::for_cell(
                CellId {
                    face: Face(0),
                    level,
                    i: 1 << (level - 1),
                    j: 1 << (level - 1),
                },
                true,
                12345.125,
            )
        }

        #[test]
        fn datums_start_together_and_keep_exact_requests_and_stage_outcomes() {
            use DependencyOutcome::*;
            for (case, outcomes) in [
                [Ready, Ready],
                [TransientFailure, Ready],
                [Ready, CircuitOpenCacheMiss],
                [NoData, NoData],
            ]
            .into_iter()
            .enumerate()
            {
                let (registry, sources) = fixture(outcomes);
                let plan = plan(16);
                let pack = building_pack();
                let geometry =
                    vearth_gen::built_surface::geometry_source_footprints(plan.cell, &pack);
                assert_eq!(geometry.len(), 1, "a centred building needs one datum cap");
                let footprints = [
                    geometry.first().copied(),
                    vearth_gen::built_surface::roof_source_footprint(plan.cell, &pack),
                ];
                let mut future = Box::pin(plan.acquire_geometry_async(&registry, &pack));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(future.as_mut().poll(&mut cx).is_pending());
                for (source, footprint) in sources.iter().zip(footprints) {
                    let footprint = footprint.expect("building needs both datum sources");
                    assert_eq!(
                        *source.requests.lock().unwrap(),
                        [PrepareRequest {
                            terrain_tiles: Vec::new(),
                            footprint: vearth_data::async_prep::SourceFootprint {
                                center_lat: footprint.center_lat,
                                center_lon: footprint.center_lon,
                                radius_rad: footprint.radius_rad,
                                sample_hint_m: footprint.sample_hint_m,
                            },
                            vector_bounds: None,
                            night_building_bounds: None,
                            priority: plan.priority,
                        }],
                        "both exact requests must start before either datum is released"
                    );
                }
                let first = case % 2;
                sources[first].release.add_permits(1);
                assert!(future.as_mut().poll(&mut cx).is_pending());
                for source in &sources {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 1);
                }
                sources[1 - first].release.add_permits(1);
                let Poll::Ready((prepared, complete)) = future.as_mut().poll(&mut cx) else {
                    panic!("both released datums must finish");
                };
                drop(future);
                let summaries = outcomes.map(|outcome| {
                    let mut summary = PrepareSummary::default();
                    summary.record(outcome);
                    summary
                });
                assert_eq!(complete, summaries.map(PrepareSummary::complete));
                let mut expected = summaries[0];
                expected.combine(summaries[1]);
                assert_eq!(prepared.summary, expected);
                for source in &sources {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 1);
                }
                drop(prepared);
                for source in &sources {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 0);
                }
            }
        }

        #[test]
        fn canceling_join_releases_pending_and_already_completed_datum_leases() {
            for released in [None, Some(0), Some(1)] {
                let (registry, sources) = fixture([DependencyOutcome::Ready; 2]);
                let plan = plan(16);
                let pack = building_pack();
                let mut future = Box::pin(plan.acquire_geometry_async(&registry, &pack));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(future.as_mut().poll(&mut cx).is_pending());
                if let Some(index) = released {
                    sources[index].release.add_permits(1);
                    assert!(future.as_mut().poll(&mut cx).is_pending());
                }
                for source in &sources {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 1);
                }
                drop(future);
                for source in &sources {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 0);
                }
            }
        }

        #[test]
        fn absent_datums_stay_complete_without_extra_source_requests() {
            for (level, pack, expected_requests) in [
                (16, FeaturePack::empty(), [0, 0]),
                (15, building_pack(), [1, 0]),
            ] {
                let (registry, sources) = fixture([DependencyOutcome::Ready; 2]);
                let plan = plan(level);
                for source in &sources {
                    source.release.add_permits(1);
                }
                let (prepared, complete) =
                    pollster::block_on(plan.acquire_geometry_async(&registry, &pack));
                assert_eq!(complete, [true; 2]);
                assert!(prepared.summary.complete());
                for (source, expected) in sources.iter().zip(expected_requests) {
                    assert_eq!(source.requests.lock().unwrap().len(), expected);
                }
            }
        }

        #[test]
        fn cell_acquisition_keeps_initial_and_both_datum_leases_until_handoff() {
            for outcomes in [
                [DependencyOutcome::Ready; 2],
                [
                    DependencyOutcome::Ready,
                    DependencyOutcome::TransientFailure,
                ],
            ] {
                let (registry, sources) = fixture(outcomes);
                let plan = plan(16);
                let mut future = Box::pin(plan.acquire_async(&registry));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(future.as_mut().poll(&mut cx).is_pending());
                for source in &sources {
                    let requests = source.requests.lock().unwrap();
                    assert_eq!(requests.len(), 2);
                    assert_eq!(requests[0].terrain_tiles, plan.terrain_tiles);
                    assert_eq!(requests[0].vector_bounds, plan.vector_bounds);
                    assert_eq!(requests[0].priority, plan.priority);
                    assert!(requests[1].terrain_tiles.is_empty());
                    assert_eq!(source.leases.load(Ordering::Relaxed), 2);
                    source.release.add_permits(1);
                }
                let Poll::Ready(acquired) = future.as_mut().poll(&mut cx) else {
                    panic!("released cell acquisition must finish");
                };
                drop(future);
                assert_eq!(acquired.complete, outcomes == [DependencyOutcome::Ready; 2]);
                assert_eq!(acquired.retained.summary.complete(), acquired.complete);
                assert_eq!(acquired.features.as_ref().unwrap().buildings.len(), 1);
                for source in &sources {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 2);
                }
                drop(acquired);
                for source in &sources {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 0);
                }
            }
        }

        fn overlap_fixture() -> (Registry, [Arc<GatedSource>; 3]) {
            let (mut registry, [elevation, surface]) = fixture([DependencyOutcome::Ready; 2]);
            let vector = gated_source(DependencyOutcome::Ready);
            registry.add_vectors_async(
                vearth_data::registry::ProviderInfo {
                    name: "gated projected vectors".into(),
                    resolution_m: 0.5,
                    coverage: vearth_data::registry::Coverage::Global,
                    attribution: String::new(),
                    license: String::new(),
                    url: String::new(),
                },
                vector.clone(),
            );
            let sources = [elevation, surface, vector];
            for source in &sources {
                source.initial_release.try_acquire().unwrap().forget();
            }
            (registry, sources)
        }

        #[test]
        fn vector_datums_progress_before_initial_rasters_and_preserve_failures() {
            for (failed_initial, projection_complete) in [
                (None, true),
                (Some(0usize), true),
                (Some(1), true),
                (Some(2), true),
                (None, false),
            ] {
                let (registry, sources) = overlap_fixture();
                if let Some(index) = failed_initial {
                    sources[index]
                        .initial_failure
                        .store(true, Ordering::Relaxed);
                }
                sources[2]
                    .projection_complete
                    .store(projection_complete, Ordering::Relaxed);
                let plan = plan(16);
                let mut future = Box::pin(plan.acquire_async(&registry));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(future.as_mut().poll(&mut cx).is_pending());
                assert_eq!(sources[2].projections.load(Ordering::Relaxed), 0);
                for source in &sources {
                    assert_eq!(source.requests.lock().unwrap().len(), 1);
                }

                sources[2].initial_release.add_permits(1);
                assert!(future.as_mut().poll(&mut cx).is_pending());
                assert_eq!(sources[2].projections.load(Ordering::Relaxed), 1);
                for source in &sources[..2] {
                    assert!(!source.initial_completed.load(Ordering::Relaxed));
                    let requests = source.requests.lock().unwrap();
                    assert_eq!(
                        requests.len(),
                        2,
                        "datums must start while base rasters wait"
                    );
                    assert_eq!(requests[0].terrain_tiles, plan.terrain_tiles);
                    assert_eq!(requests[0].vector_bounds, plan.vector_bounds);
                    assert!(requests[1].terrain_tiles.is_empty());
                    assert_eq!(requests[1].vector_bounds, None);
                    assert!(
                        requests
                            .iter()
                            .all(|request| request.priority == plan.priority)
                    );
                    source.release.add_permits(1);
                }
                assert!(future.as_mut().poll(&mut cx).is_pending());
                for source in &sources[..2] {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 2);
                    source.initial_release.add_permits(1);
                }
                assert_eq!(sources[2].leases.load(Ordering::Relaxed), 1);
                let Poll::Ready(acquired) = future.as_mut().poll(&mut cx) else {
                    panic!("initial rasters must complete the joined cell preparation");
                };
                drop(future);
                assert_eq!(
                    acquired.complete,
                    failed_initial.is_none() && projection_complete
                );
                assert_eq!(
                    acquired.retained.summary.complete(),
                    failed_initial.is_none()
                );
                assert_eq!(acquired.features.is_some(), projection_complete);
                for (source, expected) in sources.iter().zip([2, 2, 1]) {
                    assert_eq!(source.leases.load(Ordering::Relaxed), expected);
                    assert_eq!(source.requests.lock().unwrap().len(), expected);
                }
                drop(acquired);
                for source in &sources {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 0);
                }
            }
        }

        #[test]
        fn canceling_overlapped_acquisition_releases_every_branch_lease() {
            // Cancel before vector readiness, after raster readiness, during
            // datum preparation, and after datums finish ahead of rasters.
            for progress in 0..4 {
                let (registry, sources) = overlap_fixture();
                let plan = plan(16);
                let mut future = Box::pin(plan.acquire_async(&registry));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(future.as_mut().poll(&mut cx).is_pending());
                if progress == 1 {
                    for source in &sources[..2] {
                        source.initial_release.add_permits(1);
                    }
                    assert!(future.as_mut().poll(&mut cx).is_pending());
                } else if progress >= 2 {
                    sources[2].initial_release.add_permits(1);
                    assert!(future.as_mut().poll(&mut cx).is_pending());
                    if progress == 3 {
                        for source in &sources[..2] {
                            source.release.add_permits(1);
                        }
                        assert!(future.as_mut().poll(&mut cx).is_pending());
                    }
                }
                for (index, source) in sources.iter().enumerate() {
                    assert_eq!(
                        source.leases.load(Ordering::Relaxed),
                        if progress >= 2 && index < 2 { 2 } else { 1 },
                    );
                }
                drop(future);
                for source in &sources {
                    assert_eq!(source.leases.load(Ordering::Relaxed), 0);
                }
            }
        }
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn async_acquisition_completeness_is_local_to_its_dependencies() {
        struct Source {
            global_errors: std::sync::atomic::AtomicU64,
        }
        impl vearth_data::ElevationSource for Source {
            fn elevation_m(&self, _: f64, _: f64, _: f64) -> f32 {
                0.0
            }
            fn fetch_errors(&self) -> u64 {
                self.global_errors.load(Ordering::Relaxed)
            }
        }
        impl vearth_data::async_prep::AsyncSourcePreparer for Source {
            fn prepare<'a>(
                &'a self,
                _: &'a vearth_data::async_prep::PrepareRequest,
            ) -> vearth_data::async_prep::PrepareFuture<'a> {
                Box::pin(async move {
                    // Another request changes the provider's aggregate health
                    // while every dependency of this request succeeds.
                    self.global_errors.fetch_add(1, Ordering::Relaxed);
                    let mut summary = vearth_data::async_prep::PrepareSummary::default();
                    summary.record(vearth_data::async_prep::DependencyOutcome::Ready);
                    summary
                })
            }
        }
        let mut registry = Registry::new();
        registry.add_elevation_async(
            vearth_data::registry::ProviderInfo {
                name: "independent requests".into(),
                resolution_m: 1.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(Source {
                global_errors: std::sync::atomic::AtomicU64::new(0),
            }),
        );
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let plan = CellPlan::for_cell(queue_cell(0, 0), false, 1.0);
        assert!(runtime.block_on(plan.acquire_async(&registry)).complete);
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn acquired_rasters_remain_owned_until_cpu_sampling_finishes() {
        use vearth_data::async_prep::{DependencyOutcome, PreparedSources};
        struct Lease(Arc<std::sync::atomic::AtomicUsize>);
        impl Drop for Lease {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::Relaxed);
            }
        }
        struct Source {
            leases: Arc<std::sync::atomic::AtomicUsize>,
            samples: std::sync::atomic::AtomicUsize,
        }
        impl vearth_data::ElevationSource for Source {
            fn elevation_m(&self, _: f64, _: f64, _: f64) -> f32 {
                assert!(
                    self.leases.load(Ordering::Relaxed) > 0,
                    "acquired raster was released before CPU sampling"
                );
                self.samples.fetch_add(1, Ordering::Relaxed);
                100.0
            }
        }
        impl vearth_data::async_prep::AsyncSourcePreparer for Source {
            fn prepare<'a>(
                &'a self,
                _: &'a vearth_data::async_prep::PrepareRequest,
            ) -> vearth_data::async_prep::PrepareFuture<'a> {
                Box::pin(async {
                    let mut summary = Default::default();
                    vearth_data::async_prep::PrepareSummary::record(
                        &mut summary,
                        DependencyOutcome::Ready,
                    );
                    summary
                })
            }
            fn prepare_retained_for_coverage<'a>(
                &'a self,
                _: &'a vearth_data::async_prep::PrepareRequest,
                _: vearth_data::async_prep::PreparationCoverage,
            ) -> vearth_data::async_prep::RetainedPrepareFuture<'a> {
                Box::pin(async move {
                    self.leases.fetch_add(1, Ordering::Relaxed);
                    let mut acquired = PreparedSources::default();
                    acquired.summary.record(DependencyOutcome::Ready);
                    acquired.retain(Arc::new(Lease(self.leases.clone())));
                    acquired
                })
            }
        }
        let source = Arc::new(Source {
            leases: Default::default(),
            samples: Default::default(),
        });
        let mut registry = Registry::new();
        registry.add_elevation_async(
            vearth_data::registry::ProviderInfo {
                name: "retained raster".into(),
                resolution_m: 1.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            source.clone(),
        );
        let registry = Arc::new(registry);
        let cell = queue_cell(0, 0);
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let plan = CellPlan::for_cell(cell, false, 1.0);
        let acquired = runtime.block_on(plan.acquire_async(&registry));
        assert!(acquired.complete);
        assert_eq!(source.leases.load(Ordering::Relaxed), 1);
        assert_eq!(source.samples.load(Ordering::Relaxed), 0);
        generate_prepared(cell, &registry, None, acquired).unwrap();
        assert!(source.samples.load(Ordering::Relaxed) > 0);
        assert_eq!(source.leases.load(Ordering::Relaxed), 0);
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn acquired_vectors_reuse_complete_windows_and_requery_recovered_geometry() {
        struct CountedVectors {
            queries: std::sync::Mutex<Vec<(f64, f64, f64, f64)>>,
            complete: bool,
        }
        impl vearth_data::vectors::VectorSource for CountedVectors {
            fn features(&self, lat: f64, lon: f64, half_e: f64, half_n: f64) -> FeaturePack {
                let mut queries = self.queries.lock().unwrap();
                queries.push((lat, lon, half_e, half_n));
                let mut pack = FeaturePack::empty();
                pack.complete = self.complete || queries.len() > 1;
                if pack.complete {
                    pack.buildings.push(vearth_data::vectors::Building {
                        location_rad: None,
                        roof_shape: None,
                        use_kind: Default::default(),
                        rings: vec![vec![
                            [-20.0, -20.0],
                            [20.0, -20.0],
                            [20.0, 20.0],
                            [-20.0, 20.0],
                        ]],
                        height_m: 24.0,
                        min_height_m: 0.0,
                        seed: 41,
                        kind: vearth_data::vectors::BuildingKind::Building,
                    });
                }
                pack
            }
        }
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let cell = CellId {
            face: Face(0),
            level: 14,
            i: 8192,
            j: 8192,
        };
        for complete in [true, false] {
            let vectors = Arc::new(CountedVectors {
                queries: Default::default(),
                complete,
            });
            let mut registry = Registry::new();
            registry.add_vectors(
                vearth_data::registry::ProviderInfo {
                    name: "counted projected source".into(),
                    resolution_m: 1.0,
                    coverage: vearth_data::registry::Coverage::Global,
                    attribution: String::new(),
                    license: String::new(),
                    url: String::new(),
                },
                vectors.clone(),
            );
            let registry = Arc::new(registry);
            let plan = CellPlan::for_cell(cell, true, 1.0);
            let acquired = runtime.block_on(plan.acquire_async(&registry));
            assert_eq!(acquired.complete, complete);
            assert_eq!(acquired.features.is_some(), complete);
            let dir = std::env::temp_dir().join(format!(
                "vearth-acquired-vectors-{}-{complete}",
                std::process::id()
            ));
            std::fs::remove_dir_all(&dir).ok();
            let cache = ChunkCache::with_ram_budget(&dir, "acquired-vectors", 1024 * 1024);
            let prepared = generate_prepared(cell, &registry, Some(&cache), acquired).unwrap();
            assert!(prepared.complete);
            assert!(
                prepared.height_range.1 >= 24.0,
                "recovered building geometry must be rendered"
            );
            let expected_queries = if complete { 1 } else { 2 };
            assert_eq!(
                *vectors.queries.lock().unwrap(),
                vec![vearth_gen::vector_query_bounds(cell); expected_queries]
            );
            cache.flush();
            assert_eq!(
                cache.load(cell).is_some(),
                complete,
                "acquisition failures must still prevent persistence"
            );
            let reference = generate_prepared(
                cell,
                &registry,
                None,
                AcquiredSources {
                    #[cfg(feature = "async-providers")]
                    retained: Default::default(),
                    complete,
                    features: None,
                },
            )
            .unwrap();
            assert_eq!(
                prepared.compute_semantic_fingerprint(),
                reference.compute_semantic_fingerprint()
            );
            assert_eq!(vectors.queries.lock().unwrap().len(), expected_queries + 1);
            drop(cache);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn async_preparer_runs_before_synchronous_generation() {
        struct PreparedElevation {
            prepared: Arc<std::sync::atomic::AtomicBool>,
            samples: Arc<std::sync::atomic::AtomicU64>,
        }

        impl vearth_data::ElevationSource for PreparedElevation {
            fn elevation_m(&self, _lat: f64, _lon: f64, _sample_pitch_m: f64) -> f32 {
                assert!(
                    self.prepared.load(std::sync::atomic::Ordering::Acquire),
                    "CPU sampling ran before async preparation completed"
                );
                self.samples
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                0.0
            }
        }

        impl vearth_data::async_prep::AsyncSourcePreparer for PreparedElevation {
            fn prepare<'a>(
                &'a self,
                _request: &'a vearth_data::async_prep::PrepareRequest,
            ) -> vearth_data::async_prep::PrepareFuture<'a> {
                Box::pin(async move {
                    self.prepared
                        .store(true, std::sync::atomic::Ordering::Release);
                    let mut summary = vearth_data::async_prep::PrepareSummary::default();
                    summary.record(vearth_data::async_prep::DependencyOutcome::Ready);
                    summary
                })
            }
        }

        let prepared = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let samples = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let source = Arc::new(PreparedElevation {
            prepared: prepared.clone(),
            samples: samples.clone(),
        });
        let mut registry = Registry::new();
        registry.add_elevation_async(
            vearth_data::registry::ProviderInfo {
                name: "prepared test".into(),
                resolution_m: 30.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            source,
        );
        let mut pool = GenPool::new(Arc::new(registry), None, 2);
        assert!(pool.async_acquisition());
        let cell = CellId {
            face: Face(0),
            level: MIN_RENDER_LEVEL,
            i: 0,
            j: 0,
        };
        pool.sync_queue(&[(cell, 4.0)]);
        let deadline =
            vearth_platform::time::Instant::now() + vearth_platform::time::Duration::from_secs(2);
        loop {
            if let Some(result) = pool.drain_done().pop() {
                assert_eq!(result.cell(), cell);
                break;
            }
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "async staged pipeline stalled"
            );
            vearth_platform::thread::sleep(vearth_platform::time::Duration::from_millis(1));
        }
        assert!(prepared.load(std::sync::atomic::Ordering::Acquire));
        assert!(samples.load(std::sync::atomic::Ordering::Relaxed) > 0);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn canceled_async_acquisition_drops_stale_work_before_cpu() {
        check_async_cancellation(false);
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn shutdown_cancels_provider_futures_before_pool_is_dropped() {
        check_async_cancellation(true);
    }

    #[cfg(feature = "async-providers")]
    fn check_async_cancellation(shutdown: bool) {
        struct GatedAsyncElevation {
            started: std::sync::mpsc::Sender<()>,
            release: Arc<std::sync::atomic::AtomicBool>,
            canceled: Arc<std::sync::atomic::AtomicBool>,
            samples: Arc<std::sync::atomic::AtomicU64>,
        }

        struct CancellationSignal(Arc<std::sync::atomic::AtomicBool>);

        impl Drop for CancellationSignal {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Release);
            }
        }

        impl vearth_data::ElevationSource for GatedAsyncElevation {
            fn elevation_m(&self, _lat: f64, _lon: f64, _sample_pitch_m: f64) -> f32 {
                self.samples
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                0.0
            }
        }

        impl vearth_data::async_prep::AsyncSourcePreparer for GatedAsyncElevation {
            fn prepare<'a>(
                &'a self,
                _request: &'a vearth_data::async_prep::PrepareRequest,
            ) -> vearth_data::async_prep::PrepareFuture<'a> {
                Box::pin(async move {
                    let _signal = CancellationSignal(self.canceled.clone());
                    let _ = self.started.send(());
                    while !self.release.load(std::sync::atomic::Ordering::Acquire) {
                        vearth_platform::rt::task::yield_now().await;
                    }
                    let mut summary = vearth_data::async_prep::PrepareSummary::default();
                    summary.record(vearth_data::async_prep::DependencyOutcome::Ready);
                    summary
                })
            }
        }

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let canceled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let samples = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut registry = Registry::new();
        registry.add_elevation_async(
            vearth_data::registry::ProviderInfo {
                name: "gated async".into(),
                resolution_m: 30.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(GatedAsyncElevation {
                started: started_tx,
                release: release.clone(),
                canceled: canceled.clone(),
                samples: samples.clone(),
            }),
        );
        let mut pool = GenPool::new(Arc::new(registry), None, 1);
        let cell = queue_cell(0, 0);
        pool.sync_queue(&[(cell, 1.0)]);
        started_rx
            .recv_timeout(vearth_platform::time::Duration::from_secs(1))
            .expect("async preparation started");
        if shutdown {
            pool.shutdown();
            pool.sync_queue(&[(queue_cell(1, 0), 2.0)]);
        } else {
            pool.sync_queue(&[]);
        }
        let deadline =
            vearth_platform::time::Instant::now() + vearth_platform::time::Duration::from_secs(1);
        while pool.counts().1 != 0 || !canceled.load(std::sync::atomic::Ordering::Acquire) {
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "canceled async acquisition stayed running"
            );
            vearth_platform::thread::yield_now();
        }
        assert!(pool.drain_done().is_empty());
        assert!(canceled.load(std::sync::atomic::Ordering::Acquire));
        assert!(!release.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(samples.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(pool.stage_counts(), PoolStageCounts::default());
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn shutdown_rejects_staged_cpu_work_and_later_queue_updates() {
        let mut pool = GenPool::new(Arc::new(Registry::new()), None, 1);
        let work = WorkItem {
            cell: queue_cell(0, 0),
            revision: 17,
            priority: 1.0,
        };
        {
            let mut state = pool.shared.state.lock().unwrap();
            state.running.insert(work.cell, work.revision);
            state.desired.insert(work.cell);
            state.stages.insert(work.cell, CellStage::Cpu);
        }
        pool.shutdown();
        pool.shutdown();
        pool.sync_queue(&[(queue_cell(1, 0), 2.0)]);
        assert!(!advance_stage(&pool.shared, work, CellStage::Cpu));
        assert!(take_pending(&pool.shared).is_none());
        assert_eq!(pool.counts(), (0, 0));
        assert_eq!(pool.stage_counts(), PoolStageCounts::default());
    }

    #[cfg(feature = "async-providers")]
    #[test]
    fn panicking_async_preparer_releases_running_revision() {
        struct PanickingPreparer {
            started: std::sync::mpsc::Sender<()>,
        }

        impl vearth_data::ElevationSource for PanickingPreparer {
            fn elevation_m(&self, _lat: f64, _lon: f64, _sample_pitch_m: f64) -> f32 {
                0.0
            }
        }

        impl vearth_data::async_prep::AsyncSourcePreparer for PanickingPreparer {
            fn prepare<'a>(
                &'a self,
                _request: &'a vearth_data::async_prep::PrepareRequest,
            ) -> vearth_data::async_prep::PrepareFuture<'a> {
                Box::pin(async move {
                    let _ = self.started.send(());
                    vearth_platform::rt::time::sleep(vearth_platform::time::Duration::from_millis(
                        1,
                    ))
                    .await;
                    panic!("intentional preparer panic");
                })
            }
        }

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let mut registry = Registry::new();
        registry.add_elevation_async(
            vearth_data::registry::ProviderInfo {
                name: "panicking async".into(),
                resolution_m: 30.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(PanickingPreparer {
                started: started_tx,
            }),
        );
        let mut pool = GenPool::new(Arc::new(registry), None, 1);
        pool.sync_queue(&[(queue_cell(0, 0), 1.0)]);
        started_rx
            .recv_timeout(vearth_platform::time::Duration::from_secs(1))
            .expect("preparer started");
        let deadline =
            vearth_platform::time::Instant::now() + vearth_platform::time::Duration::from_secs(1);
        while pool.counts().1 != 0 {
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "panicking acquisition stranded a running revision"
            );
            vearth_platform::thread::yield_now();
        }
        assert_eq!(pool.stage_counts(), PoolStageCounts::default());
        assert!(pool.drain_done().is_empty());
    }

    #[test]
    fn canceled_acquisition_never_enters_cpu_generation() {
        struct GatedPrefetch {
            started: std::sync::mpsc::Sender<()>,
            release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
            samples: Arc<std::sync::atomic::AtomicU64>,
        }
        impl vearth_data::ElevationSource for GatedPrefetch {
            fn elevation_m(&self, _lat: f64, _lon: f64, _sample_pitch_m: f64) -> f32 {
                self.samples
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                0.0
            }

            fn prefetch(&self, _tiles: &[(u8, u32, u32)]) {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
        }

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let samples = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut registry = Registry::new();
        registry.add_elevation(
            vearth_data::registry::ProviderInfo {
                name: "gated prefetch".into(),
                resolution_m: 30.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(GatedPrefetch {
                started: started_tx,
                release: std::sync::Mutex::new(release_rx),
                samples: samples.clone(),
            }),
        );
        let mut pool = GenPool::new(Arc::new(registry), None, 1);
        assert!(pool.staged_pipeline());
        let cell = queue_cell(0, 0);
        pool.sync_queue(&[(cell, 1.0)]);
        started_rx
            .recv_timeout(vearth_platform::time::Duration::from_secs(1))
            .expect("acquisition started");

        pool.sync_queue(&[]);
        release_tx.send(()).unwrap();
        let deadline =
            vearth_platform::time::Instant::now() + vearth_platform::time::Duration::from_secs(1);
        while pool.counts().1 != 0 {
            assert!(
                vearth_platform::time::Instant::now() < deadline,
                "canceled acquisition stayed running"
            );
            vearth_platform::thread::yield_now();
        }
        assert!(pool.drain_done().is_empty());
        assert_eq!(samples.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(pool.stage_counts(), PoolStageCounts::default());
    }

    #[test]
    fn cell_plan_is_deterministic_and_vector_bounds_are_explicit() {
        let cell = CellId {
            face: Face(0),
            level: vearth_gen::tier0::WATER_MIN_LEVEL,
            i: 100,
            j: 200,
        };
        let first = CellPlan::for_cell(cell, true, 7.0);
        let second = CellPlan::for_cell(cell, true, 7.0);
        assert_eq!(first, second);
        assert!(!first.terrain_tiles.is_empty());
        assert!(first.vector_bounds.is_some());
        assert!(
            first.night_building_bounds.is_none(),
            "facade-bound lights use the ordinary visible-building query"
        );
        let without_vectors = CellPlan::for_cell(cell, false, 7.0);
        assert!(without_vectors.vector_bounds.is_none());
        assert!(without_vectors.night_building_bounds.is_none());
    }

    #[test]
    fn prefetch_failure_cannot_persist_a_later_tombstoned_fallback() {
        struct PrefetchFailure {
            errors: std::sync::atomic::AtomicU64,
        }
        impl vearth_data::ElevationSource for PrefetchFailure {
            fn elevation_m(&self, _lat: f64, _lon: f64, _sample_pitch_m: f64) -> f32 {
                // A tombstoned provider can return fallback data without a
                // second error increment during generation.
                0.0
            }

            fn sample_elevation(&self, _: f64, _: f64, _: f64) -> vearth_data::SourceSample<f32> {
                vearth_data::SourceSample::unavailable()
            }

            fn prefetch(&self, _tiles: &[(u8, u32, u32)]) {
                self.errors
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }

            fn fetch_errors(&self) -> u64 {
                self.errors.load(std::sync::atomic::Ordering::Relaxed)
            }
        }

        let cell = queue_cell(0, 0);
        let mut registry = Registry::new();
        registry.add_elevation(
            vearth_data::registry::ProviderInfo {
                name: "failing prefetch".into(),
                resolution_m: 30.0,
                coverage: vearth_data::registry::Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(PrefetchFailure {
                errors: std::sync::atomic::AtomicU64::new(0),
            }),
        );
        let plan = CellPlan::for_cell(cell, false, 1.0);
        plan.acquire(&registry);

        let root = std::env::temp_dir().join(format!(
            "vearth-prefetch-failure-{}-{:?}",
            std::process::id(),
            vearth_platform::thread::current().id()
        ));
        let cache = ChunkCache::with_ram_budget(root.to_str().unwrap(), "test", 1024);
        let surface = generate_surface_cell(cell, &Sources::terrain_only(&registry));
        assert!(!surface.complete);
        assert!(!persist_complete_column(&cache, &surface, true));
        assert_eq!(cache.stats().resident_entries, 0);
        assert!(cache.load(cell).is_none());
    }

    #[test]
    fn ancestor_fallback_resolves() {
        let mut idx = ResidentStore::new();
        let leaf = CellId {
            face: Face(0),
            level: 8,
            i: 100,
            j: 50,
        };
        let grandparent = leaf.parent().unwrap().parent().unwrap();
        idx.insert_for_test(grandparent, 0);
        let wanted = vec![WantedCell {
            cell: leaf,
            screen_px: 10.0,
            lane: ViewLane::Legacy,
        }];
        let (draw, missing) = idx.drawable_cut(&wanted);
        assert_eq!(draw, vec![grandparent]);
        assert_eq!(missing, vec![(leaf, 10.0)]);
    }

    #[test]
    fn backdrop_stays_until_quad_completes_but_arrivals_draw() {
        // Parent resident, 4 wanted children: with 3 of 4 resident, the
        // parent stays as backdrop AND the 3 arrived children draw on top
        // (progressive refinement); the parent leaves the cut on exactly
        // the frame the 4th child lands.
        let mut idx = ResidentStore::new();
        let parent = CellId {
            face: Face(0),
            level: 6,
            i: 10,
            j: 20,
        };
        idx.insert_for_test(parent, 0);
        let children = parent.children();
        let wanted: Vec<WantedCell> = children
            .iter()
            .map(|&c| WantedCell {
                cell: c,
                screen_px: 5.0,
                lane: ViewLane::Legacy,
            })
            .collect();

        for ch in children.iter().take(3) {
            idx.insert_for_test(*ch, 0);
        }
        let (draw, missing) = idx.drawable_cut(&wanted);
        assert!(draw.contains(&parent), "backdrop held: {draw:?}");
        assert_eq!(draw.len(), 4, "backdrop + 3 arrivals: {draw:?}");
        assert!(children.iter().take(3).all(|c| draw.contains(c)));
        assert_eq!(missing.len(), 1);

        idx.insert_for_test(children[3], 0);
        let (draw, missing) = idx.drawable_cut(&wanted);
        assert_eq!(draw.len(), 4, "quad complete, backdrop gone: {draw:?}");
        assert!(children.iter().all(|c| draw.contains(c)));
        assert!(missing.is_empty());
    }

    #[test]
    fn fine_content_survives_a_new_missing_sibling() {
        // Street-level pan regression: A,B,C,D all resident and drawn at
        // fine level; the camera moves and D refines — one of D's children
        // is missing. The still-valid fine cells A,B,C and D's arrived
        // children must keep drawing (D as local backdrop); the view must
        // NOT collapse to the coarse parent.
        let mut idx = ResidentStore::new();
        let parent = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        idx.insert_for_test(parent, 0);
        let children = parent.children();
        for ch in &children {
            idx.insert_for_test(*ch, 0);
        }
        let d = children[3];
        let grandkids = d.children();
        for g in grandkids.iter().take(3) {
            idx.insert_for_test(*g, 0);
        }
        let mut wanted: Vec<WantedCell> = children[..3]
            .iter()
            .map(|&c| WantedCell {
                cell: c,
                screen_px: 2.0,
                lane: ViewLane::Legacy,
            })
            .collect();
        wanted.extend(grandkids.iter().map(|&g| WantedCell {
            cell: g,
            screen_px: 4.0,
            lane: ViewLane::Legacy,
        }));

        let (draw, missing) = idx.drawable_cut(&wanted);
        assert!(!draw.contains(&parent), "must not collapse to the parent");
        assert!(
            children[..3].iter().all(|c| draw.contains(c)),
            "fine kept: {draw:?}"
        );
        assert!(draw.contains(&d), "D backdrops its missing quadrant");
        assert!(
            grandkids.iter().take(3).all(|g| draw.contains(g)),
            "arrivals drawn"
        );
        assert_eq!(missing, vec![(grandkids[3], 4.0)]);
    }

    #[test]
    fn merge_uses_nearest_complete_resident_descendant_cover() {
        // The wanted parent was evicted, but its old fine cut still covers
        // every quadrant. Keep that cut while the parent re-streams instead
        // of popping to the coarser ancestor. Cached grandchildren are finer
        // than both the requested cut and its complete resident fallback.
        let mut idx = ResidentStore::new();
        let wanted_parent = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        let ancestor = wanted_parent.parent().unwrap();
        idx.insert_for_test(ancestor, 0);

        let children = wanted_parent.children();
        for child in children {
            idx.insert_for_test(child, 0);
        }
        let grandchildren = children[0].children();
        for grandchild in grandchildren {
            idx.insert_for_test(grandchild, 0);
        }

        let wanted = vec![WantedCell {
            cell: wanted_parent,
            screen_px: 2.0,
            lane: ViewLane::Legacy,
        }];
        let (draw, missing) = idx.drawable_cut(&wanted);

        assert_eq!(missing, vec![(wanted_parent, 2.0)]);
        assert!(
            !draw.contains(&ancestor),
            "complete descendants replace ancestor: {draw:?}"
        );
        assert!(
            children.iter().all(|cell| draw.contains(cell)),
            "nearest resident cover remains sufficient: {draw:?}"
        );
        assert!(grandchildren.iter().all(|cell| !draw.contains(cell)));
        assert_eq!(draw.len(), 4);
    }

    #[test]
    fn merge_fallback_does_not_resurrect_cached_detail_on_successive_coarsening() {
        let mut store = ResidentStore::new();
        let parent = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        let coarser = parent.parent().unwrap();
        store.insert_for_test(coarser.parent().unwrap(), 0);
        for sibling in coarser
            .children()
            .into_iter()
            .filter(|cell| *cell != parent)
        {
            store.insert_for_test(sibling, 0);
        }
        for child in parent.children() {
            store.insert_for_test(child, 0);
        }
        for cached in descendants_at(parent, 15) {
            store.insert_for_test(cached, 0);
        }
        let requested = |cell| {
            [WantedCell {
                cell,
                screen_px: 2.0,
                lane: ViewLane::Legacy,
            }]
        };
        let (draw, missing) = store.drawable_cut(&requested(parent));
        assert_eq!(missing, vec![(parent, 2.0)]);
        assert_eq!(draw, parent.children().to_vec());
        store.insert_for_test(parent, 1);
        assert_eq!(store.drawable_cut(&requested(parent)).0, vec![parent]);
        let (draw, missing) = store.drawable_cut(&requested(coarser));
        assert_eq!(missing, vec![(coarser, 2.0)]);
        assert_eq!(draw, coarser.children().to_vec());
        assert!(draw.iter().all(|cell| cell.level == 10));
        store.insert_for_test(coarser, 2);
        assert_eq!(store.drawable_cut(&requested(coarser)).0, vec![coarser]);
    }

    #[test]
    fn merge_fallback_preserves_mixed_depth_gaps_with_or_without_ancestor() {
        let parent = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        let children = parent.children();
        let complete = children[1].children();
        let partial = children[2].children();
        for with_ancestor in [false, true] {
            let mut store = ResidentStore::new();
            let ancestor = parent.parent().unwrap();
            if with_ancestor {
                store.insert_for_test(ancestor, 0);
            }
            store.insert_for_test(children[0], 0);
            for cell in complete.into_iter().chain(partial[..3].iter().copied()) {
                store.insert_for_test(cell, 0);
            }
            for covered in [children[0], complete[0], partial[0]] {
                for hidden in covered.children() {
                    store.insert_for_test(hidden, 0);
                }
            }
            let wanted = [WantedCell {
                cell: parent,
                screen_px: 2.0,
                lane: ViewLane::Legacy,
            }];
            let (draw, missing) = store.drawable_cut(&wanted);
            assert_eq!(missing, vec![(parent, 2.0)]);
            let mut expected = vec![children[0]];
            expected.extend(complete);
            expected.extend_from_slice(&partial[..3]);
            if with_ancestor {
                expected.insert(0, ancestor);
            }
            assert_eq!(draw, expected);
            let coverage = crate::transition::CoverageIndex::from_cells(draw.iter().copied());
            assert_eq!(coverage.fully_covers(parent), with_ancestor);
            assert!(coverage.fully_covers(children[0]));
            assert!(coverage.fully_covers(children[1]));
            assert_eq!(coverage.fully_covers(children[2]), with_ancestor);
            assert_eq!(coverage.fully_covers(children[3]), with_ancestor);
        }
    }

    #[test]
    fn merge_keeps_ancestor_for_descendant_coverage_gaps() {
        // Three children and one isolated grandchild are not a cover of the
        // missing wanted parent. The ancestor must remain for the uncovered
        // area, while every fine resident still draws progressively over it.
        let mut idx = ResidentStore::new();
        let wanted_parent = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        let ancestor = wanted_parent.parent().unwrap();
        idx.insert_for_test(ancestor, 0);

        let children = wanted_parent.children();
        for child in &children[..3] {
            idx.insert_for_test(*child, 0);
        }
        let isolated = children[3].children()[0];
        idx.insert_for_test(isolated, 0);

        let wanted = vec![WantedCell {
            cell: wanted_parent,
            screen_px: 2.0,
            lane: ViewLane::Legacy,
        }];
        let (draw, missing) = idx.drawable_cut(&wanted);

        assert_eq!(missing, vec![(wanted_parent, 2.0)]);
        assert!(
            draw.contains(&ancestor),
            "gap requires ancestor backdrop: {draw:?}"
        );
        assert!(children[..3].iter().all(|cell| draw.contains(cell)));
        assert!(
            draw.contains(&isolated),
            "partial fine arrival remains progressive: {draw:?}"
        );
        assert_eq!(
            draw.len(),
            5,
            "ancestor + every available descendant: {draw:?}"
        );
    }

    #[test]
    fn coverage_bootstrap_skips_every_intermediate_when_a_backdrop_exists() {
        let mut store = ResidentStore::new();
        let leaf = CellId {
            face: Face(0),
            level: 19,
            i: 100_000,
            j: 220_000,
        };
        let mut root = leaf;
        while root.level > MIN_RENDER_LEVEL {
            root = root.parent().unwrap();
        }
        assert_eq!(missing_coverage_root(leaf, &store), Some(root));
        let mut backdrop = leaf;
        while backdrop.level > 6 {
            backdrop = backdrop.parent().unwrap();
        }
        store.insert_for_test(backdrop, 0);
        assert_eq!(missing_coverage_root(leaf, &store), None);
        store.remove(backdrop);
        store.insert_for_test(root, 0);
        assert_eq!(missing_coverage_root(leaf, &store), None);
        assert_eq!(missing_coverage_root(root, &store), None);
    }

    #[test]
    fn coverage_bootstrap_zoom_out_converges_after_unused_ancestor_eviction() {
        use crate::residency::{
            LeaseBook, LeaseKind, ResidencyBudget, ResidentCell, eviction_plan,
        };

        let mut store = ResidentStore::new();
        let parent = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        let ancestor = parent.parent().unwrap();
        let children = parent.children();
        let fine_cut: Vec<_> = children[0]
            .children()
            .into_iter()
            .chain(children[1..].iter().copied())
            .collect();
        for cell in &fine_cut {
            store.insert(*cell, 10, (0.0, 100.0), 1.0, 0.0, None, None, 1024);
        }
        store.insert(ancestor, 0, (0.0, 100.0), 4.0, 0.0, None, None, 512);
        let wanted = [WantedCell {
            cell: parent,
            screen_px: 2.0,
            lane: ViewLane::Visible,
        }];
        let budget = ResidencyBudget {
            hard_bytes: 2048,
            low_bytes: 1024,
            evict_bytes_per_frame: 16 * 1024,
        };
        let mut leases = LeaseBook::default();
        for (_, record) in store.iter() {
            leases.register_resident(record.cell, 1000);
        }
        let evict = |store: &ResidentStore, drawn: &[CellId], leases: &mut LeaseBook, frame| {
            for &cell in drawn {
                leases.protect(cell, LeaseKind::Presented, frame + 4);
            }
            eviction_plan(
                store.len(),
                store.gpu_bytes(),
                frame,
                leases,
                budget,
                |cell| {
                    let (_, record) = store.get_cell(cell).unwrap();
                    ResidentCell {
                        cell,
                        gpu_bytes: record.gpu_bytes,
                        last_used_frame: record.last_drawn_frame,
                    }
                },
            )
        };

        // The old fine cut exceeds the budget but must remain visible until
        // its requested merge parent arrives. Its unused ancestor is evictable.
        let (draw, _) = store.drawable_cut(&wanted);
        assert_eq!(draw.len(), fine_cut.len());
        let victims = evict(&store, &draw, &mut leases, 1000);
        assert_eq!(victims, [ancestor]);
        for cell in victims {
            store.remove(cell);
            leases.unregister_resident(cell);
        }
        let (draw, pending) = store.drawable_cut(&wanted);
        assert_eq!(draw.len(), fine_cut.len());
        assert_eq!(missing_coverage_root(parent, &store), None);
        assert_eq!(
            pending,
            [(parent, wanted[0].stream_priority())],
            "unused coarse backfills must not outrank and starve the merge parent"
        );

        store.insert(parent, 1000, (0.0, 100.0), 2.0, 0.0, None, None, 1024);
        leases.register_resident(parent, 1000);
        let (draw, missing) = store.drawable_cut(&wanted);
        assert_eq!(draw, [parent]);
        assert!(missing.is_empty());
        assert!(evict(&store, &draw, &mut leases, 1001).is_empty());
        let victims = evict(&store, &draw, &mut leases, 1005);
        assert_eq!(victims.len(), fine_cut.len());
        for cell in victims {
            store.remove(cell);
            leases.unregister_resident(cell);
        }
        assert_eq!(store.gpu_bytes(), 1024);
        for _ in 0..3 {
            let (draw, missing) = store.drawable_cut(&wanted);
            assert_eq!(draw, [parent]);
            assert!(missing.is_empty());
            assert_eq!(missing_coverage_root(parent, &store), None);
            assert!(evict(&store, &draw, &mut leases, 1006).is_empty());
        }
    }

    #[test]
    fn coverage_bootstrap_tracks_partial_and_nested_descendant_coverage() {
        let mut store = ResidentStore::new();
        let parent = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        let children = parent.children();
        let grandchildren = children[0].children();
        let mut root = parent;
        while root.level > MIN_RENDER_LEVEL {
            root = root.parent().unwrap();
        }
        for cell in children[1..].iter().chain(grandchildren[..3].iter()) {
            store.insert_for_test(*cell, 0);
        }
        assert_eq!(
            missing_coverage_root(parent, &store),
            Some(root),
            "one uncovered grandchild still needs coverage"
        );
        store.insert_for_test(grandchildren[3], 0);
        assert_eq!(missing_coverage_root(parent, &store), None);
        let sibling = parent.parent().unwrap().children()[1];
        assert_ne!(parent, sibling);
        assert_eq!(
            missing_coverage_root(sibling, &store),
            Some(root),
            "a complete merge cut cannot suppress its uncovered sibling's bootstrap"
        );
        store.remove(grandchildren[0]);
        assert_eq!(missing_coverage_root(parent, &store), Some(root));
    }

    #[test]
    fn covered_by_drawn_matches_relatives_only() {
        let cell = CellId {
            face: Face(0),
            level: 9,
            i: 40,
            j: 80,
        };
        let mut drawn = HashSet::new();
        assert!(!covered_by_drawn(cell, &drawn), "empty: uncovered");
        // Unrelated neighbor does not cover.
        drawn.insert(CellId {
            face: Face(0),
            level: 9,
            i: 41,
            j: 80,
        });
        assert!(!covered_by_drawn(cell, &drawn));
        // An ancestor covers (backdrop/refine pairing).
        drawn.insert(cell.parent().unwrap().parent().unwrap());
        assert!(covered_by_drawn(cell, &drawn));
        // A drawn descendant covers (merge pairing).
        let mut drawn2 = HashSet::new();
        drawn2.insert(cell.children()[2].children()[1]);
        assert!(covered_by_drawn(cell, &drawn2));
        // Same-index cell on another face never covers.
        let mut drawn3 = HashSet::new();
        drawn3.insert(CellId {
            face: Face(1),
            level: 7,
            i: 10,
            j: 20,
        });
        assert!(!covered_by_drawn(
            CellId {
                face: Face(0),
                level: 9,
                i: 40,
                j: 80
            },
            &drawn3
        ));
    }

    #[test]
    fn full_coverage_requires_every_descendant_quadrant() {
        let cell = CellId {
            face: Face(0),
            level: 9,
            i: 40,
            j: 80,
        };
        let mut drawn = HashSet::from([cell.children()[0]]);
        assert!(covered_by_drawn(cell, &drawn));
        assert!(
            !fully_covered_by_drawn(cell, &drawn),
            "one child must not start a parent fade over three sky quadrants"
        );
        drawn.extend(cell.children()[1..].iter().copied());
        assert!(fully_covered_by_drawn(cell, &drawn));

        let children = cell.children();
        drawn.remove(&children[0]);
        drawn.extend(children[0].children());
        assert!(
            fully_covered_by_drawn(cell, &drawn),
            "mixed-depth antichain covers recursively"
        );
        assert!(fully_covered_by_drawn(
            cell,
            &HashSet::from([cell.parent().unwrap()])
        ));
    }

    #[test]
    fn deep_dive_shows_arrivals_progressively() {
        // Fast-dive regression: only a coarse ancestor is resident and the
        // wanted leaves sit several levels deeper. Every arrived leaf must
        // draw immediately over the coarse backdrop — not stay hidden until
        // the entire subtree completes.
        let mut idx = ResidentStore::new();
        let coarse = CellId {
            face: Face(0),
            level: 6,
            i: 10,
            j: 20,
        };
        idx.insert_for_test(coarse, 0);
        // Wanted: all 16 L8 descendants; 5 scattered ones have arrived.
        let leaves: Vec<CellId> = coarse
            .children()
            .into_iter()
            .flat_map(|c| c.children())
            .collect();
        assert_eq!(leaves.len(), 16);
        for leaf in leaves.iter().step_by(3).take(5) {
            idx.insert_for_test(*leaf, 0);
        }
        let wanted: Vec<WantedCell> = leaves
            .iter()
            .map(|&c| WantedCell {
                cell: c,
                screen_px: 6.0,
                lane: ViewLane::Legacy,
            })
            .collect();
        let (draw, missing) = idx.drawable_cut(&wanted);
        assert!(draw.contains(&coarse), "coarse backdrop kept");
        for leaf in leaves.iter().step_by(3).take(5) {
            assert!(draw.contains(leaf), "arrived leaf hidden: {leaf:?}");
        }
        assert_eq!(missing.len(), 11);
    }

    fn assert_collision_broad_phase_matches_scan(
        store: &ResidentStore,
        center: DVec3,
        half_axes: [DVec3; 3],
        origin: DVec3,
        direction: DVec3,
        max_t: f64,
    ) {
        assert_eq!(
            store.box_hit(center, half_axes, &mut HashMap::new()),
            store.box_hit_with_broad_phase::<false>(center, half_axes, &mut HashMap::new()),
            "box at {center:?} with {half_axes:?}"
        );
        let actual = store.raycast(origin, direction, max_t, &mut HashMap::new());
        let expected =
            store.raycast_with_broad_phase::<false>(origin, direction, max_t, &mut HashMap::new());
        assert_eq!(actual, expected, "ray from {origin:?} along {direction:?}");
    }

    #[test]
    fn collision_broad_phase_matches_full_scan_across_refined_coverage() {
        for face in 0..Face::COUNT {
            for level in [11, 15, 18] {
                let edge = 1 << level;
                let coarse = CellId {
                    face: Face(face),
                    level,
                    i: edge / 3,
                    j: edge / 2 + 3,
                };
                let frame = ChunkFrame::of(ChunkAddr { cell: coarse, k: 0 });
                let mut store = ResidentStore::new();
                let walls = |frame: &ChunkFrame, x: f32| {
                    SurfaceCollision::from_triangles(crate::prepared_render::test_box_triangles(
                        frame,
                        [x, 0.0, 0.0],
                        [x + 0.01, 25.0, 32.0],
                    ))
                };
                store.set_collision_for_test(coarse, walls(&frame, 12.0));
                let children = coarse.children();
                store.set_collision_for_test(children[2], SurfaceCollision::default());
                let fine_frame = ChunkFrame::of(ChunkAddr {
                    cell: children[3],
                    k: 0,
                });
                store.set_collision_for_test(children[3], walls(&fine_frame, 18.0));
                store
                    .set_collision_for_test(children[3].children()[0], SurfaceCollision::default());
                for revision in 0..3 {
                    for sample in 0..48 {
                        let x = -4.0 + f64::from((sample * 13) % 41);
                        let z = -4.0 + f64::from((sample * 7) % 41);
                        let y = -2.0 + f64::from((sample * 11) % 31);
                        let local = DVec3::new(x, y, z);
                        let center = frame.local_to_ecef(local);
                        let half_axes = [
                            frame.x_axis * (0.1 + f64::from(sample % 9)) + frame.z_axis * 0.3,
                            frame.y_axis * 0.4,
                            frame.z_axis * 0.8 - frame.x_axis * 0.2,
                        ];
                        let start = frame.local_to_ecef(local - DVec3::new(18.0, 0.0, 0.0));
                        let end = frame.local_to_ecef(local + DVec3::new(18.0, 0.0, 0.0));
                        assert_collision_broad_phase_matches_scan(
                            &store,
                            center,
                            half_axes,
                            start,
                            (end - start).normalize(),
                            start.distance(end),
                        );
                    }
                    match revision {
                        0 => store.set_collision_for_test(children[3], walls(&fine_frame, 5.0)),
                        1 => {
                            store.remove(children[2]);
                            store.remove(children[3]);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    #[test]
    fn collision_broad_phase_retains_boundaries_and_face_seam_fallback() {
        for face in 0..Face::COUNT {
            let cell = CellId {
                face: Face(face),
                level: 19,
                i: 100_003,
                j: 200_005,
            };
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let mut store = ResidentStore::new();
            store.set_collision_for_test(cell, SurfaceCollision::default());
            for child in cell.children() {
                store.set_collision_for_test(child, SurfaceCollision::default());
            }
            for (u, v) in [cell.center_uv(), cell.uv_at(0.0, 0.0), cell.uv_at(1.0, 1.0)] {
                let point = vearth_core::cubesphere::face_uv_to_dir(cell.face, u, v)
                    * ellipsoid::MEAN_RADIUS;
                assert_collision_broad_phase_matches_scan(
                    &store,
                    point,
                    [
                        frame.x_axis * 1e-5,
                        frame.y_axis * 1e-5,
                        frame.z_axis * 1e-5,
                    ],
                    point - frame.y_axis,
                    frame.y_axis + frame.x_axis * 1e-16,
                    2.0,
                );
            }
            let seam = vearth_core::cubesphere::face_uv_to_dir(cell.face, 1.0, 0.0)
                * ellipsoid::MEAN_RADIUS;
            assert!(collision_query_range(&[seam]).is_none());
            let points = [seam - cell.face.right(), seam + cell.face.right()];
            assert!(collision_query_range(&points).is_none());
            assert_collision_broad_phase_matches_scan(
                &store,
                seam,
                [cell.face.right(), cell.face.up(), cell.face.normal()],
                points[0],
                cell.face.right(),
                2.0,
            );
        }
        assert!(collision_query_range(&[]).is_none());
        assert!(collision_query_range(&[DVec3::ZERO]).is_none());
        assert!(collision_query_range(&[DVec3::splat(f64::NAN)]).is_none());
        assert!(collision_query_range(&[DVec3::splat(f64::INFINITY)]).is_none());
    }

    #[test]
    fn collision_broad_phase_skips_distant_regions_and_keeps_containing_fallback() {
        let cell = CellId {
            face: Face(0),
            level: 19,
            i: 100_003,
            j: 200_005,
        };
        let mut store = ResidentStore::new();
        let owner = cell.parent().unwrap().parent().unwrap();
        store.set_collision_for_test(owner, SurfaceCollision::default());
        for offset in 1..=512 {
            store.set_collision_for_test(
                CellId {
                    i: cell.i + offset * 8,
                    ..cell
                },
                SurfaceCollision::default(),
            );
        }
        store.ensure_collision_regions();
        let regions = store.collision_regions.borrow();
        let regions = &regions.as_ref().unwrap().regions;
        assert_eq!(regions.len(), 513);
        let point = cell.center_dir() * ellipsoid::MEAN_RADIUS;
        let candidates = collision_region_candidates(regions, &[point]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].owner, store.id(owner).unwrap());
        assert!(candidates[0].range.start() < CellKey::from(cell).range().start());
    }

    #[test]
    fn collision_local_cache_replaces_broad_covers_and_reuses_the_same_neighbourhood() {
        let local = CellId {
            face: Face(0),
            level: 19,
            i: 100_003,
            j: 200_005,
        };
        let distant = CellId {
            i: 180_003,
            ..local
        };
        let mut store = ResidentStore::new();
        store.set_collision_for_test(local, SurfaceCollision::default());
        store.set_collision_for_test(distant, SurfaceCollision::default());
        let local_point = local.center_dir() * ellipsoid::MEAN_RADIUS;
        let distant_point = distant.center_dir() * ellipsoid::MEAN_RADIUS;
        let broad_cover = collision_cache_cover(&[local_point, distant_point]).unwrap();
        assert_eq!(broad_cover.level, 1);
        store.ensure_collision_regions_for_cover(Some(broad_cover));
        assert_eq!(
            store
                .collision_regions
                .borrow()
                .as_ref()
                .unwrap()
                .regions
                .len(),
            2
        );

        let narrow_cover = collision_cache_cover(&[local_point]).unwrap();
        assert_eq!(narrow_cover.level, 16);
        store.ensure_collision_regions_for_cover(Some(narrow_cover));
        let capacity = {
            let mut cache = store.collision_regions.borrow_mut();
            let cache = cache.as_mut().unwrap();
            assert_eq!(cache.cover, Some(narrow_cover));
            assert_eq!(cache.regions.len(), 1);
            assert_eq!(cache.regions[0].owner, store.id(local).unwrap());
            // Extra capacity makes an unnecessary rebuild observable even if
            // the allocator reuses the old vector's address.
            cache.regions.reserve_exact(64);
            cache.regions.capacity()
        };
        for offset in [-2.0, 0.0, 2.0] {
            let nearby = local_point + local.face.right() * offset;
            let nearby_cover = collision_cache_cover(&[nearby]).unwrap();
            assert_eq!(nearby_cover, narrow_cover);
            store.ensure_collision_regions_for_cover(Some(nearby_cover));
            let cache = store.collision_regions.borrow();
            let cache = cache.as_ref().unwrap();
            assert_eq!(cache.cover, Some(narrow_cover));
            assert_eq!(cache.regions.capacity(), capacity);
            assert_eq!(cache.regions.len(), 1);
        }
    }

    #[test]
    fn collision_local_cache_ignores_distant_owners_and_invalidates_nearby_changes() {
        let local = CellId {
            face: Face(0),
            level: 19,
            i: 100_003,
            j: 200_005,
        };
        let distant = CellId {
            i: local.i + 1024,
            ..local
        };
        let mut store = ResidentStore::new();
        store.set_collision_for_test(local, SurfaceCollision::default());
        let point = local.center_dir() * ellipsoid::MEAN_RADIUS;
        let cover = collision_cache_cover(&[point]).unwrap();
        assert_eq!(cover.level, 16);
        store.ensure_collision_regions_for_cover(Some(cover));
        let local_id = store.id(local).unwrap();
        let cache_owner = |store: &ResidentStore| {
            let cache = store.collision_regions.borrow();
            let cache = cache.as_ref().expect("local cache retained");
            assert_eq!(cache.cover, Some(cover));
            assert_eq!(cache.regions.len(), 1);
            cache.regions[0].owner
        };
        assert_eq!(cache_owner(&store), local_id);
        let distant_id = store.insert_for_test(distant, 0);
        store.finish_upload(
            distant_id,
            Some(SurfaceCollision::default()),
            None,
            Vec::new(),
            Vec::new(),
            0,
        );
        assert_eq!(cache_owner(&store), local_id);
        store.finish_upload(distant_id, None, None, Vec::new(), Vec::new(), 0);
        assert_eq!(cache_owner(&store), local_id);
        store.finish_upload(
            distant_id,
            Some(SurfaceCollision::default()),
            None,
            Vec::new(),
            Vec::new(),
            0,
        );
        store.remove(distant);
        assert_eq!(cache_owner(&store), local_id);
        store.set_collision_for_test(distant, SurfaceCollision::default());
        assert_eq!(cache_owner(&store), local_id);

        store.finish_upload(
            local_id,
            Some(SurfaceCollision::default()),
            None,
            Vec::new(),
            Vec::new(),
            0,
        );
        assert_eq!(cache_owner(&store), local_id);
        let ancestor = cover.parent().unwrap();
        store.set_collision_for_test(ancestor, SurfaceCollision::default());
        assert!(store.collision_regions.borrow().is_none());
        store.ensure_collision_regions_for_cover(Some(cover));
        store.remove(local);
        assert!(store.collision_regions.borrow().is_none());
        store.ensure_collision_regions_for_cover(Some(cover));
        {
            let cache = store.collision_regions.borrow();
            let cache = cache.as_ref().unwrap();
            assert_eq!(cache.regions.len(), 1);
            assert_eq!(cache.regions[0].owner, store.id(ancestor).unwrap());
            assert_eq!(cache.regions[0].range, CellKey::from(cover).range());
        }
        let distant_id = store.id(distant).unwrap();
        store.get_mut(distant_id).unwrap();
        assert!(store.collision_regions.borrow().is_none());
        store.ensure_collision_regions_for_cover(Some(cover));
        store.ensure_collision_regions();
        assert!(
            store
                .collision_regions
                .borrow()
                .as_ref()
                .unwrap()
                .cover
                .is_none()
        );
        store.ensure_collision_regions_for_cover(Some(cover));
        assert_eq!(
            store.collision_regions.borrow().as_ref().unwrap().cover,
            Some(cover)
        );
    }

    #[test]
    fn collision_region_cache_survives_draw_and_noncollision_residency_changes() {
        let cell = CellId {
            face: Face(0),
            level: 19,
            i: 1 << 18,
            j: 1 << 18,
        };
        let mut store = ResidentStore::new();
        let id = store.insert(
            cell,
            0,
            (0.0, 20.0),
            1.0,
            0.0,
            None,
            Some(SurfaceCollision::default()),
            0,
        );
        store.ensure_collision_regions();
        let signature = |store: &ResidentStore| {
            store
                .collision_regions
                .borrow()
                .as_ref()
                .unwrap()
                .regions
                .iter()
                .map(|region| {
                    (
                        region.owner,
                        region.planes.map(|plane| plane.map(f64::to_bits)),
                    )
                })
                .collect::<Vec<_>>()
        };
        let original = signature(&store);
        for frame in 1..10 {
            assert_eq!(
                if frame % 2 == 0 {
                    store.touch(cell, frame)
                } else {
                    store.touch_id(id, frame)
                },
                Some(id)
            );
            assert_eq!(store.get(id).unwrap().last_drawn_frame, frame);
            assert!(store.collision_regions.borrow().is_some());
        }
        let background = cell.parent().unwrap();
        let background_id = store.insert_for_test(background, 0);
        assert!(store.collision_regions.borrow().is_some());
        store.finish_upload(background_id, None, None, Vec::new(), Vec::new(), 256);
        assert!(store.collision_regions.borrow().is_some());
        store.insert_for_test(background, 5);
        assert!(store.collision_regions.borrow().is_some());
        store.remove(background);
        assert_eq!(signature(&store), original);
        *store.collision_regions.get_mut() = None;
        store.ensure_collision_regions();
        assert_eq!(signature(&store), original);
    }

    #[test]
    fn collision_region_cache_reads_replacement_payload_and_tracks_owner_membership() {
        let cell = CellId {
            face: Face(0),
            level: 19,
            i: 1 << 18,
            j: 1 << 18,
        };
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let collider = |x| {
            SurfaceCollision::from_triangles(crate::prepared_render::test_box_triangles(
                &frame,
                [x, 0.0, 0.0],
                [x + 0.1, 25.0, 32.0],
            ))
        };
        let mut store = ResidentStore::new();
        let id = store.insert(
            cell,
            0,
            (0.0, 25.0),
            1.0,
            0.0,
            None,
            Some(collider(10.0)),
            0,
        );
        let start = frame.local_to_ecef(DVec3::new(2.0, 10.0, 16.0));
        let end = frame.local_to_ecef(DVec3::new(28.0, 10.0, 16.0));
        let mut memo = HashMap::new();
        let ray = |store: &ResidentStore, memo: &mut HashMap<CellId, ChunkFrame>| {
            store.raycast(start, (end - start).normalize(), start.distance(end), memo)
        };
        let first = ray(&store, &mut memo).unwrap();
        store.finish_upload(id, Some(collider(20.0)), None, Vec::new(), Vec::new(), 0);
        assert!(store.collision_regions.borrow().is_some());
        let replaced = ray(&store, &mut memo).unwrap();
        assert!(replaced > first + frame.grid_scale.x * 9.0);
        *store.collision_regions.get_mut() = None;
        assert_eq!(ray(&store, &mut memo), Some(replaced));

        store.finish_upload(id, None, None, Vec::new(), Vec::new(), 0);
        assert!(store.collision_regions.borrow().is_none());
        assert_eq!(ray(&store, &mut memo), None);
        store.finish_upload(id, Some(collider(10.0)), None, Vec::new(), Vec::new(), 0);
        assert!(store.collision_regions.borrow().is_none());
        assert_eq!(ray(&store, &mut memo), Some(first));
        store.insert(
            cell,
            1,
            (0.0, 25.0),
            1.0,
            0.0,
            None,
            Some(collider(20.0)),
            0,
        );
        assert!(store.collision_regions.borrow().is_some());
        assert_eq!(ray(&store, &mut memo), Some(replaced));
        store.remove(cell);
        assert!(store.collision_regions.borrow().is_none());
        assert_eq!(ray(&store, &mut memo), None);
    }

    #[test]
    fn continuous_resident_collision_retains_thin_geometry_and_clears_replacements() {
        let cell = CellId {
            face: Face(0),
            level: 19,
            i: 1 << 18,
            j: 1 << 18,
        };
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let collision =
            SurfaceCollision::from_triangles(crate::prepared_render::test_box_triangles(
                &frame,
                [15.125, 0.0, 0.0],
                [15.135, 25.0, 32.0],
            ));
        let mut store = ResidentStore::new();
        let id = store.insert(cell, 0, (0.0, 25.0), 1.0, 0.0, None, Some(collision), 0);
        let mut memo = HashMap::new();
        let inside = frame.local_to_ecef(DVec3::new(15.13, 10.0, 16.0));
        assert_eq!(store.solid_at(inside, &mut memo), Some(true));
        let start = frame.local_to_ecef(DVec3::new(2.0, 10.0, 16.0));
        let end = frame.local_to_ecef(DVec3::new(28.0, 10.0, 16.0));
        let direction = (end - start).normalize();
        let hit = store
            .raycast(start, direction, start.distance(end), &mut memo)
            .unwrap();
        assert!(
            (hit - frame
                .local_to_ecef(DVec3::new(15.125, 10.0, 16.0))
                .distance(start))
            .abs()
                < 0.001
        );
        assert_eq!(
            store.box_hit(
                (start + end) * 0.5,
                [(end - start) * 0.5, frame.y_axis * 0.2, frame.z_axis * 0.2],
                &mut memo
            ),
            Some(true)
        );
        assert_eq!(
            store.insert(cell, 1, (0.0, 0.0), 1.0, 0.0, None, None, 0),
            id
        );
        assert!(store.get(id).unwrap().solid_collision.is_none());
        assert_eq!(store.solid_at(inside, &mut memo), None);
    }

    #[test]
    fn heightfield_queries_retain_ridges_and_finest_resident_ownership() {
        let cell = CellId {
            face: Face(0),
            level: 19,
            i: 1 << 18,
            j: 1 << 18,
        };
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let mut heights = Box::new([0.0f32; 33 * 33]);
        for z in 14..=18 {
            heights[z * 33 + 12] = 8.0;
            heights[z * 33 + 13] = 8.0;
        }
        let mut store = ResidentStore::new();
        let id = store.insert(
            cell,
            0,
            (0.0, 8.0),
            frame.grid_scale.y as f32,
            0.0,
            None,
            None,
            0,
        );
        store.finish_upload(
            id,
            Some(SurfaceCollision::default()),
            Some(TerrainCollision {
                heights_local: heights,
                source_columns: [u32::MAX; 32],
            }),
            Vec::new(),
            Vec::new(),
            0,
        );
        let start = frame.local_to_ecef(DVec3::new(2.0, 5.0, 16.0));
        let end = frame.local_to_ecef(DVec3::new(28.0, 5.0, 16.0));
        let center = frame.local_to_ecef(DVec3::new(16.0, 5.0, 16.0));
        let half_axes = [
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(0.0, 0.2, 0.0),
            DVec3::new(0.0, 0.0, 0.2),
        ]
        .map(|delta| frame.local_to_ecef(DVec3::new(16.0, 5.0, 16.0) + delta) - center);
        let mut memo = HashMap::new();
        assert_eq!(store.solid_at(center, &mut memo), Some(false));
        assert_eq!(
            store.box_hit(center, half_axes, &mut memo),
            Some(true),
            "ridge between the center and box corners must remain solid"
        );
        assert!(
            store
                .raycast(
                    start,
                    (end - start).normalize(),
                    start.distance(end),
                    &mut memo
                )
                .is_some()
        );
        for child in cell.children() {
            store.set_collision_for_test(child, SurfaceCollision::default());
        }
        assert_eq!(store.box_hit(center, half_axes, &mut memo), Some(false));
        assert!(
            store
                .raycast(
                    start,
                    (end - start).normalize(),
                    start.distance(end),
                    &mut memo
                )
                .is_none(),
            "superseded coarse heightfield cannot block finer empty coverage"
        );
        for child in cell.children() {
            store.remove(child);
        }
        assert!(
            store
                .raycast(
                    start,
                    (end - start).normalize(),
                    start.distance(end),
                    &mut memo
                )
                .is_some()
        );
    }

    #[test]
    fn adjacent_fine_collision_regions_exclude_parent_and_retain_thin_walls() {
        let coarse = CellId {
            face: Face(0),
            level: 18,
            i: 1 << 17,
            j: 1 << 17,
        };
        let frame = ChunkFrame::of(ChunkAddr { cell: coarse, k: 0 });
        let mut store = ResidentStore::new();
        store.set_collision_for_test(
            coarse,
            SurfaceCollision::from_triangles(crate::prepared_render::test_box_triangles(
                &frame,
                [0., 0., 0.],
                [32., 20., 32.],
            )),
        );
        let fine = coarse.children();
        for cell in [fine[2], fine[3]] {
            store.set_collision_for_test(cell, SurfaceCollision::default());
        }
        let center = frame.local_to_ecef(DVec3::new(16., 10., 8.));
        let mut memo = HashMap::new();
        assert_eq!(
            store.box_hit(
                center,
                [frame.x_axis * 2., frame.y_axis * 0.5, frame.z_axis * 0.5],
                &mut memo
            ),
            Some(false),
            "a coarse volume must not block across two refined children"
        );
        assert!(
            store
                .raycast(center + frame.y_axis * 20., -frame.y_axis, 30., &mut memo)
                .is_none(),
            "hidden parent roof must not stop a camera ray"
        );
        let child_frame = ChunkFrame::of(ChunkAddr {
            cell: fine[3],
            k: 0,
        });
        store.set_collision_for_test(
            fine[3],
            SurfaceCollision::from_triangles(crate::prepared_render::test_box_triangles(
                &child_frame,
                [8.125, 0., 0.],
                [8.135, 30., 32.],
            )),
        );
        let start = frame.local_to_ecef(DVec3::new(10., 10., 8.));
        let end = frame.local_to_ecef(DVec3::new(26., 10., 8.));
        assert_eq!(
            store.box_hit(
                (start + end) * 0.5,
                [(end - start) * 0.5, frame.y_axis * 0.2, frame.z_axis * 0.2],
                &mut memo
            ),
            Some(true),
            "sweep spanning adjacent cells must retain a centimetre wall"
        );
        assert!(
            store
                .raycast(
                    start,
                    (end - start).normalize(),
                    start.distance(end),
                    &mut memo
                )
                .is_some()
        );
        store.remove(fine[2]);
        store.remove(fine[3]);
        assert_eq!(
            store.solid_at(center, &mut memo),
            Some(true),
            "parent coverage returns when children leave"
        );
    }

    #[test]
    fn ray_clips_hidden_first_parent_hit_without_losing_later_uncovered_hit() {
        let coarse = CellId {
            face: Face(0),
            level: 18,
            i: 1 << 17,
            j: 1 << 17,
        };
        let frame = ChunkFrame::of(ChunkAddr { cell: coarse, k: 0 });
        let mut triangles =
            crate::prepared_render::test_box_triangles(&frame, [10., 0., 0.], [10.1, 30., 32.]);
        triangles.extend(crate::prepared_render::test_box_triangles(
            &frame,
            [24., 0., 0.],
            [24.1, 30., 32.],
        ));
        let mut store = ResidentStore::new();
        store.set_collision_for_test(coarse, SurfaceCollision::from_triangles(triangles));
        store.set_collision_for_test(coarse.children()[2], SurfaceCollision::default());
        let start = frame.local_to_ecef(DVec3::new(5., 10., 8.));
        let end = frame.local_to_ecef(DVec3::new(28., 10., 8.));
        let expected = frame
            .local_to_ecef(DVec3::new(24., 10., 8.))
            .distance(start);
        let actual = store
            .raycast(
                start,
                (end - start).normalize(),
                start.distance(end),
                &mut HashMap::new(),
            )
            .unwrap();
        assert!((actual - expected).abs() < 0.002, "{actual} != {expected}");
    }

    #[test]
    fn nearest_road_guide_prefers_alignment_and_rejects_crossings() {
        let n = 1u32 << 19;
        let cell = CellId {
            face: Face(0),
            level: 19,
            i: n / 2,
            j: n / 2,
        };
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let mut idx = ResidentStore::new();
        idx.set_road_guides_for_test(
            cell,
            vec![
                RoadGuide {
                    a_local: [3.0, 16.0],
                    b_local: [29.0, 16.0],
                    a_exit: [0.0; 2],
                    b_exit: [0.0; 2],
                    width_m: 6.5,
                    assist: 0.9,
                },
                RoadGuide {
                    a_local: [16.0, 3.0],
                    b_local: [16.0, 29.0],
                    a_exit: [0.0; 2],
                    b_exit: [0.0; 2],
                    width_m: 6.5,
                    assist: 0.9,
                },
            ],
        );
        let p = frame.local_to_ecef(DVec3::new(18.0, 5.0, 18.0));
        let along_x = idx
            .nearest_road_guide(p, frame.x_axis, 5.0, 0.0)
            .expect("aligned road should capture");
        assert!(along_x.tangent.dot(frame.x_axis) > 0.99);
        assert!(along_x.center_offset.dot(frame.z_axis) < 0.0);

        let crossing_only = {
            let mut idx = ResidentStore::new();
            idx.set_road_guides_for_test(
                cell,
                vec![RoadGuide {
                    a_local: [3.0, 16.0],
                    b_local: [29.0, 16.0],
                    a_exit: [0.0; 2],
                    b_exit: [0.0; 2],
                    width_m: 6.5,
                    assist: 0.9,
                }],
            );
            idx
        };
        assert!(
            crossing_only
                .nearest_road_guide(p, frame.z_axis, 5.0, 0.0)
                .is_none(),
            "transverse off-road travel must not latch"
        );
    }

    #[test]
    fn nearest_road_guide_anticipates_authored_sharp_turn() {
        let n = 1u32 << 19;
        let cell = CellId {
            face: Face(0),
            level: 19,
            i: n / 2,
            j: n / 2,
        };
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let mut idx = ResidentStore::new();
        idx.set_road_guides_for_test(
            cell,
            vec![RoadGuide {
                a_local: [3.0, 16.0],
                b_local: [29.0, 16.0],
                a_exit: [0.0, -1.0],
                b_exit: [0.0, 1.0],
                width_m: 6.5,
                assist: 0.9,
            }],
        );
        let p = frame.local_to_ecef(DVec3::new(23.0, 5.0, 16.0));

        let without_lookahead = idx
            .nearest_road_guide(p, frame.x_axis, 5.0, 0.0)
            .expect("current segment should capture");
        let anticipated = idx
            .nearest_road_guide(p, frame.x_axis, 5.0, 12.0)
            .expect("turning segment should capture");

        assert!(without_lookahead.tangent.dot(frame.x_axis) > 0.99);
        assert!(anticipated.tangent.dot(frame.x_axis) > 0.2);
        assert!(anticipated.tangent.dot(frame.z_axis) > 0.6);

        let reverse_p = frame.local_to_ecef(DVec3::new(9.0, 5.0, 16.0));
        let reverse = idx
            .nearest_road_guide(reverse_p, -frame.x_axis, 5.0, 12.0)
            .expect("reverse turn should capture");
        assert!(reverse.tangent.dot(frame.x_axis) < -0.2);
        assert!(reverse.tangent.dot(frame.z_axis) < -0.6);
    }

    #[test]
    fn readiness_recurses_through_replaced_children() {
        // Parent resident; children A,B,C resident and wanted; D missing
        // but D's own 4 children are resident and wanted (mixed-level cut).
        // D counts as ready via its children: the parent must swap out.
        let mut idx = ResidentStore::new();
        let parent = CellId {
            face: Face(0),
            level: 6,
            i: 3,
            j: 5,
        };
        idx.insert_for_test(parent, 0);
        let children = parent.children();
        let mut wanted = Vec::new();
        for ch in children.iter().take(3) {
            idx.insert_for_test(*ch, 0);
            wanted.push(WantedCell {
                cell: *ch,
                screen_px: 5.0,
                lane: ViewLane::Legacy,
            });
        }
        let d = children[3];
        for g in d.children() {
            idx.insert_for_test(g, 0);
            wanted.push(WantedCell {
                cell: g,
                screen_px: 6.0,
                lane: ViewLane::Legacy,
            });
        }
        let (draw, missing) = idx.drawable_cut(&wanted);
        assert!(missing.is_empty());
        assert_eq!(draw.len(), 7, "3 children + 4 grandchildren: {draw:?}");
        assert!(!draw.contains(&parent));
        assert!(!draw.contains(&d));
    }
}
