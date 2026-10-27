@group(0) @binding(2) var sky_environment_texture: texture_2d<f32>;
@group(0) @binding(3) var sky_environment_sampler: sampler;

fn sample_reflected_environment(reflected_direction: vec3<f32>) -> vec3<f32> {
    let frame = sky_environment_frame();
    let up = frame[2];
    // Match the analytic helper's horizon fallback before lookup. This also
    // avoids the arbitrary azimuth of a reflection pointing straight down.
    let direction = normalize(reflected_direction
        - up * min(dot(reflected_direction, up), 0.0) + up * 0.00001);
    let local = transpose(frame) * direction;
    return textureSampleLevel(sky_environment_texture, sky_environment_sampler,
        sky_environment_encode(local), 0.0).rgb;
}
