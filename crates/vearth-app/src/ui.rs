//! In-app search, navigation, HUD, settings, and data-attribution overlays.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use vearth_data::geocode::{Geocoder, Place};
use vearth_gen::landmarks::{Base, CalibrationOverride, Landmark};

use crate::capture::{PhotoSettings, PhotoStatus};
use crate::local_time::LocalTimeHud;
#[cfg(not(target_arch = "wasm32"))]
use crate::settings::CACHE_LIMIT_OPTIONS_GB;
use crate::settings::{
    CacheUsage, PanSpeedPreference, PendingCacheClear, ShadowPreference, SolarSpeedPreference,
    UserPreferences, VehiclePreference, WeatherQualityPreference, WindPreference,
};
use crate::vehicles::VehicleKind;
use crate::weather::{PrecipitationKind, WeatherMode, WeatherQuality, WindInfluence};

const PLEX_SANS_REGULAR: &str = "IBM Plex Sans Regular";
const PLEX_SANS_MEDIUM: &str = "IBM Plex Sans Medium";
const PLEX_MONO_REGULAR: &str = "IBM Plex Mono Regular";
const NOTO_SANS_CJK: &str = crate::location_title::CJK_FONT;
const PHASE_MANTA_MOTION_LINE_COUNT: usize = 16;
const PHASE_MANTA_MOTION_BURST_S: f64 = 1.25;
const PHASE_MANTA_BOOST_SETTLED: f64 = 0.92;
const HUD_INK: egui::Color32 = egui::Color32::from_rgb(244, 240, 229);
const PANEL_INK: egui::Color32 = egui::Color32::from_rgb(198, 203, 197);

fn configure_game_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.window_margin = egui::Margin::same(18);
        style.animation_time = 0.12;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(14.0, plex_medium_family()),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(20.0, plex_medium_family()),
        );
        let visuals = &mut style.visuals;
        visuals.window_fill = egui::Color32::from_rgb(28, 34, 34);
        visuals.panel_fill = visuals.window_fill;
        visuals.window_stroke = egui::Stroke::NONE;
        visuals.window_corner_radius = egui::CornerRadius::same(2);
        visuals.menu_corner_radius = egui::CornerRadius::same(2);
        visuals.window_highlight_topmost = false;
        visuals.weak_text_color = Some(PANEL_INK);
        visuals.hyperlink_color = HUD_INK;
        visuals.selection.bg_fill = egui::Color32::from_rgb(74, 84, 80);
        visuals.selection.stroke = egui::Stroke::new(1.0, HUD_INK);
        visuals.extreme_bg_color = egui::Color32::from_rgb(17, 22, 23);
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = egui::CornerRadius::same(2);
            widget.bg_stroke = egui::Stroke::NONE;
            widget.fg_stroke = egui::Stroke::new(1.5, HUD_INK);
        }
        visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_white_alpha(24);
        visuals.widgets.active.weak_bg_fill = egui::Color32::from_white_alpha(40);
        visuals.widgets.noninteractive.fg_stroke.color = HUD_INK;
        visuals.widgets.noninteractive.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(30));
    });
}

fn hud_text(
    painter: &egui::Painter,
    position: egui::Pos2,
    galley: Arc<egui::Galley>,
    color: egui::Color32,
) {
    painter.galley_with_override_text_color(
        position + egui::vec2(0.0, 1.0),
        galley.clone(),
        egui::Color32::from_black_alpha(220),
    );
    painter.galley_with_override_text_color(position, galley, color);
}

fn hud_button(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::new(14.0, plex_medium_family()),
        HUD_INK,
    );
    let response = ui.add(
        egui::Button::new("")
            .frame(false)
            .min_size(egui::vec2(galley.size().x + 18.0, 34.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    let color = if selected || response.hovered() || response.has_focus() {
        HUD_INK
    } else {
        PANEL_INK
    };
    hud_text(
        ui.painter(),
        response.rect.center() - galley.size() * 0.5,
        galley,
        color,
    );
    if selected || response.has_focus() {
        let y = response.rect.bottom() - 2.0;
        ui.painter().line_segment(
            [
                egui::pos2(response.rect.left() + 9.0, y),
                egui::pos2(response.rect.right() - 9.0, y),
            ],
            egui::Stroke::new(2.0, HUD_INK),
        );
    }
    response
}

fn hud_scrim(rect: egui::Rect) -> egui::Shape {
    // Feather into the scene; the HUD has no visible panel edge, even over snow.
    let center = rect.center();
    let radii = rect.size() * 0.5 + egui::vec2(24.0, 17.0);
    let mut mesh = egui::Mesh::default();
    const SEGMENTS: u32 = 48;
    for (radius, alpha) in [(0.0, 65), (0.4, 40), (0.7, 10), (1.0, 0)] {
        for i in 0..SEGMENTS {
            let angle = i as f32 * std::f32::consts::TAU / SEGMENTS as f32;
            mesh.colored_vertex(
                center + egui::vec2(angle.cos(), angle.sin()) * radii * radius,
                egui::Color32::from_black_alpha(alpha),
            );
        }
    }
    for ring in 0..3 {
        for i in 0..SEGMENTS {
            let next = (i + 1) % SEGMENTS;
            let a = ring * SEGMENTS + i;
            let b = ring * SEGMENTS + next;
            mesh.add_triangle(a, b, a + SEGMENTS);
            mesh.add_triangle(b, b + SEGMENTS, a + SEGMENTS);
        }
    }
    egui::Shape::mesh(mesh)
}

fn show_solar_controls(
    ui: &mut egui::Ui,
    sun: &SunHud,
    local_time: Option<&LocalTimeHud>,
) -> UiAction {
    let mut action = UiAction::None;
    let mut backdrop = ui.painter().clone();
    backdrop.set_clip_rect(ui.ctx().content_rect());
    // Reserve the background's paint order before measuring the controls.
    let scrim = backdrop.add(egui::Shape::Noop);
    let mut controls_rect = egui::Rect::NOTHING;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
        ui.spacing_mut().interact_size.y = 34.0;
        let steps = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if hud_button(ui, "−1h", false)
                .on_hover_text("Move sunlight back one hour")
                .clicked()
            {
                action = UiAction::ShiftSunHours(-1.0);
            }
            let label = if sun.paused {
                "Resume solar cycle"
            } else {
                "Pause solar cycle"
            };
            if solar_icon_button(
                ui,
                if sun.paused {
                    SolarControlIcon::Play
                } else {
                    SolarControlIcon::Pause
                },
                label,
            )
            .on_hover_text(label)
            .clicked()
            {
                action = UiAction::ToggleSunPause;
            }
            if hud_button(ui, "+1h", false)
                .on_hover_text("Move sunlight forward one hour")
                .clicked()
            {
                action = UiAction::ShiftSunHours(1.0);
            }
        });
        controls_rect = controls_rect.union(steps.response.rect);
        ui.separator();
        let speeds = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for scale in [1.0, 60.0, 600.0] {
                if hud_button(
                    ui,
                    &format!("{scale:.0}×"),
                    (sun.time_scale - scale).abs() < f64::EPSILON,
                )
                .on_hover_text(format!("Run the solar cycle at {scale:.0}× speed"))
                .clicked()
                {
                    action = UiAction::SetSunTimeScale(scale);
                }
            }
        });
        controls_rect = controls_rect.union(speeds.response.rect);
        ui.separator();
        let label = local_time.map_or("--:-- LOCAL", |time| time.label.as_str());
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            egui::FontId::new(17.0, plex_medium_family()),
            HUD_INK,
        );
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(galley.size().x + 8.0, 34.0),
            egui::Sense::hover(),
        );
        controls_rect = controls_rect.union(rect);
        hud_text(
            ui.painter(),
            rect.center() - galley.size() * 0.5,
            galley,
            HUD_INK,
        );
        response.on_hover_text(
            local_time.map_or("Resolving local timezone…", |time| time.detail.as_str()),
        );
    });
    backdrop.set(scrim, hud_scrim(controls_rect));
    action
}

/// The interface and title fonts. Native builds embed every face. The
/// browser starts with the interface fonts and adds the CJK and title faces
/// as they are downloaded on demand (see `Ui::poll_fonts`).
fn vearth_font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        PLEX_SANS_REGULAR.to_owned(),
        Arc::new(egui::FontData::from_static(vearth_platform::asset_bytes!(
            "../assets/fonts/IBMPlexSans-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        PLEX_SANS_MEDIUM.to_owned(),
        Arc::new(egui::FontData::from_static(vearth_platform::asset_bytes!(
            "../assets/fonts/IBMPlexSans-Medium.ttf"
        ))),
    );
    fonts.font_data.insert(
        PLEX_MONO_REGULAR.to_owned(),
        Arc::new(egui::FontData::from_static(vearth_platform::asset_bytes!(
            "../assets/fonts/IBMPlexMono-Regular.ttf"
        ))),
    );
    #[cfg(not(target_arch = "wasm32"))]
    let cjk = true;
    #[cfg(target_arch = "wasm32")]
    let cjk = crate::location_title::face_loaded(crate::location_title::Face::Cjk);
    if cjk {
        fonts.font_data.insert(
            NOTO_SANS_CJK.to_owned(),
            Arc::new(egui::FontData::from_static(
                crate::location_title::cjk_bytes(),
            )),
        );
    }

    let proportional = fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .expect("egui default proportional family");
    proportional.insert(0, PLEX_SANS_REGULAR.to_owned());
    if cjk {
        proportional.push(NOTO_SANS_CJK.to_owned());
    }

    let monospace = fonts
        .families
        .get_mut(&egui::FontFamily::Monospace)
        .expect("egui default monospace family");
    monospace.insert(0, PLEX_MONO_REGULAR.to_owned());
    if cjk {
        monospace.push(NOTO_SANS_CJK.to_owned());
    }

    let mut medium_fallbacks = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .expect("egui default proportional family")
        .clone();
    medium_fallbacks.insert(0, PLEX_SANS_MEDIUM.to_owned());
    fonts
        .families
        .insert(plex_medium_family(), medium_fallbacks);
    crate::location_title::register_fonts(&mut fonts);
    fonts
}

pub(crate) fn plex_medium_family() -> egui::FontFamily {
    egui::FontFamily::Name(PLEX_SANS_MEDIUM.into())
}

/// Register bundled UI and arrival-title faces, including native-script coverage.
pub fn configure_fonts(ctx: &egui::Context) {
    ctx.set_fonts(vearth_font_definitions());
    ctx.all_styles_mut(|style| {
        if let Some(heading) = style.text_styles.get_mut(&egui::TextStyle::Heading) {
            heading.family = plex_medium_family();
        }
    });
}

pub enum UiAction {
    None,
    FlyTo(Place),
    ResetNorth,
    ToggleSunPause,
    ShiftSunHours(f64),
    SetSunTimeScale(f64),
    SetSunUtc(f64),
    ApplyLandmarkCalibration {
        value: CalibrationOverride,
        recenter: bool,
    },
    CapturePhoto(PhotoSettings),
    CaptureScreenshot,
    ExitPlay,
    ApplyPreferences(UserPreferences),
    ResetPreferences,
    RefreshCacheUsage,
    ScheduleCacheClear(PendingCacheClear),
    CancelCacheClear,
    /// Stop the app and have the page remove everything the site stored.
    #[cfg(target_arch = "wasm32")]
    ClearBrowserData,
}

struct DateEditor {
    date: String,
    source_utc: f64,
    error: Option<String>,
    focus: bool,
}

impl DateEditor {
    fn new(source_utc: f64) -> Self {
        Self {
            date: crate::solar::format_utc_date(source_utc),
            source_utc,
            error: None,
            focus: true,
        }
    }
}

struct CalibrationUi {
    landmark: &'static Landmark,
    east_m: f64,
    north_m: f64,
    up_m: f64,
    heading_deg: f64,
    move_step_m: f64,
    angle_step_deg: f64,
    copied_at: Option<vearth_platform::time::Instant>,
}

impl CalibrationUi {
    fn new(landmark: &'static Landmark) -> Self {
        Self {
            landmark,
            east_m: 0.0,
            north_m: 0.0,
            up_m: 0.0,
            heading_deg: landmark.heading_deg.rem_euclid(360.0),
            move_step_m: 1.0,
            angle_step_deg: 1.0,
            copied_at: None,
        }
    }

    fn value(&self) -> CalibrationOverride {
        CalibrationOverride {
            name: self.landmark.name,
            east_m: self.east_m,
            north_m: self.north_m,
            up_m: self.up_m,
            heading_deg: self.heading_deg.rem_euclid(360.0),
        }
    }

    fn select(&mut self, landmark: &'static Landmark) {
        *self = Self::new(landmark);
    }

    fn reset(&mut self) {
        let landmark = self.landmark;
        *self = Self::new(landmark);
    }

    fn clipboard_payload(&self) -> String {
        let value = self.value();
        let resolved = vearth_gen::landmarks::resolve_calibration(value);
        let source_base = match self.landmark.base {
            Base::Fixed(altitude_m) => serde_json::json!({
                "mode": "fixed",
                "altitudeM": altitude_m,
            }),
            Base::Terrain => serde_json::json!({
                "mode": "terrain",
            }),
            Base::TerrainOffset(offset_m) => serde_json::json!({
                "mode": "terrain",
                "offsetM": offset_m,
            }),
        };
        let resolved_base = match self.landmark.base {
            Base::Fixed(altitude_m) => serde_json::json!({
                "mode": "fixed",
                "altitudeM": f64::from(altitude_m) + resolved.base_offset_m,
                "offsetM": resolved.base_offset_m,
            }),
            Base::Terrain => serde_json::json!({
                "mode": "terrain",
                "offsetM": resolved.base_offset_m,
            }),
            Base::TerrainOffset(_) => serde_json::json!({
                "mode": "terrain",
                "offsetM": resolved.base_offset_m,
            }),
        };
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "vearth.landmark-alignment/v1",
            "landmark": self.landmark.name,
            "sourcePlacement": {
                "latDeg": self.landmark.lat_deg,
                "lonDeg": self.landmark.lon_deg,
                "headingDeg": self.landmark.heading_deg,
                "base": source_base,
            },
            "placement": {
                "latDeg": resolved.lat_deg,
                "lonDeg": resolved.lon_deg,
                "headingDeg": resolved.heading_deg,
                "base": resolved_base,
            },
            "editorOffsetEnuM": {
                "east": value.east_m,
                "north": value.north_m,
                "up": value.up_m,
            },
            "conventions": {
                "frame": "local ENU at sourcePlacement anchor",
                "modelAxes": { "x": "east at heading 0", "y": "up", "z": "north at heading 0" },
                "heading": "degrees clockwise from true north; model +Z points along heading",
                "angleRange": "[0, 360)",
            },
        }))
        .expect("landmark calibration payload is serializable")
    }
}

/// Camera/streaming numbers the HUD displays.
pub struct HudInfo<'a> {
    /// When the page last became visible; `None` while it is hidden.
    pub visible_since: Option<vearth_platform::time::Instant>,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub alt_m: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub fps: f64,
    pub resident_cells: usize,
    /// Cells wanted by the view, waiting for a worker.
    pub queued: usize,
    /// Cells a worker is currently generating (or landing).
    pub generating: usize,
    /// Share of the last stats window generated cold (no chunk-cache hit);
    /// None when nothing generated recently.
    pub cold_pct: Option<u32>,
    /// Non-nominal provider health notes; empty = ok.
    pub provider_status: Vec<String>,
    pub bortle_class: u8,
    pub limiting_magnitude: f32,
    /// Terrain evidence granting mountain refinement; L switches it here.
    pub lod_ruggedness: &'static str,
    pub local_time: Option<&'a LocalTimeHud>,
    pub sun: SunHud,
    pub weather: WeatherHud,
    pub photo: PhotoStatus,
    /// Set while play mode is active.
    pub play: Option<PlayHud>,
    /// Transient, bottom-centre geographic arrival title.
    pub location_toast: Option<crate::location::LocationToast>,
    pub preferences: &'a UserPreferences,
    pub provider_controls: &'a [vearth_data::registry::ProviderControl],
    pub storage: StorageHud<'a>,
}

