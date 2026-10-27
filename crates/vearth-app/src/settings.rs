//! Versioned user preferences and safe cache-maintenance requests.
//!
//! Preferences live in Application Support, separately from disposable
//! downloaded data. Cache deletion is scheduled for the next launch so no
//! open SQLite connection or generation worker can race the removal.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use vearth_data::registry::{ProviderControl, ProviderId, ProviderLayer};

const SETTINGS_SCHEMA_VERSION: u32 = 1;
const PREFERENCES_FILE: &str = "preferences.json";
const CACHE_CLEAR_FILE: &str = "clear-downloads-on-next-launch.json";
pub const DEFAULT_CACHE_LIMIT_GB: u64 = 12;
pub const CACHE_LIMIT_OPTIONS_GB: [u64; 4] = [4, 12, 24, 48];

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowPreference {
    Dark,
    #[default]
    Normal,
    Off,
}

impl ShadowPreference {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Normal, Self::Off];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Normal => "Normal",
            Self::Off => "Off",
        }
    }

    pub const fn strength(self) -> f32 {
        match self {
            Self::Dark => 1.0,
            Self::Normal => 0.5,
            Self::Off => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PanSpeedPreference {
    Slow,
    #[default]
    Standard,
    Fast,
    VeryFast,
}

impl PanSpeedPreference {
    pub const ALL: [Self; 4] = [Self::Slow, Self::Standard, Self::Fast, Self::VeryFast];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Slow => "Slow (1x)",
            Self::Standard => "Standard (1.5x)",
            Self::Fast => "Fast (2x)",
            Self::VeryFast => "Very fast (2.5x)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VehiclePreference {
    #[default]
    #[serde(alias = "riptide")]
    Kestrel,
    PhaseManta,
    Sparrow,
}

impl VehiclePreference {
    pub const ALL: [Self; 3] = [Self::Kestrel, Self::PhaseManta, Self::Sparrow];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Kestrel => "Kestrel",
            Self::PhaseManta => "Phase Manta",
            Self::Sparrow => "Sparrow",
        }
    }
}

/// Render resolution: caps the pixels per point the app renders at, trading
/// sharpness on high-density displays for frame rate. The interface keeps its
/// size; the system scales the rendered image to the window.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderResolutionPreference {
    #[default]
    Native,
    Balanced,
    Performance,
}

