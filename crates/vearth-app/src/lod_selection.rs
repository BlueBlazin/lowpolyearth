//! Asynchronous LOD selection for the interactive viewer.
//!
//! The render thread submits immutable camera snapshots. A single worker
//! owns a mirror of the terrain evidence and separate residency used by the selector,
//! so submitting work never clones the resident index. Both queued requests
//! and completed results are replaceable: a slow selection cannot build a
//! backlog of obsolete camera poses.

use crate::streaming::{
    LodView, TerrainMetricRef, WantedCell, WantedStats, compute_wanted,
    compute_wanted_for_view_with_stats_and_metric_lookup,
    compute_wanted_with_stats_and_metric_lookup,
};
use glam::DVec3;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, mpsc};
use vearth_core::cell::CellId;
use vearth_platform::thread::{self, JoinHandle};
use vearth_platform::time::{Duration, Instant};

const DEST_PREFETCH_CAP: usize = 384;
const PREDICT_PREFETCH_CAP: usize = 128;

#[derive(Clone, Copy, Debug)]
pub(crate) enum SelectionMode {
    Legacy,
    View(LodView),
}

pub(crate) struct SelectionRequest {
    pub cam_ecef: DVec3,
    pub viewport_h: f64,
    pub fov_y: f64,
    pub mode: SelectionMode,
    pub prev_split: HashSet<CellId>,
    pub predicted_cam: Option<DVec3>,
    pub destination_cam: Option<DVec3>,
}

pub(crate) struct Selection {
    pub wanted: Vec<WantedCell>,
    pub split: HashSet<CellId>,
    pub stats: WantedStats,
    pub predicted_prefetch: Vec<(CellId, f64)>,
    pub destination_prefetch: Vec<(CellId, f64)>,
    pub metric_dependencies: Vec<CellId>,
}

#[path = "terrain_metric_store.rs"]
mod terrain_metric_store;
pub(crate) use terrain_metric_store::{TerrainMetric, TerrainMetricStore};

pub(crate) fn compute_selection<'a>(
    request: SelectionRequest,
    metric_lookup: impl Fn(CellId) -> Option<TerrainMetricRef<'a>> + 'a,
    is_resident: impl Fn(CellId) -> bool,
) -> Selection {
    let metric = metric_lookup;
    let (mut wanted, split, stats) = match request.mode {
        SelectionMode::Legacy => compute_wanted_with_stats_and_metric_lookup(
            request.cam_ecef,
            request.viewport_h,
            request.fov_y,
            &request.prev_split,
            &metric,
        ),
        SelectionMode::View(view) => compute_wanted_for_view_with_stats_and_metric_lookup(
            request.cam_ecef,
            request.viewport_h,
            &view,
            &request.prev_split,
            &metric,
        ),
    };
    wanted.sort_by(WantedCell::stream_order);

    let predicted_prefetch = request.predicted_cam.map_or_else(Vec::new, |predicted| {
        let desired: HashSet<_> = wanted.iter().map(|wanted| wanted.cell).collect();
        let (predicted_wanted, _) = compute_wanted(
            predicted,
            request.viewport_h,
            request.fov_y,
            &HashSet::new(),
        );
        let mut candidates: Vec<_> = predicted_wanted
            .into_iter()
            .filter(|candidate| !desired.contains(&candidate.cell) && !is_resident(candidate.cell))
            .map(|candidate| (candidate.cell, candidate.screen_px * 0.35))
            .collect();
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        candidates.truncate(PREDICT_PREFETCH_CAP);
        candidates
    });

    let destination_prefetch = request
        .destination_cam
        .map_or_else(Vec::new, |destination| {
            let (destination_wanted, _) = compute_wanted(
                destination,
                request.viewport_h,
                request.fov_y,
                &HashSet::new(),
            );
            let mut candidates: Vec<_> = destination_wanted
                .into_iter()
                .filter(|candidate| !is_resident(candidate.cell))
                .map(|candidate| (candidate.cell, candidate.screen_px * 0.8))
                .collect();
            candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
            candidates.truncate(DEST_PREFETCH_CAP);
            candidates
        });

    Selection {
        wanted,
        split,
        stats,
        predicted_prefetch,
        destination_prefetch,
        metric_dependencies: Vec::new(),
    }
}