#[derive(Clone, Copy)]
pub struct StorageHud<'a> {
    pub cache_path: &'a std::path::Path,
    pub usage: Option<CacheUsage>,
    pub scanning: bool,
    pub error: Option<&'a str>,
    pub active_limit_gb: u64,
    pub pending_clear: Option<PendingCacheClear>,
}

pub struct SunHud {
    pub time_scale: f64,
    pub paused: bool,
}

pub struct PlayHud {
    pub vehicle: VehicleKind,
    pub vehicle_name: &'static str,
    pub phased: bool,
    pub speed_ms: f64,
    pub airspeed_ms: f64,
    pub altitude_m: f64,
    pub boost: bool,
    pub boost_level: f64,
}

pub struct WeatherHud {
    pub mode: WeatherMode,
    pub quality: WeatherQuality,
    pub wind_influence: WindInfluence,
    pub seed: u64,
    pub preset: u64,
    pub atlas_version: u32,
    pub model_version: u32,
    pub tick0: i64,
    pub tick1: i64,
    pub tick_alpha: f32,
    pub temperature_c: f32,
    pub humidity: f32,
    pub precipitation_mm_h: f32,
    pub precipitation_wet_fraction: f32,
    pub precipitation_conditional_mm_h: f32,
    pub precipitation_kind: PrecipitationKind,
    pub cloud_coverage: f32,
    pub low_cloud_coverage: f32,
    pub mid_cloud_coverage: f32,
    pub high_cloud_coverage: f32,
    pub cloud_optical_depth: f32,
    pub aerosol: f32,
    pub aerosol_dust_fraction: f32,
    pub visibility_m: f32,
    pub wind_east_mps: f32,
    pub wind_north_mps: f32,
    pub wind_low: [f32; 2],
    pub wind_mid: [f32; 2],
    pub wind_upper: [f32; 2],
    pub wetness: f32,
    pub snow: f32,
    pub sea_state: f32,
}

/// Progress within one uninterrupted loading session. The largest observed
/// remaining count is the session total; reaching zero ends and resets it.
#[derive(Default)]
struct CellLoadProgress {
    total: usize,
}

impl CellLoadProgress {
    fn update(&mut self, remaining: usize) -> Option<f32> {
        if remaining == 0 {
            self.total = 0;
            return None;
        }
        self.total = self.total.max(remaining);
        Some(1.0 - remaining as f32 / self.total as f32)
    }
}

impl HudInfo<'_> {
    /// The shareable deep link for the current view (same format the
    /// `--at` flag parses).
    pub fn deep_link(&self) -> String {
        let mut link = format!(
            "{:.5},{:.5},{:.0},{:.0},{:.2}",
            self.lat_deg, self.lon_deg, self.alt_m, self.heading_deg, self.pitch_deg
        );
        if self.weather.mode != WeatherMode::Procedural {
            match self.weather.mode {
                WeatherMode::Clear => link.push_str(" --weather clear"),
                WeatherMode::Preset => {
                    link.push_str(&format!(" --weather preset:#{:016x}", self.weather.preset));
                }
                WeatherMode::Procedural => {}
            }
        }
        if self.weather.seed != crate::weather::DEFAULT_WEATHER_SEED {
            link.push_str(&format!(" --weather-seed {}", self.weather.seed));
        }
        link
    }
}

/// Live search debounce: a lookup fires this long after the last keystroke.
const SEARCH_DEBOUNCE: vearth_platform::time::Duration =
    vearth_platform::time::Duration::from_millis(350);
/// Minimum query length for search-as-you-type (Enter searches anything).
const LIVE_MIN_CHARS: usize = 3;

/// Minimal UI used while the real provider registry and generation pool are
/// assembled off the event-loop thread.
pub struct LaunchUi {
    ctx: egui::Context,
    input: crate::egui_input::EguiInput,
    renderer: egui_wgpu::Renderer,
    pending_textures: PendingTextureDeltas,
}

impl LaunchUi {
    pub fn new(
        window: &crate::host::Window,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> Self {
        let ctx = egui::Context::default();
        configure_fonts(&ctx);
        let input = crate::egui_input::EguiInput::new(ctx.clone(), window);
        Self {
            ctx,
            input,
            renderer: egui_wgpu::Renderer::new(
                device,
                format,
                egui_wgpu::RendererOptions::default(),
            ),
            pending_textures: PendingTextureDeltas::default(),
        }
    }

    pub fn run(
        &mut self,
        window: &crate::host::Window,
        elapsed: vearth_platform::time::Duration,
        failure: Option<&str>,
    ) -> UiPaint {
        let input = self.input.take_egui_input(window);
        let output = self.ctx.run_ui(input, |ctx| {
            ctx.request_repaint_after(vearth_platform::time::Duration::from_millis(16));
            show_controls_screen(ctx, elapsed.as_secs_f32(), failure, false);
        });
        self.input
            .handle_platform_output(window, output.platform_output);
        self.pending_textures.append(output.textures_delta);
        UiPaint {
            primitives: self.ctx.tessellate(output.shapes, output.pixels_per_point),
            pixels_per_point: output.pixels_per_point,
        }
    }

    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        frame: UiPaint,
    ) {
        paint_egui(
            &mut self.renderer,
            &mut self.pending_textures,
            device,
            queue,
            target,
            width,
            height,
            frame,
        );
    }
}

fn show_controls_screen(ctx: &mut egui::Ui, elapsed: f32, failure: Option<&str>, resumable: bool) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(8, 11, 11));

            let wordmark_alpha = launch_ease((elapsed - 0.05) / 0.62);
            let panel_alpha = launch_ease((elapsed - 0.28) / 0.62);
            let safe_width = (rect.width() - 48.0).max(240.0);
            let wordmark_width = (rect.width() * 0.50).clamp(310.0, 620.0).min(safe_width);
            let wordmark_size = egui::vec2(wordmark_width, wordmark_width * 0.45);
            let panel_width = (rect.width() * 0.54).clamp(440.0, 660.0).min(safe_width);
            let panel_height = 278.0;
            let gap = 22.0;
            let content_height = wordmark_size.y + gap + panel_height;
            let content_top = rect.center().y - content_height * 0.5;

            let wordmark_rect = egui::Rect::from_center_size(
                egui::pos2(
                    rect.center().x,
                    content_top + wordmark_size.y * 0.5 + (1.0 - wordmark_alpha) * 14.0,
                ),
                wordmark_size,
            );
            paint_launch_wordmark(painter, wordmark_rect, wordmark_alpha);

            let panel_top = content_top + wordmark_size.y + gap + (1.0 - panel_alpha) * 18.0;
            let panel = egui::Rect::from_min_size(
                egui::pos2(rect.center().x - panel_width * 0.5, panel_top),
                egui::vec2(panel_width, panel_height),
            );
            paint_launch_controls(painter, panel, elapsed, panel_alpha, failure);
        });
    if resumable {
        let rect = ctx.content_rect();
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("controls resume hint"),
        ))
        .text(
            egui::pos2(rect.center().x, rect.bottom() - 24.0),
            egui::Align2::CENTER_BOTTOM,
            "ESC — RETURN TO EXPLORING",
            egui::FontId::monospace(11.0),
            egui::Color32::from_rgb(167, 169, 159),
        );
    }
}

fn paint_launch_wordmark(painter: &egui::Painter, rect: egui::Rect, alpha: f32) {
    let texture = crate::launch_wordmark::texture(painter.ctx());
    painter.image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        egui::Color32::from_white_alpha((alpha * 255.0) as u8),
    );
}

fn launch_ease(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    1.0 - (1.0 - value).powi(3)
}

fn launch_color(rgb: [u8; 3], alpha: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        rgb[0],
        rgb[1],
        rgb[2],
        (alpha.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

fn launch_loading_pulse(panel: egui::Rect, elapsed: f32) -> egui::Rect {
    let pulse_width = panel.width() * 0.18;
    let travel = (elapsed * 0.32).fract();
    let pulse_left = panel.left() + (panel.width() + pulse_width) * travel - pulse_width;
    egui::Rect::from_min_max(
        egui::pos2(pulse_left.max(panel.left()), panel.top()),
        egui::pos2(
            (pulse_left + pulse_width).min(panel.right()),
            panel.top() + 2.0,
        ),
    )
}

fn paint_launch_controls(
    painter: &egui::Painter,
    panel: egui::Rect,
    elapsed: f32,
    alpha: f32,
    failure: Option<&str>,
) {
    let paper = launch_color([236, 233, 223], alpha);
    let line = launch_color([236, 233, 223], alpha * 0.18);
    let orange = launch_color([240, 107, 50], alpha);
    painter.rect_filled(panel, 0.0, launch_color([14, 18, 17], alpha * 0.96));
    painter.rect_stroke(
        panel,
        0.0,
        egui::Stroke::new(1.0, line),
        egui::StrokeKind::Inside,
    );
    let pulse = launch_loading_pulse(panel, elapsed);
    if pulse.is_positive() {
        painter.rect_filled(pulse, 0.0, orange);
    }

    painter.text(
        panel.left_top() + egui::vec2(22.0, 21.0),
        egui::Align2::LEFT_TOP,
        if failure.is_some() {
            "STARTUP FAILED"
        } else {
            "FLIGHT CONTROLS"
        },
        egui::FontId::monospace(11.0),
        if failure.is_some() {
            launch_color([255, 126, 126], alpha)
        } else {
            orange
        },
    );
    painter.line_segment(
        [
            egui::pos2(panel.left() + 22.0, panel.top() + 50.0),
            egui::pos2(panel.right() - 22.0, panel.top() + 50.0),
        ],
        egui::Stroke::new(1.0, line),
    );

    if let Some(error) = failure {
        painter.text(
            panel.left_top() + egui::vec2(22.0, 76.0),
            egui::Align2::LEFT_TOP,
            error,
            egui::FontId::monospace(12.0),
            paper,
        );
        return;
    }

    let gutter = 22.0;
    let column_gap = 24.0;
    let column_width = (panel.width() - gutter * 2.0 - column_gap) * 0.5;
    let left = panel.left() + gutter;
    let right = left + column_width + column_gap;
    paint_launch_control(
        painter,
        egui::pos2(left, panel.top() + 70.0),
        "W  A  S  D",
        "PAN / FLY",
        alpha,
    );
    paint_launch_control(
        painter,
        egui::pos2(left, panel.top() + 134.0),
        if crate::host::command_modifier_is_cmd() {
            "CMD + DRAG"
        } else {
            "CTRL + DRAG"
        },
        "TILT / ROTATE",
        alpha,
    );
    paint_launch_control(
        painter,
        egui::pos2(right, panel.top() + 70.0),
        "P",
        "SPAWN DRONE",
        alpha,
    );
    paint_launch_control(
        painter,
        egui::pos2(right, panel.top() + 134.0),
        "SHIFT",
        "DRONE BOOST",
        alpha,
    );
    paint_launch_control(
        painter,
        egui::pos2(left, panel.top() + 198.0),
        "ENTER",
        "WORLD MAP",
        alpha,
    );
    paint_launch_control(
        painter,
        egui::pos2(right, panel.top() + 198.0),
        "V",
        "CYCLE VEHICLE",
        alpha,
    );
    painter.text(
        egui::pos2(panel.center().x, panel.top() + 246.0),
        egui::Align2::CENTER_CENTER,
        "1 KESTREL  ·  2 PHASE MANTA  ·  3 HOVER BIKE  ·  T LOCATION TITLE",
        egui::FontId::monospace(9.0),
        launch_color([167, 169, 159], alpha),
    );
}

fn paint_launch_control(
    painter: &egui::Painter,
    origin: egui::Pos2,
    key: &str,
    action: &str,
    alpha: f32,
) {
    let key_rect = egui::Rect::from_min_size(origin, egui::vec2(112.0, 32.0));
    painter.rect_stroke(
        key_rect,
        0.0,
        egui::Stroke::new(1.0, launch_color([236, 233, 223], alpha * 0.24)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        key_rect.center(),
        egui::Align2::CENTER_CENTER,
        key,
        egui::FontId::monospace(11.0),
        launch_color([242, 239, 230], alpha),
    );
    painter.text(
        egui::pos2(origin.x + 126.0, origin.y + 16.0),
        egui::Align2::LEFT_CENTER,
        action,
        egui::FontId::monospace(10.0),
        launch_color([167, 169, 159], alpha),
    );
}

fn format_storage_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / GIB)
    } else {
        format!("{:.1} MB", bytes as f64 / MIB)
    }
}

fn settings_combo<T: Copy + Eq>(
    ui: &mut egui::Ui,
    id: &'static str,
    label: &str,
    value: &mut T,
    options: impl IntoIterator<Item = T>,
    option_label: impl Fn(T) -> &'static str,
) -> bool {
    let mut changed = false;
    let row = ui.horizontal(|ui| {
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            egui::vec2(width * 0.52, 48.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(label);
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::ComboBox::from_id_salt(id)
                .width((width * 0.48 - 10.0).max(80.0))
                .selected_text(option_label(*value))
                .show_ui(ui, |ui| {
                    for option in options {
                        changed |= ui
                            .selectable_value(value, option, option_label(option))
                            .changed();
                    }
                });
        });
    });
    ui.painter().hline(
        row.response.rect.x_range(),
        row.response.rect.bottom(),
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(22)),
    );
    changed
}

fn settings_toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    let galley = ui.painter().layout(
        label.to_owned(),
        egui::FontId::proportional(14.0),
        HUD_INK,
        (ui.available_width() - 36.0).max(80.0),
    );
    let mut response = ui.add(egui::Button::new("").frame(false).min_size(egui::vec2(
        ui.available_width(),
        48.0_f32.max(galley.size().y + 20.0),
    )));
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label)
    });
    let painter = ui.painter();
    painter.galley(
        egui::pos2(
            response.rect.left(),
            response.rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        HUD_INK,
    );
    let box_rect = egui::Rect::from_center_size(
        egui::pos2(response.rect.right() - 8.0, response.rect.center().y),
        egui::vec2(15.0, 15.0),
    );
    painter.rect(
        box_rect,
        1.0,
        if *value {
            HUD_INK
        } else {
            egui::Color32::TRANSPARENT
        },
        egui::Stroke::new(1.0, PANEL_INK),
        egui::StrokeKind::Inside,
    );
    if *value {
        painter.add(egui::Shape::line(
            vec![
                box_rect.center() + egui::vec2(-4.0, 0.0),
                box_rect.center() + egui::vec2(-1.0, 3.0),
                box_rect.center() + egui::vec2(4.0, -3.0),
            ],
            egui::Stroke::new(1.6, egui::Color32::from_rgb(28, 34, 34)),
        ));
    }
    painter.hline(
        response.rect.x_range(),
        response.rect.bottom(),
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(22)),
    );
    response
}

fn panel_heading(ui: &mut egui::Ui, title: &str) -> bool {
    ui.painter().hline(
        (ui.cursor().min.x - 24.0)..=(ui.cursor().min.x + ui.available_width() + 24.0),
        ui.cursor().min.y - 20.0,
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(232, 229, 209, 140),
        ),
    );
    ui.horizontal(|ui| {
        ui.heading(title.to_uppercase());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = ui.add(
                egui::Button::new("")
                    .frame(false)
                    .min_size(egui::vec2(28.0, 28.0)),
            );
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
            paint_hud_icon(ui, response.rect.center(), HudIcon::Close, 16.0);
            response.on_hover_text("Close").clicked()
        })
        .inner
    })
    .inner
}

fn settings_tab(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::new(13.0, plex_medium_family()),
        if selected { HUD_INK } else { PANEL_INK },
    );
    let response = ui.add(
        egui::Button::new("")
            .frame(false)
            .min_size(egui::vec2(galley.size().x, 38.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    ui.painter().galley(
        response.rect.center() - galley.size() * 0.5,
        galley,
        HUD_INK,
    );
    if selected || response.has_focus() {
        ui.painter().hline(
            response.rect.x_range(),
            response.rect.bottom(),
            egui::Stroke::new(2.0, HUD_INK),
        );
    }
    response
}

fn game_panel_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(28, 34, 34))
        .inner_margin(egui::Margin::symmetric(24, 20))
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 32,
            spread: 0,
            color: egui::Color32::from_black_alpha(64),
        })
}

