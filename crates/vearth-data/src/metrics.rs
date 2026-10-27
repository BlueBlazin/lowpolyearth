//! Process-wide data acquisition telemetry.
//!
//! `snapshot()` exposes aggregate `(requests, bytes, errors)` HTTP counters.
//! `acquisition_snapshots()` provides per-provider detail:
//! cache behavior, queue and in-flight gauges, latency, bytes, and terminal
//! outcomes, keyed by provider/operation name.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use vearth_platform::time::{Duration, Instant};

pub static HTTP_REQS: AtomicU64 = AtomicU64::new(0);
pub static HTTP_BYTES: AtomicU64 = AtomicU64::new(0);
pub static HTTP_ERRS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn record(bytes: usize) {
    HTTP_REQS.fetch_add(1, Ordering::Relaxed);
    HTTP_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

pub(crate) fn record_err() {
    HTTP_REQS.fetch_add(1, Ordering::Relaxed);
    HTTP_ERRS.fetch_add(1, Ordering::Relaxed);
}

/// Backwards-compatible `(requests, bytes, errors)` totals used by the HUD.
pub fn snapshot() -> (u64, u64, u64) {
    (
        HTTP_REQS.load(Ordering::Relaxed),
        HTTP_BYTES.load(Ordering::Relaxed),
        HTTP_ERRS.load(Ordering::Relaxed),
    )
}

/// Cell-level buckets; the final bucket also accepts future levels above 31.
pub const COMPLETENESS_LEVELS: usize = 32;
pub const ACQUISITION_COMPLETENESS_STAGES: [&str; 6] = [
    "completed",
    "incomplete",
    "base",
    "features",
    "geometry",
    "roof",
];
pub const REQUIRED_QUERY_CLASSES: [&str; 3] =
    ["terrain_detail", "terrain_landcover", "night_buildings"];

#[derive(Clone, Copy)]
pub(crate) enum RequiredQueryClass {
    TerrainDetail,
    TerrainLandcover,
    NightBuildings,
}

/// Opt-in bounded counters. Acquisition stages count completed cell plans;
/// query caps count planner evaluations by source zoom, which can repeat
/// during preparation and consumption of the same cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceCompletenessSnapshot {
    pub acquisition_by_stage: [[u64; COMPLETENESS_LEVELS]; 6],
    pub required_query_caps: [[u64; COMPLETENESS_LEVELS]; 3],
}

struct SourceCompletenessCounters {
    enabled: AtomicBool,
    acquisition: [[AtomicU64; COMPLETENESS_LEVELS]; 6],
    query_caps: [[AtomicU64; COMPLETENESS_LEVELS]; 3],
}

