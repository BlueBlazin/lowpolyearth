use glam::Vec3;
use hw_skymodel::rgb::{Channel, SkyParams, SkyState};
use std::{borrow::Cow, time::Duration};
use wgpu::util::DeviceExt;

const SOURCE: &str = include_str!("../src/water.wgsl");
const PALETTE: &str = include_str!("../src/sky_palette.wgsl");
const WORLD_SOURCE: &str = include_str!("../src/shader.wgsl");

// Exercise the production packed decoder with the same Rust-produced bytes
// without constructing the unrelated world-render bind groups.
fn packed_decoder() -> &'static str {
    let start = WORLD_SOURCE.find("fn unpack_fixed_i16(").unwrap();
    let end = WORLD_SOURCE[start..]
        .find("\n@vertex\nfn vs_packed(")
        .unwrap();
    &WORLD_SOURCE[start..start + end]
}

fn shader_source() -> String {
    let construction = include_str!("../src/construction_material.wgsl");
    let pigment = &construction[..construction.find("fn construction_height(").unwrap()];
    format!(
        "{PALETTE}\n{SOURCE}\n{pigment}\n{}\n{HARNESS}",
        packed_decoder()
    )
}

const HARNESS: &str = r#"
struct PackedVsIn {
    pos_xy: u32, pos_z_face: u32, rgba: vec4<u32>,
};
struct Case {
    position_time: vec4<f32>, normal_footprint: vec4<f32>, climate: vec4<f32>,
    sun_direction: vec4<f32>, moon_illumination: vec4<f32>, reflected_cosine: vec4<f32>,
    sky_conditions: vec4<f32>, crest_parameters: vec4<f32>, water_body: vec4<f32>,
    sunlight: vec4<f32>, water_metadata: vec4<u32>, packed_vertex: vec4<u32>,
};
struct Result {
    normal_roughness: vec4<f32>, response: vec4<f32>, environment: vec4<f32>, clear: vec4<f32>,
    variance_response: vec4<f32>, palette: vec4<f32>, sheltered: vec4<f32>,
    scattering: vec4<f32>,
    decoded_position: vec4<f32>, decoded_payload: vec4<u32>,
    bed: vec4<f32>, azulejo: vec4<f32>,
};
@group(1) @binding(0) var<storage, read> cases: array<Case>;
@group(1) @binding(1) var<storage, read_write> results: array<Result>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&cases) { return; }
    let c = cases[id.x];
    results[id.x].azulejo = vec4<f32>(azulejo_pattern_color(c.water_body.xyz, c.water_body.w), 0.0);
    let surface = water_surface(c.position_time.xyz, c.normal_footprint.xyz,
        c.climate.x, c.climate.y, c.position_time.w, c.normal_footprint.w);
    let energy = 0.30 + 1.15 * clamp(c.climate.y, 0.0, 1.0)
        + 0.55 * clamp(c.climate.x / 22.0, 0.0, 1.0);
    let field = water_wave_field(c.position_time.xyz, normalize(c.normal_footprint.xyz),
        energy, c.position_time.w, c.normal_footprint.w);
    results[id.x].normal_roughness = vec4<f32>(surface.normal, surface.roughness);
    results[id.x].response = vec4<f32>(surface.crest, water_fresnel(c.reflected_cosine.w),
        field.height, field.suppressed_variance);
    results[id.x].variance_response = vec4<f32>(
        water_roughness_from_slope_variance(0.12, 0.0),
        water_roughness_from_slope_variance(0.12, 0.001),
        water_roughness_from_slope_variance(0.12, 0.02),
        water_roughness_from_slope_variance(0.12, 0.08));
    results[id.x].environment = vec4<f32>(water_reflected_environment(c.reflected_cosine.xyz,
        c.normal_footprint.xyz, c.sun_direction.xyz, c.moon_illumination.xyz,
        c.moon_illumination.w, c.climate.z, c.climate.w,
        c.sky_conditions.xyz), 1.0);
    results[id.x].clear = vec4<f32>(water_clear_sky(c.reflected_cosine.xyz,
        c.normal_footprint.xyz, c.sun_direction.xyz, c.sky_conditions.xyz), 1.0);
    results[id.x].palette = vec4<f32>(sunset_sky_grade(vec3<f32>(1.0),
        dot(c.reflected_cosine.xyz, c.normal_footprint.xyz),
        dot(c.sun_direction.xyz, c.normal_footprint.xyz),
        dot(c.reflected_cosine.xyz, c.sun_direction.xyz),
        c.sky_conditions.x, c.sky_conditions.y, c.sky_conditions.z), 1.0);
    let exposure = water_exposure(c.water_metadata.x);
    let sheltered = water_surface(c.position_time.xyz, c.normal_footprint.xyz,
        c.climate.x * exposure.x, c.climate.y * exposure.y,
        c.position_time.w, c.normal_footprint.w);
    results[id.x].sheltered = vec4<f32>(sheltered.normal, sheltered.roughness);
    results[id.x].scattering = vec4<f32>(water_crest_scattering(c.water_body.xyz,
        c.crest_parameters.x, c.crest_parameters.y, c.crest_parameters.z,
        c.sunlight.xyz), 0.0);
    let packed = PackedVsIn(c.packed_vertex.x, c.packed_vertex.y,
        vec4<u32>(c.packed_vertex.z & 255u, (c.packed_vertex.z >> 8u) & 255u,
            (c.packed_vertex.z >> 16u) & 255u, c.packed_vertex.z >> 24u));
    let rgba = unpack_packed_rgba(packed);
    let payload = unpack_packed_face(packed);
    results[id.x].decoded_position = vec4<f32>(unpack_packed_position(packed), 0.0);
    results[id.x].decoded_payload = vec4<u32>(payload,
        rgba.r | (rgba.g << 8u) | (rgba.b << 16u) | (rgba.a << 24u),
        select(0u, 1u, packed_surface_normal(packed)), (payload >> 13u) & 3u);
    results[id.x].bed = vec4<f32>(water_bed_color(c.water_body.xyz,
        c.position_time.xyz, c.normal_footprint.w, c.water_metadata.x), 0.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    position_time: [f32; 4],
    normal_footprint: [f32; 4],
    climate: [f32; 4],
    sun_direction: [f32; 4],
    moon_illumination: [f32; 4],
    reflected_cosine: [f32; 4],
    sky_conditions: [f32; 4],
    crest_parameters: [f32; 4],
    water_body: [f32; 4],
    sunlight: [f32; 4],
    water_metadata: [u32; 4],
    packed_vertex: [u32; 4],
}

