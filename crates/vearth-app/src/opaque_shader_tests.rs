use super::*;
use bytemuck::Zeroable;
use std::{collections::HashSet, time::Duration};
use wgpu::util::DeviceExt;

const REFERENCE: &str = include_str!("../tests/fixtures/opaque-fragment-reference.wgsl");

fn comparison_source() -> String {
    format!("{}\n{REFERENCE}", world_shader_source())
}

pub(super) fn inspect_calls(
    module: &naga::Module,
    block: &naga::Block,
    seen: &mut HashSet<naga::Handle<naga::Function>>,
) -> usize {
    let mut kills = 0;
    for statement in block.iter() {
        match statement {
            naga::Statement::Block(block) => kills += inspect_calls(module, block, seen),
            naga::Statement::If { accept, reject, .. } => {
                kills += inspect_calls(module, accept, seen);
                kills += inspect_calls(module, reject, seen);
            }
            naga::Statement::Switch { cases, .. } => {
                for case in cases {
                    kills += inspect_calls(module, &case.body, seen);
                }
            }
            naga::Statement::Loop {
                body, continuing, ..
            } => {
                kills += inspect_calls(module, body, seen);
                kills += inspect_calls(module, continuing, seen);
            }
            naga::Statement::Call { function, .. } if seen.insert(*function) => {
                kills += inspect_calls(module, &module.functions[*function].body, seen);
            }
            naga::Statement::Kill => kills += 1,
            naga::Statement::ImageStore { .. } | naga::Statement::Atomic { .. } => {
                panic!("surface fragment has an external side effect")
            }
            _ => {}
        }
    }
    kills
}

#[test]
fn opaque_fragment_has_no_reachable_feedback_or_depth_side_effect() {
    let source = comparison_source();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    for (_, variable) in module.global_variables.iter() {
        if let naga::AddressSpace::Storage { access } = variable.space {
            assert!(!access.contains(naga::StorageAccess::STORE));
        }
    }
    for (name, expected_kills) in [("fs_opaque", 0), ("fs_main", 3), ("fs_reference", 3)] {
        let entry = module
            .entry_points
            .iter()
            .find(|entry| entry.name == name)
            .unwrap();
        assert_eq!(entry.stage, naga::ShaderStage::Fragment);
        let result = entry.function.result.as_ref().unwrap();
        // A sole color result cannot write fragment depth or sample coverage.
        assert!(matches!(
            result.binding,
            Some(naga::Binding::Location { location: 0, .. })
        ));
        assert!(matches!(
            module.types[result.ty].inner,
            naga::TypeInner::Vector { .. }
        ));
        assert_eq!(
            inspect_calls(&module, &entry.function.body, &mut HashSet::new()),
            expected_kills
        );
    }
    let source = include_str!("shader.wgsl");
    let feedback =
        &source[source.find("fn fs_main(").unwrap()..source.find("fn fs_opaque(").unwrap()];
    for derivative in [
        "let vehicle_uv_dx = dpdx(in.vehicle_uv);",
        "let vehicle_uv_dy = dpdy(in.vehicle_uv);",
    ] {
        assert!(feedback.find(derivative).unwrap() < feedback.find("discard;").unwrap());
    }
}

const WIDTH: u32 = 384;
const HEIGHT: u32 = 320;

#[derive(Clone, Copy)]
enum VertexPattern {
    Constant,
    TerrainBlend,
    SurfaceGradient,
}

#[derive(Clone, Copy)]
struct Panel {
    payload: u32,
    fade: f32,
    tree_blend: f32,
    opaque: bool,
    pattern: VertexPattern,
}

