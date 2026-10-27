use glam::Vec3;
use std::{borrow::Cow, time::Duration};
use wgpu::util::DeviceExt;

const HARNESS: &str = r#"
struct Case {
    view_material: vec4<f32>,
    light_variance: vec4<f32>,
    albedo: vec4<f32>,
};
struct Result {
    response: vec4<f32>,
    parameters: vec4<f32>,
};
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<Result>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    if (index.x >= arrayLength(&cases)) { return; }
    let c = cases[index.x];
    if (c.view_material.w < 0.0) {
        results[index.x].response = vec4<f32>(vehicle_environment_radiance(c.albedo.rgb), 1.0);
        results[index.x].parameters = vec4<f32>(0.0);
        return;
    }
    let material = vehicle_material(u32(c.view_material.w));
    results[index.x].response = vec4<f32>(vehicle_direct_response(material,
        c.albedo.rgb, vec3<f32>(0.0, 0.0, 1.0), c.view_material.xyz,
        c.light_variance.xyz, c.light_variance.w), 1.0);
    results[index.x].parameters = vec4<f32>(material.roughness, material.metallic,
        material.clearcoat, material.emission);
}
"#;

fn source() -> String {
    let world = include_str!("../src/shader.wgsl");
    let start = world.find("struct VehicleMaterial").unwrap();
    let end = world.find("fn vehicle_lighting(").unwrap();
    format!("{}\n{HARNESS}", &world[start..end])
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    view_material: [f32; 4],
    light_variance: [f32; 4],
    albedo: [f32; 4],
}

impl Case {
    fn new(material: u32, view: Vec3, light: Vec3, variance: f32) -> Self {
        Self {
            view_material: [view.x, view.y, view.z, material as f32],
            light_variance: [light.x, light.y, light.z, variance],
            albedo: [0.65, 0.42, 0.18, 1.0],
        }
    }

