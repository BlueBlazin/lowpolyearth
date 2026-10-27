use glam::Vec3;
use std::{borrow::Cow, time::Duration};
use vearth_app::atlas;
use wgpu::util::DeviceExt;

const TERRAIN: &str = include_str!("../src/terrain_material.wgsl");
const HARNESS: &str = r#"
@group(0) @binding(0) var atlas_tex: texture_2d_array<f32>;
@group(0) @binding(1) var atlas_samp: sampler;
struct Case {
    position_detail: vec4<f32>, normal_family: vec4<f32>, weights: vec4<f32>,
    dx: vec4<f32>, dy: vec4<f32>,
};
struct Result {
    normal_roughness: vec4<f32>, reference: vec4<f32>, tones: vec4<f32>,
};
@group(1) @binding(0) var<storage, read> cases: array<Case>;
@group(1) @binding(1) var<storage, read_write> results: array<Result>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    if (index.x >= arrayLength(&cases)) { return; }
    let c = cases[index.x];
    let family = u32(c.normal_family.w);
    let surface = terrain_material(c.position_detail.xyz, c.normal_family.xyz, c.weights,
        c.position_detail.w, c.dx.xyz, c.dy.xyz);
    let reference = sample_material_grad(c.position_detail.xyz, c.normal_family.xyz,
        family + 2u, TERRAIN_PERIODS[family], c.position_detail.w, c.dx.xyz, c.dy.xyz);
    results[index.x].normal_roughness = vec4<f32>(surface.normal, surface.roughness);
    results[index.x].reference = vec4<f32>(reference.normal, reference.roughness);
    results[index.x].tones = vec4<f32>(surface.tone, reference.tone, 0.0, 0.0);
}
"#;

const ATLAS_COMPARISON_HARNESS: &str = r#"
@group(0) @binding(0) var atlas_tex: texture_2d_array<f32>;
@group(0) @binding(1) var atlas_samp: sampler;
struct Case {
    position_detail: vec4<f32>, normal_family: vec4<f32>, weights: vec4<f32>,
    dx: vec4<f32>, dy: vec4<f32>,
};
struct Result {
    normal_roughness: vec4<f32>, reference: vec4<f32>, tones: vec4<f32>,
};
@group(1) @binding(0) var<storage, read> cases: array<Case>;
@group(1) @binding(1) var<storage, read_write> results: array<Result>;

// Unconditional two-sample reference, independent of endpoint branches.
fn reference_varied_material(uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>,
                            layer: u32, selector: f32) -> vec4<f32> {
    let index = u32(floor(selector));
    let blend = smoothstep(0.15, 0.85, fract(selector));
    let a = textureSampleGrad(atlas_tex, atlas_samp, uv + material_offset(index), i32(layer), dx, dy);
    let b = textureSampleGrad(atlas_tex, atlas_samp, uv + material_offset(index + 1u), i32(layer), dx, dy);
    return mix(a, b, blend);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    if (index.x >= arrayLength(&cases)) { return; }
    let c = cases[index.x];
    let layer = u32(c.normal_family.w);
    let selector = c.position_detail.w;
    results[index.x].normal_roughness = varied_material(c.position_detail.xy,
        c.dx.xy, c.dy.xy, layer, selector);
    results[index.x].reference = reference_varied_material(c.position_detail.xy,
        c.dx.xy, c.dy.xy, layer, selector);
    results[index.x].tones = vec4<f32>(smoothstep(0.15, 0.85, fract(selector)), 0.0, 0.0, 0.0);
}
"#;

const PAVING_HARNESS: &str = r#"
@group(0) @binding(0) var atlas_tex: texture_2d_array<f32>;
@group(0) @binding(1) var atlas_samp: sampler;
struct Case {
    position_detail: vec4<f32>, normal_family: vec4<f32>, weights: vec4<f32>,
    dx: vec4<f32>, dy: vec4<f32>,
};
struct Result {
    normal_roughness: vec4<f32>, reference: vec4<f32>, tones: vec4<f32>,
};
@group(1) @binding(0) var<storage, read> cases: array<Case>;
@group(1) @binding(1) var<storage, read_write> results: array<Result>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    if (index.x >= arrayLength(&cases)) { return; }
    let c = cases[index.x];
    let layer = u32(c.normal_family.w);
    let actual = sample_material_grad(c.position_detail.xyz, c.normal_family.xyz,
        layer, c.weights.x, 1.0, c.dx.xyz, c.dy.xyz);
    let fixed_bond = sample_material_selected_grad(c.position_detail.xyz, c.normal_family.xyz,
        layer, c.weights.x, 1.0, c.dx.xyz, c.dy.xyz, 0.0);
    results[index.x].normal_roughness = vec4<f32>(actual.normal, actual.roughness);
    results[index.x].reference = vec4<f32>(fixed_bond.normal, fixed_bond.roughness);
    results[index.x].tones = vec4<f32>(actual.tone, fixed_bond.tone, 0.0, 0.0);
}
"#;

