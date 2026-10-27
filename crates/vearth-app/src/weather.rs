//! Deterministic, stateless climate and weather sampling.
//!
//! Authoritative values are a pure function of model/atlas versions, the
//! bundled terrain pack, seed, simulation UTC and position.  No frame history,
//! wall clock, GPU result or runtime weather request participates in the result.

use glam::DVec3;
use std::f64::consts::TAU;
use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;
use vearth_core::ellipsoid;
use vearth_data::ElevationSource;
use vearth_data::climate::{CLIMATE_ATLAS_VERSION, ClimateAtlas};
use vearth_data::terrarium::MemoryTiles;

pub const WEATHER_MODEL_VERSION: u32 = 5;
pub const WEATHER_TICK_SECONDS: f64 = 600.0;
pub const DEFAULT_WEATHER_SEED: u64 = 0x7665_6172_7468_0001;
const LIGHTNING_EVENT_SECONDS: f64 = 6.0;
const LIGHTNING_REGION_RADIANS: f64 = 0.25_f64.to_radians();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WeatherMode {
    Clear,
    #[default]
    Procedural,
    Preset,
}

impl fmt::Display for WeatherMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Clear => "clear",
            Self::Procedural => "procedural",
            Self::Preset => "preset",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WeatherQuality {
    Low,
    #[default]
    Standard,
    High,
}

impl WeatherQuality {
    pub const fn shader_value(self) -> f32 {
        match self {
            Self::Low => 0.0,
            Self::Standard => 1.0,
            Self::High => 2.0,
        }
    }
}

impl FromStr for WeatherQuality {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "standard" => Ok(Self::Standard),
            "high" => Ok(Self::High),
            _ => Err(format!(
                "unknown weather quality {value:?}; expected low|standard|high"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindInfluence {
    Off,
    #[default]
    Assisted,
    Full,
}

impl FromStr for WindInfluence {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "assisted" => Ok(Self::Assisted),
            "full" => Ok(Self::Full),
            _ => Err(format!(
                "unknown wind influence {value:?}; expected off|assisted|full"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherConfig {
    pub mode: WeatherMode,
    pub quality: WeatherQuality,
    pub seed: u64,
    /// Stable hash of an authored preset id; zero means no preset.
    pub preset: u64,
    pub wind_influence: WindInfluence,
    pub precipitation_camera_effects: bool,
    /// Optional deterministic test controls applied after procedural sampling.
    pub overrides: WeatherOverrides,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            mode: WeatherMode::Procedural,
            quality: WeatherQuality::Standard,
            seed: DEFAULT_WEATHER_SEED,
            preset: 0,
            wind_influence: WindInfluence::Assisted,
            precipitation_camera_effects: false,
            overrides: WeatherOverrides::default(),
        }
    }
}

impl WeatherConfig {
    pub fn parse_mode(value: &str) -> Result<(WeatherMode, u64), String> {
        match value.to_ascii_lowercase().as_str() {
            "clear" => Ok((WeatherMode::Clear, 0)),
            "procedural" => Ok((WeatherMode::Procedural, 0)),
            _ => {
                let Some(id) = value.strip_prefix("preset:") else {
                    return Err(format!(
                        "unknown weather mode {value:?}; expected clear|procedural|preset:<id>"
                    ));
                };
                if id.is_empty() {
                    return Err("preset id may not be empty".into());
                }
                let preset = if let Some(hex) = id.strip_prefix('#') {
                    u64::from_str_radix(hex, 16).map_err(|_| {
                        "preset hash must contain 1–16 hexadecimal digits".to_string()
                    })?
                } else {
                    Self::preset_hash(id)
                };
                Ok((WeatherMode::Preset, preset))
            }
        }
    }

    /// Stable identifier used by authored presets in both CLI and UI paths.
    pub fn preset_hash(id: &str) -> u64 {
        stable_text_hash(id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrecipitationKind {
    None,
    Rain,
    Snow,
    Mixed,
    Hail,
}

impl PrecipitationKind {
    const fn shader_value(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Rain => 1.0,
            Self::Snow => 2.0,
            Self::Mixed => 3.0,
            Self::Hail => 4.0,
        }
    }
}

impl FromStr for PrecipitationKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "dry" => Ok(Self::None),
            "rain" => Ok(Self::Rain),
            "snow" => Ok(Self::Snow),
            "mixed" | "sleet" => Ok(Self::Mixed),
            "hail" => Ok(Self::Hail),
            _ => Err(format!(
                "unknown precipitation kind {value:?}; expected none|rain|snow|mixed|hail"
            )),
        }
    }
}

impl fmt::Display for PrecipitationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::None => "dry",
            Self::Rain => "rain",
            Self::Snow => "snow",
            Self::Mixed => "mixed",
            Self::Hail => "hail",
        })
    }
}

/// Deterministic CLI test controls for the primary visible weather state.
/// Derived cloud layers, optical depth and altitude winds remain internally
/// coherent with these controls.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeatherOverrides {
    pub air_temperature_c: Option<f32>,
    pub relative_humidity: Option<f32>,
    pub visibility_m: Option<f32>,
    pub precipitation_mm_h: Option<f32>,
    pub precipitation_kind: Option<PrecipitationKind>,
    pub cloud_base_m: Option<f32>,
    pub cloud_top_m: Option<f32>,
    pub cloud_coverage: Option<f32>,
    pub convective_intensity: Option<f32>,
    pub wetness: Option<f32>,
    pub snow_cover: Option<f32>,
    pub sea_ice: Option<f32>,
    pub sea_state: Option<f32>,
    pub vegetation_phase: Option<f32>,
    pub drought_stress: Option<f32>,
    pub aerosol: Option<f32>,
    pub lightning: Option<f32>,
    pub wind_east_mps: Option<f32>,
    pub wind_north_mps: Option<f32>,
    pub wind_up_mps: Option<f32>,
    pub wind_gust_mps: Option<f32>,
    pub turbulence: Option<f32>,
}

