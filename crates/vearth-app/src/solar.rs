//! Astronomical Sun/Moon positioning and the day/night clock.
//!
//! Sun and Moon lighting remain render-time concerns. The calendar month also
//! selects the matching NASA Blue Marble surface atlas; crossing a month
//! boundary therefore refreshes generated terrain colours. The solar model is
//! the compact low-precision formula from the Astronomical Almanac. Its
//! sub-degree accuracy is comfortably inside the visual accuracy required by
//! the voxel renderer while keeping the application dependency-free.

use glam::DVec3;

const SECONDS_PER_DAY: f64 = 86_400.0;
const JULIAN_DATE_UNIX_EPOCH: f64 = 2_440_587.5;
const JULIAN_DATE_J2000: f64 = 2_451_545.0;

/// Initial clock state supplied by CLI/deep-link entry points.
#[derive(Clone, Copy, Debug, Default)]
pub enum SolarStart {
    /// Begin at the wall clock and advance in real time.
    #[default]
    Live,
    /// Begin at an explicit UTC instant. Frozen unless a non-zero scale is
    /// supplied separately.
    At(f64),
}

/// The values consumed by the renderer for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Lighting {
    /// Earth-to-Sun direction in ECEF.
    pub direction_ecef: DVec3,
    /// Earth-to-Moon direction in ECEF. The visual model ignores the small
    /// observer parallax because sub-degree accuracy is sufficient here.
    pub moon_direction_ecef: DVec3,
    /// Visible illuminated fraction, from 0 at new Moon to 1 at full Moon.
    pub moon_illumination: f64,
    /// Simulation UTC used to rotate the inertial star catalog with Earth.
    pub utc_seconds: f64,
}

impl Lighting {
    /// Navigation lights come on shortly before sunset and stay on through
    /// civil twilight/night.
    pub fn is_dark_at(self, position_ecef: DVec3) -> bool {
        let surface = vearth_core::ellipsoid::surface_point_along(position_ecef);
        let up = vearth_core::ellipsoid::geodetic_normal(surface);
        up.dot(self.direction_ecef) < 4.0_f64.to_radians().sin()
    }
}

/// Mutable application clock for the day/night cycle.
#[derive(Clone, Copy, Debug)]
pub struct SolarClock {
    utc_seconds: f64,
    time_scale: f64,
    paused: bool,
}

impl SolarClock {
    pub fn new(start: SolarStart, time_scale: Option<f64>) -> Self {
        let utc_seconds = match start {
            SolarStart::Live => unix_now_seconds(),
            SolarStart::At(seconds) => seconds,
        };
        let paused = matches!(start, SolarStart::At(_)) && time_scale.is_none();
        Self {
            utc_seconds,
            time_scale: sanitize_scale(time_scale.unwrap_or(1.0)),
            paused,
        }
    }

    pub fn utc_seconds(&self) -> f64 {
        self.utc_seconds
    }