fn show_settings_contents(
    ui: &mut egui::Ui,
    preferences: &UserPreferences,
    providers: &[vearth_data::registry::ProviderControl],
    storage: StorageHud<'_>,
    cache_clear_confirmation: &mut Option<PendingCacheClear>,
    reset_confirmation: &mut bool,
    notice: Option<&str>,
    show_privacy: &mut bool,
    photo: &PhotoStatus,
) -> Option<UiAction> {
    let mut next = preferences.clone();
    let mut changed = false;
    let mut action = None;

    let tab_id = ui.make_persistent_id("settings category");
    let mut selected_tab = ui.data_mut(|data| data.get_temp::<usize>(tab_id).unwrap_or(0));
    ui.spacing_mut().item_spacing.y = 0.0;
    let tabs = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        for (index, label) in ["Interface", "World", "Flight", "Data", "Photos", "Storage"]
            .into_iter()
            .enumerate()
        {
            if settings_tab(ui, label, selected_tab == index).clicked() {
                selected_tab = index;
            }
        }
    });
    ui.data_mut(|data| data.insert_temp(tab_id, selected_tab));
    ui.painter().hline(
        tabs.response.rect.x_range(),
        tabs.response.rect.bottom(),
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(40)),
    );
    if let Some(notice) = notice {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(notice).color(egui::Color32::from_rgb(198, 232, 93)));
    }

    ui.add_space(10.0);
    if selected_tab == 0 {
        changed |= settings_toggle(ui, &mut next.show_search, "Search control").changed();
        changed |= settings_toggle(ui, &mut next.show_solar, "Solar controls").changed();
        changed |= settings_toggle(ui, &mut next.show_compass, "Compass").changed();
        changed |= settings_toggle(ui, &mut next.show_loading, "Loading progress").changed();
        changed |= settings_toggle(ui, &mut next.show_attribution, "Data attribution").changed();
        changed |= settings_toggle(ui, &mut next.show_debug, "Debug panel").changed();
        changed |= settings_toggle(ui, &mut next.twenty_four_hour, "Use 24-hour time").changed();
    }

    if selected_tab == 1 {
        changed |= settings_combo(
            ui,
            "solar speed setting",
            "Solar cycle speed",
            &mut next.solar_speed,
            SolarSpeedPreference::ALL,
            SolarSpeedPreference::label,
        );
        changed |= settings_toggle(ui, &mut next.weather_enabled, "Weather system").changed();
        changed |= settings_toggle(ui, &mut next.clouds_visible, "Clouds").changed();
        changed |=
            settings_toggle(ui, &mut next.precipitation_visible, "Visible precipitation").changed();
        changed |= settings_toggle(
            ui,
            &mut next.precipitation_camera_effects,
            "Precipitation camera effects",
        )
        .changed();
        changed |=
            settings_toggle(ui, &mut next.building_night_lights, "Building night lights").changed();
        changed |= settings_combo(
            ui,
            "shadows setting",
            "Shadows",
            &mut next.shadows,
            ShadowPreference::ALL,
            ShadowPreference::label,
        );
        changed |= settings_toggle(ui, &mut next.color_grading, "Color grading").changed();
        changed |= settings_toggle(ui, &mut next.filmic_saturation, "Filmic saturation").changed();
        changed |= settings_combo(
            ui,
            "render resolution setting",
            "Render resolution",
            &mut next.render_resolution,
            crate::settings::RenderResolutionPreference::ALL,
            crate::settings::RenderResolutionPreference::label,
        );
        changed |= settings_combo(
            ui,
            "weather quality setting",
            "Weather quality",
            &mut next.weather_quality,
            WeatherQualityPreference::ALL,
            WeatherQualityPreference::label,
        );
        changed |= settings_combo(
            ui,
            "wind influence setting",
            "Wind influence",
            &mut next.wind_influence,
            WindPreference::ALL,
            WindPreference::label,
        );
    }

    if selected_tab == 2 {
        changed |= settings_combo(
            ui,
            "pan speed setting",
            "WASD pan speed",
            &mut next.pan_speed,
            PanSpeedPreference::ALL,
            PanSpeedPreference::label,
        );
        changed |= settings_combo(
            ui,
            "default vehicle setting",
            "Default vehicle",
            &mut next.selected_vehicle,
            VehiclePreference::ALL,
            VehiclePreference::label,
        );
    }

    if selected_tab == 3 {
        ui.label(
            egui::RichText::new(
                "Online providers receive your IP address and requests identifying explored areas. See the Privacy Policy for details.",
            )
            .weak()
            .size(11.0),
        );
        for layer in [
            vearth_data::registry::ProviderLayer::Elevation,
            vearth_data::registry::ProviderLayer::Surface,
            vearth_data::registry::ProviderLayer::Imagery,
            vearth_data::registry::ProviderLayer::Vectors,
        ] {
            let layer_providers: Vec<_> = providers
                .iter()
                .filter(|provider| provider.id.layer == layer)
                .collect();
            if layer_providers.is_empty() {
                continue;
            }
            ui.add_space(5.0);
            ui.label(egui::RichText::new(layer.label()).strong().size(11.0));
            for provider in layer_providers {
                let mut enabled = provider.enabled;
                if settings_toggle(ui, &mut enabled, &provider.id.name).changed() {
                    next.set_provider(&provider.id, enabled);
                    changed = true;
                }
            }
        }
    }

    if selected_tab == 4 {
        ui.label("Save a 4K image and scene recipe to Pictures/Low Poly Earth.");
        ui.label("The recipe includes the scene's geographic position.");
        if ui
            .add_enabled(!photo.busy(), egui::Button::new("Save photo"))
            .clicked()
        {
            action = Some(UiAction::CapturePhoto(PhotoSettings::default()));
        }
        match photo {
            PhotoStatus::Idle => {}
            PhotoStatus::Preparing { remaining, .. } => {
                ui.label(format!("Preparing scene: {remaining} cells remaining…"));
            }
            PhotoStatus::Capturing { width, height } => {
                ui.label(format!("Saving {width} × {height} photo…"));
            }
            PhotoStatus::Saved(artifact) => {
                ui.label(format!("Saved {}", artifact.image_path.display()));
            }
            PhotoStatus::Failed(error) => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("Could not save photo: {error}"),
                );
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    if selected_tab == 5 {
        action = browser_storage_tab(ui, &storage, cache_clear_confirmation);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if selected_tab == 5 {
        match storage.usage {
            Some(usage) => {
                ui.label(format!(
                    "Downloaded data: {} across {} files",
                    format_storage_bytes(usage.bytes),
                    usage.files
                ));
            }
            None if storage.scanning => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Calculating downloaded data…");
                });
            }
            None => {
                ui.label("Downloaded data size is not available.");
            }
        }
        if let Some(error) = storage.error {
            ui.label(egui::RichText::new(error).weak().size(10.0));
        }
        ui.horizontal(|ui| {
            ui.label("Storage limit");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("cache storage limit")
                    .selected_text(format!("{} GB", next.cache_limit_gb))
                    .show_ui(ui, |ui| {
                        for limit in CACHE_LIMIT_OPTIONS_GB {
                            changed |= ui
                                .selectable_value(
                                    &mut next.cache_limit_gb,
                                    limit,
                                    format!("{limit} GB"),
                                )
                                .changed();
                        }
                    });
            });
        });
        if next.cache_limit_gb != storage.active_limit_gb {
            ui.label(
                egui::RichText::new("The new storage limit takes effect after restart.")
                    .weak()
                    .size(10.0),
            );
        }
        ui.label(
            egui::RichText::new(storage.cache_path.display().to_string())
                .monospace()
                .weak()
                .size(9.0),
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!storage.scanning, egui::Button::new("Refresh size"))
                .clicked()
            {
                action = Some(UiAction::RefreshCacheUsage);
            }
            if ui.button("Clear source downloads…").clicked() {
                *cache_clear_confirmation = Some(PendingCacheClear {
                    source_downloads: true,
                    generated_terrain: false,
                });
            }
            if ui.button("Clear generated terrain…").clicked() {
                *cache_clear_confirmation = Some(PendingCacheClear {
                    source_downloads: false,
                    generated_terrain: true,
                });
            }
            if ui.button("Clear everything…").clicked() {
                *cache_clear_confirmation = Some(PendingCacheClear::all());
            }
        });
        ui.label(
            egui::RichText::new(
                "Cleared data is downloaded or regenerated as needed. Photos and settings are never removed.",
            )
            .weak()
            .size(10.0),
        );
        if let Some(pending) = storage.pending_clear {
            ui.add_space(5.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Scheduled for next launch: {}", pending.label()));
                if ui.small_button("Cancel").clicked() {
                    action = Some(UiAction::CancelCacheClear);
                }
            });
        }
    }

    ui.add_space(12.0);
    ui.label(
        egui::RichText::new("Changes are saved automatically.")
            .weak()
            .size(12.0),
    );
    if ui.link("Privacy Policy").clicked() {
        *show_privacy = true;
    }
    ui.add_space(12.0);
    ui.separator();
    if !*reset_confirmation {
        if ui.button("Reset all settings…").clicked() {
            *reset_confirmation = true;
        }
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.label("Restore every setting and data-source choice to its default?");
            if ui.button("Reset settings").clicked() {
                *reset_confirmation = false;
                action = Some(UiAction::ResetPreferences);
            }
            if ui.button("Cancel").clicked() {
                *reset_confirmation = false;
            }
        });
    }

    if changed && action.is_none() {
        action = Some(UiAction::ApplyPreferences(next));
    }
    action
}

/// The browser's Storage tab. The browser manages the site's storage quota,
/// and a running page cannot remove its own HTTP cache or files the app has
/// open, so the one action stops the app and has the page clear the site.
#[cfg(target_arch = "wasm32")]
fn browser_storage_tab(
    ui: &mut egui::Ui,
    storage: &StorageHud<'_>,
    confirmation: &mut Option<PendingCacheClear>,
) -> Option<UiAction> {
    let mut action = None;
    // Saved data grows as terrain is generated; re-read it while visible.
    let refreshed_id = ui.make_persistent_id("storage usage refreshed");
    let now = ui.input(|input| input.time);
    let refreshed = ui.data(|data| data.get_temp::<f64>(refreshed_id));
    if !storage.scanning && refreshed.is_none_or(|at| now - at >= 5.0) {
        ui.data_mut(|data| data.insert_temp(refreshed_id, now));
        action = Some(UiAction::RefreshCacheUsage);
    }
    let saved = match storage.usage {
        Some(usage) => format_storage_bytes(usage.bytes),
        None if storage.error.is_some() => "Unavailable".into(),
        None => "…".into(),
    };
    let row = ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width() * 0.52, 48.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label("Saved data");
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(saved);
        });
    });
    ui.painter().hline(
        row.response.rect.x_range(),
        row.response.rect.bottom(),
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(22)),
    );
    ui.add_space(12.0);
    if outlined_button(ui, "Clear all data and exit…").clicked() {
        *confirmation = Some(PendingCacheClear::all());
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(
            "Removes saved terrain, map data and settings from this browser. Elevation tiles stay in the browser's cache until it clears them.",
        )
        .weak()
        .size(11.0),
    );
    action
}

/// A bordered button, for a consequential action among the panel's
/// frameless controls.
#[cfg(target_arch = "wasm32")]
fn outlined_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
        ui.add(
            egui::Button::new(text)
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(120))),
        )
    })
    .inner
}

pub struct Ui {
    pub ctx: egui::Context,
    input: crate::egui_input::EguiInput,
    renderer: egui_wgpu::Renderer,
    /// Texture changes outlive an individual swapchain frame. Surface
    /// acquisition can legitimately fail while a window is occluded or being
    /// resized; retaining the deltas ensures the renderer still receives the
    /// initial font-atlas allocation before any later partial update.
    pending_textures: PendingTextureDeltas,
    query: String,
    results: Vec<Place>,
    status: Option<String>,
    searching: bool,
    /// Monotonic id of the newest kicked search; stale results are ignored.
    search_seq: u64,
    /// Query text of the newest kicked search (dedup for the debouncer).
    submitted: Option<String>,
    /// Latest edit while a search was in flight; kicked on completion.
    pending_next: Option<String>,
    last_edit: Option<vearth_platform::time::Instant>,
    tx: Sender<(u64, Result<Vec<Place>, String>)>,
    rx: Receiver<(u64, Result<Vec<Place>, String>)>,
    /// Opened on first use, on a worker: the browser downloads the index.
    geocoder: Arc<std::sync::OnceLock<Geocoder>>,
    focus_search: bool,
    search_has_focus: bool,
    pending_paste: Option<String>,
    hide_panels: bool,
    show_controls: bool,
    show_search: bool,
    search_expanded: bool,
    show_debug: bool,
    show_solar: bool,
    show_compass: bool,
    screenshot_notice: Option<String>,
    show_map: bool,
    map_renderer: crate::map::MapRenderer,
    map_navigation: crate::map::MapNavigation,
    map_places: Option<crate::map::MapPlaceCatalog>,
    map_places_rx: Option<Receiver<Result<crate::map::MapPlaceCatalog, String>>>,
    map_places_requested: bool,
    show_loading: bool,
    show_attribution: bool,
    show_settings: bool,
    show_privacy: bool,
    show_settings_button: bool,
    cache_clear_confirmation: Option<PendingCacheClear>,
    reset_confirmation: bool,
    settings_notice: Option<String>,
    date_editor: Option<DateEditor>,
    loading_progress: CellLoadProgress,
    phase_manta_motion_burst: PhaseMantaMotionBurst,
    attributions: Vec<vearth_data::registry::AttributionEntry>,
    calibration: Option<CalibrationUi>,
    #[cfg(target_arch = "wasm32")]
    web_fonts: WebFonts,
    performance: PerformanceAdvisor,
}

/// Frame-rate watch: after the first view has loaded, a frame rate below
/// [`PerformanceAdvisor::SLOW_FPS`] for [`PerformanceAdvisor::SLOW_FOR`]
/// offers a lower render resolution, or says the computer is too slow once
/// none is left. Time just after the window becomes visible is ignored,
/// since the frame-rate window then still spans the hidden period.
#[derive(Default)]
struct PerformanceAdvisor {
    first_view_loaded: bool,
    slow_since: Option<vearth_platform::time::Instant>,
    advice: Option<PerformanceAdvice>,
    dismissed: bool,
}

#[derive(Clone)]
struct PerformanceAdvice {
    fps: f64,
    /// The next lower render resolution that renders fewer pixels, if any.
    lower: Option<crate::settings::RenderResolutionPreference>,
}

enum PerformanceChoice {
    Lower(crate::settings::RenderResolutionPreference),
    Dismiss,
}

impl PerformanceAdvisor {
    const SLOW_FPS: f64 = 20.0;
    const SLOW_FOR: vearth_platform::time::Duration =
        vearth_platform::time::Duration::from_secs(10);
    const VISIBLE_GRACE: vearth_platform::time::Duration =
        vearth_platform::time::Duration::from_secs(5);

    /// `pixel_ratio` is the pixels per point the app renders at.
    fn update(
        &mut self,
        hud: &HudInfo<'_>,
        first_view_loaded: bool,
        pixel_ratio: f64,
    ) -> Option<PerformanceAdvice> {
        let now = vearth_platform::time::Instant::now();
        self.first_view_loaded |= first_view_loaded;
        let measuring = self.first_view_loaded
            && hud
                .visible_since
                .is_some_and(|since| now.duration_since(since) >= Self::VISIBLE_GRACE)
            && hud.fps > 0.0;
        if !measuring || hud.fps >= Self::SLOW_FPS {
            self.slow_since = None;
        } else if !self.dismissed
            && self.advice.is_none()
            && now.duration_since(*self.slow_since.get_or_insert(now)) >= Self::SLOW_FOR
        {
            use crate::settings::RenderResolutionPreference as Resolution;
            let current = hud.preferences.render_resolution;
            let lower = Resolution::ALL
                .into_iter()
                .skip_while(|resolution| *resolution != current)
                .skip(1)
                .find(|resolution| {
                    resolution
                        .max_pixel_ratio()
                        .is_some_and(|cap| cap < pixel_ratio - 0.01)
                });
            self.advice = Some(PerformanceAdvice {
                fps: hud.fps,
                lower,
            });
        }
        self.advice.clone()
    }

