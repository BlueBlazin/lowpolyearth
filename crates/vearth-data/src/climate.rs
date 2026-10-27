//! Compact observed monthly climatology for deterministic offline weather.
//!
//! Atlas v2 is prepared offline at one-degree resolution from WorldClim 2.1
//! temperature/vapour-pressure/precipitation normals, NOAA/NCEP 1991–2020
//! surface and pressure-level wind/humidity normals, and the bundled NASA
//! Black Marble emission proxy used to distribute anthropogenic haze.  The
//! application never performs climate or air-quality network requests.

use flate2::read::ZlibDecoder;
use std::f64::consts::{PI, TAU};
use std::io::Read;
use std::sync::OnceLock;

pub const CLIMATE_ATLAS_VERSION: u32 = 2;
pub const CLIMATE_ATLAS_PROVENANCE: &str =
    "WorldClim 2.1 + NOAA/NCEP 1991-2020 normals + NASA Black Marble derived Climate Atlas v2";

fn compressed() -> &'static [u8] {
    vearth_platform::asset_bytes!("../assets/climate-atlas-v2.bin.z")
}
const HEADER_BYTES: usize = 16;
const WIDTH: usize = 360;
const HEIGHT: usize = 180;
const MONTHS: usize = 12;
const STRIDE: usize = 19;
const CELLS_PER_MONTH: usize = WIDTH * HEIGHT;
const DECOMPRESSED_BYTES: usize = MONTHS * CELLS_PER_MONTH * STRIDE;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateSample {
    pub mean_temperature_c: f32,
    pub precipitation_mm_day: f32,
    /// Fraction of hours expected to contain measurable precipitation.
    pub precipitation_wet_fraction: f32,
    pub relative_humidity: f32,
    pub surface_wind_east_mps: f32,
    pub surface_wind_north_mps: f32,
    pub low_wind_east_mps: f32,
    pub low_wind_north_mps: f32,
    pub mid_wind_east_mps: f32,
    pub mid_wind_north_mps: f32,
    pub upper_wind_east_mps: f32,
    pub upper_wind_north_mps: f32,
    pub low_cloud_fraction: f32,
    pub mid_cloud_fraction: f32,
    pub high_cloud_fraction: f32,
    pub wet_season_phase: f32,
    pub wet_season_amplitude: f32,
    pub freezing_level_m: f32,
    pub snow_likelihood: f32,
    pub sea_ice_likelihood: f32,
    /// Monthly mean total aerosol optical depth at 550 nm, art-direction capped.
    pub aerosol: f32,
    /// Coarse fraction of the aerosol column attributable to mineral dust.
    pub aerosol_dust_fraction: f32,
    pub land_fraction: f32,
    pub roughness: f32,
    /// 0 = sheltered/small water, 1 = open-ocean fetch.
    pub fetch: f32,
    pub phenology_phase: f32,
    pub phenology_amplitude: f32,
}

/// Time-neutral climate cues used by deterministic generated terrain.
/// Monthly weather remains dynamic; this annual summary only helps classify
/// persistent surface families such as desert, steppe, rainforest and ice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceClimate {
    pub mean_temperature_c: f32,
    pub precipitation_mm_day: f32,
    pub relative_humidity: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ClimateAtlas;

#[derive(Clone, Copy, Default)]
struct MonthlyCell {
    temperature_c: f32,
    humidity: f32,
    precipitation_mm_month: f32,
    wet_fraction: f32,
    low_cloud: f32,
    mid_cloud: f32,
    high_cloud: f32,
    surface_east: f32,
    surface_north: f32,
    low_east: f32,
    low_north: f32,
    mid_east: f32,
    mid_north: f32,
    upper_east: f32,
    upper_north: f32,
    aerosol: f32,
    dust_fraction: f32,
    land: f32,
}

