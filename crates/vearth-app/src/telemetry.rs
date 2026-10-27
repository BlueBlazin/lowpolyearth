//! Low-overhead rolling distributions for smoothness and streaming metrics.
//!
//! Histograms retain every recent frame in a fixed allocation, exposing
//! p95/p99 spikes without growing memory during long sessions.

use std::collections::VecDeque;

const DEFAULT_WINDOW: usize = 4096;

/// Indices of submitted world triangles, grouped by their flat material ID.
/// Separate player, tree-instance, flora and shadow draws are excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialIndexCounts(pub [u64; 256]);

impl Default for MaterialIndexCounts {
    fn default() -> Self {
        Self([0; 256])
    }
}

impl serde::Serialize for MaterialIndexCounts {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(self.0.as_slice(), serializer)
    }
}

impl MaterialIndexCounts {
    pub fn total(self) -> u64 {
        self.0.into_iter().sum()
    }
}

/// Opt-in census snapshots. Peak bins come from one complete frame, rather
/// than unrelated per-material maxima that never occurred together.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct MaterialIndexReport {
    pub samples: u64,
    pub latest: Option<MaterialIndexCounts>,
    pub peak_frame: Option<MaterialIndexCounts>,
}

impl MaterialIndexReport {
    fn record(&mut self, counts: Option<MaterialIndexCounts>) {
        let Some(counts) = counts else {
            return;
        };
        self.samples += 1;
        self.latest = Some(counts);
        if self
            .peak_frame
            .is_none_or(|previous| counts.total() > previous.total())
        {
            self.peak_frame = Some(counts);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Distribution {
    pub count: usize,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// Fixed-capacity rolling sample window. Values are sorted only when a
/// report is requested, not on the render path.
#[derive(Debug)]
pub struct RollingHistogram {
    samples: VecDeque<f64>,
    capacity: usize,
}

impl RollingHistogram {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        RollingHistogram {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn record(&mut self, value: f64) {
        if !value.is_finite() || value < 0.0 {
            return;
        }
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    pub fn summary(&self) -> Distribution {
        if self.samples.is_empty() {
            return Distribution::default();
        }
        let mut sorted: Vec<_> = self.samples.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let percentile = |p: f64| {
            let index = ((sorted.len() - 1) as f64 * p).ceil() as usize;
            sorted[index]
        };
        Distribution {
            count: sorted.len(),
            p50: percentile(0.50),
            p95: percentile(0.95),
            p99: percentile(0.99),
            max: *sorted.last().unwrap(),
        }
    }
}

impl Default for RollingHistogram {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW)
    }
}

/// Cumulative counters that should remain inspectable even after the rolling
/// frame window advances.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipelineTotals {
    pub frames: u64,
    pub missed_vsync: u64,
    pub uploaded_cells: u64,
    pub uploaded_bytes: u64,
    pub evicted_cells: u64,
    pub evicted_bytes: u64,
    pub stale_results: u64,
    pub stale_gpu_admissions: u64,
    pub ready_backpressure_drops: u64,
    pub draw_overflow: u64,
    pub sun_shadow_capacity_drops: u64,
    pub transitions_started: u64,
    pub transitions_completed: u64,
    pub transition_reversals: u64,
    pub uncovered_transitions: u64,
    pub wanted_budget_saturated: u64,
    pub wanted_topology_changes: u64,
    pub protected_eviction_attempts: u64,
}

/// One frame's CPU/GPU durations in milliseconds and submitted-work counters.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct FrameMeasurement {
    pub interval_ms: f64,
    pub main_ms: f64,
    pub wanted_ms: f64,
    pub pump_ms: f64,
    pub upload_ms: f64,
    pub resolve_ms: f64,
    pub transition_ms: f64,
    pub residency_ms: f64,
    /// Compatibility aggregate: transition + residency.
    pub eviction_ms: f64,
    pub render_submit_ms: f64,
    pub cpu_visibility_ms: f64,
    pub cpu_world_draw_construction_ms: f64,
    pub cpu_sorting_ms: f64,
    pub cpu_uniform_construction_ms: f64,
    pub cpu_grouping_ms: f64,
    pub cpu_indirect_construction_ms: f64,
    pub cpu_sun_shadow_preparation_ms: f64,
    pub upload_preparation_ms: f64,
    pub queue_upload_ms: f64,
    pub gpu_ms: Option<f64>,
    pub gpu_shadow_ms: Option<f64>,
    pub gpu_world_ms: Option<f64>,
    pub gpu_flora_ms: Option<f64>,
    pub gpu_atmosphere_ms: Option<f64>,
    pub gpu_cloud_ms: Option<f64>,
    pub gpu_precipitation_ms: Option<f64>,
    pub gpu_tonemap_ms: Option<f64>,
    pub completion_backlog: usize,
    pub oldest_ready_ms: f64,
    pub wanted_presented_delta: usize,
    /// Submitted terrain/tree/light/flora batches, distinct from visible terrain
    /// chunks because several batches may represent one chunk.
    pub world_draw_batches: usize,
    pub sun_shadow_caster_chunks: usize,
    pub sun_shadow_caster_indices: u64,
    pub sun_shadow_batches: usize,
    pub submitted_world_vertices: u64,
    pub world_material_indices: Option<MaterialIndexCounts>,
    /// World indices submitted through the fragment path without coverage kills.
    pub opaque_world_indices: u64,
    /// World indices submitted through the fade/tree coverage path.
    pub feedback_world_indices: u64,
    pub triangles: u64,
    /// Mesh uploads plus transient world/tree draw data written this frame.
    pub frame_upload_bytes: u64,
    /// Exact resident terrain/player mesh payload at submission time.
    pub resident_gpu_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SmoothnessReport {
    pub frame_interval_ms: Distribution,
    pub main_ms: Distribution,
    pub wanted_ms: Distribution,
    pub pump_ms: Distribution,
    pub upload_ms: Distribution,
    pub resolve_ms: Distribution,
    pub transition_ms: Distribution,
    pub residency_ms: Distribution,
    pub eviction_ms: Distribution,
    pub render_submit_ms: Distribution,
    pub cpu_visibility_ms: Distribution,
    pub cpu_world_draw_construction_ms: Distribution,
    pub cpu_sorting_ms: Distribution,
    pub cpu_uniform_construction_ms: Distribution,
    pub cpu_grouping_ms: Distribution,
    pub cpu_indirect_construction_ms: Distribution,
    pub cpu_sun_shadow_preparation_ms: Distribution,
    pub upload_preparation_ms: Distribution,
    pub queue_upload_ms: Distribution,
    pub gpu_ms: Distribution,
    pub gpu_shadow_ms: Distribution,
    pub gpu_world_ms: Distribution,
    pub gpu_flora_ms: Distribution,
    pub gpu_atmosphere_ms: Distribution,
    pub gpu_cloud_ms: Distribution,
    pub gpu_precipitation_ms: Distribution,
    pub gpu_tonemap_ms: Distribution,
    pub completion_backlog: Distribution,
    pub oldest_ready_ms: Distribution,
    pub wanted_presented_delta: Distribution,
    pub world_draw_batches: Distribution,
    pub sun_shadow_caster_chunks: Distribution,
    pub sun_shadow_caster_indices: Distribution,
    pub sun_shadow_batches: Distribution,
    pub submitted_world_vertices: Distribution,
    pub world_material_indices: MaterialIndexReport,
    pub opaque_world_indices: Distribution,
    pub feedback_world_indices: Distribution,
    pub triangles: Distribution,
    pub frame_upload_bytes: Distribution,
    pub resident_gpu_bytes: Distribution,
    pub totals: PipelineTotals,
}

/// Application-owned metrics collector. `record_frame` performs no sorting
/// and no allocation after the rolling buffers reach capacity.
pub struct SmoothnessTelemetry {
    frame_interval_ms: RollingHistogram,
    main_ms: RollingHistogram,
    wanted_ms: RollingHistogram,
    pump_ms: RollingHistogram,
    upload_ms: RollingHistogram,
    resolve_ms: RollingHistogram,
    transition_ms: RollingHistogram,
    residency_ms: RollingHistogram,
    eviction_ms: RollingHistogram,
    render_submit_ms: RollingHistogram,
    cpu_visibility_ms: RollingHistogram,
    cpu_world_draw_construction_ms: RollingHistogram,
    cpu_sorting_ms: RollingHistogram,
    cpu_uniform_construction_ms: RollingHistogram,
    cpu_grouping_ms: RollingHistogram,
    cpu_indirect_construction_ms: RollingHistogram,
    cpu_sun_shadow_preparation_ms: RollingHistogram,
    upload_preparation_ms: RollingHistogram,
    queue_upload_ms: RollingHistogram,
    gpu_ms: RollingHistogram,
    gpu_shadow_ms: RollingHistogram,
    gpu_world_ms: RollingHistogram,
    gpu_flora_ms: RollingHistogram,
    gpu_atmosphere_ms: RollingHistogram,
    gpu_cloud_ms: RollingHistogram,
    gpu_precipitation_ms: RollingHistogram,
    gpu_tonemap_ms: RollingHistogram,
    completion_backlog: RollingHistogram,
    oldest_ready_ms: RollingHistogram,
    wanted_presented_delta: RollingHistogram,
    world_draw_batches: RollingHistogram,
    sun_shadow_caster_chunks: RollingHistogram,
    sun_shadow_caster_indices: RollingHistogram,
    sun_shadow_batches: RollingHistogram,
    submitted_world_vertices: RollingHistogram,
    world_material_indices: MaterialIndexReport,
    opaque_world_indices: RollingHistogram,
    feedback_world_indices: RollingHistogram,
    triangles: RollingHistogram,
    frame_upload_bytes: RollingHistogram,
    resident_gpu_bytes: RollingHistogram,
    pub totals: PipelineTotals,
}

impl SmoothnessTelemetry {
    pub fn new(window: usize) -> Self {
        let histogram = || RollingHistogram::new(window);
        SmoothnessTelemetry {
            frame_interval_ms: histogram(),
            main_ms: histogram(),
            wanted_ms: histogram(),
            pump_ms: histogram(),
            upload_ms: histogram(),
            resolve_ms: histogram(),
            transition_ms: histogram(),
            residency_ms: histogram(),
            eviction_ms: histogram(),
            render_submit_ms: histogram(),
            cpu_visibility_ms: histogram(),
            cpu_world_draw_construction_ms: histogram(),
            cpu_sorting_ms: histogram(),
            cpu_uniform_construction_ms: histogram(),
            cpu_grouping_ms: histogram(),
            cpu_indirect_construction_ms: histogram(),
            cpu_sun_shadow_preparation_ms: histogram(),
            upload_preparation_ms: histogram(),
            queue_upload_ms: histogram(),
            gpu_ms: histogram(),
            gpu_shadow_ms: histogram(),
            gpu_world_ms: histogram(),
            gpu_flora_ms: histogram(),
            gpu_atmosphere_ms: histogram(),
            gpu_cloud_ms: histogram(),
            gpu_precipitation_ms: histogram(),
            gpu_tonemap_ms: histogram(),
            completion_backlog: histogram(),
            oldest_ready_ms: histogram(),
            wanted_presented_delta: histogram(),
            world_draw_batches: histogram(),
            sun_shadow_caster_chunks: histogram(),
            sun_shadow_caster_indices: histogram(),
            sun_shadow_batches: histogram(),
            submitted_world_vertices: histogram(),
            world_material_indices: MaterialIndexReport::default(),
            opaque_world_indices: histogram(),
            feedback_world_indices: histogram(),
            triangles: histogram(),
            frame_upload_bytes: histogram(),
            resident_gpu_bytes: histogram(),
            totals: PipelineTotals::default(),
        }
    }

    pub fn record_frame(&mut self, frame: FrameMeasurement) {
        self.totals.frames += 1;
        if frame.interval_ms > 1000.0 / 59.0 {
            self.totals.missed_vsync += 1;
        }
        self.frame_interval_ms.record(frame.interval_ms);
        self.main_ms.record(frame.main_ms);
        self.wanted_ms.record(frame.wanted_ms);
        self.pump_ms.record(frame.pump_ms);
        self.upload_ms.record(frame.upload_ms);
        self.resolve_ms.record(frame.resolve_ms);
        self.transition_ms.record(frame.transition_ms);
        self.residency_ms.record(frame.residency_ms);
        self.eviction_ms.record(frame.eviction_ms);
        self.render_submit_ms.record(frame.render_submit_ms);
        self.cpu_visibility_ms.record(frame.cpu_visibility_ms);
        self.cpu_world_draw_construction_ms
            .record(frame.cpu_world_draw_construction_ms);
        self.cpu_sorting_ms.record(frame.cpu_sorting_ms);
        self.cpu_uniform_construction_ms
            .record(frame.cpu_uniform_construction_ms);
        self.cpu_grouping_ms.record(frame.cpu_grouping_ms);
        self.cpu_indirect_construction_ms
            .record(frame.cpu_indirect_construction_ms);
        self.cpu_sun_shadow_preparation_ms
            .record(frame.cpu_sun_shadow_preparation_ms);
        self.upload_preparation_ms
            .record(frame.upload_preparation_ms);
        self.queue_upload_ms.record(frame.queue_upload_ms);
        if let Some(gpu_ms) = frame.gpu_ms {
            self.gpu_ms.record(gpu_ms);
        }
        if let Some(gpu_shadow_ms) = frame.gpu_shadow_ms {
            self.gpu_shadow_ms.record(gpu_shadow_ms);
        }
        if let Some(gpu_world_ms) = frame.gpu_world_ms {
            self.gpu_world_ms.record(gpu_world_ms);
        }
        if let Some(gpu_flora_ms) = frame.gpu_flora_ms {
            self.gpu_flora_ms.record(gpu_flora_ms);
        }
        if let Some(gpu_atmosphere_ms) = frame.gpu_atmosphere_ms {
            self.gpu_atmosphere_ms.record(gpu_atmosphere_ms);
        }
        if let Some(gpu_cloud_ms) = frame.gpu_cloud_ms {
            self.gpu_cloud_ms.record(gpu_cloud_ms);
        }
        if let Some(gpu_precipitation_ms) = frame.gpu_precipitation_ms {
            self.gpu_precipitation_ms.record(gpu_precipitation_ms);
        }
        if let Some(gpu_tonemap_ms) = frame.gpu_tonemap_ms {
            self.gpu_tonemap_ms.record(gpu_tonemap_ms);
        }
        self.completion_backlog
            .record(frame.completion_backlog as f64);
        self.oldest_ready_ms.record(frame.oldest_ready_ms);
        self.wanted_presented_delta
            .record(frame.wanted_presented_delta as f64);
        self.world_draw_batches
            .record(frame.world_draw_batches as f64);
        self.sun_shadow_caster_chunks
            .record(frame.sun_shadow_caster_chunks as f64);
        self.sun_shadow_caster_indices
            .record(frame.sun_shadow_caster_indices as f64);
        self.sun_shadow_batches
            .record(frame.sun_shadow_batches as f64);
        self.submitted_world_vertices
            .record(frame.submitted_world_vertices as f64);
        self.world_material_indices
            .record(frame.world_material_indices);
        self.opaque_world_indices
            .record(frame.opaque_world_indices as f64);
        self.feedback_world_indices
            .record(frame.feedback_world_indices as f64);
        self.triangles.record(frame.triangles as f64);
        self.frame_upload_bytes
            .record(frame.frame_upload_bytes as f64);
        self.resident_gpu_bytes
            .record(frame.resident_gpu_bytes as f64);
    }

    pub fn report(&self) -> SmoothnessReport {
        SmoothnessReport {
            frame_interval_ms: self.frame_interval_ms.summary(),
            main_ms: self.main_ms.summary(),
            wanted_ms: self.wanted_ms.summary(),
            pump_ms: self.pump_ms.summary(),
            upload_ms: self.upload_ms.summary(),
            resolve_ms: self.resolve_ms.summary(),
            transition_ms: self.transition_ms.summary(),
            residency_ms: self.residency_ms.summary(),
            eviction_ms: self.eviction_ms.summary(),
            render_submit_ms: self.render_submit_ms.summary(),
            cpu_visibility_ms: self.cpu_visibility_ms.summary(),
            cpu_world_draw_construction_ms: self.cpu_world_draw_construction_ms.summary(),
            cpu_sorting_ms: self.cpu_sorting_ms.summary(),
            cpu_uniform_construction_ms: self.cpu_uniform_construction_ms.summary(),
            cpu_grouping_ms: self.cpu_grouping_ms.summary(),
            cpu_indirect_construction_ms: self.cpu_indirect_construction_ms.summary(),
            cpu_sun_shadow_preparation_ms: self.cpu_sun_shadow_preparation_ms.summary(),
            upload_preparation_ms: self.upload_preparation_ms.summary(),
            queue_upload_ms: self.queue_upload_ms.summary(),
            gpu_ms: self.gpu_ms.summary(),
            gpu_shadow_ms: self.gpu_shadow_ms.summary(),
            gpu_world_ms: self.gpu_world_ms.summary(),
            gpu_flora_ms: self.gpu_flora_ms.summary(),
            gpu_atmosphere_ms: self.gpu_atmosphere_ms.summary(),
            gpu_cloud_ms: self.gpu_cloud_ms.summary(),
            gpu_precipitation_ms: self.gpu_precipitation_ms.summary(),
            gpu_tonemap_ms: self.gpu_tonemap_ms.summary(),
            completion_backlog: self.completion_backlog.summary(),
            oldest_ready_ms: self.oldest_ready_ms.summary(),
            wanted_presented_delta: self.wanted_presented_delta.summary(),
            world_draw_batches: self.world_draw_batches.summary(),
            sun_shadow_caster_chunks: self.sun_shadow_caster_chunks.summary(),
            sun_shadow_caster_indices: self.sun_shadow_caster_indices.summary(),
            sun_shadow_batches: self.sun_shadow_batches.summary(),
            submitted_world_vertices: self.submitted_world_vertices.summary(),
            world_material_indices: self.world_material_indices,
            opaque_world_indices: self.opaque_world_indices.summary(),
            feedback_world_indices: self.feedback_world_indices.summary(),
            triangles: self.triangles.summary(),
            frame_upload_bytes: self.frame_upload_bytes.summary(),
            resident_gpu_bytes: self.resident_gpu_bytes.summary(),
            totals: self.totals,
        }
    }
}

impl Default for SmoothnessTelemetry {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn material_census_keeps_coherent_snapshots_without_counting_disabled_frames() {
        use super::{FrameMeasurement, MaterialIndexCounts, SmoothnessTelemetry};
        let mut telemetry = SmoothnessTelemetry::new(2);
        let mut first = MaterialIndexCounts::default();
        first.0[15] = 30;
        let mut latest = MaterialIndexCounts::default();
        latest.0[12] = 21;
        for counts in [Some(first), None, Some(latest)] {
            telemetry.record_frame(FrameMeasurement {
                world_material_indices: counts,
                ..Default::default()
            });
        }
        let report = telemetry.report().world_material_indices;
        assert_eq!(report.samples, 2);
        assert_eq!(report.latest, Some(latest));
        assert_eq!(report.peak_frame, Some(first));
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(json["latest"].as_array().unwrap().len(), 256);
        assert_eq!(json["latest"][12], 21);
        assert_eq!(json["peak_frame"][15], 30);
    }

    use super::*;

    #[test]
    fn rolling_window_reports_tail_percentiles() {
        let mut histogram = RollingHistogram::new(100);
        for value in 1..=100 {
            histogram.record(value as f64);
        }
        let s = histogram.summary();
        assert_eq!(s.count, 100);
        assert_eq!(s.p50, 51.0);
        assert_eq!(s.p95, 96.0);
        assert_eq!(s.p99, 100.0);
        assert_eq!(s.max, 100.0);
    }

    #[test]
    fn rolling_window_discards_old_samples_without_growth() {
        let mut histogram = RollingHistogram::new(3);
        for value in 0..10 {
            histogram.record(value as f64);
        }
        let s = histogram.summary();
        assert_eq!(s.count, 3);
        assert_eq!(s.p50, 8.0);
        assert_eq!(s.max, 9.0);
    }

    #[test]
    fn invalid_samples_do_not_poison_reports() {
        let mut histogram = RollingHistogram::new(3);
        histogram.record(f64::NAN);
        histogram.record(-1.0);
        histogram.record(2.0);
        assert_eq!(histogram.summary().count, 1);
    }

    #[test]
    fn optional_gpu_pass_samples_are_reported_independently() {
        let mut telemetry = SmoothnessTelemetry::new(4);
        telemetry.record_frame(FrameMeasurement {
            gpu_ms: Some(3.5),
            gpu_shadow_ms: Some(0.35),
            gpu_world_ms: Some(3.0),
            gpu_flora_ms: Some(0.2),
            gpu_atmosphere_ms: Some(0.4),
            gpu_cloud_ms: Some(0.25),
            gpu_precipitation_ms: Some(0.15),
            gpu_tonemap_ms: Some(0.1),
            ..FrameMeasurement::default()
        });
        telemetry.record_frame(FrameMeasurement::default());

        let report = telemetry.report();
        assert_eq!(report.gpu_ms.count, 1);
        assert_eq!(report.gpu_shadow_ms.p50, 0.35);
        assert_eq!(report.gpu_world_ms.p50, 3.0);
        assert_eq!(report.gpu_flora_ms.p50, 0.2);
        assert_eq!(report.gpu_atmosphere_ms.p50, 0.4);
        assert_eq!(report.gpu_cloud_ms.p50, 0.25);
        assert_eq!(report.gpu_precipitation_ms.p50, 0.15);
        assert_eq!(report.gpu_tonemap_ms.p50, 0.1);
        assert_eq!(report.totals.frames, 2);
    }

    #[test]
    fn render_work_breakdown_is_reported() {
        let mut telemetry = SmoothnessTelemetry::new(4);
        telemetry.record_frame(FrameMeasurement {
            transition_ms: 1.25,
            residency_ms: 2.5,
            eviction_ms: 3.75,
            cpu_visibility_ms: 0.11,
            cpu_world_draw_construction_ms: 0.115,
            cpu_sorting_ms: 0.12,
            cpu_uniform_construction_ms: 0.13,
            cpu_grouping_ms: 0.14,
            cpu_indirect_construction_ms: 0.15,
            upload_preparation_ms: 0.16,
            queue_upload_ms: 0.17,
            world_draw_batches: 9,
            submitted_world_vertices: 100_000,
            opaque_world_indices: 30_000,
            feedback_world_indices: 7_035,
            triangles: 12_345,
            frame_upload_bytes: 4_096,
            resident_gpu_bytes: 8_388_608,
            ..FrameMeasurement::default()
        });

        let report = telemetry.report();
        assert_eq!(report.transition_ms.p50, 1.25);
        assert_eq!(report.residency_ms.p50, 2.5);
        assert_eq!(report.eviction_ms.p50, 3.75);
        assert_eq!(report.cpu_visibility_ms.p50, 0.11);
        assert_eq!(report.cpu_world_draw_construction_ms.p50, 0.115);
        assert_eq!(report.cpu_sorting_ms.p50, 0.12);
        assert_eq!(report.cpu_uniform_construction_ms.p50, 0.13);
        assert_eq!(report.cpu_grouping_ms.p50, 0.14);
        assert_eq!(report.cpu_indirect_construction_ms.p50, 0.15);
        assert_eq!(report.upload_preparation_ms.p50, 0.16);
        assert_eq!(report.queue_upload_ms.p50, 0.17);
        assert_eq!(report.world_draw_batches.p50, 9.0);
        assert_eq!(report.submitted_world_vertices.p50, 100_000.0);
        assert_eq!(report.opaque_world_indices.p50, 30_000.0);
        assert_eq!(report.feedback_world_indices.p50, 7_035.0);
        assert_eq!(report.triangles.p50, 12_345.0);
        assert_eq!(report.frame_upload_bytes.p50, 4_096.0);
        assert_eq!(report.resident_gpu_bytes.p50, 8_388_608.0);
    }
}
