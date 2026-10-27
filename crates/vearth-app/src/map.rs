//! Full-viewport, interactive Web Mercator position map.
//!
//! Natural Earth paths are tessellated once into static GPU buffers. Panning
//! and zooming only update a 32-byte transform uniform.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use lyon_tessellation::{
    FillOptions, FillRule, FillTessellator, FillVertex, FillVertexConstructor,
    geometry_builder::{BuffersBuilder, VertexBuffers},
    math::point,
    path::Path,
};
use wgpu::util::DeviceExt;

use vearth_data::place_search::{MapPlace, PlaceKind};

const MERCATOR_LAT_LIMIT: f64 = 85.051_128_78;
const INITIAL_ZOOM: f32 = 4.0;
const MIN_ZOOM: f32 = 1.0;
const MAX_ZOOM: f32 = 256.0;
const LABEL_BUCKET_ZOOM: u8 = 8;
const LABEL_BUCKETS: usize = 1 << LABEL_BUCKET_ZOOM;
const LABEL_FADE_RATIO: f32 = 1.5;
const LABEL_EDGE_INSET: f32 = 14.0;
const LOCAL_LOCATION_LABEL_MIN_ZOOM: f32 = 40.0;
const EARTH_RADIUS_M: f64 = 6_378_137.0;
const SCALE_BAR_TARGET_PX: f32 = 110.0;

#[derive(Clone)]
pub(crate) struct MapView {
    /// World coordinate held at the viewport centre, in normalized Mercator.
    center: egui::Vec2,
    focus: egui::Vec2,
    zoom: f32,
}

/// Owns the live map camera plus the last viewport used during play. Explore
/// mode continues to recenter on every open, while Enter toggles in play mode
/// round-trip the exact pan and zoom independently of other map use.
#[derive(Default)]
pub(crate) struct MapNavigation {
    view: MapView,
    saved_play_view: Option<MapView>,
    opened_in_play: bool,
}

impl MapNavigation {
    pub(crate) fn open(&mut self, lat_deg: f64, lon_deg: f64, in_play: bool) {
        self.opened_in_play = in_play;
        if in_play && let Some(saved) = self.saved_play_view.take() {
            self.view = saved;
        } else {
            self.view.track(lat_deg, lon_deg);
        }
    }

    pub(crate) fn close(&mut self) {
        if self.opened_in_play {
            self.saved_play_view = Some(self.view.clone());
        }
        self.opened_in_play = false;
    }

    pub(crate) fn view_mut(&mut self) -> &mut MapView {
        &mut self.view
    }
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            center: egui::vec2(0.5, 0.5),
            focus: egui::vec2(0.5, 0.5),
            zoom: INITIAL_ZOOM,
        }
    }
}

impl MapView {
    pub(crate) fn track(&mut self, lat_deg: f64, lon_deg: f64) {
        self.focus = project(lat_deg, lon_deg);
        self.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.center = self.focus;
        self.zoom = INITIAL_ZOOM;
    }

