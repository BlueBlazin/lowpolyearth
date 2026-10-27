//! Provider registry for resolving data sources by coverage and resolution.

use crate::vectors::{FeaturePack, VectorSource};
use crate::{ColorSource, ElevationSource, SourceSample};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub enum Coverage {
    Global,
    /// Degrees, inclusive: (lat_min, lat_max, lon_min, lon_max).
    LatLonBox(f64, f64, f64, f64),
}

impl Coverage {
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        match self {
            Coverage::Global => true,
            Coverage::LatLonBox(la0, la1, lo0, lo1) => {
                let lat = lat.to_degrees();
                let raw_lon = lon.to_degrees();
                let mut lon = (raw_lon + 180.0).rem_euclid(360.0) - 180.0;
                if lon == -180.0 && raw_lon > 0.0 {
                    lon = 180.0;
                }
                let longitude_inside = if lo0 <= lo1 {
                    lon >= *lo0 && lon <= *lo1
                } else {
                    lon >= *lo0 || lon <= *lo1
                };
                lat >= *la0 && lat <= *la1 && longitude_inside
            }
        }
    }

    #[cfg(feature = "async-http")]
    fn preparation_coverage(&self) -> crate::async_prep::PreparationCoverage {
        match *self {
            Coverage::Global => crate::async_prep::PreparationCoverage::Global,
            Coverage::LatLonBox(lat0, lat1, lon0, lon1) => {
                crate::async_prep::PreparationCoverage::LatLonBox(lat0, lat1, lon0, lon1)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub name: String,
    /// Native ground resolution — providers are tried finest-first.
    pub resolution_m: f64,
    pub coverage: Coverage,
    pub attribution: String,
    /// License short-name for the attribution screen ("ODbL 1.0", "public
    /// domain", "OGL v3", …). Empty = unstated.
    pub license: String,
    /// Canonical dataset/source URL for the attribution screen.
    pub url: String,
}

/// Stable provider grouping shared by user-facing controls.
/// The enum names describe the internal data channels; `label` describes the
/// user-visible effect of disabling that channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderLayer {
    Elevation,
    Surface,
    Imagery,
    Vectors,
}

impl ProviderLayer {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Elevation => "Terrain Height",
            Self::Surface => "Building Surfaces",
            Self::Imagery => "Appearance",
            Self::Vectors => "Map Features",
        }
    }
}

/// A provider's stable control identity. Names need only be unique within a
/// layer, matching how the registry resolves its source chains.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderId {
    pub layer: ProviderLayer,
    pub name: String,
}

/// Snapshot used by UI surfaces. Provider enablement itself is atomic and
/// lives in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderControl {
    pub id: ProviderId,
    pub enabled: bool,
}

/// One row of the in-app attribution screen: a registered provider
/// with its layer, license and source link.
#[derive(Debug, Clone)]
pub struct AttributionEntry {
    /// User-facing provider group, shared with the data-source controls.
    pub layer: &'static str,
    pub name: String,
    pub attribution: String,
    pub license: String,
    pub url: String,
}

struct Slot<T: ?Sized> {
    info: ProviderInfo,
    source: Arc<T>,
    enabled: AtomicBool,
    #[cfg(feature = "async-http")]
    preparer: Option<Arc<dyn crate::async_prep::AsyncSourcePreparer>>,
}

impl<T: ?Sized> Slot<T> {
    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
}

/// Surface-model (DSM) chain: first-return height — ground
/// plus built/vegetation mass — as opposed to the bare-earth elevation
/// chain. Same finest-covering-first per-sample resolution, but with no
/// global fallback: None simply means "no LiDAR surface here" and the
/// generator falls back to footprint extrusion.
#[derive(Default)]
pub struct SurfaceChain {
    slots: Vec<Slot<dyn ElevationSource>>,
}

fn resolve_samples<T>(samples: impl IntoIterator<Item = SourceSample<T>>) -> SourceSample<T> {
    let mut complete = true;
    for sample in samples {
        complete &= sample.complete;
        if sample.value.is_some() {
            return SourceSample {
                value: sample.value,
                complete,
            };
        }
    }
    SourceSample {
        value: None,
        complete,
    }
}

