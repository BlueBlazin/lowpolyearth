//! Camera traces and scripted offline-search actions for native benchmarks.
//!
//! Traces are deliberately small CSV files rather than recordings of winit
//! events.  Each row is an absolute camera pose at a render-frame number, so
//! the same sequence is applied even when one run renders more slowly than
//! another. Missing frames are linearly interpolated. Search scripts dispatch
//! one ordinary UI action and observe the interactive navigation lifecycle.

use crate::camera::Camera;

/// One keyframe in a camera replay.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct ReplayPose {
    pub frame: u64,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub alt_m: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
}

impl ReplayPose {
    fn validate(self, line: usize) -> Result<Self, String> {
        let values = [
            self.lat_deg,
            self.lon_deg,
            self.alt_m,
            self.heading_deg,
            self.pitch_deg,
        ];
        if values.iter().any(|v| !v.is_finite()) {
            return Err(format!("line {line}: pose contains a non-finite value"));
        }
        if !(-90.0..=90.0).contains(&self.lat_deg) {
            return Err(format!("line {line}: latitude must be in [-90, 90]"));
        }
        if self.alt_m < 0.0 {
            return Err(format!("line {line}: altitude must be non-negative"));
        }
        if !(0.0..=89.9).contains(&self.pitch_deg) {
            return Err(format!("line {line}: pitch must be in [0, 89.9]"));
        }
        Ok(self)
    }

    /// Apply this absolute pose without invoking navigation easing.
    pub fn apply(self, camera: &mut Camera) {
        camera.lat = self.lat_deg.to_radians();
        camera.lon = self.lon_deg.to_radians();
        camera.alt_m = self.alt_m;
        camera.heading = self.heading_deg.to_radians();
        camera.pitch = self.pitch_deg.to_radians();
    }

    fn lerp(self, other: Self, frame: u64) -> Self {
        if other.frame <= self.frame || frame <= self.frame {
            return ReplayPose { frame, ..self };
        }
        if frame >= other.frame {
            return ReplayPose { frame, ..other };
        }
        let t = (frame - self.frame) as f64 / (other.frame - self.frame) as f64;
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        // Follow the shortest wrapped longitude/heading arc. This prevents a
        // trace crossing the date line from orbiting almost a full turn.
        let wrapped_lerp = |a: f64, b: f64| {
            let mut d = (b - a) % 360.0;
            if d > 180.0 {
                d -= 360.0;
            } else if d < -180.0 {
                d += 360.0;
            }
            a + d * t
        };
        ReplayPose {
            frame,
            lat_deg: lerp(self.lat_deg, other.lat_deg),
            lon_deg: wrapped_lerp(self.lon_deg, other.lon_deg),
            // Interpolate altitude logarithmically. Zoom traces then have a
            // constant multiplicative speed, matching the interactive camera.
            alt_m: if self.alt_m > 0.0 && other.alt_m > 0.0 {
                (lerp(self.alt_m.ln(), other.alt_m.ln())).exp()
            } else {
                lerp(self.alt_m, other.alt_m)
            },
            heading_deg: wrapped_lerp(self.heading_deg, other.heading_deg),
            pitch_deg: lerp(self.pitch_deg, other.pitch_deg),
        }
    }
}

/// Parsed, immutable trace. The first keyframe must be frame zero.
#[derive(Clone, Debug)]
pub struct ReplayTrace {
    poses: Vec<ReplayPose>,
}

impl ReplayTrace {
    /// Parse `frame,lat_deg,lon_deg,alt_m,heading_deg,pitch_deg` CSV.
    /// Blank lines, `#` comments, and one textual header are accepted.
    pub fn parse(csv: &str) -> Result<Self, String> {
        let mut poses: Vec<ReplayPose> = Vec::new();
        for (line_idx, raw) in csv.lines().enumerate() {
            let line_no = line_idx + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            if fields
                .first()
                .is_some_and(|v| v.eq_ignore_ascii_case("frame"))
            {
                continue;
            }
            if fields.len() != 6 {
                return Err(format!("line {line_no}: expected 6 comma-separated fields"));
            }
            let number = |index: usize, name: &str| -> Result<f64, String> {
                fields[index]
                    .parse::<f64>()
                    .map_err(|_| format!("line {line_no}: invalid {name}"))
            };
            let frame = fields[0]
                .parse::<u64>()
                .map_err(|_| format!("line {line_no}: invalid frame"))?;
            let pose = ReplayPose {
                frame,
                lat_deg: number(1, "latitude")?,
                lon_deg: number(2, "longitude")?,
                alt_m: number(3, "altitude")?,
                heading_deg: number(4, "heading")?,
                pitch_deg: number(5, "pitch")?,
            }
            .validate(line_no)?;
            if let Some(previous) = poses.last()
                && pose.frame <= previous.frame
            {
                return Err(format!(
                    "line {line_no}: frames must be strictly increasing"
                ));
            }
            poses.push(pose);
        }
        if poses.is_empty() {
            return Err("trace contains no poses".into());
        }
        if poses[0].frame != 0 {
            return Err("the first trace pose must be frame 0".into());
        }
        Ok(ReplayTrace { poses })
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes = std::fs::read_to_string(path)
            .map_err(|e| format!("could not read replay {}: {e}", path.display()))?;
        Self::parse(&bytes).map_err(|e| format!("invalid replay {}: {e}", path.display()))
    }

