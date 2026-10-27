// Fitted radiative-transfer clear sky with weather-dependent aerosol/overcast.
// Celestial discs use a bounded perceptual size cue near the local horizon.
// Their directions, phase, lighting and atmospheric extinction remain physical.

struct Globals {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    moon_dir: vec4<f32>,
    ambient: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
    planet_center: vec4<f32>, // xyz = center (camera-relative), w = ground R
    viewport: vec4<f32>,
    sky: vec4<f32>,
    horizon_glow_0: vec4<f32>,
    horizon_glow_1: vec4<f32>,
    weather0: vec4<f32>,
    weather1: vec4<f32>,
    wind: vec4<f32>,
    surface: vec4<f32>,
    weather2: vec4<f32>,
    weather3: vec4<f32>,
    weather4: vec4<f32>, // drought, turbulence, strike latitude/longitude
    weather5: vec4<f32>, // low/mid/high cloud fraction, optical depth
    weather6: vec4<f32>, // dust fraction, wet fraction, conditional rate, year phase
    wind_low: vec4<f32>,
    wind_mid: vec4<f32>,
    wind_upper: vec4<f32>,
    prev_view_proj: mat4x4<f32>,
    camera_to_prev: vec4<f32>, // xyz translation, w elapsed simulation seconds
    temporal: vec4<f32>, // x history valid, y jitter phase, z previous flash
    material_origin: vec4<f32>,
    aerial_sunward: vec4<f32>,
    aerial_side: vec4<f32>,
    aerial_away: vec4<f32>,
    cloud_drift: vec4<f32>, // xyz wrapped ECEF advection, w history sample cap
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var depth_tex: texture_depth_2d;
@group(1) @binding(1) var milky_way_tex: texture_2d<f32>;
@group(1) @binding(2) var milky_way_sampler: sampler;
@group(1) @binding(3) var atmosphere_lut: texture_2d<f32>;
@group(1) @binding(4) var atmosphere_lut_sampler: sampler;
@group(1) @binding(5) var climate_atlas: texture_2d_array<f32>;
@group(1) @binding(6) var climate_sampler: sampler;
struct FittedSkyModel {
    values: array<vec4<f32>, 8>,
};
@group(1) @binding(7) var<uniform> fitted_sky: FittedSkyModel;
@group(1) @binding(8) var cloud_shape_tex: texture_3d<f32>;
@group(1) @binding(9) var cloud_shape_sampler: sampler;
@group(2) @binding(0) var cloud_history_color: texture_2d<f32>;
@group(2) @binding(1) var cloud_history_meta: texture_2d<f32>;
@group(2) @binding(2) var cloud_history_sampler: sampler;
@group(2) @binding(3) var world_depth_samples: texture_depth_multisampled_2d;
@group(3) @binding(0) var cloud_update_color: texture_2d<f32>;
@group(3) @binding(1) var cloud_update_meta: texture_2d<f32>;
@group(3) @binding(2) var cloud_update_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Fullscreen triangle.
    var out: VsOut;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, y, 0.5, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

struct PrecipOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) particle: vec4<f32>,
};

fn particle_hash(index: u32, salt: u32) -> f32 {
    var value = index * 0x9e3779b9u + salt * 0x85ebca6bu;
    value ^= value >> 16u;
    value *= 0x7feb352du;
    value ^= value >> 15u;
    value *= 0x846ca68bu;
    value ^= value >> 16u;
    return f32(value & 0x00ffffffu) * (1.0 / 16777216.0);
}

fn invisible_precipitation() -> PrecipOut {
    var out: PrecipOut;
    out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.particle = vec4<f32>(0.0);
    return out;
}

/// Instanced camera-local world-space precipitation. Every drop is placed in
/// an ENU volume, moved by a physical velocity and only then projected. Camera
/// roll or pitch therefore cannot rotate gravity into upward/sideways rain.
@vertex
fn vs_precipitation(
        @builtin(vertex_index) vertex_index: u32,
        @builtin(instance_index) instance_index: u32) -> PrecipOut {
    if g.surface.w < 0.0 || g.weather0.w <= 0.03 {
        return invisible_precipitation();
    }
    let center = g.planet_center.xyz;
    let up = normalize(-center);
    let observer_altitude = max(length(center) - g.planet_center.w, 0.0);
    let precipitation_ceiling = clamp(g.weather3.x + 2200.0, 2200.0, 6000.0);
    if observer_altitude >= precipitation_ceiling {
        return invisible_precipitation();
    }
    var east = cross(vec3<f32>(0.0, 0.0, 1.0), up);
    if dot(east, east) < 1e-5 {
        east = cross(vec3<f32>(1.0, 0.0, 0.0), up);
    }
    east = normalize(east);
    let north = normalize(cross(up, east));
    let rate = max(g.weather0.w, 0.0);
    let activity = clamp(log2(1.0 + rate) / log2(81.0), 0.025, 1.0);
    let existence = 0.025 + activity * 0.86;
    if particle_hash(instance_index, 3u) > existence {
        return invisible_precipitation();
    }

    let phase_code = floor(g.weather2.z + 0.01);
    var phase = phase_code;
    if phase_code == 3.0 {
        phase = select(1.0, 2.0, particle_hash(instance_index, 5u) > 0.48);
    }
    let is_snow = phase == 2.0;
    let is_hail = phase == 4.0;
    let fall_speed = select(
        mix(15.0, 29.0, activity),
        mix(2.0, 5.0, activity),
        is_snow,
    );
    let effective_fall_speed = select(fall_speed, mix(13.0, 24.0, activity), is_hail);
    let volume_height = select(52.0, 38.0, is_snow);
    let cycle_seconds = volume_height / effective_fall_speed;
    let life = fract(
        g.weather2.x / cycle_seconds
            + particle_hash(instance_index, 7u)
            + g.weather2.y * 0.37,
    );
    let elapsed = life * cycle_seconds;
    let radius = select(46.0, 34.0, is_snow);
    let wind_scale = select(1.0, 0.58, is_snow);
    let wind_east = g.wind_low.x * wind_scale;
    let wind_north = g.wind_low.y * wind_scale;
    let initial_x = (particle_hash(instance_index, 11u) * 2.0 - 1.0) * radius;
    let initial_y = (particle_hash(instance_index, 13u) * 2.0 - 1.0) * radius;
    let x = fract((initial_x + wind_east * elapsed + radius) / (2.0 * radius))
        * (2.0 * radius) - radius;
    let y = fract((initial_y + wind_north * elapsed + radius) / (2.0 * radius))
        * (2.0 * radius) - radius;
    let z = mix(volume_height * 0.88, -4.0, life);
    let position = east * x + north * y + up * z;
    let velocity = east * wind_east + north * wind_north
        + up * (g.wind_low.z - effective_fall_speed);
    let clip = g.view_proj * vec4<f32>(position, 1.0);
    if clip.w <= 0.05 {
        return invisible_precipitation();
    }
    let exposure = select(mix(0.020, 0.034, activity), 0.12, is_snow);
    let tail_clip = g.view_proj * vec4<f32>(position - velocity * exposure, 1.0);
    let head_ndc = clip.xy / clip.w;
    let tail_ndc = tail_clip.xy / max(tail_clip.w, 0.05);
    let delta_pixels = (tail_ndc - head_ndc) * g.viewport.xy * 0.5;
    var along = vec2<f32>(0.0, 1.0);
    if length(delta_pixels) > 0.01 {
        along = normalize(delta_pixels);
    }
    let across = vec2<f32>(-along.y, along.x);
    let half_length_pixels = max(
        length(delta_pixels) * 0.5,
        select(0.8, mix(1.8, 3.2, activity), is_snow),
    );
    let half_width_pixels = select(
        mix(0.42, 0.72, activity),
        mix(1.2, 2.2, activity),
        is_snow,
    );
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex_index & 3u];
    let center_ndc = (head_ndc + tail_ndc) * 0.5;
    let offset_pixels = across * corner.x * half_width_pixels
        + along * corner.y * half_length_pixels;
    let ndc = center_ndc + offset_pixels * 2.0 / max(g.viewport.xy, vec2<f32>(1.0));
    var out: PrecipOut;
    out.pos = vec4<f32>(ndc * clip.w, clip.z, clip.w);
    out.local = corner;
    let altitude_fade = 1.0 - smoothstep(
        precipitation_ceiling * 0.58,
        precipitation_ceiling,
        observer_altitude,
    );
    out.particle = vec4<f32>(length(position), phase, activity, altitude_fade);
    return out;
}

@fragment
fn fs_precipitation_world(in: PrecipOut) -> @location(0) vec4<f32> {
    let coordinate = clamp(
        vec2<i32>(in.pos.xy),
        vec2<i32>(0),
        vec2<i32>(textureDimensions(depth_tex)) - 1,
    );
    let depth = textureLoad(depth_tex, coordinate, 0);
    if depth > 0.0 {
        let ndc = vec2<f32>(
            in.pos.x / max(g.viewport.x, 1.0) * 2.0 - 1.0,
            1.0 - in.pos.y / max(g.viewport.y, 1.0) * 2.0,
        );
        let surface = g.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
        let surface_distance = length(surface.xyz / max(abs(surface.w), 1e-5));
        if in.particle.x >= surface_distance - 0.25 {
            discard;
        }
    }
    let phase = in.particle.y;
    let activity = in.particle.z;
    let edge = 1.0 - smoothstep(0.45, 1.0, abs(in.local.x));
    let cap = 1.0 - smoothstep(0.78, 1.0, abs(in.local.y));
    let radial = 1.0 - smoothstep(0.42, 1.0, length(in.local));
    let snow = select(0.0, 1.0, phase == 2.0);
    let hail = select(0.0, 1.0, phase == 4.0);
    let rain = 1.0 - max(snow, hail);
    let shape = rain * edge * cap + snow * radial + hail * radial;
    let alpha = shape * in.particle.w * (
        rain * mix(0.11, 0.28, activity)
            + snow * mix(0.34, 0.68, activity)
            + hail * mix(0.42, 0.76, activity)
    );
    if alpha <= 0.002 {
        discard;
    }
    let color = vec3<f32>(0.30, 0.38, 0.46) * rain
        + vec3<f32>(0.90, 0.94, 0.98) * snow
        + vec3<f32>(0.72, 0.84, 0.94) * hail;
    return vec4<f32>(color, clamp(alpha, 0.0, 0.82));
}

const ATMO_THICKNESS: f32 = 180000.0;
const SCALE_HEIGHT: f32 = 22000.0;
const RAYLEIGH: vec3<f32> = vec3<f32>(0.28, 0.5, 1.0);

// The disc uses spherical atmospheric columns rather than a ground-level
// air-mass tint. Earth density heights follow Bruneton's reference atmosphere:
// 8 km molecular, 1.2 km aerosol, and ozone between 10 and 40 km. This small
// quadrature runs only on the solar disc, never on every sky or cloud sample.
fn solar_density(height_m: f32) -> vec3<f32> {
    let height = max(height_m, 0.0);
    let ozone = clamp(1.0 - abs(height - 25000.0) / 15000.0, 0.0, 1.0);
    return vec3<f32>(exp(-height / 8000.0), exp(-height / 1200.0), ozone);
}

fn solar_ray_height(distance_m: f32, observer_height: f32,
        radial_projection: f32, ground_radius: f32) -> f32 {
    let height_numerator = observer_height * (2.0 * ground_radius + observer_height)
        + distance_m * (2.0 * radial_projection + distance_m);
    // Rationalizing radius minus ground_radius retains near-ground precision.
    let radius = sqrt(max(ground_radius * ground_radius + height_numerator, 1.0));
    return max(height_numerator / (radius + ground_radius), 0.0);
}

fn solar_column_segment(start_m: f32, end_m: f32, observer_height: f32,
        radial_projection: f32, ground_radius: f32) -> vec3<f32> {
    let segment_length = abs(end_m - start_m);
    if segment_length < 0.001 {
        return vec3<f32>(0.0);
    }
    let direction = select(-1.0, 1.0, end_m > start_m);
    let start_height = solar_ray_height(start_m, observer_height,
        radial_projection, ground_radius);
    let radial_slope = abs(radial_projection + start_m) / (ground_radius + start_height);
    let sample_scale = 1200.0 / max(radial_slope, sqrt(1200.0 / ground_radius));
    let exponent = log(1.0 + segment_length / sample_scale);
    var column = vec3<f32>(0.0);
    var previous = 0.0;
    for (var i = 0u; i < 12u; i++) {
        let next = sample_scale * (exp(exponent * f32(i + 1u) / 12.0) - 1.0);
        let midpoint = start_m + direction * (previous + next) * 0.5;
        // Two-point Gaussian integration preserves thin aerosol layers without
        // allocating most samples to nearly empty upper atmosphere.
        let offset = (next - previous) * 0.2886751346;
        let first = solar_ray_height(midpoint - offset, observer_height,
            radial_projection, ground_radius);
        let second = solar_ray_height(midpoint + offset, observer_height,
            radial_projection, ground_radius);
        column += (solar_density(first) + solar_density(second)) * ((next - previous) * 0.5);
        previous = next;
    }
    return column;
}

fn celestial_size_scale(zenith_cosine: f32, observer_height: f32,
        ground_radius: f32) -> f32 {
    let height = max(observer_height, 0.0);
    let radius = max(ground_radius, 1.0);
    let horizon_sine = -sqrt(height * (2.0 * radius + height)) / (radius + height);
    let above_horizon = max(zenith_cosine - horizon_sine, 0.0);
    let horizon_cue = 1.0 - smoothstep(sin(radians(3.0)), sin(radians(30.0)), above_horizon);
    let ground_cue = 1.0 - smoothstep(8000.0, 80000.0, height);
    // A perceptual presentation choice, not atmospheric magnification. Neither
    // camera pitch nor zoom changes the cue; orbit returns to angular size.
    return 1.0 + (0.75 + 2.25 * horizon_cue) * ground_cue;
}