impl ElevationSource for SurfaceChain {
    fn sample_elevation(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> SourceSample<f32> {
        resolve_samples(
            self.slots
                .iter()
                .filter(|slot| slot.enabled() && slot.info.coverage.contains(lat, lon))
                .map(|slot| slot.source.sample_elevation(lat, lon, sample_pitch_m)),
        )
    }

    fn elevation_m(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> f32 {
        self.elevation_opt(lat, lon, sample_pitch_m).unwrap_or(0.0)
    }

    fn elevation_opt(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> Option<f32> {
        self.slots
            .iter()
            .filter(|s| s.enabled() && s.info.coverage.contains(lat, lon))
            .find_map(|s| s.source.elevation_opt(lat, lon, sample_pitch_m))
    }

    fn fetch_errors(&self) -> u64 {
        self.slots
            .iter()
            .filter(|s| s.enabled())
            .map(|s| s.source.fetch_errors())
            .sum()
    }
}

/// Layered multi-provider registry. Also *is* each source trait, resolving
/// per-sample — so the generator keeps its simple `&dyn` view of the world.
#[derive(Default)]
pub struct Registry {
    elevation: Vec<Slot<dyn ElevationSource>>,
    surface: SurfaceChain,
    imagery: Vec<Slot<dyn ColorSource>>,
    vectors: Vec<Slot<dyn VectorSource>>,
    /// Once controls change, generated chunks bypass the default persistent
    /// cache for the rest of the session so differently sourced worlds can
    /// never contaminate entries for the configured default source profile.
    configuration_changed: AtomicBool,
}

fn insert_by_resolution<T: ?Sized>(v: &mut Vec<Slot<T>>, slot: Slot<T>) {
    let at = v.partition_point(|s| s.info.resolution_m <= slot.info.resolution_m);
    v.insert(at, slot);
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_elevation(&mut self, info: ProviderInfo, source: Arc<dyn ElevationSource>) {
        self.add_elevation_with_initial_state(info, source, true);
    }

    /// Register an elevation provider that remains visible to controls and
    /// attribution, but does not participate in sampling until the user
    /// enables it. This initial state is part of registry construction, not
    /// a runtime configuration change.
    pub fn add_elevation_disabled(&mut self, info: ProviderInfo, source: Arc<dyn ElevationSource>) {
        self.add_elevation_with_initial_state(info, source, false);
    }

    fn add_elevation_with_initial_state(
        &mut self,
        info: ProviderInfo,
        source: Arc<dyn ElevationSource>,
        enabled: bool,
    ) {
        insert_by_resolution(
            &mut self.elevation,
            Slot {
                info,
                source,
                enabled: AtomicBool::new(enabled),
                #[cfg(feature = "async-http")]
                preparer: None,
            },
        );
    }

    pub fn add_surface(&mut self, info: ProviderInfo, source: Arc<dyn ElevationSource>) {
        self.add_surface_with_initial_state(info, source, true);
    }

    /// Register a surface provider that remains visible to controls and
    /// attribution, but does not participate in sampling until the user
    /// enables it. This initial state is part of registry construction, not
    /// a runtime configuration change.
    pub fn add_surface_disabled(&mut self, info: ProviderInfo, source: Arc<dyn ElevationSource>) {
        self.add_surface_with_initial_state(info, source, false);
    }

    fn add_surface_with_initial_state(
        &mut self,
        info: ProviderInfo,
        source: Arc<dyn ElevationSource>,
        enabled: bool,
    ) {
        insert_by_resolution(
            &mut self.surface.slots,
            Slot {
                info,
                source,
                enabled: AtomicBool::new(enabled),
                #[cfg(feature = "async-http")]
                preparer: None,
            },
        );
    }

    /// The DSM chain as an elevation-shaped source, or None when no surface
    /// provider is registered (the generator then skips surface sampling
    /// entirely).
    pub fn surface(&self) -> Option<&dyn ElevationSource> {
        self.surface
            .slots
            .iter()
            .any(|slot| slot.enabled())
            .then_some(&self.surface as &dyn ElevationSource)
    }

    pub fn add_imagery(&mut self, info: ProviderInfo, source: Arc<dyn ColorSource>) {
        self.add_imagery_with_initial_state(info, source, true);
    }

    /// Register an imagery provider that remains visible to controls and
    /// attribution, but does not participate in sampling until the user
    /// enables it. This initial state is part of registry construction, not
    /// a runtime configuration change.
    pub fn add_imagery_disabled(&mut self, info: ProviderInfo, source: Arc<dyn ColorSource>) {
        self.add_imagery_with_initial_state(info, source, false);
    }

    fn add_imagery_with_initial_state(
        &mut self,
        info: ProviderInfo,
        source: Arc<dyn ColorSource>,
        enabled: bool,
    ) {
        insert_by_resolution(
            &mut self.imagery,
            Slot {
                info,
                source,
                enabled: AtomicBool::new(enabled),
                #[cfg(feature = "async-http")]
                preparer: None,
            },
        );
    }

    pub fn add_vectors(&mut self, info: ProviderInfo, source: Arc<dyn VectorSource>) {
        insert_by_resolution(
            &mut self.vectors,
            Slot {
                info,
                source,
                enabled: AtomicBool::new(true),
                #[cfg(feature = "async-http")]
                preparer: None,
            },
        );
    }

    #[cfg(feature = "async-http")]
    pub fn add_elevation_async<S>(&mut self, info: ProviderInfo, source: Arc<S>)
    where
        S: ElevationSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        self.add_elevation_async_with_initial_state(info, source, true);
    }

    /// Async counterpart to [`Registry::add_elevation_disabled`]. Disabled
    /// providers do not prepare or fetch tiles until enabled at runtime.
    #[cfg(feature = "async-http")]
    pub fn add_elevation_async_disabled<S>(&mut self, info: ProviderInfo, source: Arc<S>)
    where
        S: ElevationSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        self.add_elevation_async_with_initial_state(info, source, false);
    }

    #[cfg(feature = "async-http")]
    fn add_elevation_async_with_initial_state<S>(
        &mut self,
        info: ProviderInfo,
        source: Arc<S>,
        enabled: bool,
    ) where
        S: ElevationSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        let sampler: Arc<dyn ElevationSource> = source.clone();
        let preparer: Arc<dyn crate::async_prep::AsyncSourcePreparer> = source;
        insert_by_resolution(
            &mut self.elevation,
            Slot {
                info,
                source: sampler,
                enabled: AtomicBool::new(enabled),
                preparer: Some(preparer),
            },
        );
    }

