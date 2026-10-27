use std::{borrow::Cow, time::Duration};
use wgpu::util::DeviceExt;

const SOURCE: &str = include_str!("../src/cloud_common.wgsl");
const HARNESS: &str = r#"
struct Case { position_coverage: vec4<f32>, weather: vec4<f32>, lighting: vec4<f32>, ray: vec4<f32> };
struct Result { density: vec4<f32>, radiance: vec4<f32>, wind: vec4<f32>, depth_visibility: vec4<f32>, extinction: vec4<f32>, temporal: vec4<f32>, far_field: vec4<f32>, far_bounds: vec4<f32>, scene_depth: vec4<f32>, filtered: vec4<f32>, local_segment: vec4<f32>, water: vec4<f32> };
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<Result>;
@group(0) @binding(2) var shape_tex: texture_3d<f32>;
@group(0) @binding(3) var shape_sampler: sampler;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&cases) { return; }
    let c = cases[id.x];
    let wind_enu = vec3<f32>(8.0, 3.0, 0.0);
    let context = CloudContext(6371000.0, 1000.0, 5000.0,
        c.position_coverage.w, c.weather.x, c.weather.y, 0.25,
        cloud_common_wind(normalize(c.position_coverage.xyz), wind_enu) * c.weather.z, 1.0);
    let sample = cloud_common_sample(c.position_coverage.xyz, context, shape_tex, shape_sampler);
    var storm = context;
    storm.water = cloud_common_water(10.0);
    results[id.x].water = vec4<f32>(cloud_common_water(1.5), storm.water,
        cloud_common_sample(c.position_coverage.xyz, storm, shape_tex, shape_sampler).density, 0.0);
    let filtered = cloud_common_sample_filtered(c.position_coverage.xyz, context,
        c.weather.w, shape_tex, shape_sampler);
    results[id.x].filtered = vec4<f32>(filtered.density, filtered.height_fraction, 0.0, 0.0);
    results[id.x].local_segment = vec4<f32>(cloud_common_local_segment(c.position_coverage.xyz,
        normalize(c.ray.xyz), context, c.ray.w), 0.0, 0.0);
    let column = cloud_common_column(normalize(c.position_coverage.xyz), context, shape_tex, shape_sampler);
    let segment = cloud_common_extinction(sample.density, 300.0);
    results[id.x].density = vec4<f32>(sample.density, sample.height_fraction, column, segment);
    results[id.x].radiance = vec4<f32>(cloud_common_lighting(c.lighting.x,
        0.6, c.lighting.y, c.lighting.z, c.lighting.w,
        vec3<f32>(1.0, 0.8, 0.55), vec3<f32>(0.3, 0.4, 0.5)),
        cloud_common_phase(c.lighting.x, c.lighting.z, c.lighting.w));
    results[id.x].wind = vec4<f32>(cloud_common_wind(normalize(c.position_coverage.xyz),
        wind_enu), 1.0);
    results[id.x].depth_visibility = vec4<f32>(
        select(0.0, 1.0, cloud_common_depth_visible(500.0, 300.0)),
        select(0.0, 1.0, cloud_common_depth_visible(500.0, 700.0)),
        select(0.0, 1.0, cloud_common_depth_visible(1e30, 10000.0)),
        select(0.0, 1.0, cloud_common_depth_visible(500.0, 0.0)));
    let cluster = cloud_common_cluster(cloud_common_drifted(c.position_coverage.xyz, context),
        context, 0.0, shape_tex, shape_sampler);
    let light_depth = cloud_common_near_light_optical_depth(c.position_coverage.xyz, context,
        normalize(c.position_coverage.xyz), sample.density, cluster, shape_tex, shape_sampler);
    results[id.x].extinction = vec4<f32>(cloud_common_light_transmittance(light_depth),
        cloud_common_light_transmittance(1320.0), cloud_common_extinction(1.0, 660.0),
        cloud_common_light_transmittance(660.0));
    var accumulated = 0.0;
    var count = 0.0;
    for (var i = 0u; i < 8u; i += 1u) {
        let value = select(0.0, 0.8, (i & 1u) != 0u);
        accumulated = mix(value, accumulated, cloud_common_history_feedback(count, 0.7,
            cloud_common_history_stability(0.0)));
        count = cloud_common_history_count(count, 1.0);
    }
    results[id.x].temporal = vec4<f32>(accumulated,
        cloud_common_history_feedback(0.0, 0.7, 1.0),
        cloud_common_history_feedback(31.0, 0.7, cloud_common_history_stability(2.0)), count);
    var far_context = context;
    far_context.coverage = 0.75;
    let far_origin = vec3<f32>(6377000.0, 0.0, 0.0);
    let far_ray = normalize(vec3<f32>(-0.05, 1.0, 0.0));
    let far_sample = cloud_common_far_sample(far_origin, far_ray, far_context,
        vec3<f32>(1.0, 0.0, 0.0), 1e30, 0.002, shape_tex, shape_sampler);
    let far_occluded = cloud_common_far_sample(far_origin, far_ray, far_context,
        vec3<f32>(1.0, 0.0, 0.0), 5000.0, 0.002, shape_tex, shape_sampler);
    far_context.coverage = 0.0;
    let far_clear = cloud_common_far_sample(far_origin, far_ray, far_context,
        vec3<f32>(1.0, 0.0, 0.0), 1e30, 0.002, shape_tex, shape_sampler);
    results[id.x].far_field = vec4<f32>(far_sample.alpha, far_occluded.alpha,
        far_clear.alpha, far_sample.distance);
    // Convective cells build into the upper layer; stratiform decks stay low.
    far_context.coverage = 1.0;
    far_context.convection = 1.0;
    let upper = cloud_common_far_sample(vec3<f32>(6374500.0, 0.0, 0.0),
        vec3<f32>(0.0, 1.0, 0.0), far_context, vec3<f32>(1.0, 0.0, 0.0),
        1e30, 0.002, shape_tex, shape_sampler);
    let tangent_sine = -sqrt(1.0 - pow(6376000.0 / 6377000.0, 2.0));
    let above = cloud_common_far_sample(far_origin,
        normalize(vec3<f32>(tangent_sine + 0.0001, 1.0, 0.0)), far_context,
        vec3<f32>(1.0, 0.0, 0.0), 1e30, 0.002, shape_tex, shape_sampler);
    let tangent = cloud_common_far_sample(far_origin,
        normalize(vec3<f32>(tangent_sine - 0.0001, 1.0, 0.0)), far_context,
        vec3<f32>(1.0, 0.0, 0.0), 1e30, 0.002, shape_tex, shape_sampler);
    results[id.x].far_bounds = vec4<f32>(upper.alpha, above.alpha, tangent.alpha, upper.distance);
    results[id.x].scene_depth = vec4<f32>(
        cloud_common_scene_distance(vec4<f32>(0.6, 0.0, 0.8, 1.0 / 1000.0)),
        cloud_common_scene_distance(vec4<f32>(0.6, 0.0, 0.8, 1.0 / 250000.0)),
        cloud_common_scene_distance(vec4<f32>(0.6, 0.0, 0.8, 1.0 / 40000000.0)),
        cloud_common_scene_distance(vec4<f32>(0.6, 0.0, 0.8, 0.0)));
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Case {
    position_coverage: [f32; 4],
    weather: [f32; 4],
    lighting: [f32; 4],
    /// Local-segment ray direction and scene distance; the origin is the
    /// sample position.
    ray: [f32; 4],
}

