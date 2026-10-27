//! Render the production surface fragment against controlled geometric fixtures.
use super::*;
use bytemuck::Zeroable;
use glam::Vec3;
use vearth_platform::time::Duration;
use wgpu::util::DeviceExt;

const SIDE: u32 = 64;
const FIXTURE_VERTEX: &str = r#"
struct CityCase { color: vec4<f32>, geometry: vec4<f32> };
@group(1) @binding(3) var<uniform> city_case: CityCase;
@vertex
fn vs_city_test(@builtin(vertex_index) index: u32) -> VsOut {
    let corners = array<vec2<f32>, 6>(vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),
        vec2<f32>(1.0,1.0),vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,1.0));
    var out: VsOut;
    let p = corners[index];
    out.clip = vec4<f32>(p, city_case.geometry.y, 1.0);
    out.world_pos = vec3<f32>(p * 0.25, -city_case.geometry.x);
    out.local_pos = vec3<f32>(p, 0.0);
    // Byte conversion is the production vertex lookup, not a test approximation.
    out.color = vec3<f32>(SRGB_TO_LINEAR[u32(city_case.color.r)],
        SRGB_TO_LINEAR[u32(city_case.color.g)],SRGB_TO_LINEAR[u32(city_case.color.b)]);
    out.normal = vec3<f32>(0.0,0.0,1.0);
    out.base_normal = out.normal;
    out.ao = 1.0;
    out.sample_pitch_m = 1.0;
    out.fade = 1.0;
    out.face_mat = (u32(city_case.color.w) << 5u) | 4u | u32(city_case.geometry.z);
    out.tree_lod_alpha = 1.0;
    out.tree_transition = 1.0;
    return out;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Panel {
    color: [f32; 4],
    geometry: [f32; 4],
}
impl Panel {
    fn light(rgb: [u8; 3], distance: f32, depth: f32) -> Self {
        Self {
            color: [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, 94.0],
            geometry: [distance, depth, 0.0, 0.0],
        }
    }
}

fn globals(day: bool, lights: bool, time: f32, visibility: f32) -> Globals {
    Globals {
        view_proj: Mat4::IDENTITY.to_cols_array(),
        inv_view_proj: Mat4::IDENTITY.to_cols_array(),
        prev_view_proj: Mat4::IDENTITY.to_cols_array(),
        sun_dir: [0.45, if day { 0.7 } else { -0.3 }, 0.55, 1.0],
        moon_dir: [-0.3, 0.5, 0.8, 0.0],
        ambient: [0.22, 0.30, 0.40, 1.0],
        planet_center: [0.0, -6_378_137.0, 0.0, 6_378_137.0],
        viewport: [SIDE as f32, SIDE as f32, 0.5, f32::from(lights)],
        weather0: [14.0, 0.5, visibility, 0.0],
        surface: [0.4, 0.0, 0.08, 1.0],
        weather2: [time, 0.37, 0.0, 0.0],
        weather3: [1500.0, 6000.0, 0.0, 0.0],
        temporal: [0.0, 0.0, 0.0, time % 3600.0],
        aerial_sunward: [0.08, 0.10, 0.12, 0.0],
        aerial_side: [0.03, 0.05, 0.08, 0.0],
        aerial_away: [0.04, 0.06, 0.09, 0.0],
        ..Globals::zeroed()
    }
}

