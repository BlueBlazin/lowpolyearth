// Opaque water surface response. Wave gradients follow the sum-of-waves
// construction in Finch, GPU Gems chapter 1; neither geometry nor occupancy
// moves. The reflection uses the same Hosek-Wilkie coefficients as the sky.
// https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-1-effective-water-simulation-physical-models
// https://cgg.mff.cuni.cz/projects/SkylightModelling/

struct WaterSkyModel {
    values: array<vec4<f32>, 8>,
};
@group(0) @binding(1) var<uniform> water_sky: WaterSkyModel;

struct WaterSurface {
    normal: vec3<f32>,
    roughness: f32,
    crest: f32,
};

struct WaterWaveField {
    gradient: vec3<f32>,
    height: f32,
    height_bound: f32,
    suppressed_variance: f32,
};

fn water_bed_hash(cell: vec3<i32>, mask: i32) -> f32 {
    let p = vec3<u32>(cell & vec3<i32>(mask));
    var h = (p.x * 1597334677u) ^ (p.y * 3812015801u) ^ (p.z * 2798796415u);
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = (h ^ (h >> 13u)) * 3266489917u;
    return f32((h ^ (h >> 16u)) & 65535u) / 65535.0;
}

fn water_bed_noise(p: vec3<f32>, mask: i32) -> f32 {
    let cell = vec3<i32>(floor(p));
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(water_bed_hash(cell, mask), water_bed_hash(cell + vec3<i32>(1, 0, 0), mask), w.x),
        mix(water_bed_hash(cell + vec3<i32>(0, 1, 0), mask), water_bed_hash(cell + vec3<i32>(1, 1, 0), mask), w.x), w.y);
    let b = mix(mix(water_bed_hash(cell + vec3<i32>(0, 0, 1), mask), water_bed_hash(cell + vec3<i32>(1, 0, 1), mask), w.x),
        mix(water_bed_hash(cell + vec3<i32>(0, 1, 1), mask), water_bed_hash(cell + vec3<i32>(1, 1, 1), mask), w.x), w.y);
    return mix(a, b, w.z);
}

// Stationary benthic colour, separate from animated surface normals. Only
// ocean water with appreciable returning bed light exposes these patches.
// Periodic coordinates preserve the 8192 m material-origin wrap. Subpixel
// detail loses contrast and unresolved patches fade with their pixel footprint.
fn water_bed_color(body: vec3<f32>, position_m: vec3<f32>, footprint_m: f32, kind: u32) -> vec3<f32> {
    if kind != 1u { return body; }
    let visible = smoothstep(0.025, 0.24, body.g);
    if visible <= 0.0 || footprint_m >= 32.0 { return body; }
    let p = fract(position_m / 8192.0) * 8192.0;
    let large = water_bed_noise(p / 256.0, 31);
    let medium_weight = 1.0 - smoothstep(3.0, 16.0, footprint_m);
    let fine_weight = 1.0 - smoothstep(0.75, 4.0, footprint_m);
    // Integer-ratio periods and periodic warps keep every band continuous.
    let medium = water_bed_noise(p / 16.0 + vec3<f32>(large * 3.8), 511);
    let fine = water_bed_noise(p / 4.0 + vec3<f32>(medium * 2.2), 2047);
    let shape = medium * 0.72 + large * 0.28 + (fine - 0.5) * 0.16 * fine_weight;
    let reef = smoothstep(0.62, 0.80, shape) * medium_weight;
    return body * mix(vec3<f32>(1.0), vec3<f32>(0.10, 0.17, 0.15), reef * visible * 0.94);
}