impl Default for Case {
    fn default() -> Self {
        Self {
            position_time: [12.25, -71.375, 1024.03125, 17.125],
            normal_footprint: [0.0, 0.0, 1.0, 0.01],
            climate: [8.0, 0.4, 0.0, 0.0],
            sun_direction: [0.5, 0.0, 0.866_025_4, 1.0],
            moon_illumination: [0.0, 0.0, 1.0, 0.0],
            reflected_cosine: [0.0, 0.0, 1.0, 0.5],
            sky_conditions: [0.0, 0.08, 0.5, 0.0],
            crest_parameters: [0.9, 0.2, 1.0, 0.0],
            water_body: [0.025, 0.16, 0.13, 0.0],
            sunlight: [1.0, 0.85, 0.65, 0.0],
            water_metadata: [1, 0, 0, 0],
            packed_vertex: [0, 58 << 16, 0xff_12_34_56, 0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Result {
    normal_roughness: [f32; 4],
    response: [f32; 4],
    environment: [f32; 4],
    clear: [f32; 4],
    variance_response: [f32; 4],
    palette: [f32; 4],
    sheltered: [f32; 4],
    scattering: [f32; 4],
    decoded_position: [f32; 4],
    decoded_payload: [u32; 4],
    bed: [f32; 4],
    azulejo: [f32; 4],
}

#[test]
fn water_helpers_validate_without_a_graphics_adapter() {
    let source = shader_source();
    let module = naga::front::wgsl::parse_str(&source).expect("parse water shader");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validate water shader");
}

#[test]
fn gpu_seabed_color_is_stationary_periodic_depth_limited_and_filtered() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let state = sky(60.0_f32.to_radians(), 2.0);
    let base = Case {
        water_body: [0.15, 0.42, 0.35, 0.0],
        ..Case::default()
    };
    let mut cases = vec![base];
    for axis in 0..3 {
        for offset in [-8192.0, 8192.0] {
            let mut sample = base;
            sample.position_time[axis] += offset;
            cases.push(sample);
        }
    }
    let mut later = base;
    later.position_time[3] = 49_001.0;
    cases.push(later);
    let matching = cases.len();
    for kind in [0, 2, 3] {
        let mut sample = base;
        sample.water_metadata[0] = kind;
        cases.push(sample);
    }
    let mut far = base;
    far.normal_footprint[3] = 32.0;
    cases.push(far);
    let mut dark = base;
    dark.water_body = [0.004, 0.012, 0.04, 0.0];
    cases.push(dark);
    let pattern_start = cases.len();
    for i in 0..256 {
        let mut sample = base;
        sample.position_time[0] = i as f32 * 4.0 + 0.25;
        cases.push(sample);
    }
    let results = fixture.run(&state, &cases);
    for result in &results[1..matching] {
        assert!((rgb(result.bed) - rgb(results[0].bed)).length() < 1e-5);
    }
    for (case, result) in cases[matching..pattern_start]
        .iter()
        .zip(&results[matching..pattern_start])
    {
        assert_eq!(rgb(result.bed), rgb(case.water_body));
    }
    let mut minimum = f32::INFINITY;
    let mut maximum = f32::NEG_INFINITY;
    for result in &results[pattern_start..] {
        let color = rgb(result.bed);
        assert!(color.is_finite());
        assert!(color.cmpge(Vec3::ZERO).all() && color.cmple(rgb(base.water_body)).all());
        minimum = minimum.min(color.y);
        maximum = maximum.max(color.y);
    }
    assert!(
        maximum - minimum > 0.18,
        "submerged rocks must read against sand"
    );
    // Crossing the wrap itself must be continuous, not just repeat after it.
    for axis in 0..3 {
        let mut left = base;
        let mut right = base;
        left.position_time[axis] = -0.001;
        right.position_time[axis] = 0.001;
        let result = fixture.run(&state, &[left, right]);
        assert!((rgb(result[0].bed) - rgb(result[1].bed)).length() < 0.004);
    }
}

struct Fixture {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}

impl Fixture {
    fn new() -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("water numerical GPU test skipped: no adapter ({error})");
                return None;
            }
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("water numerical fixture"),
            ..Default::default()
        }))
        .expect("water test device");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production water helpers"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader_source())),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("water numerical fixture"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Some(Self {
            device,
            queue,
            pipeline,
        })
    }

    fn run(&self, state: &SkyState, cases: &[Case]) -> Vec<Result> {
        let (parameters, radiances) = state.raw();
        let mut model = [0.0_f32; 32];
        model[..27].copy_from_slice(&parameters);
        model[27..30].copy_from_slice(&radiances);
        let buffer = |bytes: &[u8], usage| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytes,
                    usage,
                })
        };
        let sky = buffer(bytemuck::cast_slice(&model), wgpu::BufferUsages::UNIFORM);
        let inputs = buffer(bytemuck::cast_slice(cases), wgpu::BufferUsages::STORAGE);
        let bytes = (cases.len() * size_of::<Result>()) as u64;
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sky_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: sky.as_entire_binding(),
            }],
        });
        let case_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(1),
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
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &sky_group, &[]);
            pass.set_bind_group(1, &case_group, &[]);
            pass.dispatch_workgroups(cases.len().div_ceil(64) as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .expect("water readback");
        rx.recv().unwrap().unwrap();
        let results = bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
        readback.unmap();
        results
    }
}