fn celestial_disc_edge(ray: vec3<f32>, direction: vec3<f32>, radius: f32,
        pixel_angle: f32) -> f32 {
    // Chord distance avoids loss of f32 precision from subtracting cosines
    // close to one. The soft edge follows the angular footprint of a pixel.
    let chord = length(ray - direction);
    let half_pixel = max(pixel_angle * 0.55, radius * 0.015);
    return 1.0 - smoothstep(radius - half_pixel, radius + half_pixel, chord);
}

fn lunar_mare(uv: vec2<f32>, center: vec2<f32>, axes: vec2<f32>) -> f32 {
    let point = (uv - center) / axes;
    return 1.0 - smoothstep(0.48, 1.0, dot(point, point));
}

fn lunar_surface_albedo(uv: vec2<f32>, pixel_width: f32) -> f32 {
    // Broad, overlapping maria remain legible on a small disc. These are
    // procedural markings, not a surveyed lunar map or a libration model.
    let western = max(lunar_mare(uv, vec2<f32>(-0.42, 0.19), vec2<f32>(0.40, 0.53)),
        lunar_mare(uv, vec2<f32>(-0.15, 0.43), vec2<f32>(0.34, 0.28)));
    let eastern = max(lunar_mare(uv, vec2<f32>(0.31, 0.19), vec2<f32>(0.31, 0.29)),
        lunar_mare(uv, vec2<f32>(0.49, -0.05), vec2<f32>(0.29, 0.25)));
    let isolated = lunar_mare(uv, vec2<f32>(0.68, 0.30), vec2<f32>(0.15, 0.18));
    let maria = max(western, max(eastern * 0.86, isolated * 0.90));
    let crater_offset = uv - vec2<f32>(-0.12, -0.61);
    let crater = exp(-dot(crater_offset, crater_offset) * 70.0);
    let surface = 0.94 - maria * 0.40 + crater * 0.06;
    let resolved = 1.0 - smoothstep(0.16, 0.48, max(pixel_width, 0.0));
    return mix(0.82, surface, resolved);
}

fn solar_disc_transmittance(observer_height: f32, zenith_cosine: f32,
        ground_radius: f32, aerosol_vertical_depth: vec3<f32>) -> vec3<f32> {
    let height = max(observer_height, 0.0);
    let radius = ground_radius + height;
    let projection = radius * clamp(zenith_cosine, -1.0, 1.0);
    let ground_discriminant = projection * projection - height * (2.0 * ground_radius + height);
    if projection < 0.0 && ground_discriminant >= 0.0 {
        return vec3<f32>(0.0);
    }
    let outer_radius = ground_radius + 180000.0;
    let atmosphere_discriminant = projection * projection
        + (outer_radius - radius) * (outer_radius + radius);
    if atmosphere_discriminant <= 0.0 {
        return vec3<f32>(1.0);
    }
    let root = sqrt(atmosphere_discriminant);
    let entry = max(-projection - root, 0.0);
    let exit = -projection + root;
    if exit <= entry {
        return vec3<f32>(1.0);
    }
    let closest = clamp(-projection, entry, exit);
    let column = solar_column_segment(closest, entry, height, projection, ground_radius)
        + solar_column_segment(closest, exit, height, projection, ground_radius);
    let molecular = vec3<f32>(5.802e-6, 13.558e-6, 33.100e-6);
    let ozone = vec3<f32>(0.006, 0.022, 0.004) / 15000.0;
    let extinction = molecular * column.x
        + max(aerosol_vertical_depth, vec3<f32>(0.0)) * (column.y / 1200.0)
        + ozone * column.z;
    return exp(-extinction);
}

fn rayleigh_phase(mu: f32) -> f32 {
    return 0.75 * (1.0 + mu * mu);
}

fn mie_phase(mu: f32, asymmetry: f32) -> f32 {
    let g2 = asymmetry * asymmetry;
    return (1.0 - g2)
        / pow(max(1.0 + g2 - 2.0 * asymmetry * mu, 0.015), 1.5);
}

fn fitted_sky_value(index: u32) -> f32 {
    return fitted_sky.values[index / 4u][index & 3u];
}

// GPU evaluation of the Hosek-Wilkie RGB sky distribution. Its coefficients
// are fitted to a brute-force radiative-transfer simulation and uploaded from
// the CPU for the current solar elevation, measured aerosol optical depth and
// ground albedo.
fn fitted_sky_channel(theta: f32, gamma: f32, channel: u32) -> f32 {
    let base = channel * 9u;
    let p0 = fitted_sky_value(base);
    let p1 = fitted_sky_value(base + 1u);
    let p2 = fitted_sky_value(base + 2u);
    let p3 = fitted_sky_value(base + 3u);
    let p4 = fitted_sky_value(base + 4u);
    let p5 = fitted_sky_value(base + 5u);
    let p6 = fitted_sky_value(base + 6u);
    let p7 = fitted_sky_value(base + 7u);
    let p8 = fitted_sky_value(base + 8u);
    let cosine_gamma = cos(gamma);
    let cosine_gamma2 = cosine_gamma * cosine_gamma;
    let cosine_theta = abs(cos(theta));
    let exponential = exp(p4 * gamma);
    let rayleigh = cosine_gamma2;
    let mie = (1.0 + cosine_gamma2)
        / pow(max(1.0 + p8 * p8 - 2.0 * p8 * cosine_gamma, 1e-5), 1.5);
    let zenith = sqrt(cosine_theta);
    let distribution = (1.0 + p0 * exp(p1 / (cosine_theta + 0.01)))
        * (p2 + p3 * exponential + p5 * rayleigh + p6 * mie + p7 * zenith);
    return max(fitted_sky_value(27u + channel) * distribution, 0.0);
}

fn fitted_clear_sky(
        view_direction: vec3<f32>, local_up: vec3<f32>,
        sun_direction: vec3<f32>) -> vec3<f32> {
    let solar_sine = clamp(dot(local_up, sun_direction), -1.0, 1.0);
    let horizontal_sun = sun_direction - local_up * solar_sine;
    var effective_sun = local_up;
    if dot(horizontal_sun, horizontal_sun) > 1e-7 {
        effective_sun = normalize(horizontal_sun
            + local_up * max(solar_sine, 0.001));
    }
    let theta = acos(clamp(dot(view_direction, local_up), 0.0, 1.0));
    let gamma = acos(clamp(dot(view_direction, effective_sun), -1.0, 1.0));
    let radiance = vec3<f32>(
        fitted_sky_channel(theta, gamma, 0u),
        fitted_sky_channel(theta, gamma, 1u),
        fitted_sky_channel(theta, gamma, 2u),
    );
    // The fitted model covers the Sun at and above the horizon. Preserve that
    // same directional solution into civil twilight and only reduce its total
    // energy as the directly lit atmospheric shell shrinks.
    let civil_light = smoothstep(-0.22, -0.015, solar_sine);
    let horizon_light = smoothstep(-0.065, 0.025, solar_sine);
    let illumination = civil_light * (0.18 + 0.82 * horizon_light);
    return sunset_sky_grade(radiance * (0.035 * illumination),
        dot(view_direction, local_up), solar_sine, dot(view_direction, effective_sun),
        max(length(g.planet_center.xyz) - g.planet_center.w, 0.0),
        g.surface.z, g.weather0.y);
}

// One intersection routine and one (-1, -1) miss value for sky and cloud.
fn ray_sphere(ro: vec3<f32>, rd: vec3<f32>, c: vec3<f32>, r: f32) -> vec2<f32> {
    return cloud_common_sphere_interval(ro - c, rd, r);
}

fn lightning_strike_ground() -> vec3<f32> {
    let latitude = g.weather4.z;
    let longitude = g.weather4.w;
    let cos_latitude = cos(latitude);
    let direction = vec3<f32>(
        cos_latitude * cos(longitude),
        cos_latitude * sin(longitude),
        sin(latitude),
    );
    return g.planet_center.xyz + direction * g.planet_center.w;
}

fn lightning_hash(value: f32) -> f32 {
    let geographic_seed = g.weather4.z * 117.31 + g.weather4.w * 269.17
        + g.weather2.y * 431.79;
    return fract(sin(value * 91.17 + geographic_seed) * 43758.5453);
}

fn lightning_main_point(fraction: f32) -> vec3<f32> {
    let ground = lightning_strike_ground();
    let up = normalize(ground - g.planet_center.xyz);
    var east = cross(vec3<f32>(0.0, 0.0, 1.0), up);
    if dot(east, east) < 1e-5 {
        east = cross(vec3<f32>(1.0, 0.0, 0.0), up);
    }
    east = normalize(east);
    let north = normalize(cross(up, east));
    let cloud_base = clamp(g.weather3.x, 300.0, 9000.0);
    let cloud_top = clamp(max(g.weather3.y, cloud_base + 600.0), cloud_base + 600.0, 15000.0);
    let channel_top = mix(cloud_base, cloud_top, 0.58);
    let segment = floor(fraction * 12.0 + 0.5);
    let taper = sin(fraction * 3.14159265);
    let east_offset = (lightning_hash(segment * 2.0 + 1.0) - 0.5) * 310.0;
    let north_offset = (lightning_hash(segment * 2.0 + 2.0) - 0.5) * 310.0;
    return ground + up * (8.0 + channel_top * fraction)
        + (east * east_offset + north * north_offset) * taper;
}

fn ray_segment_distance(rd: vec3<f32>, start: vec3<f32>, end: vec3<f32>) -> vec2<f32> {
    let segment = end - start;
    let perpendicular_start = start - rd * dot(start, rd);
    let perpendicular_segment = segment - rd * dot(segment, rd);
    let denominator = dot(perpendicular_segment, perpendicular_segment);
    var along_segment = 0.0;
    if denominator > 1e-5 {
        along_segment = clamp(
            -dot(perpendicular_start, perpendicular_segment) / denominator,
            0.0,
            1.0,
        );
    }
    let point = start + segment * along_segment;
    let along_ray = dot(point, rd);
    return vec2<f32>(length(point - rd * along_ray), along_ray);
}

fn lightning_segment_energy(
        rd: vec3<f32>, start: vec3<f32>, end: vec3<f32>,
        opaque_distance: f32, strength: f32) -> f32 {
    let closest = ray_segment_distance(rd, start, end);
    if closest.y <= 0.0 || closest.y >= opaque_distance + 2.0 {
        return 0.0;
    }
    let pixel_m = max(
        closest.y * 2.0 * g.viewport.z / max(g.viewport.y, 1.0),
        0.12,
    );
    let core = 1.0 - smoothstep(pixel_m * 0.30, pixel_m * 1.15, closest.x);
    let glow = 1.0 - smoothstep(pixel_m * 0.75, pixel_m * 8.0, closest.x);
    return strength * (core * 7.5 + glow * glow * 0.72);
}

fn lightning_bolt_radiance(rd: vec3<f32>, opaque_distance: f32) -> vec3<f32> {
    let flash = g.weather2.w;
    if flash <= 0.002 {
        return vec3<f32>(0.0);
    }
    var energy = 0.0;
    for (var index = 0u; index < 12u; index += 1u) {
        let a = f32(index) / 12.0;
        let b = f32(index + 1u) / 12.0;
        energy += lightning_segment_energy(
            rd, lightning_main_point(a), lightning_main_point(b),
            opaque_distance, 1.0,
        );
    }

    // Two short stepped-leader branches share the main channel's geography
    // but terminate in air. Their lower energy keeps the silhouette legible.
    let ground = lightning_strike_ground();
    let up = normalize(ground - g.planet_center.xyz);
    var east = cross(vec3<f32>(0.0, 0.0, 1.0), up);
    if dot(east, east) < 1e-5 {
        east = cross(vec3<f32>(1.0, 0.0, 0.0), up);
    }
    east = normalize(east);
    let north = normalize(cross(up, east));
    for (var branch = 0u; branch < 2u; branch += 1u) {
        let branch_f = f32(branch);
        let origin = lightning_main_point(0.72 + branch_f * 0.11);
        let side = east * (lightning_hash(41.0 + branch_f) - 0.5) * 1150.0
            + north * (lightning_hash(47.0 + branch_f) - 0.5) * 1150.0;
        let end = lightning_main_point(0.28 + branch_f * 0.09) + side;
        var previous = origin;
        for (var index = 1u; index <= 4u; index += 1u) {
            let fraction = f32(index) * 0.25;
            let bend = east * (lightning_hash(61.0 + branch_f * 9.0 + f32(index)) - 0.5)
                * 120.0 * sin(fraction * 3.14159265);
            let point = mix(origin, end, fraction) + bend;
            energy += lightning_segment_energy(
                rd, previous, point, opaque_distance, 0.38,
            );
            previous = point;
        }
    }
    return vec3<f32>(0.63, 0.76, 1.0) * energy * flash;
}

fn dome_at(index: u32) -> f32 {
    if index < 4u {
        return g.horizon_glow_0[index];
    }
    return g.horizon_glow_1[index - 4u];
}

fn horizon_glow(direction: vec3<f32>, up: vec3<f32>) -> f32 {
    var east = cross(vec3<f32>(0.0, 0.0, 1.0), up);
    if dot(east, east) < 1e-5 {
        east = cross(vec3<f32>(1.0, 0.0, 0.0), up);
    }
    east = normalize(east);
    let north = normalize(cross(up, east));
    let azimuth = atan2(dot(direction, east), dot(direction, north));
    let sector = fract(azimuth / 6.2831853 + 1.0) * 8.0;
    let i0 = u32(floor(sector)) & 7u;
    let i1 = (i0 + 1u) & 7u;
    return mix(dome_at(i0), dome_at(i1), fract(sector));
}

