use super::*;
use glam::{Mat3, Vec3};
use hw_skymodel::rgb::{SkyParams, SkyState};
use vearth_platform::time::Duration;
use wgpu::util::DeviceExt;

const COMPARE: &str = r#"
struct SkyComparison { analytic: vec4<f32>, cached: vec4<f32>, palette: vec4<f32> };
@group(2) @binding(0) var<storage, read> sky_directions: array<vec4<f32>>;
@group(2) @binding(1) var<storage, read_write> sky_comparisons: array<SkyComparison>;
@compute @workgroup_size(64)
fn compare_sky_environment(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&sky_directions) { return; }
    let direction = sky_directions[id.x].xyz;
    let up = normalize(-g.planet_center.xyz);
    let sky_conditions = vec3<f32>(max(length(g.planet_center.xyz) - g.planet_center.w, 0.0),
        g.surface.z, g.weather0.y);
    let analytic = water_reflected_environment(direction, up, g.sun_dir.xyz,
        g.moon_dir.xyz, g.moon_dir.w, g.weather1.z, g.weather5.w,
        sky_conditions);
    sky_comparisons[id.x].analytic = vec4<f32>(analytic, 1.0);
    sky_comparisons[id.x].cached = vec4<f32>(sample_reflected_environment(direction), 1.0);
    sky_comparisons[id.x].palette = vec4<f32>(sunset_sky_grade(vec3<f32>(1.0),
        dot(direction, up), dot(g.sun_dir.xyz, up), dot(direction, g.sun_dir.xyz),
        sky_conditions.x, sky_conditions.y, sky_conditions.z), 1.0);
}
"#;

fn comparison_source() -> String {
    format!(
        "{}\n{}\n{COMPARE}",
        compute_source(),
        include_str!("sky_environment_sample.wgsl"),
    )
}

