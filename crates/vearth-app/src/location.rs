//! Coarse location transitions and arrival-toast timing.
//!
//! The worker resolves meaningful named geography from the same Protomaps
//! archive as the terrain and composes it with a bundled containing region.
//! The frame loop only sends occasional coordinates and never blocks on I/O.

use crate::player::Player;
use crate::streaming::ResidentStore;
use glam::DVec3;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use vearth_data::vectors::{NamedLocation, ProtomapsSource};
use vearth_platform::time::{Duration, Instant};

const MAX_LOCATION_AGL_M: f64 = 19_000.0;
const QUERY_TRAVEL_M: f64 = 750.0;
const CHANGE_CONFIRMATION: Duration = Duration::from_secs(2);
const TOAST_FADE_IN_S: f32 = 0.45;
const TOAST_HOLD_S: f32 = 4.60;
const TOAST_FADE_OUT_S: f32 = 1.00;

struct ResolveRequest {
    sequence: u64,
    lat: f64,
    lon: f64,
}

struct ResolveResponse {
    sequence: u64,
    result: Result<Option<NamedLocation>, String>,
}

#[derive(Clone)]
struct ActiveToast {
    title: Arc<crate::location_title::ArrivalTitle>,
    name: String,
    started: Instant,
}

/// Fully composed toast state consumed by the immediate-mode UI.
#[derive(Debug, Clone, PartialEq)]
pub struct LocationToast {
    pub name: String,
    pub alpha: f32,
    pub elapsed_s: f32,
    pub title: Arc<crate::location_title::ArrivalTitle>,
}

pub struct LocationTracker {
    tx: Option<Sender<ResolveRequest>>,
    rx: Receiver<ResolveResponse>,
    next_sequence: u64,
    in_flight: Option<u64>,
    last_query_pos: Option<DVec3>,
    last_response_at: Option<Instant>,
    failed_at: Option<Instant>,
    was_active: bool,
    current: Option<NamedLocation>,
    candidate: Option<(NamedLocation, Instant)>,
    empty_since: Option<Instant>,
    toast: Option<ActiveToast>,
    // Explicit requests also work while automatic drone tracking is inactive.
    requested_title: Option<(u64, DVec3)>,
    manual_toast: Option<ActiveToast>,
}