fn sky(elevation: f32, turbidity: f32) -> SkyState {
    SkyState::new(&SkyParams {
        elevation: elevation.max(0.0),
        turbidity,
        albedo: [0.12, 0.13, 0.15],
    })
    .unwrap()
}

fn rgb(value: [f32; 4]) -> Vec3 {
    Vec3::new(value[0], value[1], value[2])
}

fn luminance(value: [f32; 4]) -> f32 {
    rgb(value).dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

#[test]
fn sunset_palette_preserves_day_zenith_night_and_orbit() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let elevation = 1.0_f32.to_radians();
    let base = Case {
        sun_direction: [elevation.cos(), 0.0, elevation.sin(), 1.0],
        reflected_cosine: [elevation.cos(), 0.0, elevation.sin(), 0.5],
        sky_conditions: [0.0, 0.025, 0.45, 0.0],
        ..Case::default()
    };
    let cases = [
        base,
        Case {
            sky_conditions: [0.0, 0.20, 0.90, 0.0],
            ..base
        },
        Case {
            sun_direction: [0.0, 0.0, 1.0, 1.0],
            ..base
        },
        Case {
            sun_direction: [0.0, 0.0, -1.0, 1.0],
            ..base
        },
        Case {
            reflected_cosine: [0.0, 0.0, 1.0, 0.5],
            ..base
        },
        Case {
            reflected_cosine: [-elevation.cos(), 0.0, elevation.sin(), 0.5],
            ..base
        },
        Case {
            sky_conditions: [24_000.0, 0.20, 0.90, 0.0],
            ..base
        },
    ];
    let results = fixture.run(&sky(elevation, 2.0), &cases);
    for result in &results {
        let grade = rgb(result.palette);
        assert!(grade.is_finite() && grade.min_element() >= 0.35 && grade.max_element() <= 1.101);
    }
    assert!(results[1].palette[0] > results[0].palette[0]);
    assert!(results[1].palette[2] < results[0].palette[2]);
    for result in &results[2..] {
        assert_eq!(rgb(result.palette), Vec3::ONE);
    }
}

