//! Bounded, opt-in replay frame windows with asynchronous GPU sample attribution.

use crate::{
    replay::ReplayPose,
    telemetry::{FrameMeasurement, SmoothnessReport, SmoothnessTelemetry},
};
use serde::Serialize;
use std::collections::HashMap;
use vearth_gen::contracts::GenerationStageSnapshot;

const MAX_MEASURED_FRAMES: u64 = 4096;

/// Inclusive trace-frame window. The limit bounds retained CPU/GPU metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MeasurementRange {
    start: u64,
    end: u64,
}

impl MeasurementRange {
    pub fn parse(value: &str) -> Result<Self, String> {
        let (start, end) = value
            .split_once(':')
            .ok_or_else(|| "--replay-measure-frames expects START:END".to_string())?;
        let parse = |v: &str| {
            v.parse::<u64>().map_err(|_| {
                "--replay-measure-frames expects nonnegative integer frames".to_string()
            })
        };
        let (start, end) = (parse(start)?, parse(end)?);
        let count = end.checked_sub(start).and_then(|span| span.checked_add(1));
        if !count.is_some_and(|n| n <= MAX_MEASURED_FRAMES) {
            return Err(format!(
                "--replay-measure-frames must contain 1..={MAX_MEASURED_FRAMES} increasing frames"
            ));
        }
        Ok(Self { start, end })
    }

    pub fn validate_trace(self, last_frame: u64) -> Result<(), String> {
        if self.end > last_frame {
            return Err(format!(
                "measurement ends at frame {}, beyond trace end {last_frame}",
                self.end
            ));
        }
        Ok(())
    }

    pub(crate) fn contains(self, frame: u64) -> bool {
        self.start <= frame && frame <= self.end
    }

