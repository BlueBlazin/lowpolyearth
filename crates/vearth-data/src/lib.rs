//! Geospatial data providers and cache infrastructure.
//!
//! The registry combines global Terrarium/Protomaps data with the bundled
//! surface imagery. Sampling traits remain synchronous and deterministic; the
//! native async pipeline prepares their caches before generation.

pub mod admin_regions;
pub mod bluemarble;
pub mod bridge_pack;
pub mod climate;
pub mod geocode;
mod jpeg;
mod local_tiles;
/// Process-wide data-layer acquisition metrics.
pub mod metrics;
pub mod mvt;
pub mod packed_terrain;
pub mod place_search;
pub mod pmtiles;
pub mod protomaps;
pub mod protomaps_observability;
pub mod protomaps_semantics;
pub mod registry;
pub mod road_profile;
pub mod settlement;
#[cfg(not(target_arch = "wasm32"))]
pub mod source_cache;
#[cfg(target_arch = "wasm32")]
#[path = "source_cache_web.rs"]
pub mod source_cache;
pub mod surface_pack;
pub mod terrarium;
pub mod timezones;
pub mod vectors;
#[cfg(target_arch = "wasm32")]
mod web_http;

/// Native async whole-body/range transports with priority-aware admission.
/// Provider preparers use these to fill synchronous sampling caches before
/// submitting a cell to deterministic generation.
#[cfg(feature = "async-http")]
pub mod async_http;
/// Provider cache-preparation interface used by the native async pipeline.
#[cfg(feature = "async-http")]
pub mod async_prep;

/// One sampled value and whether every dependency needed for it was available.
/// A complete `None` is genuine NoData; an incomplete value may still provide
/// a useful visual fallback but must not become a persistent generated cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceSample<T> {
    pub value: Option<T>,
    pub complete: bool,
}

impl<T> SourceSample<T> {
    pub fn ready(value: Option<T>) -> Self {
        Self {
            value,
            complete: true,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            value: None,
            complete: false,
        }
    }
}

/// A source of real surface color by geodetic position (radians). Returns
/// None outside coverage; the generator falls back to its procedural palette.
pub trait ColorSource: Send + Sync {
    fn color(&self, lat: f64, lon: f64) -> Option<[u8; 3]>;

    fn sample_color(&self, lat: f64, lon: f64) -> SourceSample<[u8; 3]> {
        SourceSample::ready(self.color(lat, lon))
    }

    fn sample_color_detail(
        &self,
        lat: f64,
        lon: f64,
        sample_pitch_m: f64,
    ) -> SourceSample<[u8; 3]> {
        SourceSample::ready(self.color_detail(lat, lon, sample_pitch_m))
    }

