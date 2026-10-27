use std::{borrow::Cow, time::Duration};
use wgpu::util::DeviceExt;

const WIDTH: u32 = 128;
const HEIGHT: u32 = 64;
const SOURCE: &str = include_str!("../src/tonemap.wgsl");

#[test]
fn spatial_resolve_shader_validates_without_a_graphics_adapter() {
    let module = naga::front::wgsl::parse_str(SOURCE).expect("parse tone resolve");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validate tone resolve");
}

struct ResolveFixture {
    device: wgpu::Device,
    queue: wgpu::Queue,
    filtered: wgpu::RenderPipeline,
    reference: wgpu::RenderPipeline,
    resources: wgpu::BindGroup,
    globals: wgpu::Buffer,
    post: wgpu::Buffer,
    global_group: wgpu::BindGroup,
    post_group: wgpu::BindGroup,
    input: wgpu::Texture,
    depth: wgpu::Texture,
    depth_input: wgpu::Texture,
    depth_pipeline: wgpu::RenderPipeline,
    depth_group: wgpu::BindGroup,
    output: wgpu::Texture,
    readback: wgpu::Buffer,
}

impl ResolveFixture {
    fn new() -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: None,
                ..Default::default()
            })) {
                Ok(adapter) => adapter,
                Err(error) => {
                    eprintln!("spatial AA GPU test skipped: no graphics adapter ({error})");
                    return None;
                }
            };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("spatial AA numerical validation"),
            ..Default::default()
        }))
        .expect("request test device");
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let resources_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[
                Some(&uniform_layout),
                Some(&resources_layout),
                Some(&uniform_layout),
            ],
            immediate_size: 0,
        });
        let pipeline = |source: &str| {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("production tone resolve"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let filtered = pipeline(SOURCE);
        // The control shares the exact exposure, filmic curve, creative grade
        // and dither. Coverage reconstruction and contact shading are bypassed.
        let reference_source = SOURCE
            .replace(
                "var mapped = resolve_edges(pixel, center);",
                "var mapped = creative_grade(center);",
            )
            .replace("    mapped *= contact_visibility(pixel, hdr);", "");
        assert_ne!(reference_source, SOURCE);
        let reference = pipeline(&reference_source);
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[0.0_f32; 64]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let post = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[0_u32; 4]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_group = |buffer: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            })
        };
        let global_group = uniform_group(&globals);
        let post_group = uniform_group(&post);
        let texture = |format, dimension, size, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension,
                format,
                usage,
                view_formats: &[],
            })
        };
        let size = wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        };
        let input = texture(
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureDimension::D2,
            size,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let depth = texture(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureDimension::D2,
            size,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        // WebGPU forbids CPU-to-depth copies. Populate the real depth attachment
        // through fragment depth writes, as the production scene pass does.
        let depth_input = texture(
            wgpu::TextureFormat::R32Float,
            wgpu::TextureDimension::D2,
            size,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let depth_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &depth_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &depth_input.create_view(&Default::default()),
                ),
            }],
        });
        let depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("synthetic scene depth prepass"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(
                r#"
                @group(0) @binding(0) var depth_values: texture_2d<f32>;
                @vertex fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
                    let points = array<vec2<f32>, 3>(
                        vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
                    return vec4<f32>(points[index], 0.0, 1.0);
                }
                @fragment fn fs(@builtin(position) pixel: vec4<f32>) -> @builtin(frag_depth) f32 {
                    return textureLoad(depth_values, vec2<i32>(pixel.xy), 0).r;
                }
            "#,
            )),
        });
        let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[Some(&depth_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &depth_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &depth_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let output = texture(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureDimension::D2,
            size,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let lut = texture(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureDimension::D3,
            wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 2,
            },
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let mut lut_bytes = Vec::new();
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    lut_bytes.extend([12 + r * 204, 10 + g * 229, 15 + b * 179, 255]);
                }
            }
        }
        queue.write_texture(
            lut.as_image_copy(),
            &lut_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(2),
            },
            lut.size(),
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &resources_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &input.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &lut.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        &depth.create_view(&Default::default()),
                    ),
                },
            ],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(WIDTH * HEIGHT * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(Self {
            device,
            queue,
            filtered,
            reference,
            resources,
            globals,
            post,
            global_group,
            post_group,
            input,
            depth,
            depth_input,
            depth_pipeline,
            depth_group,
            output,
            readback,
        })
    }

    fn set_grade(&self, enabled: bool) {
        self.queue.write_buffer(
            &self.post,
            0,
            bytemuck::cast_slice(&[u32::from(enabled), u32::from(enabled), 0, 0]),
        );
    }

    fn render(&self, input: &[[f32; 4]], exposure: f32, filtered: bool) -> Vec<u8> {
        self.render_with_depth(input, exposure, filtered, None)
    }

    fn render_with_depth(
        &self,
        input: &[[f32; 4]],
        exposure: f32,
        filtered: bool,
        geometry: Option<(&[f32], glam::Mat4)>,
    ) -> Vec<u8> {
        assert_eq!(input.len(), (WIDTH * HEIGHT) as usize);
        let mut globals = [0.0_f32; 64];
        globals[27] = exposure;
        if let Some((_, inverse)) = geometry {
            globals[28..44].copy_from_slice(&inverse.to_cols_array());
        }
        let empty_depth = [0.0_f32; (WIDTH * HEIGHT) as usize];
        let depths = geometry.map_or(&empty_depth[..], |(depth, _)| depth);
        assert_eq!(depths.len(), (WIDTH * HEIGHT) as usize);
        self.queue.write_texture(
            self.depth_input.as_image_copy(),
            bytemuck::cast_slice(depths),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
            self.depth_input.size(),
        );
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::cast_slice(&globals));
        self.queue.write_texture(
            self.input.as_image_copy(),
            bytemuck::cast_slice(input),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 16),
                rows_per_image: Some(HEIGHT),
            },
            self.input.size(),
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let view = self.depth.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("synthetic scene depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.depth_pipeline);
            pass.set_bind_group(0, &self.depth_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let view = self.output.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(if filtered {
                &self.filtered
            } else {
                &self.reference
            });
            pass.set_bind_group(0, &self.global_group, &[]);
            pass.set_bind_group(1, &self.resources, &[]);
            pass.set_bind_group(2, &self.post_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            self.output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(WIDTH * 4),
                    rows_per_image: Some(HEIGHT),
                },
            },
            self.output.size(),
        );
        self.queue.submit([encoder.finish()]);
        let slice = self.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .expect("readback completion");
        rx.recv().unwrap().unwrap();
        let bytes = slice.get_mapped_range().unwrap().to_vec();
        self.readback.unmap();
        bytes
    }
}