impl Default for Case {
    fn default() -> Self {
        Self {
            position_coverage: [6_374_000.0, 0.0, 0.0, 0.65],
            weather: [0.4, 1.0, 60.0, 0.0],
            lighting: [0.4, 0.6, 0.8, 0.85],
            ray: [0.0, 1.0, 0.0, 1e30],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Result {
    density: [f32; 4],
    radiance: [f32; 4],
    wind: [f32; 4],
    depth_visibility: [f32; 4],
    extinction: [f32; 4],
    temporal: [f32; 4],
    far_field: [f32; 4],
    far_bounds: [f32; 4],
    scene_depth: [f32; 4],
    filtered: [f32; 4],
    local_segment: [f32; 4],
    water: [f32; 4],
}

#[test]
fn shared_cloud_helpers_validate_without_graphics_adapter() {
    let source = format!("{SOURCE}\n{HARNESS}");
    let module = naga::front::wgsl::parse_str(&source).expect("parse cloud shader");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validate cloud shader");
}

fn run(cases: &[Case], side: u32, mips: &[Vec<u8>]) -> Option<Vec<Result>> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("cloud numerical GPU test skipped: no adapter ({error})");
            return None;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("cloud numerical fixture"),
        ..Default::default()
    }))
    .expect("cloud test device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production cloud helpers"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{SOURCE}\n{HARNESS}"))),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cloud numerical fixture"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("constant hybrid cloud field"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: side,
        },
        mip_level_count: mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, data) in mips.iter().enumerate() {
        let width = (side >> level).max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(width),
            },
            wgpu::Extent3d {
                width,
                height: width,
                depth_or_array_layers: width,
            },
        );
    }
    let view = texture.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let inputs = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
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
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: inputs.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
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
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .expect("cloud readback");
    rx.recv().unwrap().unwrap();
    let results = bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
    readback.unmap();
    Some(results)
}