    pub fn last_frame(&self) -> u64 {
        self.poses.last().map_or(0, |pose| pose.frame)
    }

    pub fn pose_at(&self, frame: u64) -> ReplayPose {
        let upper = self.poses.partition_point(|pose| pose.frame < frame);
        match upper {
            0 => self.poses[0],
            i if i >= self.poses.len() => ReplayPose {
                frame,
                ..self.poses[self.poses.len() - 1]
            },
            i => self.poses[i - 1].lerp(self.poses[i], frame),
        }
    }
}

/// Mutable cursor used by the render loop. One call equals one presented
/// frame; wall-clock variance cannot change the sampled camera path.
pub struct TracePlayer {
    trace: ReplayTrace,
    frame: u64,
}

impl TracePlayer {
    pub fn new(trace: ReplayTrace) -> Self {
        TracePlayer { trace, frame: 0 }
    }

    pub fn next_pose(&mut self) -> ReplayPose {
        let pose = self.trace.pose_at(self.frame);
        self.frame = self.frame.saturating_add(1);
        pose
    }

    /// Next trace frame, independent of failed or skipped GPU timing samples.
    pub fn next_frame(&self) -> u64 {
        self.frame
    }

    pub fn finished(&self) -> bool {
        self.frame > self.trace.last_frame()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE: &str = "\
frame,lat_deg,lon_deg,alt_m,heading_deg,pitch_deg
0,0,179,10000,350,0
10,10,-179,1000,10,60
";

    #[test]
    fn trace_interpolation_is_frame_deterministic_and_wraps_angles() {
        let trace = ReplayTrace::parse(TRACE).unwrap();
        let middle = trace.pose_at(5);
        assert_eq!(middle.frame, 5);
        assert!((middle.lat_deg - 5.0).abs() < 1e-9);
        assert!((middle.lon_deg - 180.0).abs() < 1e-9);
        assert!((middle.alt_m - 3162.277660).abs() < 1e-3);
        assert!((middle.heading_deg - 360.0).abs() < 1e-9);
        assert!((middle.pitch_deg - 30.0).abs() < 1e-9);
    }

    #[test]
    fn malformed_and_non_monotonic_traces_are_rejected() {
        assert!(ReplayTrace::parse("").is_err());
        assert!(ReplayTrace::parse("1,0,0,1,0,0").is_err());
        assert!(ReplayTrace::parse("0,0,0,1,0,0\n0,0,0,1,0,0").is_err());
        assert!(ReplayTrace::parse("0,91,0,1,0,0").is_err());
    }

    #[test]
    fn player_advances_exactly_one_trace_frame() {
        let mut player = TracePlayer::new(ReplayTrace::parse(TRACE).unwrap());
        assert_eq!(player.next_pose().frame, 0);
        assert_eq!(player.next_pose().frame, 1);
        for _ in 2..=10 {
            player.next_pose();
        }
        assert!(player.finished());
    }

    #[test]
    fn checked_in_benchmark_traces_are_valid() {
        let traces = [
            include_str!("../testdata/replays/orbit_to_street.csv"),
            include_str!("../testdata/replays/slow_street_pan.csv"),
            include_str!("../testdata/replays/fixed_eye_turn.csv"),
            include_str!("../testdata/replays/anchor_orbit_tilt.csv"),
            include_str!("../testdata/replays/zoom_threshold_jitter.csv"),
            include_str!("../testdata/replays/cold_teleport.csv"),
            include_str!("../testdata/replays/boosted_low_flight.csv"),
            include_str!("../testdata/replays/flora_field_boundary_low_flight.csv"),
            include_str!("../testdata/replays/flora_alpine_descent.csv"),
            include_str!("../testdata/replays/flora_destination_descent.csv"),
        ];
        for csv in traces {
            let trace = ReplayTrace::parse(csv).expect("checked-in replay must parse");
            assert!(trace.last_frame() >= 420, "trace is too short to settle");
        }
    }
}

/// One offline search result accepted through the ordinary UI action handler.
/// This measures navigation and loading, not keyboard or debounce latency.
#[derive(Clone, Debug)]
pub struct SearchBenchmarkSpec {
    pub query: String,
    pub result_index: usize,
    pub trigger_frame: u64,
    pub place: vearth_data::geocode::Place,
    pub index_metadata: serde_json::Value,
    pub drone_flight: bool,
    pub city_tour: bool,
}

impl SearchBenchmarkSpec {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve(query: &str, result_index: usize, trigger_frame: u64) -> Result<Self, String> {
        let query = query.trim();
        if query.is_empty() {
            return Err("--benchmark-search requires a nonempty query".into());
        }
        if trigger_frame == 0 {
            return Err("--benchmark-search-frame must be greater than zero".into());
        }
        let geocoder = vearth_data::geocode::Geocoder::new();
        let results = geocoder.search(query).map_err(|error| error.to_string())?;
        let place = results.get(result_index).cloned().ok_or_else(|| {
            format!(
                "search {query:?} returned {} results; result {result_index} does not exist",
                results.len()
            )
        })?;
        let metadata = geocoder.metadata();
        Ok(Self {
            query: query.to_string(),
            result_index,
            trigger_frame,
            place,
            drone_flight: false,
            city_tour: false,
            index_metadata: serde_json::json!({
                "dataset_date": metadata.dataset_date,
                "source": metadata.source,
                "representative": metadata.representative,
                "record_count": metadata.record_count,
                "alias_count": metadata.alias_count,
                "installed_bytes": metadata.installed_bytes,
            }),
        })
    }
}

/// Observes the real flight without assigning camera poses after dispatch.
#[derive(Debug)]
pub struct SearchBenchmarkPlayer {
    pub spec: SearchBenchmarkSpec,
    dispatched: bool,
    arrived: bool,
    initial_pose: Option<ReplayPose>,
    action_pose: Option<ReplayPose>,
    requested_pose: Option<ReplayPose>,
    arrival_pose: Option<ReplayPose>,
    trigger_after_ms: Option<f64>,
    arrival_after_ms: Option<f64>,
    destination_requests: u64,
    prefetch_adoptions: u64,
    prefetch_cell_observations: u64,
    prefetch_peak_cells: usize,
    latest_requested_selection: Option<u64>,
    latest_adopted_selection: Option<u64>,
    first_post_arrival_request: Option<u64>,
    post_arrival_selection_adopted: bool,
    post_arrival_synchronous_selection: bool,
}

pub fn observed_pose(frame: u64, camera: &Camera) -> ReplayPose {
    ReplayPose {
        frame,
        lat_deg: camera.lat.to_degrees(),
        lon_deg: camera.lon.to_degrees(),
        alt_m: camera.alt_m,
        heading_deg: camera.heading.to_degrees(),
        pitch_deg: camera.pitch.to_degrees(),
    }
}

impl SearchBenchmarkPlayer {
    pub fn new(spec: SearchBenchmarkSpec) -> Self {
        Self {
            spec,
            dispatched: false,
            arrived: false,
            initial_pose: None,
            action_pose: None,
            requested_pose: None,
            arrival_pose: None,
            trigger_after_ms: None,
            arrival_after_ms: None,
            destination_requests: 0,
            prefetch_adoptions: 0,
            prefetch_cell_observations: 0,
            prefetch_peak_cells: 0,
            latest_requested_selection: None,
            latest_adopted_selection: None,
            first_post_arrival_request: None,
            post_arrival_selection_adopted: false,
            post_arrival_synchronous_selection: false,
        }
    }

