//! Provider-level asynchronous preparation boundary.
//!
//! The deterministic generator remains synchronous. Before it runs, every
//! covering provider receives one conservative geodetic footprint and warms
//! the exact cache that its synchronous sampling implementation reads. An
//! async-backed provider must make an unexpected sampling miss cache-only: it
//! may mark the cell incomplete, but it must never start network I/O from the
//! CPU generation lane.

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

/// Conservative source footprint for one generated cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceFootprint {
    /// Geodetic center, radians.
    pub center_lat: f64,
    pub center_lon: f64,
    /// Angular radius on the mean Earth sphere, radians.
    pub radius_rad: f64,
    /// Sampling-resolution hint passed to source implementations, metres.
    pub sample_hint_m: f64,
}

/// Full provider preparation request for one cell revision.
#[derive(Clone, Debug, PartialEq)]
pub struct PrepareRequest {
    /// Exact legacy Terrarium prediction. Async Terrarium also covers the
    /// conservative footprint so water/road/landmark edge samples cannot
    /// escape into network I/O during generation.
    pub terrain_tiles: Vec<(u8, u32, u32)>,
    pub footprint: SourceFootprint,
    /// Existing deterministic vector feature window.
    pub vector_bounds: Option<(f64, f64, f64, f64)>,
    /// Optional fixed-detail building window for detached night-light proxies.
    /// Production cell plans currently omit it.
    pub night_building_bounds: Option<(f64, f64, f64, f64)>,
    /// Cell priority at admission time. Coordinators use larger values first.
    pub priority: f64,
}

/// Coverage assigned to one registered preparer. Provider planners use this
/// to clip a conservative cell footprint before issuing regional tile
/// requests; synchronous sampling already applies the same registry coverage
/// point-by-point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreparationCoverage {
    Global,
    /// Degrees, inclusive: latitude min/max, longitude min/max. A longitude
    /// minimum greater than its maximum denotes an antimeridian-wrapping box.
    LatLonBox(f64, f64, f64, f64),
}

impl PreparationCoverage {
    /// Whether a non-wrapping provider tile in degrees intersects this
    /// coverage. Touching boundaries count because generator samples can lie
    /// exactly on a tile/provider edge.
    pub fn intersects_tile(
        self,
        tile_lat0: f64,
        tile_lat1: f64,
        tile_lon0: f64,
        tile_lon1: f64,
    ) -> bool {
        match self {
            Self::Global => true,
            Self::LatLonBox(lat0, lat1, lon0, lon1) => {
                if tile_lat1 < lat0 || tile_lat0 > lat1 {
                    return false;
                }
                if lon0 <= lon1 {
                    tile_lon1 >= lon0 && tile_lon0 <= lon1
                } else {
                    tile_lon1 >= lon0 || tile_lon0 <= lon1
                }
            }
        }
    }
}

/// Terminal dependency state. These remain distinct because collapsing a
/// circuit-open or transient failure into genuine NoData can poison the
/// persistent generated-cell cache with degraded fallback output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependencyOutcome {
    Ready,
    NoData,
    TransientFailure,
    CircuitOpenCacheMiss,
}

/// Aggregate outcomes for a provider or a complete cell preparation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrepareSummary {
    pub ready: u32,
    pub no_data: u32,
    pub transient_failures: u32,
    pub circuit_open_misses: u32,
}

impl PrepareSummary {
    pub fn record(&mut self, outcome: DependencyOutcome) {
        match outcome {
            DependencyOutcome::Ready => self.ready += 1,
            DependencyOutcome::NoData => self.no_data += 1,
            DependencyOutcome::TransientFailure => self.transient_failures += 1,
            DependencyOutcome::CircuitOpenCacheMiss => self.circuit_open_misses += 1,
        }
    }

    pub fn combine(&mut self, other: Self) {
        self.ready += other.ready;
        self.no_data += other.no_data;
        self.transient_failures += other.transient_failures;
        self.circuit_open_misses += other.circuit_open_misses;
    }

    /// Genuine NoData is a complete provider answer. Only degraded or
    /// unavailable dependency states forbid persistent cell publication.
    pub fn complete(self) -> bool {
        self.transient_failures == 0 && self.circuit_open_misses == 0
    }
}

pub type PrepareFuture<'a> = Pin<Box<dyn Future<Output = PrepareSummary> + Send + 'a>>;

/// Prepared dependencies whose cache ownership lasts until their consumer ends.
#[derive(Default)]
pub struct PreparedSources {
    pub summary: PrepareSummary,
    retained: Vec<std::sync::Arc<dyn std::any::Any + Send + Sync>>,
}