#[test]
fn gpu_water_preserves_optics_periodicity_and_minified_energy() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let state = sky(60.0_f32.to_radians(), 2.0);
    let base = Case::default();
    let mut cases = vec![base];
    for axis in 0..3 {
        for offset in [-8192.0, 8192.0] {
            let mut sample = base;
            sample.position_time[axis] += offset;
            cases.push(sample);
        }
    }
    let mut clock_wrap = base;
    clock_wrap.position_time[3] += 65536.0;
    cases.push(clock_wrap);
    let mut moved = base;
    moved.position_time[3] += 0.137;
    cases.push(moved);
    let values = fixture.run(&state, &cases);
    let first = rgb(values[0].normal_roughness);
    for result in &values[1..8] {
        assert!(first.distance(rgb(result.normal_roughness)) < 0.00001);
    }
    assert!(first.distance(rgb(values[8].normal_roughness)) > 0.002);

    // The world spans all tangent-frame orientations. A carrier approaching
    // the surface normal must lose its slope continuously, without a pole.
    let carrier_up = Vec3::new(3.0, 1.0, 2.0).normalize();
    let orientations = [
        Vec3::X,
        Vec3::Y,
        Vec3::Z,
        -Vec3::X,
        -Vec3::Y,
        -Vec3::Z,
        (carrier_up + Vec3::X * 0.00001).normalize(),
        (carrier_up - Vec3::X * 0.00001).normalize(),
    ];
    let orientation_cases: Vec<_> = orientations
        .iter()
        .map(|normal| Case {
            normal_footprint: [normal.x, normal.y, normal.z, 0.01],
            climate: [22.0, 1.0, 0.0, 0.0],
            ..base
        })
        .collect();
    let oriented = fixture.run(&state, &orientation_cases);
    for (normal, result) in orientations.into_iter().zip(&oriented) {
        let surface_normal = rgb(result.normal_roughness);
        assert!(surface_normal.is_finite());
        assert!((surface_normal.length() - 1.0).abs() < 0.00001);
        assert!(surface_normal.dot(normal) > 0.90);
    }
    assert!(rgb(oriented[6].normal_roughness).distance(rgb(oriented[7].normal_roughness)) < 0.001);

    let fresnel_cases: Vec<_> = (0..=100)
        .map(|i| Case {
            reflected_cosine: [0.0, 0.0, 1.0, i as f32 / 100.0],
            ..base
        })
        .collect();
    let fresnel = fixture.run(&state, &fresnel_cases);
    assert!((fresnel[0].response[1] - 1.0).abs() < 0.00001);
    let ior = 1.333_f32;
    let normal_reflection = ((ior - 1.0) / (ior + 1.0)).powi(2);
    assert!((fresnel[100].response[1] - normal_reflection).abs() < 0.00001);
    assert!(
        fresnel
            .windows(2)
            .all(|pair| pair[0].response[1] >= pair[1].response[1])
    );

    let mut variances = Vec::new();
    for footprint in [0.0, 0.15, 0.4, 1.0, 8.0] {
        let samples: Vec<_> = (0..256)
            .map(|i| Case {
                position_time: [i as f32 * 0.731, (i * 73 % 257) as f32 * 0.417, 0.0, 17.125],
                normal_footprint: [0.0, 0.0, 1.0, footprint],
                ..base
            })
            .collect();
        let results = fixture.run(&state, &samples);
        let variance = results
            .iter()
            .map(|r| rgb(r.normal_roughness).distance_squared(Vec3::Z))
            .sum::<f32>()
            / results.len() as f32;
        variances.push(variance);
        for result in results {
            let normal = rgb(result.normal_roughness);
            assert!(normal.is_finite() && (normal.length() - 1.0).abs() < 0.00001);
            assert!(normal.dot(Vec3::Z) > 0.90);
            assert!((0.095..=1.0).contains(&result.normal_roughness[3]));
            assert!((0.0..=1.0).contains(&result.response[0]));
            if footprint == 8.0 {
                assert!(normal.distance(Vec3::Z) < 0.00001);
                assert!(result.normal_roughness[3] > values[0].normal_roughness[3]);
            }
        }
    }
    assert!(variances[0] > 0.0001);
    assert!(
        variances
            .windows(2)
            .all(|pair| pair[1] <= pair[0] * 1.03 + 1e-8),
        "{variances:?}"
    );
    println!("water normal variance by pixel footprint: {variances:?}");

    let energy = |wind: f32, sea: f32| {
        let cases: Vec<_> = (0..128)
            .map(|i| Case {
                position_time: [i as f32 * 0.731, (i * 73 % 257) as f32 * 0.417, 0.0, 17.125],
                climate: [wind, sea, 0.0, 0.0],
                ..base
            })
            .collect();
        let results = fixture.run(&state, &cases);
        results
            .iter()
            .map(|r| rgb(r.normal_roughness).distance_squared(Vec3::Z))
            .sum::<f32>()
            / results.len() as f32
    };
    assert!(energy(22.0, 1.0) > energy(0.0, 0.0) * 8.0);
}