struct WorkRequest {
    generation: u64,
    id: u64,
    request: SelectionRequest,
}

pub(crate) struct CompletedSelection {
    generation: u64,
    pub id: u64,
    pub selection: Selection,
    pub compute_time: Duration,
}

enum Command {
    Upsert(Vec<(CellId, TerrainMetric, bool)>),
    Remove(Vec<CellId>),
    Invalidate {
        generation: u64,
        clear_metrics: bool,
    },
    Select(Box<WorkRequest>),
    Stop,
}

pub(crate) struct LodSelector {
    tx: mpsc::Sender<Command>,
    completed: Arc<Mutex<Option<CompletedSelection>>>,
    worker: Option<JoinHandle<()>>,
    generation: u64,
    next_id: u64,
}

impl LodSelector {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let completed = Arc::new(Mutex::new(None));
        let worker_completed = Arc::clone(&completed);
        let worker = thread::Builder::new()
            .name("vearth-lod".into())
            .spawn(move || worker_loop(rx, worker_completed))
            .expect("spawn LOD selection worker");
        Self {
            tx,
            completed,
            worker: Some(worker),
            generation: 0,
            next_id: 0,
        }
    }

    pub fn submit(&mut self, request: SelectionRequest) -> u64 {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        let _ = self.tx.send(Command::Select(Box::new(WorkRequest {
            generation: self.generation,
            id,
            request,
        })));
        id
    }

    pub fn take_latest(&self) -> Option<CompletedSelection> {
        let result = self.completed.lock().unwrap().take()?;
        (result.generation == self.generation).then_some(result)
    }

    pub fn upsert(&self, updates: Vec<(CellId, TerrainMetric, bool)>) {
        if !updates.is_empty() {
            let _ = self.tx.send(Command::Upsert(updates));
        }
    }

    pub fn remove(&self, cells: Vec<CellId>) {
        if !cells.is_empty() {
            let _ = self.tx.send(Command::Remove(cells));
        }
    }

    /// Reject in-flight work while retaining the resident metric mirror.
    pub fn invalidate(&mut self) {
        self.invalidate_inner(false);
    }

    /// Reject in-flight work and clear metrics after a world reset.
    pub fn reset(&mut self) {
        self.invalidate_inner(true);
    }

    fn invalidate_inner(&mut self, clear_metrics: bool) {
        self.generation = self.generation.wrapping_add(1);
        self.completed.lock().unwrap().take();
        let _ = self.tx.send(Command::Invalidate {
            generation: self.generation,
            clear_metrics,
        });
    }
}

impl Drop for LodSelector {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker_loop(rx: mpsc::Receiver<Command>, completed: Arc<Mutex<Option<CompletedSelection>>>) {
    let mut metrics = TerrainMetricStore::default();
    let mut generation = 0;