#[test]
fn reflection_compute_and_sampling_validate_without_adapter() {
    let source = comparison_source();
    let module = naga::front::wgsl::parse_str(&source).expect("sky environment WGSL");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("sky environment validation");
    assert_eq!(SIDE * SIDE * 8, 32 * 1024);
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Comparison {
    analytic: [f32; 4],
    cached: [f32; 4],
    palette: [f32; 4],
}

fn rgb(value: [f32; 4]) -> Vec3 {
    Vec3::new(value[0], value[1], value[2])
}

#[test]
fn gpu_cached_sky_tracks_analytic_across_weather_horizon_and_camera_frames() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("sky environment GPU comparison skipped: no adapter ({error})");
            return;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
        .expect("sky environment fixture device");
    let source = comparison_source();
    let parsed = naga::front::wgsl::parse_str(&source).unwrap();
    let uniform = &parsed
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("Globals"))
        .unwrap()
        .1
        .inner;
    let naga::TypeInner::Struct { members, span } = uniform else {
        panic!("Globals must remain a struct");
    };
    let buffer = |label, bytes: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage,
        })
    };
    let globals = buffer(
        "fixture Globals",
        &vec![0; *span as usize],
        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    );
    let coefficients = buffer(
        "fixture Hosek coefficients",
        &[0; 128],
        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    );
    let environment = SkyEnvironment::new(&device, &globals, &coefficients);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("analytic versus cached sky"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(source)),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("analytic versus cached sky"),
        layout: None,
        module: &shader,
        entry_point: Some("compare_sky_environment"),
        compilation_options: Default::default(),
        cache: None,
    });
    let shared = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: coefficients.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&environment.view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&environment.sampler),
            },
        ],
    });
    let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[],
    });
    let mut local_directions = Vec::new();
    for index in 0..4096 {
        let z = 1.0 - 2.0 * (index as f32 + 0.5) / 4096.0;
        let angle = index as f32 * 2.399_963_1;
        let radius = (1.0 - z * z).sqrt();
        local_directions.push(Vec3::new(radius * angle.cos(), radius * angle.sin(), z));
    }
    // Adjacent reflected rays straddle the horizon and octahedral axis seams.
    for azimuth in 0..128 {
        let angle = azimuth as f32 * std::f32::consts::TAU / 128.0;
        for z in [-0.0001, 0.0001] {
            local_directions.push(Vec3::new(angle.cos(), angle.sin(), z).normalize());
        }
    }
    let byte_count = (local_directions.len() * size_of::<Comparison>()) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: byte_count,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: byte_count,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut mean_luminance = std::collections::HashMap::new();
    for (name, elevation_deg, turbidity, cloud, optical, up) in [
        ("noon", 60.0_f32, 2.0, 0.0, 0.0, Vec3::Z),
        (
            "dawn",
            2.0,
            3.0,
            0.0,
            0.0,
            Vec3::new(0.7, 0.3, 0.2).normalize(),
        ),
        ("haze", 20.0, 8.0, 0.0, 0.0, Vec3::X),
        ("overcast", 30.0, 5.0, 1.0, 12.0, Vec3::NEG_Z),
        ("thin overcast", 30.0, 5.0, 1.0, 5.0, Vec3::NEG_Z),
        ("twilight", -2.0, 4.0, 0.2, 3.0, Vec3::NEG_X),
        ("night", -25.0, 3.0, 0.4, 6.0, Vec3::Y),
        ("clear afternoon", 35.0, 3.0, 0.0, 0.0, Vec3::Z),
        ("broken cumulus", 35.0, 3.0, 0.6, 2.0, Vec3::Z),
    ] {
        let east = if up.truncate().length_squared() < 1e-12 {
            Vec3::Y
        } else {
            Vec3::new(-up.y, up.x, 0.0).normalize()
        };
        let frame = Mat3::from_cols(east, up.cross(east), up);
        let elevation = elevation_deg.to_radians();
        let sun = east * elevation.cos() + up * elevation.sin();
        let mut bytes = vec![0_u8; *span as usize];
        for (field, value) in [
            ("planet_center", (-up * 6_378_137.0).extend(6_378_137.0)),
            ("sun_dir", sun.extend(1.0)),
            ("moon_dir", up.extend(0.7)),
            ("weather1", glam::Vec4::new(0.0, 0.0, cloud, 0.0)),
            ("weather5", glam::Vec4::new(0.0, 0.0, 0.0, optical)),
            ("surface", glam::Vec4::new(0.0, 0.0, 0.16, 0.0)),
            ("weather0", glam::Vec4::new(0.0, 0.72, 0.0, 0.0)),
            ("weather3", glam::Vec4::new(600.0, 2600.0, 0.0, 0.0)),
        ] {
            let at = members
                .iter()
                .find(|m| m.name.as_deref() == Some(field))
                .unwrap()
                .offset as usize;
            bytes[at..at + 16].copy_from_slice(bytemuck::cast_slice(&value.to_array()));
        }
        queue.write_buffer(&globals, 0, &bytes);
        let sky = SkyState::new(&SkyParams {
            elevation: elevation.max(0.0),
            turbidity,
            albedo: [0.12, 0.13, 0.15],
        })
        .unwrap();
        let (parameters, radiances) = sky.raw();
        let mut model = [0.0_f32; 32];
        model[..27].copy_from_slice(&parameters);
        model[27..30].copy_from_slice(&radiances);
        queue.write_buffer(&coefficients, 0, bytemuck::cast_slice(&model));
        let directions: Vec<[f32; 4]> = local_directions
            .iter()
            .map(|direction| (frame * *direction).extend(0.0).to_array())
            .collect();
        let inputs = buffer(
            "reflected directions",
            bytemuck::cast_slice(&directions),
            wgpu::BufferUsages::STORAGE,
        );
        let cases = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(2),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: inputs.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        environment.encode(&mut encoder, None);
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &shared, &[]);
            pass.set_bind_group(1, &empty, &[]);
            pass.set_bind_group(2, &cases, &[]);
            pass.dispatch_workgroups(directions.len().div_ceil(64) as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, byte_count);
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let results: Vec<Comparison> =
            bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
        readback.unmap();
        mean_luminance.insert(
            name,
            results[..4096]
                .iter()
                .map(|r| rgb(r.analytic).dot(Vec3::new(0.2126, 0.7152, 0.0722)))
                .sum::<f32>()
                / 4096.0,
        );
        let mut errors = Vec::new();
        let mut total_error = 0.0;
        let mut total_signal = 0.0;
        for (result, direction) in results.iter().zip(&directions) {
            let direction = rgb(*direction);
            let expected_palette = Vec3::from_array(crate::gfx::sunset_sky_grade_cpu(
                [1.0; 3],
                direction.dot(up),
                sun.dot(up),
                direction.dot(sun),
                0.0,
                0.16,
                0.72,
            ));
            assert!(
                rgb(result.palette).distance(expected_palette) < 0.00001,
                "CPU aerial palette must match the visible and reflected sky grade"
            );
            let analytic = rgb(result.analytic);
            let cached = rgb(result.cached);
            assert!(cached.is_finite() && cached.min_element() >= 0.0);
            let error = (cached - analytic).abs().max_element();
            // A mixed absolute/relative bound avoids magnifying tiny twilight
            // channels: below 0.02 linear HDR, p95 permits at most 0.002.
            errors.push(error / analytic.max_element().max(0.02));
            total_error += (cached - analytic).length_squared();
            total_signal += analytic.length_squared();
        }
        errors.sort_by(f32::total_cmp);
        let relative_rms = (total_error / total_signal).sqrt();
        let p95 = errors[errors.len() * 95 / 100];
        eprintln!(
            "{name}: relative RMS={relative_rms:.6}, p95 normalized={p95:.6}, max={:.6}",
            errors.last().unwrap()
        );
        assert!(
            relative_rms < 0.035,
            "{name} environment energy error {relative_rms}"
        );
        assert!(p95 < 0.10, "{name} environment color error {p95}");
        assert!(errors.last().unwrap() < &0.12, "{name} localized sky error");
        for pair in results[4096..].chunks_exact(2) {
            let a = rgb(pair[0].cached);
            let b = rgb(pair[1].cached);
            assert!(
                a.distance(b) < 0.015 * a.length().max(0.02),
                "horizon seam in {name}"
            );
        }
    }
    assert!(
        mean_luminance["broken cumulus"] > mean_luminance["clear afternoon"] * 1.05,
        "broken cloud lightens the reflected sky: {mean_luminance:?}"
    );
    assert!(
        mean_luminance["overcast"] < mean_luminance["thin overcast"] * 0.9,
        "a raining deck darkens the reflected sky: {mean_luminance:?}"
    );
}