fn texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layers: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("city light fixture"),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}
fn buffer(device: &wgpu::Device, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytes,
        usage,
    })
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    materials: wgpu::BindGroup,
    shadows: wgpu::BindGroup,
    environment: wgpu::TextureView,
    sampler: wgpu::Sampler,
}
impl Gpu {
    fn new() -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("city light GPU validation skipped: no adapter ({error})");
                return None;
            }
        };
        eprintln!(
            "city light production fragment GPU: {:?}",
            adapter.get_info()
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("city lighting production fragment"),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .unwrap();
        let source = format!("{}\n{FIXTURE_VERTEX}", world_shader_source());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("unchanged world fragment with controlled vertex inputs"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(source)),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("production CITY_LIGHT fragment"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_city_test"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_opaque"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba32Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: surface_primitive_state(true),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let atlas = texture(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            22,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let atlas_view = atlas.create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let materials = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(2),
            entries: &[
                entry(0, wgpu::BindingResource::TextureView(&atlas_view)),
                entry(1, wgpu::BindingResource::Sampler(&sampler)),
                entry(2, wgpu::BindingResource::TextureView(&atlas_view)),
            ],
        });
        let shadow = texture(
            &device,
            wgpu::TextureFormat::Depth32Float,
            2,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let shadow_view = shadow.create_view(&Default::default());
        let comparison = device.create_sampler(&wgpu::SamplerDescriptor {
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_buffer = buffer(
            &device,
            bytemuck::bytes_of(&SunShadowU {
                light_view_proj_0: Mat4::IDENTITY.to_cols_array(),
                light_view_proj_1: Mat4::IDENTITY.to_cols_array(),
                splits: [1000.0, 2000.0, 840.0, 1800.0],
                tuning: [0.25, 0.0, 0.0, 0.0],
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let shadows = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(3),
            entries: &[
                entry(0, wgpu::BindingResource::TextureView(&shadow_view)),
                entry(1, wgpu::BindingResource::Sampler(&comparison)),
                entry(2, shadow_buffer.as_entire_binding()),
            ],
        });
        let environment = texture(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING,
        )
        .create_view(&Default::default());
        Some(Self {
            device,
            queue,
            pipeline,
            materials,
            shadows,
            environment,
            sampler,
        })
    }

    fn render(&self, globals: Globals, panels: &[Panel]) -> ([f32; 4], f32) {
        let device = &self.device;
        let globals_buffer = buffer(
            device,
            bytemuck::bytes_of(&globals),
            wgpu::BufferUsages::UNIFORM,
        );
        let cloud_buffer = buffer(
            device,
            bytemuck::bytes_of(&crate::cloud_shadows::Frame::default()),
            wgpu::BufferUsages::UNIFORM,
        );
        let global_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, globals_buffer.as_entire_binding()),
                entry(2, wgpu::BindingResource::TextureView(&self.environment)),
                entry(3, wgpu::BindingResource::Sampler(&self.sampler)),
                entry(4, wgpu::BindingResource::TextureView(&self.environment)),
                entry(5, wgpu::BindingResource::Sampler(&self.sampler)),
                entry(6, cloud_buffer.as_entire_binding()),
            ],
        });
        let panel_groups: Vec<_> = panels
            .iter()
            .map(|panel| {
                let b = buffer(
                    device,
                    bytemuck::bytes_of(panel),
                    wgpu::BufferUsages::UNIFORM,
                );
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.pipeline.get_bind_group_layout(1),
                    entries: &[entry(3, b.as_entire_binding())],
                })
            })
            .collect();
        let color = texture(
            device,
            wgpu::TextureFormat::Rgba32Float,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            device,
            wgpu::TextureFormat::Depth32Float,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 + u64::from(SIDE * SIDE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("city light and occluder"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &global_group, &[]);
            pass.set_bind_group(2, &self.materials, &[]);
            pass.set_bind_group(3, &self.shadows, &[]);
            for group in &panel_groups {
                pass.set_bind_group(1, group, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        for (texture, offset, aspect) in [
            (&color, 0, wgpu::TextureAspect::All),
            (&depth, 256, wgpu::TextureAspect::DepthOnly),
        ] {
            let is_depth = aspect == wgpu::TextureAspect::DepthOnly;
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: if is_depth {
                        wgpu::Origin3d::ZERO
                    } else {
                        wgpu::Origin3d {
                            x: SIDE / 2,
                            y: SIDE / 2,
                            z: 0,
                        }
                    },
                    aspect,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(if is_depth { SIDE } else { 1 }),
                    },
                },
                wgpu::Extent3d {
                    width: if is_depth { SIDE } else { 1 },
                    height: if is_depth { SIDE } else { 1 },
                    depth_or_array_layers: 1,
                },
            );
        }
        self.queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let values: &[f32] = bytemuck::cast_slice(&mapped);
        let color = [values[0], values[1], values[2], values[3]];
        assert!(color.iter().all(|v| v.is_finite() && *v >= 0.0));
        assert_eq!(color[3], 1.0, "fixture must cover the sampled pixel");
        (color, values[64 + (SIDE * (SIDE / 2) + SIDE / 2) as usize])
    }
}

