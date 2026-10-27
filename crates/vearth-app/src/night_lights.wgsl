// Detached building-light proxies retained for compatible prepared columns.
// Current generation emits building light through voxel facades instead.

struct Globals {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    moon_dir: vec4<f32>,
    ambient: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
    planet_center: vec4<f32>,
    viewport: vec4<f32>,
    sky: vec4<f32>,
    horizon_glow_0: vec4<f32>,
    horizon_glow_1: vec4<f32>,
};

struct LightU {
    model: mat4x4<f32>,
    bx: vec4<f32>,
    by: vec4<f32>,
    bz: vec4<f32>,
    fade: vec4<f32>,
    light_fade0: vec4<f32>,
    light_fade1: vec4<f32>,
    light_fade2: vec4<f32>,
    light_fade3: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<storage, read> chunks: array<LightU>;

struct VsIn {
    @location(0) center: vec3<f32>,
    @location(1) energy: f32,
    @location(2) corner: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) bucket: u32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) strength: f32,
};

fn bucket_fade(bucket: u32, c: LightU) -> f32 {
    switch bucket / 4u {
        case 0u: { return c.light_fade0[bucket & 3u]; }
        case 1u: { return c.light_fade1[bucket & 3u]; }
        case 2u: { return c.light_fade2[bucket & 3u]; }
        default: { return c.light_fade3[bucket & 3u]; }
    }
}

@vertex
fn vs_main(in: VsIn, @builtin(instance_index) chunk_index: u32) -> VsOut {
    var out: VsOut;
    let c = chunks[chunk_index];
    var world = c.model * vec4<f32>(in.center, 1.0);
    let up = normalize(world.xyz - g.planet_center.xyz);
    let solar_alt = dot(up, normalize(g.sun_dir.xyz));
    let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);

    // Pull the proxy to the camera-facing side of its represented facade.
    // It remains depth-tested against terrain and nearer geometry without
    // being buried at the source building's centroid.
    let toward_camera = -normalize(world.xyz);
    world = vec4<f32>(
        world.xyz + toward_camera * min(18.0, sqrt(max(in.energy, 0.0)) * 0.055),
        world.w,
    );

    let distance_m = max(length(world.xyz), 1.0);
    // Proxy cells stop at L15. Keep their raster footprint sub-pixel even if
    // a coarse fallback is viewed unusually close; they must never become
    // camera-facing glowing discs among buildings.
    let radius_px = 0.52;
    // Brightness depends on represented facade energy, not camera distance.
    // Projected-area attenuation made still-visible buildings cross a dark
    // ring merely because their terrain cells were farther from the camera.
    let energy_gain = clamp(sqrt(max(in.energy, 0.0)) * 0.060, 0.34, 1.0);
    // Match the world's near/regional aerial transmittance so distant city
    // lights sit inside haze instead of drawing sharp points over it.
    let r_ell = 6378137.0 * (1.0 - 0.0033528 * up.z * up.z);
    let h0 = clamp(length(world.xyz - g.planet_center.xyz) - r_ell, 0.0, 20000.0);
    let h_fog = 18000.0;
    let mu = abs(dot(normalize(world.xyz), up));
    let path = min(distance_m, h_fog / max(mu, 0.015));
    let transmittance = exp(-0.5e-5 * exp(-h0 / h_fog) * path);

    var clip = g.view_proj * world;
    clip = vec4<f32>(
        clip.xy + in.corner * radius_px * 2.0
            / max(g.viewport.xy, vec2<f32>(1.0)) * clip.w,
        clip.zw,
    );
    out.clip = clip;
    out.color = pow(in.color.rgb, vec3<f32>(2.2));
    out.strength = night * bucket_fade(in.bucket, c) * energy_gain * transmittance
        * select(0.0, 1.0, g.viewport.w > 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.strength <= 0.0) {
        discard;
    }
    // One energy sample, without a radial halo. At its intended far-city
    // range this reads as unresolved facade luminance rather than an orb.
    let rgb = in.color * (2.4 * in.strength);
    return vec4<f32>(rgb, 1.0);
}