#[test]
fn cloud_weather_envelope_scattering_and_polar_wind_are_bounded() {
    let mut cases = Vec::new();
    for quality in [0.0, 1.0, 2.0] {
        for coverage in [0.0, 0.3, 0.6, 1.0] {
            for height in [
                0.0, 999.0, 1000.0, 1300.0, 2600.0, 4000.0, 4999.0, 5000.0, 6000.0,
            ] {
                cases.push(Case {
                    position_coverage: [6_371_000.0 + height, 0.0, 0.0, coverage],
                    weather: [0.4, quality, 60.0, 0.0],
                    ..Case::default()
                });
            }
        }
    }
    let body_count = cases.len();
    cases.extend([
        Case {
            lighting: [1.0, 0.8, 1.0, 1.0],
            ..Case::default()
        },
        Case {
            lighting: [-1.0, 0.8, 1.0, 1.0],
            ..Case::default()
        },
        Case {
            lighting: [0.4, 0.6, 0.0, 0.9],
            ..Case::default()
        },
        Case {
            lighting: [0.4, 0.6, 1.0, 0.9],
            ..Case::default()
        },
        Case {
            position_coverage: [0.0, 0.0, 6_374_000.0, 0.65],
            ..Case::default()
        },
        Case {
            position_coverage: [0.0, 0.0, -6_374_000.0, 0.65],
            ..Case::default()
        },
    ]);
    let Some(results) = run(&cases, 1, &[vec![168, 160, 154, 150]]) else {
        return;
    };
    for (case, result) in cases.iter().zip(&results) {
        assert!(
            (result.temporal[0] - 0.4).abs() < 1e-6,
            "stable stochastic hit/miss history must converge to its unbiased mean"
        );
        assert_eq!(
            result.temporal[1], 0.0,
            "first sample must not inherit uninitialized history"
        );
        assert!(
            (result.temporal[2] - 0.7).abs() < 1e-6,
            "moving history retains responsive feedback"
        );
        assert_eq!(result.temporal[3], 8.0);
        assert!(
            result.far_field[0] > 0.1 && result.far_field[0] <= 1.0,
            "cloud field must continue beyond local march range"
        );
        assert_eq!(
            result.far_field[1], 0.0,
            "foreground terrain blocks distant clouds"
        );
        assert_eq!(
            result.far_field[2], 0.0,
            "clear weather has no distant clouds"
        );
        assert!(result.far_field[3] > 47000.0);
        assert!(
            result.far_bounds[0] > 0.01 && result.far_bounds[3] > 42000.0,
            "upper-layer horizontal rays retain their actual far volume: {:?}",
            result.far_bounds
        );
        assert_eq!(
            result.far_bounds[1], 0.0,
            "cloud cannot extend above the actual layer tangent"
        );
        assert!(
            result.far_bounds[2] < 0.02,
            "vanishing tangent path must not make an opaque shell line: {:?}",
            result.far_bounds
        );
        for (actual, expected) in result
            .scene_depth
            .into_iter()
            .zip([1000.0, 250000.0, 40000000.0, 1e12])
        {
            assert!(
                (actual - expected).abs() <= expected * 1e-5,
                "scene depth must preserve foreground and far-Earth occluders: {actual} != {expected}"
            );
        }
        assert_eq!(
            result.depth_visibility,
            [1.0, 0.0, 1.0, 1.0],
            "cloud in front of terrain/over sky is visible; cloud behind terrain is occluded"
        );
        assert!(
            result
                .extinction
                .iter()
                .all(|x| x.is_finite() && (0.0..=1.0).contains(x))
        );
        assert!((result.extinction[1] - (-1.0_f32).exp()).abs() < 1e-5);
        assert!(
            (result.extinction[2] - (-1.5_f32).exp()).abs() < 1e-5,
            "view extinction uses its own 440 m scale: {:?}",
            result.extinction
        );
        assert!(
            result.extinction[2] < result.extinction[3],
            "the view ray is denser than the multiply scattered light path"
        );
        assert!(
            result
                .density
                .iter()
                .chain(&result.radiance)
                .chain(&result.wind)
                .all(|x| x.is_finite())
        );
        assert!(result.density.iter().all(|x| (0.0..=1.0).contains(x)));
        assert_eq!(result.water[0], 1.0, "fair-weather cloud keeps its density");
        assert_eq!(
            result.water[1], 3.0,
            "deep raining decks hold three times the water"
        );
        assert!(
            (result.water[2] - result.density[0] * 3.0).abs() < 1e-5,
            "water content scales density, not shape: {:?}",
            result.water
        );
        assert!(result.radiance.iter().all(|x| *x >= 0.0 && *x <= 5.0));
        let height = glam::Vec3::from_array(case.position_coverage[..3].try_into().unwrap())
            .length()
            - 6_371_000.0;
        if case.position_coverage[3] == 0.0 {
            assert_eq!(
                result.density[0], 0.0,
                "zero coverage must be exactly empty"
            );
            assert_eq!(
                result.density[2], 0.0,
                "zero coverage column must be exactly empty"
            );
        } else if height <= 1000.0 || height >= 5000.0 {
            // Native GPU length() may round an Earth-radius vector by a metre
            // differently from CPU sqrt. At the exact layer boundary that
            // moves the smooth density ramp by a few millionths.
            let near_boundary = (height - 1000.0).abs().min((height - 5000.0).abs()) <= 1.0;
            let tolerance = if near_boundary { 2e-5 } else { 0.0 };
            assert!(
                result.density[0] <= tolerance,
                "cloud outside meteorological envelope: height={height}, density={}",
                result.density[0]
            );
        }
        let up =
            glam::Vec3::from_array(case.position_coverage[..3].try_into().unwrap()).normalize();
        let wind = glam::Vec3::from_array(result.wind[..3].try_into().unwrap());
        assert!(
            wind.dot(up).abs() < 1e-4,
            "horizontal wind must remain Earth-tangent"
        );
        assert!(
            (wind.length_squared() - 73.0).abs() < 1e-3,
            "ENU conversion must preserve speed"
        );
    }
    assert!(
        results[..body_count]
            .iter()
            .any(|result| result.density[0] > 0.7)
    );
    for quality in 0..3 {
        for coverage in 0..3 {
            // Coverage changes the cumulus/stratus height profile as well as
            // the occupied area. Test its monotonic broad body and aerial
            // column, not individual samples in the changing boundary fades.
            for height in [4, 5] {
                let a = (quality * 4 + coverage) * 9 + height;
                assert!(
                    results[a].density[0] <= results[a + 9].density[0] + 1e-6,
                    "bulk coverage pair {coverage}, quality {quality}, height case {height}: {} -> {}",
                    results[a].density[0],
                    results[a + 9].density[0]
                );
            }
            let a = (quality * 4 + coverage) * 9 + 4;
            assert!(
                results[a].density[2] <= results[a + 9].density[2] + 1e-6,
                "column coverage pair {coverage}, quality {quality}: {} -> {}",
                results[a].density[2],
                results[a + 9].density[2]
            );
        }
    }
    assert!(
        results[body_count].radiance[3] > results[body_count + 1].radiance[3] * 4.0,
        "thin backlit cloud must have a brighter forward rim"
    );
    assert!(
        results[body_count + 2].radiance[0] < results[body_count + 3].radiance[0],
        "sun-occluded body must be darker than illuminated body"
    );
}