impl RenderResolutionPreference {
    pub const ALL: [Self; 3] = [Self::Native, Self::Balanced, Self::Performance];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Native => "Native",
            Self::Balanced => "Balanced (1.5x)",
            Self::Performance => "Performance (1x)",
        }
    }

    /// Largest device pixels per CSS pixel to render; `None` is uncapped.
    pub const fn max_pixel_ratio(self) -> Option<f64> {
        match self {
            Self::Native => None,
            Self::Balanced => Some(1.5),
            Self::Performance => Some(1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherQualityPreference {
    Low,
    #[default]
    Standard,
    High,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SolarSpeedPreference {
    #[default]
    Realtime,
    MinutePerSecond,
    TenMinutesPerSecond,
}

impl SolarSpeedPreference {
    pub const ALL: [Self; 3] = [
        Self::Realtime,
        Self::MinutePerSecond,
        Self::TenMinutesPerSecond,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Realtime => "Real time (1x)",
            Self::MinutePerSecond => "60x",
            Self::TenMinutesPerSecond => "600x",
        }
    }

    pub const fn scale(self) -> f64 {
        match self {
            Self::Realtime => 1.0,
            Self::MinutePerSecond => 60.0,
            Self::TenMinutesPerSecond => 600.0,
        }
    }

    pub fn from_scale(scale: f64) -> Self {
        if scale >= 330.0 {
            Self::TenMinutesPerSecond
        } else if scale >= 30.5 {
            Self::MinutePerSecond
        } else {
            Self::Realtime
        }
    }
}

impl WeatherQualityPreference {
    pub const ALL: [Self; 3] = [Self::Low, Self::Standard, Self::High];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Standard => "Standard",
            Self::High => "High",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindPreference {
    Off,
    #[default]
    Assisted,
    Full,
}

impl WindPreference {
    pub const ALL: [Self; 3] = [Self::Off, Self::Assisted, Self::Full];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Assisted => "Assisted",
            Self::Full => "Full",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ProviderPreference {
    pub layer: String,
    pub name: String,
    pub enabled: bool,
}

impl Default for ProviderPreference {
    fn default() -> Self {
        Self {
            layer: String::new(),
            name: String::new(),
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct UserPreferences {
    pub schema_version: u32,
    pub show_search: bool,
    pub show_settings_button: bool,
    pub show_debug: bool,
    pub show_solar: bool,
    pub show_compass: bool,
    pub show_loading: bool,
    pub show_attribution: bool,
    pub solar_speed: SolarSpeedPreference,
    pub twenty_four_hour: bool,
    pub weather_enabled: bool,
    pub weather_quality: WeatherQualityPreference,
    pub wind_influence: WindPreference,
    pub precipitation_camera_effects: bool,
    pub clouds_visible: bool,
    pub precipitation_visible: bool,
    pub building_night_lights: bool,
    pub shadows: ShadowPreference,
    pub color_grading: bool,
    pub filmic_saturation: bool,
    pub render_resolution: RenderResolutionPreference,
    pub pan_speed: PanSpeedPreference,
    pub selected_vehicle: VehiclePreference,
    pub cache_limit_gb: u64,
    pub providers: Vec<ProviderPreference>,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            show_search: true,
            show_settings_button: true,
            show_debug: false,
            show_solar: true,
            show_compass: true,
            show_loading: true,
            show_attribution: false,
            solar_speed: SolarSpeedPreference::Realtime,
            twenty_four_hour: false,
            weather_enabled: true,
            weather_quality: WeatherQualityPreference::Standard,
            wind_influence: WindPreference::Assisted,
            precipitation_camera_effects: false,
            clouds_visible: true,
            precipitation_visible: true,
            building_night_lights: true,
            shadows: ShadowPreference::default(),
            color_grading: true,
            filmic_saturation: true,
            render_resolution: RenderResolutionPreference::Native,
            pan_speed: PanSpeedPreference::Standard,
            selected_vehicle: VehiclePreference::Kestrel,
            cache_limit_gb: DEFAULT_CACHE_LIMIT_GB,
            providers: Vec::new(),
        }
    }
}

impl UserPreferences {
    pub fn normalize(&mut self) {
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        if !CACHE_LIMIT_OPTIONS_GB.contains(&self.cache_limit_gb) {
            self.cache_limit_gb = DEFAULT_CACHE_LIMIT_GB;
        }
        self.providers.retain(|provider| {
            layer_from_key(&provider.layer).is_some() && !provider.name.trim().is_empty()
        });
        self.providers
            .sort_by(|a, b| a.layer.cmp(&b.layer).then_with(|| a.name.cmp(&b.name)));
        self.providers
            .dedup_by(|a, b| a.layer == b.layer && a.name == b.name);
    }

    pub fn cache_budgets(&self) -> CacheBudgets {
        let total = self.cache_limit_gb.saturating_mul(1024 * 1024 * 1024);
        let generated = total / 3;
        CacheBudgets {
            source_bytes: total.saturating_sub(generated),
            generated_bytes: generated,
        }
    }

    pub fn provider_enabled(&self, id: &ProviderId) -> Option<bool> {
        let layer = layer_key(id.layer);
        self.providers
            .iter()
            .find(|provider| provider.layer == layer && provider.name == id.name)
            .map(|provider| provider.enabled)
    }

    pub fn set_provider(&mut self, id: &ProviderId, enabled: bool) {
        let layer = layer_key(id.layer).to_owned();
        if let Some(provider) = self
            .providers
            .iter_mut()
            .find(|provider| provider.layer == layer && provider.name == id.name)
        {
            provider.enabled = enabled;
            return;
        }
        self.providers.push(ProviderPreference {
            layer,
            name: id.name.clone(),
            enabled,
        });
        self.normalize();
    }

    pub fn capture_providers(&mut self, controls: &[ProviderControl]) {
        self.providers.clear();
        self.providers
            .extend(controls.iter().map(|control| ProviderPreference {
                layer: layer_key(control.id.layer).to_owned(),
                name: control.id.name.clone(),
                enabled: control.enabled,
            }));
        self.normalize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheBudgets {
    pub source_bytes: u64,
    pub generated_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct PendingCacheClear {
    pub source_downloads: bool,
    pub generated_terrain: bool,
}

impl PendingCacheClear {
    pub const fn all() -> Self {
        Self {
            source_downloads: true,
            generated_terrain: true,
        }
    }

    pub const fn is_empty(self) -> bool {
        !self.source_downloads && !self.generated_terrain
    }

    pub fn label(self) -> &'static str {
        match (self.source_downloads, self.generated_terrain) {
            (true, true) => "All downloaded data",
            (true, false) => "Source downloads",
            (false, true) => "Generated terrain",
            (false, false) => "Nothing",
        }
    }
}

pub struct PreferencesStore {
    path: PathBuf,
}

impl PreferencesStore {
    pub fn load(application_support: &Path) -> (Self, UserPreferences) {
        let path = application_support.join(PREFERENCES_FILE);
        let store = Self { path };
        let mut preferences = match vearth_platform::fs::read(&store.path) {
            Ok(bytes) => match serde_json::from_slice::<UserPreferences>(&bytes) {
                Ok(preferences) => preferences,
                Err(error) => {
                    log::warn!(
                        "preferences: could not read {}: {error}",
                        store.path.display()
                    );
                    UserPreferences::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                UserPreferences::default()
            }
            Err(error) => {
                log::warn!(
                    "preferences: could not open {}: {error}",
                    store.path.display()
                );
                UserPreferences::default()
            }
        };
        preferences.normalize();
        (store, preferences)
    }

    pub fn save(&self, preferences: &UserPreferences) -> std::io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| std::io::Error::other("preferences path has no parent"))?;
        vearth_platform::fs::create_dir_all(parent)?;
        let temporary = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(preferences).map_err(std::io::Error::other)?;
        vearth_platform::fs::write(&temporary, bytes)?;
        vearth_platform::fs::rename(temporary, &self.path)
    }
}

pub fn pending_cache_clear(application_support: &Path) -> Option<PendingCacheClear> {
    let path = application_support.join(CACHE_CLEAR_FILE);
    let bytes = vearth_platform::fs::read(path).ok()?;
    serde_json::from_slice::<PendingCacheClear>(&bytes)
        .ok()
        .filter(|pending| !pending.is_empty())
}

pub fn schedule_cache_clear(
    application_support: &Path,
    requested: PendingCacheClear,
) -> std::io::Result<PendingCacheClear> {
    let current = pending_cache_clear(application_support).unwrap_or_default();
    let pending = PendingCacheClear {
        source_downloads: current.source_downloads || requested.source_downloads,
        generated_terrain: current.generated_terrain || requested.generated_terrain,
    };
    vearth_platform::fs::create_dir_all(application_support)?;
    let path = application_support.join(CACHE_CLEAR_FILE);
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(&pending).map_err(std::io::Error::other)?;
    vearth_platform::fs::write(&temporary, bytes)?;
    vearth_platform::fs::rename(temporary, path)?;
    Ok(pending)
}

pub fn cancel_cache_clear(application_support: &Path) -> std::io::Result<()> {
    match vearth_platform::fs::remove_file(application_support.join(CACHE_CLEAR_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Apply a previously requested clear before any cache database is opened.
pub fn apply_pending_cache_clear(
    cache: &Path,
    application_support: &Path,
) -> std::io::Result<bool> {
    let Some(pending) = pending_cache_clear(application_support) else {
        return Ok(false);
    };
    if pending == PendingCacheClear::all() {
        remove_path_if_present(cache)?;
        vearth_platform::fs::create_dir_all(cache)?;
        cancel_cache_clear(application_support)?;
        return Ok(true);
    }
    if pending.source_downloads {
        // `dem` and `ortho` hold files left by removed regional sources.
        for name in ["terrarium", "dem", "ortho", "vectors"] {
            remove_path_if_present(&cache.join(name))?;
        }
        for name in [
            "source_tiles.sqlite3",
            "source_tiles.sqlite3-wal",
            "source_tiles.sqlite3-shm",
            "source_tiles.sqlite3-journal",
        ] {
            remove_path_if_present(&cache.join(name))?;
        }
    }
    if pending.generated_terrain {
        remove_path_if_present(&cache.join("chunks"))?;
    }
    vearth_platform::fs::create_dir_all(cache)?;
    cancel_cache_clear(application_support)?;
    Ok(true)
}

fn remove_path_if_present(path: &Path) -> std::io::Result<()> {
    // Browser storage removes recursively and ignores missing entries.
    #[cfg(target_arch = "wasm32")]
    return vearth_platform::fs::remove_dir_all(path);
    #[cfg(not(target_arch = "wasm32"))]
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            vearth_platform::fs::remove_dir_all(path)
        }
        Ok(_) => vearth_platform::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheUsage {
    pub bytes: u64,
    pub files: u64,
}

pub struct CacheUsageTracker {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    cache: PathBuf,
    receiver: Option<Receiver<std::io::Result<CacheUsage>>>,
    usage: Option<CacheUsage>,
    error: Option<String>,
}

impl CacheUsageTracker {
    pub fn new(cache: PathBuf) -> Self {
        let mut tracker = Self {
            cache,
            receiver: None,
            usage: None,
            error: None,
        };
        tracker.refresh();
        tracker
    }

    pub fn refresh(&mut self) {
        if self.receiver.is_some() {
            return;
        }
        let (sender, receiver) = channel();
        // The browser reports the site's stored bytes; its storage cannot be
        // walked like a directory tree.
        #[cfg(target_arch = "wasm32")]
        {
            vearth_platform::rt::spawn_local_on_reactor(move || async move {
                let _ = sender.send(browser_storage_usage().await);
            });
            self.receiver = Some(receiver);
            self.error = None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let path = self.cache.clone();
        #[cfg(not(target_arch = "wasm32"))]
        if vearth_platform::thread::Builder::new()
            .name("cache-usage-scan".into())
            .spawn(move || {
                let _ = sender.send(scan_cache_usage(&path));
            })
            .is_ok()
        {
            self.receiver = Some(receiver);
            self.error = None;
        } else {
            self.error = Some("Could not start storage scan".into());
        }
    }

    pub fn poll(&mut self) {
        let result = match self.receiver.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Disconnected)) => Some(Err(std::io::Error::other(
                "storage scan stopped unexpectedly",
            ))),
            Some(Err(TryRecvError::Empty)) | None => None,
        };
        let Some(result) = result else { return };
        self.receiver = None;
        match result {
            Ok(usage) => {
                self.usage = Some(usage);
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub fn usage(&self) -> Option<CacheUsage> {
        self.usage
    }

    pub fn scanning(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(target_arch = "wasm32")]
async fn browser_storage_usage() -> std::io::Result<CacheUsage> {
    use wasm_bindgen::JsCast;
    let unavailable = || std::io::Error::other("the browser did not report storage usage");
    let navigator = js_sys::global()
        .unchecked_into::<web_sys::WorkerGlobalScope>()
        .navigator();
    let promise = navigator.storage().estimate().map_err(|_| unavailable())?;
    let estimate = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|_| unavailable())?;
    let bytes = js_sys::Reflect::get(&estimate, &"usage".into())
        .ok()
        .and_then(|usage| usage.as_f64())
        .ok_or_else(unavailable)?;
    Ok(CacheUsage {
        bytes: bytes as u64,
        files: 0,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn scan_cache_usage(root: &Path) -> std::io::Result<CacheUsage> {
    let mut usage = CacheUsage::default();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                usage.files = usage.files.saturating_add(1);
                usage.bytes = usage.bytes.saturating_add(metadata.len());
            }
        }
    }
    Ok(usage)
}

fn layer_key(layer: ProviderLayer) -> &'static str {
    match layer {
        ProviderLayer::Elevation => "elevation",
        ProviderLayer::Surface => "surface",
        ProviderLayer::Imagery => "imagery",
        ProviderLayer::Vectors => "vectors",
    }
}

fn layer_from_key(value: &str) -> Option<ProviderLayer> {
    match value {
        "elevation" => Some(ProviderLayer::Elevation),
        "surface" => Some(ProviderLayer::Surface),
        "imagery" => Some(ProviderLayer::Imagery),
        "vectors" => Some(ProviderLayer::Vectors),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "vearth-settings-{name}-{}-{}",
            std::process::id(),
            vearth_platform::time::SystemTime::now()
                .duration_since(vearth_platform::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn preferences_round_trip_and_unknown_fields_are_ignored() {
        let root = temporary_directory("round-trip");
        let (store, mut preferences) = PreferencesStore::load(&root);
        assert!(preferences.show_settings_button);
        preferences.show_settings_button = false;
        preferences.show_compass = false;
        preferences.cache_limit_gb = 24;
        store.save(&preferences).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(PREFERENCES_FILE)).unwrap()).unwrap();
        value["future_field"] = serde_json::json!(true);
        value["solar_cycle"] = serde_json::json!(false);
        std::fs::write(
            root.join(PREFERENCES_FILE),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let (_, loaded) = PreferencesStore::load(&root);
        assert!(!loaded.show_settings_button);
        assert!(!loaded.show_compass);
        assert_eq!(loaded.cache_limit_gb, 24);
        assert!(
            serde_json::to_value(&loaded)
                .unwrap()
                .get("solar_cycle")
                .is_none()
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn shadow_preferences_default_and_migrate_to_normal_and_persist_each_choice() {
        let root = temporary_directory("shadows");
        let (store, preferences) = PreferencesStore::load(&root);
        assert_eq!(preferences.shadows, ShadowPreference::Normal);
        assert_eq!(preferences.shadows.strength(), 0.5);
        // A pre-shadows preferences file must retain its other choices.
        std::fs::write(
            root.join(PREFERENCES_FILE),
            br#"{"show_compass":false,"color_grading":false}"#,
        )
        .unwrap();
        let (_, mut preferences) = PreferencesStore::load(&root);
        assert!(preferences.show_settings_button);
        assert_eq!(preferences.shadows, ShadowPreference::Normal);
        assert!(!preferences.show_compass);
        assert!(!preferences.color_grading);
        for (preset, name) in ShadowPreference::ALL
            .into_iter()
            .zip(["dark", "normal", "off"])
        {
            preferences.shadows = preset;
            store.save(&preferences).unwrap();
            let (_, loaded) = PreferencesStore::load(&root);
            assert_eq!(loaded, preferences);
            assert_eq!(serde_json::to_value(loaded.shadows).unwrap(), name);
        }
        assert_eq!(ShadowPreference::Dark.strength(), 1.0);
        assert_eq!(ShadowPreference::Off.strength(), 0.0);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn retired_vehicle_preference_migrates_without_resetting_other_settings() {
        let root = temporary_directory("retired-vehicle");
        std::fs::write(
            root.join(PREFERENCES_FILE),
            br#"{"selected_vehicle":"riptide","show_compass":false,"cache_limit_gb":24}"#,
        )
        .unwrap();
        let (store, preferences) = PreferencesStore::load(&root);
        assert_eq!(preferences.selected_vehicle, VehiclePreference::Kestrel);
        assert!(!preferences.show_compass);
        assert_eq!(preferences.cache_limit_gb, 24);
        store.save(&preferences).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(PREFERENCES_FILE)).unwrap()).unwrap();
        assert_eq!(value["selected_vehicle"], "kestrel");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn malformed_preferences_fall_back_to_defaults() {
        let root = temporary_directory("malformed");
        std::fs::write(root.join(PREFERENCES_FILE), b"not json").unwrap();
        let (_, loaded) = PreferencesStore::load(&root);
        assert_eq!(loaded, UserPreferences::default());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn scheduled_clear_is_scoped_and_keeps_preferences() {
        let root = temporary_directory("clear");
        let cache = root.join("cache");
        let support = root.join("support");
        std::fs::create_dir_all(cache.join("chunks/current")).unwrap();
        std::fs::create_dir_all(cache.join("vectors/build")).unwrap();
        std::fs::create_dir_all(&support).unwrap();
        std::fs::write(cache.join("chunks/current/a"), b"generated").unwrap();
        std::fs::write(cache.join("vectors/build/a"), b"source").unwrap();
        std::fs::write(support.join(PREFERENCES_FILE), b"preferences").unwrap();
        schedule_cache_clear(
            &support,
            PendingCacheClear {
                source_downloads: false,
                generated_terrain: true,
            },
        )
        .unwrap();

        assert!(apply_pending_cache_clear(&cache, &support).unwrap());
        assert!(!cache.join("chunks").exists());
        assert!(cache.join("vectors/build/a").exists());
        assert!(support.join(PREFERENCES_FILE).exists());
        assert!(pending_cache_clear(&support).is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn clear_everything_removes_the_cache_root_only() {
        let root = temporary_directory("clear-all");
        let cache = root.join("cache");
        let support = root.join("support");
        std::fs::create_dir_all(cache.join("future-cache-kind")).unwrap();
        std::fs::create_dir_all(&support).unwrap();
        std::fs::write(cache.join("future-cache-kind/data"), b"cache").unwrap();
        std::fs::write(support.join(PREFERENCES_FILE), b"preferences").unwrap();
        schedule_cache_clear(&support, PendingCacheClear::all()).unwrap();

        assert!(apply_pending_cache_clear(&cache, &support).unwrap());
        assert!(cache.exists());
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
        assert!(support.join(PREFERENCES_FILE).exists());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn usage_scan_does_not_follow_symlinks() {
        let root = temporary_directory("usage");
        let cache = root.join("cache");
        let outside = root.join("outside");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(cache.join("inside"), vec![0; 10]).unwrap();
        std::fs::write(outside.join("outside"), vec![0; 100]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, cache.join("link")).unwrap();

        let usage = scan_cache_usage(&cache).unwrap();
        assert_eq!(usage.bytes, 10);
        assert_eq!(usage.files, 1);
        std::fs::remove_dir_all(root).ok();
    }
}
