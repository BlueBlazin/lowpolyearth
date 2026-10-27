//! High-quality still capture primitives.
//!
//! Interactive capture renders directly into an offscreen presentation
//! target. UI overlays are a separate pass and therefore never enter the
//! exported pixels.

use crate::gfx::{
    CLOUD_HISTORY_WARMUP_FRAMES, DrawCell, FrameStats, PlayerDraw, Renderer, create_depth,
};
use crate::solar::Lighting;
#[cfg(not(target_arch = "wasm32"))]
use crate::streaming::{GenPool, LodView, ResidentStore, compute_wanted_for_view};
use crate::weather::{WeatherConfig, WeatherRenderState};
use glam::{DMat4, DVec3};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use vearth_core::cell::CellId;
#[cfg(not(target_arch = "wasm32"))]
use vearth_platform::time::Duration;
use vearth_platform::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhotoResolution {
    K1,
    K2,
    #[default]
    K4,
    K8,
}

impl PhotoResolution {
    pub const ALL: [Self; 4] = [Self::K1, Self::K2, Self::K4, Self::K8];

    pub const fn label(self) -> &'static str {
        match self {
            Self::K1 => "1024",
            Self::K2 => "2K",
            Self::K4 => "4K UHD",
            Self::K8 => "8K UHD",
        }
    }

    pub const fn long_edge(self) -> u32 {
        match self {
            Self::K1 => 1_024,
            Self::K2 => 2_048,
            Self::K4 => 3_840,
            Self::K8 => 7_680,
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "1024" | "1k" => Ok(Self::K1),
            "2048" | "2k" => Ok(Self::K2),
            "3840" | "4k" | "4k-uhd" | "4k uhd" | "uhd" => Ok(Self::K4),
            "7680" | "8k" | "8k-uhd" | "8k uhd" => Ok(Self::K8),
            _ => Err(format!(
                "unknown photo quality {value:?}; expected 1024|2k|4k|8k"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhotoAspect {
    #[default]
    Current,
    Landscape16x9,
    Photo3x2,
    Classic4x3,
    Square,
    Portrait9x16,
}

impl PhotoAspect {
    pub const ALL: [Self; 6] = [
        Self::Current,
        Self::Landscape16x9,
        Self::Photo3x2,
        Self::Classic4x3,
        Self::Square,
        Self::Portrait9x16,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Current => "Current view",
            Self::Landscape16x9 => "16:9 landscape",
            Self::Photo3x2 => "3:2 photo",
            Self::Classic4x3 => "4:3 classic",
            Self::Square => "1:1 square",
            Self::Portrait9x16 => "9:16 portrait",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "current" | "view" => Ok(Self::Current),
            "16:9" | "16x9" | "landscape" => Ok(Self::Landscape16x9),
            "3:2" | "3x2" | "photo" => Ok(Self::Photo3x2),
            "4:3" | "4x3" | "classic" => Ok(Self::Classic4x3),
            "1:1" | "1x1" | "square" => Ok(Self::Square),
            "9:16" | "9x16" | "portrait" => Ok(Self::Portrait9x16),
            _ => Err(format!(
                "unknown photo aspect {value:?}; expected current|16:9|3:2|4:3|1:1|9:16"
            )),
        }
    }

    fn ratio(self, current_width: u32, current_height: u32) -> f64 {
        match self {
            Self::Current => current_width.max(1) as f64 / current_height.max(1) as f64,
            Self::Landscape16x9 => 16.0 / 9.0,
            Self::Photo3x2 => 3.0 / 2.0,
            Self::Classic4x3 => 4.0 / 3.0,
            Self::Square => 1.0,
            Self::Portrait9x16 => 9.0 / 16.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhotoSettings {
    pub resolution: PhotoResolution,
    pub aspect: PhotoAspect,
    pub output_dir: PathBuf,
}

impl Default for PhotoSettings {
    fn default() -> Self {
        Self {
            resolution: PhotoResolution::K4,
            aspect: PhotoAspect::Current,
            output_dir: crate::platform_paths::RuntimePaths::discover().photos,
        }
    }
}

impl PhotoSettings {
    /// Fit the selected aspect ratio inside the preset's named long edge.
    pub fn dimensions(&self, current_width: u32, current_height: u32) -> (u32, u32) {
        dimensions_for_long_edge(
            self.resolution.long_edge(),
            self.aspect.ratio(current_width, current_height),
        )
    }
}

pub fn dimensions_for_long_edge(long_edge: u32, aspect: f64) -> (u32, u32) {
    let long_edge = long_edge.max(1);
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    if aspect >= 1.0 {
        (
            long_edge,
            ((long_edge as f64 / aspect).round() as u32).max(1),
        )
    } else {
        (
            ((long_edge as f64 * aspect).round() as u32).max(1),
            long_edge,
        )
    }
}

#[derive(Clone, Debug, Default)]
pub enum PhotoStatus {
    #[default]
    Idle,
    Preparing {
        width: u32,
        height: u32,
        remaining: usize,
    },
    Capturing {
        width: u32,
        height: u32,
    },
    Saved(PhotoArtifact),
    Failed(String),
}

impl PhotoStatus {
    pub fn busy(&self) -> bool {
        matches!(self, Self::Preparing { .. } | Self::Capturing { .. })
    }
}

#[derive(Clone, Debug)]
pub struct PhotoArtifact {
    pub image_path: PathBuf,
    pub recipe_path: PathBuf,
    pub width: u32,
    pub height: u32,
}

pub struct CaptureFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub output_format: wgpu::TextureFormat,
    pub cam_pos: DVec3,
    pub view_rot: DMat4,
    pub fov_y: f64,
    pub draw_cells: &'a [DrawCell],
    pub lighting: Lighting,
    pub weather: WeatherRenderState,
    pub player: Option<&'a PlayerDraw>,
    /// Number of samples used to settle temporal cloud history.
    pub render_samples: usize,
}

pub struct CapturedPixels {
    pub rgba: Vec<u8>,
    pub stats: FrameStats,
    /// Every temporal-settling submission, retained for bounded screenshot
    /// percentile reports. Interactive and cinematic one-frame captures keep
    /// a single element.
    pub sample_stats: Vec<FrameStats>,
    pub sample_wall_ms: Vec<f64>,
}

#[derive(Clone, Copy)]
pub struct PhotoRecipe {
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub alt_m: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub fov_y_deg: f64,
    pub utc_seconds: f64,
    pub weather: WeatherConfig,
}

/// A capture whose pixels are being copied back from the GPU.
pub struct PendingCapture {
    readback: wgpu::Buffer,
    mapped: Arc<std::sync::Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>,
    width: u32,
    height: u32,
    bytes_per_row: u32,
    bgra: bool,
    sample_stats: Vec<FrameStats>,
    sample_wall_ms: Vec<f64>,
}

impl PendingCapture {
    /// The pixels once the readback has mapped, or `self` to try again after
    /// the device has progressed.
    pub fn try_finish(self) -> Result<Result<CapturedPixels, Box<dyn std::error::Error>>, Self> {
        let mapped = self.mapped.lock().unwrap().take();
        match mapped {
            None => Err(self),
            Some(Err(error)) => Ok(Err(error.into())),
            Some(Ok(())) => Ok(self.read()),
        }
    }

    fn read(self) -> Result<CapturedPixels, Box<dyn std::error::Error>> {
        let slice = self.readback.slice(..);
        let mapped = slice.get_mapped_range()?;
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in 0..self.height {
            let start = (row * self.bytes_per_row) as usize;
            let source = &mapped[start..start + (self.width * 4) as usize];
            if self.bgra {
                for pixel in source.chunks_exact(4) {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
            } else {
                rgba.extend_from_slice(source);
            }
        }
        drop(mapped);
        Ok(CapturedPixels {
            rgba,
            stats: *self
                .sample_stats
                .last()
                .expect("at least one capture sample is rendered"),
            sample_stats: self.sample_stats,
            sample_wall_ms: self.sample_wall_ms,
        })
    }
}

/// Renders a still and waits for its pixels. Native only: in the browser the
/// device progresses on the worker's event loop, so callers use
/// [`submit_capture`] and poll [`PendingCapture::try_finish`] across frames.
#[cfg(not(target_arch = "wasm32"))]
pub fn render_rgba(
    renderer: &mut Renderer,
    frame: CaptureFrame<'_>,
) -> Result<CapturedPixels, Box<dyn std::error::Error>> {
    let pending = submit_capture(renderer, frame)?;
    renderer.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(120)),
    })?;
    match pending.try_finish() {
        Ok(result) => result,
        Err(_) => Err("photo readback did not complete".into()),
    }
}

/// Renders a still offscreen and starts copying its pixels back.
pub fn submit_capture(
    renderer: &mut Renderer,
    frame: CaptureFrame<'_>,
) -> Result<PendingCapture, Box<dyn std::error::Error>> {
    let max_dimension = renderer.device.limits().max_texture_dimension_2d;
    if frame.width == 0 || frame.height == 0 {
        return Err("capture dimensions must be greater than zero".into());
    }
    if frame.width > max_dimension || frame.height > max_dimension {
        return Err(format!(
            "{}x{} exceeds this GPU's {} pixel texture limit",
            frame.width, frame.height, max_dimension
        )
        .into());
    }
    if !matches!(
        frame.output_format,
        wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Bgra8UnormSrgb
    ) {
        return Err(format!(
            "capture requires an 8-bit RGBA/BGRA output format, got {:?}",
            frame.output_format
        )
        .into());
    }

    let color = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("photo capture color"),
        size: wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: frame.output_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color.create_view(&Default::default());
    let depth_view = create_depth(&renderer.device, frame.width, frame.height);

    let sample_count = frame.render_samples.max(1);
    let mut sample_stats = Vec::with_capacity(sample_count);
    let mut sample_wall_ms = Vec::with_capacity(sample_count);
    for _ in 0..frame.render_samples.max(1) {
        let sample_started = Instant::now();
        let sample = renderer.render(
            &color_view,
            &depth_view,
            frame.width,
            frame.height,
            frame.cam_pos,
            frame.view_rot,
            frame.fov_y,
            frame.draw_cells,
            frame.lighting,
            frame.weather,
            frame.player,
        );
        // Headless stills have no swapchain pacing. Waiting here prevents the
        // temporal warmup loop from measuring queue growth instead of frame
        // cost and lets the asynchronous timestamp ring advance once per
        // sample. This path is never used by interactive rendering. Browser
        // devices cannot block; their samples simply queue.
        #[cfg(not(target_arch = "wasm32"))]
        renderer.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(120)),
        })?;
        sample_stats.push(sample);
        sample_wall_ms.push(sample_started.elapsed().as_secs_f64() * 1e3);
    }

    let bytes_per_row = (frame.width * 4).div_ceil(256) * 256;
    let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("photo capture readback"),
        size: u64::from(bytes_per_row) * u64::from(frame.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(frame.height),
            },
        },
        wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit([encoder.finish()]);

    let mapped = Arc::new(std::sync::Mutex::new(None));
    let signal = mapped.clone();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            *signal.lock().unwrap() = Some(result);
        });
    Ok(PendingCapture {
        readback,
        mapped,
        width: frame.width,
        height: frame.height,
        bytes_per_row,
        bgra: matches!(
            frame.output_format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ),
        sample_stats,
        sample_wall_ms,
    })
}

