use super::*;

#[test]
fn cloud_sunlight_shader_validates() {
    let source = compute_source();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("cloud sunlight validation");
}

#[test]
fn light_plane_preserves_sun_rays_and_gates_night_orbit_and_disabled_clouds() {
    let radius = 6_371_000.0;
    for up in [DVec3::X, DVec3::Z, DVec3::new(0.2, 0.8, 0.5).normalize()] {
        for sun in [up, (up + DVec3::Y * 0.7).normalize()] {
            let camera = up * (radius + 1700.0);
            let f = frame(camera, sun, radius, true);
            assert_eq!(f.origin[3], 1.0);
            let right = DVec3::new(f.right[0] as f64, f.right[1] as f64, f.right[2] as f64);
            let upward = DVec3::new(f.upward[0] as f64, f.upward[1] as f64, f.upward[2] as f64);
            assert!(right.dot(sun).abs() < 1e-7);
            assert!(upward.dot(sun).abs() < 1e-7);
            assert!(right.dot(upward).abs() < 1e-7);
            assert_eq!(frame(camera, -up, radius, true).origin[3], 0.0);
            assert_eq!(frame(camera, sun, radius, false).origin[3], 0.0);
            assert_eq!(
                frame(up * (radius + 70_000.0), sun, radius, true).origin[3],
                0.0
            );
        }
    }
}

const COMPARE: &str = r#"
@group(2) @binding(0) var<storage, read_write> comparisons: array<vec4<f32>>;
@compute @workgroup_size(64)
fn compare_columns(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&comparisons) { return; }
    let size = f32(textureDimensions(cloud_sunlight_tex).x);
    let pixel = floor(vec2<f32>(0.125 + f32(id.x % 8u) * 0.105, 0.125 + f32(id.x / 8u) * 0.105) * size) + vec2<f32>(0.5);
    let offset = (pixel / size - vec2<f32>(0.5)) / frame.right.w;
    let origin = frame.origin.xyz - g.planet_center.xyz
        + frame.right.xyz * offset.x + frame.upward.xyz * offset.y;
    let layer = cloud_shadow_layer();
    let sun = normalize(g.sun_dir.xyz);
    let start = cloud_shadow_shell_distance(origin, sun, g.planet_center.w + layer.x);
    let end = cloud_shadow_shell_distance(origin, sun, g.planet_center.w + layer.y);
    let step = (end - start) / 192.0;
    let context = CloudContext(g.planet_center.w, layer.x, layer.y, g.weather5.x,
        g.weather1.w, 0.0, g.weather2.y, g.cloud_drift.xyz, cloud_common_water(g.weather5.w));
    var optical = 0.0;
    for (var probe = 0u; probe < 192u; probe += 1u) {
        let p = origin + sun * (start + (f32(probe) + 0.5) * step);
        let density = cloud_common_sample_filtered(p, context, 1.0 / (frame.right.w * size), shape_tex, shape_sampler).density;
        optical += cloud_common_optical_density(density) * step;
    }
    let receiver = origin + sun * start + g.planet_center.xyz;
    let cached = cloud_sunlight(receiver, 1.0, 0.37);
    let above = cloud_sunlight(origin + sun * (end + 20.0) + g.planet_center.xyz, 1.0, 0.37);
    let coarse = cloud_sunlight(receiver, 0.0, 0.37);
    comparisons[id.x] = vec4<f32>(cached, cloud_common_light_transmittance(optical), above, coarse);
}
"#;

