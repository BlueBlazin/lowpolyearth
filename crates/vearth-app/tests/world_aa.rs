use std::{borrow::Cow, time::Duration};
use wgpu::util::DeviceExt;

#[path = "../src/world_aa.rs"]
mod world_aa;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 16;
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[test]
fn depth_resolve_shader_validates_without_an_adapter() {
    let module = naga::front::wgsl::parse_str(include_str!("../src/world_aa.wgsl")).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

struct Fixture {
    device: wgpu::Device,
    queue: wgpu::Queue,
    aa: world_aa::WorldAa,
    pipeline: wgpu::RenderPipeline,
}

impl Fixture {
    fn new(samples: u32) -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("world AA GPU test skipped: no adapter ({error})");
                return None;
            }
        };
        // WebGPU guarantees one and four samples; two needs adapter support.
        let required_features = if samples == 2 {
            let supported = [COLOR, DEPTH].into_iter().all(|format| {
                adapter
                    .get_texture_format_features(format)
                    .flags
                    .sample_count_supported(2)
            });
            let feature = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
            if !supported || !adapter.features().contains(feature) {
                eprintln!("world AA 2x test skipped: adapter lacks two-sample targets");
                return None;
            }
            feature
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features,
            ..Default::default()
        }))
        .unwrap();
        let mut aa = world_aa::WorldAa::new(&device, DEPTH);
        aa.samples = samples;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("moving bright architectural edge"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(
                "@vertex fn vs(@location(0) p: vec3<f32>) -> @builtin(position) vec4<f32> {
                    return vec4<f32>(p, 1.0);
                }
                @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
            )),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(COLOR.into())],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: aa.state(),
            multiview_mask: None,
            cache: None,
        });
        Some(Self {
            device,
            queue,
            aa,
            pipeline,
        })
    }

    fn render(&self, vertices: &[[f32; 3]]) -> (Vec<f32>, Vec<f32>) {
        let extent = wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        };
        let texture = |format| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let color = texture(COLOR);
        let depth = texture(DEPTH);
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let ms = self.aa.targets(&self.device, WIDTH, HEIGHT, COLOR, DEPTH);
        assert_eq!(ms.is_some(), self.aa.samples > 1);
        let vertices_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: ms.as_ref().map_or(&color_view, |t| &t.color),
                    resolve_target: ms.as_ref().map(|_| &color_view),
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: ms.as_ref().map_or(&depth_view, |t| &t.depth),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, vertices_buffer.slice(..));
            pass.draw(0..vertices.len() as u32, 0..1);
        }
        if let Some(targets) = &ms {
            self.aa.resolve_depth(&mut encoder, targets, &depth_view);
        }
        let buffers: Vec<_> = [&color, &depth]
            .into_iter()
            .map(|texture| {
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(WIDTH * HEIGHT * 4),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                encoder.copy_texture_to_buffer(
                    texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(WIDTH * 4),
                            rows_per_image: Some(HEIGHT),
                        },
                    },
                    extent,
                );
                buffer
            })
            .collect();
        self.queue.submit([encoder.finish()]);
        let mut result = Vec::new();
        for buffer in buffers {
            let (send, recv) = std::sync::mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(Duration::from_secs(30)),
                })
                .unwrap();
            recv.recv().unwrap().unwrap();
            result.push(buffer.slice(..).get_mapped_range().unwrap().to_vec());
            buffer.unmap();
        }
        let pixels = result[0]
            .chunks_exact(4)
            .map(|p| f32::from(p[0]) / 255.0)
            .collect();
        let depths = result[1]
            .chunks_exact(4)
            .map(|p| f32::from_ne_bytes(p.try_into().unwrap()))
            .collect();
        (pixels, depths)
    }
}

fn strip(offset: f32, width: f32, slope: f32, depth: f32) -> Vec<[f32; 3]> {
    let p = |y: f32, dx: f32| {
        [
            (25.0 + offset + slope * y + dx) / WIDTH as f32 * 2.0 - 1.0,
            1.0 - y / HEIGHT as f32 * 2.0,
            depth,
        ]
    };
    let a = p(0.0, 0.0);
    let b = p(0.0, width);
    let c = p(HEIGHT as f32, width);
    let d = p(HEIGHT as f32, 0.0);
    vec![a, b, c, a, c, d]
}

