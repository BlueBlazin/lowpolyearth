// Shared by world surfaces and procedural flora. Horizon radiance comes from
// the same fitted sky state as the visible sky; this is a compact aerial
// perspective approximation, not a multiple-scattering or shadowed-fog solver.

fn aerial_height(position: vec3<f32>) -> f32 {
    let radial = position - g.planet_center.xyz;
    let radius = length(radial);
    let polar = radial.z / max(radius, 1.0);
    let surface_radius = 6378137.0 * (1.0 - 0.0033528 * polar * polar);
    return max(radius - surface_radius, 0.0);
}

// Exact exponential integral for a linearly changing height. The symmetric
// small-difference limit avoids cancellation on flat ground and ridge lines.
fn aerial_layer_column(start_height: f32, end_height: f32,
        distance_m: f32, scale_height: f32) -> f32 {
    let difference = (end_height - start_height) / scale_height;
    if abs(difference) < 0.01 {
        return distance_m * exp(-(start_height + end_height) * (0.5 / scale_height))
            * (1.0 + difference * difference / 24.0);
    }
    return distance_m * (exp(-start_height / scale_height)
        - exp(-end_height / scale_height)) / difference;
}

fn aerial_weather_medium(observer_altitude: f32) -> f32 {
    if g.surface.w < 0.0 {
        return 0.0;
    }
    let low_cloud = clamp(g.weather5.x, 0.0, 1.0);
    let cloud_optical_depth = max(g.weather5.w, 0.0);
    let rain_load = 1.0 - exp(-max(g.weather0.w, 0.0) / 4.0);
    let cloud_base = clamp(g.weather3.x, 300.0, 9000.0);
    let cloud_top = clamp(max(g.weather3.y, cloud_base + 600.0),
        cloud_base + 600.0, 15000.0);
    let access = 1.0 - smoothstep(cloud_top + 1200.0,
        cloud_top + 9000.0, observer_altitude);
    return access * (low_cloud * smoothstep(1.2, 8.0, cloud_optical_depth) * 1.8
        + rain_load * 1.35);
}

fn aerial_optical_depth(position: vec3<f32>) -> f32 {
    let distance_m = length(position);
    let observer_height = max(length(g.planet_center.xyz) - g.planet_center.w, 0.0);
    let midpoint_height = aerial_height(position * 0.5);
    let surface_height = aerial_height(position);
    let half_distance = distance_m * 0.5;
    // The midpoint retains Earth curvature; the analytic halves also retain
    // thin near-surface air when the camera is far above the atmosphere.
    let molecular_column = aerial_layer_column(observer_height, midpoint_height,
        half_distance, 8000.0) + aerial_layer_column(midpoint_height, surface_height,
        half_distance, 8000.0);
    let aerosol_column = aerial_layer_column(observer_height, midpoint_height,
        half_distance, 1800.0) + aerial_layer_column(midpoint_height, surface_height,
        half_distance, 1800.0);
    let local_weather = 1.0 - smoothstep(25000.0, 120000.0, observer_height);
    let visibility = clamp(g.weather0.z, 120.0, 250000.0);
    // A 2% contrast-distance convention maps visibility to extinction.
    // One bounded presentation gain keeps distant landforms readable as layers.
    let aerosol_extinction = max(3.912 / visibility - 1.6e-5, 0.0);
    let precipitation = clamp(g.weather0.w / 24.0, 0.0, 1.0);
    let snow = 1.0 - smoothstep(-0.8, 1.8, g.weather0.x);
    let weather_extinction = precipitation * mix(1.8e-5, 7.0e-5, snow)
        + aerial_weather_medium(observer_height) * 0.5e-5;
    return max(1.45 * (molecular_column * 1.6e-5
        + aerosol_column * (aerosol_extinction + weather_extinction) * local_weather), 0.0);
}

fn aerial_horizon_radiance(direction: vec3<f32>) -> vec3<f32> {
    let up = normalize(-g.planet_center.xyz);
    let sun = normalize(g.sun_dir.xyz);
    let solar_sine = dot(up, sun);
    let horizontal_view = direction - up * dot(direction, up);
    let horizontal_sun = sun - up * solar_sine;
    let heading_cosine = clamp(dot(horizontal_view, horizontal_sun)
        / max(length(horizontal_view) * length(horizontal_sun), 0.01), -1.0, 1.0);
    let toward = pow(smoothstep(0.0, 1.0, heading_cosine), 3.0);
    let away = smoothstep(0.0, 1.0, -heading_cosine);
    let horizon = mix(mix(g.aerial_side.rgb, g.aerial_sunward.rgb, toward),
        g.aerial_away.rgb, away);
    let civil = smoothstep(-0.22, -0.015, solar_sine);
    let horizon_light = smoothstep(-0.065, 0.025, solar_sine);
    let sky_energy = civil * (0.18 + 0.82 * horizon_light);
    var radiance = max(horizon, vec3<f32>(0.0)) * sky_energy;
    let overcast = clamp(smoothstep(2.4, 8.5, g.weather5.w) * 0.82
        + clamp(g.weather0.y, 0.0, 1.0) * 0.08
        + clamp(g.weather0.w / 24.0, 0.0, 1.0) * 0.16, 0.0, 0.86);
    let cloud_radiance = vec3<f32>(0.27, 0.33, 0.42)
        * (0.20 + 0.80 * max(solar_sine, 0.0));
    radiance = mix(radiance, cloud_radiance * sky_energy, overcast);
    let night = mix(vec3<f32>(0.008, 0.014, 0.040),
        vec3<f32>(0.020, 0.024, 0.032), overcast);
    radiance = mix(night, radiance, smoothstep(-0.22, -0.08, solar_sine));
    return radiance;
}

fn apply_aerial_perspective(color: vec3<f32>, position: vec3<f32>, sample_pitch_m: f32) -> vec3<f32> {
    let distance_m = length(position);
    if distance_m < 0.001 {
        return color;
    }
    // Coarse globe geometry retains its established limb ownership; sparse
    // orbital sample spacing must not turn coarse surface heights into fog rings.
    // That per-cell fade differs between neighbouring LODs, so from orbit, where
    // both are on screen, it must yield to one camera-altitude fade shared by
    // every pixel; otherwise mixed-LOD views show a hazy/clear patchwork.
    let observer_height = max(length(g.planet_center.xyz) - g.planet_center.w, 0.0);
    let orbital_fade = 1.0 - smoothstep(250000.0, 420000.0, observer_height);
    let coarse_fade = clamp(1.0 - (sample_pitch_m - 1000.0) / 3000.0, 0.0, 1.0) * orbital_fade;
    if coarse_fade <= 0.0 {
        return color;
    }
    let fog = (1.0 - exp(-min(aerial_optical_depth(position), 40.0))) * coarse_fade;
    return mix(color, aerial_horizon_radiance(position / distance_m), fog);
}