fn current_naked_eye_limit(up: vec3<f32>, altitude: f32,
        air_column: f32) -> f32 {
    let effective_glow = clamp(g.sky.w * air_column, 0.0, 1.0);
    let site_limit = 7.8 - 5.3 * sqrt(effective_glow);
    let moon_alt = max(dot(up, normalize(g.moon_dir.xyz)), 0.0);
    let moon_penalty = 3.3 * pow(g.moon_dir.w, 0.6)
        * sqrt(moon_alt) * air_column;
    let solar_alt = dot(up, normalize(g.sun_dir.xyz));
    let twilight = smoothstep(sin(radians(-18.0)), sin(radians(-6.0)), solar_alt)
        * exp(-altitude / 20000.0);
    return mix(site_limit - moon_penalty, -2.0, twilight);
}

fn milky_way_radiance(direction_ecef: vec3<f32>, up: vec3<f32>,
        altitude: f32, air_column: f32) -> vec3<f32> {
    // Diffuse Galactic structure is lower contrast than point stars and is
    // therefore lost first. Moon, twilight, local glow, and directional city
    // domes all act through the same limiting-magnitude budget as the catalog.
    let sin_altitude = max(dot(up, direction_ecef), 0.0);
    let horizon_lobe = pow(1.0 - sin_altitude, 4.0);
    let dome_penalty = horizon_glow(direction_ecef, up) * air_column
        * horizon_lobe * 2.2;
    let limit = current_naked_eye_limit(up, altitude, air_column) - dome_penalty;
    let contrast = smoothstep(5.75, 6.85, limit);
    if contrast <= 0.001 {
        return vec3<f32>(0.0);
    }

    // Undo sidereal rotation, then use the standard J2000 equatorial-to-
    // Galactic rotation. The ESO panorama is Galactic equirectangular with
    // l=0, b=0 (the Galactic centre) at the texture centre.
    let c = g.sky.x;
    let s = g.sky.y;
    let equatorial = vec3<f32>(
        c * direction_ecef.x - s * direction_ecef.y,
        s * direction_ecef.x + c * direction_ecef.y,
        direction_ecef.z,
    );
    let galactic = vec3<f32>(
        dot(vec3<f32>(-0.0548755604, -0.8734370902, -0.4838350155), equatorial),
        dot(vec3<f32>( 0.4941094279, -0.4448296300,  0.7469822445), equatorial),
        dot(vec3<f32>(-0.8676661490, -0.1980763734,  0.4559837762), equatorial),
    );
    let longitude = atan2(galactic.y, galactic.x);
    let latitude = asin(clamp(galactic.z, -1.0, 1.0));
    let uv = vec2<f32>(
        0.5 - longitude / 6.2831853,
        clamp(0.5 - latitude / 3.14159265, 0.0, 1.0),
    );
    let panorama = max(textureSample(milky_way_tex,
        milky_way_sampler, uv).rgb - vec3<f32>(0.0012), vec3<f32>(0.0));
    let airmass = clamp(1.0 / max(sin_altitude, 0.035), 1.0, 12.0);
    let extinction_mag = 0.13 * (airmass - 1.0) * air_column;
    let extinction = pow(2.5118864, -extinction_mag);
    return panorama * (0.044 * contrast * extinction);
}

fn cloud_hash(p: vec3<f32>) -> f32 {
    // Integer lattice hashing is both cheaper and free of the directional
    // sine correlations that became visible as orbit-scale bands.
    var h = bitcast<u32>(i32(p.x)) * 0x8da6b343u;
    h ^= bitcast<u32>(i32(p.y)) * 0xd8163841u;
    h ^= bitcast<u32>(i32(p.z)) * 0xcb1ab31fu;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h & 0x00ffffffu) * (1.0 / 16777216.0);
}