#[test]
fn gpu_water_roughness_preserves_slope_variance_before_carrier_scaling() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let state = sky(60.0_f32.to_radians(), 2.0);
    let probe = fixture.run(&state, &[Case::default()])[0];
    let base_energy = 0.12_f32.powi(4);
    for (roughness, variance) in probe
        .variance_response
        .into_iter()
        .zip([0.0, 0.001, 0.02, 0.08])
    {
        let recovered = roughness.powi(4) - base_energy;
        assert!((recovered - variance).abs() < 1e-7);
        // A large unresolved carrier can become subtle after amplitude scaling.
        // Clamping before that scale irreversibly discards highlight energy.
        let attenuated_energy = base_energy + recovered * 0.2_f32.powi(2);
        let expected = base_energy + variance * 0.2_f32.powi(2);
        assert!((attenuated_energy - expected).abs() < 1e-8);
    }
    assert!(probe.variance_response[3] > 0.50);

    let footprints = [0.0, 0.15, 0.4, 1.0, 8.0];
    for (wind, sea) in [(8.0_f32, 0.4_f32), (22.0, 1.0)] {
        let base_roughness = 0.095 + 0.075 * wind / 22.0 + 0.10 * sea;
        let base_energy = base_roughness.powi(4);
        let mut random_state = 719_u32;
        let mut random = || {
            random_state = random_state
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            (random_state >> 8) as f32 / 16_777_216.0
        };
        let mut cases = Vec::new();
        for _ in 0..1024 {
            let position_time = [
                (random() - 0.5) * 128.0,
                (random() - 0.5) * 128.0,
                0.0,
                random() * 128.0,
            ];
            for footprint in footprints {
                cases.push(Case {
                    position_time,
                    normal_footprint: [0.0, 0.0, 1.0, footprint],
                    climate: [wind, sea, 0.0, 0.0],
                    ..Case::default()
                });
            }
        }
        let results = fixture.run(&state, &cases);
        let mut total_energy = [0.0; 5];
        for sample in results.chunks_exact(footprints.len()) {
            for (index, result) in sample.iter().enumerate() {
                let lost_variance = result.normal_roughness[3].powi(4) - base_energy;
                assert!(
                    (lost_variance - result.response[3]).abs() < 1e-7,
                    "perceptual roughness must preserve the field's mean-square slope variance"
                );
                let normal = rgb(result.normal_roughness);
                let resolved_gradient = Vec3::Z - normal / normal.z;
                total_energy[index] += resolved_gradient.length_squared() + lost_variance;
            }
        }
        for value in &mut total_energy {
            *value /= 1024.0;
        }
        // Independent observed slope samples include carrier cross terms;
        // their finite-sample mean should agree with the filtered variance.
        for energy in total_energy {
            assert!((energy / total_energy[0] - 1.0).abs() < 0.12);
        }
        println!("water total slope energy wind={wind} sea={sea}: {total_energy:?}");
    }
}