impl MonthlyCell {
    fn lerp(self, other: Self, alpha: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * alpha;
        Self {
            temperature_c: mix(self.temperature_c, other.temperature_c),
            humidity: mix(self.humidity, other.humidity),
            precipitation_mm_month: mix(self.precipitation_mm_month, other.precipitation_mm_month),
            wet_fraction: mix(self.wet_fraction, other.wet_fraction),
            low_cloud: mix(self.low_cloud, other.low_cloud),
            mid_cloud: mix(self.mid_cloud, other.mid_cloud),
            high_cloud: mix(self.high_cloud, other.high_cloud),
            surface_east: mix(self.surface_east, other.surface_east),
            surface_north: mix(self.surface_north, other.surface_north),
            low_east: mix(self.low_east, other.low_east),
            low_north: mix(self.low_north, other.low_north),
            mid_east: mix(self.mid_east, other.mid_east),
            mid_north: mix(self.mid_north, other.mid_north),
            upper_east: mix(self.upper_east, other.upper_east),
            upper_north: mix(self.upper_north, other.upper_north),
            aerosol: mix(self.aerosol, other.aerosol),
            dust_fraction: mix(self.dust_fraction, other.dust_fraction),
            land: mix(self.land, other.land),
        }
    }
}

impl ClimateAtlas {
    pub const fn bundled() -> Self {
        Self
    }

    pub const fn version(self) -> u32 {
        CLIMATE_ATLAS_VERSION
    }

    pub const fn provenance(self) -> &'static str {
        CLIMATE_ATLAS_PROVENANCE
    }

    /// Annual observed normals for deterministic terrain generation.
    pub fn annual_surface(self, lat_rad: f64, lon_rad: f64) -> SurfaceClimate {
        let position = SurfacePosition::new(lat_rad, lon_rad);
        let mut temperature = 0.0;
        let mut precipitation_mm_month = 0.0;
        let mut humidity = 0.0;
        for month in 0..MONTHS {
            let cell = position.sample(month);
            temperature += cell.temperature_c;
            precipitation_mm_month += cell.precipitation_mm_month;
            humidity += cell.humidity;
        }
        SurfaceClimate {
            mean_temperature_c: temperature / MONTHS as f32,
            precipitation_mm_day: precipitation_mm_month / (MONTHS as f32 * 30.4375),
            relative_humidity: humidity / MONTHS as f32,
        }
    }

    /// Spatially continuous normals for a one-based imagery month.
    /// Uses the same monthly cells as weather, without a hemisphere shift.
    pub fn monthly_surface(self, lat_rad: f64, lon_rad: f64, month: u8) -> SurfaceClimate {
        assert!((1..=12).contains(&month));
        let cell = SurfacePosition::new(lat_rad, lon_rad).sample(usize::from(month - 1));
        SurfaceClimate {
            mean_temperature_c: cell.temperature_c,
            precipitation_mm_day: cell.precipitation_mm_month / 30.4375,
            relative_humidity: cell.humidity,
        }
    }

    /// Sample continuously in space and between monthly midpoints.
    pub fn sample(self, lat_rad: f64, lon_rad: f64, year_phase: f64) -> ClimateSample {
        let phase = year_phase.rem_euclid(1.0);
        let month_position = phase * MONTHS as f64;
        let month0 = month_position.floor() as usize % MONTHS;
        let month1 = (month0 + 1) % MONTHS;
        let month_alpha = month_position.fract() as f32;
        let a = sample_month(lat_rad, lon_rad, month0);
        let b = sample_month(lat_rad, lon_rad, month1);
        let cell = a.lerp(b, month_alpha);

        let lat = lat_rad.clamp(-PI * 0.5, PI * 0.5);
        let abs_lat = lat.abs();
        let hemisphere_phase = if lat >= 0.0 {
            phase
        } else {
            (phase + 0.5).fract()
        };
        let freezing_level = ((cell.temperature_c + 4.0) * 145.0).clamp(0.0, 5_200.0);
        let snow =
            smoothstep(3.0, -9.0, f64::from(cell.temperature_c)) as f32 * (0.30 + 0.70 * cell.land);
        let sea_ice =
            smoothstep(-2.0, -16.0, f64::from(cell.temperature_c)) as f32 * (1.0 - cell.land);
        let roughness = (0.035 + cell.land * 0.58).clamp(0.0, 1.0);
        let fetch = ((1.0 - cell.land) * (0.76 + 0.24 * (lon_rad * 2.0).cos().abs() as f32))
            .clamp(0.0, 1.0);
        let phenology_amplitude =
            ((abs_lat / 0.35).clamp(0.0, 1.0) as f32 * (0.42 + 0.58 * cell.land)).clamp(0.0, 1.0);

        ClimateSample {
            mean_temperature_c: cell.temperature_c,
            precipitation_mm_day: cell.precipitation_mm_month / 30.4375,
            precipitation_wet_fraction: cell.wet_fraction,
            relative_humidity: cell.humidity,
            surface_wind_east_mps: cell.surface_east,
            surface_wind_north_mps: cell.surface_north,
            low_wind_east_mps: cell.low_east,
            low_wind_north_mps: cell.low_north,
            mid_wind_east_mps: cell.mid_east,
            mid_wind_north_mps: cell.mid_north,
            upper_wind_east_mps: cell.upper_east,
            upper_wind_north_mps: cell.upper_north,
            low_cloud_fraction: cell.low_cloud,
            mid_cloud_fraction: cell.mid_cloud,
            high_cloud_fraction: cell.high_cloud,
            wet_season_phase: phase as f32,
            wet_season_amplitude: cell.wet_fraction,
            freezing_level_m: freezing_level,
            snow_likelihood: snow,
            sea_ice_likelihood: sea_ice,
            aerosol: cell.aerosol,
            aerosol_dust_fraction: cell.dust_fraction,
            land_fraction: cell.land,
            roughness,
            fetch,
            phenology_phase: hemisphere_phase as f32,
            phenology_amplitude,
        }
    }

    /// Four RGBA8 layers per month for the global cloud/atmosphere shader.
    /// Signed winds and temperature are rebased by 128 for normalized upload.
    pub fn gpu_texture(self) -> (u32, u32, u32, Vec<u8>) {
        let source = atlas_bytes();
        let mut texture = vec![0u8; MONTHS * 4 * CELLS_PER_MONTH * 4];
        for month in 0..MONTHS {
            for cell in 0..CELLS_PER_MONTH {
                let src = month * CELLS_PER_MONTH * STRIDE + cell * STRIDE;
                let layer = |index: usize| (month * 4 + index) * CELLS_PER_MONTH * 4 + cell * 4;
                let l0 = layer(0);
                texture[l0..l0 + 4].copy_from_slice(&source[src + 4..src + 8]);
                let l1 = layer(1);
                for (target, source_index) in (l1..l1 + 4).zip([8usize, 9, 12, 13]) {
                    texture[target] = (source[src + source_index] as i8 as i16 + 128) as u8;
                }
                let l2 = layer(2);
                texture[l2] = (source[src + 14] as i8 as i16 + 128) as u8;
                texture[l2 + 1] = (source[src + 15] as i8 as i16 + 128) as u8;
                texture[l2 + 2] = source[src + 16];
                texture[l2 + 3] = source[src + 17];
                let l3 = layer(3);
                texture[l3] = (source[src] as i8 as i16 + 128) as u8;
                texture[l3 + 1] = source[src + 1];
                let precipitation =
                    f32::from(u16::from_le_bytes([source[src + 2], source[src + 3]])) * 0.5;
                texture[l3 + 2] = (precipitation.sqrt() / 36.0 * 255.0).clamp(0.0, 255.0) as u8;
                texture[l3 + 3] = source[src + 18];
            }
        }
        (WIDTH as u32, HEIGHT as u32, (MONTHS * 4) as u32, texture)
    }
}