// Integer carriers and power-of-two spatial periods repeat at every 8192 m
// material-origin wrap. Two shared long waves bend phase and vary amplitude;
// their derivatives are retained so the normal follows the displaced height.
// Integer temporal cycles retain the 65536 s weather-clock wrap.
fn water_wave_field(position_m: vec3<f32>, normal: vec3<f32>, energy: f32,
                    simulation_seconds: f32, pixel_footprint_m: f32) -> WaterWaveField {
    let tau = 6.28318530718;
    let carriers = array<vec3<f32>, 4>(
        vec3<f32>(3.0, 1.0, 2.0), vec3<f32>(1.0, 3.0, -2.0),
        vec3<f32>(2.0, -1.0, -3.0), vec3<f32>(-1.0, 2.0, 1.0),
    );
    let periods_m = array<f32, 4>(16.0, 8.0, 4.0, 1.0);
    let cycles = array<f32, 4>(26713.0, 40177.0, 62723.0, 110017.0);
    let slopes = array<f32, 4>(0.052, 0.042, 0.030, 0.020);
    let phase_a = array<f32, 4>(3.1, 2.3, -2.8, 3.7);
    let phase_b = array<f32, 4>(2.7, -3.4, 2.1, -2.6);
    let amplitude_a = array<f32, 4>(0.20, -0.20, 0.20, -0.20);
    let amplitude_b = array<f32, 4>(0.08, 0.08, -0.08, -0.08);
    let modulation_a = vec3<f32>(17.0, -11.0, 7.0);
    let modulation_b = vec3<f32>(-43.0, 71.0, 37.0);
    let clock = fract(simulation_seconds / 65536.0);
    let theta_a = tau * fract(dot(fract(position_m / 1024.0), modulation_a) - clock * 911.0);
    let theta_b = tau * fract(dot(fract(position_m / 8192.0), modulation_b) - clock * 431.0);
    let sin_a = sin(theta_a);
    let sin_b = sin(theta_b);
    let cos_a = cos(theta_a);
    let cos_b = cos(theta_b);
    let modulation_gradient_a = tau / 1024.0
        * (modulation_a - normal * dot(modulation_a, normal));
    let modulation_gradient_b = tau / 8192.0
        * (modulation_b - normal * dot(modulation_b, normal));
    let frequency_a = length(modulation_gradient_a) / tau;
    let frequency_b = length(modulation_gradient_b) / tau;
    let footprint = max(pixel_footprint_m, 0.0);
    var gradient = vec3<f32>(0.0);
    var suppressed_variance = 0.0;
    var height = 0.0;
    var height_bound = 0.0;
    for (var index = 0u; index < 4u; index += 1u) {
        let carrier = carriers[index];
        let tangent = carrier - normal * dot(carrier, normal);
        let base_gradient = tangent * (tau / periods_m[index]);
        let phase_gradient = base_gradient
            + phase_a[index] * cos_a * modulation_gradient_a
            + phase_b[index] * cos_b * modulation_gradient_b;
        let envelope = 0.72 + amplitude_a[index] * sin_a + amplitude_b[index] * sin_b;
        let envelope_gradient = amplitude_a[index] * cos_a * modulation_gradient_a
            + amplitude_b[index] * cos_b * modulation_gradient_b;
        // A conservative bandwidth includes phase spreading and the amplitude
        // sidebands. Locally cancelling gradients must not reveal aliased waves.
        let bandwidth = length(tangent) / periods_m[index]
            + (abs(phase_a[index]) + 1.0) * frequency_a
            + (abs(phase_b[index]) + 1.0) * frequency_b;
        let resolved = 1.0 - smoothstep(0.12, 0.50, footprint * bandwidth);
        let amplitude = slopes[index] * energy * periods_m[index] / (tau * length(carrier));
        let variance = amplitude * amplitude * 0.5
            * (envelope * envelope * dot(phase_gradient, phase_gradient)
                + dot(envelope_gradient, envelope_gradient));
        suppressed_variance += variance * (1.0 - resolved * resolved);
        if resolved > 0.0 {
            let spatial = dot(fract(position_m / periods_m[index]), carrier);
            let phase = tau * fract(spatial - clock * cycles[index])
                + phase_a[index] * sin_a + phase_b[index] * sin_b;
            let phase_sin = sin(phase);
            let phase_cos = cos(phase);
            gradient += amplitude * resolved
                * (envelope * phase_cos * phase_gradient + phase_sin * envelope_gradient);
            height += amplitude * resolved * envelope * phase_sin;
            height_bound += amplitude * envelope;
        }
    }
    return WaterWaveField(gradient, height, height_bound, suppressed_variance);
}

