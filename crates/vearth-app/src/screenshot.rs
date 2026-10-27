//! Headless screenshot mode: stream a view to completion, render offscreen,
//! and save a PNG without opening a window.
//!
//! ```text
//! vearth-app --screenshot out.png [--lat 46] [--lon 8] [--alt 16e6]
//!     [--heading 0] [--pitch 0] [--size 1280x800] [--timeout 90] [--play]
//!     [--play-vehicle kestrel|sparrow|phase-manta]
//!     [--play-heading-offset 0] [--play-pitch 20] [--play-bank 0]
//!     [--play-distance METERS] [--play-throttle 0.65] [--play-boost 0.25]
//!     [--time live|YYYY-MM-DDTHH:MM[:SS]Z] [--time-scale 600]
//!     [--surface-month 1..12]
//!     [--weather clear|procedural|preset:<id>] [--weather-seed N]
//!     [--weather-override key=value,key=value,...]
//!     [--screenshot-report report.json]
//!     [--fresh-chunks]
//!     [--no-color-grading] [--no-filmic-saturation]
//! ```

use crate::camera::Camera;
use crate::capture::{CaptureFrame, render_rgba, still_render_samples, write_png};
use crate::gfx::{PostProcessSettings, Renderer};
use crate::lod_selection::{
    Selection, SelectionMode, SelectionRequest, TerrainMetric, compute_selection,
};
use crate::streaming::{GenPool, LodView, ResidentId, ResidentStore, WantedCell};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use vearth_core::cell::CellId;
use vearth_platform::time::{Duration, Instant};

const SELECTION_REFRESH: Duration = Duration::from_millis(120);

fn select_capture_view(
    camera: &Camera,
    viewport_h: f64,
    mode: SelectionMode,
    index: &ResidentStore,
    metrics: &HashMap<CellId, TerrainMetric>,
    prev_split: HashSet<CellId>,
) -> Selection {
    compute_selection(
        SelectionRequest {
            cam_ecef: camera.eye_ecef(),
            viewport_h,
            fov_y: camera.fov_y,
            mode,
            prev_split,
            predicted_cam: None,
            destination_cam: None,
        },
        |cell| metrics.get(&cell).map(TerrainMetric::as_ref),
        |cell| index.contains(cell),
    )
}

fn selection_is_stable(previous: &Selection, next: &Selection) -> bool {
    previous.split == next.split
        && previous.wanted.len() == next.wanted.len()
        && previous.wanted.iter().zip(&next.wanted).all(|(a, b)| {
            a.cell == b.cell && a.lane == b.lane && a.screen_px.to_bits() == b.screen_px.to_bits()
        })
}

fn remove_unused_capture_residents(
    index: &mut ResidentStore,
    wanted: &[WantedCell],
) -> Vec<ResidentId> {
    let (draw, _) = index.drawable_cut(wanted);
    let keep: HashSet<_> = wanted
        .iter()
        .map(|wanted| wanted.cell)
        .chain(draw)
        .collect();
    let obsolete: Vec<_> = index.cells().filter(|cell| !keep.contains(cell)).collect();
    obsolete
        .into_iter()
        .filter_map(|cell| index.remove(cell).map(|(id, _)| id))
        .collect()
}

fn sample_distribution(values: impl IntoIterator<Item = f64>) -> serde_json::Value {
    let values: Vec<_> = values.into_iter().collect();
    let mut histogram = crate::telemetry::RollingHistogram::new(values.len().max(1));
    for value in values {
        histogram.record(value);
    }
    let summary = histogram.summary();
    serde_json::json!({
        "count": summary.count,
        "p50": summary.p50,
        "p95": summary.p95,
        "p99": summary.p99,
        "max": summary.max,
    })
}

fn require_complete_scene(missing: usize, in_flight: usize, trees: usize) -> Result<(), String> {
    if missing == 0 && in_flight == 0 && trees == 0 {
        Ok(())
    } else {
        Err(format!(
            "screenshot incomplete: {missing} cells missing, {in_flight} jobs in flight, {trees} tree assets pending; no image saved"
        ))
    }
}