    pub fn time_scale(&self) -> f64 {
        self.time_scale
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn tick(&mut self, real_dt_seconds: f64) {
        if !self.paused {
            self.utc_seconds += real_dt_seconds.max(0.0) * self.time_scale;
        }
    }

    pub fn toggle_paused(&mut self) {
        self.paused = !self.paused;
    }

    pub fn shift_hours(&mut self, hours: f64) {
        self.utc_seconds += hours * 3600.0;
    }

    /// Set an explicit astronomical instant without resetting the selected
    /// date to wall-clock time. The existing pause/rate state is preserved.
    pub fn set_utc_seconds(&mut self, utc_seconds: f64) {
        if utc_seconds.is_finite() {
            self.utc_seconds = utc_seconds;
        }
    }

    pub fn set_time_scale(&mut self, scale: f64) {
        self.time_scale = sanitize_scale(scale);
        self.paused = false;
    }

    pub fn lighting(&self) -> Lighting {
        let sun = sun_direction_ecef(self.utc_seconds);
        let moon = moon_direction_ecef(self.utc_seconds);
        Lighting {
            direction_ecef: sun,
            moon_direction_ecef: moon,
            moon_illumination: illuminated_fraction(sun, moon),
            utc_seconds: self.utc_seconds,
        }
    }
}

fn sanitize_scale(scale: f64) -> f64 {
    if scale.is_finite() {
        scale.clamp(-10_000.0, 10_000.0)
    } else {
        1.0
    }
}

/// Approximate Earth-to-Sun direction in the project's ECEF axes.
///
/// ECEF +X is 0°N, 0°E; +Y is 0°N, 90°E; +Z is the north pole. The Sun is
/// effectively at infinity, so one direction is valid for the entire globe.
pub fn sun_direction_ecef(unix_seconds: f64) -> DVec3 {
    let jd = unix_seconds / SECONDS_PER_DAY + JULIAN_DATE_UNIX_EPOCH;
    let n = jd - JULIAN_DATE_J2000;

    let mean_longitude = (280.460 + 0.985_647_4 * n).rem_euclid(360.0).to_radians();
    let mean_anomaly = (357.528 + 0.985_600_3 * n).rem_euclid(360.0).to_radians();
    let ecliptic_longitude = mean_longitude
        + (1.915_f64.to_radians()) * mean_anomaly.sin()
        + (0.020_f64.to_radians()) * (2.0 * mean_anomaly).sin();
    let obliquity = (23.439 - 0.000_000_4 * n).to_radians();

    // Earth-centred inertial direction, equatorial axes.
    let inertial = DVec3::new(
        ecliptic_longitude.cos(),
        obliquity.cos() * ecliptic_longitude.sin(),
        obliquity.sin() * ecliptic_longitude.sin(),
    );

    // Greenwich mean sidereal time rotates the inertial direction into ECEF.
    let gmst = greenwich_sidereal_time(unix_seconds);
    let (sin_gmst, cos_gmst) = gmst.sin_cos();
    DVec3::new(
        cos_gmst * inertial.x + sin_gmst * inertial.y,
        -sin_gmst * inertial.x + cos_gmst * inertial.y,
        inertial.z,
    )
    .normalize()
}

/// Greenwich mean sidereal angle, rotating ICRS-like equatorial directions
/// into the project's Earth-fixed axes. Visual star positions use the same
/// compact Earth-rotation model as the Sun and Moon.
pub fn greenwich_sidereal_time(unix_seconds: f64) -> f64 {
    let jd = unix_seconds / SECONDS_PER_DAY + JULIAN_DATE_UNIX_EPOCH;
    let n = jd - JULIAN_DATE_J2000;
    let centuries = n / 36_525.0;
    (280.460_618_37 + 360.985_647_366_29 * n + 0.000_387_933 * centuries * centuries
        - centuries * centuries * centuries / 38_710_000.0)
        .rem_euclid(360.0)
        .to_radians()
}

/// Approximate Earth-to-Moon direction in the project's ECEF axes.
///
/// This is the compact low-precision lunar series: the dominant longitude
/// and latitude terms are retained, then rotated from the ecliptic through
/// the equatorial frame into ECEF. It is intentionally visual rather than
/// ephemeris-grade, but keeps the disc and phase mutually consistent without
/// adding a date-time or astronomy dependency.
pub fn moon_direction_ecef(unix_seconds: f64) -> DVec3 {
    let jd = unix_seconds / SECONDS_PER_DAY + JULIAN_DATE_UNIX_EPOCH;
    let n = jd - JULIAN_DATE_J2000;
    let radians = |degrees: f64| degrees.rem_euclid(360.0).to_radians();

    let mean_longitude = radians(218.316 + 13.176_396 * n);
    let mean_anomaly = radians(134.963 + 13.064_993 * n);
    let argument_latitude = radians(93.272 + 13.229_350 * n);
    let elongation = radians(297.850 + 12.190_749 * n);
    let solar_anomaly = radians(357.529 + 0.985_600_28 * n);

    let longitude = mean_longitude
        + (6.289 * mean_anomaly.sin()
            + 1.274 * (2.0 * elongation - mean_anomaly).sin()
            + 0.658 * (2.0 * elongation).sin()
            + 0.214 * (2.0 * mean_anomaly).sin()
            - 0.186 * solar_anomaly.sin()
            - 0.059 * (2.0 * elongation - 2.0 * mean_anomaly).sin()
            - 0.057 * (2.0 * elongation - mean_anomaly - solar_anomaly).sin()
            + 0.053 * (2.0 * elongation + mean_anomaly).sin()
            + 0.046 * (2.0 * elongation - solar_anomaly).sin()
            + 0.041 * (mean_anomaly - solar_anomaly).sin()
            - 0.035 * elongation.sin()
            - 0.031 * (mean_anomaly + solar_anomaly).sin()
            - 0.015 * (2.0 * argument_latitude - 2.0 * elongation).sin()
            + 0.011 * (mean_anomaly - 4.0 * elongation).sin())
        .to_radians();
    let latitude = (5.128 * argument_latitude.sin()
        + 0.280 * (mean_anomaly + argument_latitude).sin()
        + 0.277 * (mean_anomaly - argument_latitude).sin()
        + 0.173 * (2.0 * elongation - argument_latitude).sin()
        + 0.055 * (2.0 * elongation + argument_latitude - mean_anomaly).sin()
        + 0.046 * (2.0 * elongation - argument_latitude - mean_anomaly).sin()
        + 0.033 * (2.0 * elongation + argument_latitude).sin()
        + 0.017 * (2.0 * mean_anomaly + argument_latitude).sin())
    .to_radians();

    let (sin_lon, cos_lon) = longitude.sin_cos();
    let (sin_lat, cos_lat) = latitude.sin_cos();
    let ecliptic = DVec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat);
    let obliquity = (23.439 - 0.000_000_4 * n).to_radians();
    let (sin_obliquity, cos_obliquity) = obliquity.sin_cos();
    let inertial = DVec3::new(
        ecliptic.x,
        cos_obliquity * ecliptic.y - sin_obliquity * ecliptic.z,
        sin_obliquity * ecliptic.y + cos_obliquity * ecliptic.z,
    );