impl WeatherOverrides {
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

impl FromStr for WeatherOverrides {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let mut result = Self::default();
        if spec.trim().is_empty() {
            return Err("--weather-override expects comma-separated key=value pairs".into());
        }
        for assignment in spec.split(',') {
            let (raw_key, raw_value) = assignment
                .split_once('=')
                .ok_or_else(|| format!("weather override {assignment:?} must be key=value"))?;
            let key = raw_key.trim().to_ascii_lowercase().replace('-', "_");
            let value = raw_value.trim();
            let scalar = || {
                value
                    .parse::<f32>()
                    .map_err(|_| format!("weather override {raw_key:?} expects a number"))
            };
            let ranged = |min: f32, max: f32| {
                let parsed = scalar()?;
                if !parsed.is_finite() || !(min..=max).contains(&parsed) {
                    return Err(format!(
                        "weather override {raw_key:?} must be between {min} and {max}"
                    ));
                }
                Ok(parsed)
            };
            let unit = || ranged(0.0, 1.0);
            match key.as_str() {
                "temperature" | "temp" | "air_temperature_c" => {
                    result.air_temperature_c = Some(ranged(-100.0, 70.0)?)
                }
                "humidity" | "relative_humidity" => result.relative_humidity = Some(unit()?),
                "visibility" | "visibility_m" => {
                    result.visibility_m = Some(ranged(25.0, 250_000.0)?)
                }
                "precipitation" | "precip" | "precipitation_mm_h" => {
                    result.precipitation_mm_h = Some(ranged(0.0, 300.0)?)
                }
                "kind" | "precipitation_kind" => result.precipitation_kind = Some(value.parse()?),
                "cloud_base" | "cloud_base_m" => result.cloud_base_m = Some(ranged(0.0, 20_000.0)?),
                "cloud_top" | "cloud_top_m" => result.cloud_top_m = Some(ranged(50.0, 30_000.0)?),
                "cloud" | "cloud_coverage" => result.cloud_coverage = Some(unit()?),
                "convection" | "convective_intensity" => {
                    result.convective_intensity = Some(unit()?)
                }
                "wetness" => result.wetness = Some(unit()?),
                "snow" | "snow_cover" => result.snow_cover = Some(unit()?),
                "sea_ice" => result.sea_ice = Some(unit()?),
                "sea_state" => result.sea_state = Some(unit()?),
                "vegetation" | "vegetation_phase" => result.vegetation_phase = Some(unit()?),
                "drought" | "drought_stress" => result.drought_stress = Some(unit()?),
                "aerosol" => result.aerosol = Some(unit()?),
                "lightning" => result.lightning = Some(unit()?),
                "wind_east" | "wind_east_mps" => {
                    result.wind_east_mps = Some(ranged(-150.0, 150.0)?)
                }
                "wind_north" | "wind_north_mps" => {
                    result.wind_north_mps = Some(ranged(-150.0, 150.0)?)
                }
                "wind_up" | "wind_up_mps" => result.wind_up_mps = Some(ranged(-80.0, 80.0)?),
                "gust" | "wind_gust" | "wind_gust_mps" => {
                    result.wind_gust_mps = Some(ranged(0.0, 150.0)?)
                }
                "turbulence" => result.turbulence = Some(unit()?),
                _ => {
                    return Err(format!(
                        "unknown weather override {raw_key:?}; use temperature, humidity, visibility, precip, kind, cloud_base, cloud_top, cloud, convection, wetness, snow, sea_ice, sea_state, vegetation, drought, aerosol, lightning, wind_east, wind_north, wind_up, gust, or turbulence"
                    ));
                }
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindSample {
    pub east_mps: f32,
    pub north_mps: f32,
    pub up_mps: f32,
    pub gust_mps: f32,
    pub turbulence: f32,
}

impl WindSample {
    pub fn ecef(self, position_ecef: DVec3) -> DVec3 {
        let (east, north, up) = enu_at(position_ecef);
        east * f64::from(self.east_mps)
            + north * f64::from(self.north_mps)
            + up * f64::from(self.up_mps)
    }

    fn lerp(self, other: Self, alpha: f32) -> Self {
        Self {
            east_mps: lerp(self.east_mps, other.east_mps, alpha),
            north_mps: lerp(self.north_mps, other.north_mps, alpha),
            up_mps: lerp(self.up_mps, other.up_mps, alpha),
            gust_mps: lerp(self.gust_mps, other.gust_mps, alpha),
            turbulence: lerp(self.turbulence, other.turbulence, alpha),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherSample {
    pub air_temperature_c: f32,
    pub relative_humidity: f32,
    pub visibility_m: f32,
    pub precipitation_mm_h: f32,
    /// Monthly probability of measurable precipitation at an arbitrary hour.
    pub precipitation_wet_fraction: f32,
    /// Mean rate while precipitation is occurring, before the live storm anomaly.
    pub precipitation_conditional_mm_h: f32,
    pub precipitation_kind: PrecipitationKind,
    pub cloud_base_m: f32,
    pub cloud_top_m: f32,
    pub cloud_coverage: f32,
    pub low_cloud_coverage: f32,
    pub mid_cloud_coverage: f32,
    pub high_cloud_coverage: f32,
    pub cloud_optical_depth: f32,
    pub convective_intensity: f32,
    pub wetness: f32,
    pub snow_cover: f32,
    pub sea_ice: f32,
    pub sea_state: f32,
    pub vegetation_phase: f32,
    pub drought_stress: f32,
    pub aerosol: f32,
    pub aerosol_dust_fraction: f32,
    pub lightning: f32,
    pub wind: WindSample,
    pub wind_low: WindSample,
    pub wind_mid: WindSample,
    pub wind_upper: WindSample,
}

impl WeatherSample {
    pub const CLEAR: Self = Self {
        air_temperature_c: 15.0,
        relative_humidity: 0.35,
        visibility_m: 100_000.0,
        precipitation_mm_h: 0.0,
        precipitation_wet_fraction: 0.0,
        precipitation_conditional_mm_h: 0.0,
        precipitation_kind: PrecipitationKind::None,
        cloud_base_m: 2_000.0,
        cloud_top_m: 2_000.0,
        cloud_coverage: 0.0,
        low_cloud_coverage: 0.0,
        mid_cloud_coverage: 0.0,
        high_cloud_coverage: 0.0,
        cloud_optical_depth: 0.0,
        convective_intensity: 0.0,
        wetness: 0.0,
        snow_cover: 0.0,
        sea_ice: 0.0,
        sea_state: 0.0,
        vegetation_phase: 0.5,
        drought_stress: 0.0,
        aerosol: 0.08,
        aerosol_dust_fraction: 0.0,
        lightning: 0.0,
        wind: WindSample {
            east_mps: 0.0,
            north_mps: 0.0,
            up_mps: 0.0,
            gust_mps: 0.0,
            turbulence: 0.0,
        },
        wind_low: WindSample {
            east_mps: 0.0,
            north_mps: 0.0,
            up_mps: 0.0,
            gust_mps: 0.0,
            turbulence: 0.0,
        },
        wind_mid: WindSample {
            east_mps: 0.0,
            north_mps: 0.0,
            up_mps: 0.0,
            gust_mps: 0.0,
            turbulence: 0.0,
        },
        wind_upper: WindSample {
            east_mps: 0.0,
            north_mps: 0.0,
            up_mps: 0.0,
            gust_mps: 0.0,
            turbulence: 0.0,
        },
    };

    fn lerp(self, other: Self, alpha: f32) -> Self {
        let temperature = lerp(self.air_temperature_c, other.air_temperature_c, alpha);
        let precipitation = lerp(self.precipitation_mm_h, other.precipitation_mm_h, alpha);
        let convection = lerp(self.convective_intensity, other.convective_intensity, alpha);
        Self {
            air_temperature_c: temperature,
            relative_humidity: lerp(self.relative_humidity, other.relative_humidity, alpha),
            visibility_m: lerp(self.visibility_m, other.visibility_m, alpha),
            precipitation_mm_h: precipitation,
            precipitation_wet_fraction: lerp(
                self.precipitation_wet_fraction,
                other.precipitation_wet_fraction,
                alpha,
            ),
            precipitation_conditional_mm_h: lerp(
                self.precipitation_conditional_mm_h,
                other.precipitation_conditional_mm_h,
                alpha,
            ),
            precipitation_kind: precipitation_kind(temperature, precipitation, convection),
            cloud_base_m: lerp(self.cloud_base_m, other.cloud_base_m, alpha),
            cloud_top_m: lerp(self.cloud_top_m, other.cloud_top_m, alpha),
            cloud_coverage: lerp(self.cloud_coverage, other.cloud_coverage, alpha),
            low_cloud_coverage: lerp(self.low_cloud_coverage, other.low_cloud_coverage, alpha),
            mid_cloud_coverage: lerp(self.mid_cloud_coverage, other.mid_cloud_coverage, alpha),
            high_cloud_coverage: lerp(self.high_cloud_coverage, other.high_cloud_coverage, alpha),
            cloud_optical_depth: lerp(self.cloud_optical_depth, other.cloud_optical_depth, alpha),
            convective_intensity: convection,
            wetness: lerp(self.wetness, other.wetness, alpha),
            snow_cover: lerp(self.snow_cover, other.snow_cover, alpha),
            sea_ice: lerp(self.sea_ice, other.sea_ice, alpha),
            sea_state: lerp(self.sea_state, other.sea_state, alpha),
            vegetation_phase: lerp_wrapped(self.vegetation_phase, other.vegetation_phase, alpha),
            drought_stress: lerp(self.drought_stress, other.drought_stress, alpha),
            aerosol: lerp(self.aerosol, other.aerosol, alpha),
            aerosol_dust_fraction: lerp(
                self.aerosol_dust_fraction,
                other.aerosol_dust_fraction,
                alpha,
            ),
            lightning: lerp(self.lightning, other.lightning, alpha),
            wind: self.wind.lerp(other.wind, alpha),
            wind_low: self.wind_low.lerp(other.wind_low, alpha),
            wind_mid: self.wind_mid.lerp(other.wind_mid, alpha),
            wind_upper: self.wind_upper.lerp(other.wind_upper, alpha),
        }
    }

    fn apply_overrides(mut self, overrides: WeatherOverrides) -> Self {
        macro_rules! replace {
            ($field:ident) => {
                if let Some(value) = overrides.$field {
                    self.$field = value;
                }
            };
        }
        replace!(air_temperature_c);
        replace!(relative_humidity);
        replace!(visibility_m);
        replace!(precipitation_mm_h);
        replace!(cloud_base_m);
        replace!(cloud_top_m);
        replace!(cloud_coverage);
        replace!(convective_intensity);
        replace!(wetness);
        replace!(snow_cover);
        replace!(sea_ice);
        replace!(sea_state);
        replace!(vegetation_phase);
        replace!(drought_stress);
        replace!(aerosol);
        replace!(lightning);
        if let Some(value) = overrides.wind_east_mps {
            self.wind.east_mps = value;
        }
        if let Some(value) = overrides.wind_north_mps {
            self.wind.north_mps = value;
        }
        if let Some(value) = overrides.wind_up_mps {
            self.wind.up_mps = value;
        }
        if let Some(value) = overrides.wind_gust_mps {
            self.wind.gust_mps = value;
        }
        if let Some(value) = overrides.turbulence {
            self.wind.turbulence = value;
        }
        if overrides.cloud_coverage.is_some() {
            self.low_cloud_coverage = self.cloud_coverage;
            self.mid_cloud_coverage = self.cloud_coverage * 0.62;
            self.high_cloud_coverage = self.cloud_coverage * 0.38;
            self.cloud_optical_depth = (self.low_cloud_coverage * 2.6
                + self.mid_cloud_coverage * 2.0
                + self.high_cloud_coverage * 0.7)
                .clamp(0.0, 14.0);
        }
        if overrides.wind_east_mps.is_some()
            || overrides.wind_north_mps.is_some()
            || overrides.wind_up_mps.is_some()
            || overrides.wind_gust_mps.is_some()
            || overrides.turbulence.is_some()
        {
            self.wind_low = self.wind;
            self.wind_mid = self.wind;
            self.wind_upper = self.wind;
        }
        self.cloud_top_m = self.cloud_top_m.max(self.cloud_base_m + 50.0);
        self.precipitation_kind = overrides.precipitation_kind.unwrap_or_else(|| {
            precipitation_kind(
                self.air_temperature_c,
                self.precipitation_mm_h,
                self.convective_intensity,
            )
        });
        if self.precipitation_mm_h < 0.03 {
            self.precipitation_kind = PrecipitationKind::None;
        } else if self.precipitation_mm_h >= 2.0 && overrides.cloud_coverage.is_none() {
            let overcast = smoothstep(2.0, 14.0, f64::from(self.precipitation_mm_h)) as f32;
            self.low_cloud_coverage = self.low_cloud_coverage.max(0.76 + 0.22 * overcast);
            self.mid_cloud_coverage = self.mid_cloud_coverage.max(0.48 + 0.38 * overcast);
            self.high_cloud_coverage = self.high_cloud_coverage.max(0.24 + 0.42 * overcast);
            self.cloud_coverage = self.cloud_coverage.max(0.82 + 0.17 * overcast);
            self.cloud_optical_depth = self.cloud_optical_depth.max(4.5 + 7.5 * overcast);
        }
        self
    }
}

/// Values packed into the renderer's shared global uniform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherRenderState {
    /// temperature, humidity, visibility metres, precipitation mm/h
    pub weather0: [f32; 4],
    /// wetness, snow, cloud coverage, convective intensity
    pub weather1: [f32; 4],
    /// east, north, up, gust m/s
    pub wind: [f32; 4],
    /// vegetation phase, sea state, aerosol, quality (negative = clear)
    pub surface: [f32; 4],
    /// simulation time modulo, seed phase, phase code + camera-effects bit, lightning flash
    pub weather2: [f32; 4],
    /// local cloud base/top metres, sea ice, mode (0 clear, 1 live, 2 preset)
    pub weather3: [f32; 4],
    /// drought stress, turbulence, strike latitude radians, strike longitude radians
    pub weather4: [f32; 4],
    /// low, middle and high cloud fractions, cloud optical depth
    pub weather5: [f32; 4],
    /// dust fraction, wet-hour probability, conditional precipitation rate, year phase
    pub weather6: [f32; 4],
    /// Boundary-layer, middle-troposphere and upper-troposphere winds (east, north, up, gust).
    pub wind_low: [f32; 4],
    pub wind_mid: [f32; 4],
    pub wind_upper: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LightningEvent {
    flash: f32,
    strike_lat_rad: f32,
    strike_lon_rad: f32,
}

impl WeatherRenderState {
    pub fn new(
        sample: WeatherSample,
        config: WeatherConfig,
        utc_seconds: f64,
        position_ecef: DVec3,
    ) -> Self {
        let quality = if config.mode == WeatherMode::Clear {
            -1.0
        } else {
            config.quality.shader_value()
        };
        let lightning = lightning_event(sample.lightning, config, utc_seconds, position_ecef);
        Self {
            weather0: [
                sample.air_temperature_c,
                sample.relative_humidity,
                sample.visibility_m,
                sample.precipitation_mm_h,
            ],
            weather1: [
                sample.wetness,
                sample.snow_cover,
                sample.cloud_coverage,
                sample.convective_intensity,
            ],
            wind: [
                sample.wind.east_mps,
                sample.wind.north_mps,
                sample.wind.up_mps,
                sample.wind.gust_mps,
            ],
            surface: [
                sample.vegetation_phase,
                sample.sea_state,
                sample.aerosol,
                quality,
            ],
            weather2: [
                utc_seconds.rem_euclid(65_536.0) as f32,
                ((mix64(config.seed ^ config.preset) >> 40) as f32) / (1u32 << 24) as f32,
                sample.precipitation_kind.shader_value()
                    + if config.precipitation_camera_effects {
                        0.25
                    } else {
                        0.0
                    },
                lightning.flash,
            ],
            weather3: [
                sample.cloud_base_m,
                sample.cloud_top_m,
                sample.sea_ice,
                match config.mode {
                    WeatherMode::Clear => 0.0,
                    WeatherMode::Procedural => 1.0,
                    WeatherMode::Preset => 2.0,
                },
            ],
            weather4: [
                sample.drought_stress,
                sample.wind.turbulence,
                lightning.strike_lat_rad,
                lightning.strike_lon_rad,
            ],
            weather5: [
                sample.low_cloud_coverage,
                sample.mid_cloud_coverage,
                sample.high_cloud_coverage,
                sample.cloud_optical_depth,
            ],
            weather6: [
                sample.aerosol_dust_fraction,
                sample.precipitation_wet_fraction,
                sample.precipitation_conditional_mm_h,
                year_phase(utc_seconds) as f32,
            ],
            wind_low: [
                sample.wind_low.east_mps,
                sample.wind_low.north_mps,
                sample.wind_low.up_mps,
                sample.wind_low.gust_mps,
            ],
            wind_mid: [
                sample.wind_mid.east_mps,
                sample.wind_mid.north_mps,
                sample.wind_mid.up_mps,
                sample.wind_mid.gust_mps,
            ],
            wind_upper: [
                sample.wind_upper.east_mps,
                sample.wind_upper.north_mps,
                sample.wind_upper.up_mps,
                sample.wind_upper.gust_mps,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WeatherModel {
    pub config: WeatherConfig,
    atlas: ClimateAtlas,
}

impl WeatherModel {
    pub fn new(config: WeatherConfig) -> Self {
        Self {
            config,
            atlas: ClimateAtlas::bundled(),
        }
    }

    pub const fn atlas_version(&self) -> u32 {
        CLIMATE_ATLAS_VERSION
    }

    pub const fn model_version(&self) -> u32 {
        WEATHER_MODEL_VERSION
    }

    pub fn tick_pair(simulation_utc: f64) -> (i64, i64, f32) {
        let logical = simulation_utc / WEATHER_TICK_SECONDS;
        let tick0 = logical.floor() as i64;
        (tick0, tick0 + 1, logical.rem_euclid(1.0) as f32)
    }

    pub fn sample_weather(&self, position_ecef: DVec3, simulation_utc: f64) -> WeatherSample {
        let (lat, lon, altitude_m) = ellipsoid::ecef_to_geodetic(position_ecef);
        let (tick0, tick1, alpha) = Self::tick_pair(simulation_utc);
        let ground_m = regional_ground_m(lat, lon);
        let a = self.sample_tick(lat, lon, altitude_m, ground_m, tick0);
        let b = self.sample_tick(lat, lon, altitude_m, ground_m, tick1);
        a.lerp(b, alpha).apply_overrides(self.config.overrides)
    }

    pub fn render_state(&self, position_ecef: DVec3, simulation_utc: f64) -> WeatherRenderState {
        WeatherRenderState::new(
            self.sample_weather(position_ecef, simulation_utc),
            self.config,
            simulation_utc,
            position_ecef,
        )
    }

    fn sample_tick(
        &self,
        lat: f64,
        lon: f64,
        altitude_m: f64,
        ground_m: f64,
        tick: i64,
    ) -> WeatherSample {
        let utc = tick as f64 * WEATHER_TICK_SECONDS;
        let year_phase = year_phase(utc);
        let climate = self.atlas.sample(lat, lon, year_phase);
        if self.config.mode == WeatherMode::Clear {
            // Clear controls transient atmospheric effects, not the calendar
            // or regional growing conditions. Wet-hour frequency describes
            // monthly climate even when no precipitation is falling now.
            let humidity = climate.relative_humidity;
            let wet_fraction = climate.precipitation_wet_fraction;
            return WeatherSample {
                air_temperature_c: climate.mean_temperature_c
                    - (altitude_m.max(0.0) * 0.0065).min(35.0) as f32,
                relative_humidity: humidity,
                precipitation_wet_fraction: wet_fraction,
                precipitation_conditional_mm_h: (climate.precipitation_mm_day
                    / (24.0 * wet_fraction.max(0.006)))
                .clamp(0.0, 38.0),
                vegetation_phase: climate.phenology_phase,
                drought_stress: ((1.0 - humidity)
                    * (climate.mean_temperature_c / 32.0).clamp(0.0, 1.0)
                    * (1.0 - wet_fraction))
                    .clamp(0.0, 1.0),
                ..WeatherSample::CLEAR
            };
        }
        let base_key = self.config.seed ^ self.config.preset.rotate_left(17);
        let event_key = base_key ^ (tick as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let seed_phase = unit_hash(base_key) * TAU;
        let hours = utc / 3_600.0;

        // Smooth, world-anchored anomalies provide live weather without
        // replacing the observed monthly normals with latitude formulae.
        let synoptic = weather_noise(
            lat,
            lon,
            hours,
            base_key ^ 0x5359_4e4f_5054_4943,
            18.0,
            18.0,
        );
        let frontal = weather_noise(lat, lon, hours, base_key ^ 0x4652_4f4e_5441_4c53, 8.0, 7.0);
        let cell = weather_noise(lat, lon, hours, base_key ^ 0x434f_4e56_4543_544e, 3.5, 2.5);
        let pressure = (synoptic - 0.5) * 1.35 + (frontal - 0.5) * 0.55;
        let preset_intensity = if self.config.mode == WeatherMode::Preset {
            0.72 + 0.16 * (lon * 2.0 - lat + hours * 0.018 + seed_phase).cos()
        } else {
            0.0
        };
        let lapse = (altitude_m.max(0.0) * 0.0065).min(35.0);
        let surface_temperature =
            climate.mean_temperature_c as f64 + 4.2 * pressure + 1.2 * (cell - 0.5);
        let temperature = surface_temperature - lapse;
        let humidity = (climate.relative_humidity as f64
            + 0.20 * (-pressure).max(0.0)
            + 0.10 * (frontal - 0.5)
            + preset_intensity * 0.25)
            .clamp(0.03, 1.0);

        // Wet-hour frequency and mean monthly rainfall both come from the
        // atlas. The procedural field only determines which coherent storm
        // window is active and how strong that particular event is.
        let wet_fraction = (climate.precipitation_wet_fraction as f64 + preset_intensity * 0.32)
            .clamp(0.002, 0.90);
        let storm_signal = (0.52 * synoptic + 0.34 * frontal + 0.14 * cell).clamp(0.0, 1.0);
        let wet_threshold = (0.73 - wet_fraction * 0.76).clamp(0.12, 0.73);
        let mut rain_gate = smoothstep(wet_threshold - 0.045, wet_threshold + 0.045, storm_signal);
        if self.config.mode == WeatherMode::Preset {
            rain_gate = rain_gate.max(0.82);
        }
        let mut conditional_rate = (climate.precipitation_mm_day as f64
            / (24.0 * wet_fraction.max(0.006)))
        .clamp(0.0, 38.0);
        if self.config.mode == WeatherMode::Preset {
            conditional_rate = conditional_rate.max(5.5);
        }
        let lift = ((frontal - 0.42) * 1.05 + (-pressure).max(0.0) * 0.48).max(0.0);
        let instability =
            ((surface_temperature - 16.0) / 20.0).clamp(0.0, 1.0) * humidity * (0.45 + 0.55 * cell);
        let convection = (instability * lift * (0.4 + 0.8 * rain_gate) + preset_intensity * 0.72)
            .clamp(0.0, 1.0);
        let event_multiplier = 0.58 + 1.15 * cell + 2.15 * convection.powi(2);
        let precipitation = (rain_gate * conditional_rate * event_multiplier).clamp(0.0, 110.0);

        let storm_cloud = rain_gate * (0.34 + 0.42 * (precipitation / 12.0).clamp(0.0, 1.0));
        let mut low_cloud = (climate.low_cloud_fraction as f64
            + (0.5 - synoptic) * 0.20
            + storm_cloud
            + preset_intensity * 0.24)
            .clamp(0.0, 1.0);
        let mut mid_cloud = (climate.mid_cloud_fraction as f64
            + (frontal - 0.45) * 0.28
            + rain_gate * (0.18 + convection * 0.38))
            .clamp(0.0, 1.0);
        let mut high_cloud =
            (climate.high_cloud_fraction as f64 + (synoptic - 0.42) * 0.18 + convection * 0.38)
                .clamp(0.0, 1.0);
        if precipitation >= 2.0 {
            let overcast = smoothstep(2.0, 14.0, precipitation);
            low_cloud = low_cloud.max(0.76 + 0.22 * overcast);
            mid_cloud = mid_cloud.max(0.48 + 0.38 * overcast);
            high_cloud = high_cloud.max(0.24 + 0.42 * overcast);
        }
        let coverage =
            (1.0 - (1.0 - low_cloud) * (1.0 - mid_cloud) * (1.0 - high_cloud)).clamp(0.0, 1.0);
        let cloud_optical_depth = (low_cloud * 2.6
            + mid_cloud * 2.0
            + high_cloud * 0.7
            + rain_gate * (precipitation * 0.43).min(8.0))
        .clamp(0.0, 14.0);
        // The condensation level is a height above the regional ground; the
        // renderer and every other consumer receive it above the ellipsoid.
        let condensation_level = (2_750.0 - humidity * 2_250.0
            + surface_temperature.max(0.0) * 10.0)
            .clamp(120.0, 4_500.0);
        let cloud_base = ground_m + condensation_level;
        let cloud_top = (cloud_base + 850.0 + mid_cloud * 3_400.0 + convection * 8_500.0)
            .min(15_500.0)
            .max(cloud_base + 150.0);

        let aerosol = (climate.aerosol as f64 * (-precipitation * 0.035).exp()
            + climate.aerosol_dust_fraction as f64 * (1.0 - humidity) * 0.06)
            .clamp(0.015, 1.0);
        let humid_haze = smoothstep(0.72, 0.98, humidity) * (0.25 + aerosol);
        let aerosol_visibility = (-4.2 * (aerosol - 0.12).max(0.0)).exp();
        let visibility = (118_000.0 * (1.0 - 0.76 * humid_haze) * aerosol_visibility
            / (1.0 + precipitation * 0.10))
            .clamp(120.0, 120_000.0);

        // Closed recent-weather window: no dependency on app uptime.
        let prior_wave = weather_noise(
            lat,
            lon,
            hours - 3.0,
            base_key ^ 0x4652_4f4e_5441_4c53,
            8.0,
            7.0,
        );
        let recent_rain = precipitation * 0.62
            + (climate.precipitation_mm_day as f64 / 24.0) * (0.65 + 0.70 * prior_wave) * 3.0;
        let wetness = (1.0 - (-recent_rain * 0.095).exp())
            * (1.0 - (surface_temperature / 46.0).clamp(0.0, 0.65));
        let snow_factor = smoothstep(2.0, -4.0, surface_temperature);
        let snow_cover =
            (climate.snow_likelihood as f64 * 0.42 + wetness * snow_factor * 0.85).clamp(0.0, 1.0);

        let anomaly_east = (frontal - 0.5) * 4.2 + pressure * 1.6 + preset_intensity * 3.0;
        let anomaly_north = (synoptic - 0.5) * 3.4 * lat.sin().signum() + (cell - 0.5) * 1.4;
        let surface_east = climate.surface_wind_east_mps as f64 + anomaly_east * 0.42;
        let surface_north = climate.surface_wind_north_mps as f64 + anomaly_north * 0.42;
        let low_east = climate.low_wind_east_mps as f64 + anomaly_east * 0.68;
        let low_north = climate.low_wind_north_mps as f64 + anomaly_north * 0.68;
        let mid_east = climate.mid_wind_east_mps as f64 + anomaly_east;
        let mid_north = climate.mid_wind_north_mps as f64 + anomaly_north;
        let upper_east = climate.upper_wind_east_mps as f64 + anomaly_east * 0.72;
        let upper_north = climate.upper_wind_north_mps as f64 + anomaly_north * 0.72;
        let altitude_layer = (altitude_m / 10_000.0).clamp(0.0, 1.0);
        let roughness = climate.roughness as f64;
        let turbulence = (roughness * (1.0 - altitude_layer) * 0.42
            + convection * 0.72
            + preset_intensity * 0.25)
            .clamp(0.0, 1.0);
        let surface_speed = surface_east.hypot(surface_north);
        let gust = (surface_speed * (0.08 + 0.26 * turbulence)
            + 3.2 * (cell - 0.5).abs() * turbulence)
            .clamp(0.0, 22.0);
        let up = (convection * 4.5 * cell.max(0.0) - precipitation * 0.012).clamp(-2.0, 6.0);
        let layer = |east: f64, north: f64, vertical: f64, gust_scale: f64| WindSample {
            east_mps: east as f32,
            north_mps: north as f32,
            up_mps: vertical as f32,
            gust_mps: (gust * gust_scale) as f32,
            turbulence: turbulence as f32,
        };
        let wind_surface = layer(surface_east, surface_north, up * 0.22, 1.0);
        let wind_low = layer(low_east, low_north, up, 0.82);
        let wind_mid = layer(mid_east, mid_north, up * 0.58, 0.58);
        let wind_upper = layer(upper_east, upper_north, up * 0.18, 0.36);
        let wind = if altitude_m <= 1_800.0 {
            wind_surface.lerp(wind_low, (altitude_m.max(0.0) / 1_800.0) as f32)
        } else if altitude_m <= 5_500.0 {
            wind_low.lerp(wind_mid, ((altitude_m - 1_800.0) / 3_700.0) as f32)
        } else {
            wind_mid.lerp(
                wind_upper,
                ((altitude_m - 5_500.0) / 4_500.0).clamp(0.0, 1.0) as f32,
            )
        };
        let sea_state = ((surface_speed + gust * 0.45) / 24.0
            * (0.25 + 0.75 * climate.fetch as f64)
            + preset_intensity * 0.20)
            .clamp(0.0, 1.0);
        let drought_stress =
            ((1.0 - humidity) * (surface_temperature / 32.0).clamp(0.0, 1.0) * (1.0 - wetness))
                .clamp(0.0, 1.0);
        let lightning_gate = unit_hash(event_key ^ 0x4c49_4748_544e_494e);
        let lightning = if convection > 0.62 && lightning_gate > 0.78 {
            ((convection - 0.62) / 0.38) as f32
        } else {
            0.0
        };

        WeatherSample {
            air_temperature_c: temperature as f32,
            relative_humidity: humidity as f32,
            visibility_m: visibility as f32,
            precipitation_mm_h: precipitation as f32,
            precipitation_wet_fraction: wet_fraction as f32,
            precipitation_conditional_mm_h: conditional_rate as f32,
            precipitation_kind: precipitation_kind(
                surface_temperature as f32,
                precipitation as f32,
                convection as f32,
            ),
            cloud_base_m: cloud_base as f32,
            cloud_top_m: cloud_top as f32,
            cloud_coverage: coverage as f32,
            low_cloud_coverage: low_cloud as f32,
            mid_cloud_coverage: mid_cloud as f32,
            high_cloud_coverage: high_cloud as f32,
            cloud_optical_depth: cloud_optical_depth as f32,
            convective_intensity: convection as f32,
            wetness: wetness as f32,
            snow_cover: snow_cover as f32,
            sea_ice: (climate.sea_ice_likelihood as f64 * snow_factor) as f32,
            sea_state: sea_state as f32,
            vegetation_phase: climate.phenology_phase,
            drought_stress: drought_stress as f32,
            aerosol: aerosol as f32,
            aerosol_dust_fraction: climate.aerosol_dust_fraction,
            lightning,
            wind,
            wind_low,
            wind_mid,
            wind_upper,
        }
    }
}

fn precipitation_kind(temperature_c: f32, rate: f32, convection: f32) -> PrecipitationKind {
    if rate < 0.03 {
        PrecipitationKind::None
    } else if convection >= 0.82 && rate >= 8.0 && temperature_c > -3.0 {
        PrecipitationKind::Hail
    } else if temperature_c <= -0.8 {
        PrecipitationKind::Snow
    } else if temperature_c < 1.8 {
        PrecipitationKind::Mixed
    } else {
        PrecipitationKind::Rain
    }
}

/// A fast electrical discharge lives on top of the deliberately slow
/// ten-minute weather field. Keeping this envelope analytic makes pausing,
/// scrubbing, screenshots, and multiplayer replays agree without frame state.
fn lightning_flash_envelope(age_seconds: f64) -> f64 {
    fn pulse(age: f64, start: f64, attack: f64, decay: f64) -> f64 {
        let elapsed = age - start;
        if !(0.0..=attack + decay * 5.0).contains(&elapsed) {
            return 0.0;
        }
        if elapsed < attack {
            return smoothstep(0.0, attack, elapsed);
        }
        (-(elapsed - attack) / decay).exp()
    }

    // A dim leader, the return stroke, and one weaker restrike. The whole
    // sequence is under a second rather than being interpolated for minutes.
    (pulse(age_seconds, 0.0, 0.025, 0.045) * 0.48)
        .max(pulse(age_seconds, 0.105, 0.018, 0.115))
        .max(pulse(age_seconds, 0.285, 0.016, 0.070) * 0.42)
}

fn lightning_event(
    activity: f32,
    config: WeatherConfig,
    utc_seconds: f64,
    position_ecef: DVec3,
) -> LightningEvent {
    let (lat, lon, _) = ellipsoid::ecef_to_geodetic(position_ecef);
    let lat_cell = ((lat + std::f64::consts::FRAC_PI_2) / LIGHTNING_REGION_RADIANS).floor() as i64;
    let lon_cell = ((lon + std::f64::consts::PI) / LIGHTNING_REGION_RADIANS).floor() as i64;
    let slot = (utc_seconds / LIGHTNING_EVENT_SECONDS).floor() as i64;
    let region_key = (lat_cell as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (lon_cell as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let event_key = config.seed
        ^ config.preset.rotate_left(17)
        ^ region_key
        ^ (slot as u64).wrapping_mul(0x94d0_49bb_1331_11eb);

    // The strike remains fixed in geographic space for the event, so camera
    // motion produces parallax rather than dragging the bolt with the view.
    let strike_lat = (-std::f64::consts::FRAC_PI_2
        + (lat_cell as f64 + 0.15 + unit_hash(event_key ^ 0x4c41_5449_5455_4445) * 0.70)
            * LIGHTNING_REGION_RADIANS)
        .clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
    let strike_lon_unwrapped = -std::f64::consts::PI
        + (lon_cell as f64 + 0.15 + unit_hash(event_key ^ 0x4c4f_4e47_4954_5544) * 0.70)
            * LIGHTNING_REGION_RADIANS;
    let strike_lon = (strike_lon_unwrapped + std::f64::consts::PI)
        .rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI;

    let activity = f64::from(activity.clamp(0.0, 1.0));
    let chance = activity * (0.18 + activity * 0.72);
    let occurs = unit_hash(event_key ^ 0x4556_454e_545f_4741) < chance;
    let start = 0.30 + unit_hash(event_key ^ 0x5354_4152_545f_4f46) * 4.55;
    let age = utc_seconds.rem_euclid(LIGHTNING_EVENT_SECONDS) - start;
    let flash = if occurs {
        lightning_flash_envelope(age) * (0.38 + activity.sqrt() * 0.62)
    } else {
        0.0
    };

    // The renderer uses a spherical camera-relative planet. Encoding the
    // ellipsoid surface direction (rather than geodetic latitude directly)
    // prevents kilometre-scale vertical error away from the equator.
    let strike_surface = ellipsoid::geodetic_to_ecef(strike_lat, strike_lon, 0.0).normalize();
    let strike_geocentric_lat = strike_surface.z.asin();
    let strike_geocentric_lon = strike_surface.y.atan2(strike_surface.x);

    LightningEvent {
        flash: flash as f32,
        strike_lat_rad: strike_geocentric_lat as f32,
        strike_lon_rad: strike_geocentric_lon as f32,
    }
}

fn year_phase(utc_seconds: f64) -> f64 {
    // Unix epoch was 1 January. Offset by half a month so phase zero samples
    // the January midpoint, matching the atlas interpolation contract.
    (utc_seconds / (365.2425 * 86_400.0) - 14.5 / 365.2425).rem_euclid(1.0)
}

fn enu_at(position_ecef: DVec3) -> (DVec3, DVec3, DVec3) {
    let (lat, lon, _) = ellipsoid::ecef_to_geodetic(position_ecef);
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    (
        DVec3::new(-sin_lon, cos_lon, 0.0),
        DVec3::new(-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat),
        DVec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat),
    )
}

fn stable_text_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unit_hash(value: u64) -> f64 {
    (mix64(value) >> 11) as f64 / (1u64 << 53) as f64
}

/// Trilinear value noise on a longitude-wrapped latitude/time lattice. The
/// cell sizes are intentionally meteorological rather than visual: systems
/// persist for hours and hundreds of kilometres instead of boiling per frame.
/// Sampling pitch that keeps regional lookups at bundled zoom 2 or coarser
/// at every latitude (13–20 km pixels), so memory stays bounded.
const REGIONAL_GROUND_PITCH_M: f64 = 20_000.0;
const REGIONAL_GROUND_RADIUS_M: f64 = 20_000.0;

/// Terrain height that cloud bases are measured from. Convective boundary
/// layers follow a massif rather than the valley under the camera, so this
/// averages the bundled terrain over a ring about 40 km across. The fixed
/// ring varies smoothly with position and never waits on the network; sea
/// and missing tiles count as sea level.
fn regional_ground_m(lat: f64, lon: f64) -> f64 {
    static TERRAIN: OnceLock<MemoryTiles> = OnceLock::new();
    let terrain = TERRAIN.get_or_init(MemoryTiles::with_builtin_terrain);
    let radius = ellipsoid::MEAN_RADIUS;
    let height = |north_m: f64, east_m: f64| {
        let sample_lat = (lat + north_m / radius).clamp(-1.48, 1.48);
        let sample_lon = lon + east_m / (radius * lat.cos().max(0.05));
        f64::from(terrain.elevation_m(sample_lat, sample_lon, REGIONAL_GROUND_PITCH_M)).max(0.0)
    };
    let ring = (0..8)
        .map(|index| {
            let (sin, cos) = (f64::from(index) * TAU / 8.0).sin_cos();
            height(
                cos * REGIONAL_GROUND_RADIUS_M,
                sin * REGIONAL_GROUND_RADIUS_M,
            )
        })
        .sum::<f64>();
    height(0.0, 0.0) * 0.2 + ring * 0.1
}

fn weather_noise(
    lat: f64,
    lon: f64,
    hours: f64,
    seed: u64,
    cell_degrees: f64,
    cell_hours: f64,
) -> f64 {
    let nx = (360.0 / cell_degrees).round().max(4.0) as i64;
    let ny = (180.0 / cell_degrees).round().max(2.0) as i64;
    let x = (lon / TAU + 0.5).rem_euclid(1.0) * nx as f64;
    let y = ((lat / std::f64::consts::PI + 0.5).clamp(0.0, 1.0 - f64::EPSILON)) * ny as f64;
    let z = hours / cell_hours;
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let z0 = z.floor() as i64;
    let fade = |value: f64| value * value * (3.0 - 2.0 * value);
    let tx = fade(x.fract());
    let ty = fade(y.fract());
    let tz = fade(z.fract());
    let at = |ix: i64, iy: i64, iz: i64| {
        let key = seed
            ^ (ix.rem_euclid(nx) as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ (iy.clamp(0, ny - 1) as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
            ^ (iz as u64).wrapping_mul(0x94d0_49bb_1331_11eb);
        unit_hash(key)
    };
    let plane = |iz: i64| {
        let north = lerp64(at(x0, y0, iz), at(x0 + 1, y0, iz), tx);
        let south = lerp64(at(x0, y0 + 1, iz), at(x0 + 1, y0 + 1, iz), tx);
        lerp64(north, south, ty)
    };
    lerp64(plane(z0), plane(z0 + 1), tz)
}

fn smoothstep(edge0: f64, edge1: f64, value: f64) -> f64 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, alpha: f32) -> f32 {
    a + (b - a) * alpha
}

fn lerp64(a: f64, b: f64, alpha: f64) -> f64 {
    a + (b - a) * alpha
}

fn lerp_wrapped(a: f32, b: f32, alpha: f32) -> f32 {
    let delta = (b - a + 0.5).rem_euclid(1.0) - 0.5;
    (a + delta * alpha).rem_euclid(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sydney(altitude_m: f64) -> DVec3 {
        ellipsoid::geodetic_to_ecef(
            -33.8688_f64.to_radians(),
            151.2093_f64.to_radians(),
            altitude_m,
        )
    }

    #[test]
    fn same_key_is_byte_stable_and_evaluation_order_independent() {
        let model = WeatherModel::new(WeatherConfig::default());
        let time = 1_784_400_000.0;
        let positions = [
            sydney(40.0),
            ellipsoid::geodetic_to_ecef(42.0_f64.to_radians(), -71.0_f64.to_radians(), 900.0),
            ellipsoid::geodetic_to_ecef(35.0_f64.to_radians(), 139.0_f64.to_radians(), 20.0),
        ];
        let forward: Vec<_> = positions
            .iter()
            .map(|&p| model.sample_weather(p, time))
            .collect();
        let reverse: Vec<_> = positions
            .iter()
            .rev()
            .map(|&p| model.sample_weather(p, time))
            .collect();
        assert_eq!(forward[0], reverse[2]);
        assert_eq!(forward[1], reverse[1]);
        assert_eq!(forward[2], reverse[0]);
        assert_eq!(forward[0], model.sample_weather(positions[0], time));
    }

    #[test]
    fn cloud_base_is_measured_from_the_regional_ground() {
        let alps = regional_ground_m(46.0564_f64.to_radians(), 7.7776_f64.to_radians());
        let coast = regional_ground_m(52.37_f64.to_radians(), 4.90_f64.to_radians());
        let ocean = regional_ground_m(0.0, -30.0_f64.to_radians());
        assert!((1_500.0..3_500.0).contains(&alps), "{alps}");
        assert!(coast < 50.0, "{coast}");
        assert_eq!(ocean, 0.0);
        // One kilometre of travel moves the reference by metres, not by the
        // depth of the valley under the camera.
        let nearby = regional_ground_m(46.0654_f64.to_radians(), 7.7776_f64.to_radians());
        assert!((alps - nearby).abs() < 100.0, "{alps} -> {nearby}");
        let position =
            ellipsoid::geodetic_to_ecef(46.0564_f64.to_radians(), 7.7776_f64.to_radians(), 2_100.0);
        let sample =
            WeatherModel::new(WeatherConfig::default()).sample_weather(position, 1_785_044_040.0);
        let condensation_level = f64::from(sample.cloud_base_m) - alps;
        assert!(
            (120.0..=4_500.0).contains(&condensation_level),
            "{condensation_level} {sample:?}"
        );
    }

    #[test]
    fn rainy_alpine_media_regression_fixture_stays_inside_the_low_cloud_layer() {
        // Keep this classification stable: it exercises dense low cloud,
        // measurable rain, and a camera physically inside the cloud layer.
        // The layer sits above the regional ground of about 1.9 km.
        let camera_altitude_m = 3_800.0;
        let position = ellipsoid::geodetic_to_ecef(
            46.9801_f64.to_radians(),
            11.6391_f64.to_radians(),
            camera_altitude_m,
        );
        let sample =
            WeatherModel::new(WeatherConfig::default()).sample_weather(position, 1_785_044_040.0);
        assert!(sample.precipitation_mm_h > 1.0, "{sample:?}");
        assert!(sample.low_cloud_coverage > 0.80, "{sample:?}");
        assert!(sample.cloud_optical_depth > 4.0, "{sample:?}");
        assert!(sample.cloud_base_m < camera_altitude_m as f32, "{sample:?}");
        assert!(sample.cloud_top_m > camera_altitude_m as f32, "{sample:?}");
    }

    #[test]
    fn fixed_tick_interpolation_is_continuous_and_bounded() {
        let model = WeatherModel::new(WeatherConfig::default());
        let p = sydney(250.0);
        let edge = 2_900_000.0 * WEATHER_TICK_SECONDS;
        let before = model.sample_weather(p, edge - 0.001);
        let after = model.sample_weather(p, edge + 0.001);
        assert!((before.air_temperature_c - after.air_temperature_c).abs() < 0.01);
        assert!((before.wind.east_mps - after.wind.east_mps).abs() < 0.01);
        for value in [
            after.relative_humidity,
            after.cloud_coverage,
            after.wetness,
            after.snow_cover,
        ] {
            assert!((0.0..=1.0).contains(&value));
        }
    }

    #[test]
    fn time_jump_only_selects_adjacent_ticks() {
        let (a0, a1, _) = WeatherModel::tick_pair(0.0);
        let (b0, b1, _) = WeatherModel::tick_pair(180.0 * 86_400.0);
        assert_eq!(a1, a0 + 1);
        assert_eq!(b1, b0 + 1);
        assert!(b0 - a0 > 20_000);
    }

    #[test]
    fn wind_changes_smoothly_with_height() {
        let model = WeatherModel::new(WeatherConfig::default());
        let time = 1_784_400_000.0;
        let low = model.sample_weather(sydney(999.9), time).wind;
        let high = model.sample_weather(sydney(1_000.1), time).wind;
        assert!((low.east_mps - high.east_mps).abs() < 0.02);
        assert!((low.north_mps - high.north_mps).abs() < 0.02);
    }

    #[test]
    fn explicit_clear_path_disables_transient_weather() {
        let config = WeatherConfig {
            mode: WeatherMode::Clear,
            ..Default::default()
        };
        let model = WeatherModel::new(config);
        let sample = model.sample_weather(sydney(0.0), 123.0);
        assert_eq!(sample.precipitation_mm_h, 0.0);
        assert_eq!(sample.precipitation_kind, PrecipitationKind::None);
        assert_eq!(sample.cloud_coverage, 0.0);
        assert_eq!(sample.low_cloud_coverage, 0.0);
        assert_eq!(sample.mid_cloud_coverage, 0.0);
        assert_eq!(sample.high_cloud_coverage, 0.0);
        assert_eq!(sample.cloud_optical_depth, 0.0);
        assert_eq!(sample.convective_intensity, 0.0);
        assert_eq!(sample.lightning, 0.0);
        assert_eq!(sample.wetness, 0.0);
        assert_eq!(sample.snow_cover, 0.0);
        assert_eq!(sample.sea_state, 0.0);
        assert_eq!(sample.wind, WeatherSample::CLEAR.wind);
        assert_eq!(sample.wind_low, WeatherSample::CLEAR.wind_low);
        assert_eq!(sample.wind_mid, WeatherSample::CLEAR.wind_mid);
        assert_eq!(sample.wind_upper, WeatherSample::CLEAR.wind_upper);
        assert_eq!(sample, model.sample_weather(sydney(0.0), 123.0));
        assert!(model.render_state(sydney(0.0), 123.0).surface[3] < 0.0);
    }

    #[test]
    fn clear_weather_keeps_local_seasons_and_regional_growing_conditions() {
        let model = WeatherModel::new(WeatherConfig {
            mode: WeatherMode::Clear,
            ..Default::default()
        });
        let january = crate::solar::parse_utc_datetime("2026-01-15T12:00:00Z").unwrap();
        let july = crate::solar::parse_utc_datetime("2026-07-15T12:00:00Z").unwrap();
        let boston =
            ellipsoid::geodetic_to_ecef(42.0_f64.to_radians(), -71.0_f64.to_radians(), 0.0);
        let north_winter = model.sample_weather(boston, january);
        let north_summer = model.sample_weather(boston, july);
        let south_summer = model.sample_weather(sydney(0.0), january);
        let south_winter = model.sample_weather(sydney(0.0), july);
        assert!(north_summer.air_temperature_c > north_winter.air_temperature_c + 10.0);
        assert!(south_summer.air_temperature_c > south_winter.air_temperature_c + 5.0);
        assert!((north_summer.vegetation_phase - north_winter.vegetation_phase).abs() > 0.4);
        assert!((north_summer.vegetation_phase - south_summer.vegetation_phase).abs() < 0.02);
        let sahara = ellipsoid::geodetic_to_ecef(23.0_f64.to_radians(), 13.0_f64.to_radians(), 0.0);
        let congo = ellipsoid::geodetic_to_ecef(0.0, 20.0_f64.to_radians(), 0.0);
        let dry = model.sample_weather(sahara, july);
        let humid = model.sample_weather(congo, july);
        assert!(humid.relative_humidity > dry.relative_humidity);
        assert!(
            humid.precipitation_wet_fraction > dry.precipitation_wet_fraction,
            "humid={humid:?}; dry={dry:?}"
        );
        assert!(dry.drought_stress > humid.drought_stress);
        assert_eq!(north_winter, model.sample_weather(boston, january));
    }

    #[test]
    fn clear_weather_respects_explicit_surface_overrides() {
        let model = WeatherModel::new(WeatherConfig {
            mode: WeatherMode::Clear,
            overrides: "temperature=19,humidity=.7,vegetation=.2,drought=.1"
                .parse()
                .unwrap(),
            ..Default::default()
        });
        let sample = model.sample_weather(sydney(0.0), 1_784_400_000.0);
        assert_eq!(sample.air_temperature_c, 19.0);
        assert_eq!(sample.relative_humidity, 0.7);
        assert_eq!(sample.vegetation_phase, 0.2);
        assert_eq!(sample.drought_stress, 0.1);
        assert_eq!(sample.precipitation_mm_h, 0.0);
        assert_eq!(sample.cloud_coverage, 0.0);
    }

    #[test]
    fn lightning_is_a_sub_second_multi_stroke_flash() {
        assert_eq!(lightning_flash_envelope(-0.001), 0.0);
        assert!(lightning_flash_envelope(0.025) > 0.45);
        assert!(lightning_flash_envelope(0.123) > 0.95);
        assert!(lightning_flash_envelope(0.301) > 0.40);
        assert_eq!(lightning_flash_envelope(0.90), 0.0);
    }

    #[test]
    fn lightning_events_are_deterministic_and_geographically_fixed() {
        let config = WeatherConfig {
            seed: 99,
            overrides: WeatherOverrides {
                lightning: Some(1.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let position = sydney(200.0);
        let slot_start = 1_784_400_000.0;
        let mut active = None;
        for step in 0..1_200 {
            let utc = slot_start + f64::from(step) * 0.01;
            let event = lightning_event(1.0, config, utc, position);
            if event.flash > 0.01 {
                active = Some((utc, event));
                break;
            }
        }
        let (utc, event) = active.expect("maximum lightning activity should yield a nearby event");
        assert_eq!(event, lightning_event(1.0, config, utc, position));

        let nearby = position + enu_at(position).0 * 10.0;
        let moved = lightning_event(1.0, config, utc, nearby);
        assert_eq!(event.strike_lat_rad, moved.strike_lat_rad);
        assert_eq!(event.strike_lon_rad, moved.strike_lon_rad);
        assert!((event.strike_lat_rad as f64 - (-33.8688_f64).to_radians()).abs() < 0.01);
        assert!((event.strike_lon_rad as f64 - 151.2093_f64.to_radians()).abs() < 0.01);
    }

    #[test]
    fn precipitation_phase_tracks_temperature() {
        assert_eq!(precipitation_kind(-5.0, 1.0, 0.0), PrecipitationKind::Snow);
        assert_eq!(precipitation_kind(0.5, 1.0, 0.0), PrecipitationKind::Mixed);
        assert_eq!(precipitation_kind(10.0, 1.0, 0.0), PrecipitationKind::Rain);
        assert_eq!(precipitation_kind(18.0, 20.0, 0.9), PrecipitationKind::Hail);
        assert_eq!(precipitation_kind(-5.0, 0.0, 1.0), PrecipitationKind::None);
    }

    #[test]
    fn authored_preset_guarantees_visibly_active_weather() {
        let (mode, preset) = WeatherConfig::parse_mode("preset:storm-demo").unwrap();
        assert_eq!(preset, WeatherConfig::preset_hash("storm-demo"));
        let config = WeatherConfig {
            mode,
            preset,
            seed: 424_242,
            ..Default::default()
        };
        let utc = crate::solar::parse_utc_datetime("2026-07-19T06:30:00Z").unwrap();
        let sample = WeatherModel::new(config).sample_weather(sydney(150_000.0), utc);
        assert!(sample.cloud_coverage > 0.45, "{sample:?}");
        assert!(sample.convective_intensity > 0.40, "{sample:?}");
        assert!(sample.visibility_m > 100.0, "{sample:?}");
        assert!(
            sample.snow_cover < 0.35,
            "surface snow must not follow camera altitude: {sample:?}"
        );
    }

    #[test]
    fn cli_overrides_cover_weather_and_wind_fields() {
        let overrides: WeatherOverrides = "temperature=-4,humidity=.9,visibility=8000,precip=6,kind=snow,cloud_base=500,cloud_top=3200,cloud=.8,convection=.3,wetness=.7,snow=.9,sea_ice=.4,sea_state=.6,vegetation=.2,drought=.1,aerosol=.25,lightning=.05,wind_east=12,wind_north=-7,wind_up=1,gust=18,turbulence=.8".parse().unwrap();
        let model = WeatherModel::new(WeatherConfig {
            overrides,
            ..Default::default()
        });
        let sample = model.sample_weather(sydney(25.0), 1_784_400_000.0);
        assert_eq!(sample.air_temperature_c, -4.0);
        assert_eq!(sample.precipitation_kind, PrecipitationKind::Snow);
        assert_eq!(sample.cloud_coverage, 0.8);
        assert_eq!(sample.aerosol, 0.25);
        assert_eq!(sample.wind.east_mps, 12.0);
        assert_eq!(sample.wind.north_mps, -7.0);
        assert_eq!(sample.wind.turbulence, 0.8);
    }

    #[test]
    fn invalid_override_is_rejected_with_its_key() {
        let error = "humidity=1.5".parse::<WeatherOverrides>().unwrap_err();
        assert!(error.contains("humidity"), "{error}");
        assert!("mystery=1".parse::<WeatherOverrides>().is_err());
    }
}