#[test]
fn gpu_water_displacement_derivative_and_long_range_irregularity_agree() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let state = sky(60.0_f32.to_radians(), 2.0);
    let mut derivatives = Vec::new();
    let step = 0.002;
    for index in 0..32 {
        let base = Case {
            position_time: [
                index as f32 * 0.371,
                (index * 17 % 31) as f32 * 0.219,
                0.0,
                0.375,
            ],
            normal_footprint: [0.0, 0.0, 1.0, 0.0],
            ..Case::default()
        };
        derivatives.push(base);
        for axis in 0..2 {
            for offset in [-step, step] {
                let mut shifted = base;
                shifted.position_time[axis] += offset;
                derivatives.push(shifted);
            }
        }
    }
    let values = fixture.run(&state, &derivatives);
    for sample in values.chunks_exact(5) {
        let normal = rgb(sample[0].normal_roughness);
        let gradient = Vec3::Z - normal / normal.z;
        for axis in 0..2 {
            let finite_difference = (sample[axis * 2 + 2].response[2]
                - sample[axis * 2 + 1].response[2])
                / (step * 2.0);
            assert!(
                (gradient[axis] - finite_difference).abs() < 0.00025,
                "height derivative {finite_difference} differs from normal gradient {}",
                gradient[axis]
            );
        }
    }

    // The original four planar waves repeated exactly after 16 m translations.
    // Compare actual normal fields across patches, rather than checking that
    // a particular noise function or constant exists in the implementation.
    let mut patches = Vec::new();
    for index in 0..512 {
        let base = Case {
            position_time: [
                index as f32 * 0.193,
                (index * 73 % 509) as f32 * 0.137,
                0.0,
                7.125,
            ],
            normal_footprint: [0.0, 0.0, 1.0, 0.0],
            ..Case::default()
        };
        patches.push(base);
        for axis in 0..2 {
            let mut shifted = base;
            shifted.position_time[axis] += 16.0;
            patches.push(shifted);
        }
    }
    let fields = fixture.run(&state, &patches);
    for shift in [1, 2] {
        let mut product = 0.0;
        let mut energy_a = 0.0;
        let mut energy_b = 0.0;
        for patch in fields.chunks_exact(3) {
            let a = rgb(patch[0].normal_roughness) - Vec3::Z;
            let b = rgb(patch[shift].normal_roughness) - Vec3::Z;
            product += a.dot(b);
            energy_a += a.length_squared();
            energy_b += b.length_squared();
        }
        let correlation = product / (energy_a * energy_b).sqrt();
        eprintln!(
            "water normal correlation after 16 m on axis {}: {correlation}",
            shift - 1
        );
        assert!(
            correlation.abs() < 0.85,
            "water retains a repeated straight-wave lattice"
        );
    }
}

#[test]
fn gpu_water_sky_matches_hosek_and_tracks_sunset_storm_and_night() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    for elevation in [20.0_f32.to_radians(), 60.0_f32.to_radians()] {
        for turbidity in [2.0, 8.0] {
            let state = sky(elevation, turbidity);
            let sun = Vec3::new(elevation.cos(), 0.0, elevation.sin());
            let cases: Vec<_> = (0..64)
                .map(|i| {
                    let cosine = 0.01 + 0.98 * (i / 8) as f32 / 7.0;
                    let phi = (i % 8) as f32 * std::f32::consts::TAU / 8.0;
                    let radius = (1.0 - cosine * cosine).sqrt();
                    Case {
                        sun_direction: [sun.x, sun.y, sun.z, 1.0],
                        reflected_cosine: [radius * phi.cos(), radius * phi.sin(), cosine, 0.5],
                        ..Default::default()
                    }
                })
                .collect();
            let results = fixture.run(&state, &cases);
            for (case, result) in cases.iter().zip(results) {
                let direction = rgb(case.reflected_cosine);
                let theta = direction.z.acos();
                let gamma = direction.dot(sun).clamp(-1.0, 1.0).acos();
                for (index, channel) in [Channel::R, Channel::G, Channel::B].into_iter().enumerate()
                {
                    let expected = state.radiance(theta, gamma, channel).max(0.0) * 0.035;
                    assert!(
                        (result.clear[index] - expected).abs() < expected * 0.003 + 0.00001,
                        "elevation={elevation} turbidity={turbidity} channel={index}: {} != {expected}",
                        result.clear[index]
                    );
                }
            }
        }
    }
    let base = Case {
        reflected_cosine: [0.998_749_2, 0.0, 0.05, 0.3],
        ..Default::default()
    };
    let noon = fixture.run(&sky(60.0_f32.to_radians(), 2.0), &[base])[0].environment;
    let elevation = 2.0_f32.to_radians();
    let dusk_case = Case {
        sun_direction: [elevation.cos(), 0.0, elevation.sin(), 1.0],
        ..base
    };
    let dusk = fixture.run(&sky(elevation, 3.0), &[dusk_case])[0].environment;
    assert!(dusk[0] / dusk[2] > noon[0] / noon[2] * 1.2);
    let storm_case = Case {
        climate: [22.0, 1.0, 1.0, 12.0],
        ..base
    };
    let storm = fixture.run(&sky(60.0_f32.to_radians(), 2.0), &[storm_case])[0].environment;
    assert!(luminance(storm) < luminance(noon) * 0.75);
    assert!(rgb(storm).max_element() - rgb(storm).min_element() < 0.00001);
    let dark = Case {
        sun_direction: [0.0, 0.0, -1.0, 1.0],
        ..base
    };
    let moonlit = Case {
        moon_illumination: [0.0, 0.0, 1.0, 1.0],
        ..dark
    };
    let nights = fixture.run(&sky(0.0, 2.0), &[dark, moonlit]);
    assert!(luminance(nights[0].environment) < luminance(noon) * 0.01);
    assert!(luminance(nights[1].environment) > luminance(nights[0].environment) * 2.0);
    for color in [
        noon,
        dusk,
        storm,
        nights[0].environment,
        nights[1].environment,
    ] {
        assert!(rgb(color).is_finite() && rgb(color).min_element() >= 0.0);
    }
    println!(
        "water HDR environment: noon={noon:?} dusk={dusk:?} storm={storm:?} night={:?}",
        nights[0].environment
    );
}