fn panels() -> Vec<Panel> {
    use vearth_core::mesh::{TERRAIN_BLEND_FACE, encode_surface_normal};
    let mut panels: Vec<_> = (0..128u32)
        .filter(|id| *id != 13 && !(22..66).contains(id))
        .map(|id| Panel {
            payload: encode_surface_normal(4 | (3 << 3) | (id << 5), [0.0, 0.0, 1.0]),
            fade: 1.0,
            tree_blend: 0.75, // non-tree materials must ignore this uniform.
            opaque: true,
            pattern: VertexPattern::Constant,
        })
        .collect();
    for kind in 0..4 {
        panels.push(Panel {
            payload: 2 | (3 << 3) | (1 << 5) | (kind << 13),
            fade: 1.0,
            tree_blend: 0.0,
            opaque: true,
            pattern: VertexPattern::Constant,
        });
    }
    for family in 0..4 {
        panels.push(Panel {
            payload: encode_surface_normal(
                TERRAIN_BLEND_FACE | (family << 3) | (3 << 5),
                [0.0, 0.0, 1.0],
            ),
            fade: 1.0,
            tree_blend: 0.0,
            opaque: true,
            pattern: VertexPattern::Constant,
        });
    }
    // Distinct corner families, normals, tints and AO exercise interpolation;
    // constant material panels alone cannot constrain interstage precision.
    for (material, pattern) in [
        (3, VertexPattern::TerrainBlend),
        (66, VertexPattern::TerrainBlend),
        (8, VertexPattern::SurfaceGradient),
        (16, VertexPattern::SurfaceGradient),
        (81, VertexPattern::SurfaceGradient),
        (85, VertexPattern::SurfaceGradient),
        (122, VertexPattern::SurfaceGradient),
        (123, VertexPattern::SurfaceGradient),
        (124, VertexPattern::SurfaceGradient),
        (126, VertexPattern::SurfaceGradient),
        (127, VertexPattern::SurfaceGradient),
    ] {
        panels.push(Panel {
            payload: 4 | (3 << 3) | (material << 5),
            fade: 1.0,
            tree_blend: 0.0,
            opaque: true,
            pattern,
        });
    }
    for variant in 1..=3 {
        panels.push(Panel {
            payload: encode_surface_normal(
                4 | (3 << 3) | (93 << 5) | (variant << 13),
                [0.0, 0.0, 1.0],
            ),
            fade: 1.0,
            tree_blend: 0.0,
            opaque: true,
            pattern: VertexPattern::SurfaceGradient,
        });
    }
    for material in [108, 109, 110, 116, 117, 118] {
        panels.push(Panel {
            payload: encode_surface_normal(
                4 | (3 << 3) | (material << 5) | (1 << 13),
                [0.0, 0.0, 1.0],
            ),
            fade: 1.0,
            tree_blend: 0.0,
            opaque: true,
            pattern: VertexPattern::SurfaceGradient,
        });
    }
    // The refactor must also leave feedback coverage unchanged.
    for (id, fade, tree_blend) in [
        (3, 0.37, 0.0),
        (3, -0.63, 0.0),
        (13, 1.0, 0.0),
        (22, 1.0, 0.43),
        (65, -0.63, 0.8),
        (22, 1.0, 1.0),
    ] {
        panels.push(Panel {
            payload: 4 | (3 << 3) | (id << 5),
            fade,
            tree_blend,
            opaque: false,
            pattern: VertexPattern::Constant,
        });
    }
    assert!(panels.len() <= 12 * 10);
    panels
}

#[derive(Clone, Copy)]
struct ViewCase {
    name: &'static str,
    distance: f32,
    slant: f32,
    sun: [f32; 3],
    wetness: f32,
}

const VIEWS: [ViewCase; 6] = [
    ViewCase {
        name: "near daylight",
        distance: 12.0,
        slant: 0.0,
        sun: [0.45, 0.7, 0.55],
        wetness: 0.0,
    },
    ViewCase {
        name: "city wet daylight",
        distance: 800.0,
        slant: 0.35,
        sun: [0.45, 0.7, 0.55],
        wetness: 0.8,
    },
    ViewCase {
        name: "distant night",
        distance: 24_000.0,
        slant: -0.6,
        sun: [0.45, -0.3, 0.84],
        wetness: 0.0,
    },
    ViewCase {
        name: "near night apertures",
        distance: 12.0,
        slant: 0.0,
        sun: [0.45, -0.3, 0.84],
        wetness: 0.0,
    },
    ViewCase {
        name: "near wet night",
        distance: 12.0,
        slant: -0.6,
        sun: [0.45, -0.3, 0.84],
        wetness: 0.95,
    },
    ViewCase {
        name: "near wet specular",
        distance: 12.0,
        slant: -0.35,
        sun: [0.45, 0.2, 0.87],
        wetness: 0.95,
    },
];

