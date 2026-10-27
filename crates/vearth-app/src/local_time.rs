//! Debounced, bounded-memory civil-time resolution for the HUD.

use jiff::{Timestamp, tz::TimeZoneDatabase};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use vearth_data::timezones::TimezoneAtlas;
use vearth_platform::time::{Duration, Instant};

const POSITION_EPSILON_M: f64 = 1.0;
const SETTLE_DELAY: Duration = Duration::from_millis(400);
const MAX_MOVING_INTERVAL: Duration = Duration::from_secs(2);
const EARTH_RADIUS_M: f64 = 6_371_008.8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalTimeHud {
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Copy, Debug)]
struct LookupRequest {
    sequence: u64,
    lat_rad: f64,
    lon_rad: f64,
}

#[derive(Debug)]
struct LookupResponse {
    sequence: u64,
    lat_rad: f64,
    lon_rad: f64,
    timezone: Option<String>,
}

/// Resolves camera coordinates away from the frame loop and caches only the
/// resulting IANA name plus the current minute's formatted label.
pub struct LocalTimeTracker {
    tx: SyncSender<LookupRequest>,
    rx: Receiver<LookupResponse>,
    next_sequence: u64,
    in_flight: Option<u64>,
    observed: Option<(f64, f64)>,
    last_moved_at: Instant,
    last_submitted_at: Option<Instant>,
    resolved_at: Option<(f64, f64)>,
    timezone: Option<String>,
    twenty_four_hour: bool,
    formatted_minute: Option<i64>,
    formatted_timezone: Option<String>,
    formatted: Option<LocalTimeHud>,
}

