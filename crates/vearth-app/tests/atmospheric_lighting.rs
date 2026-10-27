//! Numerical checks of the production atmospheric helpers. The reference
//! integrators use dense uniform samples rather than the shader quadratures.

use std::{borrow::Cow, time::Duration};
use wgpu::util::DeviceExt;

const AERIAL: &str = include_str!("../src/aerial_perspective.wgsl");
const ATMOSPHERE: &str = concat!(
    include_str!("../src/aerial_perspective.wgsl"),
    "\n",
    include_str!("../src/cloud_common.wgsl"),
    "\n",
    include_str!("../src/solar_optics.wgsl"),
    "\n",
    include_str!("../src/atmosphere.wgsl")
);
const RADIUS: f64 = 6_378_137.0;

fn solar_source() -> &'static str {
    let start = ATMOSPHERE.find("fn solar_density(").unwrap();
    let end = ATMOSPHERE.find("fn rayleigh_phase(").unwrap();
    &ATMOSPHERE[start..end]
}

const HARNESS: &str = r#"
struct Globals {
    planet_center: vec4<f32>, sun_dir: vec4<f32>, weather0: vec4<f32>,
    weather3: vec4<f32>, weather5: vec4<f32>, surface: vec4<f32>, weather2: vec4<f32>, wind: vec4<f32>,
    aerial_sunward: vec4<f32>, aerial_side: vec4<f32>, aerial_away: vec4<f32>,
};
var<private> g: Globals;
struct Case {
    globals: Globals, position_grid: vec4<f32>, color: vec4<f32>,
    layer: vec4<f32>, solar: vec4<f32>, aerosol: vec4<f32>, celestial: vec4<f32>, effects: vec4<f32>,
};
struct Result {
    layer_depth: vec4<f32>, radiance: vec4<f32>, applied: vec4<f32>, transmission: vec4<f32>,
    celestial: vec4<f32>, optimized_effects: vec4<f32>, reference_effects: vec4<f32>,
};
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<Result>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&cases) { return; }
    let c = cases[id.x];
    g = c.globals;
    results[id.x].layer_depth = vec4<f32>(
        aerial_layer_column(c.layer.x, c.layer.y, c.layer.z, c.layer.w),
        aerial_optical_depth(c.position_grid.xyz), 0.0, 0.0);
    results[id.x].radiance = vec4<f32>(aerial_horizon_radiance(
        normalize(c.position_grid.xyz + vec3<f32>(0.0, 0.000001, 0.0))), 1.0);
    results[id.x].applied = vec4<f32>(apply_aerial_perspective(
        c.color.rgb, c.position_grid.xyz, c.position_grid.w), 1.0);
    results[id.x].transmission = vec4<f32>(solar_disc_transmittance(
        c.solar.x, c.solar.y, c.solar.z, c.aerosol.rgb), 1.0);
    results[id.x].celestial = vec4<f32>(
        celestial_size_scale(c.celestial.x, c.celestial.y, c.celestial.z),
        lunar_surface_albedo(c.color.xy, c.celestial.w), 0.0, 0.0);
}
@compute @workgroup_size(64)
fn effects_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&cases) { return; }
    let c = cases[id.x];
    g = c.globals;
    let rd = normalize(c.position_grid.xyz);
    let sun = normalize(g.sun_dir.xyz);
    let up = vec3<f32>(sqrt(1.0 - c.effects.z * c.effects.z), 0.0, c.effects.z);
    results[id.x].optimized_effects = vec4<f32>(transient_sky_effects(
        c.color.rgb, rd, sun, up, c.effects.x, c.effects.y), 1.0);
    results[id.x].reference_effects = vec4<f32>(reference_effects(
        c.color.rgb, rd, sun, up, c.effects.x, c.effects.y), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    planet_center: [f32; 4],
    sun_dir: [f32; 4],
    weather0: [f32; 4],
    weather3: [f32; 4],
    weather5: [f32; 4],
    surface: [f32; 4],
    weather2: [f32; 4],
    wind: [f32; 4],
    aerial_sunward: [f32; 4],
    aerial_side: [f32; 4],
    aerial_away: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    globals: Globals,
    position_grid: [f32; 4],
    color: [f32; 4],
    layer: [f32; 4],
    solar: [f32; 4],
    aerosol: [f32; 4],
    celestial: [f32; 4],
    effects: [f32; 4],
}