    loop {
        let Ok(first) = rx.recv() else {
            return;
        };
        let mut latest_request = None;
        if !apply_command(first, &mut generation, &mut metrics, &mut latest_request) {
            return;
        }
        for command in rx.try_iter() {
            if !apply_command(command, &mut generation, &mut metrics, &mut latest_request) {
                return;
            }
        }

        let Some(work) = latest_request else {
            continue;
        };
        if work.generation != generation {
            continue;
        }
        let started = Instant::now();
        metrics.begin_query();
        let mut selection = compute_selection(
            work.request,
            |cell| metrics.get(cell),
            |cell| metrics.is_resident(cell),
        );
        selection.metric_dependencies = metrics.finish_query();
        let result = CompletedSelection {
            generation: work.generation,
            id: work.id,
            selection,
            compute_time: started.elapsed(),
        };
        *completed.lock().unwrap() = Some(result);
    }
}

fn apply_command(
    command: Command,
    generation: &mut u64,
    metrics: &mut TerrainMetricStore,
    latest_request: &mut Option<WorkRequest>,
) -> bool {
    match command {
        Command::Upsert(updates) => {
            for (cell, metric, complete) in updates {
                metrics.upsert(cell, metric, complete);
            }
        }
        Command::Remove(cells) => {
            for cell in cells {
                metrics.remove_resident(cell);
            }
            metrics.prune();
        }
        Command::Invalidate {
            generation: next_generation,
            clear_metrics,
        } => {
            *generation = next_generation;
            *latest_request = None;
            if clear_metrics {
                metrics.clear();
            }
        }
        Command::Select(work) => {
            if work.generation == *generation {
                *latest_request = Some(*work);
            }
        }
        Command::Stop => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DMat4;
    use vearth_core::ellipsoid;
    use vearth_platform::thread;

    fn request(cam_ecef: DVec3) -> SelectionRequest {
        SelectionRequest {
            cam_ecef,
            viewport_h: 1080.0,
            fov_y: 45.0_f64.to_radians(),
            mode: SelectionMode::View(LodView::new(
                DMat4::IDENTITY,
                45.0_f64.to_radians(),
                16.0 / 9.0,
            )),
            prev_split: HashSet::new(),
            predicted_cam: None,
            destination_cam: None,
        }
    }

    #[test]
    fn metric_commands_keep_relief_independent_through_replacement_and_reset() {
        let cell = CellId::from_dir(DVec3::X, 4);
        let mut metrics = TerrainMetricStore::default();
        let mut generation = 0;
        let mut latest = None;
        for relief in [0.0, 700.0] {
            let metric = TerrainMetric {
                height_range: (-80_000.0, 1_000.0),
                terrain_step_m: 0.0,
                terrain_relief_m: relief,
                terrain_detail: None,
            };
            assert!(apply_command(
                Command::Upsert(vec![(cell, metric.clone(), true)]),
                &mut generation,
                &mut metrics,
                &mut latest,
            ));
            assert_eq!(metrics.get(cell), Some(metric.as_ref()));
        }
        assert!(apply_command(
            Command::Invalidate {
                generation: 1,
                clear_metrics: false
            },
            &mut generation,
            &mut metrics,
            &mut latest,
        ));
        assert_eq!(metrics.get(cell).unwrap().terrain_relief_m, 700.0);
        assert!(apply_command(
            Command::Invalidate {
                generation: 2,
                clear_metrics: true
            },
            &mut generation,
            &mut metrics,
            &mut latest,
        ));
        assert!(metrics.get(cell).is_none());
    }

    #[test]
    fn spatial_terrain_metric_commands_share_and_release_summaries() {
        let cell = CellId::from_dir(DVec3::X, 4);
        let detail = Arc::new(
            vearth_gen::TerrainDetailSummary::from_grid(&vearth_gen::TerrainDetailGrid {
                heights_m: Box::new([0.0; 33 * 33]),
                source_columns: [u32::MAX; 32],
            })
            .unwrap(),
        );
        let mut metrics = TerrainMetricStore::default();
        let mut generation = 0;
        let mut latest = None;
        let metric = TerrainMetric {
            height_range: (-50_000.0, 0.0),
            terrain_step_m: 0.0,
            terrain_relief_m: 0.0,
            terrain_detail: Some(detail.clone()),
        };
        apply_command(
            Command::Upsert(vec![(cell, metric, false)]),
            &mut generation,
            &mut metrics,
            &mut latest,
        );
        assert_eq!(Arc::strong_count(&detail), 2);
        for _ in 0..1_024 {
            let borrowed = metrics.get(cell).unwrap();
            assert!(std::ptr::eq(
                borrowed.terrain_detail.unwrap(),
                detail.as_ref()
            ));
        }
        assert_eq!(Arc::strong_count(&detail), 2);
        apply_command(
            Command::Invalidate {
                generation: 1,
                clear_metrics: false,
            },
            &mut generation,
            &mut metrics,
            &mut latest,
        );
        assert_eq!(Arc::strong_count(&detail), 2);
        apply_command(
            Command::Remove(vec![cell]),
            &mut generation,
            &mut metrics,
            &mut latest,
        );
        assert_eq!(Arc::strong_count(&detail), 1);
    }

    fn wait_for(selector: &LodSelector, id: u64) -> CompletedSelection {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = selector.take_latest()
                && result.id == id
            {
                return result;
            }
            assert!(Instant::now() < deadline, "LOD worker timed out");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn retained_metrics_worker_removal_keeps_cut_and_reenables_destination_prefetch() {
        let cam = ellipsoid::geodetic_to_ecef(0.4, -1.1, 250_000.0);
        let make_request = || {
            let mut value = request(cam);
            value.destination_cam = Some(cam);
            value
        };
        let original = compute_selection(make_request(), |_| None, |_| false);
        let source = original.destination_prefetch[0].0;
        let mut selector = LodSelector::new();
        selector.upsert(vec![(
            source,
            TerrainMetric {
                height_range: (-100.0, 100.0),
                terrain_step_m: 25.0,
                terrain_relief_m: 700.0,
                terrain_detail: None,
            },
            true,
        )]);
        let before_id = selector.submit(make_request());
        let before = wait_for(&selector, before_id).selection;
        assert!(
            before
                .destination_prefetch
                .iter()
                .all(|(cell, _)| *cell != source)
        );
        selector.remove(vec![source]);
        let after_id = selector.submit(make_request());
        let after = wait_for(&selector, after_id).selection;
        let signature = |selection: &Selection| {
            selection
                .wanted
                .iter()
                .map(|target| (target.cell, target.screen_px.to_bits(), target.lane))
                .collect::<Vec<_>>()
        };
        assert_eq!(signature(&after), signature(&before));
        assert_eq!(after.split, before.split);
        assert!(
            after
                .destination_prefetch
                .iter()
                .any(|(cell, _)| *cell == source),
            "retained evidence must not impersonate a resident mesh"
        );
        selector.reset();
        let reset_id = selector.submit(make_request());
        let reset = wait_for(&selector, reset_id).selection;
        assert_eq!(signature(&reset), signature(&original));
    }

    #[test]
    fn worker_matches_the_synchronous_selector() {
        let cam = ellipsoid::geodetic_to_ecef(0.4, -1.1, 250_000.0);
        let direct = compute_selection(request(cam), |_| None, |_| false);
        let mut selector = LodSelector::new();
        let id = selector.submit(request(cam));
        let threaded = wait_for(&selector, id).selection;

        let direct_signature: Vec<_> = direct
            .wanted
            .iter()
            .map(|wanted| (wanted.cell, wanted.screen_px.to_bits(), wanted.lane))
            .collect();
        let threaded_signature: Vec<_> = threaded
            .wanted
            .iter()
            .map(|wanted| (wanted.cell, wanted.screen_px.to_bits(), wanted.lane))
            .collect();
        assert_eq!(threaded_signature, direct_signature);
        assert_eq!(threaded.split, direct.split);
        assert_eq!(threaded.stats, direct.stats);
    }

    #[test]
    fn queued_camera_snapshots_collapse_to_the_latest() {
        let mut selector = LodSelector::new();
        let mut latest = 0;
        for altitude in [8_000_000.0, 2_000_000.0, 250_000.0, 25_000.0] {
            latest = selector.submit(request(ellipsoid::geodetic_to_ecef(0.4, -1.1, altitude)));
        }
        let result = wait_for(&selector, latest);
        assert_eq!(result.id, latest);
    }

    #[test]
    fn invalidation_rejects_an_old_generation() {
        let mut selector = LodSelector::new();
        selector.submit(request(ellipsoid::geodetic_to_ecef(0.4, -1.1, 25_000.0)));
        selector.invalidate();
        let latest = selector.submit(request(ellipsoid::geodetic_to_ecef(-0.2, 0.8, 100_000.0)));
        assert_eq!(wait_for(&selector, latest).id, latest);
    }
}