fn image(pixel: impl Fn(u32, u32) -> [f32; 3]) -> Vec<[f32; 4]> {
    (0..HEIGHT)
        .flat_map(|y| {
            let pixel = &pixel;
            (0..WIDTH).map(move |x| {
                let c = pixel(x, y);
                [c[0], c[1], c[2], 1.0]
            })
        })
        .collect()
}

#[test]
fn highlight_resolve_preserves_neutral_exposure_and_bright_grass_colour_on_gpu() {
    let Some(gpu) = ResolveFixture::new() else {
        return;
    };
    gpu.set_grade(false);
    let center = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
    let legacy_curve =
        |x: f32| ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0);
    let mut previous = 0;
    for radiance in [0.0, 0.01, 0.05, 0.18, 0.5, 1.0, 2.0, 8.0, 32.0] {
        let pixels = gpu.render(&image(|_, _| [radiance; 3]), 1.0, false);
        let rgb = &pixels[center..center + 3];
        assert_eq!(rgb[0], rgb[1]);
        assert_eq!(rgb[1], rgb[2]);
        assert!((f32::from(rgb[0]) - legacy_curve(radiance) * 255.0).abs() <= 1.0);
        assert!(rgb[0] >= previous);
        previous = rgb[0];
    }
    for exposure in [1.0, 2.0, 4.0, 8.0] {
        let pigment = [0.22, 0.5, 0.075];
        let pixels = gpu.render(&image(|_, _| pigment), exposure, false);
        let rgb = &pixels[center..center + 3];
        // Sunlight can brighten a green surface without making its red and
        // blue channels converge to the same nearly-white shoulder value.
        assert!(f32::from(rgb[1]) > f32::from(rgb[0]) * 1.30);
        assert!(f32::from(rgb[1]) > f32::from(rgb[2]) * 2.0);
        assert!((f32::from(rgb[1]) - legacy_curve(pigment[1] * exposure) * 255.0).abs() <= 1.0);
    }
}