pub fn write_photo_bundle(
    settings: &PhotoSettings,
    width: u32,
    height: u32,
    rgba: &[u8],
    recipe: PhotoRecipe,
) -> Result<PhotoArtifact, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&settings.output_dir)?;
    let stem = photo_stem(width, height);
    let image_path = unique_path(&settings.output_dir, &stem, "png");
    let recipe_path = image_path.with_extension("json");

    write_png(&image_path, width, height, rgba)?;

    let file_name = image_path.file_name().and_then(|name| name.to_str());
    let document = recipe_document(settings, width, height, file_name, recipe);
    std::fs::write(&recipe_path, serde_json::to_vec_pretty(&document)?)?;

    Ok(PhotoArtifact {
        image_path,
        recipe_path,
        width,
        height,
    })
}

/// A timestamped photo file stem, unique to the millisecond.
fn photo_stem(width: u32, height: u32) -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "low-poly-earth-{}-{:03}-{width}x{height}",
        elapsed.as_secs(),
        elapsed.subsec_millis()
    )
}

/// Encodes a photo for download from the browser. The recipe travels inside
/// the PNG as an iTXt chunk because pages cannot reliably save two files.
#[cfg(target_arch = "wasm32")]
pub fn photo_download(
    settings: &PhotoSettings,
    width: u32,
    height: u32,
    rgba: &[u8],
    recipe: PhotoRecipe,
) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    let name = format!("{}.png", photo_stem(width, height));
    let document = recipe_document(settings, width, height, Some(&name), recipe);
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.add_itxt_chunk(
        "vearth.photo".into(),
        serde_json::to_string_pretty(&document)?,
    )?;
    encoder.write_header()?.write_image_data(rgba)?;
    Ok((name, png))
}