    pub fn take_action(
        &mut self,
        frame: u64,
        camera: &Camera,
        elapsed_ms: f64,
    ) -> Option<vearth_data::geocode::Place> {
        self.initial_pose
            .get_or_insert_with(|| observed_pose(frame, camera));
        if self.dispatched || frame < self.spec.trigger_frame {
            return None;
        }
        self.dispatched = true;
        self.trigger_after_ms = Some(elapsed_ms);
        self.action_pose = Some(observed_pose(frame, camera));
        // Obtain the requested endpoint from the same flight implementation,
        // including its scale-dependent arrival pitch. Only the copy is moved.
        let mut destination = camera.clone();
        let place = &self.spec.place;
        let mut flight = crate::camera::FlyTo::new(
            camera,
            place.lat_deg,
            place.lon_deg,
            place.view_altitude_m.clamp(200.0, 12_000_000.0),
        );
        flight.update(&mut destination, flight.duration_seconds());
        self.requested_pose = Some(observed_pose(frame, &destination));
        Some(place.clone())
    }

    pub fn mark_arrived(&mut self, frame: u64, camera: &Camera, elapsed_ms: f64) -> bool {
        if !self.dispatched || self.arrived {
            return false;
        }
        self.arrived = true;
        self.arrival_after_ms = Some(elapsed_ms);
        self.arrival_pose = Some(observed_pose(frame, camera));
        true
    }