impl Default for Case {
    fn default() -> Self {
        Self {
            globals: Globals {
                planet_center: [-(RADIUS as f32), 0.0, 0.0, RADIUS as f32],
                sun_dir: [0.5, 0.866_025_4, 0.0, 1.0],
                weather0: [20.0, 0.0, 50_000.0, 0.0],
                weather3: [1500.0, 6000.0, 0.0, 0.0],
                weather5: [0.0; 4],
                surface: [0.0; 4],
                weather2: [1200.0, 0.25, 0.0, 0.0],
                wind: [3.0, 2.0, 0.0, 0.0],
                aerial_sunward: [0.85, 1.07, 1.08, 0.0],
                aerial_side: [0.36, 0.53, 0.62, 0.0],
                aerial_away: [0.55, 0.79, 0.87, 0.0],
            },
            position_grid: [0.0, 1000.0, 0.0, 1.0],
            color: [0.2, 0.3, 0.1, 1.0],
            layer: [0.0, 0.0, 1000.0, 1800.0],
            solar: [0.0, 1.0, RADIUS as f32, 0.0],
            aerosol: [0.064, 0.08, 0.1, 0.0],
            celestial: [0.0, 0.0, RADIUS as f32, 0.0],
            effects: [0.5, 1.0, 0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Result {
    layer_depth: [f32; 4],
    radiance: [f32; 4],
    applied: [f32; 4],
    transmission: [f32; 4],
    celestial: [f32; 4],
    optimized_effects: [f32; 4],
    reference_effects: [f32; 4],
}

fn source() -> String {
    let start = ATMOSPHERE.find("fn cloud_hash(").unwrap();
    let end = ATMOSPHERE.find("fn climate_uv(").unwrap();
    format!(
        "{AERIAL}\n{}\n{}\n{REFERENCE_EFFECTS}\n{HARNESS}",
        solar_source(),
        &ATMOSPHERE[start..end]
    )
}

#[test]
fn production_atmospheric_helpers_validate_without_a_graphics_adapter() {
    let source = source();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validate atmospheric helper harness");
}

struct Fixture {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}

impl Fixture {
    fn new() -> Option<Self> {
        Self::with_entry_point("main")
    }

    fn with_entry_point(entry_point: &str) -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("atmospheric numerical GPU test skipped: no adapter ({error})");
                return None;
            }
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("atmospheric numerical fixture"),
            ..Default::default()
        }))
        .expect("atmosphere test device");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production atmospheric helpers"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(source())),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("atmospheric numerical fixture"),
            layout: None,
            module: &shader,
            entry_point: Some(entry_point),
            compilation_options: Default::default(),
            cache: None,
        });
        Some(Self {
            device,
            queue,
            pipeline,
        })
    }

    fn run(&self, cases: &[Case]) -> Vec<Result> {
        let inputs = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(cases),
                usage: wgpu::BufferUsages::STORAGE,
            });
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
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
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
            pass.set_bind_group(0, &group, &[]);
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
            .expect("atmosphere readback");
        rx.recv().unwrap().unwrap();
        let results = bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
        readback.unmap();
        results
    }
}