fn recipe_document(
    settings: &PhotoSettings,
    width: u32,
    height: u32,
    file_name: Option<&str>,
    recipe: PhotoRecipe,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "vearth.photo/v1",
        "engineVersion": env!("CARGO_PKG_VERSION"),
        "image": {
            "file": file_name,
            "width": width,
            "height": height,
            "quality": settings.resolution.label(),
            "aspect": settings.aspect.label(),
            "color": "sRGB RGBA8",
        },
        "camera": {
            "latDeg": recipe.lat_deg,
            "lonDeg": recipe.lon_deg,
            "altM": recipe.alt_m,
            "headingDeg": recipe.heading_deg,
            "pitchDeg": recipe.pitch_deg,
            "verticalFovDeg": recipe.fov_y_deg,
        },
        "environment": {
            "utcSeconds": recipe.utc_seconds,
            "utc": crate::solar::format_utc(recipe.utc_seconds),
            "solarMode": "cycle",
            "weather": {
                "mode": recipe.weather.mode.to_string(),
                "quality": format!("{:?}", recipe.weather.quality).to_ascii_lowercase(),
                "seed": recipe.weather.seed,
                "preset": recipe.weather.preset,
                "windInfluence": format!("{:?}", recipe.weather.wind_influence).to_ascii_lowercase(),
                "precipitationCameraEffects": recipe.weather.precipitation_camera_effects,
                "overrides": format!("{:?}", recipe.weather.overrides),
            },
        },
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_screenshot(
    directory: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let stem = format!(
        "low-poly-earth-screenshot-{}-{:03}",
        elapsed.as_secs(),
        elapsed.subsec_millis()
    );
    let path = unique_path(directory, &stem, "png");
    write_png(&path, width, height, rgba)?;
    Ok(path)
}

pub fn write_png(
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(format!(
            "RGBA payload has {} bytes; {width}×{height} requires {expected}",
            rgba.len()
        )
        .into());
    }
    if let Some(parent) = path.as_ref().parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}