fn source() -> String {
    source_with_harness(HARNESS)
}

fn source_with_harness(harness: &str) -> String {
    let world = include_str!("../src/shader.wgsl");
    let declarations = world.find("struct MaterialSample").unwrap();
    let implicit_derivatives = world.find("fn sample_material(").unwrap();
    let explicit_derivatives = world.find("fn sample_material_grad(").unwrap();
    let end = world.find("struct VehicleMaterial").unwrap();
    // The compute entry supplies physical derivatives; all atlas sampling,
    // filtering, gradient conversion and terrain blending are production code.
    format!(
        "{}\n{}\n{TERRAIN}\n{harness}",
        &world[declarations..implicit_derivatives],
        &world[explicit_derivatives..end]
    )
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    position_detail: [f32; 4],
    normal_family: [f32; 4],
    weights: [f32; 4],
    dx: [f32; 4],
    dy: [f32; 4],
}

impl Case {
    fn new(position: Vec3, normal: Vec3, weights: [f32; 4], footprint: f32) -> Self {
        Self {
            position_detail: [position.x, position.y, position.z, 1.0],
            normal_family: [normal.x, normal.y, normal.z, 0.0],
            weights,
            dx: [footprint, 0.0, 0.0, 0.0],
            dy: [0.0, 0.0, footprint, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Result {
    normal_roughness: [f32; 4],
    reference: [f32; 4],
    tones: [f32; 4],
}

#[test]
fn production_terrain_material_validates_without_an_adapter() {
    for shader in [
        source(),
        source_with_harness(ATLAS_COMPARISON_HARNESS),
        source_with_harness(PAVING_HARNESS),
    ] {
        let module = naga::front::wgsl::parse_str(&shader).expect("parse terrain material");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("validate terrain material");
    }
}

fn evaluate(cases: &[Case]) -> Option<Vec<Result>> {
    evaluate_source(cases, source())
}

fn evaluate_source(cases: &[Case], shader_source: String) -> Option<Vec<Result>> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("terrain material GPU test skipped: no adapter ({error})");
            return None;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("terrain material numerical test"),
        ..Default::default()
    }))
    .expect("terrain material test device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production terrain material helpers"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader_source)),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("terrain material numerical test"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("production material atlas"),
        size: wgpu::Extent3d {
            width: atlas::TILE as u32,
            height: atlas::TILE as u32,
            depth_or_array_layers: atlas::LAYERS as u32,
        },
        mip_level_count: atlas::MIPS as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip, layers) in atlas::build().iter().enumerate() {
        let side = (atlas::TILE >> mip) as u32;
        for (layer, data) in layers.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * atlas::CHANNELS as u32),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: 4,
        ..Default::default()
    });
    let atlas_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = (cases.len() * size_of::<Result>()) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let case_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &atlas_group, &[]);
        pass.set_bind_group(1, &case_group, &[]);
        pass.dispatch_workgroups(cases.len().div_ceil(64) as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap()
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .expect("terrain material readback");
    receiver.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    Some(bytemuck::cast_slice(&mapped).to_vec())
}

fn normal(result: Result) -> Vec3 {
    Vec3::from_slice(&result.normal_roughness[..3])
}

fn response_distance(a: Result, b: Result) -> f32 {
    (normal(a) - normal(b))
        .abs()
        .max_element()
        .max((a.normal_roughness[3] - b.normal_roughness[3]).abs())
        .max((a.tones[0] - b.tones[0]).abs())
}

