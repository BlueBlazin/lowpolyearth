struct CloudShadowFrame {
    // Camera-relative light-plane origin; w enables the local representation.
    origin: vec4<f32>,
    right: vec4<f32>, // w = inverse map width in metres
    upward: vec4<f32>,
};

fn cloud_shadow_layer() -> vec2<f32> {
    let base = clamp(g.weather3.x, 300.0, 9000.0);
    return vec2<f32>(base, clamp(max(g.weather3.y, base + 600.0), base + 600.0, 15000.0));
}

fn cloud_shadow_uv(position: vec3<f32>, frame: CloudShadowFrame) -> vec2<f32> {
    let relative = position - frame.origin.xyz;
    return vec2<f32>(dot(relative, frame.right.xyz), dot(relative, frame.upward.xyz))
        * frame.right.w + vec2<f32>(0.5);
}

fn cloud_shadow_height_transmission(column: vec4<f32>, height: f32, layer: vec2<f32>) -> f32 {
    let slice = clamp((height - layer.x) / (layer.y - layer.x) * 3.0, 0.0, 3.0);
    let index = min(u32(slice), 2u);
    return mix(column[index], column[index + 1u], slice - f32(index));
}