    /// A lower resolution was applied; watch again at the new one.
    fn handled(&mut self) {
        self.advice = None;
        self.slow_since = None;
    }

    fn dismiss(&mut self) {
        self.advice = None;
        self.dismissed = true;
    }
}

fn show_performance_advice(
    ctx: &egui::Context,
    advice: &PerformanceAdvice,
) -> Option<PerformanceChoice> {
    let mut choice = None;
    egui::Area::new(egui::Id::new("performance advice"))
        .anchor(egui::Align2::CENTER_TOP, [0.0, 20.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_black_alpha(190))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_max_width(360.0);
                    let text = match advice.lower {
                        Some(_) => format!(
                            "Low Poly Earth is running slowly here ({:.0} fps). A lower \
                             render resolution draws fewer pixels.",
                            advice.fps
                        ),
                        None => format!(
                            "This computer's graphics are too slow to run Low Poly Earth \
                             smoothly ({:.0} fps), even at the lowest render resolution.",
                            advice.fps
                        ),
                    };
                    ui.label(
                        egui::RichText::new(text)
                            .size(12.5)
                            .color(egui::Color32::from_white_alpha(225)),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if let Some(lower) = advice.lower
                            && ui.button(format!("Use {}", lower.label())).clicked()
                        {
                            choice = Some(PerformanceChoice::Lower(lower));
                        }
                        if ui.button("Dismiss").clicked() {
                            choice = Some(PerformanceChoice::Dismiss);
                        }
                    });
                });
        });
    choice
}

/// Browser font and deferred-download state.
#[cfg(target_arch = "wasm32")]
struct WebFonts {
    started: vearth_platform::time::Instant,
    first_view_settled: bool,
    /// `location_title::font_generation` the context's fonts were built at.
    generation: u64,
    cjk_requested: bool,
}