fn dense_solar_reference(case: &Case) -> [f64; 3] {
    let height = f64::from(case.solar[0]);
    let cosine = f64::from(case.solar[1]);
    let ground_radius = f64::from(case.solar[2]);
    let radius = ground_radius + height;
    // Cartesian geometry independently computes intersections and altitude.
    let direction = [cosine, (1.0 - cosine * cosine).max(0.0).sqrt()];
    let projection = radius * direction[0];
    let discriminant = projection * projection - radius * radius + ground_radius * ground_radius;
    if direction[0] < 0.0 && discriminant >= 0.0 {
        return [0.0; 3];
    }
    let outer = ground_radius + 180_000.0;
    let discriminant = projection * projection - radius * radius + outer * outer;
    if discriminant <= 0.0 {
        return [1.0; 3];
    }
    let entry = (-projection - discriminant.sqrt()).max(0.0);
    let exit = -projection + discriminant.sqrt();
    if exit <= entry {
        return [1.0; 3];
    }
    let step = (exit - entry) / 65_536.0;
    let mut column = [0.0; 3];
    for index in 0..65_536 {
        let t = entry + (index as f64 + 0.5) * step;
        let x = radius + direction[0] * t;
        let y = direction[1] * t;
        let h = (x.hypot(y) - ground_radius).max(0.0);
        column[0] += (-h / 8000.0).exp() * step;
        column[1] += (-h / 1200.0).exp() * step;
        column[2] += (1.0 - (h - 25_000.0).abs() / 15_000.0).clamp(0.0, 1.0) * step;
    }
    let rayleigh = [5.802e-6, 13.558e-6, 33.100e-6];
    let ozone = [0.006, 0.022, 0.004];
    std::array::from_fn(|channel| {
        (-rayleigh[channel] * column[0]
            - f64::from(case.aerosol[channel]) * column[1] / 1200.0
            - ozone[channel] * column[2] / 15_000.0)
            .exp()
    })
}

#[test]
fn dense_solar_reference_matches_vertical_columns_and_vacuum() {
    let case = Case::default();
    let result = dense_solar_reference(&case);
    for channel in 0..3 {
        let expected = (-[5.802e-6, 13.558e-6, 33.100e-6][channel] * 8000.0
            - f64::from(case.aerosol[channel])
            - [0.006, 0.022, 0.004][channel])
            .exp();
        assert!((result[channel] - expected).abs() < 1e-7);
    }
    let mut vacuum = case;
    vacuum.solar[0] = 400_000.0;
    assert_eq!(dense_solar_reference(&vacuum), [1.0; 3]);
    vacuum.solar[1] = -1.0;
    assert_eq!(dense_solar_reference(&vacuum), [0.0; 3]);
}

#[test]
fn celestial_horizon_size_is_bounded_smooth_and_returns_to_real_size() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let mut cases = Vec::new();
    for height in [0.0_f64, 2000.0, 8000.0, 44_000.0, 80_000.0, 400_000.0] {
        let horizon = -(height * (2.0 * RADIUS + height)).sqrt() / (RADIUS + height);
        for step in 0..=160 {
            let above_horizon = (step as f64 * 0.25).to_radians().sin();
            cases.push(Case {
                celestial: [
                    (horizon + above_horizon) as f32,
                    height as f32,
                    RADIUS as f32,
                    0.0,
                ],
                ..Case::default()
            });
        }
    }
    let results = fixture.run(&cases);
    for (band, samples) in results.chunks_exact(161).enumerate() {
        let first = samples[0].celestial[0];
        assert!((1.0..=4.0).contains(&first));
        if band < 3 {
            assert!((first - 4.0).abs() < 0.00001);
        }
        if band >= 4 {
            assert!(samples.iter().all(|sample| sample.celestial[0] == 1.0));
        }
        for pair in samples.windows(2) {
            let (previous, next) = (pair[0].celestial[0], pair[1].celestial[0]);
            assert!(next.is_finite() && (1.0..=4.0).contains(&next));
            assert!(next <= previous + 0.000001 && previous - next < 0.035);
        }
        let high_sky = samples.last().unwrap().celestial[0];
        assert!((1.0..=1.75).contains(&high_sky));
        if band < 3 {
            assert_eq!(high_sky, 1.75);
        }
    }
    assert!((results[3 * 161].celestial[0] - 2.5).abs() < 0.00001);
}

