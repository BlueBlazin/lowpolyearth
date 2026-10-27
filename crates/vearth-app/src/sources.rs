//! Native provider-registry assembly shared by the windowed app and the
//! headless screenshot harness.

use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "async-providers")]
use vearth_data::pmtiles::PmTilesStartup;
use vearth_data::pmtiles::{HttpRange, PmTiles};
use vearth_data::registry::{Coverage, ProviderInfo, Registry};
use vearth_data::source_cache::{
    DEFAULT_SOURCE_CACHE_BUDGET, LegacyLayout, SourceTileCache, SourceTileStore,
};
use vearth_data::surface_pack::BundledSurface;
use vearth_data::terrarium::TerrariumSource;
use vearth_data::vectors::{
    PROTOMAPS_BUILD, ProtomapsSource, RegionalLandcoverMode, WaterPolygonMode,
    WatercourseOutputMode, protomaps_build_url,
};

pub struct BuiltSources {
    pub registry: Arc<Registry>,
    /// Shared terrain source used by the location worker for arrival names.
    /// Archive metadata, decoded tiles, and in-flight requests have one owner.
    pub locations: Option<Arc<ProtomapsSource>>,
    /// Physical-input tag for the generated-chunk cache key (immutable vector
    /// revision, terrain-only vector contract, bridge content, default source
    /// profile, and surface month; or "terrain" when vectors are unavailable).
    pub dataset_tag: String,
}

const PIN_FILE_NAME: &str = "pinned_build.txt";
/// Keep generated chunks scoped to the current provider defaults. The value
/// is unchanged from when optional regional sources existed: they were off by
/// default, so removing them left the defaults, and existing caches, as is.
const DEFAULT_SOURCE_PROFILE_TAG: &str = "surface-pack-v2-monthly-regional-opt-in-v1";

/// Records the vector build in use, for tools that read the cache.
fn pin_file(cache_root: &Path) -> PathBuf {
    cache_root.join("vectors").join(PIN_FILE_NAME)
}

#[cfg(feature = "async-providers")]
const STARTUP_FILE_EXTENSION: &str = "pmtiles-startup";

/// Immutable archive header, root directory and metadata of one build.
#[cfg(feature = "async-providers")]
fn startup_file(cache_root: &Path, tag: &str) -> PathBuf {
    cache_root
        .join("vectors")
        .join(format!("{tag}.{STARTUP_FILE_EXTENSION}"))
}

/// The build's startup bytes from an earlier launch, which open it without
/// a network round trip.
#[cfg(feature = "async-providers")]
fn recorded_startup(cache_root: &Path, tag: &str) -> Option<PmTilesStartup> {
    PmTilesStartup::from_bytes(&vearth_platform::fs::read(startup_file(cache_root, tag)).ok()?)
}

/// Record the build's startup bytes, replacing those of other builds.
#[cfg(feature = "async-providers")]
fn record_startup(cache_root: &Path, tag: &str, startup: &PmTilesStartup) {
    let path = startup_file(cache_root, tag);
    let directory = cache_root.join("vectors");
    if let Ok(names) = vearth_platform::fs::list_files(&directory) {
        for name in names {
            let stale = directory.join(name);
            if stale != path
                && stale
                    .extension()
                    .is_some_and(|extension| extension == STARTUP_FILE_EXTENSION)
            {
                let _ = vearth_platform::fs::remove_file(stale);
            }
        }
    }
    if let Err(error) = vearth_platform::fs::write(&path, startup.to_bytes()) {
        log::warn!("vectors: could not record startup bytes for build {tag}: {error}");
    }
}

fn source_cache_budget(configured_bytes: u64) -> u64 {
    vearth_platform::env::var("VEARTH_SOURCE_CACHE_GB")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|gb| (gb * 1024.0 * 1024.0 * 1024.0) as u64)
        .unwrap_or(configured_bytes)
}