    pub fn arrived(&self) -> bool {
        self.arrived
    }

    pub fn deadline_reason(
        &self,
        elapsed: vearth_platform::time::Duration,
        since_arrival: Option<vearth_platform::time::Duration>,
        limit: vearth_platform::time::Duration,
    ) -> Option<&'static str> {
        if !self.arrived && elapsed >= limit {
            Some("search_motion_timeout")
        } else if self.arrived && since_arrival.is_some_and(|elapsed| elapsed >= limit) {
            Some("settle_timeout")
        } else {
            None
        }
    }

    pub fn record_selection_request(&mut self, id: u64) {
        self.latest_requested_selection = Some(id);
        if self.arrived {
            self.first_post_arrival_request.get_or_insert(id);
        }
    }

    pub fn record_selection_adopted(&mut self, id: u64) {
        self.latest_adopted_selection = Some(id);
        if self
            .first_post_arrival_request
            .is_some_and(|first| id.wrapping_sub(first) < (1u64 << 63))
        {
            self.post_arrival_selection_adopted = true;
        }
    }

    pub fn record_synchronous_selection(&mut self) {
        // A synchronous computation already reads the current camera and
        // resident metrics; there is no queued worker response to fence.
        self.latest_requested_selection = None;
        self.latest_adopted_selection = None;
        if self.arrived {
            self.post_arrival_selection_adopted = true;
            self.post_arrival_synchronous_selection = true;
        }
    }

    pub(crate) fn reset_selection_fence(&mut self) {
        self.latest_requested_selection = None;
        self.latest_adopted_selection = None;
        self.first_post_arrival_request = None;
        self.post_arrival_selection_adopted = false;
        self.post_arrival_synchronous_selection = false;
    }

    pub fn can_count_idle(&self) -> bool {
        self.arrived && self.post_arrival_selection_adopted
    }

    pub fn can_settle(&self) -> bool {
        self.can_count_idle() && self.latest_requested_selection == self.latest_adopted_selection
    }

    pub fn record_destination_request(&mut self) {
        if self.dispatched && !self.arrived {
            self.destination_requests += 1;
        }
    }

    pub fn record_prefetch(&mut self, cells: usize) {
        if self.dispatched && !self.arrived && cells > 0 {
            self.prefetch_adoptions += 1;
            self.prefetch_cell_observations += cells as u64;
            self.prefetch_peak_cells = self.prefetch_peak_cells.max(cells);
        }
    }

    pub fn report(&self, terminal: Option<ReplayPose>) -> serde_json::Value {
        let place = &self.spec.place;
        serde_json::json!({
            "kind": "offline_search_flyto",
            "query": self.spec.query,
            "drone_flight_requested": self.spec.drone_flight,
            "city_tour_requested": self.spec.city_tour,
            "result_index": self.spec.result_index,
            "trigger_frame": self.spec.trigger_frame,
            "result": { "display_name": place.display_name, "lat_deg": place.lat_deg, "lon_deg": place.lon_deg, "view_altitude_m": place.view_altitude_m },
            "index": self.spec.index_metadata,
            "initial_pose": self.initial_pose,
            "action_pose": self.action_pose,
            "requested_pose": self.requested_pose,
            "arrival_pose": self.arrival_pose,
            "terminal_pose": terminal,
            "trigger_after_ms": self.trigger_after_ms,
            "arrival_after_ms": self.arrival_after_ms,
            "destination_selection_requests": self.destination_requests,
            "prefetch_adoptions": self.prefetch_adoptions,
            "prefetch_cell_observations": self.prefetch_cell_observations,
            "prefetch_peak_cells": self.prefetch_peak_cells,
            "selector_fence": {
                "latest_requested": self.latest_requested_selection,
                "latest_adopted": self.latest_adopted_selection,
                "first_post_arrival_request": self.first_post_arrival_request,
                "post_arrival_selection_adopted": self.post_arrival_selection_adopted,
                "post_arrival_synchronous_selection": self.post_arrival_synchronous_selection,
                "can_count_idle": self.can_count_idle(),
                "can_settle": self.can_settle(),
            },
            "navigation_clock": "interactive_wall_dt",
            "selector_lifecycle": "interactive",
            "keyboard_latency_measured": false,
        })
    }
}

