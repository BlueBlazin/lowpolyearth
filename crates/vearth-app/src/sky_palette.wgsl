// Shared by visible sky, reflected sky and the CPU horizon samples used for
// aerial perspective. The grade is confined to the low, Sun-facing sky.
fn sunset_sky_grade(radiance: vec3<f32>, view_sine: f32, solar_sine: f32,
        cosine_gamma: f32, observer_height: f32,
        aerosol_optical_depth: f32, humidity: f32) -> vec3<f32> {
    let twilight = smoothstep(sin(radians(-6.0)), sin(radians(-1.0)), solar_sine)
        * (1.0 - smoothstep(sin(radians(2.0)), sin(radians(10.0)), solar_sine));
    let low_sky = 1.0 - smoothstep(sin(radians(2.0)), sin(radians(20.0)), max(view_sine, 0.0));
    let sunward = smoothstep(0.0, 0.95, cosine_gamma);
    let surface = 1.0 - smoothstep(8000.0, 24000.0, max(observer_height, 0.0));
    let weather = 0.50 + 0.30 * smoothstep(0.025, 0.20, aerosol_optical_depth)
        + 0.20 * smoothstep(0.45, 0.90, humidity);
    let amount = twilight * low_sky * sunward * surface * weather;
    return radiance * mix(vec3<f32>(1.0), vec3<f32>(1.10, 0.72, 0.36), amount);
}