#[test]
fn unit_paving_keeps_its_bond_at_origin_wraps_and_all_mips() {
    let mut cases = Vec::new();
    for (layer, period) in [(39, 4.0), (40, 2.0), (41, 2.0), (43, 2.0), (44, 4.0)] {
        for mip in 0..atlas::MIPS {
            for normal in [Vec3::Y, Vec3::new(0.25, 0.96, 0.125).normalize()] {
                // Binary-exact positions isolate repeat/selector behaviour
                // from float-coordinate quantization at the origin shift.
                for x in [-0.015625, 255.984375, 256.015625, 1024.25] {
                    for wrap in [Vec3::ZERO, Vec3::new(8192.0, 8192.0, 8192.0)] {
                        let mut case = Case::new(
                            Vec3::new(x, 7.25, 64.5) + wrap,
                            normal,
                            [period, 0.0, 0.0, 0.0],
                            period * 2_f32.powi(mip as i32) / atlas::TILE as f32,
                        );
                        case.normal_family[3] = layer as f32;
                        cases.push(case);
                    }
                }
            }
        }
    }
    let Some(results) = evaluate_source(&cases, source_with_harness(PAVING_HARNESS)) else {
        return;
    };
    for (case, result) in cases.iter().zip(&results) {
        for (actual, reference) in result.normal_roughness.iter().zip(&result.reference) {
            assert!(
                (actual - reference).abs() <= 0.000002,
                "paving bond drift at {:?}",
                case.position_detail
            );
        }
        assert!((result.tones[0] - result.tones[1]).abs() <= 0.000002);
    }
    for pair in results.chunks_exact(2) {
        assert!(
            response_distance(pair[0], pair[1]) <= 0.000002,
            "paving changed across an 8192 m material-origin wrap"
        );
    }
}

#[test]
fn actual_atlas_endpoint_skips_match_unconditional_sampling_across_layers_and_mips() {
    let mut cases = Vec::new();
    for layer in 0..atlas::LAYERS {
        for mip in 0..atlas::MIPS {
            let footprint = 2_f32.powi(mip as i32) / atlas::TILE as f32;
            for (dx, dy) in [
                ([1.0, 0.0], [0.0, 1.0]),
                ([0.6, 0.8], [-0.8, 0.6]),
                ([3.2, -2.4], [0.6, 0.8]),
            ] {
                for index in [0.0, 6.0] {
                    for fraction in [0.0, 0.1499, 0.15, 0.1501, 0.5, 0.8499, 0.85, 0.8501, 0.9999] {
                        let mut case = Case::new(Vec3::ZERO, Vec3::Y, [0.0; 4], 0.0);
                        let offset = cases.len() as f32;
                        case.position_detail = [
                            -0.137 + offset * 0.013,
                            1.937 - offset * 0.029,
                            0.0,
                            index + fraction,
                        ];
                        case.normal_family[3] = layer as f32;
                        case.dx = [dx[0] * footprint, dx[1] * footprint, 0.0, 0.0];
                        case.dy = [dy[0] * footprint, dy[1] * footprint, 0.0, 0.0];
                        cases.push(case);
                    }
                }
            }
        }
    }
    let Some(results) = evaluate_source(&cases, source_with_harness(ATLAS_COMPARISON_HARNESS))
    else {
        return;
    };
    let mut counts = [0; 3];
    for (case, result) in cases.iter().zip(results) {
        let blend = result.tones[0];
        counts[if blend <= 0.0 {
            0
        } else if blend >= 1.0 {
            2
        } else {
            1
        }] += 1;
        for (actual, reference) in result.normal_roughness.into_iter().zip(result.reference) {
            assert!(actual.is_finite() && reference.is_finite());
            assert!((0.0..=1.0).contains(&actual));
            // Endpoint mix lowering can round differently across GPU backends;
            // this is far below one RGBA8 channel step, even after filtering.
            assert!(
                (actual - reference).abs() <= 0.000002,
                "layer {} selector {} gradients {:?}/{:?}: {actual} != {reference}",
                case.normal_family[3],
                case.position_detail[3],
                case.dx,
                case.dy
            );
        }
    }
    assert!(counts.into_iter().all(|count| count > 100));
}