#[test]
fn gpu_cloud_columns_match_dense_integration_and_leave_clear_sky_and_high_receivers_lit() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("cloud sunlight GPU test skipped: {error}");
            return;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let source = format!(
        "{}\n{}\n{COMPARE}",
        compute_source(),
        include_str!("cloud_shadow_sample.wgsl")
    );
    let module = naga::front::wgsl::parse_str(&source).unwrap();
    let naga::TypeInner::Struct { members, span } = &module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("Globals"))
        .unwrap()
        .1
        .inner
    else {
        panic!("Globals");
    };
    let mut bytes = vec![0u8; *span as usize];
    let set = |bytes: &mut [u8], name: &str, value: [f32; 4]| {
        let offset = members
            .iter()
            .find(|m| m.name.as_deref() == Some(name))
            .unwrap()
            .offset as usize;
        bytes[offset..offset + 16].copy_from_slice(bytemuck::bytes_of(&value));
    };
    let camera = DVec3::X * 6_372_700.0;
    let sun = DVec3::new(0.7, 0.6, 0.2).normalize();
    set(
        &mut bytes,
        "planet_center",
        [-camera.x as f32, 0.0, 0.0, 6_371_000.0],
    );
    set(
        &mut bytes,
        "sun_dir",
        [sun.x as f32, sun.y as f32, sun.z as f32, 1.0],
    );
    set(&mut bytes, "weather3", [2000.0, 5000.0, 0.0, 0.0]);
    set(&mut bytes, "weather5", [0.35, 0.0, 0.0, 2.0]);
    let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("synthetic broken cloud field"),
        size: wgpu::Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut pixels = vec![0u8; 32 * 32 * 32 * 4];
    for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let x = (i % 32) as f32 / 32.0 * std::f32::consts::TAU;
        let y = ((i / 32) % 32) as f32 / 32.0 * std::f32::consts::TAU;
        let value = ((0.5 + 0.3 * x.sin() + 0.2 * y.cos()) * 255.0) as u8;
        // B holds the cluster field's median, so coverage stays neutral.
        pixel.copy_from_slice(&[value, value, 109, 255]);
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(128),
            rows_per_image: Some(32),
        },
        wgpu::Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 32,
        },
    );
    let view = texture.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let mut shadows = CloudShadows::new(&device, &globals, &view, &sampler);
    shadows.configured = true;
    shadows.update(&queue, camera, sun, 6_371_000.0, true, 0.65);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("compare_columns"),
        compilation_options: Default::default(),
        cache: None,
    });
    let input = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: shadows.uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&shadows.view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&shadows.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: shadows.uniform.as_entire_binding(),
            },
        ],
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64 * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let result_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(2),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: output.as_entire_binding(),
        }],
    });
    let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[],
    });
    let mut previous_pattern: Vec<f32> = Vec::new();
    // This coverage of the synthetic field leaves sunlit gaps between bodies.
    for (coverage, seconds) in [(0.35, 0.0), (0.35, 900.0), (0.0, 900.0)] {
        set(&mut bytes, "weather5", [coverage, 0.0, 0.0, 2.0]);
        set(&mut bytes, "weather2", [seconds, 0.0, 0.0, 0.0]);
        set(&mut bytes, "wind_low", [10.0, 3.0, 0.0, 0.0]);
        // On the ECEF X axis east is +Y and north is +Z: the renderer's drift
        // after `seconds` of that wind.
        set(
            &mut bytes,
            "cloud_drift",
            [0.0, 10.0 * seconds, 3.0 * seconds, 32.0],
        );
        queue.write_buffer(&globals, 0, &bytes);
        let mut encoder = device.create_command_encoder(&Default::default());
        shadows.encode(&mut encoder);
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &input, &[]);
            pass.set_bind_group(1, &empty, &[]);
            pass.set_bind_group(2, &result_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 64 * 16);
        queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap()
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(vearth_platform::time::Duration::from_secs(30)),
            })
            .unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let values: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        let mut darkest = 1.0f32;
        let mut brightest = 0.0f32;
        for &[cached, reference, above, coarse] in values {
            assert!((0.0..=1.0).contains(&cached));
            assert!(
                (cached - reference).abs() < 0.055,
                "cached={cached}, dense={reference}"
            );
            assert!((above - 1.0).abs() < 1e-5, "above={above}");
            assert!((coarse - 0.37).abs() < 1e-5);
            darkest = darkest.min(cached);
            brightest = brightest.max(cached);
        }
        if coverage > 0.0 {
            assert!(brightest - darkest > 0.25, "{darkest}..{brightest}");
            if !previous_pattern.is_empty() {
                let moved = values
                    .iter()
                    .zip(&previous_pattern)
                    .filter(|(value, previous)| (value[0] - **previous).abs() > 0.05)
                    .count();
                assert!(moved > 8, "wind must move the sunlight patches");
            }
            previous_pattern = values.iter().map(|value| value[0]).collect();
        } else {
            assert!((darkest - 1.0).abs() < 1e-5);
        }
        drop(mapped);
        readback.unmap();
    }
}