impl PreparedSources {
    pub fn retain<T: std::any::Any + Send + Sync>(&mut self, source: std::sync::Arc<T>) {
        self.retained.push(source);
    }

    pub fn combine(&mut self, mut other: Self) {
        self.summary.combine(other.summary);
        self.retained.append(&mut other.retained);
    }
}

pub type RetainedPrepareFuture<'a> = Pin<Box<dyn Future<Output = PreparedSources> + Send + 'a>>;

/// Object-safe async cache warmer implemented by production providers.
pub trait AsyncSourcePreparer: Send + Sync {
    fn prepare<'a>(&'a self, request: &'a PrepareRequest) -> PrepareFuture<'a>;

    fn prepare_retained_for_coverage<'a>(
        &'a self,
        request: &'a PrepareRequest,
        coverage: PreparationCoverage,
    ) -> RetainedPrepareFuture<'a> {
        Box::pin(async move {
            PreparedSources {
                summary: self.prepare_for_coverage(request, coverage).await,
                ..PreparedSources::default()
            }
        })
    }

    /// Coverage-aware entry point used by the registry. Global providers and
    /// preparers whose own request already encodes coverage keep the default;
    /// regional raster planners override it to avoid fetching outside their
    /// registered bounds.
    fn prepare_for_coverage<'a>(
        &'a self,
        request: &'a PrepareRequest,
        _coverage: PreparationCoverage,
    ) -> PrepareFuture<'a> {
        self.prepare(request)
    }
}

pub async fn join_retained<'a>(futures: Vec<RetainedPrepareFuture<'a>>) -> PreparedSources {
    let mut futures: Vec<_> = futures.into_iter().map(Some).collect();
    let mut combined = PreparedSources::default();
    std::future::poll_fn(move |cx| {
        let mut pending = false;
        for future in &mut futures {
            let Some(active) = future.as_mut() else {
                continue;
            };
            match active.as_mut().poll(cx) {
                Poll::Ready(prepared) => {
                    combined.combine(prepared);
                    *future = None;
                }
                Poll::Pending => pending = true,
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(std::mem::take(&mut combined))
        }
    })
    .await
}

/// Await borrowed provider futures concurrently without another executor or
/// futures-crate dependency. The caller's Tokio acquisition task supplies the
/// polling context; provider HTTP futures run their sockets on the dedicated
/// transport reactor.
pub async fn join_summaries<'a>(futures: Vec<PrepareFuture<'a>>) -> PrepareSummary {
    let mut futures: Vec<_> = futures.into_iter().map(Some).collect();
    let mut results = vec![None; futures.len()];
    std::future::poll_fn(move |cx| {
        let mut pending = false;
        for (future, result) in futures.iter_mut().zip(results.iter_mut()) {
            let Some(active) = future.as_mut() else {
                continue;
            };
            match active.as_mut().poll(cx) {
                Poll::Ready(summary) => {
                    *result = Some(summary);
                    *future = None;
                }
                Poll::Pending => pending = true,
            }
        }
        if pending {
            Poll::Pending
        } else {
            let mut combined = PrepareSummary::default();
            for summary in results.iter().flatten() {
                combined.combine(*summary);
            }
            Poll::Ready(combined)
        }
    })
    .await
}

/// Publish cache bytes through a unique sibling and rename so concurrent
/// readers observe either the old complete file or the new complete file,
/// never a truncated write left by a canceled async waiter.
pub(crate) fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("vearth-cache");
    let temporary = path.with_file_name(format!(".{name}.tmp-{}-{sequence}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_preserves_failure_kinds() {
        let mut summary = PrepareSummary::default();
        summary.record(DependencyOutcome::Ready);
        summary.record(DependencyOutcome::NoData);
        assert!(summary.complete());
        summary.record(DependencyOutcome::CircuitOpenCacheMiss);
        summary.record(DependencyOutcome::TransientFailure);
        assert!(!summary.complete());
        assert_eq!(summary.ready, 1);
        assert_eq!(summary.no_data, 1);
        assert_eq!(summary.transient_failures, 1);
        assert_eq!(summary.circuit_open_misses, 1);
    }

    #[test]
    fn preparation_coverage_handles_wrapped_longitude_boxes() {
        let wrapped = PreparationCoverage::LatLonBox(-10.0, 10.0, 170.0, -170.0);
        assert!(wrapped.intersects_tile(-1.0, 1.0, 175.0, 176.0));
        assert!(wrapped.intersects_tile(-1.0, 1.0, -176.0, -175.0));
        assert!(!wrapped.intersects_tile(-1.0, 1.0, -10.0, 10.0));
        assert!(!wrapped.intersects_tile(20.0, 21.0, 175.0, 176.0));
    }
}