fn rgb(p: [f32; 4]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

#[test]
fn sculpture_metal_has_specular_contrast_without_player_textures_or_emission() {
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let mut metal = Panel::light([205, 213, 219], 20.0, 0.6);
    metal.color[3] = f32::from(vearth_gen::material::SCULPTURE_METAL);
    let scene = |oblique: bool, lights: bool, time| {
        let mut g = globals(true, lights, time, 50_000.0);
        g.sun_dir = if oblique {
            [0.0, 0.9, 0.435_889_9, 1.0]
        } else {
            [0.0, 0.2, 0.979_795_9, 1.0]
        };
        g
    };
    let head_on = gpu.render(scene(false, false, 0.0), &[metal]);
    let oblique = gpu.render(scene(true, false, 0.0), &[metal]);
    let mut diffuse = metal;
    diffuse.color[3] = f32::from(vearth_gen::material::MARBLE);
    let diffuse_head = gpu.render(scene(false, false, 0.0), &[diffuse]);
    let diffuse_side = gpu.render(scene(true, false, 0.0), &[diffuse]);
    let contrast = |a, b| rgb(a).length() / rgb(b).length().max(1.0e-6);
    assert!(
        contrast(head_on.0, oblique.0) > 2.0 * contrast(diffuse_head.0, diffuse_side.0),
        "polished metal lost its directional highlight"
    );
    for time in [0.0, 125.0, 3599.9, 65000.0] {
        assert_eq!(gpu.render(scene(false, true, time), &[metal]), head_on);
    }
    let mut payload = metal;
    payload.geometry[2] = (7u32 << 13) as f32;
    assert_eq!(gpu.render(scene(false, false, 0.0), &[payload]), head_on);
    let mut occluder = diffuse;
    occluder.geometry = [10.0, 0.8, 0.0, 0.0];
    let wall = gpu.render(scene(false, false, 0.0), &[occluder]);
    assert_eq!(
        gpu.render(scene(false, false, 0.0), &[metal, occluder]),
        wall
    );
    assert_eq!(
        gpu.render(scene(false, false, 0.0), &[occluder, metal]),
        wall
    );
    assert_eq!(head_on.1, 0.6);
}

#[test]
fn city_lights_keep_authored_hue_day_behavior_and_exactly_steady_emission() {
    let Some(gpu) = Gpu::new() else {
        return;
    };
    for color in [
        [255, 32, 12],
        [15, 255, 46],
        [16, 65, 255],
        [245, 124, 25],
        [210, 40, 240],
    ] {
        let panel = Panel::light(color, 20.0, 0.6);
        let (night_on, depth_on) = gpu.render(globals(false, true, 0.0, 50_000.0), &[panel]);
        let (night_off, depth_off) = gpu.render(globals(false, false, 0.0, 50_000.0), &[panel]);
        let emission = rgb(night_on) - rgb(night_off);
        assert!(
            emission.max_element() > 1.0,
            "night fixture failed to emit: {color:?} {emission:?}"
        );
        let expected = Vec3::from_array(color.map(|c| {
            let s = f32::from(c) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }));
        assert!(
            (emission / emission.max_element() - expected / expected.max_element())
                .abs()
                .max_element()
                < 2.0e-5,
            "emission changed authored sRGB hue: {color:?} {emission:?}"
        );
        assert_eq!(depth_on, depth_off);
        for time in [0.125, 125.0, 299.9, 300.1, 3599.9, 65000.0] {
            assert_eq!(
                gpu.render(globals(false, true, time, 50_000.0), &[panel]),
                (night_on, depth_on),
                "catalog light animated at time {time}"
            );
        }
        let day_on = gpu.render(globals(true, true, 0.0, 50_000.0), &[panel]);
        let day_off = gpu.render(globals(true, false, 0.0, 50_000.0), &[panel]);
        assert_eq!(
            day_on, day_off,
            "daylight must retain physical pigment without emission"
        );
        assert!(rgb(day_on.0).max_element() > 0.01);
    }
}

#[test]
fn city_light_emission_shares_atmospheric_extinction_and_opaque_depth() {
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let color = [240, 84, 25];
    let near = Panel::light(color, 20.0, 0.6);
    let far = Panel::light(color, 2000.0, 0.3);
    let emission = |panel, visibility| {
        rgb(gpu
            .render(globals(false, true, 0.0, visibility), &[panel])
            .0)
            - rgb(gpu
                .render(globals(false, false, 0.0, visibility), &[panel])
                .0)
    };
    let clear_near = emission(near, 50_000.0);
    let fog_near = emission(near, 2500.0);
    let fog_far = emission(far, 2500.0);
    assert!(fog_near.max_element() < clear_near.max_element());
    assert!(
        fog_far.max_element() < fog_near.max_element() * 0.4,
        "distant neon bypassed atmospheric extinction: {fog_near:?} -> {fog_far:?}"
    );
    assert!(
        (fog_far / fog_far.max_element() - fog_near / fog_near.max_element())
            .abs()
            .max_element()
            < 1.0e-4
    );
    let mut occluder = Panel::light([25, 70, 30], 10.0, 0.8);
    occluder.color[3] = 11.0;
    let g = || globals(false, true, 0.0, 50_000.0);
    let only_wall = gpu.render(g(), &[occluder]);
    assert_eq!(gpu.render(g(), &[near, occluder]), only_wall);
    assert_eq!(
        gpu.render(g(), &[occluder, near]),
        only_wall,
        "neon must not draw through an already rendered wall"
    );
    let only_near = gpu.render(g(), &[near]);
    assert_eq!(gpu.render(g(), &[near, far]), only_near);
    assert_eq!(gpu.render(g(), &[far, near]), only_near);
    assert_eq!(only_near.1, 0.6);
    assert_eq!(only_wall.1, 0.8);
}