pub fn write_search_startup_timeout(
    path: &std::path::Path,
    spec: &SearchBenchmarkSpec,
    elapsed_ms: f64,
) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let report = serde_json::json!({
        "schema_version": 1,
        "terminal_reason": "startup_timeout",
        "timing": { "startup_wall_ms": elapsed_ms },
        "search": SearchBenchmarkPlayer::new(spec.clone()).report(None),
    });
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod search_benchmark_tests {
    use super::*;

    #[test]
    fn search_benchmark_uses_the_ui_geocoder_result_and_triggers_once() {
        let spec = SearchBenchmarkSpec::resolve("Rotterdam", 0, 3).unwrap();
        let expected = vearth_data::geocode::Geocoder::new()
            .search("Rotterdam")
            .unwrap()
            .remove(0);
        assert_eq!(spec.place, expected);
        assert!(spec.place.view_altitude_m > 3000.0);
        let mut player = SearchBenchmarkPlayer::new(spec);
        let mut camera = Camera::start();
        camera.lat = 0.0;
        camera.lon = 0.0;
        camera.alt_m = camera.max_zoom_out_alt();
        let original = observed_pose(0, &camera);
        assert_eq!(
            player.deadline_reason(
                vearth_platform::time::Duration::from_secs(89),
                None,
                vearth_platform::time::Duration::from_secs(90)
            ),
            None
        );
        assert_eq!(
            player.deadline_reason(
                vearth_platform::time::Duration::from_secs(90),
                None,
                vearth_platform::time::Duration::from_secs(90)
            ),
            Some("search_motion_timeout")
        );
        assert!(player.take_action(1, &camera, 0.0).is_none());
        assert!(player.take_action(2, &camera, 17.0).is_none());
        assert_eq!(player.take_action(3, &camera, 34.0), Some(expected.clone()));
        for frame in 4..10 {
            assert!(player.take_action(frame, &camera, 50.0).is_none());
        }
        assert_eq!(observed_pose(0, &camera), original);
        let requested = player.requested_pose.unwrap();
        assert_eq!(
            requested.alt_m,
            expected.view_altitude_m.clamp(200.0, 12_000_000.0)
        );
        assert_eq!(requested.pitch_deg, 0.0);
        assert!(SearchBenchmarkSpec::resolve("", 0, 1).is_err());
        assert!(SearchBenchmarkSpec::resolve("Rotterdam", 999, 1).is_err());
        assert!(SearchBenchmarkSpec::resolve("Rotterdam", 0, 0).is_err());
    }

    #[test]
    fn search_benchmark_observes_real_landing_and_preserves_later_floor_adjustments() {
        let spec = SearchBenchmarkSpec::resolve("Rotterdam", 0, 1).unwrap();
        let mut player = SearchBenchmarkPlayer::new(spec);
        let mut camera = Camera::start();
        assert!(!player.mark_arrived(0, &camera, 0.0));
        let place = player.take_action(1, &camera, 0.0).unwrap();
        let mut fly = crate::camera::FlyTo::new(
            &camera,
            place.lat_deg,
            place.lon_deg,
            place.view_altitude_m.clamp(200.0, 12_000_000.0),
        );
        let mut frame = 2;
        loop {
            player.record_destination_request();
            player.record_prefetch(7);
            if !fly.update(&mut camera, 1.0 / 60.0) {
                break;
            }
            assert!(!player.arrived());
            assert!(
                player
                    .take_action(frame, &camera, frame as f64 * 1000.0 / 60.0)
                    .is_none()
            );
            frame += 1;
            assert!(frame < 400);
        }
        assert!(player.mark_arrived(frame, &camera, frame as f64 * 1000.0 / 60.0));
        assert_eq!(
            player.arrival_pose.unwrap().alt_m,
            player.requested_pose.unwrap().alt_m
        );
        assert_eq!(
            player.deadline_reason(
                vearth_platform::time::Duration::from_secs(900),
                Some(vearth_platform::time::Duration::from_secs(89)),
                vearth_platform::time::Duration::from_secs(90)
            ),
            None
        );
        assert_eq!(
            player.deadline_reason(
                vearth_platform::time::Duration::from_secs(900),
                Some(vearth_platform::time::Duration::from_secs(90)),
                vearth_platform::time::Duration::from_secs(90)
            ),
            Some("settle_timeout")
        );
        camera.alt_m += 700.0;
        let corrected = observed_pose(frame + 1, &camera);
        assert!(player.take_action(frame + 1, &camera, 9000.0).is_none());
        assert!(!player.mark_arrived(frame + 1, &camera, 9000.0));
        assert_eq!(observed_pose(frame + 1, &camera), corrected);
        let before = player.destination_requests;
        player.record_destination_request();
        player.record_prefetch(1000);
        assert_eq!(player.destination_requests, before);
        let report = player.report(Some(corrected));
        assert_eq!(
            report["terminal_pose"]["alt_m"].as_f64(),
            Some(corrected.alt_m)
        );
        assert_eq!(report["prefetch_peak_cells"], 7);
    }
}