// The world specular lobe consumes perceptual roughness to the fourth power.
// Keep all lost mean-square slope energy until the caller scales carriers;
// an artistic roughness cap belongs after that mixture, not in this helper.
fn water_roughness_from_slope_variance(base_roughness: f32, variance: f32) -> f32 {
    return pow(pow(base_roughness, 4.0) + max(variance, 0.0), 0.25);
}

fn water_surface(position_m: vec3<f32>, base_normal: vec3<f32>,
                 wind_mps: f32, sea_state: f32, simulation_seconds: f32,
                 pixel_footprint_m: f32) -> WaterSurface {
    let wind = clamp(wind_mps / 22.0, 0.0, 1.0);
    let sea = clamp(sea_state, 0.0, 1.0);
    let energy = 0.30 + 1.15 * sea + 0.55 * wind;
    let normal = normalize(base_normal);
    let field = water_wave_field(position_m, normal, energy, simulation_seconds, pixel_footprint_m);
    let base_roughness = 0.095 + 0.075 * wind + 0.10 * sea;
    let roughness = water_roughness_from_slope_variance(base_roughness, field.suppressed_variance);
    return WaterSurface(normalize(normal - field.gradient), roughness,
        clamp(0.5 + 0.5 * field.height / max(field.height_bound, 0.00001), 0.0, 1.0));
}

// Schlick approximation for air-to-water IOR 1.333. Reflection replaces the
// transmitted/body component with angle; it is not an additive blue coating.
fn water_fresnel(cosine: f32) -> f32 {
    let grazing = 1.0 - clamp(cosine, 0.0, 1.0);
    let grazing2 = grazing * grazing;
    return 0.02037 + 0.97963 * grazing2 * grazing2 * grazing;
}

// Source classes: 0 untyped, 1 ocean, 2 lake/pond/reservoir, 3 river/canal.
// Shelter changes amplitudes, never carrier phase, so adjoining classes and
// weather changes cannot reverse or slide the wave field.
fn water_exposure(kind: u32) -> vec2<f32> {
    if kind == 2u { return vec2<f32>(0.48, 0.12); }
    if kind == 3u { return vec2<f32>(0.32, 0.06); }
    return vec2<f32>(1.0);
}

// A small transmitted-light shoulder, rather than a luminous water coating.
// Only the sunward wave crests receive it; the caller applies Fresnel after
// mixing this body contribution. Night and occluded sunlight give zero.
fn water_crest_scattering(body: vec3<f32>, crest: f32, view_cosine: f32,
                          sun_view_cosine: f32, sunlight: vec3<f32>) -> vec3<f32> {
    let shoulder = smoothstep(0.45, 0.95, crest);
    let backlight = pow(max(sun_view_cosine, 0.0), 2.0);
    let grazing = 1.0 - clamp(view_cosine, 0.0, 1.0);
    let transmitted = mix(vec3<f32>(0.018, 0.10, 0.075), body, 0.45);
    return transmitted * sunlight * shoulder * (0.06 + backlight * 0.58)
        * (0.35 + 0.65 * grazing);
}

fn water_sky_value(index: u32) -> f32 {
    return water_sky.values[index / 4u][index & 3u];
}

fn water_sky_channel(cosine_theta: f32, gamma: f32,
                     cosine_gamma: f32, channel: u32) -> f32 {
    let base = channel * 9u;
    let p0 = water_sky_value(base);
    let p1 = water_sky_value(base + 1u);
    let p2 = water_sky_value(base + 2u);
    let p3 = water_sky_value(base + 3u);
    let p4 = water_sky_value(base + 4u);
    let p5 = water_sky_value(base + 5u);
    let p6 = water_sky_value(base + 6u);
    let p7 = water_sky_value(base + 7u);
    let p8 = water_sky_value(base + 8u);
    let cosine_gamma2 = cosine_gamma * cosine_gamma;
    let denominator = max(1.0 + p8 * p8 - 2.0 * p8 * cosine_gamma, 0.00001);
    let mie = (1.0 + cosine_gamma2) / (denominator * sqrt(denominator));
    let distribution = (1.0 + p0 * exp(p1 / (cosine_theta + 0.01)))
        * (p2 + p3 * exp(p4 * gamma) + p5 * cosine_gamma2
            + p6 * mie + p7 * sqrt(cosine_theta));
    return max(water_sky_value(27u + channel) * distribution, 0.0);
}