    fn update(&mut self, ctx: &egui::Context, viewport: egui::Rect) {
        let base_size = viewport.width().max(1.0);
        let min_zoom = MIN_ZOOM.max(viewport.height() / base_size);
        let (pointer, pointer_delta, dragging, scroll_y) = ctx.input(|input| {
            (
                input.pointer.hover_pos(),
                input.pointer.delta(),
                input.pointer.primary_down(),
                input.smooth_scroll_delta.y,
            )
        });

        if dragging && pointer.is_some_and(|position| viewport.contains(position)) {
            let map_size = base_size * self.zoom;
            self.center -= pointer_delta / map_size;
            self.center.x = self.center.x.rem_euclid(1.0);
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if pointer.is_some_and(|position| viewport.contains(position)) {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
        }

        if scroll_y.abs() > f32::EPSILON
            && let Some(pointer) = pointer.filter(|position| viewport.contains(*position))
        {
            let old_size = base_size * self.zoom;
            let world_under_pointer = self.center + (pointer - viewport.center()) / old_size;
            self.zoom = (self.zoom * (scroll_y * 0.004).exp()).clamp(min_zoom, MAX_ZOOM);
            let new_size = base_size * self.zoom;
            self.center = world_under_pointer - (pointer - viewport.center()) / new_size;
            self.center.x = self.center.x.rem_euclid(1.0);
        }

        self.zoom = self.zoom.clamp(min_zoom, MAX_ZOOM);
        let map_size = base_size * self.zoom;
        self.center.y = constrained_center(self.center.y, viewport.height(), map_size);
    }

    fn map_size(&self, viewport: egui::Rect) -> f32 {
        viewport.width().max(1.0) * self.zoom
    }
}

fn constrained_center(value: f32, viewport_extent: f32, map_extent: f32) -> f32 {
    if map_extent <= viewport_extent {
        0.5
    } else {
        let inset = viewport_extent / (2.0 * map_extent);
        value.clamp(inset, 1.0 - inset)
    }
}

const OCEAN: egui::Color32 = egui::Color32::from_rgb(198, 232, 245);
const LAND: [u8; 4] = [238, 241, 211, 255];
const LAKE: [u8; 4] = [185, 222, 237, 255];
const GRID: [u8; 4] = [139, 197, 219, 145];
const BORDER: [u8; 4] = [106, 139, 145, 220];
const LAKE_BORDER: [u8; 4] = [112, 169, 190, 205];

pub(crate) struct MapRenderer;

/// Spatially bucketed place names for cheap per-frame map queries. The
/// catalog is assembled off the UI thread and then remains immutable.
pub(crate) struct MapPlaceCatalog {
    buckets: Vec<Vec<LabelPlace>>,
}

#[derive(Debug)]
struct LabelPlace {
    name: String,
    world: egui::Vec2,
    min_zoom: f32,
    population: u32,
    geoname_id: u32,
}

impl MapPlaceCatalog {
    pub(crate) fn new(places: Vec<MapPlace>) -> Self {
        let mut buckets = (0..LABEL_BUCKETS * LABEL_BUCKETS)
            .map(|_| Vec::new())
            .collect::<Vec<_>>();
        for place in places {
            let world = project(place.latitude_deg, place.longitude_deg);
            let x = ((world.x * LABEL_BUCKETS as f32).floor() as usize).min(LABEL_BUCKETS - 1);
            let y = ((world.y * LABEL_BUCKETS as f32).floor() as usize).min(LABEL_BUCKETS - 1);
            buckets[y * LABEL_BUCKETS + x].push(LabelPlace {
                name: place.name,
                world,
                min_zoom: place_label_min_zoom(place.kind, place.population),
                population: place.population,
                geoname_id: place.geoname_id,
            });
        }
        for bucket in &mut buckets {
            bucket.sort_by(|a, b| {
                a.min_zoom
                    .total_cmp(&b.min_zoom)
                    .then_with(|| b.population.cmp(&a.population))
                    .then_with(|| a.geoname_id.cmp(&b.geoname_id))
            });
        }
        Self { buckets }
    }
}

fn place_label_min_zoom(kind: PlaceKind, population: u32) -> f32 {
    if kind == PlaceKind::Capital && population < 1_000_000 {
        return 2.5;
    }
    match population {
        5_000_000.. => 1.0,
        1_000_000.. => 2.5,
        500_000.. => 4.0,
        250_000.. => 7.0,
        100_000.. => 12.0,
        50_000.. => 22.0,
        25_000.. => 40.0,
        10_000.. => 72.0,
        5_000.. => 128.0,
        _ => 210.0,
    }
}

impl MapRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        renderer: &mut egui_wgpu::Renderer,
    ) -> Self {
        let started = vearth_platform::time::Instant::now();
        let geometry =
            Geometry::decode(vearth_platform::asset_bytes!("../assets/mercator-map.bin"));
        let mesh = GpuMapMesh::build(&geometry);
        log::info!(
            "map GPU mesh: {} fill vertices, {} triangles, {} line segments in {:.1} ms",
            mesh.fill.vertices.len(),
            mesh.fill.indices.len() / 3,
            mesh.lines.len(),
            started.elapsed().as_secs_f64() * 1000.0,
        );
        renderer
            .callback_resources
            .insert(MapGpuResources::new(device, format, mesh));
        Self
    }

    fn paint(
        &self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        viewport: egui::Rect,
        view: &MapView,
    ) {
        painter.rect_filled(viewport, 0.0, OCEAN);
        let pixels_per_point = ctx.pixels_per_point();
        let uniform = MapUniform {
            center_zoom_aspect: [
                view.center.x,
                view.center.y,
                view.zoom,
                viewport.width() / viewport.height().max(1.0),
            ],
            viewport_ppp: [
                viewport.width() * pixels_per_point,
                viewport.height() * pixels_per_point,
                pixels_per_point,
                0.0,
            ],
        };
        painter.add(egui_wgpu::Callback::new_paint_callback(
            viewport,
            MapCallback { uniform },
        ));
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MapUniform {
    center_zoom_aspect: [f32; 4],
    viewport_ppp: [f32; 4],
}

struct MapCallback {
    uniform: MapUniform,
}

impl egui_wgpu::CallbackTrait for MapCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &MapGpuResources = callback_resources
            .get()
            .expect("map GPU resources installed");
        queue.write_buffer(&resources.uniform, 0, bytemuck::bytes_of(&self.uniform));
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &MapGpuResources = callback_resources
            .get()
            .expect("map GPU resources installed");
        resources.paint(render_pass);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FillGpuVertex {
    position: [f32; 2],
    color: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LineGpuInstance {
    start: [f32; 2],
    end: [f32; 2],
    color: [f32; 4],
    width_points: f32,
    _padding: [f32; 3],
}

struct FillVertexWithColor([u8; 4]);

impl FillVertexConstructor<FillGpuVertex> for FillVertexWithColor {
    fn new_vertex(&mut self, vertex: FillVertex<'_>) -> FillGpuVertex {
        FillGpuVertex {
            position: vertex.position().to_array(),
            color: self.0,
        }
    }
}

struct GpuMapMesh {
    fill: VertexBuffers<FillGpuVertex, u32>,
    lines: Vec<LineGpuInstance>,
}

impl GpuMapMesh {
    fn build(geometry: &Geometry) -> Self {
        let mut fill = VertexBuffers::<FillGpuVertex, u32>::new();
        let mut tessellator = FillTessellator::new();
        for feature in &geometry.countries {
            tessellate_feature(&mut tessellator, &mut fill, feature, LAND);
        }
        for feature in &geometry.lakes {
            tessellate_feature(&mut tessellator, &mut fill, feature, LAKE);
        }

        let mut lines = Vec::new();
        for step in 1..12 {
            let x = step as f32 / 12.0;
            add_wrapped_line(&mut lines, [x, 0.0], [x, 1.0], GRID, 0.8);
        }
        for latitude in [-60.0, -30.0, 0.0, 30.0, 60.0] {
            let y = project(latitude, 0.0).y;
            add_wrapped_line(&mut lines, [0.0, y], [1.0, y], GRID, 0.8);
        }
        for feature in &geometry.countries {
            add_feature_lines(&mut lines, feature, BORDER, 1.05);
        }
        for feature in &geometry.lakes {
            add_feature_lines(&mut lines, feature, LAKE_BORDER, 0.8);
        }
        Self { fill, lines }
    }
}

fn tessellate_feature(
    tessellator: &mut FillTessellator,
    output: &mut VertexBuffers<FillGpuVertex, u32>,
    feature: &Feature,
    color: [u8; 4],
) {
    let mut builder = Path::builder();
    let mut has_path = false;
    for polygon in &feature.polygons {
        for ring in polygon {
            let points = open_ring(ring);
            if points.len() < 3 {
                continue;
            }
            has_path = true;
            builder.begin(point(points[0][0], points[0][1]));
            for position in &points[1..] {
                builder.line_to(point(position[0], position[1]));
            }
            builder.end(true);
        }
    }
    if !has_path {
        return;
    }
    let path = builder.build();
    let options = FillOptions::default().with_fill_rule(FillRule::EvenOdd);
    if let Err(error) = tessellator.tessellate_path(
        &path,
        &options,
        &mut BuffersBuilder::new(output, FillVertexWithColor(color)),
    ) {
        log::warn!("map feature tessellation failed: {error:?}");
    }
}

fn add_feature_lines(
    output: &mut Vec<LineGpuInstance>,
    feature: &Feature,
    color: [u8; 4],
    width_points: f32,
) {
    for polygon in &feature.polygons {
        for ring in polygon {
            let points = open_ring(ring);
            if points.len() < 2 {
                continue;
            }
            for segment in points.windows(2) {
                add_wrapped_line(output, segment[0], segment[1], color, width_points);
            }
            add_wrapped_line(
                output,
                *points.last().expect("non-empty ring"),
                points[0],
                color,
                width_points,
            );
        }
    }
}

fn open_ring(ring: &[[f32; 2]]) -> &[[f32; 2]] {
    if ring.len() > 1 && ring.first() == ring.last() {
        &ring[..ring.len() - 1]
    } else {
        ring
    }
}

fn add_wrapped_line(
    output: &mut Vec<LineGpuInstance>,
    start: [f32; 2],
    end: [f32; 2],
    color: [u8; 4],
    width_points: f32,
) {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    if dx * dx + dy * dy <= f32::EPSILON {
        return;
    }
    let color = color.map(|channel| f32::from(channel) / 255.0);
    for shift in [-1.0, 0.0, 1.0] {
        output.push(LineGpuInstance {
            start: [start[0] + shift, start[1]],
            end: [end[0] + shift, end[1]],
            color,
            width_points,
            _padding: [0.0; 3],
        });
    }
}

struct MapGpuResources {
    fill_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    fill_vertices: wgpu::Buffer,
    fill_indices: wgpu::Buffer,
    fill_index_count: u32,
    line_instances: wgpu::Buffer,
    line_instance_count: u32,
}

impl MapGpuResources {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, mesh: GpuMapMesh) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map vectors"),
            source: wgpu::ShaderSource::Wgsl(include_str!("map.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map vectors"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<MapUniform>() as u64),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map vectors"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let target = Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        });
        let fill_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map vector fills"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("fill_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<FillGpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Unorm8x4,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("color_fs"),
                compilation_options: Default::default(),
                targets: std::slice::from_ref(&target),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map vector lines"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("line_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineGpuInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                        3 => Float32,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("color_fs"),
                compilation_options: Default::default(),
                targets: &[target],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("map transform"),
            contents: bytemuck::bytes_of(&MapUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map vectors"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let fill_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("map fill vertices"),
            contents: bytemuck::cast_slice(&mesh.fill.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let fill_indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("map fill indices"),
            contents: bytemuck::cast_slice(&mesh.fill.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let line_instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("map line instances"),
            contents: bytemuck::cast_slice(&mesh.lines),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Self {
            fill_pipeline,
            line_pipeline,
            bind_group,
            uniform,
            fill_vertices,
            fill_indices,
            fill_index_count: mesh.fill.indices.len() as u32,
            line_instances,
            line_instance_count: mesh.lines.len() as u32,
        }
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_pipeline(&self.fill_pipeline);
        render_pass.set_vertex_buffer(0, self.fill_vertices.slice(..));
        render_pass.set_index_buffer(self.fill_indices.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..self.fill_index_count, 0, 0..3);

        render_pass.set_pipeline(&self.line_pipeline);
        render_pass.set_vertex_buffer(0, self.line_instances.slice(..));
        render_pass.draw(0..6, 0..self.line_instance_count);
    }
}

pub(crate) fn paint_map(
    ctx: &egui::Context,
    painter: &egui::Painter,
    viewport: egui::Rect,
    renderer: &mut MapRenderer,
    view: &mut MapView,
    lat_deg: f64,
    lon_deg: f64,
    heading_deg: f64,
    places: Option<&MapPlaceCatalog>,
    current_location_name: Option<&str>,
) {
    view.update(ctx, viewport);
    renderer.paint(ctx, painter, viewport, view);

    paint_place_labels(
        painter,
        viewport,
        view,
        places,
        current_location_name,
        lat_deg,
        lon_deg,
    );

    let world = project(lat_deg, lon_deg);
    let delta_x = wrapped_delta_x(world.x, view.center.x);
    let marker =
        viewport.center() + egui::vec2(delta_x, world.y - view.center.y) * view.map_size(viewport);
    if viewport.expand(18.0).contains(marker) {
        paint_position_marker(painter, marker, heading_deg);
    }
    paint_scale_bar(painter, viewport, view);
}

fn paint_scale_bar(painter: &egui::Painter, viewport: egui::Rect, view: &MapView) {
    let Some(scale) = map_scale(viewport, view) else {
        return;
    };
    let left = viewport.left() + 20.0;
    let baseline = viewport.bottom() - 20.0;
    let start = egui::pos2(left, baseline);
    let end = egui::pos2(left + scale.pixels, baseline);
    let label = painter.layout_no_wrap(
        scale.label,
        egui::FontId::monospace(9.0),
        egui::Color32::PLACEHOLDER,
    );
    let bounds = egui::Rect::from_min_max(
        egui::pos2(left - 6.0, baseline - label.size().y - 9.0),
        egui::pos2(end.x + 6.0, baseline + 6.0),
    );
    painter.rect_filled(
        bounds,
        3.0,
        egui::Color32::from_rgba_unmultiplied(236, 244, 226, 210),
    );
    let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(39, 65, 65));
    painter.line_segment([start, end], stroke);
    painter.line_segment(
        [start + egui::vec2(0.0, -4.0), start + egui::vec2(0.0, 2.0)],
        stroke,
    );
    painter.line_segment(
        [end + egui::vec2(0.0, -4.0), end + egui::vec2(0.0, 2.0)],
        stroke,
    );
    painter.galley(
        egui::pos2(left, baseline - label.size().y - 5.0),
        label,
        egui::Color32::from_rgb(39, 65, 65),
    );
}

#[derive(Debug, PartialEq)]
struct MapScale {
    metres: f64,
    pixels: f32,
    label: String,
}

fn map_scale(viewport: egui::Rect, view: &MapView) -> Option<MapScale> {
    let map_size = f64::from(view.map_size(viewport));
    if !map_size.is_finite() || map_size <= 0.0 {
        return None;
    }
    let latitude = (std::f64::consts::PI * (1.0 - 2.0 * f64::from(view.center.y)))
        .sinh()
        .atan();
    let metres_per_pixel = std::f64::consts::TAU * EARTH_RADIUS_M * latitude.cos() / map_size;
    let target_metres = metres_per_pixel * f64::from(SCALE_BAR_TARGET_PX);
    if !target_metres.is_finite() || target_metres <= 0.0 {
        return None;
    }
    let magnitude = 10.0_f64.powf(target_metres.log10().floor());
    let normalized = target_metres / magnitude;
    let factor = if normalized >= 5.0 {
        5.0
    } else if normalized >= 2.0 {
        2.0
    } else {
        1.0
    };
    let metres = factor * magnitude;
    let pixels = (metres / metres_per_pixel) as f32;
    let label = if metres >= 1_000.0 {
        format!("{:.0} km", metres / 1_000.0)
    } else {
        format!("{metres:.0} m")
    };
    Some(MapScale {
        metres,
        pixels,
        label,
    })
}

fn paint_place_labels(
    painter: &egui::Painter,
    viewport: egui::Rect,
    view: &MapView,
    catalog: Option<&MapPlaceCatalog>,
    current_location_name: Option<&str>,
    lat_deg: f64,
    lon_deg: f64,
) {
    let safe_viewport = viewport.shrink(LABEL_EDGE_INSET);
    if safe_viewport.width() <= 0.0 || safe_viewport.height() <= 0.0 {
        return;
    }
    let map_size = view.map_size(viewport);
    let half_world = viewport.size() / (2.0 * map_size);
    let y_min = (view.center.y - half_world.y).clamp(0.0, 1.0);
    let y_max = (view.center.y + half_world.y).clamp(0.0, 1.0);
    let y0 = ((y_min * LABEL_BUCKETS as f32).floor() as usize).min(LABEL_BUCKETS - 1);
    let y1 = ((y_max * LABEL_BUCKETS as f32).floor() as usize).min(LABEL_BUCKETS - 1);

    let visible_world_width = (viewport.width() / map_size).min(1.0);
    let x_bucket_count =
        ((visible_world_width * LABEL_BUCKETS as f32).ceil() as usize + 2).min(LABEL_BUCKETS);
    let x_start =
        ((view.center.x - visible_world_width * 0.5) * LABEL_BUCKETS as f32).floor() as isize - 1;

    let mut occupied = Vec::new();
    let current_location =
        current_location_label(current_location_name, view.zoom).and_then(|label| {
            let point = map_point(viewport, view, lat_deg, lon_deg);
            let primary = painter.layout_no_wrap(
                label.primary.to_owned(),
                egui::FontId::new(10.5, crate::ui::plex_medium_family()),
                egui::Color32::PLACEHOLDER,
            );
            let region = label.region.map(|region| {
                painter.layout_no_wrap(
                    region.to_owned(),
                    egui::FontId::monospace(8.5),
                    egui::Color32::PLACEHOLDER,
                )
            });
            let position = point + egui::vec2(12.0, -25.0);
            let size = egui::vec2(
                region.as_ref().map_or(primary.size().x, |region| {
                    primary.size().x.max(region.size().x)
                }),
                primary.size().y + region.as_ref().map_or(0.0, |region| region.size().y),
            );
            let rect = egui::Rect::from_min_size(position, size).expand2(egui::vec2(4.0, 3.0));
            safe_viewport
                .contains_rect(rect)
                .then_some((position, rect, primary, region))
        });
    if let Some((_, rect, _, _)) = &current_location {
        occupied.push(*rect);
    }

    let mut candidates = Vec::<(&LabelPlace, egui::Pos2, f32)>::new();
    if let Some(catalog) = catalog {
        for y in y0..=y1 {
            for x_offset in 0..x_bucket_count {
                let x = (x_start + x_offset as isize).rem_euclid(LABEL_BUCKETS as isize) as usize;
                for place in &catalog.buckets[y * LABEL_BUCKETS + x] {
                    if place.min_zoom > view.zoom {
                        break;
                    }
                    let alpha = place_label_alpha(view.zoom, place.min_zoom);
                    if alpha <= 0.0 {
                        continue;
                    }
                    let position = viewport.center()
                        + egui::vec2(
                            wrapped_delta_x(place.world.x, view.center.x),
                            place.world.y - view.center.y,
                        ) * map_size;
                    if safe_viewport.contains(position) {
                        candidates.push((place, position, alpha));
                    }
                }
            }
        }
    }
    candidates.sort_by(|(a, _, _), (b, _, _)| {
        a.min_zoom
            .total_cmp(&b.min_zoom)
            .then_with(|| b.population.cmp(&a.population))
            .then_with(|| a.geoname_id.cmp(&b.geoname_id))
    });

    for (place, point, alpha) in candidates {
        let font_size = if place.min_zoom <= 4.0 { 11.5 } else { 10.5 };
        let galley = painter.layout_no_wrap(
            place.name.clone(),
            egui::FontId::new(font_size, crate::ui::plex_medium_family()),
            egui::Color32::PLACEHOLDER,
        );
        let text_rect = egui::Rect::from_min_size(
            point + egui::vec2(6.0, -galley.size().y * 0.5),
            galley.size(),
        )
        .expand2(egui::vec2(3.5, 2.5));
        let dot_rect = egui::Rect::from_center_size(point, egui::vec2(7.0, 7.0));
        let collision_rect = text_rect.union(dot_rect);
        if !safe_viewport.contains_rect(collision_rect)
            || occupied.iter().any(|rect| rect.intersects(collision_rect))
        {
            continue;
        }
        occupied.push(collision_rect);

        let alpha = (alpha * 235.0).round() as u8;
        painter.circle_filled(
            point,
            if place.min_zoom <= 4.0 { 2.5 } else { 2.0 },
            egui::Color32::from_rgba_unmultiplied(34, 57, 58, alpha),
        );
        painter.galley(
            point + egui::vec2(6.0, -galley.size().y * 0.5),
            galley,
            egui::Color32::from_rgba_unmultiplied(38, 56, 57, alpha),
        );
    }
    if let Some((position, rect, primary, region)) = current_location {
        painter.rect_filled(
            rect,
            3.0,
            egui::Color32::from_rgba_unmultiplied(236, 244, 226, 225),
        );
        let region_y = position.y + primary.size().y;
        painter.galley(position, primary, egui::Color32::from_rgb(28, 54, 55));
        if let Some(region) = region {
            painter.galley(
                egui::pos2(position.x, region_y),
                region,
                egui::Color32::from_rgb(73, 91, 83),
            );
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CurrentLocationLabel<'a> {
    primary: &'a str,
    region: Option<&'a str>,
}

fn current_location_label(name: Option<&str>, zoom: f32) -> Option<CurrentLocationLabel<'_>> {
    if zoom < LOCAL_LOCATION_LABEL_MIN_ZOOM {
        return None;
    }
    let value = name?;
    let (primary, region) = value
        .split_once(',')
        .map_or((value, None), |(primary, region)| (primary, Some(region)));
    let primary = primary.trim();
    if primary.is_empty() {
        return None;
    }
    let region = region
        .map(str::trim)
        .filter(|region| !region.is_empty() && *region != primary);
    Some(CurrentLocationLabel { primary, region })
}

fn place_label_alpha(zoom: f32, min_zoom: f32) -> f32 {
    if min_zoom <= MIN_ZOOM {
        return 1.0;
    }
    ((zoom / min_zoom - 1.0) / (LABEL_FADE_RATIO - 1.0)).clamp(0.0, 1.0)
}

fn wrapped_delta_x(value: f32, center: f32) -> f32 {
    (value - center + 0.5).rem_euclid(1.0) - 0.5
}

fn map_point(viewport: egui::Rect, view: &MapView, lat_deg: f64, lon_deg: f64) -> egui::Pos2 {
    let world = project(lat_deg, lon_deg);
    viewport.center()
        + egui::vec2(
            wrapped_delta_x(world.x, view.center.x),
            world.y - view.center.y,
        ) * view.map_size(viewport)
}

fn project(lat_deg: f64, lon_deg: f64) -> egui::Vec2 {
    let longitude = (lon_deg + 180.0).rem_euclid(360.0) - 180.0;
    let latitude = lat_deg
        .clamp(-MERCATOR_LAT_LIMIT, MERCATOR_LAT_LIMIT)
        .to_radians();
    let mercator_y = (latitude.tan() + 1.0 / latitude.cos()).ln();
    egui::vec2(
        ((longitude + 180.0) / 360.0) as f32,
        (0.5 - mercator_y / (2.0 * std::f64::consts::PI)) as f32,
    )
}

fn paint_position_marker(painter: &egui::Painter, center: egui::Pos2, heading_deg: f64) {
    // User-supplied SVG path, sampled from its exact line, cubic, and arc
    // segments. The source points north-west, so rotate it 45° to make zero
    // degrees point north before applying the traveller's heading.
    //
    // M14.185 13.14 l5.644 -2.202
    // c1.625 -.634 1.538 -2.962 -.13 -3.473 l-14.319 -4.382
    // c-1.41 -.431 -2.73 .888 -2.298 2.298 l4.382 14.318
    // c.51 1.668 2.84 1.755 3.473 .13 l2.202 -5.644
    // a1.84 1.84 0 0 1 1.045 -1.045
    let rotation = heading_deg.to_radians() as f32 + std::f32::consts::FRAC_PI_4;
    let (sin, cos) = rotation.sin_cos();
    let scale = 0.92;
    let transform = |point: egui::Vec2| {
        let point = (point - egui::vec2(12.0, 12.0)) * scale;
        center + egui::vec2(point.x * cos - point.y * sin, point.x * sin + point.y * cos)
    };
    let points = position_marker_svg_points()
        .into_iter()
        .map(transform)
        .collect::<Vec<_>>();

    let fill = egui::Color32::WHITE;
    let mut mesh = egui::Mesh::default();
    mesh.vertices.push(egui::epaint::Vertex::untextured(
        transform(egui::vec2(10.0, 10.0)),
        fill,
    ));
    mesh.vertices.extend(
        points
            .iter()
            .copied()
            .map(|point| egui::epaint::Vertex::untextured(point, fill)),
    );
    for index in 0..points.len() - 1 {
        mesh.indices
            .extend_from_slice(&[0, index as u32 + 1, index as u32 + 2]);
    }
    painter.add(egui::Shape::mesh(mesh));

    let outline = egui::Color32::from_rgb(37, 72, 124);
    let outline_width = 2.0 * scale;
    for segment in points.windows(2) {
        painter.line_segment(
            [segment[0], segment[1]],
            egui::Stroke::new(outline_width, outline),
        );
    }
    for point in points {
        painter.circle_filled(point, outline_width * 0.5, outline);
    }
}

fn position_marker_svg_points() -> Vec<egui::Vec2> {
    let mut points = Vec::with_capacity(40);
    let mut current = egui::vec2(14.185, 13.14);
    points.push(current);

    current += egui::vec2(5.644, -2.202);
    points.push(current);
    current = append_cubic(
        &mut points,
        current,
        current + egui::vec2(1.625, -0.634),
        current + egui::vec2(1.538, -2.962),
        current + egui::vec2(-0.13, -3.473),
    );

    current += egui::vec2(-14.319, -4.382);
    points.push(current);
    current = append_cubic(
        &mut points,
        current,
        current + egui::vec2(-1.41, -0.431),
        current + egui::vec2(-2.73, 0.888),
        current + egui::vec2(-2.298, 2.298),
    );

    current += egui::vec2(4.382, 14.318);
    points.push(current);
    current = append_cubic(
        &mut points,
        current,
        current + egui::vec2(0.51, 1.668),
        current + egui::vec2(2.84, 1.755),
        current + egui::vec2(3.473, 0.13),
    );

    current += egui::vec2(2.202, -5.644);
    points.push(current);
    append_svg_arc(&mut points, current, current + egui::vec2(1.045, -1.045));
    points
}

fn append_cubic(
    points: &mut Vec<egui::Vec2>,
    start: egui::Vec2,
    control_1: egui::Vec2,
    control_2: egui::Vec2,
    end: egui::Vec2,
) -> egui::Vec2 {
    for step in 1..=8 {
        let t = step as f32 / 8.0;
        let one_minus_t = 1.0 - t;
        points.push(
            start * one_minus_t.powi(3)
                + control_1 * (3.0 * one_minus_t.powi(2) * t)
                + control_2 * (3.0 * one_minus_t * t * t)
                + end * t.powi(3),
        );
    }
    end
}

fn append_svg_arc(points: &mut Vec<egui::Vec2>, start: egui::Vec2, end: egui::Vec2) {
    // Exact centre for `a1.84 1.84 0 0 1 1.045 -1.045` in SVG's
    // downward-positive coordinate system.
    let radius = 1.84_f32;
    let midpoint = (start + end) * 0.5;
    let chord = end - start;
    let half_chord = chord.length() * 0.5;
    let centre_offset = (radius * radius - half_chord * half_chord).sqrt();
    let normal = egui::vec2(-chord.y, chord.x).normalized();
    let centre = midpoint + normal * centre_offset;
    let start_angle = (start.y - centre.y).atan2(start.x - centre.x);
    let mut sweep = (end.y - centre.y).atan2(end.x - centre.x) - start_angle;
    if sweep < 0.0 {
        sweep += std::f32::consts::TAU;
    }
    for step in 1..=5 {
        let angle = start_angle + sweep * step as f32 / 5.0;
        points.push(centre + egui::vec2(angle.cos(), angle.sin()) * radius);
    }
}

struct Geometry {
    countries: Vec<Feature>,
    lakes: Vec<Feature>,
}

impl Geometry {
    fn decode(bytes: &[u8]) -> Self {
        let mut reader = Reader::new(bytes);
        assert_eq!(reader.bytes(8), b"VEARTHM1", "invalid map geometry");
        let countries = reader.features();
        let lakes = reader.features();
        assert!(reader.remaining().is_empty(), "trailing map geometry bytes");
        Self { countries, lakes }
    }
}

struct Feature {
    polygons: Vec<Vec<Vec<[f32; 2]>>>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.cursor..]
    }

    fn bytes(&mut self, count: usize) -> &'a [u8] {
        let end = self.cursor.checked_add(count).expect("map geometry offset");
        let value = self
            .bytes
            .get(self.cursor..end)
            .expect("truncated map geometry");
        self.cursor = end;
        value
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.bytes(4).try_into().expect("four bytes"))
    }

    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.bytes(4).try_into().expect("four bytes"))
    }

    fn features(&mut self) -> Vec<Feature> {
        (0..self.u32()).map(|_| self.feature()).collect()
    }

    fn feature(&mut self) -> Feature {
        let polygons = (0..self.u32())
            .map(|_| {
                (0..self.u32())
                    .map(|_| (0..self.u32()).map(|_| [self.f32(), self.f32()]).collect())
                    .collect()
            })
            .collect();
        Feature { polygons }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_mercator_reference_points_are_exact() {
        assert_eq!(project(0.0, 0.0), egui::vec2(0.5, 0.5));
        let sydney = project(-33.8688, 151.2093);
        assert!((sydney.x - 0.920_026).abs() < 0.000_001);
        assert!((sydney.y - 0.600_092).abs() < 0.000_001);
    }

    #[test]
    fn bundled_geometry_decodes() {
        let geometry = Geometry::decode(include_bytes!("../assets/mercator-map.bin"));
        assert!(geometry.countries.len() > 200);
        assert!(geometry.lakes.len() > 400);
    }

    #[test]
    fn supplied_position_marker_path_closes_at_the_svg_endpoint() {
        let points = position_marker_svg_points();
        assert!((points[0] - egui::vec2(14.185, 13.14)).length() < 0.000_01);
        assert!((points.last().copied().unwrap() - egui::vec2(14.184, 13.14)).length() < 0.000_01);
    }

    #[test]
    fn play_map_restores_pan_and_zoom_while_explore_recenters() {
        let mut navigation = MapNavigation::default();
        navigation.open(-33.8688, 151.2093, true);
        navigation.view.center = egui::vec2(0.73, 0.41);
        navigation.view.zoom = 37.0;
        navigation.close();

        navigation.open(51.5074, -0.1278, true);
        assert_eq!(navigation.view.center, egui::vec2(0.73, 0.41));
        assert_eq!(navigation.view.zoom, 37.0);
        navigation.close();

        navigation.open(51.5074, -0.1278, false);
        assert_eq!(navigation.view.center, project(51.5074, -0.1278));
        assert_eq!(navigation.view.zoom, INITIAL_ZOOM);
    }

    #[test]
    fn place_labels_reveal_progressively_with_zoom() {
        assert!(
            place_label_min_zoom(PlaceKind::City, 5_000_000)
                < place_label_min_zoom(PlaceKind::City, 500_000)
        );
        assert!(
            place_label_min_zoom(PlaceKind::City, 500_000)
                < place_label_min_zoom(PlaceKind::Town, 50_000)
        );
        assert!(
            place_label_min_zoom(PlaceKind::Town, 50_000)
                < place_label_min_zoom(PlaceKind::Village, 3_000)
        );
        assert!(place_label_min_zoom(PlaceKind::Capital, 100) <= INITIAL_ZOOM);
    }

    #[test]
    fn place_label_fade_has_soft_zoom_edges() {
        let threshold = 12.0;
        assert_eq!(place_label_alpha(threshold, threshold), 0.0);
        assert!(place_label_alpha(threshold * 1.25, threshold) > 0.0);
        assert!(place_label_alpha(threshold * 1.25, threshold) < 1.0);
        assert_eq!(
            place_label_alpha(threshold * LABEL_FADE_RATIO, threshold),
            1.0
        );
    }

    #[test]
    fn current_protomaps_location_is_close_zoom_only() {
        assert_eq!(
            current_location_label(Some("Sydney, New South Wales"), 39.9),
            None
        );
        assert_eq!(
            current_location_label(Some("Sydney, New South Wales"), 40.0),
            Some(CurrentLocationLabel {
                primary: "Sydney",
                region: Some("New South Wales"),
            })
        );
        assert_eq!(
            current_location_label(Some("Singapore"), 80.0),
            Some(CurrentLocationLabel {
                primary: "Singapore",
                region: None,
            })
        );
        assert_eq!(
            current_location_label(Some("Monaco, Monaco"), 80.0),
            Some(CurrentLocationLabel {
                primary: "Monaco",
                region: None,
            })
        );
        assert_eq!(current_location_label(Some("  "), 80.0), None);
    }

    #[test]
    fn scale_bar_uses_bounded_nice_ground_distances() {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_000.0, 600.0));
        let mut view = MapView::default();
        view.center = project(0.0, 0.0);
        view.zoom = 4.0;
        let broad = map_scale(viewport, &view).unwrap();
        assert_eq!(broad.metres, 1_000_000.0);
        assert_eq!(broad.label, "1000 km");
        assert!((50.0..=SCALE_BAR_TARGET_PX).contains(&broad.pixels));

        view.center = project(-33.8688, 151.2093);
        view.zoom = 128.0;
        let close = map_scale(viewport, &view).unwrap();
        assert_eq!(close.metres, 20_000.0);
        assert_eq!(close.label, "20 km");
        assert!((50.0..=SCALE_BAR_TARGET_PX).contains(&close.pixels));
        assert!(close.metres < broad.metres);
    }
}