#[cfg(target_arch = "wasm32")]
impl WebFonts {
    fn new() -> Self {
        Self {
            started: vearth_platform::time::Instant::now(),
            first_view_settled: false,
            generation: crate::location_title::font_generation(),
            cjk_requested: false,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Ui {
    /// Longest wait for the first view before deferred downloads start (for
    /// example while flying).
    const FIRST_VIEW_DEFERRAL: vearth_platform::time::Duration =
        vearth_platform::time::Duration::from_secs(20);

    /// Whether deferred downloads may start: the first view has loaded, or
    /// waiting for it took too long.
    pub(crate) fn first_view_settled(&self) -> bool {
        self.web_fonts.first_view_settled
    }

    fn poll_fonts(&mut self, view_loaded: bool) {
        let fonts = &mut self.web_fonts;
        if !fonts.first_view_settled
            && (view_loaded || fonts.started.elapsed() >= Self::FIRST_VIEW_DEFERRAL)
        {
            fonts.first_view_settled = true;
            // Titles download their own faces first when they need them;
            // prefetching the rest keeps later titles from waiting.
            let spawned = vearth_platform::thread::Builder::new()
                .name("font-prefetch".into())
                .spawn(crate::location_title::load_all_faces);
            if let Err(error) = spawned {
                log::error!("fonts: could not start the prefetch: {error}");
            }
        }
        let cjk_text = |text: &str| {
            text.chars().any(|c| {
                matches!(c as u32,
                    0x1100..=0x11ff | 0x3000..=0x30ff | 0x3400..=0x9fff |
                    0xac00..=0xd7af | 0xf900..=0xfaff | 0xff00..=0xffef)
            })
        };
        if !fonts.cjk_requested
            && (cjk_text(&self.query)
                || self
                    .results
                    .iter()
                    .any(|place| cjk_text(&place.display_name)))
        {
            fonts.cjk_requested = true;
            let spawned = vearth_platform::thread::Builder::new()
                .name("font-loader".into())
                .spawn(|| crate::location_title::load_faces(&[crate::location_title::Face::Cjk]));
            if let Err(error) = spawned {
                log::error!("fonts: could not start the CJK loader: {error}");
            }
        }
        let generation = crate::location_title::font_generation();
        if generation != fonts.generation {
            fonts.generation = generation;
            self.ctx.set_fonts(vearth_font_definitions());
        }
    }
}

impl Ui {
    pub fn new(
        window: &crate::host::Window,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        attributions: Vec<vearth_data::registry::AttributionEntry>,
        calibration_landmark: Option<&'static Landmark>,
    ) -> Self {
        let ctx = egui::Context::default();
        configure_fonts(&ctx);
        if calibration_landmark.is_none() {
            configure_game_style(&ctx);
        } else {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = egui::Color32::from_rgb(14, 17, 18);
            visuals.window_fill = egui::Color32::from_rgba_unmultiplied(14, 17, 18, 244);
            visuals.selection.bg_fill = egui::Color32::from_rgb(198, 232, 93);
            visuals.selection.stroke.color = egui::Color32::from_rgb(8, 12, 10);
            visuals.widgets.active.bg_fill = egui::Color32::from_rgb(198, 232, 93);
            visuals.widgets.active.fg_stroke.color = egui::Color32::from_rgb(8, 12, 10);
            ctx.set_visuals(visuals);
        }
        let input = crate::egui_input::EguiInput::new(ctx.clone(), window);
        let mut renderer =
            egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        let map_renderer = crate::map::MapRenderer::new(device, format, &mut renderer);
        let (tx, rx) = channel();
        let geocoder = Arc::new(std::sync::OnceLock::new());
        Ui {
            #[cfg(target_arch = "wasm32")]
            web_fonts: WebFonts::new(),
            performance: PerformanceAdvisor::default(),
            ctx,
            input,
            renderer,
            pending_textures: PendingTextureDeltas::default(),
            query: String::new(),
            results: Vec::new(),
            status: None,
            searching: false,
            search_seq: 0,
            submitted: None,
            pending_next: None,
            last_edit: None,
            tx,
            rx,
            geocoder,
            focus_search: false,
            search_has_focus: false,
            pending_paste: None,
            hide_panels: false,
            show_controls: false,
            show_search: calibration_landmark.is_none(),
            search_expanded: false,
            show_debug: false,
            show_solar: calibration_landmark.is_none(),
            show_compass: calibration_landmark.is_none(),
            screenshot_notice: None,
            show_map: false,
            map_renderer,
            map_navigation: crate::map::MapNavigation::default(),
            map_places: None,
            map_places_rx: None,
            map_places_requested: false,
            show_loading: true,
            show_attribution: false,
            show_settings: false,
            show_privacy: false,
            show_settings_button: true,
            cache_clear_confirmation: None,
            reset_confirmation: false,
            settings_notice: None,
            date_editor: None,
            loading_progress: CellLoadProgress::default(),
            phase_manta_motion_burst: PhaseMantaMotionBurst::default(),
            attributions,
            calibration: calibration_landmark.map(CalibrationUi::new),
        }
    }

    /// Returns true if egui consumed the event (typing in the search box
    /// must not steer the camera).
    pub(crate) fn on_event(
        &mut self,
        window: &crate::host::Window,
        event: &crate::egui_input::UiEvent,
    ) -> bool {
        self.input.on_event(window, event)
    }

    // Driven by the macOS menu bar.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn set_panels_hidden(&mut self, hidden: bool) {
        self.hide_panels = hidden;
        if hidden {
            self.search_has_focus = false;
            self.focus_search = false;
            self.ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
        }
    }

    // Driven by the macOS menu bar.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn panels_hidden(&self) -> bool {
        self.hide_panels
    }

    pub(crate) fn toggle_controls(&mut self) {
        self.show_controls = !self.show_controls;
    }

    pub(crate) fn controls_visible(&self) -> bool {
        self.show_controls
    }

    pub(crate) fn run_controls(&mut self, window: &crate::host::Window) -> UiPaint {
        let input = self.input.take_egui_input(window);
        let output = self.ctx.run_ui(input, |ctx| {
            show_controls_screen(ctx, 1.0, None, true);
        });
        self.input
            .handle_platform_output(window, output.platform_output);
        self.pending_textures.append(output.textures_delta);
        UiPaint {
            primitives: self.ctx.tessellate(output.shapes, output.pixels_per_point),
            pixels_per_point: output.pixels_per_point,
        }
    }

    pub fn focus_search(&mut self) {
        self.hide_panels = false;
        self.show_search = true;
        self.search_expanded = true;
        self.focus_search = true;
    }

    pub fn open_date_editor(&mut self, utc_seconds: f64) {
        self.hide_panels = false;
        self.date_editor = Some(DateEditor::new(utc_seconds));
    }

    /// macOS routes the native Edit → Paste accelerator through the menu
    /// responder chain instead of delivering a key event to winit. Queue an
    /// ordinary egui paste event so TextEdit still owns cursor/selection
    /// semantics exactly as it does on platforms where the shortcut reaches
    /// egui-winit directly.
    pub fn paste_into_search(&mut self) {
        if !self.show_search || !self.search_expanded || !self.search_has_focus {
            return;
        }
        let Some(text) = self.input.clipboard_text() else {
            return;
        };
        let text = text.replace("\r\n", "\n");
        if !text.is_empty() {
            self.pending_paste = Some(text);
        }
    }

    pub fn set_search_visible(&mut self, visible: bool) {
        self.show_search = visible;
        if !visible {
            self.search_expanded = false;
            self.focus_search = false;
            self.clear_results();
        }
    }

    pub fn set_debug_visible(&mut self, visible: bool) {
        self.show_debug = visible;
    }

    pub fn set_solar_visible(&mut self, visible: bool) {
        self.show_solar = visible;
    }

    pub fn set_compass_visible(&mut self, visible: bool) {
        self.show_compass = visible;
    }

    pub(crate) fn set_screenshot_notice(&mut self, notice: String) {
        self.screenshot_notice = Some(notice);
    }

    pub fn open_map(&mut self, lat_deg: f64, lon_deg: f64, in_play: bool) {
        if !self.map_places_requested {
            let (tx, rx) = channel();
            let geocoder = self.geocoder.clone();
            vearth_platform::thread::Builder::new()
                .name("map-place-catalog".into())
                .spawn(move || {
                    let result = geocoder
                        .get_or_init(Geocoder::new)
                        .map_places(3_000)
                        .map(crate::map::MapPlaceCatalog::new)
                        .map_err(|error| error.to_string());
                    let _ = tx.send(result);
                })
                .expect("spawn map place catalog worker");
            self.map_places_rx = Some(rx);
            self.map_places_requested = true;
        }
        self.map_navigation.open(lat_deg, lon_deg, in_play);
        self.show_map = true;
    }

    pub fn close_map(&mut self) {
        self.map_navigation.close();
        self.show_map = false;
    }

    pub fn map_visible(&self) -> bool {
        self.show_map
    }

    pub(crate) fn run_map(
        &mut self,
        window: &crate::host::Window,
        lat_deg: f64,
        lon_deg: f64,
        heading_deg: f64,
        current_location_name: Option<&str>,
    ) -> UiPaint {
        debug_assert!(self.show_map);
        if self.map_places.is_none()
            && let Some(rx) = &self.map_places_rx
            && let Ok(result) = rx.try_recv()
        {
            match result {
                Ok(places) => self.map_places = Some(places),
                Err(error) => log::warn!("map place labels unavailable: {error}"),
            }
            self.map_places_rx = None;
        }
        let input = self.input.take_egui_input(window);
        let map_renderer = &mut self.map_renderer;
        let map_view = self.map_navigation.view_mut();
        let output = self.ctx.run_ui(input, |ctx| {
            ctx.request_repaint();
            let viewport = ctx.content_rect();
            let interaction_id = egui::Id::new("mercator position map interaction");
            egui::Area::new(interaction_id)
                .fixed_pos(viewport.min)
                .order(egui::Order::Tooltip)
                .interactable(true)
                .show(ctx, |ui| {
                    ui.allocate_exact_size(viewport.size(), egui::Sense::click_and_drag());
                });
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Debug,
                egui::Id::new("mercator position map paint"),
            ));
            crate::map::paint_map(
                ctx,
                &painter,
                viewport,
                map_renderer,
                map_view,
                lat_deg,
                lon_deg,
                heading_deg,
                self.map_places.as_ref(),
                current_location_name,
            );
        });
        self.input
            .handle_platform_output(window, output.platform_output);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.pending_textures.append(output.textures_delta);
        UiPaint {
            primitives,
            pixels_per_point: output.pixels_per_point,
        }
    }

    pub fn set_loading_visible(&mut self, visible: bool) {
        self.show_loading = visible;
    }

    pub fn set_attribution_visible(&mut self, visible: bool) {
        self.show_attribution = visible;
    }

    pub fn open_privacy(&mut self) {
        self.hide_panels = false;
        self.show_privacy = true;
    }

    pub fn open_settings(&mut self) {
        self.hide_panels = false;
        self.show_settings = true;
    }

    pub fn set_settings_button_visible(&mut self, visible: bool) {
        self.show_settings_button = visible;
    }

    pub fn settings_button_visible(&self) -> bool {
        self.show_settings_button
    }

    pub fn set_settings_notice(&mut self, notice: String) {
        self.settings_notice = Some(notice);
    }

    pub fn panel_visibility(&self) -> (bool, bool, bool, bool, bool, bool) {
        (
            self.show_search,
            self.show_debug,
            self.show_solar,
            self.show_compass,
            self.show_loading,
            self.show_attribution,
        )
    }

    /// Search-as-you-type entry point: dedups against the in-flight query;
    /// while one is running the newest edit waits its turn (never more than
    /// one lookup in flight).
    fn maybe_search(&mut self, q: &str) {
        let q = q.trim();
        if q.is_empty() || self.submitted.as_deref() == Some(q) {
            return;
        }
        if self.searching {
            self.pending_next = Some(q.to_string());
            return;
        }
        self.kick_search(q.to_string());
    }

    fn kick_search(&mut self, q: String) {
        self.searching = true;
        self.search_seq += 1;
        self.submitted = Some(q.clone());
        let seq = self.search_seq;
        let tx = self.tx.clone();
        let geocoder = self.geocoder.clone();
        vearth_platform::thread::spawn(move || {
            let res = geocoder
                .get_or_init(Geocoder::new)
                .search(&q)
                .map_err(|e| e.to_string());
            let _ = tx.send((seq, res));
        });
    }

    /// Dismiss the suggestion dropdown: clearing the query, clicking
    /// outside the search window, or Esc. Also cancels any in-flight
    /// search — bumping the sequence orphans its reply, so late results
    /// can't resurrect a dropdown the user just dismissed.
    fn clear_results(&mut self) {
        self.results.clear();
        self.status = None;
        self.submitted = None;
        self.pending_next = None;
        self.last_edit = None;
        self.search_seq += 1;
        self.searching = false;
    }

    /// Build this frame's UI; returns any action plus the paint callback to
    /// run after the 3D frame renders.
    pub fn run(&mut self, window: &crate::host::Window, hud: &HudInfo<'_>) -> (UiAction, UiPaint) {
        #[cfg(target_arch = "wasm32")]
        self.poll_fonts(hud.queued + hud.generating == 0 && hud.resident_cells > 0);
        #[cfg(target_arch = "wasm32")]
        let first_view_loaded = self.web_fonts.first_view_settled;
        #[cfg(not(target_arch = "wasm32"))]
        let first_view_loaded = hud.queued + hud.generating == 0 && hud.resident_cells > 0;
        let performance_advice =
            self.performance
                .update(hud, first_view_loaded, window.scale_factor());
        let mut performance_choice = None;
        while let Ok((seq, res)) = self.rx.try_recv() {
            if seq != self.search_seq {
                continue;
            }
            self.searching = false;
            match res {
                Ok(places) if places.is_empty() => {
                    self.results.clear();
                    self.status = Some("No results.".into());
                }
                Ok(places) => {
                    self.status = None;
                    self.results = places;
                }
                Err(e) => self.status = Some(format!("Search failed: {e}")),
            }
            if let Some(next) = self.pending_next.take() {
                self.kick_search(next);
            }
        }

        let mut action = UiAction::None;
        let mut raw = self.input.take_egui_input(window);
        if let Some(text) = self.pending_paste.take() {
            raw.events.push(egui::Event::Paste(text));
        }
        let focus_search = std::mem::take(&mut self.focus_search);
        let query = &mut self.query;
        let results = &mut self.results;
        let status = &self.status;
        let searching = self.searching;
        let show_search = &mut self.show_search;
        let search_expanded = &mut self.search_expanded;
        let show_debug = &mut self.show_debug;
        let panels_visible = !self.hide_panels;
        let show_solar = panels_visible && self.show_solar;
        let show_compass = self.show_compass;
        let screenshot_notice = self.screenshot_notice.as_deref();
        let show_loading = panels_visible && self.show_loading;
        let show_attr = &mut self.show_attribution;
        let show_settings = &mut self.show_settings;
        let show_privacy = &mut self.show_privacy;
        let show_settings_button = panels_visible && self.show_settings_button;
        let cache_clear_confirmation = &mut self.cache_clear_confirmation;
        let reset_confirmation = &mut self.reset_confirmation;
        let settings_notice = self.settings_notice.as_deref();
        let date_editor = &mut self.date_editor;
        let calibration = &mut self.calibration;
        let phase_manta_motion_burst = &mut self.phase_manta_motion_burst;
        let attributions = &self.attributions;
        let loading = hud.queued + hud.generating;
        let loading_progress = self.loading_progress.update(loading);
        let mut do_search = false;
        let mut edited = false;
        let mut dismiss = false;
        let mut search_has_focus = false;
        let mut request_search_focus = false;
        let mut collapse_search = false;
        let mut close_date_editor = false;
        let output = self.ctx.run_ui(raw, |ctx| {
            let time = ctx.input(|input| input.time);
            let motion_opacity = phase_manta_motion_burst.update(hud.play.as_ref(), time);
            if let Some(opacity) = motion_opacity
                && ctx.style().animation_time > 0.0
            {
                ctx.request_repaint();
                let viewport = ctx.content_rect();
                let streaks = phase_manta_motion_streaks(viewport, opacity);
                if !streaks.is_empty() {
                    let painter = ctx
                        .layer_painter(egui::LayerId::new(
                            egui::Order::Background,
                            egui::Id::new("phase manta peripheral motion"),
                        ))
                        .with_clip_rect(viewport);
                    paint_phase_manta_motion_streaks(&painter, &streaks);
                }
            }
            if let Some(toast) = &hud.location_toast {
                crate::location_title::paint(ctx, &toast.title, toast.elapsed_s, toast.alpha);
            }
            if panels_visible && let Some(editor) = date_editor.as_mut() {
                egui::Window::new("Set date")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.set_width(250.0);
                        ui.label(
                            egui::RichText::new("ASTRONOMICAL DATE")
                                .monospace()
                                .size(10.0)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "Keeps the current {}",
                                crate::solar::format_utc_time(editor.source_utc),
                            ))
                            .size(10.0)
                            .weak(),
                        );
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut editor.date)
                                .hint_text("YYYY-MM-DD")
                                .desired_width(f32::INFINITY),
                        );
                        if editor.focus {
                            response.request_focus();
                            editor.focus = false;
                        }
                        let submit = response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        if let Some(error) = &editor.error {
                            ui.label(egui::RichText::new(error).size(10.0).color(
                                egui::Color32::from_rgb(232, 132, 120),
                            ));
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                close_date_editor = true;
                            }
                            if ui.button("Apply date").clicked() || submit {
                                match crate::solar::replace_utc_date(
                                    editor.source_utc,
                                    &editor.date,
                                ) {
                                    Ok(utc) => {
                                        action = UiAction::SetSunUtc(utc);
                                        close_date_editor = true;
                                    }
                                    Err(error) => editor.error = Some(error),
                                }
                            }
                        });
                    });
                if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                    close_date_editor = true;
                }
            }

            if panels_visible && let Some(calibration) = calibration
                && let Some(calibration_action) =
                    show_calibration_panel(ctx, calibration, loading)
            {
                action = calibration_action;
            }

            let teleport = if panels_visible && *show_search && *search_expanded {
                egui::Window::new("teleport")
                    .title_bar(false)
                    .resizable(false)
                    .frame(egui::Frame::NONE.inner_margin(8))
                    .anchor(egui::Align2::LEFT_TOP, [20.0, 20.0])
                    .show(ctx, |ui| {
                        ui.set_width(280.0);
                        ui.spacing_mut().item_spacing = egui::vec2(10.0, 0.0);
                        let mut backdrop = ui.painter().clone();
                        backdrop.set_clip_rect(ui.ctx().content_rect());
                        let scrim = backdrop.add(egui::Shape::Noop);
                        let field = ui.horizontal(|ui| {
                            let (icon_rect, _) = ui.allocate_exact_size(
                                egui::vec2(20.0, 28.0), egui::Sense::hover(),
                            );
                            paint_hud_icon(ui, icon_rect.center(), HudIcon::Search, 20.0);
                            let edit = egui::TextEdit::singleline(query)
                                .hint_text("Search places")
                                .font(egui::FontId::proportional(15.0))
                                .frame(egui::Frame::NONE)
                                .desired_width(ui.available_width() - if searching { 60.0 } else { 34.0 });
                            let resp = ui.add(edit);
                            if focus_search {
                                resp.request_focus();
                            }
                            search_has_focus = resp.has_focus();
                            edited |= resp.changed();
                            if searching {
                                ui.spinner();
                            }
                            if resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                do_search = true;
                            }
                            let close = ui.add(egui::Button::new("").frame(false)
                                .min_size(egui::vec2(24.0, 28.0)));
                            close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close search"));
                            paint_hud_icon(ui, close.rect.center(), HudIcon::Close, 14.0);
                            if close.on_hover_text("Close search").clicked() {
                                dismiss = true;
                                collapse_search = true;
                            }
                        });
                        ui.painter().hline(
                            (field.response.rect.left() + 30.0)..=field.response.rect.right(),
                            field.response.rect.bottom() + 2.0,
                            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(45)),
                        );
                        ui.add_space(8.0);
                        if let Some(s) = status {
                            ui.horizontal(|ui| {
                                ui.add_space(30.0);
                                ui.label(egui::RichText::new(s).size(12.0).weak());
                            });
                        }
                        let mut visit_clicked: Option<usize> = None;
                        let list = egui::ScrollArea::vertical()
                            .max_height((ui.ctx().content_rect().height() - 120.0).clamp(96.0, 224.0))
                            .show(ui, |ui| {
                                for (i, place) in results.iter().enumerate() {
                                    if search_result_button(ui, &place.display_name).clicked() {
                                        visit_clicked = Some(i);
                                    }
                                }
                            });
                        backdrop.set(scrim, search_scrim(field.response.rect.union(list.inner_rect)));
                        if let Some(i) = visit_clicked {
                            action = UiAction::FlyTo(results[i].clone());
                            query.clear();
                            results.clear();
                            dismiss = true;
                            collapse_search = true;
                        }
                    })
            } else {
                None
            };
            if panels_visible && *show_search && !*search_expanded {
                egui::Area::new(egui::Id::new("search launcher"))
                    .anchor(egui::Align2::LEFT_TOP, [20.0, 20.0])
                    .show(ctx, |ui| {
                        if search_icon_button(ui)
                            .on_hover_text("Search anywhere on Earth  ( / )")
                            .clicked()
                        {
                            *search_expanded = true;
                            request_search_focus = true;
                        }
                    });
            }
            // A click anywhere outside the search window dismisses the
            // suggestion dropdown. Esc collapses the whole search control.
            // egui records presses over the 3D view, so those count too.
            if !results.is_empty() {
                let outside_click = teleport.as_ref().is_some_and(|t| {
                    ctx.input(|i| {
                        i.pointer.any_pressed()
                            && i.pointer
                                .interact_pos()
                                .is_some_and(|p| !t.response.rect.contains(p))
                    })
                });
                if outside_click {
                    dismiss = true;
                }
            }
            if *search_expanded && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                collapse_search = true;
                dismiss = true;
            }

            let solar_panel = show_solar.then(|| {
                egui::Window::new("solar controls")
                    .title_bar(false)
                    .resizable(false)
                    .frame(egui::Frame::NONE.inner_margin(6))
                    .anchor(egui::Align2::LEFT_BOTTOM, [18.0, -18.0])
                    .show(ctx, |ui| {
                        ui.set_width(460.0_f32.min((ctx.content_rect().width() - 48.0).max(240.0)));
                        let solar_action = show_solar_controls(ui, &hud.sun, hud.local_time);
                        if !matches!(solar_action, UiAction::None) {
                            action = solar_action;
                        }
                    })
            });
            let solar_panel_height = solar_panel
                .as_ref()
                .and_then(Option::as_ref)
                .map_or(0.0, |panel| panel.response.rect.height());

            let flight_hud_bottom = 18.0
                + if show_solar {
                    solar_panel_height + 8.0
                } else {
                    0.0
                };
            let flight_hud = hud
                .play
                .as_ref()
                .filter(|play| panels_visible && play.vehicle == VehicleKind::PhaseManta)
                .map(|play| {
                    let frame = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 7));
                    egui::Window::new("phase manta flight hud")
                        .title_bar(false)
                        .resizable(false)
                        .interactable(false)
                        .frame(frame)
                        .anchor(
                            egui::Align2::LEFT_BOTTOM,
                            [12.0, -flight_hud_bottom],
                        )
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.label(
                                    egui::RichText::new("SPEED")
                                        .monospace()
                                        .size(11.0)
                                        .strong()
                                        .color(PANEL_INK),
                                );
                                ui.label(
                                    egui::RichText::new(fmt_speed(play.speed_ms))
                                        .monospace()
                                        .size(14.0)
                                        .strong(),
                                );
                                ui.separator();
                                ui.label(
                                    egui::RichText::new("ALT")
                                        .monospace()
                                        .size(11.0)
                                        .strong()
                                        .color(PANEL_INK),
                                );
                                ui.label(
                                    egui::RichText::new(fmt_alt(play.altitude_m))
                                        .monospace()
                                        .size(14.0)
                                        .strong(),
                                );
                            });
                        })
                });
            let flight_hud_height = flight_hud
                .as_ref()
                .and_then(Option::as_ref)
                .map_or(0.0, |panel| panel.response.rect.height());

            if panels_visible && *show_debug {
                egui::Window::new("hud")
                    .title_bar(false)
                    .resizable(false)
                    .interactable(true)
                    .anchor(
                        egui::Align2::LEFT_BOTTOM,
                        [
                            12.0,
                            -(flight_hud_bottom
                                + if flight_hud.is_some() {
                                    flight_hud_height + 6.0
                                } else {
                                    0.0
                                }),
                        ],
                    )
                    .show(ctx, |ui| {
                    let loading = hud.queued + hud.generating;
                    let mut line = format!(
                        "{:.4}°, {:.4}°   alt {}   fps {:.0}   cells {}   loading {}",
                        hud.lat_deg,
                        hud.lon_deg,
                        fmt_alt(hud.alt_m),
                        hud.fps,
                        hud.resident_cells,
                        loading,
                    );
                    if loading > 0 {
                        line.push_str(&format!(
                            " (q {} · gen {}{})",
                            hud.queued,
                            hud.generating,
                            hud.cold_pct
                                .map(|p| format!(" · cold {p}%"))
                                .unwrap_or_default(),
                        ));
                    }
                    ui.label(egui::RichText::new(line).monospace().size(11.0));
                    ui.label(
                        egui::RichText::new(format!(
                            "sky  Bortle {} · limiting magnitude {:.1}",
                            hud.bortle_class, hud.limiting_magnitude
                        ))
                        .monospace()
                        .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "terrain LOD ruggedness {} · L to switch",
                            hud.lod_ruggedness
                        ))
                        .monospace()
                        .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "climate wet-hour {:.0}% @ {:.1} mm/h · cloud L/M/H {:.0}/{:.0}/{:.0}% OD {:.1} · aerosol {:.2} dust {:.0}%",
                            hud.weather.precipitation_wet_fraction * 100.0,
                            hud.weather.precipitation_conditional_mm_h,
                            hud.weather.low_cloud_coverage * 100.0,
                            hud.weather.mid_cloud_coverage * 100.0,
                            hud.weather.high_cloud_coverage * 100.0,
                            hud.weather.cloud_optical_depth,
                            hud.weather.aerosol,
                            hud.weather.aerosol_dust_fraction * 100.0,
                        ))
                        .monospace()
                        .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "wind layers low E{:+.1}/N{:+.1} · mid E{:+.1}/N{:+.1} · upper E{:+.1}/N{:+.1} m/s",
                            hud.weather.wind_low[0], hud.weather.wind_low[1],
                            hud.weather.wind_mid[0], hud.weather.wind_mid[1],
                            hud.weather.wind_upper[0], hud.weather.wind_upper[1],
                        ))
                        .monospace()
                        .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "weather {} q{:?} · atlas v{} model v{} · seed {} · ticks {}→{} {:.2}",
                            hud.weather.mode,
                            hud.weather.quality,
                            hud.weather.atlas_version,
                            hud.weather.model_version,
                            hud.weather.seed,
                            hud.weather.tick0,
                            hud.weather.tick1,
                            hud.weather.tick_alpha,
                        ))
                        .monospace()
                        .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "local {:+.1}°C · RH {:.0}% · {} {:.1} mm/h · vis {} · wind E{:+.1} N{:+.1} m/s · wet {:.0}% snow {:.0}% sea {:.0}%",
                            hud.weather.temperature_c,
                            hud.weather.humidity * 100.0,
                            hud.weather.precipitation_kind,
                            hud.weather.precipitation_mm_h,
                            fmt_alt(f64::from(hud.weather.visibility_m)),
                            hud.weather.wind_east_mps,
                            hud.weather.wind_north_mps,
                            hud.weather.wetness * 100.0,
                            hud.weather.snow * 100.0,
                            hud.weather.sea_state * 100.0,
                        ))
                        .monospace()
                        .size(11.0),
                    );
                    if let Some(p) = &hud.play {
                        let mut s = if p.speed_ms >= 1_000.0 {
                            format!(
                                "{}  ground {:>5.2} km/s · air {:>5.2} km/s",
                                p.vehicle_name,
                                p.speed_ms / 1_000.0,
                                p.airspeed_ms / 1_000.0
                            )
                        } else {
                            format!(
                                "{}  ground {:>5.1} m/s · air {:>5.1} m/s",
                                p.vehicle_name, p.speed_ms, p.airspeed_ms
                            )
                        };
                        if p.boost {
                            s.push_str(if p.phased {
                                "   PHASE DRIVE"
                            } else {
                                "   BOOST"
                            });
                        } else if p.phased {
                            s.push_str("   PHASED · COLLISION NULL");
                        }
                        ui.label(
                            egui::RichText::new(s)
                                .monospace()
                                .size(11.0)
                                .color(if p.phased {
                                    egui::Color32::from_rgb(216, 126, 255)
                                } else {
                                    egui::Color32::from_rgb(110, 220, 255)
                                }),
                        );
                        ui.label(
                            egui::RichText::new(
                                match p.vehicle {
                                    VehicleKind::Sparrow => "WASD steer · Space jump · Shift boost · V change vehicle · P exit",
                                    _ => "WASD move · Space/C rise/sink · Shift boost · V change vehicle · P exit",
                                },
                            )
                            .size(10.0)
                            .weak(),
                        );
                    }
                    for status in &hud.provider_status {
                        ui.label(
                            egui::RichText::new(status)
                                .monospace()
                                .size(11.0)
                                .color(egui::Color32::from_rgb(235, 180, 90)),
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("copy location link").clicked() {
                            ctx.copy_text(format!("vearth --at {}", hud.deep_link()));
                        }
                        if hud.play.is_none() {
                            ui.label(
                                egui::RichText::new("P — fly the Kestrel")
                                    .size(10.0)
                                    .weak(),
                            );
                        }
                    });
                    });
            }

            if panels_visible && *show_attr {
                // List provider and bundled-source attribution by layer.
                // Disabled providers remain visible.
                egui::Window::new("Data sources")
                    .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        let mut last_layer = "";
                        for e in attributions {
                            if e.layer != last_layer {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(e.layer).strong().size(11.0),
                                );
                                last_layer = e.layer;
                            }
                            ui.horizontal_wrapped(|ui| {
                                if e.url.is_empty() {
                                    ui.label(egui::RichText::new(&e.name).size(11.0));
                                } else {
                                    ui.hyperlink_to(
                                        egui::RichText::new(&e.name).size(11.0),
                                        &e.url,
                                    );
                                }
                                if !e.license.is_empty() {
                                    ui.label(
                                        egui::RichText::new(format!("({})", e.license))
                                            .size(10.0)
                                            .weak(),
                                    );
                                }
                            });
                            if !e.attribution.is_empty() {
                                ui.label(
                                    egui::RichText::new(&e.attribution).size(10.0).weak(),
                                );
                            }
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Distinctive water masks").strong().size(11.0));
                        ui.hyperlink_to(
                            egui::RichText::new("© OpenStreetMap contributors (ODbL 1.0)").size(11.0),
                            "https://www.openstreetmap.org/copyright",
                        );
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Architectural place data").strong().size(11.0));
                        ui.hyperlink_to(
                            egui::RichText::new("© NYC Landmarks Preservation Commission").size(11.0),
                            "https://www.nyc.gov/site/lpc/about/maps.page",
                        );
                        ui.hyperlink_to(
                            egui::RichText::new("NYC Open Data terms").size(10.0),
                            "https://data.cityofnewyork.us/stories/s/Terms-of-Use/k9k7-3cje/",
                        );
                        for (credit, url) in [
                            ("Westminster City Council (OGL v3.0)", "https://www.westminster.gov.uk/planning-building-control-and-environmental-regulations/conservation-areas"),
                            ("Gemeente Amsterdam (Creative Commons attribution)", "https://data.amsterdam.nl/"),
                            ("Câmara Municipal de Lisboa (CC0)", "https://dados.cm-lisboa.pt/dataset/limite-de-planos-de-pormenor"),
                            ("Ayuntamiento de Madrid (CC BY 4.0)", "https://datos.madrid.es/pages/condiciones-de-uso"),
                            ("Comune di Milano (CC BY 4.0)", "https://pgt.comune.milano.it/"),
                            ("Data ja kartat (c) Helsingin kaupunki, kaupunkimittauspalvelut. (CC BY 4.0)", "https://www.hel.fi/fi/paatoksenteko-ja-hallinto/tietoa-helsingista/kartat-ja-paikkatieto/paikkatiedot-tehokayttoon/avoimet-paikkatiedot"),
                        ] {
                            ui.hyperlink_to(egui::RichText::new(credit).size(10.0), url);
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Place search").strong().size(11.0));
                        ui.horizontal_wrapped(|ui| {
                            ui.hyperlink_to(
                                egui::RichText::new("GeoNames").size(11.0),
                                "https://www.geonames.org/",
                            );
                            ui.label(
                                egui::RichText::new("(CC BY 4.0)")
                                    .size(10.0)
                                    .weak(),
                            );
                        });
                    });
            }

            if show_settings_button && calibration.is_none() {
                egui::Area::new(egui::Id::new("settings launcher"))
                    .anchor(egui::Align2::RIGHT_TOP, [-20.0, 20.0])
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        if settings_icon_button(ui)
                            .on_hover_text("Open settings")
                            .clicked()
                        {
                            *show_settings = !*show_settings;
                        }
                    });
            }

            if panels_visible && *show_settings {
                let mut close_settings = false;
                egui::Window::new("Settings")
                    .title_bar(false)
                    .default_width(355.0)
                    .frame(game_panel_frame())
                    .anchor(egui::Align2::RIGHT_TOP, [-24.0, 70.0])
                    .max_height((ctx.content_rect().height() - 150.0).max(180.0))
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.set_width(307.0_f32.min((ctx.content_rect().width() - 96.0).max(230.0)));
                        close_settings = panel_heading(ui, "Settings");
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if let Some(settings_action) = show_settings_contents(
                                ui,
                                hud.preferences,
                                hud.provider_controls,
                                hud.storage,
                                cache_clear_confirmation,
                                reset_confirmation,
                                settings_notice,
                                show_privacy,
                                &hud.photo,
                            ) {
                                action = settings_action;
                            }
                        });
                    });
                if close_settings {
                    *show_settings = false;
                }
            }

            if panels_visible && *show_privacy {
                crate::privacy::show(ctx, show_privacy);
            }

            #[cfg(target_arch = "wasm32")]
            if panels_visible && cache_clear_confirmation.is_some() {
                egui::Window::new("Clear all data?")
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.set_width(360.0);
                        ui.label(
                            "Low Poly Earth closes and removes its saved terrain, map data and settings from this browser. Elevation tiles stay in the browser's cache until it clears them.",
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if outlined_button(ui, "Clear and exit").clicked() {
                                action = UiAction::ClearBrowserData;
                                *cache_clear_confirmation = None;
                            }
                            if ui.button("Cancel").clicked() {
                                *cache_clear_confirmation = None;
                            }
                        });
                    });
            }

            #[cfg(not(target_arch = "wasm32"))]
            if panels_visible && let Some(pending) = *cache_clear_confirmation {
                egui::Window::new("Clear downloaded data?")
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.set_width(360.0);
                        ui.label(format!("{} will be removed safely on the next launch.", pending.label()));
                        ui.label(
                            egui::RichText::new(
                                "Your settings and captured photos will not be affected.",
                            )
                            .weak(),
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Schedule removal").clicked() {
                                action = UiAction::ScheduleCacheClear(pending);
                                *cache_clear_confirmation = None;
                            }
                            if ui.button("Cancel").clicked() {
                                *cache_clear_confirmation = None;
                            }
                        });
                    });
            }

            if show_loading && let Some(progress) = loading_progress {
                egui::Area::new(egui::Id::new("cell loading progress"))
                    .anchor(egui::Align2::RIGHT_TOP, [-20.0, 68.0])
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(34.0, 34.0),
                            egui::Sense::hover(),
                        );
                        response.on_hover_text(format!(
                            "{loading} cell{} still loading",
                            if loading == 1 { "" } else { "s" },
                        ));
                        paint_loading_progress(ui.painter(), rect, progress);
                    });
            }

            if let Some(advice) = &performance_advice {
                performance_choice = show_performance_advice(ctx, advice);
            }

            if show_compass {
                egui::Area::new(egui::Id::new("screenshot"))
                    .anchor(egui::Align2::RIGHT_BOTTOM, [-24.0, -98.0])
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(52.0, 44.0), egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                "Save screenshot to Desktop",
                            )
                        });
                        if response.clicked() {
                            action = UiAction::CaptureScreenshot;
                        }
                        let response = response
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(
                                screenshot_notice.unwrap_or("Save screenshot to Desktop"),
                            );
                        let pressed = response.is_pointer_button_down_on();
                        let active = pressed || response.clicked();
                        let feedback = ui.ctx().animate_bool_with_time(
                            response.id.with("press_feedback"),
                            active,
                            if active { 0.0 } else { 0.35 },
                        );
                        paint_camera(ui.painter(), rect, response.hovered(), pressed, feedback);
                    });
                egui::Area::new(egui::Id::new("compass"))
                    .anchor(egui::Align2::RIGHT_BOTTOM, [-24.0, -22.0])
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(52.0, 64.0), egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Reset north")
                        });
                        if response.clicked() {
                            action = UiAction::ResetNorth;
                        }
                        let response = response.on_hover_text("Reset north");
                        let painter = ui.painter_at(rect);
                        paint_compass(&painter, rect, hud.heading_deg, response.hovered());
                    });
            }
        });
        match performance_choice {
            Some(PerformanceChoice::Lower(resolution)) => {
                self.performance.handled();
                let mut next = hud.preferences.clone();
                next.render_resolution = resolution;
                if matches!(action, UiAction::None) {
                    action = UiAction::ApplyPreferences(next);
                }
            }
            Some(PerformanceChoice::Dismiss) => self.performance.dismiss(),
            None => {}
        }
        if close_date_editor {
            *date_editor = None;
        }
        if collapse_search {
            *search_expanded = false;
        }
        if request_search_focus {
            self.focus_search = true;
        }
        self.search_has_focus = search_has_focus;

        if (edited && self.query.trim().is_empty()) || dismiss {
            self.clear_results();
            edited = false;
        }
        if edited {
            self.last_edit = Some(vearth_platform::time::Instant::now());
        }
        if do_search {
            self.last_edit = None;
            let q = self.query.clone();
            self.maybe_search(&q);
        } else if self
            .last_edit
            .is_some_and(|t| t.elapsed() >= SEARCH_DEBOUNCE)
        {
            self.last_edit = None;
            if self.query.trim().chars().count() >= LIVE_MIN_CHARS {
                let q = self.query.clone();
                self.maybe_search(&q);
            }
        }
        self.input
            .handle_platform_output(window, output.platform_output);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.pending_textures.append(output.textures_delta);
        (
            action,
            UiPaint {
                primitives,
                pixels_per_point: output.pixels_per_point,
            },
        )
    }

    /// Paint the overlay onto `target` (after the 3D passes).
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        frame: UiPaint,
    ) {
        paint_egui(
            &mut self.renderer,
            &mut self.pending_textures,
            device,
            queue,
            target,
            width,
            height,
            frame,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_egui(
    renderer: &mut egui_wgpu::Renderer,
    pending_textures: &mut PendingTextureDeltas,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::TextureView,
    width: u32,
    height: u32,
    frame: UiPaint,
) {
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point: frame.pixels_per_point,
    };
    let textures_delta = pending_textures.take();
    for (id, delta) in &textures_delta.set {
        renderer.update_texture(device, queue, *id, delta);
    }
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("egui"),
    });
    let cmd_bufs = renderer.update_buffers(device, queue, &mut encoder, &frame.primitives, &screen);
    {
        let rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        let mut rp = rp.forget_lifetime();
        renderer.render(&mut rp, &frame.primitives, &screen);
    }
    queue.submit(cmd_bufs.into_iter().chain([encoder.finish()]));
    for id in &textures_delta.free {
        renderer.free_texture(id);
    }
}

