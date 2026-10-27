//! Real-sky inputs shared by the CPU visibility model and GPU star pass.
//!
//! Hipparcos directions remain in the inertial equatorial frame until the
//! vertex shader rotates them by Greenwich sidereal time. NASA Black Marble
//! is sampled on the CPU: one blurred local value and eight azimuthal dome
//! values are enough for skyglow, with no per-pixel global texture reads.

use crate::solar::Lighting;
use glam::DVec3;

fn star_bytes() -> &'static [u8] {
    vearth_platform::asset_bytes!("../assets/sky/hipparcos_mag78.bin")
}
fn light_bytes() -> &'static [u8] {
    vearth_platform::asset_bytes!("../assets/sky/black_marble_2048x1024.bin")
}
fn milky_way_bytes() -> &'static [u8] {
    vearth_platform::asset_bytes!("../assets/sky/milky_way_1024x512.bin")
}
const HEADER_BYTES: usize = 16;
const STAR_STRIDE: usize = 5 * size_of::<f32>();
const EARTH_RADIUS_M: f64 = 6_371_008.8;

#[derive(Clone, Copy, Debug)]
pub(crate) struct BrightStar {
    pub direction_icrs: [f32; 3],
    pub magnitude: f32,
    pub bv: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SkyVisibility {
    /// Gaussian-blurred upward-emission proxy at the camera's ground point.
    pub local_glow: f32,
    /// Distant-light emission in 45° sectors, north then clockwise.
    pub horizon_glow: [f32; 8],
    /// Current naked-eye-style threshold after city, altitude, Moon, twilight.
    pub limiting_magnitude: f32,
    /// Site class from the moonless, astronomically dark threshold.
    pub bortle_class: u8,
}

impl SkyVisibility {
    pub fn at(position_ecef: DVec3, lighting: Lighting) -> Self {
        let (lat, lon, altitude_m) = vearth_core::ellipsoid::ecef_to_geodetic(position_ecef);
        let (local_glow, horizon_glow) = geographic_glow(lat, lon);
        let aerosol_column = (-altitude_m.max(0.0) / 1_500.0).exp() as f32;
        let twilight_column = (-altitude_m.max(0.0) / 20_000.0).exp() as f32;
        let effective_glow = (local_glow * aerosol_column).clamp(0.0, 1.0);
        let site_limit = 7.8 - 5.3 * effective_glow.sqrt();
        let bortle_class = bortle_from_limiting_magnitude(site_limit);

        let surface = vearth_core::ellipsoid::surface_point_along(position_ecef);
        let up = vearth_core::ellipsoid::geodetic_normal(surface);
        let limiting_magnitude =
            current_limiting_magnitude(site_limit, aerosol_column, twilight_column, up, lighting);
        Self {
            local_glow,
            horizon_glow,
            limiting_magnitude,
            bortle_class,
        }
    }
}

fn geographic_glow(lat: f64, lon: f64) -> (f32, [f32; 8]) {
    // The HUD and renderer query the same camera on the same thread. Only
    // geography enters these atlas samples; Moon, Sun and altitude stay live.
    type Glow = (f32, [f32; 8]);
    thread_local! {
        static LAST: std::cell::Cell<Option<([u64; 2], Glow)>> = const {
            std::cell::Cell::new(None)
        };
    }
    LAST.with(|last| {
        let key = [lat.to_bits(), lon.to_bits()];
        if let Some((previous, glow)) = last.get()
            && key == previous
        {
            return glow;
        }
        let glow = (gaussian_local_glow(lat, lon), horizon_domes(lat, lon));
        last.set(Some((key, glow)));
        glow
    })
}

pub fn bortle_from_limiting_magnitude(magnitude: f32) -> u8 {
    match magnitude {
        value if value >= 7.6 => 1,
        value if value >= 7.1 => 2,
        value if value >= 6.6 => 3,
        value if value >= 6.1 => 4,
        value if value >= 5.6 => 5,
        value if value >= 5.1 => 6,
        value if value >= 4.6 => 7,
        value if value >= 4.1 => 8,
        _ => 9,
    }
}

fn current_limiting_magnitude(
    site_limit: f32,
    aerosol_column: f32,
    twilight_column: f32,
    up: DVec3,
    lighting: Lighting,
) -> f32 {
    let moon_altitude = up.dot(lighting.moon_direction_ecef).max(0.0) as f32;
    let moon_penalty =
        3.3 * (lighting.moon_illumination as f32).powf(0.6) * moon_altitude.sqrt() * aerosol_column;
    let dark_limit = site_limit - moon_penalty;
    let solar_altitude = up.dot(lighting.direction_ecef) as f32;
    let twilight = smoothstep(
        (-18.0_f32).to_radians().sin(),
        (-6.0_f32).to_radians().sin(),
        solar_altitude,
    );
    let astronomical_dark = 1.0 - twilight * twilight_column;
    -2.0 + (dark_limit + 2.0) * astronomical_dark
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn gaussian_local_glow(lat: f64, lon: f64) -> f32 {
    let offsets_m = [-30_000.0_f64, -15_000.0, 0.0, 15_000.0, 30_000.0];
    let mut sum = 0.0_f32;
    let mut weights = 0.0_f32;
    for north_m in offsets_m {
        for east_m in offsets_m {
            let distance = north_m.hypot(east_m);
            let (sample_lat, sample_lon) = if distance < 1.0 {
                (lat, lon)
            } else {
                destination(lat, lon, east_m.atan2(north_m), distance)
            };
            let weight = (-0.5 * (distance / 20_000.0).powi(2)).exp() as f32;
            sum += sample_light(sample_lat, sample_lon) * weight;
            weights += weight;
        }
    }
    (sum / weights.max(1e-6) * 1.8).clamp(0.0, 1.0)
}

fn horizon_domes(lat: f64, lon: f64) -> [f32; 8] {
    let mut domes = [0.0; 8];
    let distances_m = [25_000.0_f64, 50_000.0, 75_000.0, 100_000.0];
    let weights = [0.34_f32, 0.28, 0.22, 0.16];
    for (sector, dome) in domes.iter_mut().enumerate() {
        let bearing = sector as f64 * std::f64::consts::FRAC_PI_4;
        let mut emission = 0.0;
        for (distance, weight) in distances_m.into_iter().zip(weights) {
            let (sample_lat, sample_lon) = destination(lat, lon, bearing, distance);
            emission += sample_light(sample_lat, sample_lon) * weight;
        }
        *dome = (emission * 1.6).clamp(0.0, 1.0);
    }
    domes
}

fn destination(lat: f64, lon: f64, bearing: f64, distance_m: f64) -> (f64, f64) {
    let angular = distance_m / EARTH_RADIUS_M;
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_angular, cos_angular) = angular.sin_cos();
    let (sin_bearing, cos_bearing) = bearing.sin_cos();
    let lat2 = (sin_lat * cos_angular + cos_lat * sin_angular * cos_bearing).asin();
    let lon2 =
        lon + (sin_bearing * sin_angular * cos_lat).atan2(cos_angular - sin_lat * lat2.sin());
    (lat2, lon2)
}

fn sample_light(lat: f64, lon: f64) -> f32 {
    debug_assert_eq!(&light_bytes()[0..4], b"VNLT");
    debug_assert_eq!(read_u32(light_bytes(), 4), 1);
    let width = read_u32(light_bytes(), 8) as usize;
    let height = read_u32(light_bytes(), 12) as usize;
    debug_assert_eq!(light_bytes().len(), HEADER_BYTES + width * height);
    let x = ((lon / std::f64::consts::TAU + 0.5).rem_euclid(1.0) * width as f64) - 0.5;
    let y = ((0.5 - lat / std::f64::consts::PI).clamp(0.0, 1.0) * height as f64) - 0.5;
    let x0 = x.floor() as isize;
    let y0 = y.floor() as isize;
    let tx = (x - x.floor()) as f32;
    let ty = (y - y.floor()) as f32;
    let texel = |ix: isize, iy: isize| -> f32 {
        let ix = ix.rem_euclid(width as isize) as usize;
        let iy = iy.clamp(0, height as isize - 1) as usize;
        f32::from(light_bytes()[HEADER_BYTES + iy * width + ix]) / 255.0
    };
    let north = texel(x0, y0) * (1.0 - tx) + texel(x0 + 1, y0) * tx;
    let south = texel(x0, y0 + 1) * (1.0 - tx) + texel(x0 + 1, y0 + 1) * tx;
    north * (1.0 - ty) + south * ty
}

pub(crate) fn bright_stars() -> Vec<BrightStar> {
    assert_eq!(&star_bytes()[0..4], b"VSTR", "invalid bundled star catalog");
    assert_eq!(read_u32(star_bytes(), 4), 1, "unsupported star catalog");
    let count = read_u32(star_bytes(), 8) as usize;
    assert_eq!(star_bytes().len(), HEADER_BYTES + count * STAR_STRIDE);
    (0..count)
        .map(|index| {
            let offset = HEADER_BYTES + index * STAR_STRIDE;
            BrightStar {
                direction_icrs: [
                    read_f32(star_bytes(), offset),
                    read_f32(star_bytes(), offset + 4),
                    read_f32(star_bytes(), offset + 8),
                ],
                magnitude: read_f32(star_bytes(), offset + 12),
                bv: read_f32(star_bytes(), offset + 16),
            }
        })
        .collect()
}

pub(crate) fn milky_way_texture() -> (u32, u32, &'static [u8]) {
    assert_eq!(
        &milky_way_bytes()[0..4],
        b"VMWY",
        "invalid bundled Milky Way texture"
    );
    assert_eq!(
        read_u32(milky_way_bytes(), 4),
        1,
        "unsupported Milky Way texture"
    );
    let width = read_u32(milky_way_bytes(), 8);
    let height = read_u32(milky_way_bytes(), 12);
    let pixels = &milky_way_bytes()[HEADER_BYTES..];
    assert_eq!(
        pixels.len(),
        width as usize * height as usize * 4,
        "truncated bundled Milky Way texture"
    );
    (width, height, pixels)
}

pub(crate) fn star_draw_count(magnitudes: &[f32], limiting_magnitude: f32) -> u32 {
    // The shader's smooth transition begins 0.28 magnitudes before each
    // star's catalog value. Catalog order makes this an O(log n) prefix query.
    magnitudes.partition_point(|&magnitude| magnitude <= limiting_magnitude + 0.28) as u32
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"))
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"))
}

pub fn attribution_entries() -> [vearth_data::registry::AttributionEntry; 3] {
    [
        vearth_data::registry::AttributionEntry {
            layer: "Night sky",
            name: "Hipparcos Main Catalogue".into(),
            attribution: "Credit: ESA; The Hipparcos and Tycho Catalogues, ESA SP-1200 (1997); adapted bright-star subset".into(),
            license: "CC BY-NC 3.0 IGO (noncommercial)".into(),
            url: "https://www.cosmos.esa.int/web/hipparcos/catalogues".into(),
        },
        vearth_data::registry::AttributionEntry {
            layer: "Night sky",
            name: "NASA Black Marble 2016".into(),
            attribution: "NASA Earth Observatory / VIIRS".into(),
            license: "public domain".into(),
            url: "https://science.nasa.gov/earth/earth-observatory/earth-at-night/maps/".into(),
        },
        vearth_data::registry::AttributionEntry {
            layer: "Night sky",
            name: "The Milky Way panorama".into(),
            attribution: "ESO/S. Brunier".into(),
            license: "CC BY 4.0".into(),
            url: "https://www.eso.org/public/images/eso0932a/".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geographic_glow_cache_preserves_moving_and_repeated_queries() {
        for (lat, lon) in [
            (40.758_f64, -73.9855_f64),
            (40.758, -73.9855),
            (40.758001, -73.9855),
            (46.577, 7.990),
            (89.999, 179.999),
            (-89.999, -179.999),
            (0.0, 180.0),
            (0.0, -180.0),
            (40.758, -73.9855),
        ] {
            let (lat, lon) = (lat.to_radians(), lon.to_radians());
            let expected = (gaussian_local_glow(lat, lon), horizon_domes(lat, lon));
            for _ in 0..2 {
                let actual = geographic_glow(lat, lon);
                assert_eq!(actual.0.to_bits(), expected.0.to_bits());
                assert_eq!(actual.1.map(f32::to_bits), expected.1.map(f32::to_bits));
            }
        }
    }

    #[test]
    fn bundled_catalog_reaches_exceptional_naked_eye_depth() {
        let stars = bright_stars();
        assert_eq!(stars.len(), 34_491);
        assert!(
            stars
                .windows(2)
                .all(|pair| pair[0].magnitude <= pair[1].magnitude)
        );
        assert!(stars.iter().all(|star| star.magnitude <= 7.8));
        assert!(stars.iter().all(|star| {
            let d = DVec3::from_array(star.direction_icrs.map(f64::from));
            (d.length() - 1.0).abs() < 1e-5
        }));
    }

    #[test]
    fn bundled_milky_way_texture_is_full_sphere_rgba() {
        let (width, height, pixels) = milky_way_texture();
        assert_eq!((width, height), (1024, 512));
        assert_eq!(pixels.len(), 1024 * 512 * 4);
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[0] > 64));
    }

    #[test]
    fn star_submission_prefix_tracks_current_visibility_budget() {
        let magnitudes: Vec<f32> = bright_stars()
            .into_iter()
            .map(|star| star.magnitude)
            .collect();
        let daylight = star_draw_count(&magnitudes, -2.0);
        let city = star_draw_count(&magnitudes, 4.0);
        let rural = star_draw_count(&magnitudes, 6.5);
        let exceptional = star_draw_count(&magnitudes, 7.8);
        assert_eq!(daylight, 0);
        assert!(city > 500 && city < 1_000);
        assert!(rural > 8_000 && rural < 20_000);
        assert!(city < rural);
        assert!(rural < exceptional);
        assert_eq!(exceptional, magnitudes.len() as u32);
    }

    #[test]
    fn southern_cross_anchor_is_in_the_catalog() {
        // Acrux: ICRS RA ~= 186.65°, Dec ~= -63.10°.
        let ra = 186.65_f64.to_radians();
        let dec = (-63.10_f64).to_radians();
        let expected = DVec3::new(dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin());
        assert!(bright_stars().iter().any(|star| {
            DVec3::from_array(star.direction_icrs.map(f64::from)).dot(expected) > 0.999_99
        }));
    }

    #[test]
    fn bortle_mapping_tracks_naked_eye_thresholds() {
        assert_eq!(bortle_from_limiting_magnitude(7.8), 1);
        assert_eq!(bortle_from_limiting_magnitude(6.3), 4);
        assert_eq!(bortle_from_limiting_magnitude(4.8), 7);
        assert_eq!(bortle_from_limiting_magnitude(3.5), 9);
    }

    fn moonless_midnight(position: DVec3) -> Lighting {
        let surface = vearth_core::ellipsoid::surface_point_along(position);
        let up = vearth_core::ellipsoid::geodetic_normal(surface);
        Lighting {
            direction_ecef: -up,
            moon_direction_ecef: -up,
            moon_illumination: 0.0,
            utc_seconds: 0.0,
        }
    }

    #[test]
    fn black_marble_separates_city_dark_site_and_altitude_escape() {
        let london = vearth_core::ellipsoid::geodetic_to_ecef(
            51.5074_f64.to_radians(),
            (-0.1278_f64).to_radians(),
            50.0,
        );
        let namib = vearth_core::ellipsoid::geodetic_to_ecef(
            (-23.0_f64).to_radians(),
            15.0_f64.to_radians(),
            1_000.0,
        );
        let london_high = vearth_core::ellipsoid::geodetic_to_ecef(
            51.5074_f64.to_radians(),
            (-0.1278_f64).to_radians(),
            5_000.0,
        );
        let city = SkyVisibility::at(london, moonless_midnight(london));
        let dark = SkyVisibility::at(namib, moonless_midnight(namib));
        let high = SkyVisibility::at(london_high, moonless_midnight(london_high));
        assert!(city.local_glow > dark.local_glow + 0.2);
        assert!(city.limiting_magnitude < dark.limiting_magnitude - 1.0);
        assert!(high.limiting_magnitude > city.limiting_magnitude + 1.0);
    }

    #[test]
    fn full_moon_and_twilight_reduce_the_current_limit() {
        let position = vearth_core::ellipsoid::geodetic_to_ecef(
            (-23.0_f64).to_radians(),
            15.0_f64.to_radians(),
            1_000.0,
        );
        let surface = vearth_core::ellipsoid::surface_point_along(position);
        let up = vearth_core::ellipsoid::geodetic_normal(surface);
        let new_moon = moonless_midnight(position);
        let full_moon = Lighting {
            moon_direction_ecef: up,
            moon_illumination: 1.0,
            ..new_moon
        };
        let twilight = Lighting {
            direction_ecef: up,
            ..new_moon
        };
        let dark = SkyVisibility::at(position, new_moon).limiting_magnitude;
        assert!(SkyVisibility::at(position, full_moon).limiting_magnitude < dark - 1.0);
        assert!(SkyVisibility::at(position, twilight).limiting_magnitude < 0.0);
    }
}