#[test]
fn distant_density_filters_unresolved_shape_without_changing_local_detail() {
    let mut fine = Vec::with_capacity(8 * 8 * 8 * 4);
    for _z in 0..8 {
        for y in 0..8 {
            for _x in 0..8 {
                // B holds the cluster field's median, so coverage stays neutral.
                fine.extend_from_slice(&[if y % 2 == 0 { 40 } else { 192 }, 160, 109, 150]);
            }
        }
    }
    let mips = vec![
        fine,
        [116, 160, 109, 150].repeat(64),
        [116, 160, 109, 150].repeat(8),
        vec![116, 160, 109, 150],
    ];
    let cases: Vec<_> = (0..24)
        .map(|i| {
            let angle = i as f32 * 0.00018;
            Case {
                position_coverage: [
                    6_374_000.0 * angle.cos(),
                    6_374_000.0 * angle.sin(),
                    0.0,
                    // Leaves the darker rows below the coverage contour.
                    0.45,
                ],
                weather: [0.4, 1.0, 0.0, 1e7],
                ..Case::default()
            }
        })
        .collect();
    let Some(results) = run(&cases, 8, &mips) else {
        return;
    };
    let extent = |values: Vec<f32>| {
        values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - values.iter().copied().fold(f32::INFINITY, f32::min)
    };
    let local_range = extent(results.iter().map(|r| r.density[0]).collect());
    let distant_range = extent(results.iter().map(|r| r.filtered[0]).collect());
    assert!(
        local_range > 0.10,
        "local LOD0 must retain resolved texture structure: {local_range}"
    );
    assert!(
        distant_range < 0.01,
        "unresolved shape must not alias into horizon dashes: {distant_range}"
    );
    assert!(
        results
            .iter()
            .all(|r| r.filtered[0].is_finite() && r.filtered[0] > 0.1)
    );
}