#[derive(Clone, Copy)]
enum SolarControlIcon {
    Play,
    Pause,
}

fn solar_icon_button(
    ui: &mut egui::Ui,
    icon: SolarControlIcon,
    accessible_label: &'static str,
) -> egui::Response {
    let response = ui.add(
        egui::Button::new("")
            .frame(false)
            .min_size(egui::vec2(34.0, 34.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, accessible_label)
    });
    let color = HUD_INK;
    paint_solar_control_icon(
        &ui.painter_at(response.rect),
        egui::Rect::from_center_size(
            response.rect.center() + egui::vec2(0.0, 1.0),
            egui::vec2(16.0, 16.0),
        ),
        icon,
        egui::Color32::from_black_alpha(220),
    );
    paint_solar_control_icon(
        &ui.painter_at(response.rect),
        egui::Rect::from_center_size(response.rect.center(), egui::vec2(16.0, 16.0)),
        icon,
        color,
    );
    response
}

fn paint_solar_control_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: SolarControlIcon,
    color: egui::Color32,
) {
    let center = rect.center();
    match icon {
        SolarControlIcon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + egui::vec2(-3.0, -5.0),
                    center + egui::vec2(5.0, 0.0),
                    center + egui::vec2(-3.0, 5.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        SolarControlIcon::Pause => {
            for x in [-2.7, 2.7] {
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        center + egui::vec2(x, 0.0),
                        egui::vec2(2.3, 10.0),
                    ),
                    0.7,
                    color,
                );
            }
        }
    }
}

fn launcher_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new("")
            .frame(false)
            .min_size(egui::vec2(36.0, 36.0)),
    )
}

#[derive(Clone, Copy)]
enum HudIcon {
    Search,
    Settings,
    Close,
}

fn paint_hud_icon(ui: &egui::Ui, center: egui::Pos2, icon: HudIcon, size: f32) {
    let (name, bytes): (&str, &[u8]) = match icon {
        HudIcon::Search => (
            "hud search",
            vearth_platform::asset_bytes!("../assets/ui/search.png"),
        ),
        HudIcon::Settings => (
            "hud settings",
            vearth_platform::asset_bytes!("../assets/ui/settings.png"),
        ),
        HudIcon::Close => (
            "hud close",
            vearth_platform::asset_bytes!("../assets/ui/x.png"),
        ),
    };
    let id = egui::Id::new(name);
    let texture = ui
        .ctx()
        .data_mut(|data| data.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
                .read_info()
                .expect("bundled HUD PNG");
            let mut pixels = vec![0; reader.output_buffer_size().expect("HUD image size")];
            let info = reader.next_frame(&mut pixels).expect("bundled HUD pixels");
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [info.width as usize, info.height as usize],
                &pixels[..info.buffer_size()],
            );
            let texture = ui
                .ctx()
                .load_texture(name, image, egui::TextureOptions::LINEAR);
            ui.ctx()
                .data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
    let rect = egui::Rect::from_center_size(center, egui::vec2(size, size));
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    ui.painter().image(
        texture.id(),
        rect.translate(egui::vec2(0.0, 1.0)),
        uv,
        egui::Color32::from_black_alpha(128),
    );
    ui.painter().image(texture.id(), rect, uv, HUD_INK);
}

fn search_scrim(rect: egui::Rect) -> egui::Shape {
    // A smooth opacity curve avoids the visible knee of a single feathered edge.
    let core = rect.shrink(2.0);
    let corner_radius = 4.0;
    let corners = [
        core.left_top() + egui::vec2(corner_radius, corner_radius),
        core.right_top() + egui::vec2(-corner_radius, corner_radius),
        core.right_bottom() + egui::vec2(-corner_radius, -corner_radius),
        core.left_bottom() + egui::vec2(corner_radius, -corner_radius),
    ];
    const CORNER_SEGMENTS: u32 = 8;
    const RING_VERTICES: u32 = 4 * (CORNER_SEGMENTS + 1);
    const FADE_STEPS: u32 = 24;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(core.center(), egui::Color32::from_black_alpha(105));
    for ring in 0..=FADE_STEPS {
        let t = ring as f32 / FADE_STEPS as f32;
        let opacity = 1.0 - t * t * (3.0 - 2.0 * t);
        let color = egui::Color32::from_black_alpha((105.0 * opacity).round() as u8);
        let radius = corner_radius + 20.0 * t;
        for (corner, center) in corners.iter().enumerate() {
            for step in 0..=CORNER_SEGMENTS {
                let angle = std::f32::consts::PI
                    + (corner as f32 + step as f32 / CORNER_SEGMENTS as f32)
                        * std::f32::consts::FRAC_PI_2;
                mesh.colored_vertex(
                    *center + egui::vec2(angle.cos(), angle.sin()) * radius,
                    color,
                );
            }
        }
    }
    for i in 0..RING_VERTICES {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % RING_VERTICES);
    }
    for ring in 0..FADE_STEPS {
        for i in 0..RING_VERTICES {
            let a = 1 + ring * RING_VERTICES + i;
            let b = 1 + ring * RING_VERTICES + (i + 1) % RING_VERTICES;
            mesh.add_triangle(a, b, a + RING_VERTICES);
            mesh.add_triangle(b, b + RING_VERTICES, a + RING_VERTICES);
        }
    }
    egui::Shape::mesh(mesh)
}

fn search_result_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let response = ui.add(
        egui::Button::new("")
            .frame(false)
            .min_size(egui::vec2(ui.available_width(), 32.0)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(response.rect, 0.0, egui::Color32::from_white_alpha(18));
    }
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_owned(),
        egui::FontId::proportional(13.0),
        HUD_INK,
    );
    job.wrap.max_width = (response.rect.width() - 36.0).max(80.0);
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    hud_text(
        ui.painter(),
        egui::pos2(
            response.rect.left() + 30.0,
            response.rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        HUD_INK,
    );
    response.on_hover_text(label)
}

fn settings_icon_button(ui: &mut egui::Ui) -> egui::Response {
    let response = launcher_button(ui, true);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Settings"));
    paint_hud_icon(ui, response.rect.center(), HudIcon::Settings, 20.0);
    response
}