#[test]
fn spatial_resolve_preserves_detail_and_reduces_silhouette_coverage_error_on_gpu() {
    let Some(gpu) = ResolveFixture::new() else {
        return;
    };
    for (case, input) in [
        image(|_, _| [0.18, 0.35, 0.09]),
        image(|x, _| [(x + 16) as f32 / 160.0; 3]),
        image(|x, _| if x < 64 { [0.0; 3] } else { [2.0; 3] }),
        image(|x, y| if (x + y) % 2 == 0 { [0.0; 3] } else { [1.0; 3] }),
        image(|x, y| {
            if (x, y) == (64, 32) {
                [16.0, 8.0, 2.0]
            } else {
                [0.0; 3]
            }
        }),
    ]
    .into_iter()
    .enumerate()
    {
        let control = gpu.render(&input, 1.0, false);
        let resolved = gpu.render(&input, 1.0, true);
        let changed: Vec<_> = resolved
            .iter()
            .zip(&control)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(10)
            .collect();
        assert!(
            changed.is_empty(),
            "flat/aligned/ambiguous detail case {case} changed: {changed:?}"
        );
    }

    // The geometric boundary falls midway between contrasting sample centers.
    // Compare production WGSL against 32x32 analytic pixel coverage, retaining
    // the same filmic endpoints rather than tone-mapping averaged HDR values.
    for slope in [-2.0_f32, -1.0, -0.5, -0.25, 0.25, 0.5, 1.0, 2.0] {
        for bright in [[1.0; 3], [16.0; 3], [0.0, 0.0, 16.0]] {
            let side = |x: f32, y: f32| x - 64.0 > slope * (y - 32.0) + 0.25;
            let input = image(|x, y| {
                if side(x as f32 + 0.5, y as f32 + 0.5) {
                    bright
                } else {
                    [0.0; 3]
                }
            });
            let control = gpu.render(&input, 1.0, false);
            let resolved = gpu.render(&input, 1.0, true);
            let all_bright = gpu.render(&image(|_, _| bright), 1.0, false);
            let channel = usize::from(bright[0] == 0.0) * 2;
            let mut old_error = 0.0_f32;
            let mut new_error = 0.0_f32;
            let mut changed = 0;
            for y in 10..HEIGHT - 10 {
                for x in 12..WIDTH - 12 {
                    let index = ((y * WIDTH + x) * 4) as usize + channel;
                    let mut covered = 0;
                    for sy in 0..32 {
                        for sx in 0..32 {
                            covered += u32::from(side(
                                x as f32 + (sx as f32 + 0.5) / 32.0,
                                y as f32 + (sy as f32 + 0.5) / 32.0,
                            ));
                        }
                    }
                    let truth = covered as f32 / 1024.0 * all_bright[index] as f32;
                    old_error += (control[index] as f32 - truth).powi(2);
                    new_error += (resolved[index] as f32 - truth).powi(2);
                    changed += usize::from(control[index] != resolved[index]);
                    assert!(resolved[index] <= all_bright[index].saturating_add(1));
                    if covered == 0
                        && !side(x as f32 + 0.5, y as f32 - 0.5)
                        && !side(x as f32 + 0.5, y as f32 + 1.5)
                        && !side(x as f32 - 0.5, y as f32 + 0.5)
                        && !side(x as f32 + 1.5, y as f32 + 0.5)
                    {
                        assert_eq!(resolved[index], 0, "no glow beyond immediate silhouette");
                    }
                }
            }
            eprintln!(
                "AA slope={slope} HDR={bright:?}: squared error {old_error:.1} -> {new_error:.1}; changed={changed}"
            );
            assert!(changed > 20);
            assert!(
                new_error < old_error * 0.82,
                "insufficient coverage improvement at slope {slope}: {old_error} -> {new_error}"
            );
            let scaled: Vec<_> = input
                .iter()
                .map(|p| [p[0] * 2.0, p[1] * 2.0, p[2] * 2.0, p[3]])
                .collect();
            assert!(
                resolved == gpu.render(&scaled, 0.5, true),
                "exposure must precede edge decisions"
            );
        }
    }
}