// Terrain needs only three channels. Reuse spatial coordinates across the
// annual samples, without decoding weather's wind/cloud/aerosol channels.
struct SurfacePosition {
    offsets: [usize; 4],
    tx: f32,
    ty: f32,
}

struct SurfaceCell {
    temperature_c: f32,
    humidity: f32,
    precipitation_mm_month: f32,
}

impl SurfacePosition {
    fn new(lat_rad: f64, lon_rad: f64) -> Self {
        let x = (lon_rad / TAU + 0.5).rem_euclid(1.0) * WIDTH as f64 - 0.5;
        let y = (0.5 - lat_rad.clamp(-PI * 0.5, PI * 0.5) / PI) * HEIGHT as f64 - 0.5;
        let x0 = x.floor() as isize;
        let y0 = y.floor() as isize;
        let offset = |ix: isize, iy: isize| {
            (iy.clamp(0, HEIGHT as isize - 1) as usize * WIDTH
                + ix.rem_euclid(WIDTH as isize) as usize)
                * STRIDE
        };
        Self {
            offsets: [
                offset(x0, y0),
                offset(x0 + 1, y0),
                offset(x0, y0 + 1),
                offset(x0 + 1, y0 + 1),
            ],
            tx: (x - x.floor()) as f32,
            ty: (y - y.floor()) as f32,
        }
    }