impl LocationTracker {
    pub fn new(source: Option<Arc<ProtomapsSource>>) -> Self {
        let (request_tx, request_rx) = channel::<ResolveRequest>();
        let (response_tx, response_rx) = channel::<ResolveResponse>();
        let tx = source.map(|source| {
            vearth_platform::thread::Builder::new()
                .name("location-resolver".into())
                .spawn(move || {
                    #[cfg(feature = "async-providers")]
                    let runtime = source.is_async().then(|| {
                        vearth_platform::rt::runtime::Builder::new_current_thread()
                            .enable_time()
                            .build()
                            .expect("location preparation runtime")
                    });
                    while let Ok(request) = request_rx.recv() {
                        #[cfg(feature = "async-providers")]
                        let result = if let Some(runtime) = &runtime {
                            runtime.block_on(
                                source.resolve_named_location_async(request.lat, request.lon),
                            )
                        } else {
                            source.resolve_named_location(request.lat, request.lon)
                        };
                        #[cfg(not(feature = "async-providers"))]
                        let result = source.resolve_named_location(request.lat, request.lon);
                        let result = result.map_err(|error| error.to_string());
                        if matches!(result, Ok(Some(_))) {
                            crate::location_title::prepare_fonts(Some((request.lat, request.lon)));
                        }
                        if response_tx
                            .send(ResolveResponse {
                                sequence: request.sequence,
                                result,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .expect("spawn location resolver");
            request_tx
        });
        Self {
            tx,
            rx: response_rx,
            next_sequence: 0,
            in_flight: None,
            last_query_pos: None,
            last_response_at: None,
            failed_at: None,
            was_active: false,
            current: None,
            candidate: None,
            empty_since: None,
            toast: None,
            requested_title: None,
            manual_toast: None,
        }
    }

    /// A newly deployed vehicle should identify its location even if play
    /// mode was already active (for example when switching vehicle tiers).
    pub fn restart(&mut self) {
        self.was_active = false;
        self.in_flight = None;
        self.failed_at = None;
        self.current = None;
        self.candidate = None;
        self.empty_since = None;
        self.last_query_pos = None;
        self.toast = None;
        self.requested_title = None;
        self.manual_toast = None;
    }

    pub(crate) fn show_title(&mut self, pos: DVec3, now: Instant) {
        self.requested_title = None;
        if self.current.is_some()
            && self.in_flight.is_none()
            && self.candidate.is_none()
            && self.empty_since.is_none()
            && self
                .last_query_pos
                .is_some_and(|last| last.distance(pos) < QUERY_TRAVEL_M)
            && let Some(toast) = &self.toast
        {
            self.manual_toast = Some(ActiveToast {
                started: now,
                ..toast.clone()
            });
            return;
        }

        let (lat, lon, _) = vearth_core::ellipsoid::ecef_to_geodetic(pos);
        self.next_sequence = self.next_sequence.wrapping_add(1);
        let sequence = self.next_sequence;
        if self
            .tx
            .as_ref()
            .is_some_and(|sender| sender.send(ResolveRequest { sequence, lat, lon }).is_ok())
        {
            self.requested_title = Some((sequence, pos));
        }
    }

    fn low_altitude_position(player: &Player, index: &ResidentStore) -> Option<(DVec3, f64, f64)> {
        let (lat, lon, altitude) = vearth_core::ellipsoid::ecef_to_geodetic(player.pos);
        let terrain = index
            .terrain_top_under(player.pos.normalize_or(DVec3::X))
            .map(f64::from)
            .unwrap_or(0.0);
        let agl = altitude - terrain.max(0.0);
        (agl <= MAX_LOCATION_AGL_M).then_some((player.pos, lat, lon))
    }

    pub fn update(&mut self, player: Option<&Player>, index: &ResidentStore, now: Instant) {
        while let Ok(response) = self.rx.try_recv() {
            self.apply_response(response, now);
        }

        let Some((pos, lat, lon)) =
            player.and_then(|player| Self::low_altitude_position(player, index))
        else {
            self.was_active = false;
            self.failed_at = None;
            self.current = None;
            self.candidate = None;
            self.empty_since = None;
            self.last_query_pos = None;
            self.toast = None;
            return;
        };

        if !self.was_active {
            self.was_active = true;
            self.current = None;
            self.candidate = None;
            self.empty_since = None;
            self.last_query_pos = None;
        }
        if self.in_flight.is_some() || self.tx.is_none() {
            return;
        }
        let travelled = self
            .last_query_pos
            .map_or(f64::INFINITY, |last| last.distance(pos));
        if !self.query_due(travelled, now) {
            return;
        }

        self.next_sequence = self.next_sequence.wrapping_add(1);
        let sequence = self.next_sequence;
        let request = ResolveRequest { sequence, lat, lon };
        if self
            .tx
            .as_ref()
            .is_some_and(|sender| sender.send(request).is_ok())
        {
            self.in_flight = Some(sequence);
            self.last_query_pos = Some(pos);
        }
    }

    fn apply_response(&mut self, response: ResolveResponse, now: Instant) {
        if let Some((sequence, pos)) = self.requested_title
            && sequence == response.sequence
        {
            self.requested_title = None;
            match response.result {
                Ok(Some(location)) => {
                    let (lat, lon, _) = vearth_core::ellipsoid::ecef_to_geodetic(pos);
                    self.manual_toast = Some(ActiveToast {
                        title: Arc::new(crate::location_title::ArrivalTitle::new(
                            &location,
                            Some((lat, lon)),
                        )),
                        name: location.name,
                        started: now,
                    });
                }
                Ok(None) => self.manual_toast = None,
                Err(error) => log::warn!("location lookup: {error}"),
            }
            return;
        }
        if self.in_flight != Some(response.sequence) {
            return;
        }
        self.in_flight = None;
        self.last_response_at = Some(now);
        match response.result {
            Ok(location) => {
                self.failed_at = None;
                self.apply_resolution(location, now);
            }
            Err(error) => {
                self.failed_at = Some(now);
                log::warn!("location lookup: {error}");
            }
        }
    }

    fn query_due(&self, travelled: f64, now: Instant) -> bool {
        // Failed I/O may recover while the observer stays still. Genuine
        // absence follows the ordinary arrival-confirmation policy instead.
        if let Some(failed) = self.failed_at {
            return now.duration_since(failed) >= CHANGE_CONFIRMATION;
        }
        let confirmation_due = (self.candidate.is_some() || self.empty_since.is_some())
            && self
                .last_response_at
                .is_some_and(|last| now.duration_since(last) >= CHANGE_CONFIRMATION);
        travelled >= QUERY_TRAVEL_M || confirmation_due
    }

    fn accept(&mut self, location: NamedLocation, now: Instant) {
        let position = self.last_query_pos.map(|pos| {
            let (lat, lon, _) = vearth_core::ellipsoid::ecef_to_geodetic(pos);
            (lat, lon)
        });
        self.toast = Some(ActiveToast {
            title: Arc::new(crate::location_title::ArrivalTitle::new(
                &location, position,
            )),
            name: location.name.clone(),
            started: now,
        });
        self.current = Some(location);
        self.candidate = None;
        self.empty_since = None;
    }

    fn apply_resolution(&mut self, location: Option<NamedLocation>, now: Instant) {
        let Some(location) = location else {
            self.candidate = None;
            if self.current.is_none() {
                self.empty_since = None;
            } else if self
                .empty_since
                .is_some_and(|since| now.duration_since(since) >= CHANGE_CONFIRMATION)
            {
                self.current = None;
                self.empty_since = None;
            } else {
                self.empty_since.get_or_insert(now);
            }
            return;
        };

        self.empty_since = None;
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.id == location.id)
        {
            self.candidate = None;
            return;
        }
        if self.current.is_none() {
            self.accept(location, now);
            return;
        }
        if let Some((candidate, since)) = &self.candidate
            && candidate.id == location.id
        {
            if now.duration_since(*since) >= CHANGE_CONFIRMATION {
                self.accept(location, now);
            }
            return;
        }
        self.candidate = Some((location, now));
    }

    pub fn toast(&self, now: Instant) -> Option<LocationToast> {
        let toast = self
            .toast
            .iter()
            .chain(self.manual_toast.iter())
            .max_by_key(|toast| toast.started)?;
        let elapsed = now.duration_since(toast.started).as_secs_f32();
        let alpha = toast_alpha(elapsed)?;
        Some(LocationToast {
            name: toast.name.clone(),
            alpha,
            elapsed_s: elapsed,
            title: toast.title.clone(),
        })
    }

    /// Stable accepted local name for the close-range position map. This is
    /// the same already-resolved Protomaps result as the arrival toast; reading
    /// it never performs another source query.
    pub fn current_name(&self) -> Option<&str> {
        self.current.as_ref().map(|location| location.name.as_str())
    }
}

fn toast_alpha(elapsed_s: f32) -> Option<f32> {
    let hold_end = TOAST_FADE_IN_S + TOAST_HOLD_S;
    let end = hold_end + TOAST_FADE_OUT_S;
    if !(0.0..end).contains(&elapsed_s) {
        return None;
    }
    Some(if elapsed_s < TOAST_FADE_IN_S {
        crate::location_title::smoothstep(elapsed_s / TOAST_FADE_IN_S)
    } else if elapsed_s < hold_end {
        1.0
    } else {
        1.0 - crate::location_title::smoothstep((elapsed_s - hold_end) / TOAST_FADE_OUT_S)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(id: u64, name: &str) -> NamedLocation {
        NamedLocation {
            id,
            name: name.into(),
            primary_name: Some(name.into()),
            region_name: None,
            localized_names: Vec::new(),
            distance_m: 0.0,
        }
    }

    #[test]
    fn stationary_lookup_failures_retry_after_delay_but_absence_and_stale_results_do_not() {
        let mut tracker = LocationTracker::new(None);
        let start = Instant::now();
        tracker.in_flight = Some(7);
        tracker.apply_response(
            ResolveResponse {
                sequence: 6,
                result: Err("stale".into()),
            },
            start,
        );
        assert_eq!(tracker.in_flight, Some(7));
        assert!(tracker.failed_at.is_none());
        tracker.apply_response(
            ResolveResponse {
                sequence: 7,
                result: Err("offline".into()),
            },
            start,
        );
        assert!(!tracker.query_due(0.0, start + Duration::from_millis(1999)));
        assert!(tracker.query_due(0.0, start + Duration::from_secs(2)));
        tracker.in_flight = Some(8);
        tracker.apply_response(
            ResolveResponse {
                sequence: 8,
                result: Ok(None),
            },
            start + Duration::from_secs(3),
        );
        assert!(tracker.failed_at.is_none());
        assert!(!tracker.query_due(0.0, start + Duration::from_secs(10)));
        tracker.in_flight = Some(9);
        tracker.apply_response(
            ResolveResponse {
                sequence: 9,
                result: Err("offline".into()),
            },
            start + Duration::from_secs(11),
        );
        tracker.restart();
        assert!(tracker.failed_at.is_none());
        tracker.apply_response(
            ResolveResponse {
                sequence: 9,
                result: Ok(Some(place(1, "Sydney"))),
            },
            start + Duration::from_secs(12),
        );
        assert!(tracker.current_name().is_none());
    }

    #[test]
    fn toast_fades_in_holds_and_fades_out() {
        assert_eq!(toast_alpha(-0.1), None);
        assert_eq!(toast_alpha(0.0), Some(0.0));
        assert!((toast_alpha(0.225).unwrap() - 0.5).abs() < 0.001);
        assert_eq!(toast_alpha(1.0), Some(1.0));
        assert!((toast_alpha(5.55).unwrap() - 0.5).abs() < 0.001);
        assert_eq!(toast_alpha(6.05), None);
    }

    #[test]
    fn manual_title_replays_an_expired_arrival_without_another_lookup() {
        let mut tracker = LocationTracker::new(None);
        let start = Instant::now();
        let pos = vearth_core::ellipsoid::geodetic_to_ecef(-0.59, 2.64, 100.0);
        tracker.last_query_pos = Some(pos);
        tracker.apply_resolution(Some(place(1, "Sydney")), start);
        let later = start + Duration::from_secs(10);
        assert!(tracker.toast(later).is_none());

        tracker.show_title(pos, later);
        let toast = tracker.toast(later + Duration::from_secs(1)).unwrap();
        assert_eq!(toast.name, "Sydney");
        assert_eq!(toast.elapsed_s, 1.0);
        assert_eq!(toast.alpha, 1.0);
        assert!(tracker.requested_title.is_none());
        assert_eq!(tracker.current_name(), Some("Sydney"));

        tracker.apply_resolution(Some(place(2, "Wollongong")), later + Duration::from_secs(1));
        tracker.apply_resolution(Some(place(2, "Wollongong")), later + Duration::from_secs(3));
        assert_eq!(
            tracker.toast(later + Duration::from_secs(3)).unwrap().name,
            "Wollongong"
        );
    }

    #[test]
    fn manual_lookup_uses_requested_position_and_ignores_superseded_responses() {
        let mut tracker = LocationTracker::new(None);
        let (tx, rx) = channel();
        tracker.tx = Some(tx);
        let start = Instant::now();
        let pos = vearth_core::ellipsoid::geodetic_to_ecef(-0.59, 2.64, 100.0);
        tracker.show_title(pos, start);
        let first = rx.try_recv().unwrap();
        assert!((first.lat + 0.59).abs() < 1e-8);
        assert!((first.lon - 2.64).abs() < 1e-8);
        tracker.show_title(pos, start);
        let second = rx.try_recv().unwrap();
        tracker.apply_response(
            ResolveResponse {
                sequence: first.sequence,
                result: Ok(Some(place(1, "Old location"))),
            },
            start,
        );
        assert!(tracker.toast(start).is_none());
        let (response_tx, response_rx) = channel();
        tracker.rx = response_rx;
        response_tx
            .send(ResolveResponse {
                sequence: second.sequence,
                result: Ok(Some(place(2, "Sydney"))),
            })
            .unwrap();
        let index = ResidentStore::new();
        tracker.update(None, &index, start);
        assert_eq!(tracker.toast(start).unwrap().name, "Sydney");
        tracker.update(None, &index, start + Duration::from_secs(1));
        assert_eq!(
            tracker.toast(start + Duration::from_secs(1)).unwrap().alpha,
            1.0
        );
        assert!(tracker.current_name().is_none());
        tracker.restart();
        assert!(tracker.toast(start).is_none());
    }

    #[test]
    fn first_place_is_immediate_but_changes_are_confirmed() {
        let mut tracker = LocationTracker::new(None);
        let start = Instant::now();
        tracker.apply_resolution(Some(place(1, "Sydney")), start);
        assert_eq!(tracker.current.as_ref().unwrap().name, "Sydney");
        assert_eq!(tracker.current_name(), Some("Sydney"));
        assert_eq!(tracker.toast(start).unwrap().name, "Sydney");

        tracker.apply_resolution(Some(place(2, "Wollongong")), start + Duration::from_secs(1));
        assert_eq!(tracker.current.as_ref().unwrap().name, "Sydney");
        tracker.apply_resolution(Some(place(2, "Wollongong")), start + Duration::from_secs(4));
        assert_eq!(tracker.current.as_ref().unwrap().name, "Wollongong");
    }
}