pub fn still_render_samples() -> usize {
    CLOUD_HISTORY_WARMUP_FRAMES
}

/// Long-lived, command-line-only renderer for film frames.
///
/// Each pose blocks until its complete wanted set and near-tree assets are
/// ready, then renders from the same resident store and temporal atmosphere
/// history as the preceding frame. It is deliberately isolated from the
/// interactive app, so none of this waiting or bookkeeping affects gameplay.
#[cfg(not(target_arch = "wasm32"))]
pub struct CinematicCaptureSession {
    renderer: Renderer,
    output_format: wgpu::TextureFormat,
    pool: GenPool,
    index: ResidentStore,
    split_set: HashSet<CellId>,
    vehicle_parts: crate::vehicles::VehicleRenderParts,
    rendered_frames: u64,
    last_vehicle_heading: Option<f64>,
    samples_per_frame: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl CinematicCaptureSession {
    pub fn new(
        surface_month: u8,
        post_process: crate::gfx::PostProcessSettings,
    ) -> Result<Self, Box<dyn std::error::Error>> {
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
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("vearth cinematic capture"),
                required_features,
                ..Default::default()
            }))?;
        let output_format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut renderer = Renderer::new(device, queue, output_format);
        renderer.set_post_process_settings(post_process);
        let vehicle = crate::vehicles::build_mesh_set();
        renderer.upload_player_parts_with_uv(&vehicle.meshes, &vehicle.uvs);
        renderer.write_vehicle_textures();

        let built = crate::sources::build_registry("cache", surface_month);
        let cache = Arc::new(crate::chunk_cache::ChunkCache::new(
            "cache",
            &built.dataset_tag,
        ));
        let workers = vearth_platform::thread::available_parallelism()
            .map(|count| count.get() * 2 + 8)
            .unwrap_or(16)
            .clamp(16, 32);
        Ok(Self {
            renderer,
            output_format,
            pool: GenPool::with_async_acquisition_slots(
                built.registry,
                Some(cache),
                workers,
                crate::streaming::ASYNC_ACQUISITION_CELLS.max(workers),
            ),
            index: ResidentStore::new(),
            split_set: HashSet::new(),
            vehicle_parts: vehicle.parts,
            rendered_frames: 0,
            last_vehicle_heading: None,
            samples_per_frame: 1,
        })
    }

    /// Samples rendered at each pose after the first frame, which always warms
    /// the full cloud history.
    pub fn set_samples_per_frame(&mut self, samples: usize) {
        self.samples_per_frame = samples.max(1);
    }

    fn drain_ready(&mut self) {
        for mut meshed in self.pool.drain_done() {
            let cell = meshed.cell();
            let resident = self.index.insert(
                cell,
                self.rendered_frames,
                meshed.height_range(),
                meshed.terrain_step_m(),
                meshed.terrain_relief_m(),
                meshed.terrain_detail().cloned(),
                None,
                0,
            );
            let gpu_bytes = self.renderer.upload_prepared_cell(resident, &meshed) as usize;
            let solid_collision = (cell.level >= crate::streaming::COLLISION_MIN_LEVEL)
                .then(|| meshed.take_solid_collision());
            let terrain_collision = (cell.level >= crate::streaming::COLLISION_MIN_LEVEL)
                .then(|| meshed.take_terrain_collision())
                .flatten();
            let water = meshed.take_water();
            let road_guides = meshed.take_road_guides();
            self.index.finish_upload(
                resident,
                solid_collision,
                terrain_collision,
                water,
                road_guides,
                gpu_bytes,
            );
        }
    }

    pub fn render_frame(
        &mut self,
        args: &crate::screenshot::Args,
        ground_clearance_m: Option<f64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if args.stage.is_some() {
            return Err("cinematic capture does not support object-stage frames".into());
        }
        let mut camera = crate::camera::Camera {
            lat: args.lat_deg.to_radians(),
            lon: args.lon_deg.to_radians(),
            alt_m: args.alt_m,
            heading: args.heading_deg.to_radians(),
            pitch: args.pitch_deg.to_radians(),
            fov_y: 55f64.to_radians(),
            aspect: args.width as f64 / args.height.max(1) as f64,
        };
        let mut cam_pos = camera.eye_ecef();
        let lod_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
        let (mut wanted, split) =
            compute_wanted_for_view(cam_pos, args.height as f64, &lod_view, &self.split_set);
        self.split_set = split;
        wanted.sort_by(crate::streaming::WantedCell::stream_order);

        let started = vearth_platform::time::Instant::now();
        loop {
            self.drain_ready();
            let (_, missing) = self.index.drawable_cut(&wanted);
            if missing.is_empty() && self.pool.in_flight_count() == 0 {
                break;
            }
            self.pool.sync_queue(&missing);
            if started.elapsed().as_secs_f64() > args.timeout_s {
                return Err(format!(
                    "cinematic frame {} did not reach full detail in {:.1}s ({} cells missing, {} jobs in flight)",
                    self.rendered_frames,
                    args.timeout_s,
                    missing.len(),
                    self.pool.in_flight_count(),
                )
                .into());
            }
            vearth_platform::thread::sleep(Duration::from_millis(10));
        }
        self.drain_ready();

        if let Some(clearance_m) = ground_clearance_m {
            let terrain_top = self
                .index
                .terrain_top_under(cam_pos.normalize_or(glam::DVec3::Z))
                .ok_or_else(|| {
                    format!(
                        "cinematic frame {} could not resolve terrain for {:.1}m ground clearance",
                        self.rendered_frames, clearance_m,
                    )
                })?;
            camera.alt_m = f64::from(terrain_top.max(0.0)) + clearance_m;
            cam_pos = camera.eye_ecef();
            let clearance_view = LodView::new(camera.view_rotation(), camera.fov_y, camera.aspect);
            let (updated_wanted, updated_split) = compute_wanted_for_view(
                cam_pos,
                args.height as f64,
                &clearance_view,
                &self.split_set,
            );
            wanted = updated_wanted;
            self.split_set = updated_split;
            wanted.sort_by(crate::streaming::WantedCell::stream_order);
            let clearance_started = vearth_platform::time::Instant::now();
            loop {
                self.drain_ready();
                let (_, missing) = self.index.drawable_cut(&wanted);
                if missing.is_empty() && self.pool.in_flight_count() == 0 {
                    break;
                }
                self.pool.sync_queue(&missing);
                if clearance_started.elapsed().as_secs_f64() > args.timeout_s {
                    return Err(format!(
                        "cinematic frame {} did not settle its ground-relative view in {:.1}s ({} cells missing, {} jobs in flight)",
                        self.rendered_frames,
                        args.timeout_s,
                        missing.len(),
                        self.pool.in_flight_count(),
                    )
                    .into());
                }
                vearth_platform::thread::sleep(Duration::from_millis(10));
            }
            self.drain_ready();
        }

        let (draw_cells, missing) = self.index.drawable_cut(&wanted);
        if !missing.is_empty() {
            return Err(format!(
                "cinematic frame {} still has {} missing cells after settling",
                self.rendered_frames,
                missing.len(),
            )
            .into());
        }
        let draw: Vec<DrawCell> = draw_cells
            .iter()
            .filter_map(|&cell| {
                self.index
                    .id(cell)
                    .map(|resident| DrawCell::opaque(resident, cell))
            })
            .collect();
        for &cell in &draw_cells {
            self.index.touch(cell, self.rendered_frames);
        }

        self.renderer.prepare_near_tree_assets(cam_pos, &draw);
        let trees_started = vearth_platform::time::Instant::now();
        loop {
            let pending = self.renderer.pending_tree_assets();
            if pending == 0 {
                vearth_platform::thread::sleep(
                    self.renderer.near_tree_readiness_delay(cam_pos, &draw),
                );
                break;
            }
            if trees_started.elapsed().as_secs_f64() > args.timeout_s {
                return Err(format!(
                    "cinematic frame {} did not finish {pending} near-tree assets in {:.1}s",
                    self.rendered_frames, args.timeout_s,
                )
                .into());
            }
            vearth_platform::thread::sleep(Duration::from_millis(10));
            self.renderer.prepare_near_tree_assets(cam_pos, &draw);
        }

        let solar = crate::solar::SolarClock::new(args.solar_start, args.time_scale);
        let lighting = solar.lighting();
        let weather_model = crate::weather::WeatherModel::new(args.weather);
        let weather = weather_model.render_state(cam_pos, solar.utc_seconds());
        let player_draw = args.play.then(|| {
            let vehicle = args.play_vehicle;
            let pos = cam_pos + camera.forward_ecef() * vehicle.chase_boom_m();
            let mut player = crate::player::Player::spawn_as(pos, camera.heading, vehicle);
            player.lean_f = match vehicle {
                crate::vehicles::VehicleKind::Kestrel => 0.12,
                crate::vehicles::VehicleKind::Sparrow => 0.04,
                _ => 0.18,
            };
            if let Some(previous) = self.last_vehicle_heading {
                let delta = (camera.heading - previous + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
                player.lean_r = (-delta * 3.0).clamp(-0.28, 0.28);
            }
            let (model, axes) = player.model_matrix();
            let layout = self.vehicle_parts;
            let parts = layout.draw_parts(vehicle);
            PlayerDraw {
                model,
                axes,
                parts,
                effects: Some(crate::gfx::VehicleEffects {
                    kind: vehicle,
                    throttle: 0.65,
                    boost: 0.25,
                }),
            }
        });
        if args.play {
            self.last_vehicle_heading = Some(camera.heading);
        } else {
            self.last_vehicle_heading = None;
        }

        let capture = render_rgba(
            &mut self.renderer,
            CaptureFrame {
                width: args.width,
                height: args.height,
                output_format: self.output_format,
                cam_pos,
                view_rot: camera.view_rotation(),
                fov_y: camera.fov_y,
                draw_cells: &draw,
                lighting,
                weather,
                player: player_draw.as_ref(),
                render_samples: if self.rendered_frames == 0 {
                    still_render_samples()
                } else {
                    self.samples_per_frame
                },
            },
        )?;
        write_png(&args.out, args.width, args.height, &capture.rgba)?;
        self.rendered_frames += 1;

        let victims = self.index.eviction_victims(self.rendered_frames);
        if !victims.is_empty() {
            let resident_ids: Vec<_> = victims
                .iter()
                .filter_map(|&cell| self.index.id(cell))
                .collect();
            self.renderer.evict(&resident_ids);
            for cell in victims {
                self.index.remove(cell);
            }
        }
        Ok(())
    }
}