#[derive(Clone)]
pub struct Args {
    pub out: String,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub alt_m: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub width: u32,
    pub height: u32,
    pub timeout_s: f64,
    /// Pose the selected playable craft in front of the camera.
    pub play: bool,
    pub play_vehicle: crate::vehicles::VehicleKind,
    /// Vehicle heading relative to the camera, in degrees clockwise.
    pub play_heading_offset_deg: f64,
    /// Render-only attitude, in degrees; positive pitch lowers the nose.
    pub play_pitch_deg: f64,
    pub play_bank_deg: f64,
    /// Distance ahead of the camera; absent uses the vehicle's chase boom.
    pub play_distance_m: Option<f64>,
    pub play_throttle: f32,
    pub play_boost: f32,
    pub solar_start: crate::solar::SolarStart,
    /// Optional fixed monthly imagery selection for controlled seasonal
    /// comparisons. The simulation clock and lifecycle remain unchanged.
    pub surface_month: Option<u8>,
    pub time_scale: Option<f64>,
    pub weather: crate::weather::WeatherConfig,
    pub post_process: PostProcessSettings,
    /// Optional machine-readable companion report for visual/performance
    /// baselines. The image path remains controlled independently by `out`.
    pub report_path: Option<String>,
    /// Regenerate geometry from source tiles instead of reusing chunk caches.
    pub fresh_chunks: bool,
    /// World-independent object-stage shot. When present, provider setup and
    /// world streaming are skipped entirely.
    pub stage: Option<crate::staging::StageOptions>,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            out: "screenshot.png".into(),
            lat_deg: 46.0,
            lon_deg: 8.0,
            alt_m: 16_000_000.0,
            heading_deg: 0.0,
            pitch_deg: 0.0,
            width: 1280,
            height: 800,
            timeout_s: 90.0,
            play: false,
            play_vehicle: crate::vehicles::VehicleKind::Kestrel,
            play_heading_offset_deg: 0.0,
            play_pitch_deg: 0.35_f64.to_degrees(),
            play_bank_deg: 0.0,
            play_distance_m: None,
            play_throttle: 0.65,
            play_boost: 0.25,
            solar_start: crate::solar::SolarStart::Live,
            surface_month: None,
            time_scale: None,
            weather: crate::weather::WeatherConfig::default(),
            post_process: PostProcessSettings::default(),
            report_path: None,
            fresh_chunks: false,
            stage: None,
        }
    }
}