    let gmst = greenwich_sidereal_time(unix_seconds);
    let (sin_gmst, cos_gmst) = gmst.sin_cos();
    DVec3::new(
        cos_gmst * inertial.x + sin_gmst * inertial.y,
        -sin_gmst * inertial.x + cos_gmst * inertial.y,
        inertial.z,
    )
    .normalize()
}

fn illuminated_fraction(sun_direction: DVec3, moon_direction: DVec3) -> f64 {
    ((1.0 - sun_direction.dot(moon_direction)) * 0.5).clamp(0.0, 1.0)
}

/// Visible illuminated fraction of the Moon at a UTC instant.
pub fn moon_illuminated_fraction(unix_seconds: f64) -> f64 {
    illuminated_fraction(
        sun_direction_ecef(unix_seconds),
        moon_direction_ecef(unix_seconds),
    )
}

/// Parse the intentionally narrow CLI form `YYYY-MM-DDTHH:MM[:SS]Z`.
pub fn parse_utc_datetime(value: &str) -> Result<f64, String> {
    let value = value.trim();
    let body = value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix('z'))
        .ok_or_else(|| "UTC time must end in Z".to_string())?;
    let (date, time) = body
        .split_once('T')
        .or_else(|| body.split_once(' '))
        .ok_or_else(|| "expected YYYY-MM-DDTHH:MM[:SS]Z".to_string())?;
    let mut date_parts = date.split('-');
    let year: i32 = parse_part(date_parts.next(), "year")?;
    let month: u32 = parse_part(date_parts.next(), "month")?;
    let day: u32 = parse_part(date_parts.next(), "day")?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return Err("invalid UTC date".into());
    }
    let max_day = days_in_month(year, month);
    if day == 0 || day > max_day {
        return Err("invalid UTC day".into());
    }

    let mut time_parts = time.split(':');
    let hour: u32 = parse_part(time_parts.next(), "hour")?;
    let minute: u32 = parse_part(time_parts.next(), "minute")?;
    let second: f64 = time_parts
        .next()
        .map(|part| part.parse().map_err(|_| "invalid second".to_string()))
        .transpose()?
        .unwrap_or(0.0);
    if time_parts.next().is_some() || hour > 23 || minute > 59 || !(0.0..60.0).contains(&second) {
        return Err("invalid UTC time".into());
    }
    Ok(days_from_civil(year, month, day) as f64 * SECONDS_PER_DAY
        + hour as f64 * 3600.0
        + minute as f64 * 60.0
        + second)
}

