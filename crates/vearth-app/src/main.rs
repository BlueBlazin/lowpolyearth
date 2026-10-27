//! Native entry point: windowed viewer, or `--screenshot` headless mode.

fn main() {
    init_logging();
    let argv: Vec<String> = std::env::args().collect();
    let search_benchmark = match parse_search_benchmark_args(&argv) {
        Ok(search) => search,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    if argv.iter().any(|arg| arg == "--replay-measure-frames")
        && !argv.iter().any(|arg| arg == "--replay")
    {
        eprintln!("--replay-measure-frames requires --replay and --replay-report");
        std::process::exit(2);
    }
    if let Some(capture) = parse_capture_trace_args(&argv) {
        match capture.and_then(run_capture_trace) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("cinematic capture failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(args) = parse_screenshot_args() {
        if let Err(e) = vearth_app::screenshot::run(args) {
            eprintln!("screenshot failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    let pos = parse_position_args();
    let start = vearth_app::app::StartPos {
        lat_deg: pos.lat_deg,
        lon_deg: pos.lon_deg,
        alt_m: pos.alt_m,
        heading_deg: pos.heading_deg,
        pitch_deg: pos.pitch_deg,
        play: pos.play,
        solar_start: pos.solar_start,
        time_scale: pos.time_scale,
        weather: pos.weather,
    };
    let calibration_landmark = parse_calibration_landmark(&argv);
    let stage = match vearth_app::staging::parse_options(&argv) {
        Ok(stage) => stage,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    if let Some(stage) = stage {
        if calibration_landmark.is_some() || argv.iter().any(|arg| arg == "--replay") {
            eprintln!("--stage cannot be combined with --calibrate-landmark or --replay");
            std::process::exit(2);
        }
        vearth_app::staging::run_native(stage);
        return;
    }
    if let Some((search, benchmark)) = search_benchmark {
        vearth_app::app::run_native_search_benchmark(start, search, benchmark);
        return;
    }
    if let Some(path) = argv
        .iter()
        .position(|arg| arg == "--replay")
        .and_then(|index| argv.get(index + 1))
    {
        if calibration_landmark.is_some() {
            eprintln!("--calibrate-landmark cannot be combined with --replay");
            std::process::exit(2);
        }
        let replay = match vearth_app::replay::ReplayTrace::load(path) {
            Ok(replay) => replay,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        };
        let benchmark = match parse_replay_benchmark_args(&argv, path) {
            Ok(benchmark) => benchmark,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        };
        if let Some(range) = benchmark.as_ref().and_then(|b| b.measurement_range)
            && let Err(error) = range.validate_trace(replay.last_frame())
        {
            eprintln!("{error}");
            std::process::exit(2);
        }
        vearth_app::app::run_native_replay_with_benchmark(start, replay, benchmark);
    } else if let Some(landmark) = calibration_landmark {
        vearth_app::app::run_native_landmark_calibrator(start, landmark);
    } else {
        vearth_app::app::run_native(start);
    }
}

fn init_logging() {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    let paths = vearth_app::platform_paths::RuntimePaths::discover();
    if paths.packaged && std::fs::create_dir_all(&paths.logs).is_ok() {
        let current = paths.logs.join("Low Poly Earth.log");
        let previous = paths.logs.join("Low Poly Earth.previous.log");
        const ROTATE_AT_BYTES: u64 = 8 * 1024 * 1024;
        if std::fs::metadata(&current).is_ok_and(|metadata| metadata.len() >= ROTATE_AT_BYTES) {
            let _ = std::fs::remove_file(&previous);
            let _ = std::fs::rename(&current, &previous);
        }
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(current)
        {
            builder.target(env_logger::Target::Pipe(Box::new(file)));
        }
    }
    builder.init();
}

fn parse_calibration_landmark(argv: &[String]) -> Option<&'static vearth_gen::landmarks::Landmark> {
    let index = argv.iter().position(|arg| arg == "--calibrate-landmark")?;
    let Some(name) = argv.get(index + 1).filter(|name| !name.starts_with('-')) else {
        return vearth_gen::landmarks::LANDMARKS.first();
    };
    if let Some(landmark) = vearth_gen::landmarks::LANDMARKS
        .iter()
        .find(|landmark| landmark.name == name)
    {
        return Some(landmark);
    }
    let available = vearth_gen::landmarks::LANDMARKS
        .iter()
        .map(|landmark| landmark.name)
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("unknown landmark {name:?}. Available: {available}");
    std::process::exit(2);
}

fn parse_replay_benchmark_args(
    argv: &[String],
    trace_path: &str,
) -> Result<Option<vearth_app::app::ReplayBenchmarkOptions>, String> {
    let measurement_flag_count = argv
        .iter()
        .filter(|a| *a == "--replay-measure-frames")
        .count();
    if measurement_flag_count > 1 {
        return Err("--replay-measure-frames must be provided only once".into());
    }
    let Some(report_path) = argv
        .iter()
        .position(|arg| arg == "--replay-report")
        .and_then(|index| argv.get(index + 1))
    else {
        if measurement_flag_count > 0 {
            return Err("--replay-measure-frames requires --replay-report PATH".into());
        }
        return Ok(None);
    };
    if measurement_flag_count > 0 && report_path.starts_with("--") {
        return Err("--replay-measure-frames requires --replay-report PATH".into());
    }

    let value_after = |flag: &str| {
        argv.iter()
            .position(|arg| arg == flag)
            .and_then(|index| argv.get(index + 1))
    };
    let max_settle_seconds = value_after("--replay-max-settle-seconds")
        .map(|value| {
            value.parse::<f64>().map_err(|_| {
                format!(
                    "{flag} expects a number",
                    flag = "--replay-max-settle-seconds"
                )
            })
        })
        .transpose()?
        .unwrap_or(60.0);
    if !max_settle_seconds.is_finite() || max_settle_seconds <= 0.0 {
        return Err("--replay-max-settle-seconds must be finite and greater than zero".into());
    }
    let idle_frames = value_after("--replay-idle-frames")
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| "--replay-idle-frames expects an integer".to_string())
        })
        .transpose()?
        .unwrap_or(30);
    if idle_frames == 0 {
        return Err("--replay-idle-frames must be greater than zero".into());
    }
    let surface_size = value_after("--replay-size")
        .map(|value| {
            let (width, height) = value
                .split_once('x')
                .ok_or_else(|| "--replay-size expects WIDTHxHEIGHT".to_string())?;
            let width = width
                .parse::<u32>()
                .map_err(|_| "--replay-size expects WIDTHxHEIGHT".to_string())?;
            let height = height
                .parse::<u32>()
                .map_err(|_| "--replay-size expects WIDTHxHEIGHT".to_string())?;
            if width == 0 || height == 0 {
                return Err("--replay-size dimensions must be greater than zero".to_string());
            }
            Ok((width, height))
        })
        .transpose()?;
    let measurement_range = if measurement_flag_count > 0 {
        let value = value_after("--replay-measure-frames")
            .ok_or_else(|| "--replay-measure-frames expects START:END".to_string())?;
        Some(vearth_app::replay_measurement::MeasurementRange::parse(
            value,
        )?)
    } else {
        None
    };

    Ok(Some(vearth_app::app::ReplayBenchmarkOptions {
        report_path: report_path.into(),
        trace_path: trace_path.to_string(),
        max_settle: vearth_platform::time::Duration::from_secs_f64(max_settle_seconds),
        idle_frames,
        surface_size,
        measurement_range,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landmark_calibrator_defaults_to_first_registry_entry() {
        let argv = vec!["vearth-app".into(), "--calibrate-landmark".into()];
        let selected = parse_calibration_landmark(&argv).unwrap();
        assert_eq!(selected.name, vearth_gen::landmarks::LANDMARKS[0].name);
    }

    #[test]
    fn replay_benchmark_args_are_opt_in_and_validated() {
        let plain = vec!["vearth-app".into(), "--replay".into(), "trace.csv".into()];
        assert!(
            parse_replay_benchmark_args(&plain, "trace.csv")
                .unwrap()
                .is_none()
        );

        let benchmark = vec![
            "vearth-app".into(),
            "--replay".into(),
            "trace.csv".into(),
            "--replay-report".into(),
            "report.json".into(),
            "--replay-max-settle-seconds".into(),
            "12.5".into(),
            "--replay-idle-frames".into(),
            "120".into(),
            "--replay-size".into(),
            "1920x1080".into(),
        ];
        let parsed = parse_replay_benchmark_args(&benchmark, "trace.csv")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.report_path, std::path::PathBuf::from("report.json"));
        assert_eq!(parsed.trace_path, "trace.csv");
        assert_eq!(
            parsed.max_settle,
            vearth_platform::time::Duration::from_secs_f64(12.5)
        );
        assert_eq!(parsed.idle_frames, 120);
        assert_eq!(parsed.surface_size, Some((1920, 1080)));
        assert_eq!(parsed.measurement_range, None);

        let invalid = vec![
            "vearth-app".into(),
            "--replay-report".into(),
            "report.json".into(),
            "--replay-idle-frames".into(),
            "0".into(),
        ];
        assert!(parse_replay_benchmark_args(&invalid, "trace.csv").is_err());
    }

    #[test]
    fn replay_measurement_args_require_a_report_and_one_valid_frame_window() {
        let valid = [
            "vearth-app",
            "--replay-report",
            "report.json",
            "--replay-measure-frames",
            "1801:2280",
        ]
        .map(str::to_owned);
        let options = parse_replay_benchmark_args(&valid, "trace.csv")
            .unwrap()
            .unwrap();
        let range = options.measurement_range.unwrap();
        assert!(range.validate_trace(2280).is_ok());
        assert!(range.validate_trace(2279).is_err());
        for args in [
            vec!["--replay-measure-frames", "0:10"],
            vec!["--replay-report", "--replay-measure-frames", "0:10"],
            vec!["--replay-report", "report.json", "--replay-measure-frames"],
            vec![
                "--replay-report",
                "report.json",
                "--replay-measure-frames",
                "10:0",
            ],
            vec![
                "--replay-report",
                "report.json",
                "--replay-measure-frames",
                "0:4096",
            ],
            vec![
                "--replay-report",
                "report.json",
                "--replay-measure-frames",
                "0:10",
                "--replay-measure-frames",
                "1:10",
            ],
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
            assert!(
                parse_replay_benchmark_args(&args, "trace.csv").is_err(),
                "{args:?}"
            );
        }
    }

    #[test]
    fn capture_trace_sampling_includes_both_ends_and_even_interior_frames() {
        assert_eq!(capture_trace_frames(600, 5), vec![0, 150, 300, 450, 600]);
        assert_eq!(capture_trace_frames(10, 4), vec![0, 3, 7, 10]);
    }

    #[test]
    fn play_screenshot_pose_defaults_and_overrides_are_explicit() {
        let mut args = vearth_app::screenshot::Args::default();
        parse_play_pose_args(&[], &mut args).unwrap();
        assert_eq!(args.play_heading_offset_deg, 0.0);
        assert!((args.play_pitch_deg.to_radians() - 0.35).abs() < 1.0e-12);
        assert_eq!(args.play_bank_deg, 0.0);
        assert_eq!(args.play_distance_m, None);
        assert_eq!(args.play_throttle, 0.65);
        assert_eq!(args.play_boost, 0.25);

        let argv = [
            "--play-heading-offset",
            "-45",
            "--play-pitch",
            "5",
            "--play-bank",
            "-18",
            "--play-distance",
            "12.5",
            "--play-throttle",
            "0.8",
            "--play-boost",
            "1",
        ]
        .map(str::to_owned);
        parse_play_pose_args(&argv, &mut args).unwrap();
        assert_eq!(args.play_heading_offset_deg, -45.0);
        assert_eq!(args.play_pitch_deg, 5.0);
        assert_eq!(args.play_bank_deg, -18.0);
        assert_eq!(args.play_distance_m, Some(12.5));
        assert_eq!(args.play_throttle, 0.8);
        assert_eq!(args.play_boost, 1.0);
    }

    #[test]
    fn play_screenshot_pose_rejects_invalid_or_missing_numbers() {
        for flag in [
            "--play-heading-offset",
            "--play-pitch",
            "--play-bank",
            "--play-distance",
            "--play-throttle",
            "--play-boost",
        ] {
            for value in [None, Some("NaN"), Some("inf"), Some("-inf"), Some("--play")] {
                let mut argv = vec![flag.to_owned()];
                argv.extend(value.map(str::to_owned));
                let error =
                    parse_play_pose_args(&argv, &mut vearth_app::screenshot::Args::default())
                        .unwrap_err();
                assert!(error.starts_with(flag), "{error}");
            }
        }
        for (flag, values) in [
            ("--play-pitch", ["-90.1", "90.1"]),
            ("--play-bank", ["-90.1", "90.1"]),
            ("--play-distance", ["0", "-1"]),
            ("--play-throttle", ["-0.1", "1.1"]),
            ("--play-boost", ["-0.1", "1.1"]),
        ] {
            for value in values {
                let argv = [flag, value].map(str::to_owned);
                assert!(
                    parse_play_pose_args(&argv, &mut vearth_app::screenshot::Args::default())
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn fly_to_capture_uses_the_navigation_curve_and_holds_the_destination() {
        let base = vearth_app::screenshot::Args {
            lat_deg: -5.0,
            lon_deg: -95.0,
            alt_m: 9_000_000.0,
            heading_deg: 25.0,
            pitch_deg: 0.0,
            ..Default::default()
        };
        let poses = capture_fly_to_poses(
            &base,
            (-10.268, -76.905, 5_200.0),
            61,
            10,
            Some((190.0, 65.0)),
        )
        .unwrap();
        assert_eq!(poses.len(), 61);
        assert!((poses[0].lat_deg - base.lat_deg).abs() < 1.0e-9);
        assert!((poses[50].lat_deg + 10.268).abs() < 1.0e-6);
        assert!((poses[50].lon_deg + 76.905).abs() < 1.0e-6);
        assert!((poses[50].alt_m - 5_200.0).abs() < 1.0e-6);
        assert!((poses[50].heading_deg - 190.0).abs() < 1.0e-6);
        assert!((poses[50].pitch_deg - 65.0).abs() < 1.0e-6);
        assert!(poses[51..].iter().all(|pose| {
            pose.lat_deg == poses[50].lat_deg
                && pose.lon_deg == poses[50].lon_deg
                && pose.alt_m == poses[50].alt_m
        }));

        let too_high = vearth_app::screenshot::Args {
            alt_m: 16_000_000.0,
            ..base
        };
        assert!(capture_fly_to_poses(&too_high, (0.0, 0.0, 1_000.0), 30, 0, None).is_err());
    }

    #[test]
    fn chase_capture_rejects_a_path_opposed_to_the_vehicle_heading() {
        let aligned = vearth_app::replay::ReplayTrace::parse(
            "0,-33.91,151.27,3000,315,50\n600,-33.86,151.21,1200,315,60",
        )
        .unwrap();
        assert!(validate_chase_bearings(&aligned, &[0, 300, 600]).is_ok());

        let reversed = vearth_app::replay::ReplayTrace::parse(
            "0,36.01,-112.16,4000,300,50\n600,36.06,-112.10,2000,300,60",
        )
        .unwrap();
        assert!(validate_chase_bearings(&reversed, &[0, 300, 600]).is_err());
    }

    #[test]
    fn capture_path_resamples_at_smooth_pace_and_aims_down_route() {
        let waypoints = [
            CaptureWaypoint {
                lat_deg: -29.6300,
                lon_deg: 29.4100,
                alt_m: 1_600.0,
            },
            CaptureWaypoint {
                lat_deg: -29.6290,
                lon_deg: 29.4090,
                alt_m: 1_620.0,
            },
            CaptureWaypoint {
                lat_deg: -29.6280,
                lon_deg: 29.4070,
                alt_m: 1_650.0,
            },
        ];
        let poses = capture_path_poses(&waypoints, 31, 5, 84.0).unwrap();
        assert_eq!(poses.len(), 31);
        assert_eq!(poses[0].lat_deg, waypoints[0].lat_deg);
        assert!((poses[25].lat_deg - waypoints[2].lat_deg).abs() < 1.0e-9);
        assert!((poses[25].lon_deg - waypoints[2].lon_deg).abs() < 1.0e-9);
        assert!(poses[..26].iter().all(|pose| pose.heading_deg.is_finite()));
        assert!(
            poses[..26]
                .iter()
                .all(|pose| (pose.pitch_deg - 84.0).abs() < 1.0e-9)
        );
        assert!(poses[26..].iter().all(|pose| {
            pose.lat_deg == poses[25].lat_deg && pose.lon_deg == poses[25].lon_deg
        }));
    }
}

/// The camera-position subset of the CLI flags, shared by windowed and
/// screenshot modes. `--at lat,lon[,alt[,heading[,pitch]]]` is the deep-link
/// form (what "copy location link" produces); individual flags override it.
fn parse_position_args() -> vearth_app::screenshot::Args {
    let argv: Vec<String> = std::env::args().collect();
    let mut args = vearth_app::screenshot::Args::default();
    if let Some(spec) = argv
        .iter()
        .position(|a| a == "--at")
        .and_then(|i| argv.get(i + 1))
    {
        let parts: Vec<f64> = spec
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if parts.len() >= 2 {
            args.lat_deg = parts[0];
            args.lon_deg = parts[1];
            args.alt_m = parts.get(2).copied().unwrap_or(2_000.0);
            args.heading_deg = parts.get(3).copied().unwrap_or(0.0);
            args.pitch_deg = parts.get(4).copied().unwrap_or(0.0);
        } else {
            eprintln!("--at expects lat,lon[,alt[,heading[,pitch]]]");
        }
    }
    let flag = |name: &str| -> Option<f64> {
        argv.iter()
            .position(|a| a == name)
            .and_then(|i| argv.get(i + 1))
            .and_then(|v| v.parse().ok())
    };
    if let Some(v) = flag("--lat") {
        args.lat_deg = v;
    }
    if let Some(v) = flag("--lon") {
        args.lon_deg = v;
    }
    if let Some(v) = flag("--alt") {
        args.alt_m = v;
    }
    if let Some(v) = flag("--heading") {
        args.heading_deg = v;
    }
    if let Some(v) = flag("--pitch") {
        args.pitch_deg = v;
    }
    if let Some(v) = flag("--timeout") {
        args.timeout_s = v;
    }
    if let Some(v) = flag("--time-scale") {
        args.time_scale = Some(v);
    }
    if let Some(value) = argv
        .iter()
        .position(|a| a == "--surface-month")
        .and_then(|i| argv.get(i + 1))
    {
        args.surface_month = Some(
            value
                .parse::<u8>()
                .ok()
                .filter(|month| (1..=12).contains(month))
                .unwrap_or_else(|| {
                    eprintln!("--surface-month expects an integer from 1 through 12");
                    std::process::exit(2);
                }),
        );
    }
    if let Some(value) = argv
        .iter()
        .position(|a| a == "--time")
        .and_then(|i| argv.get(i + 1))
    {
        args.solar_start = if value.eq_ignore_ascii_case("live") {
            vearth_app::solar::SolarStart::Live
        } else {
            match vearth_app::solar::parse_utc_datetime(value) {
                Ok(seconds) => vearth_app::solar::SolarStart::At(seconds),
                Err(error) => {
                    eprintln!("invalid --time {value:?}: {error}");
                    std::process::exit(2);
                }
            }
        };
    }
    args.play = argv.iter().any(|a| a == "--play");
    if let Some(value) = argv
        .iter()
        .position(|a| a == "--play-vehicle")
        .and_then(|i| argv.get(i + 1))
    {
        args.play_vehicle = vearth_app::vehicles::VehicleKind::parse(value).unwrap_or_else(|| {
            eprintln!("--play-vehicle expects kestrel|phase-manta|sparrow");
            std::process::exit(2);
        });
    }
    if let Some(value) = argv
        .iter()
        .position(|a| a == "--weather")
        .and_then(|i| argv.get(i + 1))
    {
        match vearth_app::weather::WeatherConfig::parse_mode(value) {
            Ok((mode, preset)) => {
                args.weather.mode = mode;
                args.weather.preset = preset;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        }
    }
    if let Some(value) = argv
        .iter()
        .position(|a| a == "--weather-seed")
        .and_then(|i| argv.get(i + 1))
    {
        args.weather.seed = value.parse().unwrap_or_else(|_| {
            eprintln!("--weather-seed expects an unsigned 64-bit integer");
            std::process::exit(2);
        });
    }
    if let Some(value) = argv
        .iter()
        .position(|a| a == "--weather-quality")
        .and_then(|i| argv.get(i + 1))
    {
        args.weather.quality = value.parse().unwrap_or_else(|error: String| {
            eprintln!("{error}");
            std::process::exit(2);
        });
    }
    if let Some(value) = argv
        .iter()
        .position(|a| a == "--weather-override")
        .and_then(|i| argv.get(i + 1))
    {
        args.weather.overrides = value.parse().unwrap_or_else(|error: String| {
            eprintln!("{error}");
            std::process::exit(2);
        });
    }
    if let Some(value) = argv
        .iter()
        .position(|a| a == "--wind-influence")
        .and_then(|i| argv.get(i + 1))
    {
        args.weather.wind_influence = value.parse().unwrap_or_else(|error: String| {
            eprintln!("{error}");
            std::process::exit(2);
        });
    }
    args.weather.precipitation_camera_effects = argv
        .iter()
        .any(|argument| argument == "--precipitation-camera-effects");
    args.post_process.color_grading = !argv.iter().any(|argument| argument == "--no-color-grading");
    args.post_process.filmic_saturation = !argv
        .iter()
        .any(|argument| argument == "--no-filmic-saturation");
    args
}

fn parse_play_pose_args(
    argv: &[String],
    args: &mut vearth_app::screenshot::Args,
) -> Result<(), String> {
    let number = |flag: &str, valid: fn(f64) -> bool, expected: &str| {
        let Some(index) = argv.iter().position(|argument| argument == flag) else {
            return Ok(None);
        };
        let value = argv
            .get(index + 1)
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && valid(*value))
            .ok_or_else(|| format!("{flag} expects {expected}"))?;
        Ok::<_, String>(Some(value))
    };
    if let Some(value) = number("--play-heading-offset", |_| true, "finite degrees")? {
        args.play_heading_offset_deg = value % 360.0;
    }
    for (flag, target) in [
        ("--play-pitch", &mut args.play_pitch_deg),
        ("--play-bank", &mut args.play_bank_deg),
    ] {
        if let Some(value) = number(
            flag,
            |value| (-90.0..=90.0).contains(&value),
            "finite degrees from -90 through 90",
        )? {
            *target = value;
        }
    }
    if let Some(value) = number(
        "--play-distance",
        |value| value > 0.0,
        "a finite distance greater than zero in metres",
    )? {
        args.play_distance_m = Some(value);
    }
    for (flag, target) in [
        ("--play-throttle", &mut args.play_throttle),
        ("--play-boost", &mut args.play_boost),
    ] {
        if let Some(value) = number(
            flag,
            |value| (0.0..=1.0).contains(&value),
            "a finite fraction from 0 through 1",
        )? {
            *target = value as f32;
        }
    }
    Ok(())
}

fn parse_screenshot_args() -> Option<vearth_app::screenshot::Args> {
    let argv: Vec<String> = std::env::args().collect();
    let pos = argv.iter().position(|a| a == "--screenshot")?;
    let mut args = parse_position_args();
    if let Err(error) = parse_play_pose_args(&argv, &mut args) {
        eprintln!("{error}");
        std::process::exit(2);
    }
    if let Some(path) = argv.get(pos + 1) {
        args.out = path.clone();
    }
    args.report_path = argv
        .iter()
        .position(|argument| argument == "--screenshot-report")
        .and_then(|index| argv.get(index + 1))
        .cloned();
    args.fresh_chunks = argv.iter().any(|argument| argument == "--fresh-chunks");
    let explicit_size = argv.iter().position(|a| a == "--size");
    if let Some(i) = explicit_size
        && let Some((w, h)) = argv.get(i + 1).and_then(|s| s.split_once('x'))
        && let (Ok(w), Ok(h)) = (w.parse(), h.parse())
    {
        args.width = w;
        args.height = h;
    }
    if let Some(index) = argv.iter().position(|a| a == "--photo-quality") {
        if explicit_size.is_some() {
            eprintln!("--photo-quality cannot be combined with --size");
            std::process::exit(2);
        }
        let quality = argv
            .get(index + 1)
            .ok_or_else(|| "--photo-quality expects 1024|2k|4k|8k".to_string())
            .and_then(|value| vearth_app::capture::PhotoResolution::parse(value))
            .unwrap_or_else(|error| {
                eprintln!("{error}");
                std::process::exit(2);
            });
        let aspect = argv
            .iter()
            .position(|a| a == "--photo-aspect")
            .and_then(|index| argv.get(index + 1))
            .map(|value| vearth_app::capture::PhotoAspect::parse(value))
            .transpose()
            .unwrap_or_else(|error| {
                eprintln!("{error}");
                std::process::exit(2);
            })
            .unwrap_or(vearth_app::capture::PhotoAspect::Landscape16x9);
        let settings = vearth_app::capture::PhotoSettings {
            resolution: quality,
            aspect,
            ..Default::default()
        };
        (args.width, args.height) = settings.dimensions(args.width, args.height);
    }
    args.stage = match vearth_app::staging::parse_options(&argv) {
        Ok(stage) => stage,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    Some(args)
}

#[derive(Clone)]
enum CaptureSource {
    Trace(std::path::PathBuf),
    Path {
        waypoints: std::path::PathBuf,
        hold_frames: u32,
    },
    FlyTo {
        lat_deg: f64,
        lon_deg: f64,
        alt_m: f64,
        hold_frames: u32,
        end_framing: Option<(f64, f64)>,
    },
}

#[derive(Clone)]
struct CaptureTraceArgs {
    source: CaptureSource,
    output_dir: std::path::PathBuf,
    output_frames: u32,
    time_span_seconds: f64,
    ground_clearance_m: Option<f64>,
    /// Renders per output frame after the first; more samples let temporal
    /// cloud history settle at each pose instead of dithering in motion.
    samples_per_frame: usize,
    base: vearth_app::screenshot::Args,
}

/// Deterministic, windowless marketing/visual-regression capture. Camera poses
/// come from a replay CSV or the exact smooth `FlyTo` path used by interactive
/// navigation. Every output frame waits for complete world and tree residency.
fn parse_capture_trace_args(argv: &[String]) -> Option<Result<CaptureTraceArgs, String>> {
    let trace_index = argv
        .iter()
        .position(|argument| argument == "--capture-trace");
    let fly_to_index = argv
        .iter()
        .position(|argument| argument == "--capture-fly-to");
    let path_index = argv
        .iter()
        .position(|argument| argument == "--capture-path");
    if trace_index.is_none() && fly_to_index.is_none() && path_index.is_none() {
        return None;
    }
    let parse = || {
        if usize::from(trace_index.is_some())
            + usize::from(fly_to_index.is_some())
            + usize::from(path_index.is_some())
            != 1
        {
            return Err(
                "use exactly one of --capture-trace, --capture-path, or --capture-fly-to".into(),
            );
        }
        let value_after = |flag: &str| {
            argv.iter()
                .position(|argument| argument == flag)
                .and_then(|position| argv.get(position + 1))
        };
        let output_dir = value_after("--capture-dir")
            .ok_or_else(|| "cinematic capture requires --capture-dir DIR".to_string())?;
        let output_frames = value_after("--capture-frames")
            .ok_or_else(|| "cinematic capture requires --capture-frames N".to_string())?
            .parse::<u32>()
            .map_err(|_| "--capture-frames expects an integer".to_string())?;
        if output_frames < 2 {
            return Err("--capture-frames must be at least 2".into());
        }
        let hold_frames = value_after("--capture-hold-frames")
            .map(|value| {
                value
                    .parse::<u32>()
                    .map_err(|_| "--capture-hold-frames expects an integer".to_string())
            })
            .transpose()?
            .unwrap_or(0);
        if hold_frames + 2 > output_frames {
            return Err("--capture-hold-frames must leave at least two moving frames".into());
        }
        let source = if let Some(index) = trace_index {
            let trace_path = argv
                .get(index + 1)
                .filter(|value| !value.starts_with('-'))
                .ok_or_else(|| "--capture-trace expects a replay CSV path".to_string())?;
            if hold_frames != 0 {
                return Err(
                    "--capture-hold-frames is only valid with --capture-path or --capture-fly-to"
                        .into(),
                );
            }
            CaptureSource::Trace(trace_path.into())
        } else if let Some(index) = path_index {
            let path = argv
                .get(index + 1)
                .filter(|value| !value.starts_with('-'))
                .ok_or_else(|| "--capture-path expects a waypoint CSV path".to_string())?;
            CaptureSource::Path {
                waypoints: path.into(),
                hold_frames,
            }
        } else {
            let value = argv
                .get(fly_to_index.expect("fly-to index checked") + 1)
                .ok_or_else(|| "--capture-fly-to expects LAT,LON,ALT destination".to_string())?;
            let parts: Vec<_> = value.split(',').map(str::trim).collect();
            if parts.len() != 3 {
                return Err("--capture-fly-to expects LAT,LON,ALT destination".into());
            }
            let parse_part = |index: usize, name: &str| {
                parts[index]
                    .parse::<f64>()
                    .map_err(|_| format!("--capture-fly-to has an invalid {name}"))
            };
            let lat_deg = parse_part(0, "latitude")?;
            let lon_deg = parse_part(1, "longitude")?;
            let alt_m = parse_part(2, "altitude")?;
            if !(-90.0..=90.0).contains(&lat_deg)
                || !lon_deg.is_finite()
                || !alt_m.is_finite()
                || alt_m < 0.0
            {
                return Err("--capture-fly-to destination is outside the valid range".into());
            }
            let end_heading = value_after("--capture-end-heading")
                .map(|value| {
                    value
                        .parse::<f64>()
                        .map_err(|_| "--capture-end-heading expects a number".to_string())
                })
                .transpose()?;
            let end_pitch = value_after("--capture-end-pitch")
                .map(|value| {
                    value
                        .parse::<f64>()
                        .map_err(|_| "--capture-end-pitch expects a number".to_string())
                })
                .transpose()?;
            let end_framing = match (end_heading, end_pitch) {
                (None, None) => None,
                (Some(heading), Some(pitch))
                    if heading.is_finite() && (0.0..=89.9).contains(&pitch) =>
                {
                    Some((heading, pitch))
                }
                (Some(_), Some(_)) => {
                    return Err(
                        "capture end heading must be finite and pitch must be in [0, 89.9]".into(),
                    );
                }
                _ => {
                    return Err("use --capture-end-heading and --capture-end-pitch together".into());
                }
            };
            CaptureSource::FlyTo {
                lat_deg,
                lon_deg,
                alt_m,
                hold_frames,
                end_framing,
            }
        };
        let time_span_seconds = value_after("--capture-time-span-seconds")
            .map(|value| {
                value
                    .parse::<f64>()
                    .map_err(|_| "--capture-time-span-seconds expects a finite number".to_string())
            })
            .transpose()?
            .unwrap_or(0.0);
        if !time_span_seconds.is_finite() {
            return Err("--capture-time-span-seconds expects a finite number".into());
        }
        let ground_clearance_m = value_after("--capture-ground-clearance")
            .map(|value| {
                value
                    .parse::<f64>()
                    .map_err(|_| "--capture-ground-clearance expects a positive number".to_string())
            })
            .transpose()?;
        if ground_clearance_m.is_some_and(|clearance| !clearance.is_finite() || clearance <= 0.0) {
            return Err("--capture-ground-clearance expects a positive number".into());
        }

        let samples_per_frame = value_after("--capture-samples")
            .map(|value| {
                value
                    .parse::<usize>()
                    .ok()
                    .filter(|samples| (1..=64).contains(samples))
                    .ok_or_else(|| {
                        "--capture-samples expects an integer from 1 through 64".to_string()
                    })
            })
            .transpose()?
            .unwrap_or(1);

        let mut base = parse_position_args();
        if let Some(size) = value_after("--size") {
            let (width, height) = size
                .split_once('x')
                .ok_or_else(|| "--size expects WIDTHxHEIGHT".to_string())?;
            base.width = width
                .parse::<u32>()
                .map_err(|_| "--size expects WIDTHxHEIGHT".to_string())?;
            base.height = height
                .parse::<u32>()
                .map_err(|_| "--size expects WIDTHxHEIGHT".to_string())?;
            if base.width == 0 || base.height == 0 {
                return Err("--size dimensions must be greater than zero".into());
            }
        }
        if time_span_seconds != 0.0
            && !matches!(base.solar_start, vearth_app::solar::SolarStart::At(_))
        {
            return Err(
                "--capture-time-span-seconds requires an explicit --time UTC instant".into(),
            );
        }
        base.stage = None;
        Ok(CaptureTraceArgs {
            source,
            output_dir: output_dir.into(),
            output_frames,
            time_span_seconds,
            ground_clearance_m,
            samples_per_frame,
            base,
        })
    };
    Some(parse())
}

fn capture_fly_to_poses(
    base: &vearth_app::screenshot::Args,
    destination: (f64, f64, f64),
    output_frames: u32,
    hold_frames: u32,
    end_framing: Option<(f64, f64)>,
) -> Result<Vec<vearth_app::replay::ReplayPose>, String> {
    let mut camera = vearth_app::camera::Camera {
        lat: base.lat_deg.to_radians(),
        lon: base.lon_deg.to_radians(),
        alt_m: base.alt_m,
        heading: base.heading_deg.to_radians(),
        pitch: base.pitch_deg.to_radians(),
        fov_y: 55f64.to_radians(),
        aspect: base.width as f64 / base.height.max(1) as f64,
    };
    let max_alt_m = camera.max_zoom_out_alt();
    if camera.alt_m > max_alt_m {
        return Err(format!(
            "capture start altitude {:.0}m exceeds the {:.0}m navigation zoom limit at {}x{}; lower --alt so the first frame cannot jump",
            camera.alt_m, max_alt_m, base.width, base.height,
        ));
    }
    let mut fly =
        vearth_app::camera::FlyTo::new(&camera, destination.0, destination.1, destination.2);
    let moving_frames = output_frames - hold_frames;
    let dt = fly.duration_seconds() / f64::from(moving_frames - 1);
    let mut poses = Vec::with_capacity(output_frames as usize);
    for frame in 0..moving_frames {
        if frame > 0 {
            fly.update(&mut camera, dt);
        }
        if let Some((end_heading_deg, end_pitch_deg)) = end_framing {
            let t = f64::from(frame) / f64::from(moving_frames - 1);
            let eased = t * t * (3.0 - 2.0 * t);
            let mut heading_delta = (end_heading_deg - base.heading_deg) % 360.0;
            if heading_delta > 180.0 {
                heading_delta -= 360.0;
            } else if heading_delta < -180.0 {
                heading_delta += 360.0;
            }
            camera.heading = (base.heading_deg + heading_delta * eased).to_radians();
            camera.pitch = (base.pitch_deg + (end_pitch_deg - base.pitch_deg) * eased).to_radians();
        }
        poses.push(vearth_app::replay::ReplayPose {
            frame: u64::from(frame),
            lat_deg: camera.lat.to_degrees(),
            lon_deg: camera.lon.to_degrees(),
            alt_m: camera.alt_m,
            heading_deg: camera.heading.to_degrees(),
            pitch_deg: camera.pitch.to_degrees(),
        });
    }
    let final_pose = *poses.last().expect("a fly-to has at least two poses");
    for frame in moving_frames..output_frames {
        poses.push(vearth_app::replay::ReplayPose {
            frame: u64::from(frame),
            ..final_pose
        });
    }
    Ok(poses)
}

#[derive(Clone, Copy, Debug)]
struct CaptureWaypoint {
    lat_deg: f64,
    lon_deg: f64,
    alt_m: f64,
}

fn load_capture_waypoints(path: &std::path::Path) -> Result<Vec<CaptureWaypoint>, String> {
    let csv = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read capture path {}: {error}", path.display()))?;
    let mut waypoints = Vec::new();
    for (line_index, raw) in csv.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
        if fields
            .first()
            .is_some_and(|field| field.eq_ignore_ascii_case("lat_deg"))
        {
            continue;
        }
        if fields.len() != 3 {
            return Err(format!(
                "capture path {} line {}: expected lat_deg,lon_deg,alt_m",
                path.display(),
                line_index + 1,
            ));
        }
        let parse = |index: usize, name: &str| {
            fields[index].parse::<f64>().map_err(|_| {
                format!(
                    "capture path {} line {}: invalid {name}",
                    path.display(),
                    line_index + 1,
                )
            })
        };
        let waypoint = CaptureWaypoint {
            lat_deg: parse(0, "latitude")?,
            lon_deg: parse(1, "longitude")?,
            alt_m: parse(2, "altitude")?,
        };
        if !(-90.0..=90.0).contains(&waypoint.lat_deg)
            || !waypoint.lon_deg.is_finite()
            || !waypoint.alt_m.is_finite()
            || waypoint.alt_m < 0.0
        {
            return Err(format!(
                "capture path {} line {}: waypoint is outside the valid range",
                path.display(),
                line_index + 1,
            ));
        }
        waypoints.push(waypoint);
    }
    if waypoints.len() < 2 {
        return Err(format!(
            "capture path {} requires at least two waypoints",
            path.display()
        ));
    }
    Ok(waypoints)
}

fn waypoint_distance_m(from: CaptureWaypoint, to: CaptureWaypoint) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let dlat = (to.lat_deg - from.lat_deg).to_radians();
    let dlon = wrapped_degrees(to.lon_deg - from.lon_deg).to_radians();
    let mean_lat = ((from.lat_deg + to.lat_deg) * 0.5).to_radians();
    let horizontal = EARTH_RADIUS_M * (dlat * dlat + (dlon * mean_lat.cos()).powi(2)).sqrt();
    horizontal.hypot(to.alt_m - from.alt_m)
}

/// Resample a checked-in waypoint path at a smooth, frame-deterministic pace.
/// Positions remain on the source polyline, while the chase heading looks
/// across a short frame window so road corners and aircraft turns do not snap.
fn capture_path_poses(
    waypoints: &[CaptureWaypoint],
    output_frames: u32,
    hold_frames: u32,
    pitch_deg: f64,
) -> Result<Vec<vearth_app::replay::ReplayPose>, String> {
    if !(0.0..=89.9).contains(&pitch_deg) {
        return Err("capture path pitch must be in [0, 89.9]".into());
    }
    let segment_lengths = waypoints
        .windows(2)
        .map(|pair| waypoint_distance_m(pair[0], pair[1]))
        .collect::<Vec<_>>();
    let total_distance = segment_lengths.iter().sum::<f64>();
    if !total_distance.is_finite() || total_distance < 1.0 {
        return Err("capture path must travel at least one metre".into());
    }
    let moving_frames = output_frames - hold_frames;
    let mut poses = Vec::with_capacity(output_frames as usize);
    for frame in 0..moving_frames {
        let t = f64::from(frame) / f64::from(moving_frames - 1);
        let eased = t * t * (3.0 - 2.0 * t);
        let target_distance = total_distance * eased;
        let mut traversed = 0.0;
        let mut segment = segment_lengths.len() - 1;
        for (index, length) in segment_lengths.iter().enumerate() {
            if target_distance <= traversed + length || index + 1 == segment_lengths.len() {
                segment = index;
                break;
            }
            traversed += length;
        }
        let length = segment_lengths[segment];
        let alpha = if length > 0.0 {
            ((target_distance - traversed) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let from = waypoints[segment];
        let to = waypoints[segment + 1];
        let lon_delta = wrapped_degrees(to.lon_deg - from.lon_deg);
        poses.push(vearth_app::replay::ReplayPose {
            frame: u64::from(frame),
            lat_deg: from.lat_deg + (to.lat_deg - from.lat_deg) * alpha,
            lon_deg: from.lon_deg + lon_delta * alpha,
            alt_m: from.alt_m + (to.alt_m - from.alt_m) * alpha,
            heading_deg: 0.0,
            pitch_deg,
        });
    }
    for index in 0..poses.len() {
        let from = poses[index.saturating_sub(3)];
        let to = poses[(index + 3).min(poses.len() - 1)];
        poses[index].heading_deg = course_degrees(from, to);
    }
    let final_pose = *poses.last().expect("a capture path has moving poses");
    for frame in moving_frames..output_frames {
        poses.push(vearth_app::replay::ReplayPose {
            frame: u64::from(frame),
            ..final_pose
        });
    }
    Ok(poses)
}

fn capture_trace_frames(last_trace_frame: u64, output_frames: u32) -> Vec<u64> {
    let denominator = u64::from(output_frames - 1);
    (0..output_frames)
        .map(|index| (u64::from(index) * last_trace_frame + denominator / 2) / denominator)
        .collect()
}

fn wrapped_degrees(value: f64) -> f64 {
    (value + 180.0).rem_euclid(360.0) - 180.0
}

fn course_degrees(from: vearth_app::replay::ReplayPose, to: vearth_app::replay::ReplayPose) -> f64 {
    let lat1 = from.lat_deg.to_radians();
    let lat2 = to.lat_deg.to_radians();
    let delta_lon = (to.lon_deg - from.lon_deg).to_radians();
    let y = delta_lon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * delta_lon.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

fn validate_chase_bearings(
    trace: &vearth_app::replay::ReplayTrace,
    trace_frames: &[u64],
) -> Result<(), String> {
    let mut moving_segments = 0usize;
    for pair in trace_frames.windows(2) {
        let from = trace.pose_at(pair[0]);
        let to = trace.pose_at(pair[1]);
        if (to.lat_deg - from.lat_deg).abs() + (to.lon_deg - from.lon_deg).abs() < 1.0e-9 {
            continue;
        }
        moving_segments += 1;
        let course = course_degrees(from, to);
        let difference = wrapped_degrees(from.heading_deg - course).abs();
        if difference > 55.0 {
            return Err(format!(
                "--play trace points the drone at {:.1}° while the camera travels at {:.1}° near trace frame {}; keep the difference within 55° or capture without --play",
                from.heading_deg.rem_euclid(360.0),
                course,
                pair[0]
            ));
        }
    }
    if moving_segments == 0 {
        return Err("--play trace must contain horizontal camera movement".into());
    }
    Ok(())
}

fn run_capture_trace(options: CaptureTraceArgs) -> Result<(), String> {
    let poses = match &options.source {
        CaptureSource::Trace(trace_path) => {
            let trace = vearth_app::replay::ReplayTrace::load(trace_path)?;
            if trace.last_frame() == 0 {
                return Err("capture trace must contain motion beyond frame zero".into());
            }
            if u64::from(options.output_frames) > trace.last_frame() + 1 {
                return Err(format!(
                    "--capture-frames {} exceeds the trace's {} unique frame positions",
                    options.output_frames,
                    trace.last_frame() + 1
                ));
            }
            let trace_frames = capture_trace_frames(trace.last_frame(), options.output_frames);
            if options.base.play {
                validate_chase_bearings(&trace, &trace_frames)?;
            }
            trace_frames
                .into_iter()
                .map(|frame| trace.pose_at(frame))
                .collect::<Vec<_>>()
        }
        CaptureSource::Path {
            waypoints,
            hold_frames,
        } => {
            let waypoints = load_capture_waypoints(waypoints)?;
            capture_path_poses(
                &waypoints,
                options.output_frames,
                *hold_frames,
                options.base.pitch_deg,
            )?
        }
        CaptureSource::FlyTo {
            lat_deg,
            lon_deg,
            alt_m,
            hold_frames,
            end_framing,
        } => capture_fly_to_poses(
            &options.base,
            (*lat_deg, *lon_deg, *alt_m),
            options.output_frames,
            *hold_frames,
            *end_framing,
        )?,
    };
    std::fs::create_dir_all(&options.output_dir).map_err(|error| {
        format!(
            "could not create capture directory {}: {error}",
            options.output_dir.display()
        )
    })?;
    let solar =
        vearth_app::solar::SolarClock::new(options.base.solar_start, options.base.time_scale);
    let surface_month = options
        .base
        .surface_month
        .unwrap_or_else(|| vearth_app::solar::utc_month(solar.utc_seconds()));
    let mut session =
        vearth_app::capture::CinematicCaptureSession::new(surface_month, options.base.post_process)
            .map_err(|error| format!("could not initialize cinematic renderer: {error}"))?;
    session.set_samples_per_frame(options.samples_per_frame);
    for (output_index, pose) in poses.into_iter().enumerate() {
        let mut args = options.base.clone();
        args.lat_deg = pose.lat_deg;
        args.lon_deg = pose.lon_deg;
        args.alt_m = pose.alt_m;
        args.heading_deg = pose.heading_deg;
        args.pitch_deg = pose.pitch_deg;
        args.out = options
            .output_dir
            .join(format!("frame-{output_index:04}.png"))
            .to_string_lossy()
            .into_owned();
        if let vearth_app::solar::SolarStart::At(start) = options.base.solar_start {
            let progress = output_index as f64 / f64::from(options.output_frames - 1);
            args.solar_start =
                vearth_app::solar::SolarStart::At(start + options.time_span_seconds * progress);
        }
        eprintln!(
            "capture {current}/{total}: ({lat:.5}, {lon:.5}, {alt:.0}m) -> {path}",
            current = output_index + 1,
            total = options.output_frames,
            lat = pose.lat_deg,
            lon = pose.lon_deg,
            alt = pose.alt_m,
            path = args.out,
        );
        session
            .render_frame(&args, options.ground_clearance_m)
            .map_err(|error| format!("frame {output_index} failed: {error}"))?;
    }
    Ok(())
}

fn parse_search_benchmark_args(
    argv: &[String],
) -> Result<
    Option<(
        vearth_app::replay::SearchBenchmarkSpec,
        vearth_app::app::ReplayBenchmarkOptions,
    )>,
    String,
> {
    let count = argv
        .iter()
        .filter(|arg| *arg == "--benchmark-search")
        .count();
    if count == 0 {
        if argv.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "--benchmark-search-result"
                    | "--benchmark-search-frame"
                    | "--benchmark-drone-flight"
                    | "--benchmark-city-tour"
            )
        }) {
            return Err("search benchmark options require --benchmark-search QUERY".into());
        }
        return Ok(None);
    }
    for flag in [
        "--benchmark-search",
        "--benchmark-search-result",
        "--benchmark-search-frame",
        "--benchmark-drone-flight",
        "--benchmark-city-tour",
    ] {
        if argv.iter().filter(|arg| arg.as_str() == flag).count() > 1 {
            return Err(format!("{flag} must be provided only once"));
        }
    }
    for flag in [
        "--replay",
        "--stage",
        "--calibrate-landmark",
        "--screenshot",
        "--capture-trace",
        "--capture-path",
        "--capture-fly-to",
        "--play",
        "--at",
        "--lat",
        "--lon",
        "--alt",
        "--heading",
        "--pitch",
        "--replay-measure-frames",
    ] {
        if argv.iter().any(|arg| arg == flag) {
            return Err(format!(
                "--benchmark-search cannot be combined with {flag}; it starts at the default globe and uses the normal fly-to lifecycle"
            ));
        }
    }
    let value = |flag: &str| -> Result<Option<&str>, String> {
        let Some(index) = argv.iter().position(|arg| arg == flag) else {
            return Ok(None);
        };
        argv.get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .map(|value| Some(value.as_str()))
            .ok_or_else(|| format!("{flag} requires a value"))
    };
    let query = value("--benchmark-search")?.expect("flag exists");
    let result_index = value("--benchmark-search-result")?
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| "--benchmark-search-result expects a nonnegative integer".to_string())?;
    let trigger_frame = value("--benchmark-search-frame")?
        .unwrap_or("60")
        .parse::<u64>()
        .map_err(|_| "--benchmark-search-frame expects a positive integer".to_string())?;
    if value("--replay-report")?.is_none() {
        return Err("--benchmark-search requires --replay-report PATH".into());
    }
    let label = format!("offline-search:{query}#{result_index}");
    let benchmark = parse_replay_benchmark_args(argv, &label)?
        .ok_or_else(|| "--benchmark-search requires --replay-report PATH".to_string())?;
    let mut spec =
        vearth_app::replay::SearchBenchmarkSpec::resolve(query, result_index, trigger_frame)?;
    spec.city_tour = argv.iter().any(|arg| arg == "--benchmark-city-tour");
    spec.drone_flight = spec.city_tour || argv.iter().any(|arg| arg == "--benchmark-drone-flight");
    Ok(Some((spec, benchmark)))
}

#[cfg(test)]
mod search_benchmark_arg_tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn native_search_benchmark_parses_real_result_and_existing_report_controls() {
        let argv = args(&[
            "app",
            "--benchmark-search",
            "Rotterdam",
            "--benchmark-search-frame",
            "12",
            "--replay-report",
            "out.json",
            "--replay-size",
            "1600x900",
            "--replay-max-settle-seconds",
            "90",
        ]);
        let (spec, options) = parse_search_benchmark_args(&argv).unwrap().unwrap();
        assert_eq!(
            spec.place,
            vearth_data::geocode::Geocoder::new()
                .search("Rotterdam")
                .unwrap()
                .remove(0)
        );
        assert_eq!(spec.trigger_frame, 12);
        assert_eq!(options.surface_size, Some((1600, 900)));
        assert_eq!(options.max_settle.as_secs(), 90);
        assert!(options.measurement_range.is_none());
    }

    #[test]
    fn drone_benchmark_args_require_real_search_and_keep_the_exact_result() {
        let mut argv = args(&[
            "app",
            "--benchmark-search",
            "Rotterdam",
            "--replay-report",
            "out.json",
        ]);
        let (old, _) = parse_search_benchmark_args(&argv).unwrap().unwrap();
        assert!(!old.drone_flight);
        argv.push("--benchmark-drone-flight".into());
        let (new, _) = parse_search_benchmark_args(&argv).unwrap().unwrap();
        assert!(new.drone_flight);
        assert_eq!(old.place, new.place);
        assert_eq!(old.trigger_frame, new.trigger_frame);
        argv.push("--benchmark-drone-flight".into());
        assert!(parse_search_benchmark_args(&argv).is_err());
        assert!(parse_search_benchmark_args(&args(&["app", "--benchmark-drone-flight"])).is_err());
    }

    #[test]
    fn city_tour_is_an_explicit_search_continuation() {
        let mut argv = args(&[
            "app",
            "--benchmark-search",
            "Rotterdam",
            "--replay-report",
            "out.json",
        ]);
        let (old, _) = parse_search_benchmark_args(&argv).unwrap().unwrap();
        assert!(!old.city_tour);
        argv.push("--benchmark-city-tour".into());
        let (tour, _) = parse_search_benchmark_args(&argv).unwrap().unwrap();
        assert!(tour.city_tour && tour.drone_flight);
        assert_eq!(old.place, tour.place);
        argv.push("--benchmark-city-tour".into());
        assert!(parse_search_benchmark_args(&argv).is_err());
        assert!(parse_search_benchmark_args(&args(&["app", "--benchmark-city-tour"])).is_err());
    }

    #[test]
    fn native_search_benchmark_rejects_missing_ambiguous_or_conflicting_arguments() {
        for invalid in [
            args(&["app", "--benchmark-search"]),
            args(&["app", "--benchmark-search", "Rotterdam"]),
            args(&["app", "--benchmark-search-frame", "12"]),
            args(&[
                "app",
                "--benchmark-search",
                "Rotterdam",
                "--replay-report",
                "--replay-size",
                "1600x900",
            ]),
        ] {
            assert!(parse_search_benchmark_args(&invalid).is_err());
        }
        let base = args(&[
            "app",
            "--benchmark-search",
            "Rotterdam",
            "--replay-report",
            "out.json",
        ]);
        for suffix in [
            vec!["--replay", "trace.csv"],
            vec!["--benchmark-search", "Paris"],
            vec!["--benchmark-search-result", "999"],
            vec!["--benchmark-search-frame", "0"],
            vec!["--at", "0,0,9000000"],
            vec!["--play"],
            vec!["--replay-measure-frames", "1:10"],
        ] {
            let mut argv = base.clone();
            argv.extend(suffix.into_iter().map(str::to_string));
            assert!(parse_search_benchmark_args(&argv).is_err(), "{argv:?}");
        }
    }
}