#[test]
fn gpu_water_context_shelters_inland_surfaces_without_changing_legacy_ocean() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let mut cases = Vec::new();
    for normal in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::Z] {
        for i in 0..64 {
            for kind in 0..4 {
                cases.push(Case {
                    position_time: [i as f32 * 0.731, (i * 73 % 257) as f32 * 0.417, 0.0, 17.125],
                    normal_footprint: [normal.x, normal.y, normal.z, 0.01],
                    climate: [20.0, 0.8, 0.0, 0.0],
                    water_metadata: [kind, 0, 0, 0],
                    ..Default::default()
                });
            }
        }
    }
    let results = fixture.run(&sky(1.0, 2.0), &cases);
    let mut slope_energy = [0.0_f32; 4];
    for (samples, inputs) in results.chunks_exact(4).zip(cases.chunks_exact(4)) {
        for equivalent in [samples[1].sheltered, samples[0].normal_roughness] {
            assert!(
                samples[0]
                    .sheltered
                    .into_iter()
                    .zip(equivalent)
                    .all(|(a, b)| (a - b).abs() <= 0.000001),
                "legacy/ocean mismatch: {:?} != {equivalent:?}",
                samples[0].sheltered,
            );
        }
        assert!(samples[1].sheltered[3] > samples[2].sheltered[3]);
        assert!(samples[2].sheltered[3] > samples[3].sheltered[3]);
        let up = rgb(inputs[0].normal_footprint);
        for (kind, sample) in samples.iter().enumerate() {
            let normal = rgb(sample.sheltered);
            assert!(normal.is_finite() && (normal.length() - 1.0).abs() < 0.00001);
            assert!(normal.dot(up) > 0.90);
            assert!(sample.sheltered[3].is_finite());
            slope_energy[kind] += normal.distance_squared(up);
        }
    }
    assert!(slope_energy[1] > slope_energy[2] * 2.0, "{slope_energy:?}");
    assert!(slope_energy[2] > slope_energy[3] * 1.05, "{slope_energy:?}");
}

#[test]
fn gpu_crest_scattering_is_bounded_directional_and_dark_without_sunlight() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let state = sky(1.0, 2.0);
    let base = Case::default();
    let controls = [
        base,
        Case {
            sunlight: [0.0; 4],
            ..base
        },
        Case {
            crest_parameters: [0.0, 0.2, 1.0, 0.0],
            ..base
        },
        Case {
            crest_parameters: [0.9, 0.2, -1.0, 0.0],
            ..base
        },
        Case {
            crest_parameters: [0.9, 1.0, 1.0, 0.0],
            ..base
        },
    ];
    let values = fixture.run(&state, &controls);
    assert!(luminance(values[0].scattering) > 0.01);
    assert_eq!(rgb(values[1].scattering), Vec3::ZERO);
    assert_eq!(rgb(values[2].scattering), Vec3::ZERO);
    assert!(luminance(values[0].scattering) > luminance(values[3].scattering) * 5.0);
    assert!(luminance(values[0].scattering) > luminance(values[4].scattering) * 1.5);
    let mut cases = Vec::new();
    for body in [[0.0; 4], [0.02, 0.18, 0.12, 0.0], [1.0; 4]] {
        for crest in [0.0, 0.3, 0.6, 1.0] {
            for view in [-1.0, 0.0, 0.5, 1.0] {
                for sun_view in [-1.0, 0.0, 0.5, 1.0] {
                    for sunlight in [[0.0; 4], [0.01, 0.02, 0.03, 0.0], [4.0, 2.0, 1.0, 0.0]] {
                        cases.push(Case {
                            crest_parameters: [crest, view, sun_view, 0.0],
                            water_body: body,
                            sunlight,
                            ..base
                        });
                    }
                }
            }
        }
    }
    for (value, case) in fixture.run(&state, &cases).iter().zip(&cases) {
        let color = rgb(value.scattering);
        assert!(color.is_finite() && color.min_element() >= 0.0);
        assert!(color.cmple(rgb(case.sunlight) * 0.75).all());
    }
}