fn search_icon_button(ui: &mut egui::Ui) -> egui::Response {
    let response = launcher_button(ui, true);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Search"));
    paint_hud_icon(ui, response.rect.center(), HudIcon::Search, 20.0);
    response
}

fn show_calibration_panel(
    ctx: &mut egui::Ui,
    calibration: &mut CalibrationUi,
    loading: usize,
) -> Option<UiAction> {
    let mut next_action = None;
    let panel_frame = egui::Frame::side_top_panel(ctx.style())
        .inner_margin(egui::Margin::symmetric(22, 18))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(18)));
    egui::Panel::right("landmark calibration inspector")
        .exact_size(344.0)
        .resizable(false)
        .frame(panel_frame)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Low Poly Earth / Align")
                        .monospace()
                        .size(10.0)
                        .strong()
                        .color(egui::Color32::from_rgb(198, 232, 93)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (label, color) = if loading == 0 {
                        ("LIVE", egui::Color32::from_rgb(134, 210, 151))
                    } else {
                        ("REBUILDING", egui::Color32::from_rgb(235, 180, 90))
                    };
                    ui.label(
                        egui::RichText::new(label)
                            .monospace()
                            .size(9.0)
                            .strong()
                            .color(color),
                    );
                });
            });
            ui.add_space(14.0);
            ui.label(
                egui::RichText::new("Landmark\ncalibration")
                    .size(27.0)
                    .strong()
                    .line_height(Some(28.0)),
            );
            ui.add_space(7.0);
            ui.label(
                egui::RichText::new(
                    "Place the voxel asset against its real surroundings, then copy the exact transform.",
                )
                .size(12.0)
                .color(egui::Color32::from_gray(156)),
            );
            ui.add_space(18.0);
            ui.separator();
            ui.add_space(14.0);

            section_label(ui, "MODEL");
            let mut selected_name = calibration.landmark.name;
            egui::ComboBox::from_id_salt("calibration landmark")
                .width(ui.available_width())
                .selected_text(landmark_display_name(selected_name))
                .show_ui(ui, |ui| {
                    for landmark in vearth_gen::landmarks::LANDMARKS {
                        ui.selectable_value(
                            &mut selected_name,
                            landmark.name,
                            landmark_display_name(landmark.name),
                        );
                    }
                });
            if selected_name != calibration.landmark.name {
                let landmark = vearth_gen::landmarks::LANDMARKS
                    .iter()
                    .find(|landmark| landmark.name == selected_name)
                    .expect("selected landmark exists");
                calibration.select(landmark);
                next_action = Some(UiAction::ApplyLandmarkCalibration {
                    value: calibration.value(),
                    recenter: true,
                });
            }

            let resolved = vearth_gen::landmarks::resolve_calibration(calibration.value());
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!(
                    "{:.6}°, {:.6}°",
                    resolved.lat_deg, resolved.lon_deg
                ))
                .monospace()
                .size(10.0)
                .color(egui::Color32::from_gray(125)),
            );
            ui.add_space(18.0);

            section_label(ui, "POSITION · LOCAL ENU");
            let mut changed = false;
            changed |= calibration_axis_row(
                ui,
                "East",
                &mut calibration.east_m,
                calibration.move_step_m,
            );
            changed |= calibration_axis_row(
                ui,
                "North",
                &mut calibration.north_m,
                calibration.move_step_m,
            );
            changed |= calibration_axis_row(
                ui,
                "Up",
                &mut calibration.up_m,
                calibration.move_step_m,
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("MOVE STEP")
                        .monospace()
                        .size(9.0)
                        .color(egui::Color32::from_gray(112)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for step in [10.0, 1.0, 0.1] {
                        ui.selectable_value(
                            &mut calibration.move_step_m,
                            step,
                            format_step(step, "m"),
                        );
                    }
                });
            });
            ui.add_space(20.0);

            section_label(ui, "ORIENTATION · MODEL +Z");
            ui.horizontal(|ui| {
                let available = (ui.available_width() - 92.0).max(80.0);
                let heading_response = ui.add_sized(
                    [available, 32.0],
                    egui::DragValue::new(&mut calibration.heading_deg)
                        .speed(calibration.angle_step_deg)
                        .suffix("°")
                        .max_decimals(2),
                );
                changed |= heading_response.changed();
                if ui
                    .add_sized([38.0, 32.0], egui::Button::new("−"))
                    .on_hover_text("Rotate counter-clockwise")
                    .clicked()
                {
                    calibration.heading_deg -= calibration.angle_step_deg;
                    changed = true;
                }
                if ui
                    .add_sized([38.0, 32.0], egui::Button::new("+"))
                    .on_hover_text("Rotate clockwise")
                    .clicked()
                {
                    calibration.heading_deg += calibration.angle_step_deg;
                    changed = true;
                }
            });
            calibration.heading_deg = calibration.heading_deg.rem_euclid(360.0);
            ui.add_space(9.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(82.0, 82.0),
                    egui::Sense::hover(),
                );
                paint_heading_preview(
                    ui.painter(),
                    rect,
                    calibration.heading_deg as f32,
                );
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("True north → 0°")
                            .monospace()
                            .size(10.0),
                    );
                    ui.label(
                        egui::RichText::new("Clockwise is positive")
                            .size(10.0)
                            .color(egui::Color32::from_gray(130)),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for step in [0.1, 1.0, 5.0] {
                            ui.selectable_value(
                                &mut calibration.angle_step_deg,
                                step,
                                format_step(step, "°"),
                            );
                        }
                    });
                });
            });

            if changed {
                calibration.copied_at = None;
                next_action = Some(UiAction::ApplyLandmarkCalibration {
                    value: calibration.value(),
                    recenter: false,
                });
            }

            ui.add_space(20.0);
            ui.separator();
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("Reset transform").clicked() {
                    calibration.reset();
                    next_action = Some(UiAction::ApplyLandmarkCalibration {
                        value: calibration.value(),
                        recenter: false,
                    });
                }
                if ui.button("Frame model").clicked() {
                    next_action = Some(UiAction::ApplyLandmarkCalibration {
                        value: calibration.value(),
                        recenter: true,
                    });
                }
            });
            ui.add_space(12.0);
            let copied = calibration
                .copied_at
                .is_some_and(|at| at.elapsed().as_secs_f32() < 2.0);
            let copy_label = if copied {
                "✓  Alignment copied"
            } else {
                "Copy alignment data"
            };
            let copy_button = egui::Button::new(
                egui::RichText::new(copy_label).strong().color(egui::Color32::from_rgb(8, 12, 10)),
            )
            .fill(egui::Color32::from_rgb(198, 232, 93))
            .corner_radius(5.0);
            if ui
                .add_sized([ui.available_width(), 40.0], copy_button)
                .clicked()
            {
                ui.copy_text(calibration.clipboard_payload());
                calibration.copied_at = Some(vearth_platform::time::Instant::now());
            }
            ui.add_space(9.0);
            ui.label(
                egui::RichText::new(
                    "Copies versioned JSON with final lat/lon, heading, base offset, source values, and coordinate conventions.",
                )
                .size(10.0)
                .color(egui::Color32::from_gray(116)),
            );
        });
    next_action
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .monospace()
            .size(9.0)
            .strong()
            .color(egui::Color32::from_gray(132)),
    );
    ui.add_space(6.0);
}

fn calibration_axis_row(ui: &mut egui::Ui, label: &str, value: &mut f64, step: f64) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized([48.0, 30.0], egui::Label::new(label));
        if ui.add_sized([30.0, 30.0], egui::Button::new("−")).clicked() {
            *value -= step;
            changed = true;
        }
        let response = ui.add_sized(
            [(ui.available_width() - 36.0).max(76.0), 30.0],
            egui::DragValue::new(value)
                .speed(step)
                .suffix(" m")
                .max_decimals(2),
        );
        changed |= response.changed();
        if ui.add_sized([30.0, 30.0], egui::Button::new("+")).clicked() {
            *value += step;
            changed = true;
        }
    });
    changed
}

fn landmark_display_name(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn format_step(value: f64, unit: &str) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}{unit}")
    } else {
        format!("{value:.1}{unit}")
    }
}

fn paint_heading_preview(painter: &egui::Painter, rect: egui::Rect, heading_deg: f32) {
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.43;
    painter.circle_filled(center, radius, egui::Color32::from_rgb(19, 23, 23));
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(32)),
    );
    painter.text(
        center - egui::vec2(0.0, radius + 2.0),
        egui::Align2::CENTER_BOTTOM,
        "N",
        egui::FontId::monospace(9.0),
        egui::Color32::from_gray(140),
    );
    let angle = heading_deg.to_radians() - std::f32::consts::FRAC_PI_2;
    let direction = egui::vec2(angle.cos(), angle.sin());
    let tip = center + direction * radius * 0.72;
    let tail = center - direction * radius * 0.25;
    painter.line_segment(
        [tail, tip],
        egui::Stroke::new(2.5, egui::Color32::from_rgb(198, 232, 93)),
    );
    let side = egui::vec2(-direction.y, direction.x);
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip,
            tip - direction * 9.0 + side * 4.5,
            tip - direction * 9.0 - side * 4.5,
        ],
        egui::Color32::from_rgb(198, 232, 93),
        egui::Stroke::NONE,
    ));
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        "+Z",
        egui::FontId::monospace(8.0),
        egui::Color32::from_rgb(10, 14, 11),
    );
}

fn paint_loading_progress(painter: &egui::Painter, rect: egui::Rect, progress: f32) {
    let center = rect.center();
    let radius = 11.0;
    let width = 2.75;
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(width, egui::Color32::from_white_alpha(50)),
    );

    let sweep = progress.clamp(0.0, 1.0) * std::f32::consts::TAU;
    if sweep <= 0.0 {
        return;
    }
    let segments = ((sweep / std::f32::consts::TAU * 48.0).ceil() as usize).max(2);
    let points = (0..=segments)
        .map(|step| {
            let angle = -std::f32::consts::FRAC_PI_2 + sweep * step as f32 / segments as f32;
            center + egui::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(width, egui::Color32::from_rgb(66, 148, 255)),
    ));
}

struct PhaseMantaMotionBurst {
    armed: bool,
    started_at: Option<f64>,
}

impl Default for PhaseMantaMotionBurst {
    fn default() -> Self {
        Self {
            armed: true,
            started_at: None,
        }
    }
}

impl PhaseMantaMotionBurst {
    fn update(&mut self, play: Option<&PlayHud>, time_s: f64) -> Option<f32> {
        let Some(play) = play else {
            self.armed = true;
            self.started_at = None;
            return None;
        };
        if play.vehicle != VehicleKind::PhaseManta || !play.boost {
            self.armed = true;
            self.started_at = None;
            return None;
        }

        let stable_boost = play.boost_level >= PHASE_MANTA_BOOST_SETTLED;
        if self.armed && stable_boost {
            self.armed = false;
            self.started_at = Some(time_s);
        }

        let elapsed = time_s - self.started_at?;
        if elapsed >= PHASE_MANTA_MOTION_BURST_S {
            self.started_at = None;
            return None;
        }
        let progress = (elapsed / PHASE_MANTA_MOTION_BURST_S).clamp(0.0, 1.0) as f32;
        let fade_in = (progress / 0.12).clamp(0.0, 1.0);
        let fade_in = fade_in * fade_in * (3.0 - 2.0 * fade_in);
        let fade_out = ((progress - 0.14) / 0.86).clamp(0.0, 1.0);
        let fade_out = 1.0 - fade_out * fade_out * (3.0 - 2.0 * fade_out);
        Some(fade_in * fade_out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MotionStreak {
    tail: egui::Pos2,
    head: egui::Pos2,
    width: f32,
    alpha: u8,
}

fn motion_hash(index: usize, salt: u32) -> f32 {
    let mut value = (index as u32)
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(salt.wrapping_mul(0x85eb_ca6b));
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value as f32 / u32::MAX as f32
}

fn phase_manta_motion_streaks(viewport: egui::Rect, opacity: f32) -> Vec<MotionStreak> {
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 || viewport.width() <= 1.0 || viewport.height() <= 1.0 {
        return Vec::new();
    }

    let vanishing_point = egui::pos2(
        viewport.center().x,
        viewport.top() + viewport.height() * 0.47,
    );
    let lines_per_side = PHASE_MANTA_MOTION_LINE_COUNT / 4;
    (0..PHASE_MANTA_MOTION_LINE_COUNT)
        .map(|index| {
            let side = index / lines_per_side;
            let slot = index % lines_per_side;
            let jitter = (motion_hash(index, 0) - 0.5) * 0.56;
            let along = ((slot as f32 + 0.5 + jitter) / lines_per_side as f32).clamp(0.02, 0.98);
            let edge = match side {
                0 => egui::pos2(
                    egui::lerp(viewport.left()..=viewport.right(), along),
                    viewport.top(),
                ),
                1 => egui::pos2(
                    viewport.right(),
                    egui::lerp(viewport.top()..=viewport.bottom(), along),
                ),
                2 => egui::pos2(
                    egui::lerp(viewport.right()..=viewport.left(), along),
                    viewport.bottom(),
                ),
                _ => egui::pos2(
                    viewport.left(),
                    egui::lerp(viewport.bottom()..=viewport.top(), along),
                ),
            };
            let ray = edge - vanishing_point;
            let direction = ray.normalized();
            let length_variation = 0.84 + motion_hash(index, 2) * 0.32;
            let span = 0.06 + 0.10 * length_variation;
            let tail = vanishing_point + ray * (1.0 - span);
            let head = edge + direction * viewport.height() * 0.035;
            let width = 1.10 * (0.78 + motion_hash(index, 3) * 0.34);
            let alpha = (82.0 * (0.78 + motion_hash(index, 5) * 0.22) * opacity)
                .round()
                .clamp(0.0, 82.0) as u8;
            MotionStreak {
                tail,
                head,
                width,
                alpha,
            }
        })
        .collect()
}

fn paint_phase_manta_motion_streaks(painter: &egui::Painter, streaks: &[MotionStreak]) {
    let mut mesh = egui::Mesh::default();
    // Zero-alpha borders provide a continuous falloff at the tip, the outer
    // end, and both sides without a hard stroked edge around the effect.
    const LENGTH_PROFILE: [(f32, f32); 8] = [
        (0.0, 0.0),
        (0.08, 0.12),
        (0.20, 0.40),
        (0.36, 0.78),
        (0.56, 1.0),
        (0.74, 0.80),
        (0.90, 0.40),
        (1.0, 0.0),
    ];
    const WIDTH_PROFILE: [(f32, f32); 5] = [
        (-1.0, 0.0),
        (-0.4, 0.30),
        (0.0, 1.0),
        (0.4, 0.30),
        (1.0, 0.0),
    ];
    mesh.reserve_vertices(streaks.len() * LENGTH_PROFILE.len() * WIDTH_PROFILE.len());
    mesh.reserve_triangles(streaks.len() * (LENGTH_PROFILE.len() - 1) * 8);
    for streak in streaks {
        let delta = streak.head - streak.tail;
        let length = delta.length().max(1.0e-4);
        let normal = egui::vec2(-delta.y / length, delta.x / length);
        let first = mesh.vertices.len() as u32;
        for (progress, longitudinal_alpha) in LENGTH_PROFILE {
            let centre = streak.tail + delta * progress;
            let width = streak.width * (0.35 + 0.65 * progress) * 2.0;
            for (offset, transverse_alpha) in WIDTH_PROFILE {
                let alpha =
                    (f32::from(streak.alpha) * longitudinal_alpha * transverse_alpha).round() as u8;
                mesh.colored_vertex(
                    centre + normal * offset * width,
                    egui::Color32::from_rgba_unmultiplied(205, 228, 240, alpha),
                );
            }
        }
        for row in 0..LENGTH_PROFILE.len() as u32 - 1 {
            for column in 0..WIDTH_PROFILE.len() as u32 - 1 {
                let a = first + row * WIDTH_PROFILE.len() as u32 + column;
                let b = a + WIDTH_PROFILE.len() as u32;
                mesh.add_triangle(a, b, a + 1);
                mesh.add_triangle(a + 1, b, b + 1);
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_camera(
    painter: &egui::Painter,
    rect: egui::Rect,
    hovered: bool,
    pressed: bool,
    feedback: f32,
) {
    let center = rect.center() + egui::vec2(0.0, if pressed { 1.5 } else { 0.0 });
    let radius = if pressed { 18.0 } else { 20.5 };
    let ink = HUD_INK.lerp_to_gamma(egui::Color32::from_rgb(36, 35, 31), feedback);
    painter.circle_filled(
        center,
        radius,
        egui::Color32::from_black_alpha(if hovered { 72 } else { 24 })
            .lerp_to_gamma(HUD_INK, feedback),
    );
    painter.circle_stroke(
        center + egui::vec2(0.0, 1.0),
        radius,
        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(80)),
    );
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(
            1.0 + feedback,
            egui::Color32::from_rgba_unmultiplied(244, 240, 229, if hovered { 180 } else { 96 })
                .lerp_to_gamma(HUD_INK, feedback),
        ),
    );
    let stroke = egui::Stroke::new(1.5, ink);
    painter.rect_stroke(
        egui::Rect::from_center_size(center + egui::vec2(0.0, 1.0), egui::vec2(22.0, 15.0)),
        3.0,
        stroke,
        egui::StrokeKind::Middle,
    );
    painter.line_segment(
        [
            center + egui::vec2(-4.0, -9.0),
            center + egui::vec2(4.0, -9.0),
        ],
        egui::Stroke::new(3.0, ink),
    );
    painter.circle_stroke(center + egui::vec2(0.0, 1.0), 4.0, stroke);
}

fn paint_compass(painter: &egui::Painter, rect: egui::Rect, heading_deg: f64, hovered: bool) {
    let center = egui::pos2(rect.center().x, rect.bottom() - 22.0);
    let radius = 20.5;
    let north_label = painter.layout_no_wrap(
        "N".to_owned(),
        egui::FontId::new(11.0, plex_medium_family()),
        HUD_INK,
    );
    hud_text(
        painter,
        egui::pos2(rect.center().x - north_label.size().x / 2.0, rect.top()),
        north_label,
        HUD_INK,
    );
    painter.circle_filled(
        center,
        radius,
        egui::Color32::from_black_alpha(if hovered { 72 } else { 24 }),
    );
    painter.circle_stroke(
        center + egui::vec2(0.0, 1.0),
        radius,
        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(80)),
    );
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(244, 240, 229, if hovered { 180 } else { 96 }),
        ),
    );
    for direction in [
        egui::vec2(0.0, -1.0),
        egui::vec2(1.0, 0.0),
        egui::vec2(0.0, 1.0),
        egui::vec2(-1.0, 0.0),
    ] {
        painter.line_segment(
            [center + direction * 17.0, center + direction * 19.0],
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(100)),
        );
    }
    let heading = heading_deg.to_radians() as f32;
    let north = egui::vec2(-heading.sin(), -heading.cos());
    let east = egui::vec2(-north.y, north.x);
    let tip = 14.5;
    let shoulder = 3.2;
    // Both halves use these exact same points as their base edge. Keeping
    // one shared edge avoids either overlap or a hairline gap when rotated.
    let base_left = center - east * shoulder;
    let base_right = center + east * shoulder;

    painter.add(egui::Shape::convex_polygon(
        vec![center + north * tip, base_right, base_left],
        egui::Color32::from_rgb(220, 68, 65),
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![center - north * tip, base_left, base_right],
        HUD_INK,
        egui::Stroke::NONE,
    ));
}

pub struct UiPaint {
    primitives: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
}

/// Accumulates egui texture work until a swapchain frame is actually
/// available to paint. `TexturesDelta::append` preserves the required
/// allocation-before-partial-update order across skipped frames.
#[derive(Default)]
pub(crate) struct PendingTextureDeltas {
    delta: egui::TexturesDelta,
}

impl PendingTextureDeltas {
    pub(crate) fn append(&mut self, newer: egui::TexturesDelta) {
        self.delta.append(newer);
    }

