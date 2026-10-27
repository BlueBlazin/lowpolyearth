@group(1) @binding(0) var sky_environment_output: texture_storage_2d<rgba16float, write>;

// Sky only: cloud cover shades the reflected sky through the weather inputs
// of water_reflected_environment rather than as reflected cloud bodies.
@compute @workgroup_size(8, 8)
fn generate_sky_environment(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= vec2<u32>(64u)) { return; }
    let frame = sky_environment_frame();
    let direction = frame * sky_environment_decode(id.xy);
    let sky_conditions = vec3<f32>(max(length(g.planet_center.xyz) - g.planet_center.w, 0.0),
        g.surface.z, g.weather0.y);
    let color = water_reflected_environment(direction, frame[2], g.sun_dir.xyz,
        g.moon_dir.xyz, g.moon_dir.w, g.weather1.z, g.weather5.w,
        sky_conditions);
    textureStore(sky_environment_output, vec2<i32>(id.xy), vec4<f32>(color, 1.0));
}