fn globals(view: ViewCase) -> Globals {
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(
        55.0f32.to_radians(),
        WIDTH as f32 / HEIGHT as f32,
        0.05,
    );
    Globals {
        view_proj: projection.to_cols_array(),
        inv_view_proj: projection.inverse().to_cols_array(),
        prev_view_proj: projection.to_cols_array(),
        sun_dir: [view.sun[0], view.sun[1], view.sun[2], 1.0],
        moon_dir: [-0.3, 0.5, 0.8, 0.65],
        ambient: [0.22, 0.3, 0.4, 1.0],
        planet_center: [0.0, -6_378_137.0, 0.0, 6_378_137.0],
        viewport: [
            WIDTH as f32,
            HEIGHT as f32,
            (55.0f32.to_radians() * 0.5).tan(),
            1.0,
        ],
        weather0: [14.0, 0.5, 50_000.0, 0.2],
        weather1: [view.wetness, 0.08, 0.3, 0.1],
        wind: [7.0, -3.0, 0.0, 2.0],
        surface: [0.4, 0.6, 0.08, 1.0],
        weather2: [125.0, 0.37, 0.0, 0.0],
        weather3: [1500.0, 6000.0, 0.0, 0.0],
        weather5: [0.25, 0.2, 0.15, 1.0],
        weather6: [0.0, 0.25, 0.2, 0.6],
        material_origin: [27.0, 57.0, 91.0, 1.0],
        aerial_sunward: [0.85, 1.07, 1.08, 0.0],
        aerial_side: [0.36, 0.53, 0.62, 0.0],
        aerial_away: [0.55, 0.79, 0.87, 0.0],
        ..Globals::zeroed()
    }
}

fn geometry(panels: &[Panel], view: ViewCase) -> (Vec<SurfaceVertex>, Vec<TerrainU>) {
    use vearth_core::mesh::{TERRAIN_BLEND_FACE, encode_surface_normal};
    let mut vertices = Vec::new();
    let mut frames = Vec::new();
    let distance = view.distance;
    let tan_y = (55.0f32.to_radians() * 0.5).tan();
    let tan_x = tan_y * WIDTH as f32 / HEIGHT as f32;
    for (i, panel) in panels.iter().enumerate() {
        let x = ((i % 12) as f32 + 0.5) / 6.0 - 1.0;
        let y = 1.0 - ((i / 12) as f32 + 0.5) / 5.0;
        let sx = tan_x * distance / 12.0 * 0.83;
        let sy = tan_y * distance / 10.0 * 0.83;
        let slant = view.slant;
        let model = Mat4::from_cols(
            glam::Vec4::new(sx, 0.0, sx * slant, 0.0),
            glam::Vec4::new(sy * 0.12, sy, sy * slant * 0.4, 0.0),
            glam::Vec4::new(0.0, 0.0, (sx + sy) * 0.5, 0.0),
            glam::Vec4::new(x * tan_x * distance, y * tan_y * distance, -distance, 1.0),
        );
        let axes = [
            model
                .y_axis
                .truncate()
                .cross(model.z_axis.truncate())
                .normalize(),
            model
                .z_axis
                .truncate()
                .cross(model.x_axis.truncate())
                .normalize(),
            model
                .x_axis
                .truncate()
                .cross(model.y_axis.truncate())
                .normalize(),
        ];
        frames.push(
            TerrainU {
                model: model.to_cols_array(),
                bx: axes[0].extend(0.0).to_array(),
                by: axes[1].extend(2.125).to_array(),
                bz: axes[2].extend(0.0).to_array(),
                fade: [panel.fade, panel.tree_blend, 0.0, 0.0],
            }
            .with_world_normal_parameters(world_normal_parameters(model.to_cols_array())),
        );
        let corners = [
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ];
        for corner in [0, 1, 2, 0, 2, 3] {
            let mut face_ao = panel.payload;
            let mut rgba = [110 + (i % 75) as u8, 120, 95, 255];
            if !matches!(panel.pattern, VertexPattern::Constant) {
                let normal = [
                    [-0.24, -0.18, 1.0],
                    [0.26, -0.12, 1.0],
                    [0.18, 0.26, 1.0],
                    [-0.23, 0.17, 1.0],
                ][corner];
                let face = if matches!(panel.pattern, VertexPattern::TerrainBlend) {
                    TERRAIN_BLEND_FACE
                } else {
                    4
                };
                let ao_or_family = [0, 1, 3, 2][corner];
                face_ao = encode_surface_normal(
                    (panel.payload & !31) | face | (ao_or_family << 3),
                    normal,
                );
                rgba = [
                    [73, 147, 99, 255],
                    [195, 88, 123, 255],
                    [119, 171, 207, 255],
                    [169, 133, 67, 255],
                ][corner];
            }
            vertices.push(SurfaceVertex {
                pos: corners[corner],
                face_ao,
                rgba,
            });
        }
    }
    (vertices, frames)
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    layers: u32,
    mips: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: layers,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn upload_layer(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip: u32,
    layer: u32,
    side: u32,
    data: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(side * 4),
            rows_per_image: Some(side),
        },
        wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
}