/// Replace only the civil UTC date while retaining the current time of day.
/// Used by the native menu editor so changing season does not unexpectedly
/// move a carefully framed sunset back to midnight.
pub fn replace_utc_date(unix_seconds: f64, date: &str) -> Result<f64, String> {
    let seconds = (unix_seconds.floor() as i64).rem_euclid(86_400);
    let hour = seconds / 3600;
    let minute = (seconds % 3600) / 60;
    let second = seconds % 60;
    parse_utc_datetime(&format!(
        "{}T{hour:02}:{minute:02}:{second:02}Z",
        date.trim()
    ))
}

fn parse_part<T: std::str::FromStr>(value: Option<&str>, name: &str) -> Result<T, String> {
    value
        .ok_or_else(|| format!("missing {name}"))?
        .parse()
        .map_err(|_| format!("invalid {name}"))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

// Howard Hinnant's civil-calendar conversion, returning days from 1970-01-01.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let shifted_month = month as i32 + if month > 2 { -3 } else { 9 };
    let doy = (153 * shifted_month + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) as i64
}

/// Compact UTC label used by the HUD. Calendar conversion is kept local to
/// avoid pulling in a date-time dependency.
pub fn format_utc(unix_seconds: f64) -> String {
    let whole = unix_seconds.floor() as i64;
    let days = whole.div_euclid(86_400);
    let seconds = whole.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3600;
    let minute = (seconds % 3600) / 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

/// Short UTC clock label for the compact solar controls.
pub fn format_utc_time(unix_seconds: f64) -> String {
    let seconds = (unix_seconds.floor() as i64).rem_euclid(86_400);
    let hour = seconds / 3600;
    let minute = (seconds % 3600) / 60;
    format!("{hour:02}:{minute:02} UTC")
}

/// Civil date used to seed the menu-bar date editor.
pub fn format_utc_date(unix_seconds: f64) -> String {
    let days = (unix_seconds.floor() as i64).div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// One-based UTC calendar month used by monthly surface and climate atlases.
pub fn utc_month(unix_seconds: f64) -> u8 {
    let days = (unix_seconds.floor() as i64).div_euclid(86_400);
    civil_from_days(days).1 as u8
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = (yoe + era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i32::from(month <= 2);
    (year, month as u32, day as u32)
}

pub fn unix_now_seconds() -> f64 {
    vearth_platform::time::SystemTime::now()
        .duration_since(vearth_platform::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_cycle_is_the_default_start_mode() {
        assert!(matches!(SolarStart::default(), SolarStart::Live));
        let mut clock = SolarClock::new(SolarStart::default(), None);
        assert!(!clock.paused());
        let started = clock.utc_seconds();
        clock.tick(2.0);
        assert_eq!(clock.utc_seconds(), started + 2.0);
    }

    #[test]
    fn parses_and_formats_utc_without_a_datetime_dependency() {
        assert_eq!(parse_utc_datetime("1970-01-01T00:00:00Z").unwrap(), 0.0);
        assert_eq!(
            parse_utc_datetime("2000-01-01T12:00Z").unwrap(),
            946_728_000.0
        );
        let leap = parse_utc_datetime("2024-02-29T23:59:30Z").unwrap();
        assert_eq!(format_utc(leap), "2024-02-29 23:59 UTC");
        assert_eq!(format_utc_time(leap), "23:59 UTC");
        assert_eq!(format_utc_date(leap), "2024-02-29");
        assert_eq!(utc_month(leap), 2);
        let midsummer = replace_utc_date(leap, "2026-07-19").unwrap();
        assert_eq!(format_utc(midsummer), "2026-07-19 23:59 UTC");
        assert_eq!(utc_month(midsummer), 7);
        assert!(parse_utc_datetime("2023-02-29T00:00Z").is_err());
        assert!(replace_utc_date(leap, "2025-02-29").is_err());
        assert!(parse_utc_datetime("2024-01-01T00:00").is_err());
    }

    #[test]
    fn equinox_noon_and_midnight_face_opposite_directions() {
        let direction_at = |date| {
            SolarClock::new(SolarStart::At(parse_utc_datetime(date).unwrap()), None)
                .lighting()
                .direction_ecef
        };
        let noon = direction_at("2024-03-20T12:00:00Z");
        let midnight = direction_at("2024-03-20T00:00:00Z");
        let greenwich_up = DVec3::X;
        assert!(noon.dot(greenwich_up) > 0.99, "noon sun was {noon:?}");
        assert!(
            midnight.dot(greenwich_up) < -0.99,
            "midnight sun was {midnight:?}"
        );
        assert!(
            noon.z.abs() < 0.01,
            "equinox declination was {}",
            noon.z.asin().to_degrees()
        );
    }

    #[test]
    fn solstice_declination_tracks_earths_tilt() {
        let june = sun_direction_ecef(parse_utc_datetime("2024-06-20T20:51:00Z").unwrap());
        let december = sun_direction_ecef(parse_utc_datetime("2024-12-21T09:20:00Z").unwrap());
        assert!((june.z.asin().to_degrees() - 23.44).abs() < 0.5);
        assert!((december.z.asin().to_degrees() + 23.44).abs() < 0.5);
        assert!((june.length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn lunar_direction_and_phase_track_known_new_and_full_moons() {
        let eclipse = parse_utc_datetime("2024-04-08T18:21:00Z").unwrap();
        let full = parse_utc_datetime("2024-03-25T07:00:00Z").unwrap();
        let eclipse_moon = moon_direction_ecef(eclipse);
        assert!((eclipse_moon.length() - 1.0).abs() < 1e-12);
        assert!(
            moon_illuminated_fraction(eclipse) < 0.01,
            "eclipse illumination was {}",
            moon_illuminated_fraction(eclipse)
        );
        assert!(
            moon_illuminated_fraction(full) > 0.99,
            "full-moon illumination was {}",
            moon_illuminated_fraction(full)
        );
    }

    #[test]
    fn explicit_clock_is_deterministic_and_controllable() {
        let start = parse_utc_datetime("2026-07-14T06:00:00Z").unwrap();
        let mut clock = SolarClock::new(SolarStart::At(start), Some(60.0));
        clock.tick(2.0);
        assert_eq!(clock.utc_seconds(), start + 120.0);
        clock.toggle_paused();
        clock.tick(10.0);
        assert_eq!(clock.utc_seconds(), start + 120.0);
        clock.shift_hours(1.0);
        assert_eq!(clock.utc_seconds(), start + 3720.0);

        let mut frozen = SolarClock::new(SolarStart::At(start), None);
        assert!(frozen.paused());
        assert_eq!(frozen.time_scale(), 1.0);
        frozen.tick(10.0);
        assert_eq!(frozen.utc_seconds(), start);
        frozen.toggle_paused();
        frozen.tick(10.0);
        assert_eq!(frozen.utc_seconds(), start + 10.0);
    }

    #[test]
    fn drone_darkness_uses_local_solar_elevation() {
        let position = vearth_core::ellipsoid::geodetic_to_ecef(0.0, 0.0, 100.0);
        let noon = Lighting {
            direction_ecef: DVec3::X,
            moon_direction_ecef: -DVec3::X,
            moon_illumination: 1.0,
            utc_seconds: 0.0,
        };
        let midnight = Lighting {
            direction_ecef: -DVec3::X,
            moon_direction_ecef: DVec3::X,
            moon_illumination: 1.0,
            utc_seconds: 0.0,
        };
        assert!(!noon.is_dark_at(position));
        assert!(midnight.is_dark_at(position));
    }
}