fn cloud_value_noise(p: vec3<f32>) -> f32 {
    let cell = floor(p);
    let local = fract(p);
    let blend = local * local * (vec3<f32>(3.0) - 2.0 * local);
    let n000 = cloud_hash(cell);
    let n100 = cloud_hash(cell + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = cloud_hash(cell + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = cloud_hash(cell + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = cloud_hash(cell + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = cloud_hash(cell + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = cloud_hash(cell + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = cloud_hash(cell + vec3<f32>(1.0, 1.0, 1.0));
    let z0 = mix(mix(n000, n100, blend.x), mix(n010, n110, blend.x), blend.y);
    let z1 = mix(mix(n001, n101, blend.x), mix(n011, n111, blend.x), blend.y);
    return mix(z0, z1, blend.z);
}

fn cloud_band_weight(direction_footprint: f32, frequency: f32) -> f32 {
    // Remove an octave before it exceeds the pixel footprint. This keeps the
    // same Earth-fixed field while preventing oblique globe views from
    // resolving it into crawling scan lines.
    return 1.0 - smoothstep(0.34, 0.86, direction_footprint * frequency);
}

fn cloud_noise_filtered(direction: vec3<f32>, time: f32, wind: vec2<f32>,
        direction_footprint: f32) -> f32 {
    // Smooth Earth-fixed 3D value noise avoids longitude seams and stays below
    // pixel frequency in orbit. A small domain warp breaks up obvious blobs while
    // keeping the weather readable as broad systems rather than texture.
    let seed = g.weather2.y;
    let drift = vec3<f32>(wind.x, wind.y, wind.x * 0.31 - wind.y * 0.19)
        * time * 0.00000028;
    var p = direction * 5.10 + drift
        + vec3<f32>(seed * 11.3, seed * 7.1, seed * 5.7);
    let warp = vec3<f32>(
        cloud_value_noise(p + vec3<f32>(3.7, 19.1, 7.3)),
        cloud_value_noise(p + vec3<f32>(13.4, 5.2, 17.8)),
        cloud_value_noise(p + vec3<f32>(8.6, 23.5, 2.4)),
    ) - vec3<f32>(0.5);
    p += warp * 0.92;
    let base_frequency = 5.10;
    var centered = (cloud_value_noise(p) - 0.5) * 0.46
        * cloud_band_weight(direction_footprint, base_frequency);
    centered += (cloud_value_noise(p * 1.97 + vec3<f32>(9.2)) - 0.5) * 0.26
        * cloud_band_weight(direction_footprint, base_frequency * 1.97);
    centered += (cloud_value_noise(p * 3.91 + vec3<f32>(21.7)) - 0.5) * 0.16
        * cloud_band_weight(direction_footprint, base_frequency * 3.91);
    centered += (cloud_value_noise(p * 7.79 + vec3<f32>(47.3)) - 0.5) * 0.08
        * cloud_band_weight(direction_footprint, base_frequency * 7.79);
    centered += (cloud_value_noise(p * 15.53 + vec3<f32>(83.1)) - 0.5) * 0.04
        * cloud_band_weight(direction_footprint, base_frequency * 15.53);
    return clamp(0.5 + centered, 0.0, 1.0);
}

fn cloud_noise(direction: vec3<f32>, time: f32, wind: vec2<f32>) -> f32 {
    return cloud_noise_filtered(direction, time, wind, 0.0);
}

// These rare effects have frame-uniform zero-contribution gates. Keep their
// expensive angular and noise evaluation inside the gates so ordinary clear
// skies do not pay for rainbows, polar twilight clouds, or green flashes.
fn transient_sky_effects(initial_color: vec3<f32>, rd: vec3<f32>, sun_dir: vec3<f32>,
        observer_up: vec3<f32>, solar_alt: f32, air: f32) -> vec3<f32> {
    var color = initial_color;
    // Rainbows are centered exactly on the antisolar direction and
    // only exist with sunlit rain. Narrow RGB radii give spectral
    // order without another fullscreen pass.
    let rain_gate = clamp(g.weather0.w / 8.0, 0.0, 1.0)
        * smoothstep(0.01, 0.20, solar_alt) * air;
    if rain_gate != 0.0 {
        let anti_angle = acos(clamp(dot(rd, -sun_dir), -1.0, 1.0));
        let bow_r = exp(-pow((anti_angle - radians(42.5)) / 0.010, 2.0));
        let bow_g = exp(-pow((anti_angle - radians(41.8)) / 0.010, 2.0));
        let bow_b = exp(-pow((anti_angle - radians(41.1)) / 0.010, 2.0));
        let secondary = exp(-pow((anti_angle - radians(51.0)) / 0.014, 2.0));
        color += vec3<f32>(bow_r, bow_g, bow_b) * rain_gate * 0.075;
        color += vec3<f32>(0.20, 0.32, 0.70) * secondary * rain_gate * 0.018;
    }

    // High-latitude summer twilight support for noctilucent cloud.
    let polar = smoothstep(0.65, 0.92, abs(observer_up.z));
    let nlc_twilight = smoothstep(-0.28, -0.12, solar_alt)
        * (1.0 - smoothstep(-0.04, 0.02, solar_alt));
    if polar != 0.0 && nlc_twilight != 0.0 && air != 0.0 {
        let nlc = cloud_noise(rd, g.weather2.x * 0.08, g.wind.xy * 0.15);
        color += vec3<f32>(0.12, 0.28, 0.52)
            * smoothstep(0.66, 0.88, nlc) * polar * nlc_twilight * air * 0.08;
    }

    // Rare clean-horizon green flash approximation. Deterministic
    // seed phase and calm sea gate the event; geometry stays at Sun.
    let green_event = smoothstep(0.985, 0.998, g.weather2.y)
        * (1.0 - g.surface.y) * (1.0 - g.surface.z);
    if green_event != 0.0 {
        let sun_angle = acos(clamp(dot(rd, sun_dir), -1.0, 1.0));
        let green_horizon = exp(-pow(solar_alt / 0.004, 2.0))
            * exp(-pow(sun_angle / 0.008, 2.0));
        color += vec3<f32>(0.05, 0.95, 0.24) * green_event * green_horizon * 0.22;
    }
    return color;
}

fn climate_uv(direction: vec3<f32>) -> vec2<f32> {
    let longitude = atan2(direction.y, direction.x);
    let latitude = asin(clamp(direction.z, -1.0, 1.0));
    return vec2<f32>(
        fract(longitude / 6.2831853 + 0.5),
        clamp(0.5 - latitude / 3.14159265, 0.0, 1.0),
    );
}

fn climate_layer(direction: vec3<f32>, layer_offset: u32) -> vec4<f32> {
    let month_position = fract(g.weather6.w) * 12.0;
    let month0 = u32(floor(month_position)) % 12u;
    let month1 = (month0 + 1u) % 12u;
    let blend = fract(month_position);
    let uv = climate_uv(direction);
    let a = textureSampleLevel(
        climate_atlas, climate_sampler, uv, i32(month0 * 4u + layer_offset), 0.0,
    );
    let b = textureSampleLevel(
        climate_atlas, climate_sampler, uv, i32(month1 * 4u + layer_offset), 0.0,
    );
    return mix(a, b, blend);
}

fn climate_mid_wind(direction: vec3<f32>) -> vec2<f32> {
    let packed = climate_layer(direction, 1u).zw;
    return (packed * 255.0 - vec2<f32>(128.0)) * 0.5;
}

fn advect_climate_direction(direction: vec3<f32>, seconds: f32) -> vec3<f32> {
    var east = cross(vec3<f32>(0.0, 0.0, 1.0), direction);
    if dot(east, east) < 1e-5 {
        east = cross(vec3<f32>(1.0, 0.0, 0.0), direction);
    }
    east = normalize(east);
    let north = normalize(cross(direction, east));
    let wind = climate_mid_wind(direction);
    return normalize(direction - (east * wind.x + north * wind.y) * (seconds / 6371000.0));
}

fn orbital_cloud_coverage(direction: vec3<f32>, time: f32) -> vec2<f32> {
    // Monthly cloud regimes and advection come from the observed atlas. The
    // procedural component only adds coherent synoptic variability; it no
    // longer invents a latitude climate or hard-coded wind speed.
    let seed = g.weather2.y;
    let advected = advect_climate_direction(direction, time);
    let p = advected * 2.55
        + vec3<f32>(seed * 7.3, seed * 13.7, seed * 5.1);
    let system = cloud_value_noise(p) * 0.66
        + cloud_value_noise(p * 2.07 + vec3<f32>(17.2)) * 0.34;
    let layers = climate_layer(direction, 0u).xyz;
    let climate = 1.0 - (1.0 - layers.x) * (1.0 - layers.y) * (1.0 - layers.z);
    let preset_boost = select(0.0, 0.14, g.weather3.w > 1.5);
    let coverage = clamp(climate + (system - 0.5) * 0.48 + preset_boost, 0.02, 0.96);
    // Contours of the existing broad system field become coherent fronts;
    // this reuses already-paid noise instead of adding another octave.
    let front = (1.0 - smoothstep(0.045, 0.17, abs(system - 0.54)))
        * smoothstep(0.22, 0.68, coverage);
    return vec2<f32>(coverage, front);
}

fn local_cloud_context(ground_r: f32, base_m: f32, top_m: f32,
        coverage: f32, quality: f32) -> CloudContext {
    return CloudContext(ground_r, base_m, top_m, coverage, g.weather1.w,
        quality, g.weather2.y, g.cloud_drift.xyz, cloud_common_water(g.weather5.w));
}

fn local_cloud_density(relative_ecef: vec3<f32>, ground_r: f32,
        base_m: f32, top_m: f32, coverage: f32, footprint_m: f32, cluster: f32) -> vec2<f32> {
    let sample = cloud_common_sample_clustered(relative_ecef,
        local_cloud_context(ground_r, base_m, top_m, coverage, g.surface.w),
        footprint_m, cluster, cloud_shape_tex, cloud_shape_sampler);
    return vec2<f32>(sample.density, sample.height_fraction);
}

fn local_cloud_column(direction: vec3<f32>, ground_r: f32,
        base_m: f32, top_m: f32, coverage: f32) -> f32 {
    return cloud_common_column(direction,
        local_cloud_context(ground_r, base_m, top_m, coverage, 0.0),
        cloud_shape_tex, cloud_shape_sampler);
}

/// Mesoscale cluster at a distance along the view ray. The march samples it
/// sparsely and interpolates; it varies over tens of kilometres.
fn local_cloud_cluster(rd: vec3<f32>, center: vec3<f32>, distance_m: f32,
        ground_r: f32, base_m: f32, top_m: f32, footprint_m: f32) -> f32 {
    let context = local_cloud_context(ground_r, base_m, top_m, g.weather5.x, 0.0);
    return cloud_common_cluster(cloud_common_drifted(rd * distance_m - center, context),
        context, footprint_m, cloud_shape_tex, cloud_shape_sampler);
}

/// Sunlight reaching a cloud sample (x) and the share of sky light its
/// overhead cover lets through (y). A near probe resolves self-shadowing; the
/// cached sunlight columns carry the rest of the path to the top of the layer,
/// so towers shade neighbours and deep decks darken their bases. Beyond the
/// map, at low sun or with cloud sunlight off, the remaining path is estimated
/// from the sample's own density, as the distant representation does.
fn cloud_sun_visibility(point: vec3<f32>, relative_ecef: vec3<f32>, ground_r: f32,
        base_m: f32, top_m: f32, coverage: f32, origin_density: f32, cluster: f32) -> vec2<f32> {
    let light_dir = normalize(g.sun_dir.xyz);
    var optical_depth = cloud_common_optical_density(origin_density) * 260.0;
    var probe_reach = 0.0;
    // Standard and reflected clouds share one depth-relative light probe,
    // which stands for the first three probe distances toward the Sun.
    // Low keeps origin extinction only for constrained adapters.
    if g.surface.w > 0.5 {
        optical_depth = cloud_common_near_light_optical_depth(relative_ecef,
            local_cloud_context(ground_r, base_m, top_m, coverage, 0.0),
            light_dir, origin_density, cluster, cloud_shape_tex, cloud_shape_sampler);
        probe_reach = clamp((top_m - base_m) * 0.18, 160.0, 650.0) * 3.0;
    }
    let column = cloud_sunlight_column(point + light_dir * probe_reach);
    let up = normalize(relative_ecef);
    let remaining_m = max(top_m - (length(relative_ecef) - ground_r), 0.0)
        / max(dot(up, light_dir), 0.2) - probe_reach;
    let estimate = cloud_common_light_transmittance(
        cloud_common_optical_density(origin_density) * clamp(remaining_m, 0.0, 4000.0) * 0.5);
    let beyond = mix(estimate, column.x, column.y);
    // Cover that blocks the Sun also blocks most of the sky above. The floor
    // stands in for light scattered sideways and back up from the ground.
    let sky = mix(0.45, 1.0, sqrt(beyond));
    return vec2<f32>(cloud_common_light_transmittance(optical_depth) * beyond, sky);
}

struct CloudValue {
    rgba: vec4<f32>,
    front_distance: f32,
    representative_distance: f32,
    history_samples: f32,
};

fn cloud_ray_jitter(pixel: vec2<f32>) -> f32 {
    // Interleaved-gradient noise is itself diagonally correlated. Rotating its
    // phase hid but did not remove that structure in low-opacity cloud. A
    // compact integer avalanche gives every history texel an independent,
    // deterministic offset; the golden-ratio sequence then rotates it in time.
    let integer_pixel = vec2<u32>(floor(pixel));
    var bits = (integer_pixel.x * 0x9e3779b9u)
        ^ (integer_pixel.y * 0x85ebca6bu)
        ^ (u32(g.weather2.y * 65535.0) * 0xc2b2ae35u);
    bits ^= bits >> 16u;
    bits *= 0x7feb352du;
    bits ^= bits >> 15u;
    bits *= 0x846ca68bu;
    bits ^= bits >> 16u;
    let spatial_phase = f32(bits & 0x00ffffffu) * (1.0 / 16777216.0);
    // Every texel is refreshed once per complete 16-slot cycle. Using the raw
    // frame number gives each modulo class a different constant phase and a
    // persistent 4x4 finite-sample mean. Advance by update cycle instead.
    let update_cycle = floor(g.temporal.y * (1.0 / 16.0));
    let temporal_phase = fract(spatial_phase + update_cycle * 0.61803398875);
    return 0.08 + temporal_phase * 0.84;
}

fn local_cloud_volume(rd: vec3<f32>, center: vec3<f32>,
        ground_r: f32, observer_alt: f32,
        opaque_distance: f32, pixel: vec2<f32>) -> CloudValue {
    if g.weather5.x <= 0.001 {
        return CloudValue(vec4<f32>(0.0), 0.0, 0.0, 0.0);
    }
    let base_m = clamp(g.weather3.x, 300.0, 9000.0);
    let top_m = clamp(max(g.weather3.y, base_m + 600.0), base_m + 600.0, 15000.0);
    let segment = cloud_common_local_segment(-center, rd,
        local_cloud_context(ground_r, base_m, top_m, g.weather5.x, 0.0), opaque_distance);
    if segment.y <= segment.x {
        return CloudValue(vec4<f32>(0.0), 0.0, 0.0, 0.0);
    }

    // Each refreshed history pixel integrates the complete visible interval.
    // Adaptive retries that spent the budget on only the front third produced
    // contour bands where empty/dense mode changes moved the truncated tail.
    // Stratified steps keep the same iteration ceiling and early optical exit,
    // and every ray still covers its whole interval: short intervals take
    // fewer steps rather than sampling finer than edge detail can use.
    let step_budget = select(40u, select(72u, 128u, g.surface.w > 1.5), g.surface.w > 0.5);
    let span = segment.y - segment.x;
    let steps = clamp(u32(ceil(span / 50.0)), min(16u, step_budget), step_budget);
    let jitter = cloud_ray_jitter(pixel);
    let step_m = span / f32(steps);
    let sun_dir = normalize(g.sun_dir.xyz);
    let moon_dir = normalize(g.moon_dir.xyz);
    // Angle subtended by one half-resolution history pixel, which the
    // interleaved march accumulates; cloud edge detail finer than this aliases.
    let pixel_angle = 4.0 * g.viewport.z / max(g.viewport.y, 1.0);
    // Clusters are read every eight steps and interpolated, which keeps the
    // field continuous along the ray at a fraction of the fetches.
    let cluster_stride = 8u;
    var cluster_start_m = segment.x;
    var cluster_end_m = segment.x;
    var cluster_near = 0.5;
    var cluster_far = local_cloud_cluster(rd, center, segment.x, ground_r, base_m, top_m,
        segment.x * pixel_angle);
    var transmittance = 1.0;
    var radiance = vec3<f32>(0.0);
    var weighted_distance = 0.0;
    var front_distance = 0.0;
    var light = vec2<f32>(1.0);
    var light_valid = false;
    for (var i = 0u; i < 128u; i += 1u) {
        if i >= steps {
            break;
        }
        if i % cluster_stride == 0u {
            cluster_start_m = cluster_end_m;
            cluster_end_m = segment.x + f32(min(i + cluster_stride, steps)) * step_m;
            cluster_near = cluster_far;
            cluster_far = local_cloud_cluster(rd, center, cluster_end_m, ground_r, base_m,
                top_m, cluster_end_m * pixel_angle);
        }
        let step_jitter = fract(jitter + f32(i) * 0.754877666);
        let distance_m = segment.x + (f32(i) + step_jitter) * step_m;
        let point = rd * distance_m;
        let relative = point - center;
        let cluster = mix(cluster_near, cluster_far, clamp((distance_m - cluster_start_m)
            / max(cluster_end_m - cluster_start_m, 1.0), 0.0, 1.0));
        let density_sample = local_cloud_density(
            relative, ground_r, base_m, top_m, g.weather5.x, distance_m * pixel_angle, cluster,
        );
        let range_limit = cloud_common_local_range(observer_alt, top_m);
        let distance_fade = 1.0 - smoothstep(range_limit * 0.78, range_limit, distance_m);
        let near_camera_fade = smoothstep(180.0, 650.0, distance_m);
        let density = density_sample.x * distance_fade * near_camera_fade;
        if density <= 0.002 { continue; }
        // Samples behind most of the ray's opacity reuse the last sunlight:
        // they add at most 30% of the pixel and dense interiors vary little.
        if !light_valid || transmittance > 0.3 {
            light = cloud_sun_visibility(
                point,
                relative,
                ground_r,
                base_m,
                top_m,
                g.weather5.x,
                density,
                cluster,
            );
            light_valid = true;
        }
        let sun_visibility = light.x;
        let sample_height_m = max(length(relative) - ground_r, 0.0);
        let height_fraction = density_sample.y;
        let up = normalize(relative);
        let solar_sine = dot(up, sun_dir);
        let direct_sun = solar_transmittance(solar_sine, sample_height_m);
        let ambient = cloud_common_ambient(solar_sine, dot(up, moon_dir),
            g.moon_dir.w, dust_optical_load()) * light.y;
        // Segment transmittance describes depth inside this sample, unlike
        // accumulated view transmittance which also includes unrelated clouds.
        let optical_step_m = min(step_m, 900.0);
        let segment_transmittance = cloud_common_extinction(density, optical_step_m);
        var sample_color = cloud_common_lighting(dot(rd, sun_dir), density,
            height_fraction, sun_visibility, segment_transmittance, direct_sun, ambient);
        if g.weather2.w > 0.001 {
            let strike_ground = lightning_strike_ground();
            let strike_up = normalize(strike_ground - center);
            let electrical_source = strike_ground + strike_up
                * mix(base_m, top_m, 0.48);
            let electrical_falloff = exp(-length(point - electrical_source) / 11500.0);
            sample_color += vec3<f32>(0.58, 0.70, 1.0) * g.weather2.w
                * electrical_falloff * (0.64 + height_fraction * 0.72);
        }
        let sample_alpha = 1.0 - segment_transmittance;
        let contribution = transmittance * sample_alpha;
        if front_distance <= 0.0 {
            front_distance = distance_m;
        }
        radiance += contribution * sample_color;
        weighted_distance += contribution * distance_m;
        transmittance *= 1.0 - sample_alpha;
        if transmittance < 0.025 {
            break;
        }
    }
    let alpha = 1.0 - transmittance;
    if alpha <= 0.001 {
        return CloudValue(
            vec4<f32>(0.0),
            0.0,
            (segment.x + segment.y) * 0.5, 0.0);
    }
    return CloudValue(
        vec4<f32>(radiance / alpha, alpha),
        front_distance,
        weighted_distance / alpha, 0.0);
}

/// A separate optically thin ice-cloud sheet. Keeping cirrus out of the dense
/// cumulus marcher mirrors the reference pipeline and lets high-cloud coverage
/// add long, wind-skewed structure without filling the lower volume with haze.
fn cirrus_cloud(rd: vec3<f32>, center: vec3<f32>, ground_r: f32,
        opaque_distance: f32, observer_alt: f32) -> CloudValue {
    let high_coverage = clamp(g.weather5.z, 0.0, 1.0);
    if high_coverage <= 0.015 || observer_alt > 180000.0 {
        return CloudValue(vec4<f32>(0.0), 0.0, 0.0, 0.0);
    }
    let lower_top = clamp(g.weather3.y, 1200.0, 15000.0);
    let cirrus_height = clamp(lower_top + 3600.0, 7200.0, 12500.0);
    let shell = ray_sphere(vec3<f32>(0.0), rd, center, ground_r + cirrus_height);
    let distance_m = select(shell.y, shell.x, shell.x > 0.0);
    if shell.y <= shell.x || distance_m <= 0.0 || distance_m >= opaque_distance {
        return CloudValue(vec4<f32>(0.0), 0.0, 0.0, 0.0);
    }
    let point = rd * distance_m;
    let direction = normalize(point - center);
    let seed = g.weather2.y;
    let drift = g.weather2.x * 0.000035;
    let wind_skew = vec3<f32>(
        g.wind_upper.x * drift,
        g.wind_upper.y * drift,
        (g.wind_upper.x - g.wind_upper.y) * drift * 0.4,
    );
    let broad = cloud_value_noise(
        direction * vec3<f32>(2100.0, 4300.0, 2750.0)
            + wind_skew + vec3<f32>(seed * 17.0, seed * 31.0, seed * 11.0),
    );
    let fibers = cloud_value_noise(
        direction * vec3<f32>(5700.0, 9200.0, 6400.0)
            - wind_skew * 1.7 + vec3<f32>(seed * 43.0, seed * 19.0, seed * 29.0),
    );
    let breakup = cloud_value_noise(
        direction * vec3<f32>(9800.0, 7200.0, 11500.0)
            + wind_skew * 0.7 + vec3<f32>(seed * 7.0, seed * 53.0, seed * 37.0),
    );
    let field = broad * 0.68 + fibers * 0.24 + breakup * 0.08;
    let threshold = mix(0.74, 0.54, high_coverage);
    let wisps = smoothstep(threshold, threshold + 0.16, field)
        * mix(0.42, 1.0, smoothstep(0.26, 0.74, fibers))
        * mix(0.65, 1.0, breakup);
    let grazing_fade = smoothstep(-0.025, 0.055, dot(direction, rd));
    let alpha = wisps * high_coverage * 0.30 * grazing_fade
        * (1.0 - smoothstep(120000.0, 180000.0, observer_alt));
    if alpha <= 0.001 {
        return CloudValue(vec4<f32>(0.0), 0.0, distance_m, 0.0);
    }
    let sun_dir = normalize(g.sun_dir.xyz);
    let sun_height = dot(direction, sun_dir);
    let daylight = smoothstep(-0.12, 0.18, sun_height);
    let forward = pow(max(dot(rd, sun_dir), 0.0), 10.0);
    let ice_color = mix(
        vec3<f32>(0.10, 0.14, 0.24),
        vec3<f32>(0.82, 0.87, 0.94) + vec3<f32>(0.18, 0.13, 0.08) * forward,
        daylight,
    );
    return CloudValue(vec4<f32>(ice_color, alpha), distance_m, distance_m, 0.0);
}

fn encode_cloud_distance(distance_m: f32) -> f32 {
    if distance_m <= 0.0 {
        return 0.0;
    }
    return log2(distance_m + 1.0);
}

fn decode_cloud_distance(encoded: f32) -> f32 {
    if encoded <= 0.0 {
        return 0.0;
    }
    return exp2(encoded) - 1.0;
}

fn encode_scene_distance(distance_m: f32) -> f32 {
    if distance_m >= 1e20 {
        return 0.0;
    }
    return encode_cloud_distance(distance_m);
}

fn decode_scene_distance(encoded: f32) -> f32 {
    if encoded <= 0.0 {
        return 1e30;
    }
    return decode_cloud_distance(encoded);
}

fn nearest_positive(a: f32, b: f32) -> f32 {
    if a <= 0.0 {
        return b;
    }
    if b <= 0.0 {
        return a;
    }
    return min(a, b);
}

fn scene_distance_ndc(ndc: vec2<f32>) -> f32 {
    let dimensions = vec2<i32>(textureDimensions(depth_tex));
    let uv = ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let coordinate = clamp(
        vec2<i32>(uv * vec2<f32>(dimensions)),
        vec2<i32>(0),
        dimensions - 1,
    );
    let depth = textureLoad(depth_tex, coordinate, 0);
    if depth <= 0.0 {
        return 1e30;
    }
    let surface = g.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
    return cloud_common_scene_distance(surface);
}

/// A bounded detailed volume covers normal flight altitudes. High above the
/// weather, the same column field hands off to a band-limited aerial
/// representation without ray-marching every climatological altitude layer.
fn integrate_clouds(in: VsOut, opaque_distance: f32) -> CloudValue {
    let far = g.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let rd = normalize(far.xyz / far.w);
    let center = g.planet_center.xyz;
    let ground_r = g.planet_center.w;
    let observer_alt = max(length(center) - ground_r, 0.0);
    let ground = ray_sphere(vec3<f32>(0.0), rd, center, ground_r);
    let base_m = clamp(g.weather3.x, 300.0, 9000.0);
    let top_m = clamp(max(g.weather3.y, base_m + 600.0), base_m + 600.0, 15000.0);

    var orbital = CloudValue(vec4<f32>(0.0), 0.0, 0.0, 0.0);
    let orbital_blend = smoothstep(65000.0, 130000.0, observer_alt)
        * (1.0 - smoothstep(45000000.0, 90000000.0, observer_alt));
    let aerial_height = mix(base_m, top_m, 0.62);
    let shell = ray_sphere(vec3<f32>(0.0), rd, center, ground_r + aerial_height);
    let cloud_t = select(shell.y, shell.x, shell.x > 0.0);
    let shell_valid = shell.y > shell.x && cloud_t > 0.0;
    let terrain_before_shell = ground.y > ground.x && ground.x > 0.0
        && ground.x < cloud_t;
    if (orbital_blend > 0.001 && shell_valid && !terrain_before_shell) {
        orbital.representative_distance = cloud_t;
        let point = rd * cloud_t;
        let direction = normalize(point - center);
        let footprint = length(fwidth(direction));
        let mass = cloud_noise_filtered(
            direction,
            g.weather2.x,
            climate_mid_wind(direction),
            footprint,
        );
        let coverage_front = orbital_cloud_coverage(direction, g.weather2.x);
        let coverage = coverage_front.x;
        let mesoscale_weight = cloud_band_weight(footprint, 38.0);
        let mesoscale_raw = cloud_value_noise(
            direction * 38.0
                + vec3<f32>(g.weather2.y * 13.0, g.weather2.y * 29.0, g.weather2.y * 19.0),
        );
        let mesoscale = mix(0.5, mesoscale_raw, mesoscale_weight);
        let detail_weight = cloud_band_weight(footprint, 126.0);
        let breakup_raw = cloud_value_noise(
            direction * 126.0
                + vec3<f32>(g.weather2.y * 31.0, g.weather2.y * 17.0, g.weather2.y * 43.0),
        );
        let breakup = mix(0.5, breakup_raw, detail_weight);
        let effective_coverage = clamp(mix(coverage, g.weather1.z, 0.75), 0.0, 1.0);
        let threshold = mix(0.68, 0.48, effective_coverage);
        let shaped_mass = mass * 0.58 + mesoscale * 0.29 + breakup * 0.13
            + coverage_front.y * 0.06;
        let global_density = smoothstep(threshold, threshold + 0.14, shaped_mass);
        // Preserve the actual local weather system through the aerial LOD.
        // The broad globe field takes over only once kilometre-scale local
        // detail is genuinely too small to resolve.
        let local_column = local_cloud_column(
            direction, ground_r, base_m, top_m, g.weather5.x,
        );
        let aerial_lod = smoothstep(45000.0, 130000.0, observer_alt);
        let density = mix(local_column, global_density, aerial_lod);
        let cloud_alpha = density * (0.74 + 0.26 * breakup)
            * orbital_blend * (0.24 + 0.42 * effective_coverage);

        let sun_dir = normalize(g.sun_dir.xyz);
        let moon_dir = normalize(g.moon_dir.xyz);
        let sun_height = dot(direction, sun_dir);
        let day = smoothstep(-0.10, 0.20, sun_height);
        let forward = pow(max(dot(rd, sun_dir), 0.0), 6.0);
        let direct_sun = solar_transmittance(sun_height, aerial_height);
        let direct_peak = max(direct_sun.r, max(direct_sun.g, direct_sun.b));
        let direct_color = direct_sun / max(direct_peak, 0.025);
        let warm = direct_color * direct_peak * (0.24 + 0.76 * forward)
            * smoothstep(-0.14, 0.04, sun_height)
            * (1.0 - smoothstep(0.18, 0.34, sun_height));
        let moon = vec3<f32>(0.12, 0.17, 0.29)
            * max(dot(direction, moon_dir), 0.0) * g.moon_dir.w;
        let storm = select(0.0, 1.0, g.weather3.w > 1.5);
        let sunlit = mix(
            vec3<f32>(0.76, 0.78, 0.79),
            vec3<f32>(0.54, 0.58, 0.63),
            smoothstep(0.18, 0.92, density),
        );
        var cloud_color = mix(vec3<f32>(0.035, 0.050, 0.085), sunlit, day);
        cloud_color = mix(cloud_color, vec3<f32>(0.31, 0.33, 0.37), storm * 0.18);
        cloud_color *= 0.78 + 0.26 * breakup;
        let underside = smoothstep(0.28, 0.92, density);
        cloud_color += warm * (0.36 + 0.54 * underside) + moon;
        if g.weather2.w > 0.001 {
            let strike_ground = lightning_strike_ground();
            let strike_up = normalize(strike_ground - center);
            let electrical_source = strike_ground + strike_up * mix(base_m, top_m, 0.48);
            let electrical_falloff = exp(-length(point - electrical_source) / 28000.0);
            cloud_color += vec3<f32>(0.52, 0.63, 0.94) * g.weather2.w
                * electrical_falloff;
        }
        orbital.rgba = vec4<f32>(cloud_color, clamp(cloud_alpha, 0.0, 0.88));
        if orbital.rgba.a > 0.001 {
            orbital.front_distance = cloud_t;
        }
    }

    var local = CloudValue(vec4<f32>(0.0), 0.0, 0.0, 0.0);
    let local_blend = 1.0 - smoothstep(90000.0, 170000.0, observer_alt);
    if local_blend > 0.001 {
        local = local_cloud_volume(
            rd, center, ground_r, observer_alt, opaque_distance, in.pos.xy,
        );
        local.rgba.a *= local_blend;
    }

    let adjacent_clip = g.inv_view_proj * vec4<f32>(
        in.ndc + vec2<f32>(0.0, 4.0 / max(g.viewport.y, 1.0)), 0.5, 1.0);
    let angular_footprint = length(normalize(adjacent_clip.xyz / adjacent_clip.w) - rd);
    let distant = cloud_common_far_sample(-center, rd,
        local_cloud_context(ground_r, base_m, top_m, g.weather5.x, 0.0),
        normalize(g.sun_dir.xyz), opaque_distance, angular_footprint, cloud_shape_tex, cloud_shape_sampler);
    if distant.alpha > 0.001 {
        let up = normalize(distant.position);
        let direct = solar_transmittance(dot(up, normalize(g.sun_dir.xyz)),
            max(length(distant.position) - ground_r, 0.0));
        let ambient = cloud_common_ambient(dot(up, normalize(g.sun_dir.xyz)),
            dot(up, normalize(g.moon_dir.xyz)), g.moon_dir.w, dust_optical_load());
        let color = apply_aerial_perspective(cloud_common_lighting(dot(rd, normalize(g.sun_dir.xyz)), distant.density,
            0.62, distant.sun_visibility, 0.75, direct, ambient), rd * distant.distance, 0.0);
        let weight = distant.alpha * (1.0 - local.rgba.a);
        let alpha = local.rgba.a + weight;
        local.rgba = vec4<f32>((local.rgba.rgb * local.rgba.a + color * weight)
            / max(alpha, 0.001), alpha);
        local.front_distance = nearest_positive(local.front_distance, distant.distance);
        local.representative_distance = mix(local.representative_distance,
            distant.distance, weight / max(alpha, 0.001));
    }
    let cirrus = cirrus_cloud(rd, center, ground_r, opaque_distance, observer_alt);

    let after_local = 1.0 - local.rgba.a;
    let cirrus_weight = cirrus.rgba.a * after_local;
    let orbital_weight = orbital.rgba.a * after_local * (1.0 - cirrus.rgba.a);
    let alpha = local.rgba.a + cirrus_weight + orbital_weight;
    if alpha <= 0.001 {
        let representative = select(
            cirrus.representative_distance,
            local.representative_distance,
            local.representative_distance > 0.0,
        );
        return CloudValue(vec4<f32>(0.0), 0.0, representative, 0.0);
    }
    let color = (
        local.rgba.rgb * local.rgba.a
        + cirrus.rgba.rgb * cirrus_weight
        + orbital.rgba.rgb * orbital_weight
    ) / alpha;
    let representative = (
        local.representative_distance * local.rgba.a
        + cirrus.representative_distance * cirrus_weight
        + orbital.representative_distance * orbital_weight
    ) / alpha;
    return CloudValue(
        vec4<f32>(color, alpha),
        nearest_positive(
            nearest_positive(local.front_distance, cirrus.front_distance),
            orbital.front_distance,
        ),
        representative, 0.0);
}

struct HistoryValue {
    rgba: vec4<f32>,
    metadata: vec4<f32>,
    valid: f32,
    motion_pixels: f32,
};

fn previous_cloud_point(point: vec3<f32>) -> vec3<f32> {
    // The renderer advances the drift by the camera's wind over the same
    // elapsed time, so this step matches the field's motion exactly.
    let wind_ecef = cloud_common_wind(normalize(-g.planet_center.xyz), g.wind_low.xyz);
    return point + g.camera_to_prev.xyz - wind_ecef * g.camera_to_prev.w;
}

fn reproject_cloud_history(rd: vec3<f32>, representative_distance: f32,
        current_scene_distance: f32) -> HistoryValue {
    if g.temporal.x < 0.5 || representative_distance <= 0.0 {
        return HistoryValue(vec4<f32>(0.0), vec4<f32>(0.0), 0.0, 0.0);
    }
    let point_previous = previous_cloud_point(rd * representative_distance);
    let clip = g.prev_view_proj * vec4<f32>(point_previous, 1.0);
    if clip.w <= 1e-5 {
        return HistoryValue(vec4<f32>(0.0), vec4<f32>(0.0), 0.0, 0.0);
    }
    let ndc = clip.xy / clip.w;
    if any(abs(ndc) > vec2<f32>(1.04)) {
        return HistoryValue(vec4<f32>(0.0), vec4<f32>(0.0), 0.0, 0.0);
    }
    let uv = ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let previous_distance = length(point_previous);
    let tolerance = max(800.0, previous_distance * 0.14);
    // Distance along a ray has no effect on reprojection for rotation-only or
    // near-stationary camera motion. The volume's stochastic integration can
    // move its opacity-weighted representative point by kilometres on a long
    // horizon ray; rejecting that otherwise exact history preserved stipple.
    let distance_independent = length(g.camera_to_prev.xyz) < 5.0;
    let metadata = textureSampleLevel(
        cloud_history_meta, cloud_history_sampler, uv, 0.0,
    );
    let history_distance = decode_cloud_distance(metadata.y);
    let history_scene_distance = decode_scene_distance(metadata.z);
    let current_has_terrain = current_scene_distance < 1e20;
    let history_has_terrain = history_scene_distance < 1e20;
    if current_has_terrain != history_has_terrain {
        return HistoryValue(vec4<f32>(0.0), vec4<f32>(0.0), 0.0, 0.0);
    }
    if current_has_terrain {
        let scene_tolerance = max(
            40.0,
            min(current_scene_distance, history_scene_distance) * 0.08,
        );
        if abs(history_scene_distance - current_scene_distance) > scene_tolerance {
            return HistoryValue(vec4<f32>(0.0), vec4<f32>(0.0), 0.0, 0.0);
        }
    }
    if history_distance <= 0.0
            || (!distance_independent
                && abs(history_distance - previous_distance) > tolerance) {
        return HistoryValue(vec4<f32>(0.0), vec4<f32>(0.0), 0.0, 0.0);
    }
    let current_clip = g.view_proj * vec4<f32>(rd * representative_distance, 1.0);
    let current_ndc = current_clip.xy / max(current_clip.w, 1e-5);
    let motion_pixels = length((ndc - current_ndc) * g.viewport.xy * 0.5);
    return HistoryValue(textureSampleLevel(
        cloud_history_color, cloud_history_sampler, uv, 0.0,
    ), metadata, 1.0, motion_pixels);
}

fn accumulate_cloud_history(rd: vec3<f32>, current: CloudValue,
        scene_distance: f32) -> CloudValue {
    let observer_altitude = max(length(g.planet_center.xyz) - g.planet_center.w, 0.0);
    if g.weather5.x <= 0.001 && g.weather5.z <= 0.015 && observer_altitude < 65000.0 {
        var clear_weather = current;
        clear_weather.history_samples = 1.0;
        return clear_weather;
    }
    var history = reproject_cloud_history(
        rd, current.representative_distance, scene_distance,
    );
    if history.valid < 0.5 {
        var first = current;
        first.history_samples = 1.0;
        return first;
    }
    let stable_history = cloud_common_history_stability(history.motion_pixels);
    // Fast simulated time moves the Sun and weather quickly; a smaller sample
    // cap lets history follow gradual change instead of being reset.
    history.metadata.w = min(history.metadata.w, g.cloud_drift.w);
    let observer_up = normalize(-g.planet_center.xyz);
    let solar_sine = dot(observer_up, normalize(g.sun_dir.xyz));
    // The same small Monte-Carlo alpha/radiance difference that is hidden by
    // daylight becomes a conspicuous blue-black pulse after night exposure
    // and tone mapping. Give low-light history a wider acceptance window and
    // more weight; geometry changes still escape through the existing alpha,
    // depth and disocclusion rejection paths.
    let night_stability = 1.0 - smoothstep(-0.10, 0.08, solar_sine);
    var history_rgba = history.rgba;
    var alpha_window = select(
        0.22,
        select(0.17, 0.11, g.surface.w > 1.5),
        g.surface.w > 0.5,
    );
    alpha_window *= mix(1.0, 1.65, night_stability);
    // A valid subpixel reprojection sees the same stochastic integral. Clip
    // moving history against the current sample, but let stable history
    // average isolated hit/miss noise instead of repeatedly reintroducing it.
    alpha_window = mix(alpha_window, 1.0, stable_history);
    history_rgba.a = clamp(
        history_rgba.a,
        max(current.rgba.a - alpha_window, 0.0),
        min(current.rgba.a + alpha_window, 1.0),
    );
    // A texel receives a fresh march every sixteen frames. Feedback values
    // tuned for a full per-frame refresh make the first stochastic sample
    // linger for seconds, so interleaved history deliberately converges faster.
    var base_feedback = select(
        0.62,
        select(0.70, 0.76, g.surface.w > 1.5),
        g.surface.w > 0.5,
    );
    let night_feedback = select(
        0.86,
        select(0.92, 0.94, g.surface.w > 1.5),
        g.surface.w > 0.5,
    );
    base_feedback = mix(base_feedback, night_feedback, night_stability);
    var feedback = cloud_common_history_feedback(history.metadata.w,
        base_feedback, stable_history) * mix(
        1.0 - smoothstep(0.16, 0.62, abs(history_rgba.a - current.rgba.a)),
        1.0, stable_history);
    // Never average a millisecond-scale discharge into several seconds of
    // cloud history; the prior value also keeps the first dark frame clean.
    let electrical_transition = max(g.weather2.w, g.temporal.z);
    feedback *= 1.0 - smoothstep(0.002, 0.035, electrical_transition);
    if current.rgba.a <= 0.001 {
        let empty_history_cap = select(0.12, 0.065, g.surface.w > 1.5);
        let empty_feedback = select(0.45, 0.86, g.surface.w > 1.5);
        history_rgba.a = min(history_rgba.a, mix(empty_history_cap, 1.0, stable_history));
        feedback *= mix(empty_feedback, 1.0, stable_history);
    }
    let current_premultiplied = current.rgba.rgb * current.rgba.a;
    let history_premultiplied = history_rgba.rgb * history_rgba.a;
    let alpha = mix(current.rgba.a, history_rgba.a, feedback);
    let premultiplied = mix(current_premultiplied, history_premultiplied, feedback);
    var color = vec3<f32>(0.0);
    if alpha > 0.001 {
        color = premultiplied / alpha;
    }
    var front_distance = current.front_distance;
    if front_distance <= 0.0 && alpha > 0.001 {
        front_distance = current.representative_distance;
    }
    return CloudValue(
        vec4<f32>(color, alpha),
        front_distance,
        current.representative_distance,
        cloud_common_history_count(history.metadata.w,
            stable_history * (1.0 - smoothstep(0.002, 0.035, electrical_transition))));
}

struct CloudIntegrateOut {
    @location(0) color: vec4<f32>,
    @location(1) metadata: vec4<f32>,
};

fn cloud_interleave_phase() -> vec2<u32> {
    let phase = u32(g.temporal.y) & 15u;
    // Blue-noise-like ordering avoids a visible left-to-right/top-to-bottom
    // refresh sweep during the first sixteen frames after a history reset.
    let sequence = array<vec2<u32>, 16>(
        vec2<u32>(0u, 0u), vec2<u32>(2u, 2u),
        vec2<u32>(2u, 0u), vec2<u32>(0u, 2u),
        vec2<u32>(1u, 3u), vec2<u32>(3u, 1u),
        vec2<u32>(3u, 3u), vec2<u32>(1u, 1u),
        vec2<u32>(1u, 0u), vec2<u32>(3u, 2u),
        vec2<u32>(3u, 0u), vec2<u32>(1u, 2u),
        vec2<u32>(0u, 3u), vec2<u32>(2u, 1u),
        vec2<u32>(0u, 1u), vec2<u32>(2u, 3u),
    );
    return sequence[phase];
}

/// March only one pixel in each 4x4 half-resolution history tile. The update
/// target is therefore 1/64 of the full viewport, funding a substantially
/// deeper volume integration while the resolve pass carries the other phases.
@fragment
fn fs_cloud_march_interleaved(in: VsOut) -> CloudIntegrateOut {
    let update_coordinate = vec2<u32>(floor(in.pos.xy));
    let history_coordinate = update_coordinate * 4u + cloud_interleave_phase();
    let history_pixel = vec2<f32>(history_coordinate) + vec2<f32>(0.5);
    let full_pixel = history_pixel * 2.0;
    if any(full_pixel >= g.viewport.xy) {
        discard;
    }
    let sample_ndc = full_pixel / max(g.viewport.xy, vec2<f32>(1.0))
        * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    var ray_in = in;
    ray_in.pos = vec4<f32>(history_pixel, in.pos.zw);
    ray_in.ndc = sample_ndc;
    let far = g.inv_view_proj * vec4<f32>(sample_ndc, 0.5, 1.0);
    let rd = normalize(far.xyz / far.w);
    let opaque_distance = scene_distance_ndc(sample_ndc);
    let cloud = accumulate_cloud_history(
        rd, integrate_clouds(ray_in, opaque_distance), opaque_distance,
    );
    let conservative_front = select(
        cloud.representative_distance,
        cloud.front_distance,
        cloud.front_distance > 0.0,
    );
    return CloudIntegrateOut(
        cloud.rgba,
        vec4<f32>(
            encode_cloud_distance(conservative_front),
            encode_cloud_distance(cloud.representative_distance),
            encode_scene_distance(opaque_distance),
            cloud.history_samples,
        ),
    );
}

/// Produce a complete, deliberately soft current-frame cloud image from the
/// one-eighth-resolution march target. Every update texel is a valid ray over
/// the viewport; the 4x4 phase only chooses the exact half-resolution history
/// position refreshed by that ray. Sampling this coarse image is therefore a
/// safe fallback whenever temporal history is unavailable or disoccluded.
///
/// Returning this instead of transparent black is essential: an empty return
/// exposes the active 4x4 phase as a dot matrix until all sixteen slots have
/// accumulated. The fallback is progressively replaced by exact history over
/// the normal interleave cycle.
fn coarse_cloud_fallback(coordinate: vec2<i32>, phase: vec2<i32>,
        opaque_distance: f32) -> CloudIntegrateOut {
    let update_dimensions = vec2<i32>(textureDimensions(cloud_update_color));
    let update_position = (
        vec2<f32>(coordinate) - vec2<f32>(phase)
    ) / 4.0;
    let update_uv = (update_position + vec2<f32>(0.5))
        / vec2<f32>(update_dimensions);
    let color = textureSampleLevel(
        cloud_update_color, cloud_update_sampler, update_uv, 0.0,
    );
    let nearest_update = clamp(
        vec2<i32>(floor(update_position + vec2<f32>(0.5))),
        vec2<i32>(0),
        update_dimensions - vec2<i32>(1),
    );
    let metadata = textureLoad(cloud_update_meta, nearest_update, 0);
    return CloudIntegrateOut(
        color,
        vec4<f32>(
            metadata.x,
            metadata.y,
            encode_scene_distance(opaque_distance),
            0.0,
        ),
    );
}

/// Resolve the sparse march into dense half-resolution history. Newly marched
/// pixels are copied exactly; the other fifteen phases are motion-, wind-, and
/// depth-reprojected so a camera move cannot leave a screen-space lattice.
@fragment
fn fs_cloud_resolve_interleaved(in: VsOut) -> CloudIntegrateOut {
    let coordinate = vec2<i32>(floor(in.pos.xy));
    let phase = vec2<i32>(cloud_interleave_phase());
    let opaque_distance = scene_distance_ndc(in.ndc);
    if all((coordinate & vec2<i32>(3)) == phase) {
        let update_coordinate = coordinate / 4;
        return CloudIntegrateOut(
            textureLoad(cloud_update_color, update_coordinate, 0),
            textureLoad(cloud_update_meta, update_coordinate, 0),
        );
    }

    let prior_metadata = textureLoad(cloud_history_meta, coordinate, 0);
    let representative_distance = decode_cloud_distance(prior_metadata.y);
    let far = g.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let rd = normalize(far.xyz / far.w);
    let history = reproject_cloud_history(
        rd, representative_distance, opaque_distance,
    );
    if history.valid < 0.5 {
        return coarse_cloud_fallback(coordinate, phase, opaque_distance);
    }

    let old_representative = decode_cloud_distance(history.metadata.y);
    let old_front = decode_cloud_distance(history.metadata.x);
    let distance_ratio = representative_distance / max(old_representative, 1.0);
    let current_front = select(
        representative_distance,
        old_front * distance_ratio,
        old_front > 0.0,
    );
    return CloudIntegrateOut(
        history.rgba,
        vec4<f32>(
            encode_cloud_distance(current_front),
            encode_cloud_distance(representative_distance),
            encode_scene_distance(opaque_distance),
            history.metadata.w,
        ),
    );
}

fn scene_distance(in: VsOut) -> f32 {
    return scene_distance_ndc(in.ndc);
}

fn cloud_front_distance(encoded_distance: f32) -> f32 {
    return decode_cloud_distance(encoded_distance);
}

fn valid_cloud_front(metadata: vec4<f32>, alpha: f32) -> f32 {
    if alpha <= 0.001 || metadata.y <= 0.0 {
        return 0.0;
    }
    return cloud_front_distance(metadata.x);
}

fn reconstruction_depth_weight(encoded_sample_depth: f32,
        opaque_distance: f32) -> f32 {
    let terrain_pixel = opaque_distance < 1e20;
    let sample_has_terrain = encoded_sample_depth > 0.0;
    if terrain_pixel != sample_has_terrain {
        return 0.0;
    }
    if !terrain_pixel {
        return 1.0;
    }
    let sample_distance = decode_scene_distance(encoded_sample_depth);
    let difference = abs(sample_distance - opaque_distance);
    let scale = min(sample_distance, opaque_distance);
    let exact_radius = max(6.0, scale * 0.012);
    let reject_radius = max(45.0, scale * 0.10);
    return 1.0 - smoothstep(exact_radius, reject_radius, difference);
}

/// Reconstruct structured cloud only from half-resolution rays that terminated
/// at compatible geometry. Terrain weather extinction is evaluated separately
/// at full resolution in shader.wgsl, so cloud taps never need to stand in for
/// mountain fog or cross a valley/ridge depth discontinuity.
@fragment
fn fs_cloud_composite(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.pos.xy / max(g.viewport.xy, vec2<f32>(1.0));
    let dimensions = vec2<i32>(textureDimensions(cloud_history_color));
    let texel_position = uv * vec2<f32>(dimensions) - vec2<f32>(0.5);
    let opaque_distance = scene_distance(in);
    var weight_sum = 0.0;
    var alpha_sum = 0.0;
    var premultiplied_sum = vec3<f32>(0.0);
    var front_distance = 0.0;
    if g.surface.w > 1.5 {
        let center_coordinate = vec2<i32>(floor(texel_position + vec2<f32>(0.5)));
        let offsets = array<vec2<i32>, 9>(
            vec2<i32>(-1, -1), vec2<i32>(0, -1), vec2<i32>(1, -1),
            vec2<i32>(-1, 0), vec2<i32>(0, 0), vec2<i32>(1, 0),
            vec2<i32>(-1, 1), vec2<i32>(0, 1), vec2<i32>(1, 1),
        );
        for (var index = 0u; index < 9u; index += 1u) {
            let coordinate = clamp(
                center_coordinate + offsets[index],
                vec2<i32>(0),
                dimensions - vec2<i32>(1),
            );
            let cloud_sample = textureLoad(cloud_history_color, coordinate, 0);
            let metadata = textureLoad(cloud_history_meta, coordinate, 0);
            let sample_delta = vec2<f32>(coordinate) - texel_position;
            let spatial_weight = exp(-dot(sample_delta, sample_delta) * 0.82);
            let weight = spatial_weight
                * reconstruction_depth_weight(metadata.z, opaque_distance);
            weight_sum += weight;
            alpha_sum += cloud_sample.a * weight;
            premultiplied_sum += cloud_sample.rgb * cloud_sample.a * weight;
            if weight * cloud_sample.a > 0.012 {
                front_distance = nearest_positive(
                    front_distance,
                    valid_cloud_front(metadata, cloud_sample.a),
                );
            }
        }
    } else {
        let base = vec2<i32>(floor(texel_position));
        let fraction = fract(texel_position);
        let coordinates = array<vec2<i32>, 4>(
            base,
            base + vec2<i32>(1, 0),
            base + vec2<i32>(0, 1),
            base + vec2<i32>(1, 1),
        );
        let spatial_weights = array<f32, 4>(
            (1.0 - fraction.x) * (1.0 - fraction.y),
            fraction.x * (1.0 - fraction.y),
            (1.0 - fraction.x) * fraction.y,
            fraction.x * fraction.y,
        );
        for (var index = 0u; index < 4u; index += 1u) {
            let coordinate = clamp(
                coordinates[index],
                vec2<i32>(0),
                dimensions - vec2<i32>(1),
            );
            let cloud_sample = textureLoad(cloud_history_color, coordinate, 0);
            let metadata = textureLoad(cloud_history_meta, coordinate, 0);
            let weight = spatial_weights[index]
                * reconstruction_depth_weight(metadata.z, opaque_distance);
            weight_sum += weight;
            alpha_sum += cloud_sample.a * weight;
            premultiplied_sum += cloud_sample.rgb * cloud_sample.a * weight;
            if weight * cloud_sample.a > 0.012 {
                front_distance = nearest_positive(
                    front_distance,
                    valid_cloud_front(metadata, cloud_sample.a),
                );
            }
        }
    }
    if weight_sum <= 0.001 {
        discard;
    }
    let alpha = alpha_sum / weight_sum;
    if alpha <= 0.001 {
        discard;
    }
    let sample = vec4<f32>(
        premultiplied_sum / max(alpha_sum, 0.001),
        alpha,
    );
    let observer_altitude = max(length(g.planet_center.xyz) - g.planet_center.w, 0.0);
    let cloud_base = clamp(g.weather3.x, 300.0, 9000.0);
    let cloud_top = clamp(
        max(g.weather3.y, cloud_base + 600.0),
        cloud_base + 600.0,
        15000.0,
    );
    let orbital_depth_mode = observer_altitude
        > max(cloud_top + 20000.0, 30000.0);
    if orbital_depth_mode {
        // Only true stratospheric/orbital views use the coarse guard. Alpine
        // cameras can be a kilometre above a low deck while mountains still
        // pierce or stand in front of it; treating those views as orbital
        // made the half-resolution cloud buffer leak through terrain and
        // created a constant-distance horizontal cutoff.
        let near_occluder_limit = clamp(
            (observer_altitude - cloud_top) * 0.35,
            4000.0,
            400000.0,
        );
        if opaque_distance < near_occluder_limit {
            discard;
        }
    } else if !cloud_common_depth_visible(opaque_distance, front_distance) {
        discard;
    }
    return sample;
}

/// Camera-local bounded precipitation. A distinct entry point gives the pass
/// its own timing query and lets clear/dry conditions skip all useful work.
/// Motion is the projection of a real ENU velocity, so rotating the camera
/// cannot leave gravity pinned to the screen's vertical axis.
fn projected_velocity(velocity: vec3<f32>, forward: vec3<f32>) -> vec2<f32> {
    let anchor = forward * 28.0;
    let a = g.view_proj * vec4<f32>(anchor, 1.0);
    let b = g.view_proj * vec4<f32>(anchor + velocity, 1.0);
    let ndc_a = a.xy / max(abs(a.w), 1e-5);
    let ndc_b = b.xy / max(abs(b.w), 1e-5);
    // Fragment positions have a top-left origin, opposite clip-space Y.
    return (ndc_b - ndc_a) * vec2<f32>(0.5, -0.5) * g.viewport.xy;
}

fn falling_motion(velocity: vec3<f32>, forward: vec3<f32>) -> vec2<f32> {
    var motion = projected_velocity(velocity, forward);
    // Close rain can otherwise cross more than half a procedural streak in
    // one frame and appear to reverse direction. Keep it fast but below that
    // temporal-aliasing range.
    let speed = length(motion);
    if speed > 360.0 {
        motion *= 360.0 / speed;
    }
    // In steep views gravity projects almost along the view ray, allowing
    // projected horizontal wind to point toward the top of the screen. The
    // particles are still falling in world space, so retain a small visible
    // downward component while preserving their lateral wind slant.
    motion.y = max(motion.y, 12.0);
    return motion;
}

fn rain_layer(pixel: vec2<f32>, time: f32, motion: vec2<f32>, layer: f32,
        activity: f32, surface_distance: f32) -> f32 {
    let depth = layer / 3.0;
    let proximity = 1.0 - depth;
    let depth_motion = mix(0.34, 1.0, proximity);
    let apparent_speed = length(motion) * depth_motion;
    var direction = vec2<f32>(0.0, 1.0);
    if apparent_speed > 0.5 {
        direction = normalize(motion);
    }
    let across = vec2<f32>(-direction.y, direction.x);
    let advected = pixel - motion * time * depth_motion;
    let oriented = vec2<f32>(dot(advected, across), dot(advected, direction));
    let spacing = mix(13.0, 27.0, proximity) * mix(1.08, 0.78, activity);
    let foreshortening = smoothstep(8.0, 430.0, apparent_speed);
    let streak_length = mix(5.0, mix(31.0, 61.0, activity), foreshortening)
        * mix(0.62, 1.18, proximity);
    let q = oriented;
    let cell = floor(vec2<f32>(q.x / spacing, q.y / streak_length));
    let random = cloud_hash(vec3<f32>(cell, layer * 37.0 + 11.0));
    let local = fract(vec2<f32>(q.x / spacing, q.y / streak_length));
    let center_x = 0.16 + random * 0.68;
    // When falling nearly along the view ray, perspective motion collapses
    // into growing droplets instead of falsely reverting to vertical lines.
    let life = fract(time * (0.48 + activity * 0.34) + random + layer * 0.173);
    let view_axis_growth = mix(0.72, 1.58, life)
        * (1.0 - smoothstep(18.0, 170.0, apparent_speed));
    let width = mix(0.035, 0.105, proximity) * mix(0.8, 1.2, activity)
        * max(view_axis_growth, 1.0);
    let line = 1.0 - smoothstep(width * 0.38, width, abs(local.x - center_x));
    let segment = smoothstep(0.04, 0.18, local.y)
        * (1.0 - smoothstep(0.70, 0.98, local.y));
    let exists = select(0.0, 1.0, random < 0.08 + activity * 0.28);
    let distance_random = cloud_hash(vec3<f32>(cell.yx, layer * 53.0 + 29.0));
    let particle_distance = mix(3.0, 66.0, (layer + distance_random) / 4.0);
    let in_front = select(0.0, 1.0, particle_distance < surface_distance);
    let particle_proximity = 1.0 - particle_distance / 66.0;
    let life_fade = smoothstep(0.0, 0.10, life)
        * (1.0 - smoothstep(0.84, 1.0, life));
    return line * segment * exists * in_front
        * (0.18 + 0.82 * particle_proximity * particle_proximity)
        * mix(life_fade, 1.0, smoothstep(18.0, 170.0, apparent_speed));
}

fn snow_layer(pixel: vec2<f32>, time: f32, motion: vec2<f32>, layer: f32,
        activity: f32, surface_distance: f32) -> f32 {
    let depth = clamp(layer / 4.0, 0.0, 1.0);
    let proximity = 1.0 - depth;
    let spacing = mix(25.0, 43.0, proximity) * mix(1.08, 0.78, activity);
    let drift = vec2<f32>(
        sin(time * 0.9 + layer * 2.1),
        cos(time * 0.7 + layer * 1.3),
    ) * (3.0 + 5.0 * proximity);
    let q = (pixel - time * motion * mix(0.38, 1.0, proximity) + drift) / spacing;
    let cell = floor(q);
    let random_x = cloud_hash(vec3<f32>(cell, layer * 41.0 + 7.0));
    let random_y = cloud_hash(vec3<f32>(cell.yx, layer * 67.0 + 17.0));
    let center = vec2<f32>(0.18 + random_x * 0.64, 0.18 + random_y * 0.64);
    let delta = fract(q) - center;
    let radius = mix(0.055, 0.105, activity)
        * mix(0.72, 1.0, 1.0 - layer * 0.22)
        * mix(0.42, 1.0, 1.0 - random_y);
    let flake = 1.0 - smoothstep(radius * 0.42, radius, length(delta));
    let exists = select(0.0, 1.0, random_x < 0.05 + activity * 0.18);
    let particle_distance = mix(2.5, 52.0, (layer + random_y) / 5.0);
    let in_front = select(0.0, 1.0, particle_distance < surface_distance);
    let particle_proximity = 1.0 - particle_distance / 52.0;
    return flake * exists * in_front
        * (0.22 + 0.78 * particle_proximity * particle_proximity);
}

@fragment
fn fs_precipitation(in: VsOut) -> @location(0) vec4<f32> {
    if (g.surface.w < 0.0 || g.weather0.w <= 0.03) {
        discard;
    }
    let center = g.planet_center.xyz;
    let observer_alt = max(length(center) - g.planet_center.w, 0.0);
    // Precipitation occupies the boundary layer below its parent cloud deck.
    // Capping at 6 km keeps rain out of globe/aircraft views while allowing
    // low cloud bases to bring the local cutoff down further.
    let precipitation_ceiling = clamp(g.weather3.x + 2200.0, 2200.0, 6000.0);
    if (observer_alt >= precipitation_ceiling) {
        discard;
    }
    let altitude_fade = 1.0 - smoothstep(
        precipitation_ceiling * 0.58,
        precipitation_ceiling,
        observer_alt,
    );
    let pixel = in.pos.xy;
    let time = g.weather2.x;
    let rate_mm_h = g.weather0.w;
    // Perceptual response makes light precipitation readable while preserving
    // substantial density growth for a storm.
    let activity = clamp(sqrt(rate_mm_h / 12.0), 0.06, 1.0);

    var surface_distance = 1000000.0;
    let depth = textureLoad(depth_tex, vec2<i32>(pixel), 0);
    if depth > 0.0 {
        let surface = g.inv_view_proj * vec4<f32>(in.ndc, depth, 1.0);
        surface_distance = length(surface.xyz / max(abs(surface.w), 1e-5));
    }

    let view_point = g.inv_view_proj * vec4<f32>(0.0, 0.0, 0.5, 1.0);
    let forward = normalize(view_point.xyz / max(abs(view_point.w), 1e-5));
    let up = normalize(-center);
    var east = cross(vec3<f32>(0.0, 0.0, 1.0), up);
    if dot(east, east) < 1e-5 {
        east = cross(vec3<f32>(1.0, 0.0, 0.0), up);
    }
    east = normalize(east);
    let north = normalize(cross(up, east));
    let wind_velocity = east * g.wind.x + north * g.wind.y + up * g.wind.z;
    let rain_velocity = wind_velocity - up * mix(16.0, 29.0, activity);
    let snow_velocity = wind_velocity * 0.58 - up * mix(2.2, 5.2, activity);
    let hail_velocity = wind_velocity * 0.72 - up * mix(13.0, 24.0, activity);
    let rain_motion = falling_motion(rain_velocity, forward);
    let snow_motion = falling_motion(snow_velocity, forward);
    let hail_motion = falling_motion(hail_velocity, forward);

    var rain_shape = 0.0;
    var snow_shape = 0.0;
    for (var i = 0u; i < 4u; i += 1u) {
        let layer = f32(i);
        rain_shape += rain_layer(
            pixel,
            time,
            rain_motion,
            layer,
            activity,
            surface_distance,
        );
        snow_shape += snow_layer(
            pixel,
            time,
            snow_motion,
            layer,
            activity,
            surface_distance,
        );
    }

    let phase_code = floor(g.weather2.z + 0.01);
    let rain_phase = select(0.0, 1.0, phase_code == 1.0)
        + select(0.0, 0.48, phase_code == 3.0);
    let snow_phase = select(0.0, 1.0, phase_code == 2.0)
        + select(0.0, 0.52, phase_code == 3.0);
    let hail_phase = select(0.0, 1.0, phase_code == 4.0);
    let rain_alpha = rain_shape * rain_phase
        * mix(0.045, 0.13, activity);
    let snow_alpha = snow_shape * snow_phase
        * mix(0.16, 0.36, activity);
    let hail_shape = snow_layer(
        pixel * 1.38,
        time * 4.2,
        hail_motion * 1.38,
        4.0,
        activity,
        surface_distance,
    );
    let hail_alpha = hail_shape * hail_phase * mix(0.28, 0.58, activity);
    var alpha = (rain_alpha + snow_alpha + hail_alpha) * altitude_fade;
    var camera_alpha = 0.0;

    let camera_effects = fract(g.weather2.z) > 0.125;
    if (camera_effects && snow_phase < 0.5) {
        let drop = smoothstep(0.965, 0.998,
            cloud_hash(vec3<f32>(floor(pixel / 18.0), 113.0)));
        camera_alpha = drop * activity * 0.18 * altitude_fade;
        alpha = max(alpha, camera_alpha);
    }
    if (alpha <= 0.001) {
        discard;
    }
    let weighted_color = vec3<f32>(0.22, 0.30, 0.38) * rain_alpha
        + vec3<f32>(0.90, 0.93, 0.96) * snow_alpha
        + vec3<f32>(0.72, 0.84, 0.94) * hail_alpha
        + vec3<f32>(0.40, 0.50, 0.62) * camera_alpha;
    let color = weighted_color
        / max(rain_alpha + snow_alpha + hail_alpha + camera_alpha, 0.001);
    return vec4<f32>(color, clamp(alpha, 0.0, 0.72));
}

fn atmosphere_at_depth(in: VsOut, d: f32) -> vec4<f32> {
    if d > 0.0 && g.weather2.w <= 0.002 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // View ray through this pixel (camera at origin). Lightning is evaluated
    // before the sky-only early out so nearby terrain can correctly occlude a
    // bolt while a foreground channel remains visible against distant ground.
    let far = g.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let rd = normalize(far.xyz / far.w);
    var lightning_bolt = vec3<f32>(0.0);
    if g.weather2.w > 0.002 {
        var distance = 1e30;
        if d > 0.0 {
            distance = cloud_common_scene_distance(g.inv_view_proj * vec4<f32>(in.ndc, d, 1.0));
        }
        lightning_bolt = lightning_bolt_radiance(rd, distance);
    }

    // Sky pixels only: terrain haze is handled as fog in the voxel shader
    // (depth-derived path lengths inherit per-cell chunk bulge and blotch).
    if d > 0.0 {
        return vec4<f32>(lightning_bolt, 1.0);
    }

    let c = g.planet_center.xyz;
    let r_ground = g.planet_center.w;
    let r_atmo = r_ground + ATMO_THICKNESS;
    let sun_dir = normalize(g.sun_dir.xyz);
    let moon_dir = normalize(g.moon_dir.xyz);
    let observer_alt = max(length(c) - r_ground, 0.0);
    let observer_up = normalize(-c);
    let observer_solar_alt = dot(observer_up, sun_dir);
    let observer_twilight = smoothstep(-0.26, -0.01, observer_solar_alt)
        * (1.0 - smoothstep(0.08, 0.30, observer_solar_alt));
    let local_weather_fade = 1.0 - smoothstep(25000.0, 120000.0, observer_alt);
    let humidity = clamp(g.weather0.y, 0.0, 1.0);
    let aerosol = clamp(g.surface.z, 0.0, 1.0);
    let visibility_haze = visibility_haze_factor();
    let cloudiness = clamp(g.weather1.z, 0.0, 1.0);
    let overcast = smoothstep(2.4, 8.5, g.weather5.w)
        * local_weather_fade;
    let dust_load = dust_optical_load() * local_weather_fade;
    let humid_load = humid_optical_load() * local_weather_fade;
    let pollution_load = pollution_optical_load() * local_weather_fade;
    let aerosol_load = clamp(
        aerosol * 0.44 + dust_load * 0.72 + humid_load * 0.68
            + pollution_load * 0.62 + visibility_haze * 0.30,
        0.0, 1.35,
    );

    // The planet itself blocks atmosphere and celestial bodies.
    let ground = ray_sphere(vec3<f32>(0.0), rd, c, r_ground);
    let planet_blocks = ground.y > ground.x && ground.x > 0.0;
    let seg = ray_sphere(vec3<f32>(0.0), rd, c, r_atmo);
    let t0 = max(seg.x, 0.0);
    var t1 = seg.y;
    if planet_blocks {
        t1 = min(t1, ground.x);
    }

    var color = vec3<f32>(0.0);
    if t1 > t0 {
        let midpoint = rd * ((t0 + t1) * 0.5);
        let height_m = max(length(midpoint - c) - r_ground, 0.0);
        let density = exp(-height_m / SCALE_HEIGHT);
        let optical = density * (t1 - t0);
        let up_mid = normalize(midpoint - c);
        let solar_alt_mid = dot(up_mid, sun_dir);
        let day = max(solar_alt_mid, 0.0);
        let twilight_energy = smoothstep(-0.30, 0.035, solar_alt_mid);
        let optical_uv = clamp(1.0 - exp(-optical * 2.0e-5), 0.0, 1.0);
        let lookup = textureSampleLevel(atmosphere_lut, atmosphere_lut_sampler,
            vec2<f32>(clamp(day, 0.0, 1.0), optical_uv), 0.0);
        let sky_light = max(day, twilight_energy * (0.36 + lookup.a * 0.24));
        let amount = lookup.g * sky_light;
        let cosine = dot(rd, sun_dir);
        let horizon = pow(1.0 - abs(dot(rd, up_mid)), 2.4);
        let direct_spectrum = solar_transmittance(solar_alt_mid, height_m);
        // Direct long-path sunlight belongs to the lower sky. Letting it
        // whiten the whole dome was what turned the upper sunset sky mauve;
        // the zenith remains dominated by diffuse Rayleigh scattering.
        let molecular_spectrum = RAYLEIGH
            * (vec3<f32>(0.72) + direct_spectrum * (0.85 * horizon));
        color = molecular_spectrum * rayleigh_phase(cosine) * amount
            * (0.67 + 0.25 * lookup.r);

        let asymmetry = mix(0.69, 0.82,
            clamp(humid_load * 0.72 + dust_load * 0.35, 0.0, 1.0));
        let phase_m = min(mie_phase(cosine, asymmetry) * 0.012, 0.72);
        let neutral_mie = mix(
            vec3<f32>(0.72, 0.80, 0.91),
            vec3<f32>(0.90, 0.91, 0.92),
            humid_load,
        );
        let mie_color = mix(neutral_mie, vec3<f32>(1.00, 0.54, 0.19),
            clamp(dust_load * 1.18, 0.0, 1.0));
        color += mie_color * aerosol_load * amount
            * (0.010 + phase_m * (0.012 + horizon * 0.038));

        let mineral_extinction = clamp(
            dust_load * (0.32 + 0.68 * horizon), 0.0, 0.88);
        color *= mix(vec3<f32>(1.0),
            vec3<f32>(1.0, 0.62, 0.24), mineral_extinction);
        let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
        color = mix(color, vec3<f32>(1.05, 1.00, 0.91) * luminance,
            pollution_load * (0.46 + horizon * 0.30));
        color *= 1.0 - pollution_load * (0.08 + horizon * 0.12);

        let horizon_spectrum = solar_transmittance(
            observer_solar_alt, observer_alt);
        let spectrum_peak = max(horizon_spectrum.r,
            max(horizon_spectrum.g, horizon_spectrum.b));
        let spectral_color = horizon_spectrum / max(spectrum_peak, 0.025);
        let warm_spectrum = mix(spectral_color,
            vec3<f32>(1.0, 0.24, 0.045), dust_load * 0.62);
        let spread = clamp(dust_load * 0.92 + humid_load * 0.48
            + pollution_load * 0.34 + visibility_haze * 0.22, 0.0, 1.0);
        // Twilight warmth follows the atmospheric horizon and changes
        // gradually across the solar hemisphere without a radial hotspot.
        let solar_hemisphere = 0.38 + 0.62 * pow(max(cosine, 0.0), 2.0);
        color += warm_spectrum * lookup.b * observer_twilight * horizon
            * solar_hemisphere * (0.20 + spread * 0.34);

        let overcast_light = vec3<f32>(0.30, 0.33, 0.36)
            * amount * (0.44 + 0.34 * max(observer_solar_alt, 0.0));
        color = mix(color, overcast_light, overcast * 0.88);
        color *= 1.0 - cloudiness * local_weather_fade * 0.08;
    }

    // Close to the surface, use the fitted radiative-transfer all-sky
    // distribution as the clear-air solution. The shell approximation above
    // remains for aircraft/orbit views where a surface sky dome is not
    // geometrically valid.
    let surface_altitude_weight =
        1.0 - smoothstep(8000.0, 24000.0, observer_alt);
    let above_geometric_horizon = smoothstep(
        -0.06, -0.015, dot(rd, observer_up));
    let sky_elevation_sine = max(dot(rd, observer_up), 0.0);
    let lower_atmosphere_weight =
        1.0 - smoothstep(0.12, 0.58, sky_elevation_sine);
    // Below sunset, direct warm radiance survives mainly on the solar
    // side. The opposite and upper sky stay on the spherical Rayleigh
    // solution instead of inheriting a global brown/violet tint.
    let twilight_solar_side = 0.08 + 0.92
        * smoothstep(-0.35, 0.55, dot(rd, sun_dir));
    let directional_weight = select(
        twilight_solar_side, 1.0, observer_solar_alt >= 0.0);
    let surface_sky_weight = surface_altitude_weight
        * above_geometric_horizon * lower_atmosphere_weight
        * directional_weight * (1.0 - overcast * 0.82);
    if surface_sky_weight > 0.001 {
        var surface_sky = fitted_clear_sky(rd, observer_up, sun_dir);
        let surface_luminance = dot(
            surface_sky, vec3<f32>(0.2126, 0.7152, 0.0722));
        let pollution_neutral = vec3<f32>(1.03, 1.0, 0.93)
            * surface_luminance;
        surface_sky = mix(
            surface_sky,
            pollution_neutral,
            pollution_load * (0.42 + lower_atmosphere_weight * 0.26),
        );
        surface_sky *= 1.0 - pollution_load * 0.10;
        color = mix(color, surface_sky, surface_sky_weight);
    }

    // Discs remain outside the atmosphere intersection so orbit can see them.
    // Terrain and the planetary limb still occlude the enlarged presentation.
    if !planet_blocks {
        let air = 1.0 - smoothstep(20000.0, ATMO_THICKNESS, observer_alt);
        let solar_alt = dot(observer_up, sun_dir);
        let darkness = 1.0
            - smoothstep(sin(radians(-18.0)), sin(radians(-6.0)), solar_alt);
        let aerosol_column = exp(-observer_alt / 1500.0)
            * (1.0 + local_weather_fade * (aerosol * 0.72 + humidity * 0.22));
        let sky_elevation = max(dot(rd, observer_up), 0.0);
        let city_dome = g.sky.w * (0.16 + 0.84 * pow(1.0 - sky_elevation, 2.5));
        let distant_dome = horizon_glow(rd, observer_up)
            * pow(1.0 - sky_elevation, 7.0);
        if (darkness > 0.001) {
            color += milky_way_radiance(rd, observer_up, observer_alt, aerosol_column);
        }
        let artificial = darkness * aerosol_column
            * (0.010 * city_dome + 0.026 * distant_dome);
        color += vec3<f32>(1.0, 0.52, 0.25) * artificial;
        // Natural airglow keeps a pristine moonless sky subtly alive.
        color += vec3<f32>(0.00045, 0.00082, 0.00058)
            * darkness * aerosol_column;
        let sun_mu = dot(rd, sun_dir);
        let pixel_angle = 2.0 * g.viewport.z / max(g.viewport.y, 1.0);
        if sun_mu > cos(0.00465 * 4.0 + pixel_angle) {
            let sun_radius = 0.00465 * celestial_size_scale(
                solar_alt, observer_alt, r_ground);
            let sun_disc = celestial_disc_edge(rd, sun_dir, sun_radius, pixel_angle);
            // The camera's local weather must not describe an entire remote
            // orbital limb. Above that weather column use the clear baseline.
            let aod = mix(0.08, clamp(g.surface.z, 0.0, 2.0), local_weather_fade);
            let angstrom = mix(1.3, 0.3, clamp(g.weather6.x, 0.0, 1.0) * local_weather_fade);
            let aerosol_spectrum = pow(vec3<f32>(0.55 / 0.68, 1.0, 0.55 / 0.44),
                vec3<f32>(angstrom)) * aod * (1.0 + humidity * humidity * 0.4 * local_weather_fade);
            let direct_sun = solar_disc_transmittance(observer_alt,
                dot(rd, observer_up), r_ground, aerosol_spectrum);
            color += direct_sun * sun_disc * 64.0;
        }

        let moon_mu = dot(rd, moon_dir);
        if moon_mu > cos(0.00452 * 4.0 + pixel_angle) {
            let moon_radius = 0.00452 * celestial_size_scale(
                dot(observer_up, moon_dir), observer_alt, r_ground);
            let edge = celestial_disc_edge(rd, moon_dir, moon_radius, pixel_angle);
            // Reconstruct the visible unit-sphere normal from the angular
            // offset. Sun illumination then draws the crescent/terminator
            // with the correct orientation for this date.
            let tangent = (rd - moon_dir * moon_mu) / sin(moon_radius);
            let radial_sq = dot(tangent, tangent);
            let face_normal = normalize(tangent
                - moon_dir * sqrt(max(1.0 - radial_sq, 0.0)));
            let lunar_light = smoothstep(-0.025, 0.025, dot(face_normal, sun_dir));
            let earthshine = 0.018 * (1.0 - g.moon_dir.w);
            // Celestial north fixes the markings to the Moon rather than
            // the camera or observer latitude. Lunar declination is far
            // from the poles, so this tangent basis has no singularity.
            let lunar_east = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), moon_dir));
            let lunar_north = cross(moon_dir, lunar_east);
            let lunar_uv = vec2<f32>(dot(tangent, lunar_east), dot(tangent, lunar_north));
            let albedo = lunar_surface_albedo(lunar_uv, pixel_angle / moon_radius);
            color += vec3<f32>(0.78, 0.77, 0.73) * albedo
                * edge * (earthshine + 1.25 * lunar_light);
        }
        let moon_halo = air * g.moon_dir.w * 0.018
            * pow(max(moon_mu, 0.0), 512.0);
        color += vec3<f32>(0.34, 0.48, 0.90) * moon_halo;

        color = transient_sky_effects(color, rd, sun_dir, observer_up, solar_alt, air);
    }
    color = mix(
        color,
        color * 0.30 + vec3<f32>(0.022, 0.027, 0.033)
            * (0.35 + 0.65 * max(observer_solar_alt, 0.0)),
        overcast * 0.72,
    );
    // Diffuse in-cloud discharge lifts the nearby storm sky while the narrow
    // channel remains a strongly HDR source after tone mapping.
    color += vec3<f32>(0.10, 0.14, 0.24) * g.weather2.w
        * local_weather_fade * (0.35 + cloudiness * 0.65);
    color += lightning_bolt;
    return vec4<f32>(color, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return atmosphere_at_depth(in, textureLoad(depth_tex, vec2<i32>(in.pos.xy), 0));
}

@fragment
fn fs_main_ms(in: VsOut) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.pos.xy);
    let samples = textureNumSamples(world_depth_samples);
    var sky_samples = 0u;
    var nearest = 0.0;
    for (var i = 0u; i < samples; i += 1u) {
        let d = textureLoad(world_depth_samples, pixel, i);
        sky_samples += select(0u, 1u, d == 0.0);
        nearest = max(nearest, d);
    }
    if sky_samples == 0u { return atmosphere_at_depth(in, nearest); }
    let sky = atmosphere_at_depth(in, 0.0);
    if sky_samples == samples { return sky; }
    // Resolved color already contains fractional geometry coverage. Add sky
    // only to uncovered samples; nearest depth alone would leave a dark fringe.
    return mix(atmosphere_at_depth(in, nearest), sky, f32(sky_samples) / f32(samples));
}