fn water_clear_sky(direction: vec3<f32>, up: vec3<f32>,
                   sun_direction: vec3<f32>, sky_conditions: vec3<f32>) -> vec3<f32> {
    let solar = clamp(dot(up, sun_direction), -1.0, 1.0);
    let horizontal_sun = sun_direction - up * solar;
    var effective_sun = up;
    if dot(horizontal_sun, horizontal_sun) > 0.0000001 {
        effective_sun = normalize(horizontal_sun + up * max(solar, 0.001));
    }
    let cosine_theta = clamp(dot(direction, up), 0.0, 1.0);
    let cosine_gamma = clamp(dot(direction, effective_sun), -1.0, 1.0);
    let gamma = acos(cosine_gamma);
    let radiance = vec3<f32>(
        water_sky_channel(cosine_theta, gamma, cosine_gamma, 0u),
        water_sky_channel(cosine_theta, gamma, cosine_gamma, 1u),
        water_sky_channel(cosine_theta, gamma, cosine_gamma, 2u),
    );
    let civil_light = smoothstep(-0.22, -0.015, solar);
    let horizon_light = smoothstep(-0.065, 0.025, solar);
    let graded = sunset_sky_grade(radiance, cosine_theta, solar, cosine_gamma,
        sky_conditions.x, sky_conditions.y, sky_conditions.z);
    return graded * (0.035 * civil_light * (0.18 + 0.82 * horizon_light));
}

// A sky-only environment: reflected buildings, shoreline foam and seabed
// refraction require additional source/scene data. Cloud shades the reflected
// sky without reflected bodies: waves blur broken cloud into a coverage-
// weighted mix of sky and brighter cloud, and overcast uses the current sky's
// luminance rather than retaining clear-blue chroma under thick cloud.
fn water_reflected_environment(reflected_direction: vec3<f32>, local_up: vec3<f32>,
                               sun_direction: vec3<f32>, moon_direction: vec3<f32>,
                               moon_illumination: f32, cloud_fraction: f32,
                               cloud_optical_depth: f32,
                               sky_conditions: vec3<f32>) -> vec3<f32> {
    let up = normalize(local_up);
    // Sub-horizon wave normals have no observed reflected terrain here. Use
    // the horizon sky continuously instead of extrapolating the analytic dome.
    let below = min(dot(reflected_direction, up), 0.0);
    let direction = normalize(reflected_direction - up * below + up * 0.00001);
    let effective_sun = normalize(sun_direction);
    let solar = dot(effective_sun, up);
    let clear = water_clear_sky(direction, up, effective_sun, sky_conditions);
    let value = dot(clear, vec3<f32>(0.2126, 0.7152, 0.0722));
    let coverage = clamp(cloud_fraction, 0.0, 1.0);
    let overcast = coverage * smoothstep(1.5, 7.0, cloud_optical_depth);
    let elevation = clamp(dot(direction, up), 0.0, 1.0);
    // Perspective packs broken cloud toward the horizon.
    let broken = coverage * (1.0 - overcast) * mix(0.75, 0.35, elevation);
    let cloud_light = mix(vec3<f32>(value), clear, 0.25) * 1.6;
    // Raining decks are optically deeper over the same range the cloud
    // volume uses for water content, so their reflection darkens too.
    let storm = smoothstep(2.5, 9.0, cloud_optical_depth);
    let gray = vec3<f32>(value) * (0.48 + 0.22 * elevation) * mix(1.0, 0.6, storm);
    let daylight_sky = mix(mix(clear, cloud_light, broken), gray, overcast);
    let night = 1.0 - smoothstep(-0.22, -0.04, solar);
    let moon_height = max(dot(normalize(moon_direction), up), 0.0);
    let moonlight = clamp(moon_illumination, 0.0, 1.0) * moon_height;
    let moon_sky = vec3<f32>(0.0014, 0.0018, 0.0025) * (0.25 + 0.75 * moonlight);
    return max(daylight_sky + moon_sky * night * (1.0 - 0.75 * overcast), vec3<f32>(0.0));
}