#[cfg(test)]
mod search_selector_fence_tests {
    use super::*;

    fn arrived_player() -> SearchBenchmarkPlayer {
        let mut player =
            SearchBenchmarkPlayer::new(SearchBenchmarkSpec::resolve("Rotterdam", 0, 1).unwrap());
        let camera = Camera::start();
        player.record_selection_request(10);
        player.record_selection_adopted(10);
        player.take_action(1, &camera, 0.0).unwrap();
        assert!(player.mark_arrived(2, &camera, 1000.0));
        player
    }

    #[test]
    fn search_selector_fence_requires_an_adopted_post_arrival_request() {
        let mut player = arrived_player();
        assert!(!player.can_count_idle());
        assert!(!player.can_settle());
        player.record_selection_request(11);
        player.record_selection_adopted(10);
        assert!(!player.can_count_idle());
        player.record_selection_request(12);
        player.record_selection_adopted(12);
        assert!(player.can_count_idle());
        assert!(player.can_settle());
        let report = player.report(None);
        let fence = &report["selector_fence"];
        assert_eq!(fence["first_post_arrival_request"], 11);
        assert_eq!(fence["latest_adopted"], 12);
    }

    #[test]
    fn drone_benchmark_fence_reset_rejects_the_old_camera_selection() {
        let mut player = arrived_player();
        player.record_selection_request(11);
        player.record_selection_adopted(11);
        assert!(player.can_settle());
        player.reset_selection_fence();
        assert!(player.arrived());
        assert!(!player.can_count_idle());
        player.record_selection_adopted(11);
        assert!(!player.can_count_idle());
        player.record_selection_request(12);
        player.record_selection_adopted(11);
        assert!(!player.can_count_idle());
        player.record_selection_request(13);
        player.record_selection_adopted(13);
        assert!(player.can_settle());
        player.reset_selection_fence();
        assert!(!player.can_settle());
        player.record_synchronous_selection();
        assert!(player.can_settle());
    }

    #[test]
    fn search_selector_cadence_preserves_idle_window_but_waits_for_final_adoption() {
        let mut player = arrived_player();
        player.record_selection_request(11);
        player.record_selection_adopted(11);
        let mut idle_frames = 0;
        let mut newest = 11;
        for frame in 0..30 {
            if frame % 7 == 0 {
                newest += 1;
                player.record_selection_request(newest);
                assert!(!player.can_settle());
            } else if frame % 7 == 1 {
                player.record_selection_adopted(newest);
            }
            if player.can_count_idle() {
                idle_frames += 1;
            } else {
                idle_frames = 0;
            }
        }
        assert_eq!(idle_frames, 30);
        player.record_selection_request(newest + 1);
        assert!(player.can_count_idle());
        assert!(!player.can_settle());
        player.record_selection_adopted(newest + 1);
        assert!(player.can_settle());
        let mut synchronous = arrived_player();
        synchronous.record_synchronous_selection();
        assert!(synchronous.can_count_idle() && synchronous.can_settle());
    }
}