fn read_buffer(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<f32> {
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let result = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    buffer.unmap();
    result
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

#[test]
fn opaque_fragment_preserves_native_full_and_packed_color_depth_and_feedback() {
    verify_world_interface(WorldInterface::Original);
}

fn verify_highrise_pigment_and_coverage(device: &wgpu::Device, queue: &wgpu::Queue) {
    // Execute the production material helper with known derivatives. Lighting
    // and photographic exposure cannot conceal a glass-tinted opaque band.
    let source = include_str!("shader.wgsl");
    let helpers = &source[source.find("struct HighriseSample").unwrap()
        ..source.find("fn highrise_material(").unwrap()];
    let source = format!(
        "{}\n{helpers}\n{}",
        highrise_shader_profiles(),
        r#"
@group(0) @binding(0) var<storage, read_write> samples: array<vec4<f32>>;
@compute @workgroup_size(1)
fn probe(@builtin(global_invocation_id) gid: vec3<u32>) {
    let material = gid.x / 6u;
    let case_id = gid.x % 6u;
    let floor_coord = select(0.05, 0.5, case_id == 1u || case_id == 3u || case_id == 5u);
    let footprint = select(0.001, 2.0, case_id == 2u || case_id == 3u || case_id == 5u);
    let variant = select(0u, 1u, case_id >= 4u);
    let sample = highrise_sample(vec3<f32>(0.7, 0.12, 0.025), 103u + material,
        variant, floor_coord, footprint, highrise_profile(material % 8u));
    samples[gid.x * 2u] = vec4<f32>(sample.color, sample.glass);
    samples[gid.x * 2u + 1u] = vec4<f32>(sample.emission, sample.roughness);
}
"#
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production highrise pigment and area coverage"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(source)),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("highrise numerical probe"),
        layout: None,
        module: &shader,
        entry_point: Some("probe"),
        compilation_options: Default::default(),
        cache: None,
    });
    let size = 16 * 6 * 2 * 16;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[entry(0, output.as_entire_binding())],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(16 * 6, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let values = read_buffer(device, &readback);
    let wall = [0.7_f64, 0.12, 0.025];
    for material in 0..16 {
        let system = vearth_gen::highrise::System::ALL[material % 8];
        let profile = system.profile();
        let duty = profile.glazing.1 - profile.glazing.0;
        let glass_color = profile.glass.map(|c| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        for case_id in 0..6 {
            let distant = [2, 3, 5].contains(&case_id);
            let phase: f64 = if [1, 3, 5].contains(&case_id) {
                0.5
            } else {
                0.05
            };
            let t = (phase / 0.6).clamp(0.0, 1.0);
            let interior = if distant {
                1.0
            } else {
                0.93 + 0.1 * t * t * (3.0 - 2.0 * t)
            };
            let opening = if distant {
                duty
            } else if phase > profile.glazing.0 && phase < profile.glazing.1 {
                1.0
            } else {
                0.0
            };
            let coarse = case_id >= 4;
            let glass = if coarse {
                (1.0 - 2.0 * profile.pier_m / profile.bay_m.max(4.0)).clamp(0.0, 1.0)
                    * if system.residential() {
                        duty
                    } else {
                        0.72 + 0.28 * duty
                    }
            } else if system.residential() {
                opening
            } else {
                0.72 + opening * 0.28
            };
            let expected: [f64; 3] = std::array::from_fn(|i| {
                if coarse {
                    wall[i]
                } else {
                    wall[i] * (1.0 - glass) + glass_color[i] * interior * glass
                }
            });
            let offset = (material * 6 + case_id) * 8;
            for (channel, expected) in expected.into_iter().chain([glass]).enumerate() {
                assert!(
                    (f64::from(values[offset + channel]) - expected).abs() < 2e-5,
                    "{system:?} case {case_id} channel {channel}: {} != {expected}",
                    values[offset + channel]
                );
            }
            if material < 8 || (system.residential() && case_id == 0) {
                assert_eq!(&values[offset + 4..offset + 7], &[0.0; 3]);
            }
        }
    }
}

pub(super) fn verify_world_interface(interface: WorldInterface) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            assert_eq!(
                interface,
                WorldInterface::Original,
                "world interface GPU fixture requires an adapter: {error}"
            );
            eprintln!("opaque shader GPU fixture skipped: {error}");
            return;
        }
    };
    let required_features = if interface == WorldInterface::Half {
        if !adapter.features().contains(wgpu::Features::SHADER_F16) {
            eprintln!("half world interface GPU fixture unsupported: SHADER_F16 unavailable");
            return;
        }
        wgpu::Features::SHADER_F16
    } else {
        wgpu::Features::empty()
    };
    eprintln!(
        "opaque fragment GPU ({interface:?}): {:?}",
        adapter.get_info()
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("opaque fragment parity"),
        required_features,
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .unwrap();
    if interface == WorldInterface::Original {
        verify_highrise_pigment_and_coverage(&device, &queue);
    }
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("opaque and independent feedback reference"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!(
            "{}\n{REFERENCE}",
            world_shader_source_with_interface(interface),
        ))),
    });
    let bind = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty,
        count: None,
    };
    let uniform = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let image = |dimension, sample_type| wgpu::BindingType::Texture {
        sample_type,
        view_dimension: dimension,
        multisampled: false,
    };
    let sample_type = wgpu::TextureSampleType::Float { filterable: true };
    let sampler_type = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
    let entries = [
        vec![
            bind(0, uniform),
            bind(1, uniform),
            bind(2, image(wgpu::TextureViewDimension::D2, sample_type)),
            bind(3, sampler_type),
            bind(4, image(wgpu::TextureViewDimension::D2, sample_type)),
            bind(5, sampler_type),
            bind(6, uniform),
        ],
        vec![bind(
            0,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        )],
        vec![
            bind(0, image(wgpu::TextureViewDimension::D2Array, sample_type)),
            bind(1, sampler_type),
            bind(2, image(wgpu::TextureViewDimension::D2Array, sample_type)),
        ],
        vec![
            bind(
                0,
                image(
                    wgpu::TextureViewDimension::D2Array,
                    wgpu::TextureSampleType::Depth,
                ),
            ),
            bind(
                1,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            ),
            bind(2, uniform),
        ],
    ];
    let layouts: Vec<_> = entries
        .iter()
        .map(|entries| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries,
            })
        })
        .collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &layouts.iter().map(Some).collect::<Vec<_>>(),
        immediate_size: 0,
    });
    let attributes_full = wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32, 2 => Uint8x4];
    let attributes_packed = wgpu::vertex_attr_array![0 => Uint32, 1 => Uint32, 2 => Uint8x4];
    let pipelines: Vec<_> = [false, true]
        .into_iter()
        .map(|packed| {
            let opaque_entry = if interface == WorldInterface::Original {
                "fs_opaque"
            } else {
                "fs_world_opaque"
            };
            ["fs_reference", "fs_main", opaque_entry].map(|entry| {
                let vertex_entry = match (entry == "fs_world_opaque", packed) {
                    (false, false) => "vs_main",
                    (false, true) => "vs_packed",
                    (true, false) => "vs_world",
                    (true, true) => "vs_world_packed",
                };
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(vertex_entry),
                        compilation_options: Default::default(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: if packed {
                                std::mem::size_of::<PackedSurfaceVertex>()
                            } else {
                                std::mem::size_of::<SurfaceVertex>()
                            } as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: if packed {
                                &attributes_packed
                            } else {
                                &attributes_full
                            },
                        })],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(entry),
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
                })
            })
        })
        .collect();
    let atlas = texture(
        &device,
        "production physical atlas",
        crate::atlas::TILE as u32,
        crate::atlas::TILE as u32,
        crate::atlas::LAYERS as u32,
        crate::atlas::MIPS as u32,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    for (mip, layers) in crate::atlas::build().iter().enumerate() {
        for (layer, pixels) in layers.iter().enumerate() {
            upload_layer(
                &queue,
                &atlas,
                mip as u32,
                layer as u32,
                (crate::atlas::TILE >> mip) as u32,
                pixels,
            );
        }
    }
    let environment = texture(
        &device,
        "bounded reflection colors",
        4,
        4,
        1,
        1,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let pixels: Vec<u8> = (0..16)
        .flat_map(|i| [80 + i * 7, 130 + i * 4, 180 + i * 2, 255])
        .collect();
    upload_layer(&queue, &environment, 0, 0, 4, &pixels);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let comparison = device.create_sampler(&wgpu::SamplerDescriptor {
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });
    let shadow = texture(
        &device,
        "two shadow layers",
        4,
        4,
        2,
        1,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let shadow_uniform = SunShadowU {
        light_view_proj_0: Mat4::IDENTITY.to_cols_array(),
        light_view_proj_1: Mat4::IDENTITY.to_cols_array(),
        splits: [1000.0, 2000.0, 840.0, 1800.0],
        tuning: [0.25, 0.4, 0.0001, 0.0002],
    };
    let buffer = |label, bytes: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage,
        })
    };
    let shadow_buffer = buffer(
        "shadow receiver",
        bytemuck::bytes_of(&shadow_uniform),
        wgpu::BufferUsages::UNIFORM,
    );
    let atlas_view = atlas.create_view(&Default::default());
    let environment_view = environment.create_view(&Default::default());
    let shadow_view = shadow.create_view(&Default::default());
    let material_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layouts[2],
        entries: &[
            entry(0, wgpu::BindingResource::TextureView(&atlas_view)),
            entry(1, wgpu::BindingResource::Sampler(&sampler)),
            entry(2, wgpu::BindingResource::TextureView(&atlas_view)),
        ],
    });
    let shadow_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layouts[3],
        entries: &[
            entry(0, wgpu::BindingResource::TextureView(&shadow_view)),
            entry(1, wgpu::BindingResource::Sampler(&comparison)),
            entry(2, shadow_buffer.as_entire_binding()),
        ],
    });
    let panels = panels();
    let color = texture(
        &device,
        "parity color",
        WIDTH,
        HEIGHT,
        1,
        1,
        wgpu::TextureFormat::Rgba32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        &device,
        "parity depth",
        WIDTH,
        HEIGHT,
        1,
        1,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let color_view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let mut max_error = 0.0f32;
    let mut compared = 0usize;
    for view in VIEWS {
        let globals = globals(view);
        let sky = sky_model_lighting(globals.sun_dir[1], 0.08, 0.5, 0.0).uniform;
        let globals_buffer = buffer(
            "globals",
            bytemuck::bytes_of(&globals),
            wgpu::BufferUsages::UNIFORM,
        );
        let sky_buffer = buffer("sky", bytemuck::bytes_of(&sky), wgpu::BufferUsages::UNIFORM);
        let cloud_frame_buffer = buffer(
            "disabled cloud sunlight",
            bytemuck::bytes_of(&crate::cloud_shadows::Frame::default()),
            wgpu::BufferUsages::UNIFORM,
        );
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layouts[0],
            entries: &[
                entry(0, globals_buffer.as_entire_binding()),
                entry(1, sky_buffer.as_entire_binding()),
                entry(2, wgpu::BindingResource::TextureView(&environment_view)),
                entry(3, wgpu::BindingResource::Sampler(&sampler)),
                entry(4, wgpu::BindingResource::TextureView(&environment_view)),
                entry(5, wgpu::BindingResource::Sampler(&sampler)),
                entry(6, cloud_frame_buffer.as_entire_binding()),
            ],
        });
        let (vertices, frames) = geometry(&panels, view);
        let packed: Vec<_> = vertices
            .iter()
            .copied()
            .map(|vertex| {
                PackedSurfaceVertex::try_from_full(vertex)
                    .expect("fixture uses losslessly packed vertices")
            })
            .collect();
        assert_eq!(
            packed
                .iter()
                .copied()
                .map(PackedSurfaceVertex::to_full)
                .collect::<Vec<_>>(),
            vertices
        );
        let frame_buffer = buffer(
            "frames",
            bytemuck::cast_slice(&frames),
            wgpu::BufferUsages::STORAGE,
        );
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layouts[1],
            entries: &[entry(0, frame_buffer.as_entire_binding())],
        });
        let groups = [&globals_group, &frame_group, &material_group, &shadow_group];
        for packed_format in [false, true] {
            let vertex_buffer = buffer(
                "surface vertices",
                if packed_format {
                    bytemuck::cast_slice(&packed)
                } else {
                    bytemuck::cast_slice(&vertices)
                },
                wgpu::BufferUsages::VERTEX,
            );
            let mut rendered = Vec::new();
            for route in 0..3 {
                let color_readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(WIDTH * HEIGHT) * 16,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let depth_readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(WIDTH * HEIGHT) * 4,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("opaque material equivalence"),
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
                    for (index, group) in groups.iter().enumerate() {
                        pass.set_bind_group(index as u32, *group, &[]);
                    }
                    pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                    for (i, panel) in panels.iter().enumerate() {
                        let selected = if route == 2 && !panel.opaque {
                            1
                        } else {
                            route
                        };
                        pass.set_pipeline(&pipelines[usize::from(packed_format)][selected]);
                        pass.draw(i as u32 * 6..(i as u32 + 1) * 6, i as u32..i as u32 + 1);
                    }
                }
                for (texture, readback, bytes, aspect) in [
                    (&color, &color_readback, 16, wgpu::TextureAspect::All),
                    (&depth, &depth_readback, 4, wgpu::TextureAspect::DepthOnly),
                ] {
                    encoder.copy_texture_to_buffer(
                        wgpu::TexelCopyTextureInfo {
                            texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect,
                        },
                        wgpu::TexelCopyBufferInfo {
                            buffer: readback,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(WIDTH * bytes),
                                rows_per_image: Some(HEIGHT),
                            },
                        },
                        wgpu::Extent3d {
                            width: WIDTH,
                            height: HEIGHT,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                queue.submit([encoder.finish()]);
                rendered.push((
                    read_buffer(&device, &color_readback),
                    read_buffer(&device, &depth_readback),
                ));
            }
            for route in 1..3 {
                assert_eq!(
                    rendered[0]
                        .1
                        .iter()
                        .map(|value| value.to_bits())
                        .collect::<Vec<_>>(),
                    rendered[route]
                        .1
                        .iter()
                        .map(|value| value.to_bits())
                        .collect::<Vec<_>>(),
                    "coverage/depth changed: view={}, packed={packed_format}, route={route}",
                    view.name
                );
                for (expected, actual) in rendered[0].0.iter().zip(&rendered[route].0) {
                    assert!(expected.is_finite() && actual.is_finite());
                    let error = (expected - actual).abs();
                    max_error = max_error.max(error);
                    // Half interstage values allow a bounded change in the
                    // material response. Full precision keeps the existing
                    // helper-inlining allowance; coverage/depth stay exact.
                    let tolerance = if interface == WorldInterface::Half && route == 2 {
                        2.0e-3
                    } else {
                        2.0e-5
                    };
                    assert!(
                        error <= tolerance * (1.0 + expected.abs()),
                        "color {expected} -> {actual}: interface={interface:?}, view={}, packed={packed_format}, route={route}",
                        view.name
                    );
                    compared += 1;
                }
            }
            assert!(
                rendered[0].1.iter().filter(|value| **value > 0.0).count()
                    > WIDTH as usize * HEIGHT as usize / 3,
                "fixture must render substantial visible material coverage"
            );
        }
    }
    eprintln!(
        "opaque/feedback native parity ({interface:?}): panels={}, scalar comparisons={compared}, max color error={max_error}, depth and coverage bit-exact",
        panels.len()
    );
}