pub fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(stage) = args.stage.clone() {
        return crate::staging::run_screenshot(
            &args.out,
            args.width.max(1),
            args.height.max(1),
            stage,
        );
    }
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let required_features = crate::gfx::renderer_features(
        adapter.features(),
        adapter.get_downlevel_capabilities().flags,
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("vearth headless"),
        required_features,
        ..Default::default()
    }))?;

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(device, queue, format);
    renderer.set_post_process_settings(args.post_process);

    let camera = Camera {
        lat: args.lat_deg.to_radians(),
        lon: args.lon_deg.to_radians(),
        alt_m: args.alt_m,
        heading: args.heading_deg.to_radians(),
        pitch: args.pitch_deg.to_radians(),
        fov_y: 55f64.to_radians(),
        aspect: args.width as f64 / args.height.max(1) as f64,
    };
    let cam_pos = camera.eye_ecef();

    let solar = crate::solar::SolarClock::new(args.solar_start, args.time_scale);
    let surface_month = args
        .surface_month
        .unwrap_or_else(|| crate::solar::utc_month(solar.utc_seconds()));
    let runtime_paths = crate::platform_paths::RuntimePaths::discover();
    runtime_paths.ensure_directories()?;
    let built = crate::sources::build_registry(&runtime_paths.cache, surface_month);
    let cache = (!args.fresh_chunks).then(|| {
        Arc::new(crate::chunk_cache::ChunkCache::new(
            &runtime_paths.cache,
            &built.dataset_tag,
        ))
    });
    let workers = vearth_platform::thread::available_parallelism()
        .map(|n| n.get() * 2 + 8)
        .unwrap_or(16)
        .clamp(16, 32);
    let mut pool = GenPool::with_async_acquisition_slots(
        built.registry,
        cache,
        workers,
        crate::streaming::ASYNC_ACQUISITION_CELLS.max(workers),
    );
    let mut index = ResidentStore::new();
    // Selector history is small and independent of the GPU residency needed by
    // the current cut. Evicting an obsolete mesh must not erase known relief.
    let mut metrics = HashMap::new();

    let selection_mode = if crate::app::legacy_lod_enabled() {
        SelectionMode::Legacy
    } else {
        SelectionMode::View(LodView::new(
            camera.view_rotation(),
            camera.fov_y,
            camera.aspect,
        ))
    };
    let mut selection = select_capture_view(
        &camera,
        args.height as f64,
        selection_mode,
        &index,
        &metrics,
        HashSet::new(),
    );
    let mut desired: HashSet<_> = selection.wanted.iter().map(|wanted| wanted.cell).collect();
    log::info!(
        "screenshot: {} wanted cells at ({:.3}, {:.3}, {:.0} m)",
        selection.wanted.len(),
        args.lat_deg,
        args.lon_deg,
        args.alt_m
    );

    // A timed-out view must never become a successful screenshot.
    let t0 = Instant::now();
    let mut selection_at = Instant::now();
    let mut metrics_dirty = false;
    loop {
        for mut meshed in pool.drain_done() {
            let cell = meshed.cell();
            let metric = TerrainMetric {
                height_range: meshed.height_range(),
                terrain_step_m: meshed.terrain_step_m(),
                terrain_relief_m: meshed.terrain_relief_m(),
                terrain_detail: meshed.terrain_detail().cloned(),
            };
            metrics_dirty |= metrics.get(&cell) != Some(&metric);
            metrics.insert(cell, metric);
            if !desired.contains(&cell) || index.contains(cell) {
                continue;
            }
            let resident = index.insert(
                cell,
                0,
                meshed.height_range(),
                meshed.terrain_step_m(),
                meshed.terrain_relief_m(),
                meshed.terrain_detail().cloned(),
                None,
                0,
            );
            let gpu_bytes = renderer.upload_prepared_cell(resident, &meshed) as usize;
            let solid_collision = (cell.level >= crate::streaming::COLLISION_MIN_LEVEL)
                .then(|| meshed.take_solid_collision());
            let terrain_collision = (cell.level >= crate::streaming::COLLISION_MIN_LEVEL)
                .then(|| meshed.take_terrain_collision())
                .flatten();
            let water = meshed.take_water();
            let road_guides = meshed.take_road_guides();
            index.finish_upload(
                resident,
                solid_collision,
                terrain_collision,
                water,
                road_guides,
                gpu_bytes,
            );
        }
        renderer.evict(&remove_unused_capture_residents(
            &mut index,
            &selection.wanted,
        ));
        let (_, mut missing) = index.drawable_cut(&selection.wanted);
        let settled = missing.is_empty() && pool.in_flight_count() == 0;
        // Completion of the previous cut can reveal relief that requests a finer
        // cut. Re-evaluate even inside the refresh interval before saving a view.
        if settled || (metrics_dirty && selection_at.elapsed() >= SELECTION_REFRESH) {
            let next = select_capture_view(
                &camera,
                args.height as f64,
                selection_mode,
                &index,
                &metrics,
                selection.split.clone(),
            );
            let stable = selection_is_stable(&selection, &next);
            selection = next;
            desired = selection.wanted.iter().map(|wanted| wanted.cell).collect();
            selection_at = Instant::now();
            metrics_dirty = false;
            renderer.evict(&remove_unused_capture_residents(
                &mut index,
                &selection.wanted,
            ));
            (_, missing) = index.drawable_cut(&selection.wanted);
            if settled && stable && missing.is_empty() && pool.in_flight_count() == 0 {
                break;
            }
        }
        pool.sync_queue(&missing);
        if t0.elapsed().as_secs_f64() > args.timeout_s {
            require_complete_scene(missing.len(), pool.in_flight_count(), 0)?;
            return Err(
                "screenshot incomplete: LOD selection did not stabilize; no image saved".into(),
            );
        }
        vearth_platform::thread::sleep(Duration::from_millis(15));
    }
    let wanted = selection.wanted;
    let (draw, missing) = index.drawable_cut(&wanted);
    require_complete_scene(missing.len(), pool.in_flight_count(), 0)?;
    let draw: Vec<crate::gfx::DrawCell> = draw
        .into_iter()
        .filter_map(|cell| {
            index
                .id(cell)
                .map(|resident| crate::gfx::DrawCell::opaque(resident, cell))
        })
        .collect();
    renderer.prepare_near_tree_assets(cam_pos, &draw);
    let tree_wait_started = Instant::now();
    loop {
        let pending = renderer.pending_tree_assets();
        if pending == 0 {
            // Settle geometry before starting the measured render samples;
            // frame count alone is not a wall-clock readiness guarantee.
            vearth_platform::thread::sleep(renderer.near_tree_readiness_delay(cam_pos, &draw));
            break;
        }
        if t0.elapsed().as_secs_f64() > args.timeout_s {
            require_complete_scene(0, pool.in_flight_count(), pending)?;
        }
        vearth_platform::thread::sleep(Duration::from_millis(15));
    }
    log::info!(
        "screenshot: near-tree assets settled in {:.2}s",
        tree_wait_started.elapsed().as_secs_f64()
    );
    let elapsed = t0.elapsed().as_secs_f64();
    let ground_alt_m = index
        .terrain_top_under(cam_pos.normalize_or(DVec3::X))
        .map(f64::from);
    let camera_agl_m = ground_alt_m.map(|ground| args.alt_m - ground);
    let (n_gen, hits) = pool.stats();
    let cache_stats = pool.cache_stats();
    let (reqs, bytes, errs) = vearth_data::metrics::snapshot();
    log::info!(
        "bench: {} cells in {elapsed:.1}s ({:.0} cells/s) | generated {n_gen} (cache hits {hits}) | http {reqs} reqs, {:.1} MB, {errs} errs",
        draw.len(),
        draw.len() as f64 / elapsed.max(0.001),
        bytes as f64 / 1e6,
    );

    // Explicit timestamps are frozen by default, making golden-hour/night
    // evidence reproducible. `live` samples the wall clock once for the shot.
    let lighting = solar.lighting();
    let weather_model = crate::weather::WeatherModel::new(args.weather);
    let weather_sample = weather_model.sample_weather(cam_pos, solar.utc_seconds());
    let weather = weather_model.render_state(cam_pos, solar.utc_seconds());
    let (tick0, tick1, tick_alpha) = crate::weather::WeatherModel::tick_pair(solar.utc_seconds());
    log::info!(
        "weather: atlas v{} model v{} mode={} seed={} preset={} utc={:.3} ticks={tick0}->{tick1} alpha={tick_alpha:.3} quality={:?} overrides={} | {} {:.1} mm/h (wet-hour {:.0}%, conditional {:.1}) · {:+.1} C · humidity {:.0}% · cloud {:.0}% L/M/H {:.0}/{:.0}/{:.0}% OD {:.1} ({:.0}-{:.0} m) · vis {:.1} km · surface wind E{:+.1}/N{:+.1}/U{:+.1} gust {:.1} m/s · mid E{:+.1}/N{:+.1} · upper E{:+.1}/N{:+.1} · aerosol {:.2} dust {:.0}%",
        vearth_data::climate::CLIMATE_ATLAS_VERSION,
        crate::weather::WEATHER_MODEL_VERSION,
        args.weather.mode,
        args.weather.seed,
        args.weather.preset,
        solar.utc_seconds(),
        args.weather.quality,
        !args.weather.overrides.is_empty(),
        weather_sample.precipitation_kind,
        weather_sample.precipitation_mm_h,
        weather_sample.precipitation_wet_fraction * 100.0,
        weather_sample.precipitation_conditional_mm_h,
        weather_sample.air_temperature_c,
        weather_sample.relative_humidity * 100.0,
        weather_sample.cloud_coverage * 100.0,
        weather_sample.low_cloud_coverage * 100.0,
        weather_sample.mid_cloud_coverage * 100.0,
        weather_sample.high_cloud_coverage * 100.0,
        weather_sample.cloud_optical_depth,
        weather_sample.cloud_base_m,
        weather_sample.cloud_top_m,
        weather_sample.visibility_m / 1000.0,
        weather_sample.wind.east_mps,
        weather_sample.wind.north_mps,
        weather_sample.wind.up_mps,
        weather_sample.wind.gust_mps,
        weather_sample.wind_mid.east_mps,
        weather_sample.wind_mid.north_mps,
        weather_sample.wind_upper.east_mps,
        weather_sample.wind_upper.north_mps,
        weather_sample.aerosol,
        weather_sample.aerosol_dust_fraction * 100.0,
    );

    // Pose the craft through the gameplay mesh, material, and effects paths.
    let player_draw = args.play.then(|| {
        let set = crate::vehicles::build_mesh_set();
        renderer.upload_player_parts_with_uv(&set.meshes, &set.uvs);
        renderer.write_vehicle_textures();
        let vehicle = args.play_vehicle;
        let distance_m = args.play_distance_m.unwrap_or(vehicle.chase_boom_m());
        let pos = cam_pos + camera.forward_ecef() * distance_m;
        let heading = camera.heading + args.play_heading_offset_deg.to_radians();
        let mut player = crate::player::Player::spawn_as(pos, heading, vehicle);
        player.lean_f = args.play_pitch_deg.to_radians();
        player.lean_r = args.play_bank_deg.to_radians();
        let (model, axes) = player.model_matrix();
        let layout = set.parts;
        let parts = layout.draw_parts(vehicle);
        crate::gfx::PlayerDraw {
            model,
            axes,
            parts,
            effects: Some(crate::gfx::VehicleEffects {
                kind: vehicle,
                throttle: args.play_throttle,
                boost: args.play_boost,
            }),
        }
    });

    let render_started = Instant::now();
    let capture = render_rgba(
        &mut renderer,
        CaptureFrame {
            width: args.width,
            height: args.height,
            output_format: format,
            cam_pos,
            view_rot: camera.view_rotation(),
            fov_y: camera.fov_y,
            draw_cells: &draw,
            lighting,
            weather,
            player: player_draw.as_ref(),
            render_samples: still_render_samples(),
        },
    )?;
    let render_wall_ms = render_started.elapsed().as_secs_f64() * 1e3;
    let mut seen_gpu_samples = std::collections::HashSet::new();
    let gpu_samples: Vec<_> = capture
        .sample_stats
        .iter()
        .filter(|sample| {
            sample
                .gpu_timing_sample_id
                .is_some_and(|id| seen_gpu_samples.insert(id))
        })
        .collect();
    let render_sample_report = serde_json::json!({
        "wall_ms": sample_distribution(capture.sample_wall_ms.iter().copied()),
        "cpu_sun_shadow_preparation_ms": sample_distribution(
            capture.sample_stats.iter().map(|sample| sample.cpu_sun_shadow_preparation_ms),
        ),
        "gpu_shadow_ms": sample_distribution(
            gpu_samples.iter().filter_map(|sample| sample.gpu_shadow_ms),
        ),
        "gpu_world_ms": sample_distribution(
            gpu_samples.iter().filter_map(|sample| sample.gpu_world_ms),
        ),
        "gpu_flora_ms": sample_distribution(
            gpu_samples.iter().filter_map(|sample| sample.gpu_flora_ms),
        ),
        "crop_geometry": vearth_platform::env::var("VEARTH_CROP_GEOMETRY").unwrap_or_else(|_| "canopy".into()),
        "ruggedness_policy": crate::streaming::ruggedness_policy().label(),
        "gpu_total_ms": sample_distribution(
            gpu_samples.iter().filter_map(|sample| sample.gpu_total_ms),
        ),
        "sun_shadow_updated_cascades": sample_distribution(
            capture.sample_stats.iter().map(|sample| sample.sun_shadow_updated_cascades as f64),
        ),
        "sun_shadow_caster_chunks": sample_distribution(
            capture.sample_stats.iter().map(|sample| sample.sun_shadow_caster_chunks as f64),
        ),
        "sun_shadow_caster_indices": sample_distribution(
            capture.sample_stats.iter().map(|sample| sample.sun_shadow_caster_indices as f64),
        ),
        "sun_shadow_batches": sample_distribution(
            capture.sample_stats.iter().map(|sample| sample.sun_shadow_batches as f64),
        ),
        "sun_shadow_capacity_drops": capture
            .sample_stats
            .iter()
            .map(|sample| sample.sun_shadow_capacity_drops)
            .sum::<usize>(),
    });
    let stats = capture.stats;
    log::info!(
        "screenshot: {} draws in {} batches (indirect {}, u16 {}, landmark {}, {} bytes), Sun shadows {}/{} cascades with {} caster chunks in {} batches ({} drops, {:.1} MiB), {} trees in {} batches ({} bytes), {} light cells in {} batches, {} tris, {} culled, {:.1} MiB resident mesh ({:.1} vertex + {:.1} index)",
        stats.draws,
        stats.submitted_world_batches,
        stats.indirect_world_batches,
        stats.submitted_u16_index_chunks,
        stats.submitted_landmark_chunks,
        stats.world_draw_upload_bytes,
        stats.sun_shadow_active,
        stats.sun_shadow_cascades,
        stats.sun_shadow_caster_chunks,
        stats.sun_shadow_batches,
        stats.sun_shadow_capacity_drops,
        stats.sun_shadow_working_set_bytes as f64 / (1024.0 * 1024.0),
        stats.submitted_tree_instances,
        stats.submitted_tree_batches,
        stats.tree_instance_upload_bytes,
        stats.submitted_light_cells,
        stats.submitted_light_batches,
        stats.triangles,
        stats.culled,
        stats.mesh.total_bytes() as f64 / (1024.0 * 1024.0),
        stats.mesh.vertex_bytes as f64 / (1024.0 * 1024.0),
        stats.mesh.index_bytes as f64 / (1024.0 * 1024.0),
    );

    write_png(&args.out, args.width, args.height, &capture.rgba)?;
    if let Some(report_path) = &args.report_path {
        let weather_report = serde_json::json!({
            "mode": args.weather.mode.to_string(),
            "seed": args.weather.seed,
            "preset": args.weather.preset,
            "temperature_c": weather_sample.air_temperature_c,
            "humidity": weather_sample.relative_humidity,
            "precipitation_mm_h": weather_sample.precipitation_mm_h,
            "precipitation_kind": weather_sample.precipitation_kind.to_string(),
            "wetness": weather_sample.wetness,
            "snow": weather_sample.snow_cover,
            "vegetation_phase": weather_sample.vegetation_phase,
            "drought": weather_sample.drought_stress,
        });
        let flora_debug = serde_json::json!({
            "visible_by_calendar": stats.flora.visible_by_calendar,
            "visible_by_registry": stats.flora.visible_by_registry,
            "glyph_budget": stats.flora.glyph_budget,
            "instances": stats.flora.admitted_instances,
            "instance_upload_budget_bytes": stats.flora.instance_upload_budget_bytes,
            "suppression_reason": stats.flora.suppression_reason.as_str(),
            "nearest_visible_m": stats.flora.nearest_visible_m,
            "farthest_visible_m": stats.flora.farthest_visible_m,
            "estimated_occupied_glyphs": stats.flora.estimated_occupied_glyphs,
            "estimated_flower_heads": stats.flora.estimated_flower_heads,
            "estimated_crop_clusters": stats.flora.estimated_crop_clusters,
            "estimated_crop_plant_equivalents":
                stats.flora.estimated_crop_plant_equivalents,
        });
        let report = serde_json::json!({
            "schema_version": 1,
            "flora_control": "procedural_gpu_flower_lod_coupling",
            "image_path": args.out,
            "pose": {
                "lat_deg": args.lat_deg,
                "lon_deg": args.lon_deg,
                "alt_m": args.alt_m,
                "ground_alt_m": ground_alt_m,
                "camera_agl_m": camera_agl_m,
                "heading_deg": args.heading_deg,
                "pitch_deg": args.pitch_deg,
            },
            "player": args.play.then(|| serde_json::json!({
                "vehicle": args.play_vehicle.name(),
                "heading_offset_deg": args.play_heading_offset_deg,
                "pitch_deg": args.play_pitch_deg,
                "bank_deg": args.play_bank_deg,
                "distance_m": args.play_distance_m.unwrap_or(args.play_vehicle.chase_boom_m()),
                "throttle": args.play_throttle,
                "boost": args.play_boost,
            })),
            "weather": weather_report,
            "surface": {
                "width": args.width,
                "height": args.height,
                "imagery_month": surface_month,
            },
            "timing": {
                "stream_wall_ms": elapsed * 1e3,
                "render_wall_ms": render_wall_ms,
                "cpu_visibility_ms": stats.cpu_visibility_ms,
                "cpu_world_draw_construction_ms": stats.cpu_world_draw_construction_ms,
                "cpu_sorting_ms": stats.cpu_sorting_ms,
                "cpu_uniform_construction_ms": stats.cpu_uniform_construction_ms,
                "cpu_grouping_ms": stats.cpu_grouping_ms,
                "cpu_indirect_construction_ms": stats.cpu_indirect_construction_ms,
                "cpu_sun_shadow_preparation_ms": stats.cpu_sun_shadow_preparation_ms,
                "upload_preparation_ms": stats.upload_preparation_ms,
                "queue_upload_ms": stats.queue_upload_ms,
                "gpu_total_ms": stats.gpu_total_ms,
                "gpu_shadow_ms": stats.gpu_shadow_ms,
                "gpu_world_ms": stats.gpu_world_ms,
            },
            "render_samples": render_sample_report,
            "generation": {
                "wanted_cells": wanted.len(),
                "missing_cells": missing.len(),
                "in_flight_jobs": pool.in_flight_count(),
                "pending_tree_assets": renderer.pending_tree_assets(),
                "cells": n_gen,
                "chunk_cache_hits": hits,
                "visible_cells": draw.len(),
                "cache": cache_stats.map(|cache| serde_json::json!({
                    "column_ram_hits": cache.ram_hits,
                    "column_disk_hits": cache.disk_hits,
                    "column_misses": cache.misses,
                    "column_ram_entries": cache.resident_entries,
                    "column_ram_bytes": cache.resident_bytes,
                    "shared_ram_budget_bytes": cache.budget_bytes,
                    "column_writes": cache.writes,
                    "column_evictions": cache.evictions,
                    "column_corruptions": cache.corruptions,
                    "column_disk_entries": cache.disk_entries,
                    "logical_disk_bytes": cache.logical_disk_bytes,
                    "physical_disk_bytes": cache.physical_disk_bytes,
                    "shared_disk_budget_bytes": cache.disk_budget_bytes,
                })),
            },
            "network": {
                "requests": reqs,
                "bytes": bytes,
                "errors": errs,
                "providers": vearth_data::metrics::acquisition_snapshots().into_iter().map(|s| {
                    serde_json::json!({"name": s.name, "errors": s.errors,
                        "not_found": s.not_found, "successes": s.successes,
                        "in_flight": s.in_flight, "queued": s.queued})
                }).collect::<Vec<_>>(),
            },
            "render": {
                "terrain_chunks": stats.draws,
                "indirect_first_instance": stats.indirect_world_batches,
                "submitted_world_indices": stats.submitted_world_indices,
                "submitted_world_vertices": stats.submitted_world_vertices,
                "world_draw_batches": stats.submitted_world_batches,
                "tree_instances": stats.submitted_tree_instances,
                "tree_batches": stats.submitted_tree_batches,
                "light_cells": stats.submitted_light_cells,
                "light_batches": stats.submitted_light_batches,
                "sun_shadow_configured": stats.sun_shadow_configured,
                "sun_shadow_active": stats.sun_shadow_active,
                "sun_shadow_cascades": stats.sun_shadow_cascades,
                "sun_shadow_updated_cascades": stats.sun_shadow_updated_cascades,
                "sun_shadow_caster_chunks": stats.sun_shadow_caster_chunks,
                "sun_shadow_caster_chunks_by_cascade":
                    stats.sun_shadow_caster_chunks_by_cascade,
                "sun_shadow_caster_indices": stats.sun_shadow_caster_indices,
                "sun_shadow_caster_indices_by_cascade":
                    stats.sun_shadow_caster_indices_by_cascade,
                "sun_shadow_batches": stats.sun_shadow_batches,
                "sun_shadow_capacity_drops": stats.sun_shadow_capacity_drops,
                "sun_shadow_working_set_bytes": stats.sun_shadow_working_set_bytes,
                "flora_visible_cells": stats.flora.visible_cells,
                "flora_visible_by_lod": stats.flora.visible_by_lod,
                "flora_visible_by_archetype": stats.flora.visible_by_archetype,
                "flora_visible_by_crop": vearth_gen::agriculture_catalog::CROP_TREATMENTS.iter()
                    .map(|treatment| (treatment.label, stats.flora.visible_by_crop[treatment.kind as usize]))
                    .collect::<std::collections::BTreeMap<_, _>>(),
                "flora_inferred_by_archetype": stats.flora.inferred_by_archetype,
                "flora_glyphs": stats.flora.glyphs,
                "flora_glyphs_by_lod": stats.flora.glyphs_by_lod,
                "flora_triangles": stats.flora.triangles,
                "flora_triangles_by_lod": stats.flora.triangles_by_lod,
                "flora_draw_calls": stats.flora.draw_calls,
                "flora_instance_upload_bytes": stats.flora.instance_upload_bytes,
                "flora_cells_suppressed_by_glyph_budget":
                    stats.flora.cells_suppressed_by_glyph_budget,
                "flora_cells_suppressed_by_triangle_budget":
                    stats.flora.cells_suppressed_by_triangle_budget,
                "flora_cells_suppressed_by_upload_budget":
                    stats.flora.cells_suppressed_by_upload_budget,
                "flora_debug": flora_debug,
                "flora_curated_destination_matches":
                    stats.flora.curated_destination_matches,
                "flora_cpu_preparation_ms": stats.flora.cpu_preparation_ms,
                "flora_gpu_ms": stats.gpu_flora_ms,
                "triangles": stats.triangles,
                "draw_upload_bytes": stats.world_draw_upload_bytes
                    + stats.tree_instance_upload_bytes
                    + stats.flora.instance_upload_bytes,
                "mesh_upload_bytes": stats.uploads.total_bytes(),
                "resident_gpu_bytes": stats.mesh.total_bytes(),
                "resident_vertex_bytes": stats.mesh.vertex_bytes,
                "resident_index_bytes": stats.mesh.index_bytes,
                "resident_flora_cells": stats.mesh.flora_cells,
                "resident_flora_descriptor_bytes": stats.mesh.flora_descriptor_bytes,
                "culled_chunks": stats.culled,
                "dropped_chunks": stats.dropped_chunks,
            },
        });
        let path = std::path::Path::new(report_path);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        log::info!("screenshot: wrote report {}", path.display());
    }
    log::info!("screenshot: wrote {}", args.out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::ellipsoid;

    #[test]
    fn incomplete_capture_is_rejected_even_when_ancestors_cover_the_view() {
        for (missing, jobs, trees) in [(1, 0, 0), (0, 1, 0), (0, 0, 1), (4, 2, 3)] {
            let error = require_complete_scene(missing, jobs, trees).unwrap_err();
            assert!(error.contains("no image saved"));
        }
        assert!(require_complete_scene(0, 0, 0).is_ok());
    }

    #[test]
    fn loaded_relief_requires_refinement_before_capture_can_settle() {
        let camera = Camera {
            lat: 40.015f64.to_radians(),
            lon: (-105.2705f64).to_radians(),
            alt_m: 4_000.0,
            heading: (-90f64).to_radians(),
            pitch: 70f64.to_radians(),
            fov_y: 55f64.to_radians(),
            aspect: 1.6,
        };
        let mountain_dir =
            ellipsoid::geodetic_to_ecef(40.02f64.to_radians(), (-105.55f64).to_radians(), 0.0)
                .normalize();
        let mountain_region = CellId::from_dir(mountain_dir, 9);
        let mode = SelectionMode::View(LodView::new(
            camera.view_rotation(),
            camera.fov_y,
            camera.aspect,
        ));
        let mut index = ResidentStore::new();
        let mut metrics = HashMap::new();
        let initial = select_capture_view(&camera, 1_280.0, mode, &index, &metrics, HashSet::new());
        let level_at_mountain = |selection: &Selection| {
            selection
                .wanted
                .iter()
                .find(|wanted| CellId::from_dir(mountain_dir, wanted.cell.level) == wanted.cell)
                .unwrap()
                .cell
                .level
        };
        let load = |selection: &Selection,
                    index: &mut ResidentStore,
                    metrics: &mut HashMap<CellId, TerrainMetric>| {
            for wanted in &selection.wanted {
                let mut ancestor = wanted.cell;
                while ancestor.level > mountain_region.level {
                    ancestor = ancestor.parent().unwrap();
                }
                let (height, step, relief) = if ancestor == mountain_region {
                    ((1_500.0, 3_000.0), 90.0, 1_500.0)
                } else {
                    ((0.0, 0.0), 0.0, 0.0)
                };
                index.insert(wanted.cell, 0, height, step, relief, None, None, 0);
                metrics.insert(
                    wanted.cell,
                    TerrainMetric {
                        height_range: height,
                        terrain_step_m: step,
                        terrain_relief_m: relief,
                        terrain_detail: None,
                    },
                );
            }
        };
        load(&initial, &mut index, &mut metrics);
        let (_, stale_missing) = index.drawable_cut(&initial.wanted);
        assert!(require_complete_scene(stale_missing.len(), 0, 0).is_ok());

        let mut selection = select_capture_view(
            &camera,
            1_280.0,
            mode,
            &index,
            &metrics,
            initial.split.clone(),
        );
        assert!(level_at_mountain(&selection) > level_at_mountain(&initial));
        assert!(!selection_is_stable(&initial, &selection));
        let (_, missing) = index.drawable_cut(&selection.wanted);
        assert!(require_complete_scene(missing.len(), 0, 0).is_err());

        for _ in 0..12 {
            load(&selection, &mut index, &mut metrics);
            remove_unused_capture_residents(&mut index, &selection.wanted);
            let next = select_capture_view(
                &camera,
                1_280.0,
                mode,
                &index,
                &metrics,
                selection.split.clone(),
            );
            let stable = selection_is_stable(&selection, &next);
            selection = next;
            let (_, missing) = index.drawable_cut(&selection.wanted);
            if stable && missing.is_empty() {
                assert!(
                    metrics.len() > index.len(),
                    "obsolete meshes should be evicted while their relief metadata survives"
                );
                assert!(selection.predicted_prefetch.is_empty());
                assert!(selection.destination_prefetch.is_empty());
                assert!(require_complete_scene(0, 0, 0).is_ok());
                return;
            }
        }
        panic!("loaded, stationary terrain selection did not converge");
    }

    #[test]
    fn capture_reclaims_obsolete_meshes_after_their_backdrop_is_replaced() {
        use crate::streaming::ViewLane;
        use vearth_core::cubesphere::Face;
        let parent = CellId {
            face: Face(0),
            level: 8,
            i: 100,
            j: 50,
        };
        let unrelated = CellId { i: 110, ..parent };
        let children = parent.children();
        let wanted: Vec<_> = children
            .iter()
            .map(|&cell| WantedCell {
                cell,
                screen_px: 100.0,
                lane: ViewLane::Visible,
            })
            .collect();
        let mut index = ResidentStore::new();
        let parent_id = index.insert(parent, 0, (100.0, 900.0), 90.0, 800.0, None, None, 100);
        let unrelated_id = index.insert(unrelated, 0, (0.0, 0.0), 0.0, 0.0, None, None, 100);
        for &cell in &children[..3] {
            index.insert(cell, 0, (100.0, 900.0), 45.0, 800.0, None, None, 100);
        }
        assert_eq!(
            remove_unused_capture_residents(&mut index, &wanted),
            vec![unrelated_id]
        );
        assert!(index.contains(parent));
        assert_eq!(index.drawable_cut(&wanted).1.len(), 1);
        index.insert(children[3], 0, (100.0, 900.0), 45.0, 800.0, None, None, 100);
        assert_eq!(
            remove_unused_capture_residents(&mut index, &wanted),
            vec![parent_id]
        );
        assert!(!index.contains(parent));
        assert_eq!(index.len(), 4);
        assert_eq!(index.gpu_bytes(), 400);
        assert!(index.drawable_cut(&wanted).1.is_empty());
        assert!(remove_unused_capture_residents(&mut index, &wanted).is_empty());
    }
}