#[test]
fn lunar_markings_remain_bounded_and_fade_when_unresolved() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let mut cases = Vec::new();
    for pixel_width in [0.0, 0.24, 0.5] {
        for y in -10..=10 {
            for x in -10..=10 {
                if x * x + y * y > 100 {
                    continue;
                }
                cases.push(Case {
                    color: [x as f32 * 0.1, y as f32 * 0.1, 0.0, 1.0],
                    celestial: [0.0, 0.0, RADIUS as f32, pixel_width],
                    ..Case::default()
                });
            }
        }
    }
    let results = fixture.run(&cases);
    let per_band = results.len() / 3;
    let mut contrasts = Vec::new();
    for samples in results.chunks_exact(per_band) {
        let mut range = (f32::MAX, f32::MIN);
        for sample in samples {
            let albedo = sample.celestial[1];
            assert!(albedo.is_finite() && (0.53..=1.01).contains(&albedo));
            range.0 = range.0.min(albedo);
            range.1 = range.1.max(albedo);
        }
        contrasts.push(range.1 - range.0);
    }
    assert!(contrasts[0] > 0.35 && contrasts[1] < contrasts[0]);
    assert_eq!(contrasts[2], 0.0);
    assert!(
        results[2 * per_band..]
            .iter()
            .all(|sample| sample.celestial[1] == 0.82)
    );
}

#[test]
fn solar_disc_matches_dense_spherical_integration_from_ground_to_orbit() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let mut cases = Vec::new();
    for height in [0.0, 2000.0, 10_000.0, 80_000.0, 400_000.0, 35_786_000.0] {
        for cosine in [1.0, 0.5, 0.05, 0.0, -0.05, -1.0] {
            for aerosol in [[0.064, 0.08, 0.1, 0.0], [0.3, 0.4, 0.5, 0.0]] {
                cases.push(Case {
                    solar: [height, cosine, RADIUS as f32, 0.0],
                    aerosol,
                    ..Case::default()
                });
            }
        }
        if height > 15_000.0 {
            // A ray that reaches 15 km before leaving the far side of the air.
            let tangent = (RADIUS + 15_000.0) / (RADIUS + f64::from(height));
            cases.push(Case {
                solar: [
                    height,
                    -(1.0 - tangent * tangent).sqrt() as f32,
                    RADIUS as f32,
                    0.0,
                ],
                ..Case::default()
            });
        }
    }
    let results = fixture.run(&cases);
    let mut worst_error = 0.0_f64;
    for (case, result) in cases.iter().zip(results) {
        let expected = dense_solar_reference(case);
        for channel in 0..3 {
            let actual = f64::from(result.transmission[channel]);
            let error = (actual - expected[channel]).abs();
            worst_error = worst_error.max(error);
            assert!(actual.is_finite() && (0.0..=1.000_001).contains(&actual));
            assert!(
                error < 0.006,
                "solar {:?}: {actual} vs {}",
                case.solar,
                expected[channel]
            );
        }
    }
    println!("solar disc dense-reference worst absolute transmittance error: {worst_error:.6}");
    let mut dawn = Case::default();
    dawn.solar[1] = 0.03;
    let mut high = Case::default();
    high.solar[0] = 10_000.0;
    let mut vacuum = Case::default();
    vacuum.solar[0] = 400_000.0;
    let results = fixture.run(&[Case::default(), dawn, high, vacuum]);
    assert!(results[1].transmission[0] > results[1].transmission[2] * 5.0);
    for channel in 0..3 {
        assert!(results[2].transmission[channel] > results[0].transmission[channel]);
        assert_eq!(results[3].transmission[channel], 1.0);
    }
}

