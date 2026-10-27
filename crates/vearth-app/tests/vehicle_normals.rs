use glam::{Mat3, Mat4, Vec3, Vec4};
use std::{borrow::Cow, time::Duration};
use vearth_app::{player::Player, vehicles::VehicleKind};
use vearth_core::mesh::{decode_surface_normal, encode_surface_normal};
use wgpu::util::DeviceExt;

const HARNESS: &str = r#"
struct Case {
    model: mat4x4<f32>,
    payload: vec4<u32>,
    fallback: vec4<f32>,
    light: vec4<f32>,
};
struct Result {
    normal: vec4<f32>,
    lighting: vec4<f32>,
};
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<Result>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= arrayLength(&cases)) { return; }
    let c = cases[id.x];
    let n = surface_normal(c.payload.x, c.model, c.fallback.xyz);
    let light = normalize(c.light.xyz);
    results[id.x].normal = vec4<f32>(n, 1.0);
    results[id.x].lighting = vec4<f32>(max(dot(n, light), 0.0),
        max(dot(n, -light), 0.0), 0.0, 0.0);
}
"#;

fn source() -> String {
    let world = include_str!("../src/shader.wgsl");
    let start = world.find("fn surface_normal(").unwrap();
    let end = world[start..].find("\nfn surface_vertex(").unwrap() + start;
    format!("{}\n{HARNESS}", &world[start..end])
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    model: [[f32; 4]; 4],
    payload: [u32; 4],
    fallback: [f32; 4],
    light: [f32; 4],
}

impl Case {
    fn new(model: Mat4, payload: u32, fallback: Vec3, light: Vec3) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            payload: [payload, 0, 0, 0],
            fallback: fallback.extend(0.0).to_array(),
            light: light.extend(0.0).to_array(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Result {
    normal: [f32; 4],
    lighting: [f32; 4],
}

fn evaluate(cases: &[Case]) -> Option<Vec<Result>> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("vehicle normal GPU test skipped: no adapter ({error})");
            return None;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("vehicle normal transform test"),
        ..Default::default()
    }))
    .expect("vehicle normal test device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production surface normal transform"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(source())),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("vehicle normal test"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vehicle normal cases"),
        contents: bytemuck::cast_slice(cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = (cases.len() * std::mem::size_of::<Result>()) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vehicle normal results"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vehicle normal readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("vehicle normal inputs"),
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
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups((cases.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
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
fn production_normal_transform_validates_without_an_adapter() {
    let module = naga::front::wgsl::parse_str(&source()).expect("parse normal transform");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validate normal transform");
}

#[test]
fn authored_normals_face_the_light_in_player_and_sheared_frames() {
    let mut transforms = vec![
        Mat4::IDENTITY,
        Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0)),
        Mat4::from_scale(Vec3::new(2.0, 0.5, 3.0)),
    ];
    for handedness in [1.0, -1.0] {
        transforms.push(Mat4::from_cols(
            Vec4::new(1.0, 0.2, 0.0, 0.0),
            Vec4::new(0.3, 2.0, 0.1, 0.0),
            Vec4::new(-0.1, 0.4, 0.7 * handedness, 0.0),
            Vec4::new(12.0, -5.0, 30.0, 1.0),
        ));
    }
    for (latitude, longitude, heading, lean) in [
        (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64),
        (45.435, 12.335, 73.0, 0.18),
        (-33.86, 151.21, 245.0, -0.24),
    ] {
        let position = vearth_core::ellipsoid::geodetic_to_ecef(
            latitude.to_radians(),
            longitude.to_radians(),
            300.0,
        );
        let mut player = Player::spawn_as(position, heading.to_radians(), VehicleKind::Kestrel);
        player.lean_f = lean;
        player.lean_r = lean * 0.7;
        let (model, _) = player.model_matrix();
        let model = Mat4::from_cols_array(&model.to_cols_array().map(|value| value as f32));
        assert!(
            model.determinant() > 0.0,
            "production frames must preserve authored lettering and winding"
        );
        transforms.push(model);
    }

    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for model in transforms {
        let inverse_transpose = Mat3::from_mat4(model).inverse().transpose();
        for normal in [
            Vec3::X,
            -Vec3::X,
            Vec3::Y,
            -Vec3::Y,
            Vec3::Z,
            -Vec3::Z,
            Vec3::new(1.0, 2.0, 3.0).normalize(),
            Vec3::new(-0.5, 0.8, -0.2).normalize(),
        ] {
            let payload = encode_surface_normal(2 | (3 << 3) | (96 << 5), normal.to_array());
            let encoded = Vec3::from_array(decode_surface_normal(payload).unwrap());
            let outward = (inverse_transpose * encoded).normalize();
            cases.push(Case::new(model, payload, Vec3::X, outward));
            expected.push(outward);
        }
        let fallback = Vec3::new(0.25, 0.5, 0.75);
        cases.push(Case::new(model, 2 | (3 << 3), fallback, fallback));
        expected.push(fallback);
    }
    let Some(results) = evaluate(&cases) else {
        return;
    };
    for (index, (result, expected)) in results.iter().zip(expected).enumerate() {
        let actual = Vec3::from_slice(&result.normal[..3]);
        assert!(actual.is_finite());
        assert!(
            (actual - expected).abs().max_element() < 2.0e-5,
            "case {index}: normal {actual:?} must face {expected:?}"
        );
        assert!(
            (result.lighting[0] - expected.length()).abs() < 2.0e-5,
            "case {index}: a light facing the authored surface must illuminate it"
        );
        assert!(
            result.lighting[1] < 1.0e-6,
            "case {index}: the opposite light must not illuminate the surface"
        );
    }
}