    #[cfg(feature = "async-http")]
    pub fn add_surface_async<S>(&mut self, info: ProviderInfo, source: Arc<S>)
    where
        S: ElevationSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        self.add_surface_async_with_initial_state(info, source, true);
    }

    /// Async counterpart to [`Registry::add_surface_disabled`]. Disabled
    /// providers do not prepare or fetch tiles until enabled at runtime.
    #[cfg(feature = "async-http")]
    pub fn add_surface_async_disabled<S>(&mut self, info: ProviderInfo, source: Arc<S>)
    where
        S: ElevationSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        self.add_surface_async_with_initial_state(info, source, false);
    }

    #[cfg(feature = "async-http")]
    fn add_surface_async_with_initial_state<S>(
        &mut self,
        info: ProviderInfo,
        source: Arc<S>,
        enabled: bool,
    ) where
        S: ElevationSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        let sampler: Arc<dyn ElevationSource> = source.clone();
        let preparer: Arc<dyn crate::async_prep::AsyncSourcePreparer> = source;
        insert_by_resolution(
            &mut self.surface.slots,
            Slot {
                info,
                source: sampler,
                enabled: AtomicBool::new(enabled),
                preparer: Some(preparer),
            },
        );
    }

    #[cfg(feature = "async-http")]
    pub fn add_imagery_async<S>(&mut self, info: ProviderInfo, source: Arc<S>)
    where
        S: ColorSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        self.add_imagery_async_with_initial_state(info, source, true);
    }

    /// Async counterpart to [`Registry::add_imagery_disabled`]. Disabled
    /// providers do not prepare or fetch tiles until enabled at runtime.
    #[cfg(feature = "async-http")]
    pub fn add_imagery_async_disabled<S>(&mut self, info: ProviderInfo, source: Arc<S>)
    where
        S: ColorSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        self.add_imagery_async_with_initial_state(info, source, false);
    }

    #[cfg(feature = "async-http")]
    fn add_imagery_async_with_initial_state<S>(
        &mut self,
        info: ProviderInfo,
        source: Arc<S>,
        enabled: bool,
    ) where
        S: ColorSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        let sampler: Arc<dyn ColorSource> = source.clone();
        let preparer: Arc<dyn crate::async_prep::AsyncSourcePreparer> = source;
        insert_by_resolution(
            &mut self.imagery,
            Slot {
                info,
                source: sampler,
                enabled: AtomicBool::new(enabled),
                preparer: Some(preparer),
            },
        );
    }

    #[cfg(feature = "async-http")]
    pub fn add_vectors_async<S>(&mut self, info: ProviderInfo, source: Arc<S>)
    where
        S: VectorSource + crate::async_prep::AsyncSourcePreparer + 'static,
    {
        let sampler: Arc<dyn VectorSource> = source.clone();
        let preparer: Arc<dyn crate::async_prep::AsyncSourcePreparer> = source;
        insert_by_resolution(
            &mut self.vectors,
            Slot {
                info,
                source: sampler,
                enabled: AtomicBool::new(true),
                preparer: Some(preparer),
            },
        );
    }

    /// Current provider controls, grouped in the same finest-first layer
    /// order used for sampling and attribution.
    pub fn provider_controls(&self) -> Vec<ProviderControl> {
        fn append<T: ?Sized>(
            out: &mut Vec<ProviderControl>,
            layer: ProviderLayer,
            slots: &[Slot<T>],
        ) {
            out.extend(slots.iter().map(|slot| ProviderControl {
                id: ProviderId {
                    layer,
                    name: slot.info.name.clone(),
                },
                enabled: slot.enabled(),
            }));
        }

        let mut out = Vec::new();
        append(&mut out, ProviderLayer::Elevation, &self.elevation);
        append(&mut out, ProviderLayer::Surface, &self.surface.slots);
        append(&mut out, ProviderLayer::Imagery, &self.imagery);
        append(&mut out, ProviderLayer::Vectors, &self.vectors);
        out
    }

    /// Enable or disable one registered provider. Returns true only when a
    /// matching provider changed state.
    pub fn set_provider_enabled(&self, id: &ProviderId, enabled: bool) -> bool {
        fn set<T: ?Sized>(slots: &[Slot<T>], name: &str, enabled: bool) -> bool {
            slots
                .iter()
                .find(|slot| slot.info.name == name)
                .is_some_and(|slot| slot.enabled.swap(enabled, Ordering::AcqRel) != enabled)
        }

        let changed = match id.layer {
            ProviderLayer::Elevation => set(&self.elevation, &id.name, enabled),
            ProviderLayer::Surface => set(&self.surface.slots, &id.name, enabled),
            ProviderLayer::Imagery => set(&self.imagery, &id.name, enabled),
            ProviderLayer::Vectors => set(&self.vectors, &id.name, enabled),
        };
        if changed {
            self.configuration_changed.store(true, Ordering::Release);
        }
        changed
    }

    /// Select the calendar month on every time-varying imagery provider.
    /// A change invalidates the default generated-cache profile for the rest
    /// of the session, just like a provider toggle, because voxel colours are
    /// baked during generation.
    pub fn set_imagery_month(&self, month: u8) -> bool {
        let mut changed = false;
        for slot in &self.imagery {
            let source_changed = slot.source.set_month(month);
            changed |= slot.enabled() && source_changed;
        }
        if changed {
            self.configuration_changed.store(true, Ordering::Release);
        }
        changed
    }

    /// The persistent generated-chunk cache represents the registry's
    /// configured default source profile. Once runtime controls change,
    /// bypass it for session safety.
    pub fn generated_cache_allowed(&self) -> bool {
        !self.configuration_changed.load(Ordering::Acquire)
    }

    #[cfg(feature = "async-http")]
    pub fn has_async_preparers(&self) -> bool {
        self.elevation
            .iter()
            .chain(self.surface.slots.iter())
            .any(|slot| slot.enabled() && slot.preparer.is_some())
            || self
                .imagery
                .iter()
                .any(|slot| slot.enabled() && slot.preparer.is_some())
            || self
                .vectors
                .iter()
                .any(|slot| slot.enabled() && slot.preparer.is_some())
    }

    #[cfg(feature = "async-http")]
    pub async fn prepare_async(
        &self,
        request: &crate::async_prep::PrepareRequest,
    ) -> crate::async_prep::PrepareSummary {
        self.prepare_retained_async(request).await.summary
    }

    #[cfg(feature = "async-http")]
    pub async fn prepare_retained_async(
        &self,
        request: &crate::async_prep::PrepareRequest,
    ) -> crate::async_prep::PreparedSources {
        let mut futures = self.raster_preparations(request);
        futures.extend(self.vector_preparations(request));
        crate::async_prep::join_retained(futures).await
    }

    /// Prepare elevation, surface and imagery dependencies, retaining their
    /// source slots independently of vector preparation.
    #[cfg(feature = "async-http")]
    pub async fn prepare_raster_retained_async(
        &self,
        request: &crate::async_prep::PrepareRequest,
    ) -> crate::async_prep::PreparedSources {
        crate::async_prep::join_retained(self.raster_preparations(request)).await
    }

    /// Prepare vector dependencies so projection and geometry-datum discovery
    /// can begin while independent raster preparation is still running.
    #[cfg(feature = "async-http")]
    pub async fn prepare_vector_retained_async(
        &self,
        request: &crate::async_prep::PrepareRequest,
    ) -> crate::async_prep::PreparedSources {
        crate::async_prep::join_retained(self.vector_preparations(request)).await
    }

    #[cfg(feature = "async-http")]
    fn raster_preparations<'a>(
        &'a self,
        request: &'a crate::async_prep::PrepareRequest,
    ) -> Vec<crate::async_prep::RetainedPrepareFuture<'a>> {
        let mut futures = Vec::new();
        for slot in self.elevation.iter().chain(self.surface.slots.iter()) {
            if slot.enabled()
                && coverage_intersects(&slot.info.coverage, request.footprint)
                && let Some(preparer) = &slot.preparer
            {
                futures.push(preparer.prepare_retained_for_coverage(
                    request,
                    slot.info.coverage.preparation_coverage(),
                ));
            }
        }
        for slot in &self.imagery {
            if slot.enabled()
                && coverage_intersects(&slot.info.coverage, request.footprint)
                && let Some(preparer) = &slot.preparer
            {
                futures.push(preparer.prepare_retained_for_coverage(
                    request,
                    slot.info.coverage.preparation_coverage(),
                ));
            }
        }
        futures
    }

    #[cfg(feature = "async-http")]
    fn vector_preparations<'a>(
        &'a self,
        request: &'a crate::async_prep::PrepareRequest,
    ) -> Vec<crate::async_prep::RetainedPrepareFuture<'a>> {
        let mut futures = Vec::new();
        for slot in &self.vectors {
            if slot.enabled()
                && coverage_intersects(&slot.info.coverage, request.footprint)
                && let Some(preparer) = &slot.preparer
            {
                futures.push(preparer.prepare_retained_for_coverage(
                    request,
                    slot.info.coverage.preparation_coverage(),
                ));
            }
        }
        futures
    }

    /// Warm geometry datums from either bare-earth elevation or surface models.
    #[cfg(feature = "async-http")]
    pub async fn prepare_geometry_async(
        &self,
        request: &crate::async_prep::PrepareRequest,
        surface_only: bool,
    ) -> crate::async_prep::PrepareSummary {
        self.prepare_geometry_retained_async(request, surface_only)
            .await
            .summary
    }

    #[cfg(feature = "async-http")]
    pub async fn prepare_geometry_retained_async(
        &self,
        request: &crate::async_prep::PrepareRequest,
        surface_only: bool,
    ) -> crate::async_prep::PreparedSources {
        let mut futures = Vec::new();
        let slots = if surface_only {
            &self.surface.slots
        } else {
            &self.elevation
        };
        for slot in slots {
            if slot.enabled()
                && coverage_intersects(&slot.info.coverage, request.footprint)
                && let Some(preparer) = &slot.preparer
            {
                futures.push(preparer.prepare_retained_for_coverage(
                    request,
                    slot.info.coverage.preparation_coverage(),
                ));
            }
        }
        crate::async_prep::join_retained(futures).await
    }

    pub fn has_vectors(&self) -> bool {
        self.vectors.iter().any(|slot| slot.enabled())
    }

    /// One line per registered provider for the in-app attribution list.
    pub fn attributions(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .attribution_entries()
            .into_iter()
            .filter(|e| !e.attribution.is_empty())
            .map(|e| format!("{}: {}", e.name, e.attribution))
            .collect();
        out.dedup();
        out
    }

    /// Non-nominal provider health notes for the HUD, as `"<provider>: <note>"`.
    /// Empty when everything is nominal.
    pub fn provider_status(&self) -> Vec<String> {
        let note = |name: &str, st: Option<String>| st.map(|st| format!("{name}: {st}"));
        self.elevation
            .iter()
            .chain(self.surface.slots.iter())
            .filter(|s| s.enabled())
            .filter_map(|s| note(&s.info.name, s.source.status_line()))
            .chain(
                self.imagery
                    .iter()
                    .filter(|s| s.enabled())
                    .filter_map(|s| note(&s.info.name, s.source.status_line())),
            )
            .collect()
    }

    /// Structured attribution rows for every registered provider, including
    /// disabled providers, with layer, license and source link.
    pub fn attribution_entries(&self) -> Vec<AttributionEntry> {
        let layer = |layer: ProviderLayer, i: &ProviderInfo| AttributionEntry {
            layer: layer.label(),
            name: i.name.clone(),
            attribution: i.attribution.clone(),
            license: i.license.clone(),
            url: i.url.clone(),
        };
        self.elevation
            .iter()
            .map(|s| layer(ProviderLayer::Elevation, &s.info))
            .chain(
                self.surface
                    .slots
                    .iter()
                    .map(|s| layer(ProviderLayer::Surface, &s.info)),
            )
            .chain(
                self.imagery
                    .iter()
                    .map(|s| layer(ProviderLayer::Imagery, &s.info)),
            )
            .chain(
                self.vectors
                    .iter()
                    .map(|s| layer(ProviderLayer::Vectors, &s.info)),
            )
            .collect()
    }
}