#[test]
fn aerial_columns_match_dense_integration_including_horizontal_and_orbital_views() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let mut cases = Vec::new();
    for (start, end) in [
        (0.0, 0.0),
        (1000.0, 1000.0),
        (0.0, 20_000.0),
        (20_000.0, 0.0),
        (10_000.0, 10_010.0),
        (800_000.0, 0.0),
    ] {
        for scale in [1800.0, 8000.0] {
            cases.push(Case {
                layer: [start, end, 100_000.0, scale],
                ..Case::default()
            });
        }
    }
    for (case, result) in cases.iter().zip(fixture.run(&cases)) {
        let [start, end, length, scale] = case.layer.map(f64::from);
        let expected: f64 = (0..65_536)
            .map(|index| {
                let t = (index as f64 + 0.5) / 65_536.0;
                (-(start + (end - start) * t) / scale).exp() * length / 65_536.0
            })
            .sum();
        let error = (f64::from(result.layer_depth[0]) - expected).abs() / expected.max(0.001);
        assert!(error < 0.0001, "layer {:?}: {error}", case.layer);
    }
}

#[test]
fn aerial_visibility_direction_night_and_coarse_limb_guards_are_bounded() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let mut cases = vec![Case::default(); 9];
    cases[1].globals.weather0[2] = 5000.0;
    cases[2].globals.planet_center[0] -= 10_000.0;
    cases[3].globals.sun_dir = [-0.4, 0.916_515_1, 0.0, 1.0];
    cases[4].position_grid = [0.0, 0.0, 1000.0, 1.0];
    cases[5].position_grid[1] = -1000.0;
    cases[6].position_grid[3] = 4000.0;
    cases[7].position_grid = [0.0; 4];
    cases[8].globals.weather0[3] = 24.0;
    cases[8].globals.weather5 = [1.0, 1.0, 1.0, 8.5];
    let result = fixture.run(&cases);
    assert!(result[1].layer_depth[1] > result[0].layer_depth[1] * 8.0);
    assert!(result[2].layer_depth[1] < result[0].layer_depth[1] * 0.15);
    assert!(result[8].layer_depth[1] > result[0].layer_depth[1]);
    for channel in 0..3 {
        assert!(result[3].radiance[channel] < 0.041);
        assert!(
            (result[0].radiance[channel] - cases[0].globals.aerial_sunward[channel]).abs() < 1e-5
        );
        assert!((result[4].radiance[channel] - cases[0].globals.aerial_side[channel]).abs() < 1e-5);
        assert!((result[5].radiance[channel] - cases[0].globals.aerial_away[channel]).abs() < 1e-5);
        assert_eq!(result[6].applied[channel], cases[6].color[channel]);
        assert_eq!(result[7].applied[channel], cases[7].color[channel]);
    }
    for (case, result) in cases.iter().zip(result) {
        for channel in 0..3 {
            let a = case.color[channel];
            let b = result.radiance[channel];
            let actual = result.applied[channel];
            assert!(actual.is_finite() && actual >= a.min(b) - 1e-5 && actual <= a.max(b) + 1e-5);
        }
    }
}

// The pre-gating shader equations are retained here as an independent numeric
// reference. Active effects must not change when zero-contribution work is skipped.
const REFERENCE_EFFECTS: &str = r#"fn reference_effects(initial_color: vec3<f32>, rd: vec3<f32>, sun_dir: vec3<f32>, observer_up: vec3<f32>, solar_alt: f32, air: f32) -> vec3<f32> {
    var color = initial_color;
    // Rainbows are centered exactly on the antisolar direction and
    // only exist with sunlit rain. Narrow RGB radii give spectral
    // order without another fullscreen pass.
    let anti_angle = acos(clamp(dot(rd, -sun_dir), -1.0, 1.0));
    let rain_gate = clamp(g.weather0.w / 8.0, 0.0, 1.0)
        * smoothstep(0.01, 0.20, solar_alt) * air;
    let bow_r = exp(-pow((anti_angle - radians(42.5)) / 0.010, 2.0));
    let bow_g = exp(-pow((anti_angle - radians(41.8)) / 0.010, 2.0));
    let bow_b = exp(-pow((anti_angle - radians(41.1)) / 0.010, 2.0));
    let secondary = exp(-pow((anti_angle - radians(51.0)) / 0.014, 2.0));
    color += vec3<f32>(bow_r, bow_g, bow_b) * rain_gate * 0.075;
    color += vec3<f32>(0.20, 0.32, 0.70) * secondary * rain_gate * 0.018;

    let sun_angle = acos(clamp(dot(rd, sun_dir), -1.0, 1.0));
    // High-latitude summer twilight support for noctilucent cloud.
    let polar = smoothstep(0.65, 0.92, abs(observer_up.z));
    let nlc_twilight = smoothstep(-0.28, -0.12, solar_alt)
        * (1.0 - smoothstep(-0.04, 0.02, solar_alt));
    let nlc = cloud_noise(rd, g.weather2.x * 0.08, g.wind.xy * 0.15);
    color += vec3<f32>(0.12, 0.28, 0.52)
        * smoothstep(0.66, 0.88, nlc) * polar * nlc_twilight * air * 0.08;

    // Rare clean-horizon green flash approximation. Deterministic
    // seed phase and calm sea gate the event; geometry stays at Sun.
    let green_event = smoothstep(0.985, 0.998, g.weather2.y)
        * (1.0 - g.surface.y) * (1.0 - g.surface.z);
    let green_horizon = exp(-pow(solar_alt / 0.004, 2.0))
        * exp(-pow(sun_angle / 0.008, 2.0));
    color += vec3<f32>(0.05, 0.95, 0.24) * green_event * green_horizon * 0.22;
    return color;
}
"#;