fn configure_regional_landcover(source: ProtomapsSource) -> ProtomapsSource {
    let mode = match vearth_platform::env::var("VEARTH_REGIONAL_LANDCOVER").as_deref() {
        Ok("farmland-only") => RegionalLandcoverMode::ExistingFarmlandOnly,
        Ok("selective") | Err(_) => RegionalLandcoverMode::SelectiveRegionalTerrain,
        Ok(value) => {
            log::warn!(
                "unknown VEARTH_REGIONAL_LANDCOVER={value:?}; using selective regional terrain"
            );
            RegionalLandcoverMode::SelectiveRegionalTerrain
        }
    };
    let water_mode = match vearth_platform::env::var("VEARTH_WATER_POLYGONS").as_deref() {
        Ok("legacy") => WaterPolygonMode::LegacyUndifferentiated,
        Ok("typed") | Err(_) => WaterPolygonMode::Typed,
        Ok(value) => {
            log::warn!("unknown VEARTH_WATER_POLYGONS={value:?}; using typed water polygons");
            WaterPolygonMode::Typed
        }
    };
    let watercourse_mode = match vearth_platform::env::var("VEARTH_WATERCOURSES").as_deref() {
        Ok("disabled") => WatercourseOutputMode::DisabledG96,
        Ok("rivers-canals") | Err(_) => WatercourseOutputMode::RiversAndCanals,
        Ok(value) => {
            log::warn!("unknown VEARTH_WATERCOURSES={value:?}; using reviewed rivers/canals");
            WatercourseOutputMode::RiversAndCanals
        }
    };
    source
        .with_regional_landcover_mode(mode)
        .with_water_polygon_mode(water_mode)
        .with_watercourse_output_mode(watercourse_mode)
}

fn source_namespace(
    store: Option<&SourceTileStore>,
    provider: &str,
    revision: &str,
    content_type: &str,
    legacy: Option<LegacyLayout>,
) -> Option<SourceTileCache> {
    let store = store?;
    let migrate = legacy.is_some();
    match store.namespace(provider, revision, content_type, legacy) {
        Ok(cache) => {
            if migrate && let Err(error) = cache.start_legacy_migration() {
                log::warn!("source cache: could not start {provider} migration: {error}");
            }
            Some(cache)
        }
        Err(error) => {
            log::error!("source cache: namespace {provider}/{revision} unavailable: {error}");
            None
        }
    }
}

/// Whether the staged pipeline may use asynchronous providers.
///
/// The legacy pipeline always uses blocking providers. The async transport can
/// also be disabled explicitly for diagnostics.
pub fn async_providers_enabled() -> bool {
    if vearth_platform::env::var("VEARTH_PIPELINE")
        .is_ok_and(|value| value.eq_ignore_ascii_case("legacy"))
    {
        return false;
    }
    #[cfg(feature = "async-providers")]
    {
        !vearth_platform::env::var("VEARTH_ASYNC_PROVIDERS").is_ok_and(|value| {
            value.eq_ignore_ascii_case("legacy")
                || value.eq_ignore_ascii_case("off")
                || value == "0"
        })
    }
    #[cfg(not(feature = "async-providers"))]
    {
        false
    }
}

#[cfg(feature = "async-providers")]
fn block_on_startup<F: std::future::Future>(future: F) -> F::Output {
    vearth_platform::rt::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("provider startup runtime")
        .block_on(future)
}

pub fn build_registry(cache_root: impl AsRef<Path>, surface_month: u8) -> BuiltSources {
    build_registry_with_budget(cache_root, surface_month, DEFAULT_SOURCE_CACHE_BUDGET)
}