    fn sample(&self, month: usize) -> SurfaceCell {
        let bytes = atlas_bytes();
        let cells = self.offsets.map(|offset| {
            let offset = month * CELLS_PER_MONTH * STRIDE + offset;
            [
                f32::from(bytes[offset] as i8) * 0.5,
                f32::from(bytes[offset + 1]) / 255.0,
                f32::from(u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]])) * 0.5,
            ]
        });
        let mix = |i: usize| {
            let north = cells[0][i] + (cells[1][i] - cells[0][i]) * self.tx;
            let south = cells[2][i] + (cells[3][i] - cells[2][i]) * self.tx;
            north + (south - north) * self.ty
        };
        SurfaceCell {
            temperature_c: mix(0),
            humidity: mix(1),
            precipitation_mm_month: mix(2),
        }
    }
}

fn sample_month(lat_rad: f64, lon_rad: f64, month: usize) -> MonthlyCell {
    let x = (lon_rad / TAU + 0.5).rem_euclid(1.0) * WIDTH as f64 - 0.5;
    let y = (0.5 - lat_rad.clamp(-PI * 0.5, PI * 0.5) / PI) * HEIGHT as f64 - 0.5;
    let x0 = x.floor() as isize;
    let y0 = y.floor() as isize;
    let tx = (x - x.floor()) as f32;
    let ty = (y - y.floor()) as f32;
    let at = |ix: isize, iy: isize| {
        decode_cell(
            month,
            ix.rem_euclid(WIDTH as isize) as usize,
            iy.clamp(0, HEIGHT as isize - 1) as usize,
        )
    };
    let north = at(x0, y0).lerp(at(x0 + 1, y0), tx);
    let south = at(x0, y0 + 1).lerp(at(x0 + 1, y0 + 1), tx);
    north.lerp(south, ty)
}

fn decode_cell(month: usize, x: usize, y: usize) -> MonthlyCell {
    let bytes = atlas_bytes();
    let offset = (month * CELLS_PER_MONTH + y * WIDTH + x) * STRIDE;
    let signed = |index: usize| f32::from(bytes[offset + index] as i8) * 0.5;
    let unit = |index: usize| f32::from(bytes[offset + index]) / 255.0;
    MonthlyCell {
        temperature_c: signed(0),
        humidity: unit(1),
        precipitation_mm_month: f32::from(u16::from_le_bytes([
            bytes[offset + 2],
            bytes[offset + 3],
        ])) * 0.5,
        wet_fraction: unit(4),
        low_cloud: unit(5),
        mid_cloud: unit(6),
        high_cloud: unit(7),
        surface_east: signed(8),
        surface_north: signed(9),
        low_east: signed(10),
        low_north: signed(11),
        mid_east: signed(12),
        mid_north: signed(13),
        upper_east: signed(14),
        upper_north: signed(15),
        aerosol: unit(16) * 1.2,
        dust_fraction: unit(17),
        land: unit(18),
    }
}

fn atlas_bytes() -> &'static [u8] {
    static DATA: OnceLock<Box<[u8]>> = OnceLock::new();
    DATA.get_or_init(|| {
        assert_eq!(&compressed()[0..4], b"VCL2", "invalid climate atlas magic");
        assert_eq!(read_u32(compressed(), 4), CLIMATE_ATLAS_VERSION);
        assert_eq!(read_u16(compressed(), 8) as usize, WIDTH);
        assert_eq!(read_u16(compressed(), 10) as usize, HEIGHT);
        assert_eq!(compressed()[12] as usize, MONTHS);
        assert_eq!(compressed()[13] as usize, STRIDE);
        let mut decoder = ZlibDecoder::new(&compressed()[HEADER_BYTES..]);
        let mut decoded = Vec::with_capacity(DECOMPRESSED_BYTES);
        decoder
            .read_to_end(&mut decoded)
            .expect("bundled climate atlas must decompress");
        assert_eq!(decoded.len(), DECOMPRESSED_BYTES);
        decoded.into_boxed_slice()
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("atlas header"))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("atlas header"))
}