    pub(crate) fn take(&mut self) -> egui::TexturesDelta {
        std::mem::take(&mut self.delta)
    }
}

fn fmt_alt(alt_m: f64) -> String {
    if alt_m >= 10_000.0 {
        format!("{:.0} km", alt_m / 1000.0)
    } else if alt_m >= 1000.0 {
        format!("{:.1} km", alt_m / 1000.0)
    } else {
        format!("{alt_m:.0} m")
    }
}

fn fmt_speed(speed_ms: f64) -> String {
    if speed_ms >= 1_000.0 {
        format!("{:.2} km/s", speed_ms / 1_000.0)
    } else {
        format!("{speed_ms:.0} m/s")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CalibrationUi, CellLoadProgress, NOTO_SANS_CJK, PHASE_MANTA_MOTION_BURST_S,
        PHASE_MANTA_MOTION_LINE_COUNT, PLEX_MONO_REGULAR, PLEX_SANS_MEDIUM, PLEX_SANS_REGULAR,
        PendingTextureDeltas, PhaseMantaMotionBurst, PlayHud, configure_fonts, fmt_alt, fmt_speed,
        launch_loading_pulse, phase_manta_motion_streaks, plex_medium_family,
        vearth_font_definitions,
    };

    #[test]
    fn borderless_solar_controls_keep_every_direct_action() {
        let ctx = egui::Context::default();
        configure_fonts(&ctx);
        super::configure_game_style(&ctx);
        let time = crate::local_time::LocalTimeHud {
            label: "17:00 CEST".into(),
            detail: "Europe/Rome".into(),
        };
        let frame = |events: Vec<egui::Event>| {
            let mut action = super::UiAction::None;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 240.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(460.0);
                    action = super::show_solar_controls(
                        ui,
                        &super::SunHud {
                            paused: false,
                            time_scale: 1.0,
                        },
                        Some(&time),
                    );
                },
            );
            (output, action)
        };
        let (output, _) = frame(vec![]);
        let center = |label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing visible control {label}"))
        };
        let back = center("−1h");
        let forward = center("+1h");
        let pause = (back + forward.to_vec2()) * 0.5;
        use super::UiAction;
        for (position, expected) in [
            (back, UiAction::ShiftSunHours(-1.0)),
            (forward, UiAction::ShiftSunHours(1.0)),
            (center("1×"), UiAction::SetSunTimeScale(1.0)),
            (center("60×"), UiAction::SetSunTimeScale(60.0)),
            (center("600×"), UiAction::SetSunTimeScale(600.0)),
            (pause, UiAction::ToggleSunPause),
        ] {
            let pointer = |pressed| egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(vec![egui::Event::PointerMoved(position), pointer(true)]);
            let (_, action) = frame(vec![pointer(false)]);
            match (action, expected) {
                (UiAction::ShiftSunHours(actual), UiAction::ShiftSunHours(expected)) => {
                    assert_eq!(actual, expected)
                }
                (UiAction::SetSunTimeScale(actual), UiAction::SetSunTimeScale(expected)) => {
                    assert_eq!(actual, expected)
                }
                (UiAction::ToggleSunPause, UiAction::ToggleSunPause) => {}
                _ => panic!("control at {position:?} did not dispatch its action"),
            }
        }

        assert!(
            (back.y - center("600×").y).abs() < 2.0,
            "normal-width controls stay on one line"
        );
    }

    #[test]
    fn ibm_plex_is_the_primary_ui_font_family() {
        let fonts = vearth_font_definitions();
        assert_eq!(
            fonts
                .families
                .get(&egui::FontFamily::Proportional)
                .and_then(|family| family.first())
                .map(String::as_str),
            Some(PLEX_SANS_REGULAR),
        );
        assert_eq!(
            fonts
                .families
                .get(&egui::FontFamily::Monospace)
                .and_then(|family| family.first())
                .map(String::as_str),
            Some(PLEX_MONO_REGULAR),
        );
        assert_eq!(
            fonts
                .families
                .get(&plex_medium_family())
                .and_then(|family| family.first())
                .map(String::as_str),
            Some(PLEX_SANS_MEDIUM),
        );
        for family in [
            egui::FontFamily::Proportional,
            egui::FontFamily::Monospace,
            plex_medium_family(),
        ] {
            assert!(
                fonts
                    .families
                    .get(&family)
                    .is_some_and(|fonts| fonts.iter().any(|font| font == NOTO_SANS_CJK)),
                "{family:?} has the shared CJK fallback"
            );
        }
    }

    #[test]
    fn ui_font_stack_renders_chinese_japanese_and_korean_place_names() {
        let ctx = egui::Context::default();
        configure_fonts(&ctx);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        ctx.fonts_mut(|fonts| {
            for family in [
                egui::FontFamily::Proportional,
                egui::FontFamily::Monospace,
                plex_medium_family(),
            ] {
                let font_id = egui::FontId::new(14.0, family.clone());
                assert!(
                    fonts.has_glyphs(&font_id, "上海 富良野 서울"),
                    "{family:?} renders representative CJK place names"
                );
            }
        });
    }

    #[test]
    fn skipped_paints_preserve_texture_allocation_before_partial_update() {
        let id = egui::TextureId::Managed(0);
        let full = egui::epaint::ImageDelta::full(
            egui::ColorImage::filled([2, 2], egui::Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        let partial = egui::epaint::ImageDelta::partial(
            [1, 1],
            egui::ColorImage::filled([1, 1], egui::Color32::BLACK),
            egui::TextureOptions::LINEAR,
        );
        let mut pending = PendingTextureDeltas::default();
        pending.append(egui::TexturesDelta {
            set: vec![(id, full)],
            free: Vec::new(),
        });
        pending.append(egui::TexturesDelta {
            set: vec![(id, partial)],
            free: Vec::new(),
        });

        let queued = pending.take();
        assert_eq!(queued.set.len(), 2);
        assert!(queued.set[0].1.pos.is_none());
        assert_eq!(queued.set[1].1.pos, Some([1, 1]));
        assert!(pending.take().is_empty());
    }

    #[test]
    fn cell_load_progress_grows_as_remaining_count_falls_and_resets_at_zero() {
        let mut progress = CellLoadProgress::default();
        assert_eq!(progress.update(8), Some(0.0));
        assert_eq!(progress.update(6), Some(0.25));
        assert_eq!(progress.update(2), Some(0.75));
        assert_eq!(progress.update(0), None);
        assert_eq!(progress.update(4), Some(0.0));
    }

    #[test]
    fn controls_screen_reuses_the_launch_legend_and_adds_a_resume_hint() {
        let ctx = egui::Context::default();
        configure_fonts(&ctx);
        for resumable in [false, true] {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                super::show_controls_screen(ui, 1.0, None, resumable);
            });
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect();
            for label in [
                "FLIGHT CONTROLS",
                "W  A  S  D",
                "WORLD MAP",
                "CYCLE VEHICLE",
            ] {
                assert!(labels.contains(&label), "missing legend label: {label}");
            }
            assert_eq!(labels.contains(&"ESC — RETURN TO EXPLORING"), resumable);
            let wordmark = crate::launch_wordmark::texture(&ctx);
            assert!(output.shapes.iter().any(|shape| matches!(
                &shape.shape,
                egui::Shape::Mesh(mesh) if mesh.texture_id == wordmark.id()
            )));
        }
    }

    #[test]
    fn launch_loading_pulse_is_clipped_to_the_top_border() {
        let panel = egui::Rect::from_min_size(egui::pos2(40.0, 60.0), egui::vec2(800.0, 300.0));
        for elapsed in [0.0, 0.5, 1.5, 3.0] {
            let pulse = launch_loading_pulse(panel, elapsed);
            assert_eq!(pulse.top(), panel.top());
            assert_eq!(pulse.bottom(), panel.top() + 2.0);
            assert!(pulse.left() >= panel.left());
            assert!(pulse.right() <= panel.right());
        }
    }

    #[test]
    fn flight_hud_values_use_compact_units() {
        assert_eq!(fmt_speed(342.4), "342 m/s");
        assert_eq!(fmt_speed(1_240.0), "1.24 km/s");
        assert_eq!(fmt_alt(842.0), "842 m");
        assert_eq!(fmt_alt(1_240.0), "1.2 km");
    }

    #[test]
    fn phase_manta_motion_lines_are_peripheral_and_fade_together() {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
        let boost = phase_manta_motion_streaks(viewport, 1.0);
        assert_eq!(boost.len(), PHASE_MANTA_MOTION_LINE_COUNT);
        let vanishing_point = egui::pos2(
            viewport.center().x,
            viewport.top() + viewport.height() * 0.47,
        );
        for streak in &boost {
            assert!(viewport.contains(streak.tail));
            assert!(
                !viewport.contains(streak.head),
                "motion streak should remain clipped into a viewport edge"
            );
            let normalized = egui::vec2(
                (streak.tail.x - vanishing_point.x) / (viewport.width() * 0.5),
                (streak.tail.y - vanishing_point.y) / (viewport.height() * 0.5),
            );
            assert!(
                normalized.length() > 0.78,
                "motion streak entered the central viewing area"
            );
            assert!(streak.head.distance(vanishing_point) > streak.tail.distance(vanishing_point));
        }
        let faded = phase_manta_motion_streaks(viewport, 0.35);
        assert!(
            boost
                .iter()
                .zip(&faded)
                .all(|(before, after)| before.tail == after.tail
                    && before.head == after.head
                    && before.width == after.width),
            "motion streaks should fade in place instead of firing across the viewport"
        );
        assert!(
            boost
                .iter()
                .zip(&faded)
                .all(|(before, after)| before.alpha > after.alpha),
            "the shared burst opacity should fade every streak together"
        );
        assert!(
            phase_manta_motion_streaks(viewport, 0.0).is_empty(),
            "the completed burst should leave no persistent streaks"
        );
    }

    #[test]
    fn phase_manta_motion_burst_fires_once_until_boost_is_rearmed() {
        let mut play = PlayHud {
            vehicle: crate::vehicles::VehicleKind::Kestrel,
            vehicle_name: "PHASE MANTA",
            phased: true,
            speed_ms: 12_000.0,
            airspeed_ms: 12_000.0,
            altitude_m: 1_000.0,
            boost: true,
            boost_level: 1.0,
        };
        let mut burst = PhaseMantaMotionBurst::default();
        assert_eq!(burst.update(Some(&play), 10.0), None);

        play.vehicle = crate::vehicles::VehicleKind::PhaseManta;
        play.boost_level = 0.8;
        assert_eq!(burst.update(Some(&play), 10.05), None);
        play.boost_level = 1.0;
        assert_eq!(burst.update(Some(&play), 10.1), Some(0.0));
        assert_eq!(
            burst.update(Some(&play), 10.1 + PHASE_MANTA_MOTION_BURST_S * 0.12),
            Some(1.0)
        );
        let fading = burst
            .update(Some(&play), 10.1 + PHASE_MANTA_MOTION_BURST_S * 0.60)
            .unwrap();
        assert!(fading > 0.0 && fading < 1.0);
        assert_eq!(
            burst.update(Some(&play), 10.1 + PHASE_MANTA_MOTION_BURST_S),
            None
        );
        assert_eq!(burst.update(Some(&play), 15.0), None);

        play.boost = false;
        assert_eq!(burst.update(Some(&play), 15.1), None);
        play.boost = true;
        assert_eq!(burst.update(Some(&play), 15.2), Some(0.0));
    }

    #[test]
    fn calibration_clipboard_payload_is_versioned_and_resolved() {
        let landmark = vearth_gen::landmarks::LANDMARKS
            .iter()
            .find(|landmark| landmark.name == "eiffel-tower")
            .unwrap();
        let mut calibration = CalibrationUi::new(landmark);
        calibration.east_m = 2.5;
        calibration.north_m = -4.0;
        calibration.up_m = 1.25;
        calibration.heading_deg = 405.0;
        let payload: serde_json::Value =
            serde_json::from_str(&calibration.clipboard_payload()).unwrap();
        assert_eq!(payload["schema"], "vearth.landmark-alignment/v1");
        assert_eq!(payload["landmark"], "eiffel-tower");
        assert_eq!(payload["placement"]["headingDeg"], 45.0);
        assert_eq!(payload["placement"]["base"]["offsetM"], 1.25);
        assert_eq!(payload["editorOffsetEnuM"]["east"], 2.5);
    }
}