pub fn build_registry_with_budget(
    cache_root: impl AsRef<Path>,
    surface_month: u8,
    cache_budget_bytes: u64,
) -> BuiltSources {
    assert!((1..=12).contains(&surface_month), "invalid surface month");
    let cache = cache_root.as_ref().to_path_buf();
    let source_store = match SourceTileStore::open(
        cache.join("source_tiles.sqlite3"),
        source_cache_budget(cache_budget_bytes),
    ) {
        Ok(store) => Some(store),
        Err(error) => {
            // Source caches are disposable. If the indexed store cannot be
            // opened, providers continue network-only rather than recreating
            // the file-per-tile layout.
            log::error!("source cache: indexed store unavailable: {error}");
            None
        }
    };
    let mut registry = Registry::new();
    #[cfg(feature = "async-providers")]
    let async_enabled = async_providers_enabled();
    let fetcher: Arc<dyn vearth_data::TileFetcher> = Arc::new(vearth_data::HttpFetcher::default());
    let terrarium_cache = source_namespace(
        source_store.as_ref(),
        vearth_data::terrarium::CACHE_PROVIDER,
        vearth_data::terrarium::CACHE_REVISION,
        vearth_data::terrarium::CACHE_CONTENT_TYPE,
        None,
    );

    // The async terrain source warms this cache before generation and has no
    // synchronous network fallback.
    let terrarium_info = ProviderInfo {
        name: "Mapterhorn".into(),
        resolution_m: 30.0,
        coverage: Coverage::Global,
        attribution: "© Mapterhorn. Copernicus GLO-30: DLR e.V. 2010-2014 and Airbus Defence and Space GmbH 2014-2018, provided under COPERNICUS by the European Union and ESA; all rights reserved. Regional elevation models from national mapping agencies; full source list linked.".into(),
        license: "open data (Copernicus, CC BY 4.0, public domain, OGL and others); attribution required".into(),
        url: "https://mapterhorn.com/attribution".into(),
    };
    #[cfg(feature = "async-providers")]
    if async_enabled {
        let max_in_flight = vearth_data::terrarium::ASYNC_TILE_LIMIT;
        let transport: Arc<dyn vearth_data::async_http::AsyncTileFetcher> = Arc::new(
            vearth_data::async_http::AsyncHttpFetcher::for_provider("mapterhorn", max_in_flight)
                .with_stall_retry(vearth_data::terrarium::ASYNC_STALL_RETRY),
        );
        registry.add_elevation_async(
            terrarium_info,
            Arc::new(
                TerrariumSource::new_async(
                    transport,
                    vearth_data::terrarium::DEFAULT_URL,
                    terrarium_cache.clone(),
                )
                .with_builtin_terrain(),
            ),
        );
        log::info!("provider transport: async Mapterhorn terrain (limit {max_in_flight})");
    } else {
        registry.add_elevation(
            terrarium_info,
            Arc::new(
                TerrariumSource::new(
                    fetcher.clone(),
                    vearth_data::terrarium::DEFAULT_URL,
                    terrarium_cache.clone(),
                )
                .with_builtin_terrain(),
            ),
        );
    }
    #[cfg(not(feature = "async-providers"))]
    registry.add_elevation(
        terrarium_info,
        Arc::new(
            TerrariumSource::new(
                fetcher.clone(),
                vearth_data::terrarium::DEFAULT_URL,
                terrarium_cache.clone(),
            )
            .with_builtin_terrain(),
        ),
    );

    // Network-free global surface appearance. A coarse level serves orbit
    // generation and a lazy 2 km level supplies regional hue/material cues.
    log::info!(
        "imagery: bundled NASA Blue Marble NG monthly surface pack (month {surface_month:02}, public domain)"
    );
    registry.add_imagery(
        ProviderInfo {
            name: "Bundled Blue Marble NG".into(),
            resolution_m: 2_000.0,
            coverage: Coverage::Global,
            attribution: "NASA Earth Observatory / Blue Marble: Next Generation".into(),
            license: "public domain (NASA)".into(),
            url: "https://science.nasa.gov/earth/earth-observatory/blue-marble-next-generation/base-map/".into(),
        },
        Arc::new(BundledSurface::for_month(surface_month)),
    );

    // Vectors: OSM via the site's copy of a Protomaps planet build
    // (buildings, roads, water, landuse), read from its block objects.
    let mut dataset_tag = "terrain".to_string();
    let mut locations = None;
    #[cfg(feature = "async-providers")]
    let recorded = if async_enabled {
        recorded_startup(&cache, PROTOMAPS_BUILD)
    } else {
        None
    };

    let tag = PROTOMAPS_BUILD.to_string();
    let url = protomaps_build_url(&tag);
    let vector_cache = source_namespace(
        source_store.as_ref(),
        "vectors",
        &tag,
        "application/vnd.mapbox-vector-tile",
        Some(LegacyLayout::Vectors {
            root: cache.join("vectors"),
            build_tag: tag.clone(),
        }),
    );
    let info = ProviderInfo {
        name: "Protomaps/OSM".into(),
        resolution_m: 1.0,
        coverage: Coverage::Global,
        attribution: format!("© OpenStreetMap contributors (ODbL), Protomaps build {tag}"),
        license: "ODbL 1.0".into(),
        url: "https://www.openstreetmap.org/copyright".into(),
    };
    #[cfg(feature = "async-providers")]
    if async_enabled {
        let transport: Arc<dyn vearth_data::async_http::AsyncRangeFetcher> = Arc::new(
            vearth_data::async_http::AsyncHttpRange::for_provider(
                &url,
                "protomaps",
                vearth_data::vectors::ASYNC_RANGE_LIMIT,
            )
            .with_stall_retry(vearth_data::vectors::ASYNC_STALL_RETRY),
        );
        let opened = match &recorded {
            Some(startup) => ProtomapsSource::open_async_from_startup(
                transport,
                tag.clone(),
                vector_cache.clone(),
                startup,
            )
            .map(|source| (source, None)),
            None => block_on_startup(ProtomapsSource::open_async_with_startup(
                transport,
                tag.clone(),
                vector_cache.clone(),
            ))
            .map(|(source, startup)| (source, Some(startup))),
        };
        match opened {
            Ok((src, fetched_startup)) => {
                let src = Arc::new(configure_regional_landcover(src));
                log::info!(
                    "vectors: async protomaps build {tag} (OSM, ODbL){}",
                    if fetched_startup.is_none() {
                        "; opened from recorded startup bytes"
                    } else {
                        ""
                    }
                );
                let _ = vearth_platform::fs::create_dir_all(cache.join("vectors"));
                let _ = vearth_platform::fs::write(pin_file(&cache), &tag);
                if let Some(startup) = &fetched_startup {
                    record_startup(&cache, &tag, startup);
                }
                dataset_tag = src.terrain_dataset_identity_tag();
                locations = Some(src.clone());
                registry.add_vectors_async(info, src);
            }
            Err(error) => {
                log::warn!("protomaps archive unavailable or incompatible ({error}); terrain only");
            }
        }
    } else {
        match PmTiles::open(Box::new(HttpRange::new(&url))) {
            Ok(pm) => match ProtomapsSource::new(pm, tag.clone(), vector_cache.clone()) {
                Ok(src) => {
                    let src = configure_regional_landcover(src);
                    log::info!("vectors: protomaps build {tag} (OSM, ODbL)");
                    let _ = vearth_platform::fs::create_dir_all(cache.join("vectors"));
                    let _ = vearth_platform::fs::write(pin_file(&cache), &tag);
                    let src = Arc::new(src);
                    dataset_tag = src.terrain_dataset_identity_tag();
                    locations = Some(src.clone());
                    registry.add_vectors(info, src);
                }
                Err(error) => {
                    log::warn!("protomaps archive incompatible ({error}); terrain only")
                }
            },
            Err(error) => {
                log::warn!("protomaps archive unavailable ({error}); terrain only")
            }
        }
    }
    #[cfg(not(feature = "async-providers"))]
    match PmTiles::open(Box::new(HttpRange::new(&url))) {
        Ok(pm) => match ProtomapsSource::new(pm, tag.clone(), vector_cache.clone()) {
            Ok(src) => {
                let src = configure_regional_landcover(src);
                log::info!("vectors: protomaps build {tag} (OSM, ODbL)");
                let _ = vearth_platform::fs::create_dir_all(cache.join("vectors"));
                let _ = vearth_platform::fs::write(pin_file(&cache), &tag);
                let src = Arc::new(src);
                dataset_tag = src.terrain_dataset_identity_tag();
                locations = Some(src.clone());
                registry.add_vectors(info, src);
            }
            Err(error) => {
                log::warn!("protomaps archive incompatible ({error}); terrain only")
            }
        },
        Err(error) => {
            log::warn!("protomaps archive unavailable ({error}); terrain only")
        }
    }

    BuiltSources {
        registry: Arc::new(registry),
        locations,
        dataset_tag: format!("{dataset_tag}-{DEFAULT_SOURCE_PROFILE_TAG}-m{surface_month:02}"),
    }
}