    fn sample_color_hr(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> SourceSample<[u8; 3]> {
        SourceSample::ready(self.color_hr(lat, lon, sample_pitch_m))
    }

    /// Selected calendar month, if this source follows the game calendar.
    fn month(&self) -> Option<u8> {
        None
    }

    /// Select a one-based calendar month for time-varying imagery. Static
    /// providers ignore this. Returns true only when the sampled appearance
    /// actually changed, allowing callers to invalidate generated colours.
    fn set_month(&self, _month: u8) -> bool {
        false
    }

    /// Bundled regional-scale appearance. This sits between the coarse globe
    /// colour returned by [`Self::color`] and true local orthophotography:
    /// it is detailed enough to distinguish broad rock, soil, ice and
    /// vegetation regions, but must never be mistaken for a roof-scale pixel.
    fn color_detail(&self, _lat: f64, _lon: f64, _sample_pitch_m: f64) -> Option<[u8; 3]> {
        None
    }

    /// High-resolution color: answers only when the source's
    /// native resolution is meaningful at the `sample_pitch_m` sampling hint —
    /// Tier-1 ortho imagery at street LODs. Coarse global imagery keeps the
    /// default None, so callers can tell "real ortho pixel" from "7 km
    /// Blue Marble mush" and weight the palette blend accordingly.
    fn color_hr(&self, _lat: f64, _lon: f64, _sample_pitch_m: f64) -> Option<[u8; 3]> {
        None
    }

    /// Aggregate failure telemetry. Generated-cell completeness comes from
    /// each `sample_*` result, independently of concurrent provider activity.
    fn fetch_errors(&self) -> u64 {
        0
    }

    /// Short human-readable health note for the HUD, or None when nominal.
    fn status_line(&self) -> Option<String> {
        None
    }
}

/// A source of terrain elevation. `sample_pitch_m` hints the sampling resolution so
/// implementations can pick an appropriate tile zoom level.
///
/// Implementations must be thread-safe: the generator calls this from worker
/// threads. Async-backed providers prepare caches before generation, so
/// sampling is cache-only; legacy providers may block.
pub trait ElevationSource: Send + Sync {
    /// Elevation in meters (relative to the source's sea level) at geodetic
    /// lat/lon in radians. Returns 0.0 where the source has no data.
    fn elevation_m(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> f32;

    /// Like `elevation_m`, but distinguishes "no data here" (None) so a
    /// registry can chain to a coarser provider per sample. Tier-1 regional
    /// DEMs are typically land-only: their NoData over water must fall
    /// through to the global source's bathymetry instead of reading as 0.
    /// Default: every sample answers.
    fn elevation_opt(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> Option<f32> {
        Some(self.elevation_m(lat, lon, sample_pitch_m))
    }

    /// Sample with dependency status local to this query. Providers that can
    /// fail or serve degraded fallback values override this method.
    fn sample_elevation(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> SourceSample<f32> {
        SourceSample::ready(self.elevation_opt(lat, lon, sample_pitch_m))
    }

    /// Warm the cache for the given tiles before sampling; implementations
    /// may fetch concurrently. Default: no-op. Cold generation is dominated
    /// by serial tile latency without this.
    fn prefetch(&self, _tiles: &[(u8, u32, u32)]) {}

    /// Aggregate fetch/decode failure telemetry. It is not a generation
    /// validity signal: other cells may sample this provider concurrently.
    fn fetch_errors(&self) -> u64 {
        0
    }

    /// Short human-readable health note for the HUD ("paused 41s",
    /// "reconnecting"), or None when nominal. Default: always nominal.
    fn status_line(&self) -> Option<String> {
        None
    }
}

/// Analytic elevation source for tests and offline fallback.
pub struct FnElevation<F>(pub F);

impl<F> ElevationSource for FnElevation<F>
where
    F: Fn(f64, f64) -> f32 + Send + Sync,
{
    fn elevation_m(&self, lat: f64, lon: f64, _sample_pitch_m: f64) -> f32 {
        (self.0)(lat, lon)
    }
}

/// Synchronous raw-byte transport used by the rollback path.
pub trait TileFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, DataError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    Http(String),
    Decode(String),
    NotAvailable,
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::Http(e) => write!(f, "http: {e}"),
            DataError::Decode(e) => write!(f, "decode: {e}"),
            DataError::NotAvailable => write!(f, "not available"),
        }
    }
}

impl std::error::Error for DataError {}

/// Blocking HTTP fetcher: ureq natively, synchronous XHR in browser workers.
pub struct HttpFetcher {
    #[cfg(not(target_arch = "wasm32"))]
    agent: ureq::Agent,
    #[cfg(target_arch = "wasm32")]
    timeout: vearth_platform::time::Duration,
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::with_timeout(vearth_platform::time::Duration::from_secs(20))
    }
}

impl HttpFetcher {
    /// A fetcher with a custom global timeout, for endpoints where a slow
    /// response should fail fast rather than pin a blocking worker for the
    /// default timeout.
    pub fn with_timeout(timeout: vearth_platform::time::Duration) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            HttpFetcher { timeout }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let config = ureq::Agent::config_builder()
                .timeout_global(Some(timeout))
                .user_agent(concat!("vearth/", env!("CARGO_PKG_VERSION")))
                // Dozens of gen workers fetch from one tile host concurrently;
                // the default per-host idle pool (~3) would drop and re-open
                // (TLS handshake and all) most connections between requests.
                .max_idle_connections(64)
                .max_idle_connections_per_host(32)
                .build();
            HttpFetcher {
                agent: config.into(),
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn get(&self, url: &str) -> Result<Vec<u8>, DataError> {
        let mut resp = match self.agent.get(url).call() {
            Ok(resp) => resp,
            // The object does not exist; sources decide what absence means.
            Err(ureq::Error::StatusCode(404)) => return Err(DataError::NotAvailable),
            Err(e) => return Err(DataError::Http(e.to_string())),
        };
        resp.body_mut()
            .read_to_vec()
            .map_err(|e| DataError::Http(e.to_string()))
    }

    #[cfg(target_arch = "wasm32")]
    fn get(&self, url: &str) -> Result<Vec<u8>, DataError> {
        let response = web_http::request("GET", url, None, self.timeout)?;
        match response.status {
            200..=299 => Ok(response.body),
            404 => Err(DataError::NotAvailable),
            status => Err(DataError::Http(format!("{url}: status {status}"))),
        }
    }
}

impl TileFetcher for HttpFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, DataError> {
        let series = metrics::http_acquisition("tile", url);
        let request = series.enqueue().start();
        match self.get(url) {
            Ok(body) => {
                request.success(body.len());
                metrics::record(body.len());
                Ok(body)
            }
            Err(DataError::NotAvailable) => {
                request.not_found();
                Err(DataError::NotAvailable)
            }
            Err(error) => {
                request.error();
                metrics::record_err();
                Err(error)
            }
        }
    }
}
