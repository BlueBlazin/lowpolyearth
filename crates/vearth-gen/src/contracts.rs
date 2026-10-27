//! Generated surface identity and opt-in generation diagnostics.
use crate::SurfaceCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use vearth_core::contracts::{Fingerprint, SemanticHasher, mesh_semantic_fingerprint};
use vearth_platform::time::Instant;

/// Geometry, source completeness and all retained gameplay descriptors.
/// Derived BVH nodes are omitted; their immutable primitives are hashed.
pub fn surface_cell_fingerprint(column: &SurfaceCell) -> Fingerprint {
    let mut hasher = SemanticHasher::new("vearth.surface-cell.v1");
    hasher.u8(column.cell.face.0);
    hasher.u8(column.cell.level);
    hasher.u32(column.cell.i);
    hasher.u32(column.cell.j);
    hasher.bool(column.complete);
    hasher.f32(column.height_range.0);
    hasher.f32(column.height_range.1);
    hasher.f32(column.terrain_relief_m);
    hasher.f32(column.terrain_step_m);
    hasher.bool(column.terrain_detail.is_some());
    if let Some(grid) = &column.terrain_detail {
        hasher.fingerprint(grid.fingerprint());
    }
    hasher.u64(column.batches.len() as u64);
    for batch in &column.batches {
        hasher.i32(batch.addr.k);
        hasher.fingerprint(mesh_semantic_fingerprint(&batch.mesh));
        crate::built_surface::hash_detail(&mut hasher, batch.detail);
        hasher.u64(batch.shadow_omissions.len() as u64);
        for span in &batch.shadow_omissions {
            hasher.u32(span.first_index);
            hasher.u32(span.index_count);
            hasher.f32(span.max_deviation_m);
        }
    }
    crate::collision::hash_collision(&mut hasher, &column.solid_collision);
    hasher.bool(column.terrain_collision.is_some());
    if let Some(terrain) = &column.terrain_collision {
        for &height in terrain.heights_local.iter() {
            hasher.f32(height);
        }
        for mask in terrain.source_columns {
            hasher.u32(mask);
        }
    }
    hasher.u64(column.water.len() as u64);
    for water in &column.water {
        for point in water.vertices {
            for coordinate in point {
                hasher.f32(coordinate);
            }
        }
        hasher.u8(water.kind as u8);
        hasher.f32(water.depth_m);
    }
    hasher.u64(column.road_guides.len() as u64);
    for road in &column.road_guides {
        for value in road
            .a_local
            .into_iter()
            .chain(road.b_local)
            .chain(road.a_exit)
            .chain(road.b_exit)
            .chain([road.width_m, road.assist])
        {
            hasher.f32(value);
        }
    }
    hasher.u64(column.night_lights.len() as u64);
    for light in &column.night_lights {
        for value in light.local_pos {
            hasher.f32(value);
        }
        hasher.f32(light.energy);
        hasher.bytes(&light.color);
        hasher.u8(light.bucket);
    }
    hasher.u64(column.tree_instances.len() as u64);
    for tree in &column.tree_instances {
        hasher.u8(tree.profile as u8);
        hasher.u64(tree.seed);
        hasher.f64(tree.lat_rad);
        hasher.f64(tree.lon_rad);
        for value in [
            tree.elevation_m,
            tree.height_m,
            tree.yaw_rad,
            tree.cell_x,
            tree.cell_z,
        ] {
            hasher.f32(value);
        }
    }
    hasher.u64(column.flora.len() as u64);
    for flora in &column.flora {
        hasher.u8(flora.archetype as u8);
        hasher.u8(flora.calendar as u8);
        hasher.u64(flora.field_seed);
        hasher.f32(flora.row_heading_rad);
        hasher.u8(flora.palette);
        hasher.u8(flora.base_density);
        for word in flora.coverage {
            hasher.u32(word);
        }
        hasher.i32(flora.ground_base_cm);
        for value in flora.ground_cm {
            hasher.bytes(&value.to_le_bytes());
        }
        hasher.u8(flora.phenology_offset);
        hasher.u8(flora.curated_style);
    }
    hasher.finish()
}