#[test]
fn spatial_resolve_blends_after_creative_grade_on_gpu() {
    let Some(gpu) = ResolveFixture::new() else {
        return;
    };
    gpu.set_grade(true);
    let colors = [[0.025, 0.14, 0.03], [3.0, 0.3, 0.01]];
    for color in colors {
        let input = image(|_, _| color);
        assert!(gpu.render(&input, 1.0, true) == gpu.render(&input, 1.0, false));
    }
    let input = image(|x, y| colors[usize::from(x > y + 32)]);
    let resolved = gpu.render(&input, 1.0, true);
    let control = gpu.render(&input, 1.0, false);
    let low = gpu.render(&image(|_, _| colors[0]), 1.0, false);
    let high = gpu.render(&image(|_, _| colors[1]), 1.0, false);
    let mut changed = 0;
    for (index, &value) in resolved.iter().enumerate() {
        let minimum = low[index].min(high[index]).saturating_sub(1);
        let maximum = low[index].max(high[index]).saturating_add(1);
        assert!(
            (minimum..=maximum).contains(&value),
            "grade interpolation rang outside its endpoints"
        );
        changed += usize::from(value != control[index]);
    }
    assert!(
        changed > 50,
        "the enabled grade must retain spatial edge smoothing"
    );
}

fn contact_scene(with_box: bool) -> (Vec<f32>, Vec<Option<glam::Vec3>>, glam::Mat4) {
    use glam::{Vec3, Vec4};
    let camera = Vec3::new(0.0, 3.0, 6.0);
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(
        60.0_f32.to_radians(),
        WIDTH as f32 / HEIGHT as f32,
        0.1,
    );
    let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, -camera, Vec3::Y);
    let transform = projection * view;
    let inverse = transform.inverse();
    let mut depths = Vec::new();
    let mut ground_hits = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let ndc = Vec3::new(
                (x as f32 + 0.5) / WIDTH as f32 * 2.0 - 1.0,
                1.0 - (y as f32 + 0.5) / HEIGHT as f32 * 2.0,
                0.5,
            );
            let sample = inverse * ndc.extend(1.0);
            let ray = (sample.truncate() / sample.w).normalize();
            let ground = -camera.y / ray.y;
            let mut hit = if ground > 0.0 { Some(ground) } else { None };
            let mut ground_hit = hit.map(|t| camera + ray * t);
            if with_box {
                let lo = (Vec3::new(-0.65, 0.0, -0.65) - camera) / ray;
                let hi = (Vec3::new(0.65, 1.4, 0.65) - camera) / ray;
                let near = lo.min(hi).max_element();
                let far = lo.max(hi).min_element();
                if far >= near && near > 0.0 && hit.is_none_or(|ground| near < ground) {
                    hit = Some(near);
                    ground_hit = None;
                }
            }
            let depth = hit.map_or(0.0, |t| {
                let clip: Vec4 = transform * (ray * t).extend(1.0);
                clip.z / clip.w
            });
            depths.push(depth);
            ground_hits.push(ground_hit);
        }
    }
    (depths, ground_hits, inverse)
}

