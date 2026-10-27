// Hipparcos bright stars as tiny screen-space billboards. Catalog directions
// are ICRS-like equatorial vectors; Greenwich sidereal time rotates them into
// ECEF, so the sky follows UTC and the observer's real latitude automatically.

struct Globals {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    moon_dir: vec4<f32>,
    ambient: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
    planet_center: vec4<f32>,
    viewport: vec4<f32>,
    sky: vec4<f32>, // cos(GMST), sin(GMST), UTC modulo 4096 s, local glow
    horizon_glow_0: vec4<f32>, // north, north-east, east, south-east
    horizon_glow_1: vec4<f32>, // south, south-west, west, north-west
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var depth_tex: texture_depth_2d;

struct StarIn {
    @location(0) direction_icrs: vec3<f32>,
    @location(1) magnitude: f32,
    @location(2) bv: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) magnitude: f32,
    @location(2) flux: f32,
    @location(3) color: vec3<f32>,
    @location(4) @interpolate(flat) direction_ecef: vec3<f32>,
    @location(5) @interpolate(flat) noise: f32,
};

const CORNERS: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
);

fn star_color(bv: f32) -> vec3<f32> {
    let value = clamp(bv, -0.4, 2.0);
    if value < 0.65 {
        return mix(vec3<f32>(0.55, 0.70, 1.0), vec3<f32>(1.0, 0.95, 0.84),
            (value + 0.4) / 1.05);
    }
    return mix(vec3<f32>(1.0, 0.95, 0.84), vec3<f32>(1.0, 0.42, 0.16),
        (value - 0.65) / 1.35);
}

@vertex
fn vs_main(in: StarIn, @builtin(vertex_index) vi: u32,
        @builtin(instance_index) instance: u32) -> VsOut {
    var out: VsOut;
    let c = g.sky.x;
    let s = g.sky.y;
    let inertial = normalize(in.direction_icrs);
    let direction = vec3<f32>(
        c * inertial.x + s * inertial.y,
        -s * inertial.x + c * inertial.y,
        inertial.z,
    );
    let clip = g.view_proj * vec4<f32>(direction, 0.0);
    let corner = CORNERS[vi];
    let flux = pow(2.5118864, -in.magnitude);
    let radius_px = 0.82 + 0.46 * sqrt(min(flux, 5.0));
    let offset = corner * radius_px * 2.0 / max(g.viewport.xy, vec2<f32>(1.0));
    out.pos = vec4<f32>(clip.xy + offset * clip.w, clip.z, clip.w);
    out.uv = corner;
    out.magnitude = in.magnitude;
    out.flux = flux;
    out.color = star_color(in.bv);
    out.direction_ecef = direction;
    out.noise = fract(sin(f32(instance) * 91.3458 + 17.234) * 47453.5453);
    return out;
}

fn ray_sphere_hits(rd: vec3<f32>, center: vec3<f32>, radius: f32) -> bool {
    let oc = -center;
    let b = dot(oc, rd);
    let disc = b * b - (dot(oc, oc) - radius * radius);
    return disc >= 0.0 && (-b - sqrt(max(disc, 0.0))) > 0.0;
}

fn dome_at(index: u32) -> f32 {
    if index < 4u {
        return g.horizon_glow_0[index];
    }
    return g.horizon_glow_1[index - 4u];
}

fn horizon_sample(direction: vec3<f32>, up: vec3<f32>) -> f32 {
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

fn current_limit(up: vec3<f32>, altitude: f32, air_column: f32) -> f32 {
    let effective_glow = clamp(g.sky.w * air_column, 0.0, 1.0);
    let site_limit = 7.8 - 5.3 * sqrt(effective_glow);
    let moon_alt = max(dot(up, normalize(g.moon_dir.xyz)), 0.0);
    let moon_penalty = 3.3 * pow(g.moon_dir.w, 0.6) * sqrt(moon_alt) * air_column;
    let solar_alt = dot(up, normalize(g.sun_dir.xyz));
    let twilight = smoothstep(sin(radians(-18.0)), sin(radians(-6.0)), solar_alt)
        * exp(-altitude / 20000.0);
    return mix(site_limit - moon_penalty, -2.0, twilight);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if textureLoad(depth_tex, vec2<i32>(in.pos.xy), 0) > 0.0
            || ray_sphere_hits(in.direction_ecef, g.planet_center.xyz, g.planet_center.w) {
        discard;
    }
    let radial_sq = dot(in.uv, in.uv);
    if radial_sq > 1.0 {
        discard;
    }
    let up = normalize(-g.planet_center.xyz);
    let altitude = max(length(g.planet_center.xyz) - g.planet_center.w, 0.0);
    let air_column = exp(-altitude / 1500.0);
    let sin_altitude = dot(up, in.direction_ecef);
    let horizon_lobe = pow(1.0 - max(sin_altitude, 0.0), 4.0);
    let dome_penalty = horizon_sample(in.direction_ecef, up) * air_column
        * horizon_lobe * 2.2;
    let limit = current_limit(up, altitude, air_column) - dome_penalty;
    let visibility = smoothstep(in.magnitude - 0.28, in.magnitude + 0.32, limit);
    if visibility <= 0.001 {
        discard;
    }

    // Plane-parallel airmass is sufficient away from the last degree of the
    // horizon. Extinction both dims and warms long atmospheric paths.
    let airmass = clamp(1.0 / max(sin_altitude, 0.035), 1.0, 12.0);
    let extinction_mag = 0.16 * (airmass - 1.0) * air_column;
    let extinction = pow(2.5118864, -extinction_mag);
    let reddening = clamp(extinction_mag * 0.16, 0.0, 0.7);
    let tint = mix(in.color, vec3<f32>(1.0, 0.42, 0.16), reddening);
    let twinkle_amount = clamp((airmass - 1.0) / 11.0, 0.0, 1.0) * 0.26 * air_column;
    let twinkle = 1.0 + twinkle_amount
        * sin(g.sky.z * 7.1 + in.noise * 91.0);
    let profile = exp(-radial_sq * 3.8);
    let intensity = visibility * extinction * twinkle * profile
        * (0.34 + 0.48 * sqrt(min(in.flux, 5.0)));
    return vec4<f32>(tint * intensity, 1.0);
}