fn unique_path(directory: &Path, stem: &str, extension: &str) -> PathBuf {
    let first = directory.join(format!("{stem}.{extension}"));
    if !first.exists() {
        return first;
    }
    for suffix in 2..=999 {
        let candidate = directory.join(format!("{stem}-{suffix}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{stem}-overflow.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshots_save_only_pngs_with_original_dimensions_and_pixels() {
        let directory = std::env::temp_dir().join(format!(
            "vearth-screenshot-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let rgba = [255, 0, 0, 255, 0, 128, 255, 255];
        let first = write_screenshot(&directory, 2, 1, &rgba).unwrap();
        let second = write_screenshot(&directory, 2, 1, &rgba).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        for path in [first, second] {
            assert_eq!(path.extension().unwrap(), "png");
            let decoder =
                png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
            let mut reader = decoder.read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((info.width, info.height), (2, 1));
            assert_eq!(pixels, rgba);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn named_photo_presets_cover_1024_through_8k() {
        assert_eq!(PhotoResolution::K1.long_edge(), 1_024);
        assert_eq!(PhotoResolution::K2.long_edge(), 2_048);
        assert_eq!(PhotoResolution::K4.long_edge(), 3_840);
        assert_eq!(PhotoResolution::K8.long_edge(), 7_680);
        for preset in PhotoResolution::ALL {
            assert_eq!(PhotoResolution::parse(preset.label()).unwrap(), preset);
        }
    }

    #[test]
    fn aspect_presets_fit_the_named_long_edge() {
        let settings = PhotoSettings {
            resolution: PhotoResolution::K4,
            aspect: PhotoAspect::Landscape16x9,
            ..Default::default()
        };
        assert_eq!(settings.dimensions(1200, 800), (3_840, 2_160));

        let portrait = PhotoSettings {
            resolution: PhotoResolution::K8,
            aspect: PhotoAspect::Portrait9x16,
            ..Default::default()
        };
        assert_eq!(portrait.dimensions(1200, 800), (4_320, 7_680));

        let current = PhotoSettings {
            resolution: PhotoResolution::K1,
            aspect: PhotoAspect::Current,
            ..Default::default()
        };
        assert_eq!(current.dimensions(1600, 1000), (1_024, 640));
    }
}