#[test]
fn local_segment_marches_eye_level_rays_and_stops_at_layer_and_scene_bounds() {
    const GROUND: f64 = 6_371_000.0;
    const BASE: f64 = 1_000.0;
    const TOP: f64 = 5_000.0;
    // cloud_common_local_range for observers at or below cloud top.
    const RANGE: f64 = 42_000.0;
    let ray = |altitude: f64, elevation_deg: f64, scene_distance: f32| {
        let elevation = elevation_deg.to_radians();
        Case {
            position_coverage: [(GROUND + altitude) as f32, 0.0, 0.0, 0.65],
            ray: [
                elevation.sin() as f32,
                elevation.cos() as f32,
                0.0,
                scene_distance,
            ],
            ..Case::default()
        }
    };
    let sphere = |altitude: f64, elevation_deg: f64, height: f64| {
        let origin = GROUND + altitude;
        let radius = GROUND + height;
        let projection = origin * elevation_deg.to_radians().sin();
        let root = (projection * projection - (origin - radius) * (origin + radius)).sqrt();
        (-projection - root, -projection + root)
    };
    // Rays within about 1.5 degrees of eye level miss the planet sphere at
    // 2.1 km. They still cross kilometres of layer before any bound.
    let eye_level: Vec<f64> = (-9..=9).map(|step| f64::from(step) * 0.1).collect();
    let mut cases: Vec<Case> = eye_level
        .iter()
        .map(|&elevation| ray(2_100.0, elevation, 1e30))
        .collect();
    let bounded = [
        // (altitude, elevation, scene distance, expected start, expected end)
        (2_100.0, -20.0, 1e30, 0.0, sphere(2_100.0, -20.0, BASE).0),
        (2_100.0, 20.0, 1e30, 0.0, sphere(2_100.0, 20.0, TOP).1),
        (2_100.0, -89.0, 1e30, 0.0, sphere(2_100.0, -89.0, BASE).0),
        (2_100.0, 0.0, 8_000.0, 0.0, 8_000.0 - 8_000.0 * 0.0005),
        (
            500.0,
            10.0,
            1e30,
            sphere(500.0, 10.0, BASE).1,
            sphere(500.0, 10.0, TOP).1,
        ),
        (
            6_000.0,
            -30.0,
            1e30,
            sphere(6_000.0, -30.0, TOP).0,
            sphere(6_000.0, -30.0, BASE).0,
        ),
    ];
    cases.extend(
        bounded
            .iter()
            .map(|&(altitude, elevation, scene, _, _)| ray(altitude, elevation, scene)),
    );
    // Below the base, a level ray reaches the layer beyond the local range;
    // above the top, it never enters the layer.
    cases.extend([ray(500.0, 0.0, 1e30), ray(6_000.0, 0.0, 1e30)]);
    let Some(results) = run(&cases, 1, &[vec![168, 160, 154, 150]]) else {
        return;
    };
    let close = |actual: f32, expected: f64| {
        (f64::from(actual) - expected).abs() <= (expected.abs() * 1e-3).max(3.0)
    };
    for (elevation, result) in eye_level.iter().zip(&results) {
        let [start, end, ..] = result.local_segment;
        assert!(
            close(start, 0.0) && close(end, RANGE),
            "eye-level ray at {elevation} deg must march the whole local range: {start}..{end}"
        );
    }
    let offset = eye_level.len();
    for (index, &(altitude, elevation, scene, start, end)) in bounded.iter().enumerate() {
        let [actual_start, actual_end, ..] = results[offset + index].local_segment;
        assert!(
            close(actual_start, start) && close(actual_end, end),
            "altitude {altitude}, elevation {elevation}, scene {scene}: \
             {actual_start}..{actual_end} != {start}..{end}"
        );
    }
    for result in &results[offset + bounded.len()..] {
        let [start, end, ..] = result.local_segment;
        assert!(
            end <= start,
            "level ray outside the layer must be empty: {start}..{end}"
        );
    }
}
