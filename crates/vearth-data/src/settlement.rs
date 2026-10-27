//! Offline settlement context from the native GHS-SMOD 2020 kilometre grid.
//!
//! The immutable row-run index is searched in place: embedded in the native
//! executable, and fetched in blocks as lookups reach them in the browser. No
//! dense global raster or per-query allocations are needed.
//! Categories describe settlement density and contiguity, not building age,
//! materials, maintenance, wealth, or architectural tradition.

use std::f64::consts::{FRAC_PI_2, PI, SQRT_2, TAU};
use std::sync::OnceLock;
use vearth_platform::assets::Blocks;

pub const SETTLEMENT_ATLAS_VERSION: u32 = 1;
pub const SETTLEMENT_ATLAS_EPOCH: u32 = 2020;
pub const SETTLEMENT_ATLAS_PROVENANCE: &str = "European Commission, Joint Research Centre: Schiavina, Melchiorri and Pesaresi (2023), GHS-SMOD R2023A V2-0, epoch 2020; DOI 10.2905/A0DF7A6F-49DE-46EA-9BDE-563437A6E2BA. Pesaresi et al. (2024), Advances on the Global Human Settlement Layer by joint assessment of Earth Observation and population survey data; DOI 10.1080/17538947.2024.2390454. Losslessly repacked; no endorsement implied.";

fn packed() -> &'static Blocks {
    static PACKED: OnceLock<Blocks> = OnceLock::new();
    PACKED.get_or_init(|| vearth_platform::asset_blocks!("../assets/settlement-2020-v1.bin"))
}
const HEADER_BYTES: usize = 64;
const RUN_BYTES: usize = 4;
pub const SETTLEMENT_NO_DATA: i16 = -200;

/// Compact interpretation of the source Degree of Urbanisation categories.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum SettlementClass {
    #[default]
    Unknown,
    Rural,
    Village,
    Suburban,
    Town,
    City,
}