pub const GENERATION_PHASES: [&str; 11] = [
    "features_setup",
    "terrain_sampling",
    "overlay",
    "terrain_appearance",
    "water",
    "conditioned_terrain",
    "buildings",
    "bridges",
    "roads",
    "rest",
    "finalization",
];
pub const ROOF_DIAGNOSTIC_SHAPES: [&str; 11] = [
    "unknown",
    "flat",
    "gabled",
    "hipped",
    "pyramidal",
    "mansard",
    "gambrel",
    "skillion",
    "dome",
    "onion",
    "sawtooth",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoofWorkCounters {
    pub roofs: u64,
    pub parent_roofs: u64,
    pub simple_roofs: u64,
    pub measured_roofs: u64,
    pub elapsed_ns: u64,
    pub analytic_attempts: u64,
    pub analytic_accepted: u64,
    pub initial_cells: u64,
    pub coarse_grid_fallbacks: u64,
    pub plane_attempts: u64,
    pub plane_successes: u64,
    pub plane_concavity_rejections: u64,
    pub plane_rejections: [u64; 7],
    pub refinement_visits: [u64; 10],
    pub fallback_depth: u64,
    pub fallback_budget: u64,
    pub fallback_invalid: u64,
    pub discarded_triangles: u64,
    pub triangles: u64,
    pub elapsed_buckets: [u64; 8],
}
impl RoofWorkCounters {
    const ZERO: Self = Self {
        roofs: 0,
        parent_roofs: 0,
        simple_roofs: 0,
        measured_roofs: 0,
        elapsed_ns: 0,
        analytic_attempts: 0,
        analytic_accepted: 0,
        initial_cells: 0,
        coarse_grid_fallbacks: 0,
        plane_attempts: 0,
        plane_successes: 0,
        plane_concavity_rejections: 0,
        plane_rejections: [0; 7],
        refinement_visits: [0; 10],
        fallback_depth: 0,
        fallback_budget: 0,
        fallback_invalid: 0,
        discarded_triangles: 0,
        triangles: 0,
        elapsed_buckets: [0; 8],
    };
    fn add(&mut self, other: Self) {
        macro_rules! sum { ($($field:ident),*) => { $(self.$field += other.$field;)* }; }
        sum!(
            roofs,
            parent_roofs,
            simple_roofs,
            measured_roofs,
            elapsed_ns,
            analytic_attempts,
            analytic_accepted,
            initial_cells,
            coarse_grid_fallbacks,
            plane_attempts,
            plane_successes,
            plane_concavity_rejections,
            fallback_depth,
            fallback_budget,
            fallback_invalid,
            discarded_triangles,
            triangles
        );
        for (a, b) in self.plane_rejections.iter_mut().zip(other.plane_rejections) {
            *a += b;
        }
        for (a, b) in self
            .refinement_visits
            .iter_mut()
            .zip(other.refinement_visits)
        {
            *a += b;
        }
        for (a, b) in self.elapsed_buckets.iter_mut().zip(other.elapsed_buckets) {
            *a += b;
        }
    }
}
pub const ROOF_PLANE_REJECTIONS: [&str; 7] = [
    "sample",
    "degenerate",
    "too_many_planes",
    "one_plane",
    "known_height",
    "candidate_height",
    "fine_error",
];
pub const ROOF_ELAPSED_BUCKET_UPPER_NS: [u64; 7] = [
    100_000,
    500_000,
    1_000_000,
    5_000_000,
    10_000_000,
    50_000_000,
    100_000_000,
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlowRoofRecord {
    pub cell_face: u8,
    pub cell_level: u8,
    pub cell_i: u32,
    pub cell_j: u32,
    pub source_ordinal: u32,
    pub seed: u64,
    pub parent_body: bool,
    pub simple: bool,
    pub shape: u8,
    pub reentrant: bool,
    pub tiered: bool,
    pub measured: bool,
    pub rings: u32,
    pub edges: u32,
    pub extent_m: [f64; 2],
    pub rise_m: f64,
    pub pitch: f64,
    pub work: RoofWorkCounters,
}
struct RoofDiagnosticsState {
    by_shape_and_level: [[RoofWorkCounters; 32]; ROOF_DIAGNOSTIC_SHAPES.len()],
    slow: [Option<SlowRoofRecord>; 32],
}
static ROOF_DIAGNOSTICS: std::sync::Mutex<RoofDiagnosticsState> =
    std::sync::Mutex::new(RoofDiagnosticsState {
        by_shape_and_level: [[RoofWorkCounters::ZERO; 32]; ROOF_DIAGNOSTIC_SHAPES.len()],
        slow: [None; 32],
    });
static GENERATION_PHASE_NS: [[AtomicU64; 32]; 11] =
    [const { [const { AtomicU64::new(0) }; 32] }; 11];
static GENERATION_PHASE_COUNTS: [[AtomicU64; 32]; 11] =
    [const { [const { AtomicU64::new(0) }; 32] }; 11];
static BUILDING_RETRIES: [AtomicU64; 32] = [const { AtomicU64::new(0) }; 32];
static BUILDING_RETRY_NS: [AtomicU64; 32] = [const { AtomicU64::new(0) }; 32];
static BUILDING_DISCARDED_BYTES: [AtomicU64; 32] = [const { AtomicU64::new(0) }; 32];

pub(crate) fn generation_timing_enabled() -> bool {
    GENERATION_TIMING_ENABLED.load(Ordering::Relaxed)
}
impl RoofDiagnosticsState {
    fn record(&mut self, mut record: SlowRoofRecord) {
        record.work.roofs = 1;
        record.work.parent_roofs = u64::from(record.parent_body);
        record.work.simple_roofs = u64::from(record.simple);
        record.work.measured_roofs = u64::from(record.measured);
        let bucket = ROOF_ELAPSED_BUCKET_UPPER_NS.partition_point(|&n| record.work.elapsed_ns >= n);
        record.work.elapsed_buckets[bucket] = 1;
        self.by_shape_and_level[usize::from(record.shape).min(ROOF_DIAGNOSTIC_SHAPES.len() - 1)]
            [usize::from(record.cell_level).min(31)]
        .add(record.work);
        let index = self.slow.iter().position(|entry| {
            entry.is_none_or(|other| other.work.elapsed_ns < record.work.elapsed_ns)
        });
        if let Some(index) = index {
            self.slow.copy_within(index..31, index + 1);
            self.slow[index] = Some(record);
        }
    }
}
pub(crate) fn record_roof(record: SlowRoofRecord) {
    ROOF_DIAGNOSTICS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(record);
}
pub(crate) fn record_building_retry(level: u8, elapsed_ns: u64, discarded_bytes: usize) {
    let level = usize::from(level).min(31);
    BUILDING_RETRIES[level].fetch_add(1, Ordering::Relaxed);
    BUILDING_RETRY_NS[level].fetch_add(elapsed_ns, Ordering::Relaxed);
    BUILDING_DISCARDED_BYTES[level].fetch_add(discarded_bytes as u64, Ordering::Relaxed);
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerationStageSnapshot {
    pub samples: u64,
    pub generation_ns: u64,
    pub collision_build_ns: u64,
    /// Incomplete cells by stage and cell level. A cell may fail several stages.
    pub incomplete_by_stage: [[u64; 32]; GENERATION_INCOMPLETE_STAGES.len()],
    /// Failure events by reason and cell level; one cell can report several.
    pub building_failures_by_reason: [[u64; 32]; BUILDING_FAILURE_REASONS.len()],
    pub phase_ns_by_level: [[u64; 32]; 11],
    pub phase_counts_by_level: [[u64; 32]; 11],
    pub roof_by_shape_and_level: [[RoofWorkCounters; 32]; ROOF_DIAGNOSTIC_SHAPES.len()],
    pub slow_roofs: [Option<SlowRoofRecord>; 32],
    pub building_retry_count_by_level: [u64; 32],
    pub building_retry_ns_by_level: [u64; 32],
    pub building_discarded_bytes_by_level: [u64; 32],
}

pub const BUILDING_FAILURE_REASONS: [&str; 18] = [
    "plan",
    "domain",
    "datum",
    "roof",
    "collision",
    "tower",
    "budget",
    "other",
    "datum_coverage",
    "domain_cells",
    "domain_rotated",
    "domain_body",
    "domain_short_ring",
    "domain_nonfinite",
    "domain_edges",
    "domain_empty",
    "domain_required_edge_budget",
    "domain_exclusion_edge_budget",
];

pub const GENERATION_INCOMPLETE_STAGES: [&str; 9] = [
    "features",
    "elevation",
    "surface",
    "imagery",
    "water",
    "buildings",
    "bridges",
    "roads",
    "any",
];

static GENERATION_TIMING_ENABLED: AtomicBool = AtomicBool::new(false);
static GENERATION_SAMPLES: AtomicU64 = AtomicU64::new(0);
static GENERATION_NS: AtomicU64 = AtomicU64::new(0);
static COLLISION_BUILD_NS: AtomicU64 = AtomicU64::new(0);
static GENERATION_INCOMPLETE: [[AtomicU64; 32]; GENERATION_INCOMPLETE_STAGES.len()] =
    [const { [const { AtomicU64::new(0) }; 32] }; GENERATION_INCOMPLETE_STAGES.len()];
static BUILDING_FAILURES: [[AtomicU64; 32]; BUILDING_FAILURE_REASONS.len()] =
    [const { [const { AtomicU64::new(0) }; 32] }; BUILDING_FAILURE_REASONS.len()];

pub(crate) fn record_building_failure(reason: usize, level: u8) {
    if GENERATION_TIMING_ENABLED.load(Ordering::Relaxed) {
        BUILDING_FAILURES[reason][usize::from(level).min(31)].fetch_add(1, Ordering::Relaxed);
    }
}

/// Enable or disable process-wide generation timing. Disabled is the default
/// and performs no clock reads in `generate_surface_cell`.
pub fn set_generation_timing_enabled(enabled: bool) {
    GENERATION_TIMING_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn reset_generation_timing() {
    for phases in [&GENERATION_PHASE_NS, &GENERATION_PHASE_COUNTS] {
        for phase in phases {
            for value in phase {
                value.store(0, Ordering::Relaxed);
            }
        }
    }
    for values in [
        &BUILDING_RETRIES,
        &BUILDING_RETRY_NS,
        &BUILDING_DISCARDED_BYTES,
    ] {
        for value in values {
            value.store(0, Ordering::Relaxed);
        }
    }
    let mut roofs = ROOF_DIAGNOSTICS.lock().unwrap_or_else(|e| e.into_inner());
    roofs.by_shape_and_level = [[RoofWorkCounters::ZERO; 32]; ROOF_DIAGNOSTIC_SHAPES.len()];
    roofs.slow = [None; 32];
    drop(roofs);
    GENERATION_SAMPLES.store(0, Ordering::Relaxed);
    GENERATION_NS.store(0, Ordering::Relaxed);
    COLLISION_BUILD_NS.store(0, Ordering::Relaxed);
    for stage in GENERATION_INCOMPLETE.iter().chain(BUILDING_FAILURES.iter()) {
        for level in stage {
            level.store(0, Ordering::Relaxed);
        }
    }
}

pub fn generation_timing_snapshot() -> GenerationStageSnapshot {
    let roofs = ROOF_DIAGNOSTICS.lock().unwrap_or_else(|e| e.into_inner());
    GenerationStageSnapshot {
        phase_ns_by_level: std::array::from_fn(|phase| {
            std::array::from_fn(|level| GENERATION_PHASE_NS[phase][level].load(Ordering::Relaxed))
        }),
        phase_counts_by_level: std::array::from_fn(|phase| {
            std::array::from_fn(|level| {
                GENERATION_PHASE_COUNTS[phase][level].load(Ordering::Relaxed)
            })
        }),
        roof_by_shape_and_level: roofs.by_shape_and_level,
        slow_roofs: roofs.slow,
        building_retry_count_by_level: std::array::from_fn(|level| {
            BUILDING_RETRIES[level].load(Ordering::Relaxed)
        }),
        building_retry_ns_by_level: std::array::from_fn(|level| {
            BUILDING_RETRY_NS[level].load(Ordering::Relaxed)
        }),
        building_discarded_bytes_by_level: std::array::from_fn(|level| {
            BUILDING_DISCARDED_BYTES[level].load(Ordering::Relaxed)
        }),
        samples: GENERATION_SAMPLES.load(Ordering::Relaxed),
        generation_ns: GENERATION_NS.load(Ordering::Relaxed),
        collision_build_ns: COLLISION_BUILD_NS.load(Ordering::Relaxed),
        incomplete_by_stage: std::array::from_fn(|stage| {
            std::array::from_fn(|level| GENERATION_INCOMPLETE[stage][level].load(Ordering::Relaxed))
        }),
        building_failures_by_reason: std::array::from_fn(|reason| {
            std::array::from_fn(|level| BUILDING_FAILURES[reason][level].load(Ordering::Relaxed))
        }),
    }
}

pub(crate) struct GenerationStageTimer {
    generation_started: Option<Instant>,
    checkpoint_started: Option<Instant>,
    phase_ns: [u64; 11],
    phase_seen: [bool; 11],
    level: u8,
    finalization_started: Option<Instant>,
    generation_ns: u64,
}

impl GenerationStageTimer {
    pub(crate) fn start() -> Self {
        let started = generation_timing_enabled().then(Instant::now);
        Self {
            generation_started: started,
            checkpoint_started: started,
            phase_ns: [0; 11],
            phase_seen: [false; 11],
            level: 0,
            finalization_started: None,
            generation_ns: 0,
        }
    }

    pub(crate) fn checkpoint(&mut self, phase: usize, level: u8) {
        let Some(started) = self.checkpoint_started else {
            return;
        };
        let now = Instant::now();
        self.checkpoint_started = Some(now);
        self.level = level.min(31);
        self.phase_ns[phase] += now.duration_since(started).as_nanos() as u64;
        self.phase_seen[phase] = true;
    }

    pub(crate) fn record_incomplete(&self, stage: usize, level: u8, complete: bool) {
        if !complete && self.generation_started.is_some() {
            GENERATION_INCOMPLETE[stage][usize::from(level).min(31)]
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn finish_generation(&mut self) {
        if let Some(started) = self.generation_started.take() {
            self.generation_ns = started.elapsed().as_nanos() as u64;
            self.finalization_started = Some(Instant::now());
        }
    }

    pub(crate) fn finish(&mut self) {
        let Some(started) = self.finalization_started.take() else {
            return;
        };
        for phase in 0..GENERATION_PHASES.len() {
            if self.phase_seen[phase] {
                GENERATION_PHASE_NS[phase][usize::from(self.level)]
                    .fetch_add(self.phase_ns[phase], Ordering::Relaxed);
                GENERATION_PHASE_COUNTS[phase][usize::from(self.level)]
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        GENERATION_NS.fetch_add(self.generation_ns, Ordering::Relaxed);
        COLLISION_BUILD_NS.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        GENERATION_SAMPLES.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roof_diagnostics_aggregate_all_work_but_bound_repro_records() {
        let mut state = RoofDiagnosticsState {
            by_shape_and_level: [[RoofWorkCounters::ZERO; 32]; ROOF_DIAGNOSTIC_SHAPES.len()],
            slow: [None; 32],
        };
        let mut elapsed = 0;
        for ordinal in 0..100 {
            let elapsed_ns = (ordinal * 37 % 100) as u64 * 1_000_000;
            elapsed += elapsed_ns;
            state.record(SlowRoofRecord {
                source_ordinal: ordinal,
                seed: ordinal as u64 + 123,
                cell_level: 14,
                shape: 3,
                parent_body: ordinal % 2 == 0,
                simple: ordinal % 4 == 0,
                measured: ordinal % 5 == 0,
                work: RoofWorkCounters {
                    elapsed_ns,
                    plane_attempts: 3,
                    plane_successes: 1,
                    plane_rejections: [0, 0, 0, 2, 0, 0, 0],
                    refinement_visits: [1, 2, 3, 0, 0, 0, 0, 0, 0, 0],
                    ..RoofWorkCounters::default()
                },
                ..SlowRoofRecord::default()
            });
        }
        let work = state.by_shape_and_level[3][14];
        assert_eq!(work.roofs, 100);
        assert_eq!(
            (work.parent_roofs, work.simple_roofs, work.measured_roofs),
            (50, 25, 20)
        );
        assert_eq!(work.elapsed_ns, elapsed);
        assert_eq!(work.plane_attempts, 300);
        assert_eq!(
            work.plane_successes + work.plane_rejections.iter().sum::<u64>(),
            300
        );
        assert_eq!(&work.refinement_visits[..4], &[100, 200, 300, 0]);
        assert_eq!(work.elapsed_buckets.iter().sum::<u64>(), 100);
        assert_eq!(state.by_shape_and_level[3][15], RoofWorkCounters::ZERO);
        for (index, record) in state.slow.iter().enumerate() {
            let record = record.unwrap();
            assert_eq!(record.work.elapsed_ns, (99 - index) as u64 * 1_000_000);
            assert_eq!(record.seed, record.source_ordinal as u64 + 123);
        }
    }

    #[test]
    fn generation_phase_diagnostics_do_no_work_when_timer_is_disabled() {
        let mut timer = GenerationStageTimer {
            generation_started: None,
            checkpoint_started: None,
            phase_ns: [0; 11],
            phase_seen: [false; 11],
            level: 0,
            finalization_started: None,
            generation_ns: 0,
        };
        // Disabled paths return before indexing, publishing or reading a clock.
        timer.checkpoint(usize::MAX, 255);
        timer.record_incomplete(usize::MAX, 255, false);
        timer.finish_generation();
        timer.finish();
        assert_eq!(timer.phase_ns, [0; 11]);
        assert_eq!(timer.phase_seen, [false; 11]);
        assert!(timer.checkpoint_started.is_none());
        assert!(timer.finalization_started.is_none());
    }
}