fn smoothstep(edge0: f64, edge1: f64, value: f64) -> f64 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_is_bounded_and_longitude_wraps() {
        let atlas = ClimateAtlas::bundled();
        for ilat in 0..=18 {
            let lat = -PI * 0.5 + PI * ilat as f64 / 18.0;
            for ilon in 0..36 {
                let lon = -PI + TAU * ilon as f64 / 36.0;
                let sample = atlas.sample(lat, lon, 0.37);
                assert!((-75.0..=65.0).contains(&sample.mean_temperature_c));
                assert!((0.0..=1.0).contains(&sample.relative_humidity));
                assert!((0.0..=1.0).contains(&sample.land_fraction));
                assert!((0.0..=1.0).contains(&sample.low_cloud_fraction));
            }
        }
        let west = atlas.sample(0.42, -PI + 1e-8, 0.7);
        let east = atlas.sample(0.42, PI + 1e-8, 0.7);
        assert!((west.mean_temperature_c - east.mean_temperature_c).abs() < 1e-3);
    }

    #[test]
    fn regional_aerosol_and_rainfall_are_not_latitude_only() {
        let atlas = ClimateAtlas::bundled();
        let delhi = atlas.sample(28.61_f64.to_radians(), 77.21_f64.to_radians(), 0.0);
        let sydney = atlas.sample(-33.87_f64.to_radians(), 151.21_f64.to_radians(), 0.0);
        let mumbai = atlas.sample(19.08_f64.to_radians(), 72.88_f64.to_radians(), 0.55);
        assert!(delhi.aerosol > sydney.aerosol + 0.10);
        assert!(mumbai.precipitation_mm_day > delhi.precipitation_mm_day);
    }

    #[test]
    fn monthly_surface_uses_weather_cells_without_hemisphere_inversion() {
        let atlas = ClimateAtlas::bundled();
        for lat in [-44.0_f64, -3.0, 18.0, 53.0, 69.0] {
            for month in 1..=12 {
                let month_sample =
                    atlas.monthly_surface(lat.to_radians(), 76.0_f64.to_radians(), month);
                let weather = atlas.sample(
                    lat.to_radians(),
                    76.0_f64.to_radians(),
                    f64::from(month - 1) / 12.0,
                );
                assert!(
                    (month_sample.mean_temperature_c - weather.mean_temperature_c).abs() < 0.0001
                );
                assert!(
                    (month_sample.precipitation_mm_day - weather.precipitation_mm_day).abs()
                        < 0.0001
                );
            }
        }
    }

    #[test]
    fn annual_surface_distinguishes_desert_and_rainforest() {
        let atlas = ClimateAtlas::bundled();
        let sahara = atlas.annual_surface(23.4_f64.to_radians(), 25.7_f64.to_radians());
        let amazon = atlas.annual_surface(-3.5_f64.to_radians(), -62.2_f64.to_radians());
        assert!(amazon.precipitation_mm_day > sahara.precipitation_mm_day + 2.0);
        assert!(amazon.relative_humidity > sahara.relative_humidity);
    }
}
#[test]
fn terrain_climate_subset_matches_full_weather_decode_exactly() {
    for lat in (-90..=90).step_by(7).map(f64::from).chain([90.0, -89.99]) {
        for lon in (-180..=180)
            .step_by(11)
            .map(f64::from)
            .chain([180.0, 540.0])
        {
            let position = SurfacePosition::new(lat.to_radians(), lon.to_radians());
            let mut temperature = 0.0;
            let mut precipitation = 0.0;
            let mut humidity = 0.0;
            for month in 0..MONTHS {
                let full = sample_month(lat.to_radians(), lon.to_radians(), month);
                let subset = position.sample(month);
                assert_eq!(subset.temperature_c, full.temperature_c);
                assert_eq!(subset.humidity, full.humidity);
                assert_eq!(subset.precipitation_mm_month, full.precipitation_mm_month);
                temperature += full.temperature_c;
                precipitation += full.precipitation_mm_month;
                humidity += full.humidity;
            }
            assert_eq!(
                ClimateAtlas::bundled().annual_surface(lat.to_radians(), lon.to_radians()),
                SurfaceClimate {
                    mean_temperature_c: temperature / MONTHS as f32,
                    precipitation_mm_day: precipitation / (MONTHS as f32 * 30.4375),
                    relative_humidity: humidity / MONTHS as f32,
                }
            );
        }
    }
}