impl SettlementClass {
    pub const fn from_raw_code(code: i16) -> Self {
        match code {
            11 | 12 => Self::Rural,
            13 => Self::Village,
            21 => Self::Suburban,
            22 | 23 => Self::Town,
            30 => Self::City,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SettlementAtlas;

impl SettlementAtlas {
    pub const fn bundled() -> Self {
        Self
    }

    /// Native-grid classification at geodetic coordinates in radians.
    /// Water, no-data, invalid coordinates and uncovered polar caps are unknown.
    pub fn class_at(self, lat_rad: f64, lon_rad: f64) -> SettlementClass {
        self.raw_code_at(lat_rad, lon_rad)
            .map(SettlementClass::from_raw_code)
            .unwrap_or_default()
    }

    /// Unmodified source code, including water (`10`) and no-data (`-200`).
    /// `None` means invalid coordinates or a point outside the source extent.
    pub fn raw_code_at(self, lat_rad: f64, lon_rad: f64) -> Option<i16> {
        atlas().raw_code_at(lat_rad, lon_rad)
    }

    /// Continuous, authored development cue for material inference, not a
    /// population measurement. Bilinear sampling avoids native kilometre-grid
    /// edges in visual preferences; unknown neighbours contribute no evidence.
    pub fn development_at(self, lat_rad: f64, lon_rad: f64) -> Option<f32> {
        let atlas = atlas();
        let projected = mollweide(lat_rad, lon_rad, atlas.radius)?;
        let x = (projected[0] - atlas.origin[0]) / atlas.pixel_size[0] - 0.5;
        let y = (projected[1] - atlas.origin[1]) / atlas.pixel_size[1] - 0.5;
        let mut value = 0.0;
        let mut evidence = 0.0;
        for (dx, wx) in [
            (0, 1.0 - x.fract().rem_euclid(1.0)),
            (1, x.fract().rem_euclid(1.0)),
        ] {
            for (dy, wy) in [
                (0, 1.0 - y.fract().rem_euclid(1.0)),
                (1, y.fract().rem_euclid(1.0)),
            ] {
                let column = x.floor() as i64 + dx;
                let row = y.floor() as i64 + dy;
                if column < 0 || row < 0 {
                    continue;
                }
                let class = atlas
                    .pixel(column as usize, row as usize)
                    .map(SettlementClass::from_raw_code);
                let density = match class {
                    Some(SettlementClass::Rural) => 0.0,
                    Some(SettlementClass::Village) => 0.4,
                    Some(SettlementClass::Suburban) => 0.65,
                    Some(SettlementClass::Town) => 0.85,
                    Some(SettlementClass::City) => 1.0,
                    _ => continue,
                };
                let weight = (wx * wy) as f32;
                value += density * weight;
                evidence += weight;
            }
        }
        (evidence > 1.0e-6).then(|| value / evidence)
    }
}

/// Atlas bytes: the bundled [`Blocks`], or a slice in tests.
trait Source: Copy {
    fn len(self) -> usize;
    fn array<const N: usize>(self, offset: usize) -> Option<[u8; N]>;
}

impl Source for &[u8] {
    fn len(self) -> usize {
        <[u8]>::len(self)
    }

    fn array<const N: usize>(self, offset: usize) -> Option<[u8; N]> {
        self.get(offset..offset.checked_add(N)?)?.try_into().ok()
    }
}

impl Source for &Blocks {
    fn len(self) -> usize {
        Blocks::len(self)
    }

    /// A failed download is unrecoverable here, as for a whole asset:
    /// answering "unknown" would bake wrong classes into generated surfaces.
    fn array<const N: usize>(self, offset: usize) -> Option<[u8; N]> {
        let end = offset.checked_add(N)?;
        if end > Blocks::len(self) {
            return None;
        }
        let bytes = self
            .read(offset..end)
            .unwrap_or_else(|error| panic!("settlement atlas: {error}"));
        bytes.as_ref().try_into().ok()
    }
}

struct Atlas<S> {
    bytes: S,
    width: usize,
    height: usize,
    origin: [f64; 2],
    pixel_size: [f64; 2],
    radius: f64,
    runs_offset: usize,
}

fn atlas() -> &'static Atlas<&'static Blocks> {
    static ATLAS: OnceLock<Atlas<&'static Blocks>> = OnceLock::new();
    ATLAS.get_or_init(|| Atlas::open(packed()).expect("bundled settlement atlas is valid"))
}

impl<S: Source> Atlas<S> {
    fn open(bytes: S) -> Result<Self, &'static str> {
        if bytes.array(0) != Some(*b"VSM1")
            || read_u32(bytes, 4) != Some(SETTLEMENT_ATLAS_VERSION)
            || read_u32(bytes, 56) != Some(SETTLEMENT_ATLAS_EPOCH)
            || read_u32(bytes, 60) != Some(0)
        {
            return Err("invalid settlement atlas header");
        }
        let width = read_u32(bytes, 8).ok_or("missing settlement width")? as usize;
        let height = read_u32(bytes, 12).ok_or("missing settlement height")? as usize;
        let origin = [
            read_f64(bytes, 16).ok_or("missing settlement origin")?,
            read_f64(bytes, 24).ok_or("missing settlement origin")?,
        ];
        let pixel_size = [
            read_f64(bytes, 32).ok_or("missing settlement pixel size")?,
            read_f64(bytes, 40).ok_or("missing settlement pixel size")?,
        ];
        let radius = read_f64(bytes, 48).ok_or("missing settlement radius")?;
        if width == 0
            || width > u16::MAX as usize
            || height == 0
            || height > u16::MAX as usize
            || !origin.iter().all(|value| value.is_finite())
            || pixel_size != [1000.0, -1000.0]
            || radius != 6_378_137.0
        {
            return Err("invalid settlement grid");
        }
        let runs_offset = HEADER_BYTES + (height + 1) * 4;
        let run_count = read_u32(bytes, HEADER_BYTES + height * 4)
            .ok_or("truncated settlement row index")? as usize;
        if read_u32(bytes, HEADER_BYTES) != Some(0)
            || run_count
                .checked_mul(RUN_BYTES)
                .and_then(|size| runs_offset.checked_add(size))
                != Some(bytes.len())
        {
            return Err("invalid settlement payload size");
        }
        let mut previous = 0;
        for row in 1..=height {
            let next =
                read_u32(bytes, HEADER_BYTES + row * 4).ok_or("truncated settlement row index")?;
            if next <= previous || next as usize > run_count {
                return Err("invalid settlement row offsets");
            }
            previous = next;
        }
        Ok(Self {
            bytes,
            width,
            height,
            origin,
            pixel_size,
            radius,
            runs_offset,
        })
    }

    fn raw_code_at(&self, lat_rad: f64, lon_rad: f64) -> Option<i16> {
        let projected = mollweide(lat_rad, lon_rad, self.radius)?;
        if !projected.iter().all(|coordinate| coordinate.is_finite()) {
            return None;
        }
        let column = ((projected[0] - self.origin[0]) / self.pixel_size[0]).floor();
        let row = ((projected[1] - self.origin[1]) / self.pixel_size[1]).floor();
        if column < 0.0 || column >= self.width as f64 || row < 0.0 || row >= self.height as f64 {
            return None;
        }
        self.pixel(column as usize, row as usize)
    }

    fn pixel(&self, column: usize, row: usize) -> Option<i16> {
        if column >= self.width || row >= self.height {
            return None;
        }
        let mut left = read_u32(self.bytes, HEADER_BYTES + row * 4)? as usize;
        let end = read_u32(self.bytes, HEADER_BYTES + (row + 1) * 4)? as usize;
        let mut right = end;
        while left < right {
            let middle = left + (right - left) / 2;
            let run_end = read_u16(self.bytes, self.runs_offset + middle * RUN_BYTES)? as usize;
            if run_end <= column {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        (left < end)
            .then(|| read_i16(self.bytes, self.runs_offset + left * RUN_BYTES + 2))
            .flatten()
    }
}

fn mollweide(lat_rad: f64, lon_rad: f64, radius: f64) -> Option<[f64; 2]> {
    if !lat_rad.is_finite() || !lon_rad.is_finite() || lat_rad.abs() > FRAC_PI_2 {
        return None;
    }
    let longitude = (lon_rad.rem_euclid(TAU) + PI).rem_euclid(TAU) - PI;
    // ESRI:54009 uses the WGS84 semi-major axis as a spherical radius. Solve
    // 2θ + sin(2θ) = π sin(latitude); no resampling of the source grid occurs.
    let theta = if (FRAC_PI_2 - lat_rad.abs()) < 1e-12 {
        lat_rad.signum() * FRAC_PI_2
    } else {
        let target = PI * lat_rad.sin();
        let mut twice_theta = lat_rad;
        for _ in 0..30 {
            let delta = (twice_theta + twice_theta.sin() - target) / (1.0 + twice_theta.cos());
            twice_theta -= delta;
            if delta.abs() < 1e-12 {
                break;
            }
        }
        twice_theta * 0.5
    };
    Some([
        radius * 2.0 * SQRT_2 / PI * longitude * theta.cos(),
        radius * SQRT_2 * theta.sin(),
    ])
}

fn read_u16(bytes: impl Source, offset: usize) -> Option<u16> {
    bytes.array(offset).map(u16::from_le_bytes)
}

fn read_i16(bytes: impl Source, offset: usize) -> Option<i16> {
    bytes.array(offset).map(i16::from_le_bytes)
}

fn read_u32(bytes: impl Source, offset: usize) -> Option<u32> {
    bytes.array(offset).map(u32::from_le_bytes)
}

fn read_f64(bytes: impl Source, offset: usize) -> Option<f64> {
    bytes.array(offset).map(f64::from_le_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_categories_keep_water_and_no_data_unknown() {
        use SettlementClass::*;
        for (code, expected) in [
            (SETTLEMENT_NO_DATA, Unknown),
            (10, Unknown),
            (11, Rural),
            (12, Rural),
            (13, Village),
            (21, Suburban),
            (22, Town),
            (23, Town),
            (30, City),
            (99, Unknown),
        ] {
            assert_eq!(SettlementClass::from_raw_code(code), expected);
        }
    }

    #[test]
    fn invalid_coordinates_and_uncovered_polar_caps_are_unknown() {
        let atlas = SettlementAtlas::bundled();
        for (lat, lon) in [
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
            (2.0, 0.0),
            (-2.0, 0.0),
            (FRAC_PI_2, 0.0),
            (-FRAC_PI_2, 0.0),
        ] {
            assert_eq!(atlas.raw_code_at(lat, lon), None);
            assert_eq!(atlas.class_at(lat, lon), SettlementClass::Unknown);
            assert_eq!(atlas.development_at(lat, lon), None);
        }
        assert_eq!(
            atlas.class_at(0.0, -140.0_f64.to_radians()),
            SettlementClass::Unknown
        );
    }

    #[test]
    fn development_cue_is_bounded_continuous_and_periodic() {
        let atlas = SettlementAtlas::bundled();
        let mut varied = false;
        let mut previous: Option<f32> = None;
        // Traverse the eastern edge of Greater London in roughly ten-metre
        // increments; source kilometre categories must not become visual steps.
        for index in 0..2000 {
            let lat = 51.55_f64.to_radians();
            let lon = (-0.05 + f64::from(index) * 0.00015).to_radians();
            let value = atlas.development_at(lat, lon).unwrap();
            assert!((0.0..=1.000001).contains(&value));
            assert!((value - atlas.development_at(lat, lon + TAU).unwrap()).abs() < 1.0e-5);
            if let Some(previous) = previous {
                assert!((value - previous).abs() < 0.04);
                varied |= (value - previous).abs() > 0.0001;
            }
            previous = Some(value);
        }
        assert!(varied, "exercise an actual settlement transition");
        assert_eq!(atlas.development_at(0.0, -140.0_f64.to_radians()), None);
    }

    #[test]
    fn bundled_rows_have_ordered_complete_runs_and_preserved_categories() {
        let atlas = atlas();
        assert_eq!((atlas.width, atlas.height), (36_082, 18_000));
        let mut counts = std::collections::BTreeMap::<i16, u64>::new();
        for row in 0..atlas.height {
            let start = read_u32(atlas.bytes, HEADER_BYTES + row * 4).unwrap() as usize;
            let end = read_u32(atlas.bytes, HEADER_BYTES + (row + 1) * 4).unwrap() as usize;
            let mut previous = 0;
            for run in start..end {
                let offset = atlas.runs_offset + run * RUN_BYTES;
                let end_column = read_u16(atlas.bytes, offset).unwrap() as usize;
                let category = read_i16(atlas.bytes, offset + 2).unwrap();
                assert!(end_column > previous && end_column <= atlas.width);
                assert!(matches!(
                    category,
                    SETTLEMENT_NO_DATA | 10 | 11 | 12 | 13 | 21 | 22 | 23 | 30
                ));
                *counts.entry(category).or_default() += (end_column - previous) as u64;
                assert_eq!(atlas.pixel(previous, row), Some(category));
                assert_eq!(atlas.pixel(end_column - 1, row), Some(category));
                previous = end_column;
            }
            assert_eq!(previous, atlas.width);
        }
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../assets/settlement-2020-v1.source.json")).unwrap();
        for (category, count) in counts {
            assert_eq!(
                manifest["category_pixel_counts"][category.to_string()].as_u64(),
                Some(count)
            );
        }
    }

    #[test]
    fn projection_and_sampling_match_independent_gdal_proj_reference() {
        let reference: serde_json::Value = serde_json::from_str(include_str!(
            "../testdata/settlement-reference-samples.json"
        ))
        .unwrap();
        for sample in reference["samples"].as_array().unwrap() {
            let latitude = sample["lat_deg"].as_f64().unwrap().to_radians();
            let longitude = sample["lon_deg"].as_f64().unwrap().to_radians();
            let projected = mollweide(latitude, longitude, 6_378_137.0).unwrap();
            assert!(
                (projected[0] - sample["x_m"].as_f64().unwrap()).abs() < 0.001,
                "{sample}"
            );
            assert!(
                (projected[1] - sample["y_m"].as_f64().unwrap()).abs() < 0.001,
                "{sample}"
            );
            assert_eq!(
                SettlementAtlas::bundled().raw_code_at(latitude, longitude),
                Some(sample["raw_code"].as_i64().unwrap() as i16),
                "{sample}"
            );
            let raw = atlas().pixel(
                sample["column"].as_u64().unwrap() as usize,
                sample["row"].as_u64().unwrap() as usize,
            );
            assert_eq!(
                raw,
                Some(sample["raw_code"].as_i64().unwrap() as i16),
                "{sample}"
            );
        }
    }

    #[test]
    fn longitude_wrap_is_periodic() {
        for (latitude, longitude) in [
            (18.5207_f64, 73.8524_f64),
            (-18.1248, 178.4501),
            (-13.8333, -171.7667),
            (0.0, 180.0),
        ] {
            let atlas = SettlementAtlas::bundled();
            let expected = atlas.raw_code_at(latitude.to_radians(), longitude.to_radians());
            for turn in -3..=3 {
                assert_eq!(
                    atlas.raw_code_at(
                        latitude.to_radians(),
                        (longitude + 360.0 * f64::from(turn)).to_radians()
                    ),
                    expected
                );
            }
        }
    }

    #[test]
    fn malformed_headers_and_row_indices_are_rejected() {
        let bundled: &[u8] = include_bytes!("../assets/settlement-2020-v1.bin");
        assert!(Atlas::open(bundled).is_ok());
        for size in [0, 4, 16, 63, 64, 1000] {
            assert!(Atlas::open(&bundled[..size]).is_err());
        }
        let mut damaged = bundled.to_vec();
        damaged[4] = 255;
        assert!(Atlas::open(&damaged[..]).is_err());
        damaged[4] = SETTLEMENT_ATLAS_VERSION as u8;
        damaged[HEADER_BYTES + 4..HEADER_BYTES + 8].copy_from_slice(&0_u32.to_le_bytes());
        assert!(Atlas::open(&damaged[..]).is_err());
    }
}
