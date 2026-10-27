use super::{TerrainU, world_normal_parameters};
use glam::{DMat4, DVec3, Vec3};
use std::{borrow::Cow, time::Duration};
use vearth_core::{
    cell::{CellId, ChunkAddr},
    cubesphere::Face,
    frame::ChunkFrame,
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    frame: TerrainU,
    payload: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ResultRow {
    actual: [f32; 4],
    reference: [f32; 4],
}

fn shader() -> String {
    let source = include_str!("shader.wgsl");
    let record =
        &source[source.find("struct TerrainU").unwrap()..source.find("struct LandmarkU").unwrap()];
    let helpers = &source[source.find("const TERRAIN_SLOPE_LEVELS").unwrap()
        ..source.find("// Face 6 is reserved").unwrap()];
    format!(
        "{record}\n{helpers}\n{}",
        r#"
struct Case { frame: TerrainU, payload: vec4<u32>, };
struct ResultRow { actual: vec4<f32>, reference: vec4<f32>, };
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<ResultRow>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    if invocation.x >= arrayLength(&cases) { return; }
    let test = cases[invocation.x];
    let c = test.frame;
    let payload = test.payload.x;
    var fallback = -c.bz.xyz;
    switch payload & 7u {
        case 0u: { fallback = c.bx.xyz; }
        case 1u: { fallback = -c.bx.xyz; }
        case 2u: { fallback = c.by.xyz; }
        case 3u: { fallback = -c.by.xyz; }
        case 4u: { fallback = c.bz.xyz; }
        default: {}
    }
    var old = surface_normal(payload, c.model, fallback);
    var candidate = world_surface_normal(payload, c.bx, c.by, c.bz, c.fade.z, fallback);
    let water_context = ((payload >> 5u) & 127u) == 1u && ((payload >> 13u) & 3u) != 0u;
    if !water_context && (payload & 0x80000000u) == 0u {
        old = terrain_lighting_normal(test.payload.y, c.model, old);
        candidate = terrain_lighting_normal(test.payload.y, c.model, candidate);
    }
    results[invocation.x].actual = vec4<f32>(candidate, c.fade.w);
    results[invocation.x].reference = vec4<f32>(old, length(c.model[0].xyz));
}
"#
    )
}

fn record(model: DMat4, axes: [DVec3; 3]) -> TerrainU {
    let model = model.to_cols_array().map(|value| value as f32);
    let normal = world_normal_parameters(model);
    TerrainU {
        model,
        bx: axes[0].extend(0.0).to_array().map(|v| v as f32),
        by: axes[1].extend(2.125).to_array().map(|v| v as f32),
        bz: axes[2].extend(0.0).to_array().map(|v| v as f32),
        fade: [0.73, 0.28, 0.0, 0.0],
    }
    .with_world_normal_parameters(normal)
}

fn frame_record(face: u8, level: u8, i: u32, j: u32, k: i32) -> TerrainU {
    let frame = ChunkFrame::of(ChunkAddr {
        cell: CellId {
            face: Face(face),
            level,
            i,
            j,
        },
        k,
    });
    record(frame.model_matrix(), frame.face_normals())
}

#[test]
fn world_parameters_preserve_uniform_payload_and_ignore_translation() {
    for face in 0..6 {
        for level in [0, 3, 7, 11, 14, 16, 19, 30] {
            let n = 1_u32 << level;
            for (i, j) in [(0, 0), (n / 2, n / 2), (n - 1, n - 1)] {
                let frame = ChunkFrame::of(ChunkAddr {
                    cell: CellId {
                        face: Face(face),
                        level,
                        i,
                        j,
                    },
                    k: 0,
                });
                let model = frame.model_matrix();
                let parameters = world_normal_parameters(model.to_cols_array().map(|v| v as f32));
                assert!(parameters.iter().all(|v| v.is_finite() && *v != 0.0));
                for translation in [DVec3::ZERO, DVec3::splat(1e12), DVec3::splat(-1e9)] {
                    let mut shifted = model;
                    shifted.w_axis = translation.extend(1.0);
                    assert_eq!(
                        parameters,
                        world_normal_parameters(shifted.to_cols_array().map(|v| v as f32))
                    );
                }
            }
        }
    }
    let before = TerrainU {
        model: [3.0; 16],
        bx: [1.0, 2.0, 3.0, 0.0],
        by: [4.0, 5.0, 6.0, 2.125],
        bz: [7.0, 8.0, 9.0, 0.0],
        fade: [0.73, 0.28, 0.0, 0.0],
    };
    let after = before.with_world_normal_parameters([11.0, 12.0, 13.0, 14.0]);
    let before: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&before));
    let after: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&after));
    for index in 0..32 {
        if [19, 27, 30, 31].contains(&index) {
            continue;
        }
        assert_eq!(before[index], after[index], "preserved scalar {index}");
    }
    assert_eq!(
        [after[19], after[30], after[27], after[31]],
        [11.0, 12.0, 13.0, 14.0]
    );
    assert_eq!(std::mem::size_of::<TerrainU>(), 128);
}