impl LocalTimeTracker {
    pub fn new(now: Instant) -> Self {
        let (tx, request_rx) = sync_channel::<LookupRequest>(1);
        let (response_tx, rx) = sync_channel::<LookupResponse>(1);
        vearth_platform::thread::Builder::new()
            .name("timezone-resolver".into())
            .spawn(move || {
                let atlas = TimezoneAtlas::bundled();
                while let Ok(mut request) = request_rx.recv() {
                    // Coalesce any camera updates queued while the previous
                    // candidate polygon was being streamed.
                    while let Ok(newer) = request_rx.try_recv() {
                        request = newer;
                    }
                    let timezone = atlas.timezone_at(request.lat_rad, request.lon_rad);
                    if response_tx
                        .send(LookupResponse {
                            sequence: request.sequence,
                            lat_rad: request.lat_rad,
                            lon_rad: request.lon_rad,
                            timezone,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("spawn timezone resolver");
        Self {
            tx,
            rx,
            next_sequence: 0,
            in_flight: None,
            observed: None,
            last_moved_at: now,
            last_submitted_at: None,
            resolved_at: None,
            timezone: None,
            twenty_four_hour: false,
            formatted_minute: None,
            formatted_timezone: None,
            formatted: None,
        }
    }

    pub fn twenty_four_hour(&self) -> bool {
        self.twenty_four_hour
    }

    pub fn set_twenty_four_hour(&mut self, enabled: bool) {
        if self.twenty_four_hour == enabled {
            return;
        }
        self.twenty_four_hour = enabled;
        self.formatted_minute = None;
        self.formatted = None;
    }

    /// Observe the camera position and submit an exact lookup after movement
    /// settles. During continuous travel, refresh at a bounded two-second rate.
    pub fn update(&mut self, lat_rad: f64, lon_rad: f64, now: Instant) {
        while let Ok(response) = self.rx.try_recv() {
            if self.in_flight != Some(response.sequence) {
                continue;
            }
            self.in_flight = None;
            self.resolved_at = Some((response.lat_rad, response.lon_rad));
            if response.timezone != self.timezone {
                self.timezone = response.timezone;
                self.formatted_minute = None;
                self.formatted_timezone = None;
                self.formatted = None;
            }
        }

        if !lat_rad.is_finite() || !lon_rad.is_finite() {
            return;
        }
        let position = (lat_rad, lon_rad);
        if self
            .observed
            .is_none_or(|previous| distance_m(previous, position) > POSITION_EPSILON_M)
        {
            self.observed = Some(position);
            self.last_moved_at = now;
        }

        let needs_lookup = self
            .resolved_at
            .is_none_or(|resolved| distance_m(resolved, position) > POSITION_EPSILON_M);
        if !needs_lookup || self.in_flight.is_some() {
            return;
        }
        let initial = self.resolved_at.is_none() && self.last_submitted_at.is_none();
        if !lookup_due(initial, self.last_moved_at, self.last_submitted_at, now) {
            return;
        }

        self.next_sequence = self.next_sequence.wrapping_add(1);
        let request = LookupRequest {
            sequence: self.next_sequence,
            lat_rad,
            lon_rad,
        };
        match self.tx.try_send(request) {
            Ok(()) => {
                self.in_flight = Some(request.sequence);
                self.last_submitted_at = Some(now);
            }
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => {
                log::error!("timezone resolver stopped unexpectedly");
            }
        }
    }

    pub fn hud(&mut self, utc_seconds: f64) -> Option<&LocalTimeHud> {
        let timezone = self.timezone.as_deref()?;
        if !utc_seconds.is_finite() {
            return None;
        }
        let minute = (utc_seconds.floor() as i64).div_euclid(60);
        let cache_matches = self.formatted_minute == Some(minute)
            && self.formatted_timezone.as_deref() == Some(timezone);
        if !cache_matches {
            match format_local_time(utc_seconds, timezone, self.twenty_four_hour) {
                Ok(formatted) => {
                    self.formatted = Some(formatted);
                    self.formatted_minute = Some(minute);
                    self.formatted_timezone = Some(timezone.to_owned());
                }
                Err(error) => {
                    log::error!("local time for {timezone}: {error}");
                    self.formatted = None;
                    return None;
                }
            }
        }
        self.formatted.as_ref()
    }
}

fn lookup_due(
    initial: bool,
    last_moved_at: Instant,
    last_submitted_at: Option<Instant>,
    now: Instant,
) -> bool {
    initial
        || now.duration_since(last_moved_at) >= SETTLE_DELAY
        || last_submitted_at
            .is_none_or(|submitted| now.duration_since(submitted) >= MAX_MOVING_INTERVAL)
}

fn format_local_time(
    utc_seconds: f64,
    timezone: &str,
    twenty_four_hour: bool,
) -> Result<LocalTimeHud, String> {
    let seconds = utc_seconds.floor() as i64;
    let nanos = ((utc_seconds - seconds as f64) * 1_000_000_000.0) as i32;
    let timestamp = Timestamp::new(seconds, nanos).map_err(|error| error.to_string())?;
    // A fresh lightweight database handle keeps Jiff's parsed-zone cache from
    // growing as the user travels. The returned zone lives only for this
    // once-per-minute format operation; bundled TZif bytes remain static.
    let zone = TimeZoneDatabase::bundled()
        .get(timezone)
        .map_err(|error| error.to_string())?;
    let zoned = timestamp.to_zoned(zone);
    let (label_format, detail_format) = if twenty_four_hour {
        ("%H:%M %Z", "%Y-%m-%d %H:%M:%S %Z")
    } else {
        ("%-I:%M %P %Z", "%Y-%m-%d %-I:%M:%S %P %Z")
    };
    let label = zoned.strftime(label_format).to_string();
    let detail = format!(
        "Simulation local time\n{} · {} · UTC{}",
        zoned.strftime(detail_format),
        timezone,
        zoned.offset(),
    );
    Ok(LocalTimeHud { label, detail })
}

fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let delta_lat = b.0 - a.0;
    let delta_lon =
        (b.1 - a.1 + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    let haversine =
        (delta_lat * 0.5).sin().powi(2) + a.0.cos() * b.0.cos() * (delta_lon * 0.5).sin().powi(2);
    2.0 * EARTH_RADIUS_M * haversine.sqrt().min(1.0).asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(value: &str) -> f64 {
        crate::solar::parse_utc_datetime(value).unwrap()
    }

    #[test]
    fn clock_format_toggle_reformats_the_cached_minute() {
        let mut tracker = LocalTimeTracker::new(Instant::now());
        assert!(!tracker.twenty_four_hour());
        tracker.timezone = Some("Australia/Sydney".into());
        let utc_seconds = utc("2026-01-15T00:00:00Z");
        assert_eq!(tracker.hud(utc_seconds).unwrap().label, "11:00 am AEDT");

        tracker.set_twenty_four_hour(true);
        assert!(tracker.twenty_four_hour());
        assert_eq!(tracker.hud(utc_seconds).unwrap().label, "11:00 AEDT");
    }

    #[test]
    fn bundled_rules_apply_dst_and_fractional_offsets_in_both_clock_formats() {
        assert_eq!(
            format_local_time(utc("2026-01-15T00:00:00Z"), "Australia/Sydney", false,)
                .unwrap()
                .label,
            "11:00 am AEDT"
        );
        assert_eq!(
            format_local_time(utc("2026-07-15T00:00:00Z"), "Australia/Sydney", true,)
                .unwrap()
                .label,
            "10:00 AEST"
        );
        assert_eq!(
            format_local_time(utc("2026-07-15T12:00:00Z"), "Asia/Kathmandu", false)
                .unwrap()
                .label,
            "5:45 pm +0545"
        );
        assert_eq!(
            format_local_time(utc("2026-07-15T12:00:00Z"), "Asia/Kathmandu", true)
                .unwrap()
                .label,
            "17:45 +0545"
        );
    }

    #[test]
    fn distance_wraps_across_the_antimeridian() {
        let distance = distance_m(
            (0.0, 179.9_f64.to_radians()),
            (0.0, -179.9_f64.to_radians()),
        );
        assert!(
            (distance - 22_239.0).abs() < 50.0,
            "distance was {distance}"
        );
    }

    #[test]
    fn lookups_debounce_motion_but_do_not_starve_continuous_travel() {
        let start = Instant::now();
        assert!(lookup_due(true, start, None, start));
        assert!(!lookup_due(
            false,
            start + Duration::from_millis(100),
            Some(start),
            start + Duration::from_millis(300),
        ));
        assert!(lookup_due(false, start, Some(start), start + SETTLE_DELAY,));
        assert!(lookup_due(
            false,
            start + Duration::from_millis(1900),
            Some(start),
            start + MAX_MOVING_INTERVAL,
        ));
    }
}
