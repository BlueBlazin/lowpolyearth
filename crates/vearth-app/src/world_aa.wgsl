@group(0) @binding(3) var depth_samples: texture_depth_multisampled_2d;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(p * 2.0 - vec2<f32>(1.0), 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @builtin(frag_depth) f32 {
    let pixel = vec2<i32>(position.xy);
    // Reversed-Z: preserve the closest covered sample for player occlusion,
    // contact shadows and cloud reprojection. Zero remains untouched sky.
    var nearest = 0.0;
    for (var i = 0u; i < textureNumSamples(depth_samples); i += 1u) {
        nearest = max(nearest, textureLoad(depth_samples, pixel, i));
    }
    return nearest;
}
