@group(0) @binding(1) var<uniform> frame: CloudShadowFrame;
@group(0) @binding(2) var shape_tex: texture_3d<f32>;
@group(0) @binding(3) var shape_sampler: sampler;
@group(1) @binding(0) var sunlight_output: texture_storage_2d<rgba16float, write>;

// Signed outward intersection permits the light plane to cross the cloud
// layer. The factored radial difference avoids subtracting Earth-radius squares.
fn cloud_shadow_shell_distance(origin: vec3<f32>, sun: vec3<f32>, radius: f32) -> f32 {
    let radial = length(origin);
    let b = dot(origin, sun);
    let c = (radial - radius) * (radial + radius);
    return -c / max(b + sqrt(max(b * b - c, 0.0)), 1.0);
}

@compute @workgroup_size(8, 8)
fn generate_cloud_sunlight(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(sunlight_output);
    if any(id.xy >= size) { return; }
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>(size);
    let offset = (uv - vec2<f32>(0.5)) / frame.right.w;
    let origin = frame.origin.xyz - g.planet_center.xyz
        + frame.right.xyz * offset.x + frame.upward.xyz * offset.y;
    let sun = normalize(g.sun_dir.xyz);
    let layer = cloud_shadow_layer();
    let context = CloudContext(g.planet_center.w, layer.x, layer.y, g.weather5.x,
        g.weather1.w, 0.0, g.weather2.y, g.cloud_drift.xyz, cloud_common_water(g.weather5.w));
    let footprint = 1.0 / (frame.right.w * f32(size.x));
    var column = vec4<f32>(1.0);
    var optical = 0.0;
    // Three cumulative height bands share twelve density probes. Receivers
    // below the base use the full column; receivers above the top stay sunlit.
    for (var band = 2i; band >= 0i; band -= 1i) {
        let lower = mix(layer.x, layer.y, f32(band) / 3.0);
        let upper = mix(layer.x, layer.y, f32(band + 1i) / 3.0);
        let begin = cloud_shadow_shell_distance(origin, sun, g.planet_center.w + lower);
        let end = cloud_shadow_shell_distance(origin, sun, g.planet_center.w + upper);
        let step = max(end - begin, 0.0) * 0.25;
        for (var probe = 0u; probe < 4u; probe += 1u) {
            let position = origin + sun * (begin + (f32(probe) + 0.5) * step);
            let density = cloud_common_sample_filtered(position, context, footprint,
                shape_tex, shape_sampler).density;
            optical += cloud_common_optical_density(density) * step;
        }
        column[u32(band)] = cloud_common_light_transmittance(optical);
    }
    textureStore(sunlight_output, vec2<i32>(id.xy), column);
}
