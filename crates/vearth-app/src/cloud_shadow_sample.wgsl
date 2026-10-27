@group(0) @binding(4) var cloud_sunlight_tex: texture_2d<f32>;
@group(0) @binding(5) var cloud_sunlight_sampler: sampler;
@group(0) @binding(6) var<uniform> cloud_shadow_frame: CloudShadowFrame;

/// Cached sun transmission at a camera-relative position, from the top of
/// the cloud layer down to its height, and the map's support there: zero
/// outside the map, at low sun, in orbit or when cloud sunlight is off.
fn cloud_sunlight_column(position: vec3<f32>) -> vec2<f32> {
    if cloud_shadow_frame.origin.w <= 0.0 {
        return vec2<f32>(1.0, 0.0);
    }
    let layer = cloud_shadow_layer();
    let height = length(position - g.planet_center.xyz) - g.planet_center.w;
    let uv = cloud_shadow_uv(position, cloud_shadow_frame);
    let edge = max(abs(uv.x - 0.5), abs(uv.y - 0.5));
    let support = (1.0 - smoothstep(0.40, 0.49, edge)) * cloud_shadow_frame.origin.w;
    if support <= 0.0 {
        return vec2<f32>(1.0, 0.0);
    }
    let column = textureSampleLevel(cloud_sunlight_tex, cloud_sunlight_sampler, uv, 0.0);
    return vec2<f32>(cloud_shadow_height_transmission(column, height, layer), support);
}

fn cloud_sunlight(position: vec3<f32>, local_detail: f32, fallback: f32) -> f32 {
    if local_detail <= 0.0 {
        return fallback;
    }
    let column = cloud_sunlight_column(position);
    return mix(fallback, column.x, column.y * local_detail);
}