#[cfg(feature = "async-http")]
fn coverage_intersects(coverage: &Coverage, footprint: crate::async_prep::SourceFootprint) -> bool {
    match coverage {
        Coverage::Global => true,
        Coverage::LatLonBox(lat0, lat1, lon0, lon1) => {
            let radius = footprint.radius_rad.clamp(0.0, std::f64::consts::PI);
            let cap_lat0 = (footprint.center_lat - radius).max(-std::f64::consts::FRAC_PI_2);
            let cap_lat1 = (footprint.center_lat + radius).min(std::f64::consts::FRAC_PI_2);
            if cap_lat1.to_degrees() < *lat0 || cap_lat0.to_degrees() > *lat1 {
                return false;
            }

            // A cap that reaches either pole spans every longitude. Otherwise
            // use the smallest cosine in its latitude interval; this is a
            // deliberately conservative equirectangular envelope.
            let lon_radius = if cap_lat0 <= -std::f64::consts::FRAC_PI_2
                || cap_lat1 >= std::f64::consts::FRAC_PI_2
            {
                180.0
            } else {
                (radius.to_degrees() / cap_lat0.cos().abs().min(cap_lat1.cos().abs()).max(0.05))
                    .min(180.0)
            };
            let center_lon = (footprint.center_lon.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
            longitude_intervals(center_lon, lon_radius)
                .iter()
                .any(|cap| {
                    coverage_longitude_intervals(*lon0, *lon1)
                        .iter()
                        .any(|provider| cap.1 >= provider.0 && cap.0 <= provider.1)
                })
        }
    }
}

#[cfg(feature = "async-http")]
fn longitude_intervals(center: f64, radius: f64) -> Vec<(f64, f64)> {
    if radius >= 180.0 {
        return vec![(-180.0, 180.0)];
    }
    let lo = center - radius;
    let hi = center + radius;
    if lo < -180.0 {
        vec![(-180.0, hi), (lo + 360.0, 180.0)]
    } else if hi > 180.0 {
        vec![(lo, 180.0), (-180.0, hi - 360.0)]
    } else {
        vec![(lo, hi)]
    }
}

#[cfg(feature = "async-http")]
fn coverage_longitude_intervals(lon0: f64, lon1: f64) -> Vec<(f64, f64)> {
    if lon0 <= lon1 {
        vec![(lon0, lon1)]
    } else {
        vec![(lon0, 180.0), (-180.0, lon1)]
    }
}

impl ElevationSource for Registry {
    fn sample_elevation(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> SourceSample<f32> {
        let sample = resolve_samples(
            self.elevation
                .iter()
                .filter(|slot| slot.enabled() && slot.info.coverage.contains(lat, lon))
                .map(|slot| slot.source.sample_elevation(lat, lon, sample_pitch_m)),
        );
        // The ground registry guarantees a sea-level terminal fallback, as
        // elevation_opt does. Its dependency status still forbids persisting
        // that fallback when a covering provider was unavailable.
        SourceSample {
            value: Some(sample.value.unwrap_or(0.0)),
            complete: sample.complete,
        }
    }

    fn elevation_m(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> f32 {
        // Finest covering provider first, falling through on per-sample
        // NoData: land-only Tier-1 DEMs yield to global bathymetry over
        // water, and a transient fetch failure degrades to coarser data
        // instead of a 0 m crater.
        for s in &self.elevation {
            if s.enabled()
                && s.info.coverage.contains(lat, lon)
                && let Some(h) = s.source.elevation_opt(lat, lon, sample_pitch_m)
            {
                return h;
            }
        }
        0.0
    }

    fn prefetch(&self, tiles: &[(u8, u32, u32)]) {
        // Prefetch hints target the global baseline provider (tile ids are
        // provider-specific; Tier-1 providers prefetch on their own terms).
        if let Some(s) = self.elevation.iter().rev().find(|slot| slot.enabled()) {
            s.source.prefetch(tiles);
        }
    }

    fn fetch_errors(&self) -> u64 {
        self.elevation
            .iter()
            .filter(|s| s.enabled())
            .map(|s| s.source.fetch_errors())
            .sum()
    }
}

impl ColorSource for Registry {
    fn month(&self) -> Option<u8> {
        self.imagery
            .iter()
            .filter(|slot| slot.enabled())
            .find_map(|slot| slot.source.month())
    }

    fn sample_color(&self, lat: f64, lon: f64) -> SourceSample<[u8; 3]> {
        resolve_samples(
            self.imagery
                .iter()
                .filter(|slot| slot.enabled() && slot.info.coverage.contains(lat, lon))
                .map(|slot| slot.source.sample_color(lat, lon)),
        )
    }

    fn sample_color_detail(
        &self,
        lat: f64,
        lon: f64,
        sample_pitch_m: f64,
    ) -> SourceSample<[u8; 3]> {
        resolve_samples(
            self.imagery
                .iter()
                .filter(|slot| slot.enabled() && slot.info.coverage.contains(lat, lon))
                .map(|slot| slot.source.sample_color_detail(lat, lon, sample_pitch_m)),
        )
    }

    fn sample_color_hr(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> SourceSample<[u8; 3]> {
        resolve_samples(
            self.imagery
                .iter()
                .filter(|slot| slot.enabled() && slot.info.coverage.contains(lat, lon))
                .map(|slot| slot.source.sample_color_hr(lat, lon, sample_pitch_m)),
        )
    }

    fn color(&self, lat: f64, lon: f64) -> Option<[u8; 3]> {
        self.imagery
            .iter()
            .filter(|s| s.enabled() && s.info.coverage.contains(lat, lon))
            .find_map(|s| s.source.color(lat, lon))
    }

    fn color_detail(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> Option<[u8; 3]> {
        self.imagery
            .iter()
            .filter(|s| s.enabled() && s.info.coverage.contains(lat, lon))
            .find_map(|s| s.source.color_detail(lat, lon, sample_pitch_m))
    }

    fn color_hr(&self, lat: f64, lon: f64, sample_pitch_m: f64) -> Option<[u8; 3]> {
        self.imagery
            .iter()
            .filter(|s| s.enabled() && s.info.coverage.contains(lat, lon))
            .find_map(|s| s.source.color_hr(lat, lon, sample_pitch_m))
    }

    fn fetch_errors(&self) -> u64 {
        self.imagery
            .iter()
            .filter(|s| s.enabled())
            .map(|s| s.source.fetch_errors())
            .sum()
    }
}

impl VectorSource for Registry {
    fn features(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) -> FeaturePack {
        for s in &self.vectors {
            if s.enabled() && s.info.coverage.contains(lat0, lon0) {
                return s.source.features(lat0, lon0, half_e_m, half_n_m);
            }
        }
        FeaturePack::empty()
    }

    fn prefetch_vectors(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) {
        for s in &self.vectors {
            if s.enabled() && s.info.coverage.contains(lat0, lon0) {
                s.source.prefetch_vectors(lat0, lon0, half_e_m, half_n_m);
                return;
            }
        }
    }

    fn night_buildings(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) -> FeaturePack {
        for s in &self.vectors {
            if s.enabled() && s.info.coverage.contains(lat0, lon0) {
                return s.source.night_buildings(lat0, lon0, half_e_m, half_n_m);
            }
        }
        FeaturePack::empty()
    }

    fn prefetch_night_buildings(&self, lat0: f64, lon0: f64, half_e_m: f64, half_n_m: f64) {
        for s in &self.vectors {
            if s.enabled() && s.info.coverage.contains(lat0, lon0) {
                s.source
                    .prefetch_night_buildings(lat0, lon0, half_e_m, half_n_m);
                return;
            }
        }
    }

    fn attribution(&self) -> String {
        self.vectors
            .iter()
            .find(|slot| slot.enabled())
            .map(|s| s.info.attribution.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorSource, FnElevation};

    #[test]
    fn sample_resolution_tracks_only_queried_dependencies_and_distinguishes_nodata() {
        assert_eq!(
            resolve_samples([SourceSample::ready(None), SourceSample::ready(Some(12.0))]),
            SourceSample::ready(Some(12.0)),
        );
        assert_eq!(
            resolve_samples([SourceSample::unavailable(), SourceSample::ready(Some(12.0))]),
            SourceSample {
                value: Some(12.0),
                complete: false
            },
        );
        assert_eq!(
            resolve_samples([SourceSample {
                value: Some(12.0),
                complete: false
            }]),
            SourceSample {
                value: Some(12.0),
                complete: false
            },
        );
        let unused = std::iter::from_fn(|| {
            panic!("a resolved sample must not query lower priority providers")
        });
        assert_eq!(
            resolve_samples(std::iter::once(SourceSample::ready(Some(12.0))).chain(unused)),
            SourceSample::ready(Some(12.0)),
        );
    }

    struct SolidColor([u8; 3]);

    impl ColorSource for SolidColor {
        fn color(&self, _: f64, _: f64) -> Option<[u8; 3]> {
            Some(self.0)
        }
    }

    #[test]
    fn ground_terminal_fallback_preserves_geometry_and_dependency_status() {
        let mut registry = Registry::new();
        assert_eq!(
            registry.sample_elevation(0.0, 0.0, 1.0),
            SourceSample::ready(Some(0.0))
        );
        struct Missing;
        impl ElevationSource for Missing {
            fn elevation_m(&self, _: f64, _: f64, _: f64) -> f32 {
                0.0
            }
            fn sample_elevation(&self, _: f64, _: f64, _: f64) -> SourceSample<f32> {
                SourceSample::unavailable()
            }
        }
        registry.add_elevation(
            ProviderInfo {
                name: "missing ground".into(),
                resolution_m: 1.0,
                coverage: Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(Missing),
        );
        assert_eq!(
            registry.sample_elevation(0.0, 0.0, 1.0),
            SourceSample {
                value: Some(0.0),
                complete: false
            }
        );
    }

    #[test]
    fn calendar_month_follows_imagery_and_invalidates_only_on_change() {
        let mut registry = Registry::new();
        assert_eq!(registry.month(), None);
        registry.add_imagery(
            ProviderInfo {
                name: "monthly".into(),
                resolution_m: 2000.0,
                coverage: Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(crate::surface_pack::BundledSurface::for_month(12)),
        );
        assert_eq!(registry.month(), Some(12));
        assert!(!registry.set_imagery_month(12));
        assert!(registry.set_imagery_month(1));
        assert_eq!(registry.month(), Some(1));
        assert!(!registry.generated_cache_allowed());
        assert!(!registry.set_imagery_month(1));
        assert!(!registry.set_imagery_month(0));
        assert_eq!(registry.month(), Some(1));
        assert!(registry.set_imagery_month(2));
        assert!(registry.set_imagery_month(12));
        assert_eq!(registry.month(), Some(12));
    }

    #[test]
    fn finest_covering_provider_wins() {
        let mut reg = Registry::new();
        reg.add_elevation(
            ProviderInfo {
                name: "global-coarse".into(),
                resolution_m: 30.0,
                coverage: Coverage::Global,
                attribution: "global".into(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(FnElevation(|_, _| 100.0)),
        );
        reg.add_elevation(
            ProviderInfo {
                name: "nl-lidar".into(),
                resolution_m: 0.5,
                coverage: Coverage::LatLonBox(50.5, 53.7, 3.2, 7.3),
                attribution: "AHN".into(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(FnElevation(|_, _| 5.0)),
        );
        // Amsterdam → fine provider; Alps → global fallback.
        let ams = reg.elevation_m(52.37f64.to_radians(), 4.9f64.to_radians(), 1.0);
        let alps = reg.elevation_m(46.5f64.to_radians(), 8.0f64.to_radians(), 1.0);
        assert_eq!(ams, 5.0);
        assert_eq!(alps, 100.0);
        assert_eq!(reg.attributions().len(), 2);
        let entries = reg.attribution_entries();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.layer == "Terrain Height"));
        // Finest-first ordering carries into the attribution screen.
        assert_eq!(entries[0].name, "nl-lidar");
    }

    #[test]
    fn provider_layers_use_effect_based_control_groups() {
        assert_eq!(ProviderLayer::Elevation.label(), "Terrain Height");
        assert_eq!(ProviderLayer::Surface.label(), "Building Surfaces");
        assert_eq!(ProviderLayer::Imagery.label(), "Appearance");
        assert_eq!(ProviderLayer::Vectors.label(), "Map Features");
    }

    #[test]
    fn nodata_falls_through_to_coarser_provider() {
        let mut reg = Registry::new();
        reg.add_elevation(
            ProviderInfo {
                name: "global".into(),
                resolution_m: 30.0,
                coverage: Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(FnElevation(|_, _| -50.0)),
        );
        struct NoData;
        impl crate::ElevationSource for NoData {
            fn elevation_m(&self, _: f64, _: f64, _: f64) -> f32 {
                0.0
            }
            fn elevation_opt(&self, _: f64, _: f64, _: f64) -> Option<f32> {
                None // land-only source over water
            }
        }
        reg.add_elevation(
            ProviderInfo {
                name: "fine-land-only".into(),
                resolution_m: 1.0,
                coverage: Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(NoData),
        );
        // The fine provider covers but has no data: bathymetry must come
        // from the global source, not read as 0.
        assert_eq!(reg.elevation_m(0.1, 0.1, 1.0), -50.0);
    }

    #[test]
    fn provider_controls_default_on_and_disable_sampling_immediately() {
        let mut reg = Registry::new();
        for (name, resolution, elevation) in [("coarse", 30.0, 100.0), ("fine", 1.0, 5.0)] {
            reg.add_elevation(
                ProviderInfo {
                    name: name.into(),
                    resolution_m: resolution,
                    coverage: Coverage::Global,
                    attribution: String::new(),
                    license: String::new(),
                    url: String::new(),
                },
                Arc::new(FnElevation(move |_, _| elevation)),
            );
        }

        let controls = reg.provider_controls();
        assert_eq!(controls.len(), 2);
        assert!(controls.iter().all(|control| control.enabled));
        assert_eq!(reg.elevation_m(0.0, 0.0, 1.0), 5.0);
        assert!(reg.generated_cache_allowed());

        assert!(reg.set_provider_enabled(
            &ProviderId {
                layer: ProviderLayer::Elevation,
                name: "fine".into(),
            },
            false,
        ));
        assert_eq!(reg.elevation_m(0.0, 0.0, 1.0), 100.0);
        assert!(!reg.generated_cache_allowed());
        assert!(!reg.set_provider_enabled(
            &ProviderId {
                layer: ProviderLayer::Elevation,
                name: "fine".into(),
            },
            false,
        ));
    }

    #[test]
    fn disabled_elevation_is_available_but_opt_in() {
        let mut reg = Registry::new();
        reg.add_elevation(
            ProviderInfo {
                name: "global".into(),
                resolution_m: 30.0,
                coverage: Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(FnElevation(|_, _| 100.0)),
        );
        reg.add_elevation_disabled(
            ProviderInfo {
                name: "regional opt-in".into(),
                resolution_m: 1.0,
                coverage: Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(FnElevation(|_, _| 5.0)),
        );

        let controls = reg.provider_controls();
        assert_eq!(controls.len(), 2);
        assert!(
            controls
                .iter()
                .find(|control| control.id.name == "global")
                .unwrap()
                .enabled
        );
        assert!(
            !controls
                .iter()
                .find(|control| control.id.name == "regional opt-in")
                .unwrap()
                .enabled
        );
        assert_eq!(reg.elevation_m(0.0, 0.0, 1.0), 100.0);
        assert!(reg.generated_cache_allowed());

        assert!(reg.set_provider_enabled(
            &ProviderId {
                layer: ProviderLayer::Elevation,
                name: "regional opt-in".into(),
            },
            true,
        ));
        assert_eq!(reg.elevation_m(0.0, 0.0, 1.0), 5.0);
        assert!(!reg.generated_cache_allowed());
    }

    #[test]
    fn disabled_surface_is_available_but_opt_in() {
        let mut reg = Registry::new();
        reg.add_surface_disabled(
            ProviderInfo {
                name: "regional surface opt-in".into(),
                resolution_m: 1.0,
                coverage: Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(FnElevation(|_, _| 42.0)),
        );

        let control = reg
            .provider_controls()
            .into_iter()
            .find(|control| control.id.name == "regional surface opt-in")
            .unwrap();
        assert_eq!(control.id.layer, ProviderLayer::Surface);
        assert!(!control.enabled);
        assert!(reg.surface().is_none());
        assert!(reg.generated_cache_allowed());

        assert!(reg.set_provider_enabled(&control.id, true));
        assert_eq!(reg.surface().unwrap().elevation_m(0.0, 0.0, 1.0), 42.0);
        assert!(!reg.generated_cache_allowed());
    }

    #[test]
    fn disabled_imagery_is_available_but_opt_in() {
        let mut reg = Registry::new();
        reg.add_imagery(
            ProviderInfo {
                name: "global".into(),
                resolution_m: 7_000.0,
                coverage: Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(SolidColor([10, 20, 30])),
        );
        reg.add_imagery_disabled(
            ProviderInfo {
                name: "optional imagery".into(),
                resolution_m: 0.1,
                coverage: Coverage::Global,
                attribution: String::new(),
                license: String::new(),
                url: String::new(),
            },
            Arc::new(SolidColor([200, 210, 220])),
        );

        let controls = reg.provider_controls();
        assert_eq!(controls.len(), 2);
        assert!(
            controls
                .iter()
                .find(|control| control.id.name == "global")
                .unwrap()
                .enabled
        );
        assert!(
            !controls
                .iter()
                .find(|control| control.id.name == "optional imagery")
                .unwrap()
                .enabled
        );
        assert_eq!(reg.color(0.0, 0.0), Some([10, 20, 30]));
        assert!(reg.generated_cache_allowed());

        assert!(reg.set_provider_enabled(
            &ProviderId {
                layer: ProviderLayer::Imagery,
                name: "optional imagery".into(),
            },
            true,
        ));
        assert_eq!(reg.color(0.0, 0.0), Some([200, 210, 220]));
        assert!(!reg.generated_cache_allowed());
    }

    #[test]
    fn wrapped_coverage_contains_both_sides_of_antimeridian() {
        let coverage = Coverage::LatLonBox(-10.0, 10.0, 170.0, -170.0);
        assert!(coverage.contains(0.0, 175.0f64.to_radians()));
        assert!(coverage.contains(0.0, (-175.0f64).to_radians()));
        assert!(!coverage.contains(0.0, 0.0));
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn geometry_preparation_warms_only_covering_height_sources_and_preserves_failures() {
        use crate::async_prep::{
            AsyncSourcePreparer, DependencyOutcome, PreparationCoverage, PrepareFuture,
            PrepareRequest, PrepareSummary, SourceFootprint,
        };
        use std::sync::Mutex;

        struct PreparedSource {
            calls: Mutex<Vec<(PrepareRequest, PreparationCoverage)>>,
            outcome: DependencyOutcome,
        }
        impl ElevationSource for PreparedSource {
            fn elevation_m(&self, _: f64, _: f64, _: f64) -> f32 {
                assert!(!self.calls.lock().unwrap().is_empty());
                123.0
            }
        }
        impl ColorSource for PreparedSource {
            fn color(&self, _: f64, _: f64) -> Option<[u8; 3]> {
                None
            }
        }
        impl VectorSource for PreparedSource {
            fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
                FeaturePack::empty()
            }
        }
        impl AsyncSourcePreparer for PreparedSource {
            fn prepare<'a>(&'a self, _: &'a PrepareRequest) -> PrepareFuture<'a> {
                panic!("registered provider coverage must reach the preparer")
            }

            fn prepare_for_coverage<'a>(
                &'a self,
                request: &'a PrepareRequest,
                coverage: PreparationCoverage,
            ) -> PrepareFuture<'a> {
                Box::pin(async move {
                    self.calls.lock().unwrap().push((request.clone(), coverage));
                    let mut summary = PrepareSummary::default();
                    summary.record(self.outcome);
                    summary
                })
            }
        }
        let source = |outcome| {
            Arc::new(PreparedSource {
                calls: Mutex::new(Vec::new()),
                outcome,
            })
        };
        let info = |name: &str, coverage| ProviderInfo {
            name: name.into(),
            resolution_m: 1.0,
            coverage,
            attribution: String::new(),
            license: String::new(),
            url: String::new(),
        };
        let request = PrepareRequest {
            terrain_tiles: vec![(14, 8192, 8192)],
            footprint: SourceFootprint {
                center_lat: 0.0,
                center_lon: 0.0,
                radius_rad: 0.01,
                sample_hint_m: 8.0,
            },
            vector_bounds: None,
            night_building_bounds: None,
            priority: 7.0,
        };
        let runtime = vearth_platform::rt::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut registry = Registry::new();
        let elevation = source(DependencyOutcome::Ready);
        let surface = source(DependencyOutcome::Ready);
        registry.add_elevation_async(info("elevation", Coverage::Global), elevation.clone());
        registry.add_surface_async(
            info("surface", Coverage::LatLonBox(-1.0, 1.0, -1.0, 1.0)),
            surface.clone(),
        );
        let omitted: Vec<_> = (0..6)
            .map(|_| source(DependencyOutcome::TransientFailure))
            .collect();
        registry.add_elevation_async_disabled(
            info("disabled elevation", Coverage::Global),
            omitted[0].clone(),
        );
        registry.add_surface_async_disabled(
            info("disabled surface", Coverage::Global),
            omitted[1].clone(),
        );
        let outside = Coverage::LatLonBox(10.0, 11.0, 10.0, 11.0);
        registry.add_elevation_async(
            info("distant elevation", outside.clone()),
            omitted[2].clone(),
        );
        registry.add_surface_async(info("distant surface", outside), omitted[3].clone());
        registry.add_imagery_async(info("imagery", Coverage::Global), omitted[4].clone());
        registry.add_vectors_async(info("vectors", Coverage::Global), omitted[5].clone());

        let summary = runtime.block_on(registry.prepare_geometry_async(&request, false));
        assert_eq!(
            summary,
            PrepareSummary {
                ready: 1,
                ..Default::default()
            }
        );
        assert!(summary.complete());
        assert_eq!(
            *elevation.calls.lock().unwrap(),
            [(request.clone(), PreparationCoverage::Global)]
        );
        assert!(surface.calls.lock().unwrap().is_empty());
        let roof_request = PrepareRequest {
            footprint: SourceFootprint {
                sample_hint_m: 1.0,
                ..request.footprint
            },
            ..request.clone()
        };
        let summary = runtime.block_on(registry.prepare_geometry_async(&roof_request, true));
        assert_eq!(
            summary,
            PrepareSummary {
                ready: 1,
                ..Default::default()
            }
        );
        assert_eq!(elevation.calls.lock().unwrap().len(), 1);
        assert_eq!(
            *surface.calls.lock().unwrap(),
            [(
                roof_request.clone(),
                PreparationCoverage::LatLonBox(-1.0, 1.0, -1.0, 1.0)
            )]
        );
        assert!(
            omitted
                .iter()
                .all(|source| source.calls.lock().unwrap().is_empty())
        );
        assert_eq!(registry.elevation_m(0.0, 0.0, 1.0), 123.0);
        assert_eq!(
            registry.surface().unwrap().elevation_m(0.0, 0.0, 1.0),
            123.0
        );

        for outcome in [
            DependencyOutcome::NoData,
            DependencyOutcome::TransientFailure,
            DependencyOutcome::CircuitOpenCacheMiss,
        ] {
            let mut registry = Registry::new();
            registry
                .add_elevation_async(info("elevation outcome", Coverage::Global), source(outcome));
            registry.add_surface_async(info("surface outcome", Coverage::Global), source(outcome));
            for (request, surface_only) in [(&request, false), (&roof_request, true)] {
                let summary =
                    runtime.block_on(registry.prepare_geometry_async(request, surface_only));
                let mut expected = PrepareSummary::default();
                expected.record(outcome);
                assert_eq!(summary, expected);
                assert_eq!(summary.complete(), outcome == DependencyOutcome::NoData);
            }
        }

        // Splitting the initial layers must preserve the full API's exact
        // enabled/coverage-filtered requests and dependency outcomes.
        let all_sources: Vec<_> = [&elevation, &surface].into_iter().chain(&omitted).collect();
        for source in &all_sources {
            source.calls.lock().unwrap().clear();
        }
        let rasters = runtime.block_on(registry.prepare_raster_retained_async(&request));
        assert_eq!(rasters.summary.ready, 2);
        assert_eq!(rasters.summary.transient_failures, 1);
        assert!(omitted[5].calls.lock().unwrap().is_empty());
        let vectors = runtime.block_on(registry.prepare_vector_retained_async(&request));
        assert_eq!(vectors.summary.ready, 0);
        assert_eq!(vectors.summary.transient_failures, 1);
        let split_calls: Vec<_> = all_sources
            .iter()
            .map(|source| std::mem::take(&mut *source.calls.lock().unwrap()))
            .collect();
        assert!(split_calls[2..6].iter().all(Vec::is_empty));
        let mut split_summary = rasters.summary;
        split_summary.combine(vectors.summary);
        let combined = runtime.block_on(registry.prepare_retained_async(&request));
        assert_eq!(combined.summary, split_summary);
        for (source, expected) in all_sources.iter().zip(split_calls) {
            assert_eq!(*source.calls.lock().unwrap(), expected);
        }

        let vector_id = ProviderId {
            layer: ProviderLayer::Vectors,
            name: "vectors".into(),
        };
        assert!(registry.set_provider_enabled(&vector_id, false));
        omitted[5].calls.lock().unwrap().clear();
        let disabled = runtime.block_on(registry.prepare_vector_retained_async(&request));
        assert_eq!(disabled.summary, PrepareSummary::default());
        assert!(omitted[5].calls.lock().unwrap().is_empty());
        let distant_vectors = source(DependencyOutcome::TransientFailure);
        registry.add_vectors_async(
            info(
                "distant vectors",
                Coverage::LatLonBox(40.0, 41.0, 40.0, 41.0),
            ),
            distant_vectors.clone(),
        );
        let outside = runtime.block_on(registry.prepare_vector_retained_async(&request));
        assert_eq!(outside.summary, PrepareSummary::default());
        assert!(distant_vectors.calls.lock().unwrap().is_empty());
        let outside_request = PrepareRequest {
            footprint: SourceFootprint {
                center_lat: 20.0f64.to_radians(),
                center_lon: 20.0f64.to_radians(),
                ..request.footprint
            },
            ..request
        };
        surface.calls.lock().unwrap().clear();
        let distant = runtime.block_on(registry.prepare_raster_retained_async(&outside_request));
        assert_eq!(distant.summary.ready, 1);
        assert_eq!(distant.summary.transient_failures, 1);
        assert!(surface.calls.lock().unwrap().is_empty());
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_coverage_envelope_wraps_across_the_antimeridian() {
        let footprint = crate::async_prep::SourceFootprint {
            center_lat: 0.0,
            center_lon: 179.5f64.to_radians(),
            radius_rad: 2.0f64.to_radians(),
            sample_hint_m: 10.0,
        };
        assert!(coverage_intersects(
            &Coverage::LatLonBox(-1.0, 1.0, -180.0, -179.0),
            footprint,
        ));
        assert!(!coverage_intersects(
            &Coverage::LatLonBox(-1.0, 1.0, -120.0, -110.0),
            footprint,
        ));
    }

    #[cfg(feature = "async-http")]
    #[test]
    fn async_coverage_cap_that_touches_a_pole_spans_all_longitudes() {
        let footprint = crate::async_prep::SourceFootprint {
            center_lat: 89.0f64.to_radians(),
            center_lon: 10.0f64.to_radians(),
            radius_rad: 2.0f64.to_radians(),
            sample_hint_m: 10.0,
        };
        assert!(coverage_intersects(
            &Coverage::LatLonBox(89.5, 90.0, -170.0, -160.0),
            footprint,
        ));
    }
}