    fn len(self) -> usize {
        (self.end - self.start + 1) as usize
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct WorkState {
    pub desired_cells: usize,
    pub missing_desired: usize,
    pub nonopaque_desired: usize,
    pub active_transitions: usize,
    pub queued_cells: usize,
    pub running_cells: usize,
    pub ready_cells: usize,
}

impl WorkState {
    fn resident_no_cell_work(self) -> bool {
        self.desired_cells > 0
            && self.missing_desired == 0
            && self.nonopaque_desired == 0
            && self.active_transitions == 0
            && self.queued_cells == 0
            && self.running_cells == 0
            && self.ready_cells == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct WorkCounters {
    /// Worker admission/cache checks, including hits; not completed generation.
    pub started_cell_jobs: u64,
    pub generated_cache_hits: u64,
    pub network_requests: u64,
    pub network_bytes: u64,
    pub network_errors: u64,
}

impl WorkCounters {
    fn since(self, earlier: Self) -> Self {
        Self {
            started_cell_jobs: self
                .started_cell_jobs
                .saturating_sub(earlier.started_cell_jobs),
            generated_cache_hits: self
                .generated_cache_hits
                .saturating_sub(earlier.generated_cache_hits),
            network_requests: self
                .network_requests
                .saturating_sub(earlier.network_requests),
            network_bytes: self.network_bytes.saturating_sub(earlier.network_bytes),
            network_errors: self.network_errors.saturating_sub(earlier.network_errors),
        }
    }
}

pub(crate) struct FrameStart {
    frame: u64,
    elapsed_ms: f64,
    work: WorkState,
    counters: WorkCounters,
}

struct Boundary {
    frame: u64,
    work: WorkState,
    counters: WorkCounters,
    generation: Box<GenerationStageSnapshot>,
}

#[derive(Serialize)]
struct Row {
    frame: u64,
    elapsed_ms: f64,
    pose: ReplayPose,
    before: WorkState,
    after: WorkState,
    counters_before: WorkCounters,
    counters_after: WorkCounters,
    work_during_frame: WorkCounters,
    mesh_uploaded_cells: usize,
    mesh_uploaded_bytes: usize,
    resident_no_cell_work: bool,
    issued_gpu_id: Option<u64>,
    completed_gpu_id: Option<u64>,
    measurement: FrameMeasurement,
}

pub(crate) struct ReplayMeasurement {
    range: MeasurementRange,
    rows: Vec<Row>,
    gpu_rows: HashMap<u64, usize>,
    start: Option<Boundary>,
    end: Option<Boundary>,
    render_failures: usize,
}

impl ReplayMeasurement {
    pub fn new(range: MeasurementRange) -> Self {
        Self {
            range,
            rows: Vec::with_capacity(range.len()),
            gpu_rows: HashMap::with_capacity(range.len()),
            start: None,
            end: None,
            render_failures: 0,
        }
    }

    pub fn contains(&self, frame: u64) -> bool {
        self.range.contains(frame)
    }

    pub fn begin_frame(
        &mut self,
        frame: u64,
        elapsed_ms: f64,
        work: WorkState,
        counters: WorkCounters,
        snapshot: impl FnOnce() -> GenerationStageSnapshot,
    ) -> FrameStart {
        assert!(self.contains(frame));
        if self.start.is_none() {
            self.start = Some(Boundary {
                frame,
                work,
                counters,
                generation: Box::new(snapshot()),
            });
        }
        FrameStart {
            frame,
            elapsed_ms,
            work,
            counters,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn end_frame(
        &mut self,
        start: FrameStart,
        pose: ReplayPose,
        work: WorkState,
        counters: WorkCounters,
        uploaded_cells: usize,
        uploaded_bytes: usize,
        measured: Option<(FrameMeasurement, Option<u64>)>,
        snapshot: impl FnOnce() -> GenerationStageSnapshot,
    ) {
        assert_eq!(pose.frame, start.frame);
        if start.frame == self.range.end {
            self.end = Some(Boundary {
                frame: start.frame,
                work,
                counters,
                generation: Box::new(snapshot()),
            });
        }
        let Some((mut measurement, issued_gpu_id)) = measured else {
            self.render_failures += 1;
            return;
        };
        assert!(self.rows.len() < self.range.len());
        assert!(self.rows.last().is_none_or(|row| row.frame < start.frame));
        // The FrameStats GPU result belongs to an earlier issued slot, never
        // assume it describes the current CPU/workload sample.
        clear_gpu(&mut measurement);
        let work_during_frame = counters.since(start.counters);
        let resident_no_cell_work = start.work.resident_no_cell_work()
            && work.resident_no_cell_work()
            && uploaded_cells == 0
            && uploaded_bytes == 0
            && work_during_frame == WorkCounters::default();
        if let Some(id) = issued_gpu_id {
            assert!(self.gpu_rows.insert(id, self.rows.len()).is_none());
        }
        self.rows.push(Row {
            frame: start.frame,
            elapsed_ms: start.elapsed_ms,
            pose,
            before: start.work,
            after: work,
            counters_before: start.counters,
            counters_after: counters,
            work_during_frame,
            mesh_uploaded_cells: uploaded_cells,
            mesh_uploaded_bytes: uploaded_bytes,
            resident_no_cell_work,
            issued_gpu_id,
            completed_gpu_id: None,
            measurement,
        });
    }

    pub fn observe_gpu(&mut self, id: Option<u64>, sample: FrameMeasurement) {
        let Some(id) = id.filter(|_| sample.gpu_ms.is_some()) else {
            return;
        };
        let Some(&index) = self.gpu_rows.get(&id) else {
            return;
        };
        let row = &mut self.rows[index];
        if row.completed_gpu_id.is_some() {
            return;
        }
        row.completed_gpu_id = Some(id);
        row.measurement.gpu_ms = sample.gpu_ms;
        row.measurement.gpu_shadow_ms = sample.gpu_shadow_ms;
        row.measurement.gpu_world_ms = sample.gpu_world_ms;
        row.measurement.gpu_flora_ms = sample.gpu_flora_ms;
        row.measurement.gpu_atmosphere_ms = sample.gpu_atmosphere_ms;
        row.measurement.gpu_cloud_ms = sample.gpu_cloud_ms;
        row.measurement.gpu_precipitation_ms = sample.gpu_precipitation_ms;
        row.measurement.gpu_tonemap_ms = sample.gpu_tonemap_ms;
    }

    pub fn report(
        &self,
        summarize: impl Fn(SmoothnessReport) -> serde_json::Value,
        generation: impl Fn(&GenerationStageSnapshot) -> serde_json::Value,
    ) -> serde_json::Value {
        let summary = |filter: Option<bool>| {
            let mut telemetry = SmoothnessTelemetry::new(self.range.len());
            for row in &self.rows {
                if filter.is_none_or(|resident| row.resident_no_cell_work == resident) {
                    telemetry.record_frame(row.measurement);
                    telemetry.totals.uploaded_cells += row.mesh_uploaded_cells as u64;
                    telemetry.totals.uploaded_bytes += row.mesh_uploaded_bytes as u64;
                }
            }
            summarize(telemetry.report())
        };
        let boundary = |value: &Option<Boundary>| {
            value.as_ref().map(|b| {
                serde_json::json!({
                    "frame": b.frame,
                    "work": b.work,
                    "counters": b.counters,
                    "generation": generation(&b.generation),
                })
            })
        };
        let issued = self
            .rows
            .iter()
            .filter(|r| r.issued_gpu_id.is_some())
            .count();
        let joined = self
            .rows
            .iter()
            .filter(|r| r.completed_gpu_id.is_some())
            .count();
        serde_json::json!({
            "range_inclusive": self.range,
            "expected_frames": self.range.len(),
            "recorded_frames": self.rows.len(),
            "render_failed_frames": self.render_failures,
            "window_complete": self.start.is_some() && self.end.is_some()
                && self.rows.len() == self.range.len(),
            "gpu_samples": {
                "issued": issued,
                "joined": joined,
                "missing": issued - joined,
                "untimed_frames": self.rows.len() - issued,
            },
            "before_window": boundary(&self.start),
            "after_window": boundary(&self.end),
            "all": summary(None),
            "resident_no_cell_work": summary(Some(true)),
            "streaming_or_transition": summary(Some(false)),
            "frames": self.rows,
        })
    }
}

fn clear_gpu(frame: &mut FrameMeasurement) {
    frame.gpu_ms = None;
    frame.gpu_shadow_ms = None;
    frame.gpu_world_ms = None;
    frame.gpu_flora_ms = None;
    frame.gpu_atmosphere_ms = None;
    frame.gpu_cloud_ms = None;
    frame.gpu_precipitation_ms = None;
    frame.gpu_tonemap_ms = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> WorkState {
        WorkState {
            desired_cells: 100,
            ..WorkState::default()
        }
    }

    fn pose(frame: u64) -> ReplayPose {
        ReplayPose {
            frame,
            lat_deg: 1.0,
            lon_deg: 2.0,
            alt_m: 250.0,
            heading_deg: 90.0,
            pitch_deg: 55.0,
        }
    }

    fn add(
        measurement: &mut ReplayMeasurement,
        frame: u64,
        issued: Option<u64>,
        before: WorkState,
        after: WorkState,
        counters: WorkCounters,
        uploads: (usize, usize),
    ) {
        let start = measurement.begin_frame(
            frame,
            frame as f64 * 20.0,
            before,
            WorkCounters::default(),
            GenerationStageSnapshot::default,
        );
        measurement.end_frame(
            start,
            pose(frame),
            after,
            counters,
            uploads.0,
            uploads.1,
            Some((
                FrameMeasurement {
                    main_ms: frame as f64,
                    gpu_ms: Some(999.0),
                    triangles: frame * 100,
                    opaque_world_indices: frame * 200,
                    feedback_world_indices: frame * 100,
                    ..FrameMeasurement::default()
                },
                issued,
            )),
            GenerationStageSnapshot::default,
        );
    }

    fn report(measurement: &ReplayMeasurement) -> serde_json::Value {
        measurement.report(
            |s| {
                serde_json::json!({
                    "frames": s.totals.frames,
                    "gpu_count": s.gpu_ms.count,
                    "gpu_max": s.gpu_ms.max,
                    "main_max": s.main_ms.max,
                    "uploaded_bytes": s.totals.uploaded_bytes,
                })
            },
            |g| serde_json::json!({"samples": g.samples}),
        )
    }

    #[test]
    fn measurement_range_is_explicit_bounded_and_inside_trace() {
        let range = MeasurementRange::parse("1801:2280").unwrap();
        assert_eq!(range.len(), 480);
        assert!(!range.contains(1800));
        assert!(range.contains(1801));
        assert!(range.contains(2280));
        assert!(!range.contains(2281));
        assert!(range.validate_trace(2280).is_ok());
        assert!(range.validate_trace(2279).is_err());
        assert!(MeasurementRange::parse("0:4095").is_ok());
        for invalid in [
            "",
            "1",
            "-1:2",
            "2:1",
            "0:4096",
            "1:2:3",
            "a:2",
            "0:18446744073709551615",
        ] {
            assert!(MeasurementRange::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn measurement_window_excludes_holds_and_captures_only_two_generation_snapshots() {
        let mut measurement = ReplayMeasurement::new(MeasurementRange::parse("3:5").unwrap());
        let snapshots = std::cell::Cell::new(0);
        let snapshot = || {
            snapshots.set(snapshots.get() + 1);
            GenerationStageSnapshot {
                samples: snapshots.get(),
                ..GenerationStageSnapshot::default()
            }
        };
        for frame in 0..9 {
            if !measurement.contains(frame) {
                continue;
            }
            let start = measurement.begin_frame(
                frame,
                20.0 * frame as f64,
                idle(),
                WorkCounters::default(),
                snapshot,
            );
            measurement.end_frame(
                start,
                pose(frame),
                idle(),
                WorkCounters::default(),
                0,
                0,
                Some((
                    FrameMeasurement {
                        main_ms: frame as f64,
                        ..FrameMeasurement::default()
                    },
                    Some(frame + 100),
                )),
                snapshot,
            );
        }
        assert_eq!(snapshots.get(), 2);
        assert_eq!(
            measurement.rows.iter().map(|r| r.frame).collect::<Vec<_>>(),
            [3, 4, 5]
        );
        // This completion can arrive in the post-window hold. It still belongs
        // to trace frame3, independent of the current CPU/replay frame.
        measurement.observe_gpu(
            Some(103),
            FrameMeasurement {
                gpu_ms: Some(31.0),
                ..FrameMeasurement::default()
            },
        );
        let result = report(&measurement);
        assert_eq!(result["window_complete"], true);
        assert_eq!(result["all"]["frames"], 3);
        assert_eq!(result["all"]["main_max"], 5.0);
        assert_eq!(result["before_window"]["generation"]["samples"], 1);
        assert_eq!(result["after_window"]["generation"]["samples"], 2);
        assert_eq!(result["frames"][0]["measurement"]["gpu_ms"], 31.0);
        assert_eq!(
            result["frames"][1]["measurement"]["gpu_ms"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn measurement_gpu_ids_join_delayed_repeated_skipped_and_out_of_order_results_once() {
        let mut measurement = ReplayMeasurement::new(MeasurementRange::parse("30:33").unwrap());
        for (frame, id) in [(30, Some(10)), (31, None), (32, Some(12)), (33, Some(13))] {
            add(
                &mut measurement,
                frame,
                id,
                idle(),
                idle(),
                WorkCounters::default(),
                (0, 0),
            );
        }
        for (id, value) in [(12, 12.0), (10, 10.0), (12, 999.0), (99, 99.0), (10, 888.0)] {
            measurement.observe_gpu(
                Some(id),
                FrameMeasurement {
                    gpu_ms: Some(value),
                    gpu_shadow_ms: Some(value / 2.0),
                    ..FrameMeasurement::default()
                },
            );
        }
        measurement.observe_gpu(Some(13), FrameMeasurement::default());
        let result = report(&measurement);
        assert_eq!(
            result["gpu_samples"],
            serde_json::json!({"issued":3,"joined":2,"missing":1,"untimed_frames":1})
        );
        assert_eq!(result["all"]["gpu_count"], 2);
        assert_eq!(result["all"]["gpu_max"], 12.0);
        assert_eq!(result["frames"][0]["measurement"]["gpu_ms"], 10.0);
        assert_eq!(result["frames"][2]["measurement"]["gpu_shadow_ms"], 6.0);
        for (index, frame) in [30, 31, 32, 33].into_iter().enumerate() {
            let measured = &result["frames"][index]["measurement"];
            assert_eq!(measured["opaque_world_indices"], frame * 200);
            assert_eq!(measured["feedback_world_indices"], frame * 100);
        }
        for index in [1, 3] {
            assert_eq!(
                result["frames"][index]["measurement"]["gpu_ms"],
                serde_json::Value::Null
            );
        }
    }

    #[test]
    fn measurement_resident_classification_requires_idle_boundaries_and_no_work() {
        let mut measurement = ReplayMeasurement::new(MeasurementRange::parse("0:6").unwrap());
        add(
            &mut measurement,
            0,
            None,
            idle(),
            idle(),
            WorkCounters::default(),
            (0, 0),
        );
        add(
            &mut measurement,
            1,
            None,
            WorkState {
                running_cells: 1,
                ..idle()
            },
            idle(),
            WorkCounters::default(),
            (0, 0),
        );
        add(
            &mut measurement,
            2,
            None,
            idle(),
            WorkState {
                nonopaque_desired: 1,
                ..idle()
            },
            WorkCounters::default(),
            (0, 0),
        );
        add(
            &mut measurement,
            3,
            None,
            idle(),
            idle(),
            WorkCounters::default(),
            (1, 50),
        );
        add(
            &mut measurement,
            4,
            None,
            idle(),
            idle(),
            WorkCounters {
                started_cell_jobs: 1,
                ..WorkCounters::default()
            },
            (0, 0),
        );
        add(
            &mut measurement,
            5,
            None,
            idle(),
            idle(),
            WorkCounters {
                network_bytes: 20,
                ..WorkCounters::default()
            },
            (0, 0),
        );
        add(
            &mut measurement,
            6,
            None,
            idle(),
            WorkState {
                active_transitions: 1,
                ..idle()
            },
            WorkCounters::default(),
            (0, 0),
        );
        let result = report(&measurement);
        assert_eq!(result["resident_no_cell_work"]["frames"], 1);
        assert_eq!(result["streaming_or_transition"]["frames"], 6);
        assert_eq!(result["all"]["uploaded_bytes"], 50);
        assert!(!WorkState::default().resident_no_cell_work());
        for state in [
            WorkState {
                missing_desired: 1,
                ..idle()
            },
            WorkState {
                ready_cells: 1,
                ..idle()
            },
            WorkState {
                queued_cells: 1,
                ..idle()
            },
        ] {
            assert!(!state.resident_no_cell_work());
        }
    }

    #[test]
    fn measurement_failed_render_remains_an_incomplete_window() {
        let mut measurement = ReplayMeasurement::new(MeasurementRange::parse("1:1").unwrap());
        let start = measurement.begin_frame(
            1,
            20.0,
            idle(),
            WorkCounters::default(),
            GenerationStageSnapshot::default,
        );
        measurement.end_frame(
            start,
            pose(1),
            idle(),
            WorkCounters::default(),
            0,
            0,
            None,
            GenerationStageSnapshot::default,
        );
        let result = report(&measurement);
        assert_eq!(result["recorded_frames"], 0);
        assert_eq!(result["render_failed_frames"], 1);
        assert_eq!(result["window_complete"], false);
        assert_eq!(result["all"]["gpu_count"], 0);
    }
}