impl SourceCompletenessCounters {
    const fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            acquisition: [const { [const { AtomicU64::new(0) }; COMPLETENESS_LEVELS] }; 6],
            query_caps: [const { [const { AtomicU64::new(0) }; COMPLETENESS_LEVELS] }; 3],
        }
    }

    fn record_preparation(&self, level: u8, stages_complete: [bool; 4]) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let level = usize::from(level).min(COMPLETENESS_LEVELS - 1);
        self.acquisition[0][level].fetch_add(1, Ordering::Relaxed);
        if stages_complete.iter().any(|complete| !complete) {
            self.acquisition[1][level].fetch_add(1, Ordering::Relaxed);
        }
        for (stage, complete) in stages_complete.into_iter().enumerate() {
            if !complete {
                self.acquisition[stage + 2][level].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn record_query_cap(&self, kind: RequiredQueryClass, zoom: u8) {
        if self.enabled.load(Ordering::Relaxed) {
            self.query_caps[kind as usize][usize::from(zoom).min(COMPLETENESS_LEVELS - 1)]
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn reset(&self) {
        for counter in self
            .acquisition
            .iter()
            .flatten()
            .chain(self.query_caps.iter().flatten())
        {
            counter.store(0, Ordering::Relaxed);
        }
    }

    fn snapshot(&self) -> SourceCompletenessSnapshot {
        SourceCompletenessSnapshot {
            acquisition_by_stage: std::array::from_fn(|stage| {
                std::array::from_fn(|level| self.acquisition[stage][level].load(Ordering::Relaxed))
            }),
            required_query_caps: std::array::from_fn(|kind| {
                std::array::from_fn(|zoom| self.query_caps[kind][zoom].load(Ordering::Relaxed))
            }),
        }
    }
}

static SOURCE_COMPLETENESS: SourceCompletenessCounters = SourceCompletenessCounters::new();

pub fn set_source_completeness_enabled(enabled: bool) {
    SOURCE_COMPLETENESS
        .enabled
        .store(enabled, Ordering::Relaxed);
}

/// Reset before starting workers; snapshots and increments otherwise remain
/// independent relaxed counters rather than a transaction across stages.
pub fn reset_source_completeness() {
    SOURCE_COMPLETENESS.reset();
}

pub fn source_completeness_snapshot() -> SourceCompletenessSnapshot {
    SOURCE_COMPLETENESS.snapshot()
}

/// Record one finished asynchronous cell plan. Flags are base preparation,
/// feature projection, fixed-detail ground geometry, and measured roofs.
/// Absent optional stages count as complete; canceled plans are not recorded.
pub fn record_preparation_completeness(level: u8, stages_complete: [bool; 4]) {
    SOURCE_COMPLETENESS.record_preparation(level, stages_complete);
}

pub(crate) fn record_required_query_cap(kind: RequiredQueryClass, zoom: u8) {
    SOURCE_COMPLETENESS.record_query_cap(kind, zoom);
}

#[derive(Default)]
struct Counters {
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    cache_bytes: AtomicU64,
    shared_waits: AtomicU64,
    shared_wait_us: AtomicU64,
    shared_wait_us_max: AtomicU64,
    queued: AtomicU64,
    queued_peak: AtomicU64,
    queue_us: AtomicU64,
    queue_us_max: AtomicU64,
    in_flight: AtomicU64,
    in_flight_peak: AtomicU64,
    successes: AtomicU64,
    not_found: AtomicU64,
    errors: AtomicU64,
    canceled: AtomicU64,
    bytes: AtomicU64,
    latency_us: AtomicU64,
    latency_us_max: AtomicU64,
}

static ACQUISITIONS: OnceLock<Mutex<HashMap<String, Arc<Counters>>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Arc<Counters>>> {
    ACQUISITIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}

fn add_duration(total: &AtomicU64, max: &AtomicU64, duration: Duration) {
    let us = duration_us(duration);
    total.fetch_add(us, Ordering::Relaxed);
    max.fetch_max(us, Ordering::Relaxed);
}

fn increment_gauge(current: &AtomicU64, peak: &AtomicU64) {
    let value = current.fetch_add(1, Ordering::Relaxed) + 1;
    peak.fetch_max(value, Ordering::Relaxed);
}

/// Reusable handle for one provider/operation metric series.
///
/// Construct this once in a provider, then clone it freely. All event methods
/// use atomics and do not acquire the global registry lock.
#[derive(Clone)]
pub struct Acquisition {
    name: Arc<str>,
    counters: Arc<Counters>,
}

/// Look up or create a process-wide metric series.
pub fn acquisition(name: impl Into<String>) -> Acquisition {
    let name = name.into();
    let counters = {
        let mut registry = registry().lock().unwrap();
        registry.entry(name.clone()).or_default().clone()
    };
    Acquisition {
        name: Arc::from(name),
        counters,
    }
}

/// Stable authority-level series for an HTTP transport. URLs themselves are
/// never retained, so query strings and tile keys cannot explode cardinality.
pub(crate) fn http_acquisition(kind: &str, url: &str) -> Acquisition {
    let authority = url
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
        .filter(|authority| !authority.is_empty())
        .unwrap_or("unknown");
    acquisition(format!("http.{kind}:{authority}"))
}

impl Acquisition {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Record a cache hit and the bytes served from that cache, if known.
    pub fn cache_hit(&self, bytes: usize) {
        self.counters.cache_hits.fetch_add(1, Ordering::Relaxed);
        self.counters
            .cache_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn cache_miss(&self) {
        self.counters.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a caller that waited on another caller's keyed single-flight.
    pub fn shared_wait(&self, duration: Duration) {
        self.counters.shared_waits.fetch_add(1, Ordering::Relaxed);
        add_duration(
            &self.counters.shared_wait_us,
            &self.counters.shared_wait_us_max,
            duration,
        );
    }

    /// Enter the acquisition queue. Calling [`Queued::start`] transfers the
    /// guard into the in-flight state. Dropping it first records cancellation.
    pub fn enqueue(&self) -> Queued {
        increment_gauge(&self.counters.queued, &self.counters.queued_peak);
        Queued {
            counters: self.counters.clone(),
            enqueued_at: Instant::now(),
            active: true,
        }
    }

    pub fn snapshot(&self) -> AcquisitionSnapshot {
        snapshot_one(self.name.to_string(), &self.counters)
    }
}

/// RAII queue gauge for an acquisition that has not begun I/O yet.
pub struct Queued {
    counters: Arc<Counters>,
    enqueued_at: Instant,
    active: bool,
}

impl Queued {
    pub fn start(mut self) -> InFlight {
        self.active = false;
        self.counters.queued.fetch_sub(1, Ordering::Relaxed);
        add_duration(
            &self.counters.queue_us,
            &self.counters.queue_us_max,
            self.enqueued_at.elapsed(),
        );
        increment_gauge(&self.counters.in_flight, &self.counters.in_flight_peak);
        InFlight {
            counters: self.counters.clone(),
            started_at: Instant::now(),
            finished: false,
        }
    }
}

impl Drop for Queued {
    fn drop(&mut self) {
        if self.active {
            self.counters.queued.fetch_sub(1, Ordering::Relaxed);
            self.counters.canceled.fetch_add(1, Ordering::Relaxed);
            add_duration(
                &self.counters.queue_us,
                &self.counters.queue_us_max,
                self.enqueued_at.elapsed(),
            );
        }
    }
}

/// RAII in-flight gauge. Dropping without an explicit terminal outcome records
/// cancellation, which makes cancellation-safe async wiring observable.
pub struct InFlight {
    counters: Arc<Counters>,
    started_at: Instant,
    finished: bool,
}

impl InFlight {
    pub fn success(mut self, bytes: usize) {
        self.counters.successes.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
        self.finish();
    }

    pub fn not_found(mut self) {
        self.counters.not_found.fetch_add(1, Ordering::Relaxed);
        self.finish();
    }

    pub fn error(mut self) {
        self.counters.errors.fetch_add(1, Ordering::Relaxed);
        self.finish();
    }

    fn finish(&mut self) {
        self.finished = true;
        self.counters.in_flight.fetch_sub(1, Ordering::Relaxed);
        add_duration(
            &self.counters.latency_us,
            &self.counters.latency_us_max,
            self.started_at.elapsed(),
        );
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        if !self.finished {
            self.counters.in_flight.fetch_sub(1, Ordering::Relaxed);
            self.counters.canceled.fetch_add(1, Ordering::Relaxed);
            add_duration(
                &self.counters.latency_us,
                &self.counters.latency_us_max,
                self.started_at.elapsed(),
            );
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcquisitionSnapshot {
    pub name: String,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_bytes: u64,
    pub shared_waits: u64,
    pub shared_wait_us: u64,
    pub shared_wait_us_max: u64,
    pub queued: u64,
    pub queued_peak: u64,
    pub queue_us: u64,
    pub queue_us_max: u64,
    pub in_flight: u64,
    pub in_flight_peak: u64,
    pub successes: u64,
    pub not_found: u64,
    pub errors: u64,
    pub canceled: u64,
    pub bytes: u64,
    pub latency_us: u64,
    pub latency_us_max: u64,
}

fn snapshot_one(name: String, c: &Counters) -> AcquisitionSnapshot {
    let load = |v: &AtomicU64| v.load(Ordering::Relaxed);
    AcquisitionSnapshot {
        name,
        cache_hits: load(&c.cache_hits),
        cache_misses: load(&c.cache_misses),
        cache_bytes: load(&c.cache_bytes),
        shared_waits: load(&c.shared_waits),
        shared_wait_us: load(&c.shared_wait_us),
        shared_wait_us_max: load(&c.shared_wait_us_max),
        queued: load(&c.queued),
        queued_peak: load(&c.queued_peak),
        queue_us: load(&c.queue_us),
        queue_us_max: load(&c.queue_us_max),
        in_flight: load(&c.in_flight),
        in_flight_peak: load(&c.in_flight_peak),
        successes: load(&c.successes),
        not_found: load(&c.not_found),
        errors: load(&c.errors),
        canceled: load(&c.canceled),
        bytes: load(&c.bytes),
        latency_us: load(&c.latency_us),
        latency_us_max: load(&c.latency_us_max),
    }
}

/// Snapshot every provider/operation series, sorted by name for stable logs.
pub fn acquisition_snapshots() -> Vec<AcquisitionSnapshot> {
    let registry = registry().lock().unwrap();
    let mut out: Vec<_> = registry
        .iter()
        .map(|(name, counters)| snapshot_one(name.clone(), counters))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completeness_diagnostics_are_opt_in_bounded_and_count_overlapping_causes() {
        let counters = SourceCompletenessCounters::new();
        counters.record_preparation(14, [false; 4]);
        counters.record_query_cap(RequiredQueryClass::NightBuildings, 15);
        assert_eq!(counters.snapshot(), SourceCompletenessSnapshot::default());
        counters.enabled.store(true, Ordering::Relaxed);
        counters.record_preparation(14, [true; 4]);
        counters.record_preparation(14, [false, true, false, true]);
        counters.record_preparation(17, [true, false, true, false]);
        counters.record_preparation(255, [false; 4]);
        for kind in [
            RequiredQueryClass::TerrainDetail,
            RequiredQueryClass::TerrainLandcover,
            RequiredQueryClass::NightBuildings,
        ] {
            counters.record_query_cap(kind, 15);
        }
        counters.record_query_cap(RequiredQueryClass::NightBuildings, 255);
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.acquisition_by_stage[0][14], 2);
        assert_eq!(snapshot.acquisition_by_stage[1][14], 1);
        assert_eq!(snapshot.acquisition_by_stage[2][14], 1);
        assert_eq!(snapshot.acquisition_by_stage[3][14], 0);
        assert_eq!(snapshot.acquisition_by_stage[4][14], 1);
        assert_eq!(snapshot.acquisition_by_stage[5][14], 0);
        assert_eq!(snapshot.acquisition_by_stage[3][17], 1);
        assert_eq!(snapshot.acquisition_by_stage[5][17], 1);
        assert_eq!(snapshot.acquisition_by_stage[0][31], 1);
        assert!(
            snapshot
                .required_query_caps
                .iter()
                .all(|kind| kind[15] == 1)
        );
        assert_eq!(snapshot.required_query_caps[2][31], 1);
        counters.enabled.store(false, Ordering::Relaxed);
        counters.record_preparation(14, [false; 4]);
        assert_eq!(counters.snapshot(), snapshot);
        counters.reset();
        assert_eq!(counters.snapshot(), SourceCompletenessSnapshot::default());
    }

    #[test]
    fn lifecycle_tracks_cache_queue_inflight_and_outcomes() {
        let series = acquisition(format!(
            "metrics-test-{}-{:?}",
            std::process::id(),
            vearth_platform::thread::current().id()
        ));
        series.cache_hit(12);
        series.cache_miss();
        series.shared_wait(Duration::from_micros(7));
        series.enqueue().start().success(34);
        series.enqueue().start().not_found();
        series.enqueue().start().error();
        drop(series.enqueue());
        drop(series.enqueue().start());

        let s = series.snapshot();
        assert_eq!((s.cache_hits, s.cache_misses, s.cache_bytes), (1, 1, 12));
        assert_eq!(s.shared_waits, 1);
        assert_eq!(
            (s.successes, s.not_found, s.errors, s.canceled),
            (1, 1, 1, 2)
        );
        assert_eq!(s.bytes, 34);
        assert_eq!((s.queued, s.in_flight), (0, 0));
        assert_eq!((s.queued_peak, s.in_flight_peak), (1, 1));
    }
}
