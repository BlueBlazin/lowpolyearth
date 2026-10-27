//! Capture/compare runner for the checked-in render contract manifest.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("../../testdata/contracts/capture-manifest.json");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let command = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("validate-manifest");
    match command {
        "validate-manifest" => {
            vearth_app::image_contract::validate_capture_manifest_json(MANIFEST)?;
            println!("capture manifest valid");
        }
        "compare" => {
            let baseline = args.get(2).ok_or("compare needs BASELINE.png")?;
            let candidate = args.get(3).ok_or("compare needs CANDIDATE.png")?;
            let difference = args.get(4).ok_or("compare needs DIFFERENCE.png")?;
            let thresholds = if args.iter().any(|arg| arg == "--portable") {
                vearth_app::image_contract::portable_thresholds_from_manifest_json(MANIFEST)?
            } else {
                vearth_app::image_contract::ImageThresholds::EXACT
            };
            let metrics = vearth_app::image_contract::compare_pngs(
                baseline, candidate, difference, thresholds,
            )?;
            println!("{}", serde_json::to_string_pretty(&metrics_json(metrics))?);
            if !metrics.accepted(thresholds) {
                return Err("render contract failed; difference image written".into());
            }
        }
        "capture" => capture(&args[2..])?,
        _ => return Err(format!("unknown command {command:?}").into()),
    }
    Ok(())
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|argument| argument == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn capture(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let app = PathBuf::from(value_after(args, "--app").ok_or("capture needs --app PATH")?);
    let output_dir =
        PathBuf::from(value_after(args, "--output-dir").ok_or("capture needs --output-dir DIR")?);
    let baseline_dir = value_after(args, "--baseline-dir").map(PathBuf::from);
    let offline_only = args.iter().any(|argument| argument == "--offline-only");
    let portable = args.iter().any(|argument| argument == "--portable");
    std::fs::create_dir_all(&output_dir)?;
    let manifest: Value = serde_json::from_str(MANIFEST)?;
    let mut results = Vec::new();
    for case in manifest["cases"]
        .as_array()
        .ok_or("manifest cases missing")?
    {
        let id = case["id"].as_str().ok_or("case id missing")?;
        let offline = case["offline"].as_bool().unwrap_or(false);
        if offline_only && !offline {
            continue;
        }
        let output = output_dir.join(format!("{id}.png"));
        let command: Vec<_> = case["command"]
            .as_array()
            .ok_or("case command missing")?
            .iter()
            .map(|argument| {
                argument
                    .as_str()
                    .ok_or("command argument must be a string")
                    .map(|argument| argument.replace("{output}", &output.to_string_lossy()))
            })
            .collect::<Result<_, _>>()?;
        let started = std::time::Instant::now();
        let status = std::process::Command::new(&app).args(&command).status()?;
        if !status.success() {
            return Err(format!("capture {id} failed with {status}").into());
        }
        let mut result = json!({
            "id": id,
            "offline": offline,
            "image": output,
            "elapsed_ms": started.elapsed().as_secs_f64() * 1e3,
            "command": command,
            "tags": case["tags"].clone(),
        });
        if let Some(baseline_dir) = &baseline_dir {
            let baseline = baseline_dir.join(format!("{id}.png"));
            let difference = output_dir.join(format!("{id}.diff.png"));
            let thresholds = if portable {
                vearth_app::image_contract::portable_thresholds_from_manifest_json(MANIFEST)?
            } else {
                vearth_app::image_contract::ImageThresholds::EXACT
            };
            let metrics = vearth_app::image_contract::compare_pngs(
                baseline, &output, difference, thresholds,
            )?;
            result["comparison"] = metrics_json(metrics);
            if !metrics.accepted(thresholds) {
                return Err(format!("capture {id} differs from its baseline").into());
            }
        }
        results.push(result);
    }
    let report = json!({
        "schema_version": 1,
        "manifest_fingerprint": vearth_core::contracts::Fingerprint::of_bytes(
            "capture-manifest.v1",
            MANIFEST.as_bytes(),
        ).to_string(),
        "app": app,
        "revision": git_revision(),
        "working_tree_dirty": git_working_tree_dirty(),
        "hardware": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "adapter": adapter_metadata(),
        },
        "fixed_metadata": manifest["fixed_metadata"].clone(),
        "offline_only": offline_only,
        "comparison_mode": if portable { "documented-gpu-float" } else { "exact-same-adapter" },
        "cases": results,
    });
    write_json(&output_dir.join("capture-report.json"), &report)?;
    Ok(())
}

fn adapter_metadata() -> Value {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    })) {
        Ok(adapter) => {
            let info = adapter.get_info();
            json!({
                "name": info.name,
                "vendor": info.vendor,
                "device": info.device,
                "device_type": format!("{:?}", info.device_type),
                "backend": format!("{:?}", info.backend),
                "driver": info.driver,
                "driver_info": info.driver_info,
            })
        }
        Err(error) => json!({"unavailable": error.to_string()}),
    }
}

fn git_revision() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into())
}

fn git_working_tree_dirty() -> bool {
    std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
}

fn metrics_json(metrics: vearth_app::image_contract::ImageMetrics) -> Value {
    json!({
        "width": metrics.width,
        "height": metrics.height,
        "exact": metrics.exact,
        "max_absolute": metrics.max_absolute,
        "p99_absolute": metrics.p99_absolute,
        "root_mean_square": metrics.root_mean_square,
        "max_relative": metrics.max_relative,
        "changed_pixels": metrics.changed_pixels,
        "changed_pixel_fraction": metrics.changed_pixel_fraction,
    })
}

fn write_json(path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