#[test]
fn actual_atlas_preserves_endpoints_continuity_origin_phase_and_minification() {
    let position = Vec3::new(12.25, -71.375, 1024.03125);
    let slope = Vec3::new(0.35, 0.9, -0.22).normalize();
    let mut cases = Vec::new();
    let mut endpoints = Vec::new();
    for family in 0..4 {
        for normal in [Vec3::Y, slope, Vec3::new(-0.8, 0.1, 0.4).normalize()] {
            for footprint in [0.03125, 32.0, 1024.0] {
                let mut weights = [0.0; 4];
                weights[family] = 1.0;
                let mut case = Case::new(position, normal, weights, footprint);
                case.normal_family[3] = family as f32;
                endpoints.push(cases.len());
                cases.push(case);
            }
        }
    }
    let mut blends = Vec::new();
    for a in 0..4 {
        for b in a + 1..4 {
            for offset in [Vec3::ZERO, Vec3::new(7.0, 3.0, -5.0)] {
                let start = cases.len();
                for step in 0..=64 {
                    let mut weights = [0.0; 4];
                    weights[a] = 1.0 - step as f32 / 64.0;
                    weights[b] = step as f32 / 64.0;
                    cases.push(Case::new(position + offset, slope, weights, 0.125));
                }
                blends.push(start..cases.len());
            }
        }
    }
    let mut wraps = Vec::new();
    for footprint in [0.03125, 16.0, 1024.0] {
        let case = Case::new(position, slope, [0.15, 0.35, 0.3, 0.2], footprint);
        let baseline = cases.len();
        cases.push(case);
        for axis in 0..3 {
            for shift in [-8192.0, 8192.0] {
                let mut shifted = case;
                shifted.position_detail[axis] += shift;
                wraps.push((baseline, cases.len()));
                cases.push(shifted);
            }
        }
    }
    let minification_start = cases.len();
    for footprint in [0.03125, 1024.0] {
        for offset in 0..32 {
            cases.push(Case::new(
                position + Vec3::X * offset as f32 * 0.5,
                slope,
                [0.0, 0.45, 0.55, 0.0],
                footprint,
            ));
        }
    }
    let disabled_start = cases.len();
    for family in 0..4 {
        let mut weights = [0.0; 4];
        weights[family] = 1.0;
        let mut case = Case::new(position, slope, weights, 0.03125);
        case.position_detail[3] = 0.0;
        cases.push(case);
    }
    let Some(results) = evaluate(&cases) else {
        return;
    };
    for (case, result) in cases.iter().zip(&results) {
        assert!(
            result
                .normal_roughness
                .iter()
                .chain(&result.tones)
                .all(|v| v.is_finite())
        );
        assert!((normal(*result).length() - 1.0).abs() < 0.00002);
        assert!(normal(*result).dot(Vec3::from_slice(&case.normal_family[..3])) > 0.95);
        assert!((0.1..=1.00001).contains(&result.normal_roughness[3]));
    }
    for index in endpoints {
        let result = results[index];
        for (actual, expected) in result.normal_roughness.into_iter().zip(result.reference) {
            assert!(
                (actual - expected).abs() < 0.00002,
                "pure material {index}: {result:?}"
            );
        }
        if cases[index].dx[0] >= 1024.0 {
            assert!((result.tones[0] - result.tones[1]).abs() < 0.00002);
        }
    }
    for range in blends {
        let blend = &results[range];
        let (first, last) = (blend.first().unwrap(), blend.last().unwrap());
        let minimum = first.normal_roughness[3].min(last.normal_roughness[3]) - 0.00002;
        let maximum = first.normal_roughness[3].max(last.normal_roughness[3]) + 0.00002;
        for result in blend {
            assert!((minimum..=maximum).contains(&result.normal_roughness[3]));
        }
        for pair in blend.windows(2) {
            assert!(
                response_distance(pair[0], pair[1]) < 0.025,
                "material blend jumped: {pair:?}"
            );
        }
    }
    for (baseline, shifted) in wraps {
        assert!(
            response_distance(results[baseline], results[shifted]) < 0.003,
            "origin wrap changed material: {:?} -> {:?}",
            results[baseline],
            results[shifted]
        );
    }
    let near = &results[minification_start..minification_start + 32];
    let far = &results[minification_start + 32..minification_start + 64];
    let variation = |values: &[Result]| {
        values
            .windows(2)
            .map(|pair| (normal(pair[0]) - normal(pair[1])).length())
            .sum::<f32>()
    };
    assert!(
        variation(near) > 0.0001,
        "fixture must exercise material relief"
    );
    assert!(variation(far) < variation(near) * 0.05);
    for result in far {
        assert!((normal(*result) - slope).length() < 0.002);
    }
    for result in &results[disabled_start..] {
        assert!((normal(*result) - slope).length() < 0.000001);
        assert_eq!(result.normal_roughness[3], 1.0);
        assert_eq!(result.tones[0], 0.0);
    }
}