    fn environment(sky: Vec3) -> Self {
        Self {
            view_material: [0.0, 0.0, 1.0, -1.0],
            light_variance: [0.0; 4],
            albedo: [sky.x, sky.y, sky.z, 1.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Result {
    response: [f32; 4],
    parameters: [f32; 4],
}

fn evaluate(cases: &[Case]) -> Option<Vec<Result>> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("vehicle material GPU test skipped: no adapter ({error})");
            return None;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("vehicle material numerical test"),
        ..Default::default()
    }))
    .expect("vehicle material test device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production vehicle material helpers"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(source())),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("vehicle material test"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vehicle material cases"),
        contents: bytemuck::cast_slice(cases),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = (cases.len() * std::mem::size_of::<Result>()) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vehicle material results"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vehicle material readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("vehicle material inputs"),
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
fn vehicle_material_helpers_validate_without_an_adapter() {
    let module = naga::front::wgsl::parse_str(&source()).expect("parse vehicle material helpers");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validate vehicle material helpers");
}

#[test]
fn vehicle_reflections_preserve_sky_hue_without_overwhelming_painted_surfaces() {
    let skies = [
        Vec3::ZERO,
        Vec3::splat(0.001),
        Vec3::new(0.002, 0.004, 0.010),
        Vec3::new(0.06, 0.12, 0.20),
        Vec3::splat(0.22),
        Vec3::splat(0.24),
        Vec3::new(0.35, 0.70, 1.50),
        Vec3::new(2.0, 8.0, 18.0),
        Vec3::new(16.0, 8.0, 2.0),
        Vec3::new(10.0, 25.0, 75.0),
        Vec3::new(1.0e6, 2.0e6, 4.0e6),
        Vec3::new(1.0e30, 2.0e30, 3.0e30),
        Vec3::new(1.0e6, 0.0, 0.0),
        Vec3::new(0.0, 1.0e6, 0.0),
        Vec3::new(0.0, 0.0, 1.0e6),
        Vec3::new(-1.0, 0.12, 0.20),
        Vec3::splat(-1.0),
    ];
    let cases: Vec<_> = skies.into_iter().map(Case::environment).collect();
    let Some(results) = evaluate(&cases) else {
        return;
    };
    let weights = Vec3::new(0.2126, 0.7152, 0.0722);
    for (sky, result) in skies.into_iter().zip(results) {
        let input = sky.max(Vec3::ZERO);
        let actual = Vec3::from_slice(&result.response[..3]);
        assert!(actual.is_finite() && actual.min_element() >= 0.0);
        assert!(actual.dot(weights) <= 0.320005, "{sky:?} -> {actual:?}");
        assert!(actual.cmple(input + Vec3::splat(1.0e-6)).all());
        if input.dot(weights) <= 0.23 {
            assert!((actual - input).abs().max_element() < 1.0e-6);
        }
        if input.max_element() > 0.0 {
            assert!(actual.max_element() > 0.0);
            let input_ratios = input / input.max_element();
            let output_ratios = actual / actual.max_element();
            assert!(
                (input_ratios - output_ratios).abs().max_element() < 2.0e-5,
                "reflection hue changed: {sky:?} -> {actual:?}"
            );
        } else {
            assert_eq!(actual, Vec3::ZERO);
        }
    }
}

#[test]
fn vehicle_materials_keep_bounded_energy_and_distinct_surface_responses() {
    let mut cases = Vec::new();
    for material in 96..=102 {
        for view in [Vec3::Z, Vec3::new(0.98, 0.0, 0.2).normalize()] {
            for light in [Vec3::Z, -Vec3::Z, -view, Vec3::X] {
                cases.push(Case::new(material, view, light, 0.0));
            }
        }
    }
    let specular_start = cases.len();
    for material in [96, 97, 98, 99] {
        for variance in [0.0, 0.04, 0.25] {
            cases.push(Case::new(material, Vec3::Z, Vec3::Z, variance));
        }
    }
    let integral_start = cases.len();
    const ELEVATIONS: usize = 128;
    const AZIMUTHS: usize = 64;
    for material in [96, 97, 98, 99] {
        for elevation in 0..ELEVATIONS {
            let cosine = (elevation as f32 + 0.5) / ELEVATIONS as f32;
            let sine = (1.0 - cosine * cosine).sqrt();
            for azimuth in 0..AZIMUTHS {
                let angle = std::f32::consts::TAU * (azimuth as f32 + 0.5) / AZIMUTHS as f32;
                let light = Vec3::new(sine * angle.cos(), sine * angle.sin(), cosine);
                cases.push(Case::new(material, Vec3::Z, light, 0.0));
            }
        }
    }
    let Some(results) = evaluate(&cases) else {
        return;
    };
    for (case, result) in cases.iter().zip(&results) {
        assert!(result.response.iter().all(|x| x.is_finite() && *x >= 0.0));
        if case.light_variance[2] <= 0.0 {
            assert_eq!(&result.response[..3], &[0.0; 3]);
        }
    }
    assert_eq!(results[8].parameters[1], 1.0, "bare metal is conductive");
    assert_eq!(results[0].parameters[1], 0.0, "paint is dielectric");
    assert!(results[0].parameters[2] > 0.0, "paint has a clear coat");
    assert!(results[16].parameters[0] > results[24].parameters[0]);
    assert!(results[32].parameters[3] > results[40].parameters[3]);
    for sequence in results[specular_start..integral_start].chunks_exact(3) {
        assert!(sequence[0].response[0] > sequence[1].response[0]);
        assert!(sequence[1].response[0] > sequence[2].response[0]);
    }
    for integral in results[integral_start..].chunks_exact(ELEVATIONS * AZIMUTHS) {
        let reflectance = integral.iter().fold(Vec3::ZERO, |sum, value| {
            sum + Vec3::from_slice(&value.response[..3])
        }) * (2.0 / (ELEVATIONS * AZIMUTHS) as f32);
        assert!(reflectance.max_element() <= 1.02, "{reflectance:?}");
        assert!(reflectance.min_element() > 0.0);
    }
}