#[test]
fn world_normal_comparison_shader_validates_without_an_adapter() {
    let module = naga::front::wgsl::parse_str(&shader()).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

fn evaluate(cases: &[Case]) -> Option<Vec<ResultRow>> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("world normal GPU comparison skipped: no adapter ({error})");
            return None;
        }
    };
    eprintln!("world normal comparison adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("world normal equivalence"),
        ..Default::default()
    }))
    .unwrap();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader())),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = (cases.len() * std::mem::size_of::<ResultRow>()) as u64;
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
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
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
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(cases.len().div_ceil(64) as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        sender.send(r).unwrap();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    receiver.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    Some(bytemuck::cast_slice(&mapped).to_vec())
}

#[test]
fn world_normals_and_pitch_match_existing_gpu_math() {
    let mut cases = Vec::new();
    let codes = [0, 1, 31, 63, 64, 95, 126, 127];
    for face in 0..6 {
        for level in [0, 3, 7, 11, 14, 16, 19, 30] {
            let n = 1_u32 << level;
            for i in [0, n / 2, n - 1] {
                for j in [0, n / 2, n - 1] {
                    let frame = frame_record(face, level, i, j, 0);
                    for x in codes {
                        for y in codes {
                            cases.push(Case {
                                frame,
                                payload: [
                                    0x80000000 | (x << 16) | (y << 23) | 2 | (81 << 5),
                                    255,
                                    0,
                                    0,
                                ],
                            });
                        }
                    }
                    for face_id in 0..8 {
                        for alpha in [0, 34, 85, 119, 136, 170, 238, 255] {
                            cases.push(Case {
                                frame,
                                payload: [face_id, alpha, 0, 0],
                            });
                        }
                        cases.push(Case {
                            frame,
                            payload: [face_id | (1 << 5) | (1 << 13), 7, 0, 0],
                        });
                    }
                }
            }
        }
        // Every representable octahedral normal in the most sheared production
        // corner frame, including both hemispheres and fold boundaries.
        let frame = frame_record(face, 19, 0, 0, -3);
        for x in 0..128 {
            for y in 0..128 {
                cases.push(Case {
                    frame,
                    payload: [0x80000000 | (x << 16) | (y << 23) | 6, 255, 0, 0],
                });
            }
        }
    }
    // Exercise determinant sign using the same formula, without changing any
    // player/tree/landmark record or routing those draws through this helper.
    for reflection in [1.0, -1.0] {
        let model = DMat4::from_cols(
            DVec3::new(3.0, 0.0, 0.0).extend(0.0),
            DVec3::new(0.0, 2.0, 0.0).extend(0.0),
            DVec3::new(1.0, 0.0, 4.0 * reflection).extend(0.0),
            DVec3::ZERO.extend(1.0),
        );
        let [x, y, z] = [
            model.x_axis.truncate(),
            model.y_axis.truncate(),
            model.z_axis.truncate(),
        ];
        let frame = record(
            model,
            [
                y.cross(z).normalize(),
                z.cross(x).normalize(),
                x.cross(y).normalize(),
            ],
        );
        for nx in codes {
            for ny in codes {
                cases.push(Case {
                    frame,
                    payload: [0x80000000 | (nx << 16) | (ny << 23), 255, 0, 0],
                });
            }
        }
    }
    let Some(result) = evaluate(&cases) else {
        return;
    };
    let mut maximum_normal_error = 0.0_f32;
    let mut maximum_pitch_error = 0.0_f32;
    for (index, (actual, test)) in result.iter().zip(&cases).enumerate() {
        assert!(
            actual
                .actual
                .iter()
                .chain(&actual.reference)
                .all(|v| v.is_finite()),
            "finite case {index}"
        );
        let error =
            (Vec3::from_slice(&actual.actual) - Vec3::from_slice(&actual.reference)).length();
        let pitch_error = (actual.actual[3] - actual.reference[3]).abs() / actual.reference[3];
        maximum_normal_error = maximum_normal_error.max(error);
        maximum_pitch_error = maximum_pitch_error.max(pitch_error);
        assert!(
            error <= 2.0e-6,
            "normal case {index} payload {:x}: error {error}",
            test.payload[0]
        );
        assert!(
            pitch_error <= 4.0e-7,
            "pitch case {index}: error {pitch_error}"
        );
        if test.payload[0] & 0x80000000 == 0 {
            assert_eq!(
                actual.actual[..3],
                actual.reference[..3],
                "non-explicit case {index}"
            );
        }
    }
    eprintln!(
        "world normal GPU cases={} max_normal_error={maximum_normal_error} max_pitch_relative_error={maximum_pitch_error}",
        cases.len()
    );
}