/// Squared coverage and frame-to-frame errors of each fixture for moving
/// sub-pixel strips, measured against the exactly integrated pixel coverage.
fn moving_edge_errors(fixtures: &[&Fixture]) -> (Vec<f64>, Vec<f64>) {
    let mut errors = vec![0.0_f64; fixtures.len()];
    let mut temporal_errors = vec![0.0_f64; fixtures.len()];
    for slope in [-0.37, 0.0, 0.23, 1.1] {
        let mut previous: Option<(Vec<f32>, Vec<Vec<f32>>)> = None;
        for frame in 0..32 {
            let offset = frame as f32 / 32.0;
            let vertices = strip(offset, 0.6, slope, 0.35);
            let mut truth = vec![0.0; (WIDTH * HEIGHT) as usize];
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    // Integrate the exact horizontal interval through each pixel.
                    for sub in 0..128 {
                        let left = 25.0 + offset + slope * (y as f32 + (sub as f32 + 0.5) / 128.0);
                        truth[(y * WIDTH + x) as usize] +=
                            ((left + 0.6).min(x as f32 + 1.0) - left.max(x as f32)).max(0.0)
                                / 128.0;
                    }
                }
            }
            let outputs: Vec<_> = fixtures
                .iter()
                .map(|fixture| {
                    let (pixels, depth) = fixture.render(&vertices);
                    for (&coverage, &z) in pixels.iter().zip(&depth) {
                        assert!((z - if coverage > 0.0 { 0.35 } else { 0.0 }).abs() < 1e-6);
                    }
                    pixels
                })
                .collect();
            for variant in 0..fixtures.len() {
                for i in 0..truth.len() {
                    errors[variant] += f64::from((outputs[variant][i] - truth[i]).powi(2));
                    if let Some((old_truth, old_outputs)) = &previous {
                        let actual_delta = outputs[variant][i] - old_outputs[variant][i];
                        let ideal_delta = truth[i] - old_truth[i];
                        temporal_errors[variant] += f64::from((actual_delta - ideal_delta).powi(2));
                    }
                }
            }
            previous = Some((truth, outputs));
        }
    }
    (errors, temporal_errors)
}

/// Overlapping faces resolve to the nearest covered depth, including pixels
/// where that foreground face covers only one raster sample.
fn assert_nearest_depth_resolve(fixture: &Fixture) {
    let mut overlap = strip(0.0, 4.0, 0.0, 0.35);
    overlap.extend(strip(0.3, 0.6, 0.0, 0.8));
    let (_, depth) = fixture.render(&overlap);
    assert!((depth[25] - 0.8).abs() < 1e-6);
    assert!((depth[27] - 0.35).abs() < 1e-6);
    assert_eq!(depth[0], 0.0);
}

#[test]
fn multisampling_reduces_moving_subpixel_edge_error_and_preserves_occlusion() {
    let Some(single) = Fixture::new(1) else {
        return;
    };
    let multi = Fixture::new(4).unwrap();
    let (errors, temporal_errors) = moving_edge_errors(&[&single, &multi]);
    eprintln!("coverage error single/4x: {errors:?}; motion error: {temporal_errors:?}");
    assert!(errors[1] < errors[0] * 0.35, "coverage error {errors:?}");
    assert!(
        temporal_errors[1] < temporal_errors[0] * 0.5,
        "motion error {temporal_errors:?}"
    );
    assert_nearest_depth_resolve(&multi);
}

#[test]
fn two_sample_control_reduces_edge_error_and_preserves_occlusion() {
    let (Some(single), Some(double)) = (Fixture::new(1), Fixture::new(2)) else {
        return;
    };
    let (errors, temporal_errors) = moving_edge_errors(&[&single, &double]);
    eprintln!("coverage error single/2x: {errors:?}; motion error: {temporal_errors:?}");
    assert!(errors[1] < errors[0] * 0.6, "coverage error {errors:?}");
    assert!(
        temporal_errors[1] < temporal_errors[0] * 0.6,
        "motion error {temporal_errors:?}"
    );
    assert_nearest_depth_resolve(&double);
}