#[test]
fn contact_scene_uses_finite_reversed_depth_and_metre_positions() {
    let (depths, ground, inverse) = contact_scene(true);
    assert!(
        depths
            .iter()
            .all(|&depth| depth.is_finite() && (0.0..=1.0).contains(&depth))
    );
    let camera = glam::Vec3::new(0.0, 3.0, 6.0);
    for (index, hit) in ground.iter().enumerate() {
        if let Some(expected) = hit {
            let x = index % WIDTH as usize;
            let y = index / WIDTH as usize;
            let clip = glam::Vec4::new(
                (x as f32 + 0.5) / WIDTH as f32 * 2.0 - 1.0,
                1.0 - (y as f32 + 0.5) / HEIGHT as f32 * 2.0,
                depths[index],
                1.0,
            );
            let point = inverse * clip;
            assert!((point.truncate() / point.w + camera).distance(*expected) < 0.01);
        }
    }
}

#[test]
fn contact_occlusion_grounds_near_geometry_without_depth_edge_or_emissive_halos_on_gpu() {
    use glam::{Mat4, Vec3};
    let Some(gpu) = ResolveFixture::new() else {
        return;
    };
    let input = image(|_, _| [0.22, 0.27, 0.18]);
    let (flat, _, inverse) = contact_scene(false);
    let flat_control = gpu.render_with_depth(&input, 1.0, false, Some((&flat, inverse)));
    let flat_contact = gpu.render_with_depth(&input, 1.0, true, Some((&flat, inverse)));
    assert!(
        flat_control == flat_contact,
        "an isolated plane must not self-occlude"
    );
    let (depth, ground, inverse) = contact_scene(true);
    let control = gpu.render_with_depth(&input, 1.0, false, Some((&depth, inverse)));
    let contact = gpu.render_with_depth(&input, 1.0, true, Some((&depth, inverse)));
    let mut grounded_pixels = 0;
    let mut maximum_darkening = 0.0_f32;
    for (index, hit) in ground.iter().enumerate() {
        let old = control[index * 4] as f32;
        let new = contact[index * 4] as f32;
        let reduction = old - new;
        maximum_darkening = maximum_darkening.max(reduction / old.max(1.0));
        assert!(reduction >= 0.0 && reduction <= old * 0.14 + 1.0);
        if let Some(hit) = hit {
            let dx = (hit.x.abs() - 0.65).max(0.0);
            let dz = (hit.z.abs() - 0.65).max(0.0);
            if dx.hypot(dz) > 2.0 {
                assert_eq!(
                    reduction, 0.0,
                    "contact darkening escaped its world-space radius"
                );
            } else if reduction >= 2.0 {
                grounded_pixels += 1;
            }
        }
    }
    eprintln!(
        "contact: {grounded_pixels} grounded floor pixels, max darkening {maximum_darkening:.3}"
    );
    assert!(
        grounded_pixels >= 12,
        "visible geometry must produce meaningful contact shading"
    );
    let emissive = image(|_, _| [16.0, 8.0, 2.0]);
    assert!(
        gpu.render_with_depth(&emissive, 1.0, false, Some((&depth, inverse)))
            == gpu.render_with_depth(&emissive, 1.0, true, Some((&depth, inverse))),
        "bright emissive pixels must remain unoccluded"
    );
    let distant_inverse = Mat4::from_scale(Vec3::splat(200.0)) * inverse;
    assert!(
        gpu.render_with_depth(&input, 1.0, false, Some((&depth, distant_inverse)))
            == gpu.render_with_depth(&input, 1.0, true, Some((&depth, distant_inverse))),
        "distant and fogged geometry must not receive local contact shading"
    );
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(
        60.0_f32.to_radians(),
        WIDTH as f32 / HEIGHT as f32,
        0.1,
    );
    for background in [0.0, 0.1 / 20.0] {
        let step: Vec<_> = (0..HEIGHT)
            .flat_map(|_| (0..WIDTH).map(|x| if x < WIDTH / 2 { 0.1 / 5.0 } else { background }))
            .collect();
        let old = gpu.render_with_depth(&input, 1.0, false, Some((&step, projection.inverse())));
        let new = gpu.render_with_depth(&input, 1.0, true, Some((&step, projection.inverse())));
        assert!(
            old == new,
            "a sky edge or disconnected depth step must not produce a dark halo"
        );
    }
}