#[test]
fn transient_effect_gates_match_ungated_shader_in_active_and_inactive_conditions() {
    let Some(fixture) = Fixture::with_entry_point("effects_main") else {
        return;
    };
    let mut cases = Vec::new();
    for solar in [
        -0.4, -0.28, -0.20, -0.12, -0.04, 0.0, 0.01, 0.10, 0.20, 0.80,
    ] {
        for polar in [0.0, 0.65, 0.80, 0.98] {
            for air in [0.0, 0.5, 1.0] {
                for rain in [0.0, 2.0, 8.0] {
                    for seed in [0.25, 0.985, 1.0] {
                        for angle in [0.0_f32, 1.1, std::f32::consts::PI - 42.5_f32.to_radians()] {
                            let mut case = Case::default();
                            case.effects = [solar, air, polar, 0.0];
                            case.globals.sun_dir = [1.0, 0.0, 0.0, 1.0];
                            case.globals.weather0[3] = rain;
                            case.globals.weather2[1] = seed;
                            case.position_grid = [angle.cos(), angle.sin(), 0.0, 1.0];
                            cases.push(case);
                        }
                    }
                }
            }
        }
    }
    // A stratified set of directions exercises the rare noise maximum in
    // polar twilight, while the matrix above covers each exact gate boundary.
    for index in 0..2048 {
        let z = 1.0 - 2.0 * (index as f32 + 0.5) / 2048.0;
        let angle = index as f32 * 2.399_963_1;
        let horizontal = (1.0 - z * z).sqrt();
        let mut case = Case::default();
        case.effects = [-0.12, 1.0, 0.98, 0.0];
        case.position_grid = [horizontal * angle.cos(), horizontal * angle.sin(), z, 1.0];
        cases.push(case);
    }
    let results = fixture.run(&cases);
    let mut changed = 0;
    let mut polar_changed = 0;
    let polar_start = cases.len() - 2048;
    for (index, (case, result)) in cases.iter().zip(results).enumerate() {
        let active = result.optimized_effects[..3] != case.color[..3];
        changed += usize::from(active);
        polar_changed += usize::from(index >= polar_start && active);
        for channel in 0..3 {
            assert!(
                (result.optimized_effects[channel] - result.reference_effects[channel]).abs()
                    < 2e-6,
                "effect mismatch at case {index}, channel {channel}: {result:?}"
            );
        }
        if case.effects[2] == 0.0
            && case.globals.weather0[3] == 0.0
            && case.globals.weather2[1] < 0.985
        {
            assert_eq!(result.optimized_effects[..3], case.color[..3]);
        }
    }
    assert!(changed > 100, "test must include visibly active effects");
    assert!(
        polar_changed > 0,
        "test must include visible polar twilight clouds"
    );
}