#[test]
fn gpu_packed_water_decoder_preserves_kind_color_opaque_alpha_and_position() {
    use vearth_core::mesh::{PackedSurfaceVertex, SurfaceVertex};
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let mut cases = Vec::new();
    let mut originals = Vec::new();
    for kind in 0..4 {
        for pos in [
            [-20.5, 13.25, 31.996_094],
            [-128.0, 0.0, 127.996_09],
            [127.996_09, -128.0, 0.0],
        ] {
            let vertex = SurfaceVertex {
                pos,
                face_ao: 2 | (3 << 3) | (1 << 5) | (kind << 13),
                rgba: [23, 89, 117, 255],
            };
            let packed = PackedSurfaceVertex::try_from_full(vertex).expect("water payload fits");
            assert_eq!(packed.to_full(), vertex);
            cases.push(Case {
                packed_vertex: [
                    packed.pos_xy,
                    packed.pos_z_face,
                    u32::from_le_bytes(packed.rgba),
                    0,
                ],
                ..Default::default()
            });
            originals.push(vertex);
        }
    }
    for (sample, vertex) in fixture.run(&sky(1.0, 2.0), &cases).iter().zip(&originals) {
        assert_eq!(rgb(sample.decoded_position).to_array(), vertex.pos);
        assert_eq!(sample.decoded_payload[0], vertex.face_ao);
        assert_eq!(sample.decoded_payload[1], u32::from_le_bytes(vertex.rgba));
        assert_eq!(
            sample.decoded_payload[2], 0,
            "typed water retains ordinary packing"
        );
        assert_eq!(sample.decoded_payload[3], (vertex.face_ao >> 13) & 3);
        assert_eq!(sample.decoded_payload[1] >> 24, 255);
    }
}

#[test]
fn gpu_packed_construction_decoder_preserves_finish_and_explicit_normal() {
    use vearth_core::mesh::{PackedSurfaceVertex, SurfaceVertex, encode_surface_normal};
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let mut cases = Vec::new();
    let mut originals = Vec::new();
    for material in [12, 82, 84, 85, 87, 88, 90, 93, 210] {
        for variant in 0..4 {
            let vertex = SurfaceVertex {
                pos: [-127.0, 4.0, 127.984375],
                face_ao: encode_surface_normal(
                    2 | (3 << 3) | (material << 5) | (variant << 13),
                    [-0.31, 0.51, -0.79],
                ),
                rgba: [31, 107, 201, 255],
            };
            let packed =
                PackedSurfaceVertex::try_from_full(vertex).expect("finish must remain packed");
            cases.push(Case {
                packed_vertex: [
                    packed.pos_xy,
                    packed.pos_z_face,
                    u32::from_le_bytes(packed.rgba),
                    0,
                ],
                ..Default::default()
            });
            originals.push(vertex);
        }
    }
    for (sample, original) in fixture.run(&sky(1.0, 2.0), &cases).iter().zip(originals) {
        assert_eq!(rgb(sample.decoded_position).to_array(), original.pos);
        assert_eq!(sample.decoded_payload[0], original.face_ao);
        assert_eq!(sample.decoded_payload[1], u32::from_le_bytes(original.rgba));
        assert_eq!(sample.decoded_payload[2], 1);
        assert_eq!(sample.decoded_payload[3], (original.face_ao >> 13) & 3);
    }
}

#[test]
fn gpu_glazed_pattern_preserves_ink_ground_and_filtered_mean() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let ground = Vec3::new(0.806_952_24, 0.822_785_74, 0.752_942_2);
    for ink in [
        Vec3::new(0.014_443_844, 0.076_185_38, 0.300_543_79),
        Vec3::new(0.1, 0.3, 0.15),
    ] {
        let cases: Vec<_> = [0.0, 0.2, 0.5, 0.8, 1.0]
            .into_iter()
            .map(|coverage| Case {
                water_body: [ink.x, ink.y, ink.z, coverage],
                ..Default::default()
            })
            .collect();
        let results = fixture.run(&sky(1.0, 2.0), &cases);
        for (result, case) in results.iter().zip(&cases) {
            let expected = ground.lerp(ink, case.water_body[3]);
            assert!((rgb(result.azulejo) - expected).abs().max_element() < 0.000_002);
        }
        let pixel_mean = (rgb(results[1].azulejo) + rgb(results[3].azulejo)) * 0.5;
        assert!((pixel_mean - rgb(results[2].azulejo)).abs().max_element() < 0.000_002);
    }
}
