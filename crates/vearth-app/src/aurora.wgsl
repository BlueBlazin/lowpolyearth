// Procedural auroral curtains. A handful of translucent vertical ribbons
// follow a fixed, tilted geomagnetic oval in Earth-fixed space. Geometry is
// generated entirely from vertex/instance indices, so the effect needs no
// textures, streamed data, or persistent mesh buffers.

struct Globals {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    moon_dir: vec4<f32>,
    ambient: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
    planet_center: vec4<f32>,
    viewport: vec4<f32>,
    sky: vec4<f32>, // z = UTC modulo 4096 s
    horizon_glow_0: vec4<f32>,
    horizon_glow_1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) up_ecef: vec3<f32>,
    @location(2) sheet_normal: vec3<f32>,
    @location(3) magnetic_angle: f32,
    @location(4) altitude_m: f32,
    @location(5) strength: f32,
    @location(6) layer: f32,
    @location(7) lower_boundary_m: f32,
    @location(8) upper_boundary_m: f32,
};

const TAU: f32 = 6.283185307179586;
const SEGMENTS: u32 = 1024u;
const CURTAINS_PER_HEMISPHERE: u32 = 6u;
const WGS84_A: f32 = 6378137.0;
const WGS84_B: f32 = 6356752.314245;

// A restrained fixed dipole approximation: roughly 11 degrees from the
// geographic pole. It gives the oval a recognizably auroral placement without
// pretending to be a live space-weather or IGRF product.
const MAGNETIC_AXIS: vec3<f32> = vec3<f32>(0.0480, -0.1545, 0.9868);

fn ellipsoid_radius(direction: vec3<f32>) -> f32 {
    let equatorial = direction.x * direction.x + direction.y * direction.y;
    let inverse_radius_sq = equatorial / (WGS84_A * WGS84_A)
        + direction.z * direction.z / (WGS84_B * WGS84_B);
    return inverseSqrt(inverse_radius_sq);
}

// A periodic, texture-free energy-flux proxy. Integer angular frequencies
// make the value continuous where the procedural oval closes.
fn precipitation_flux(angle: f32, phase: f32, layer: f32) -> f32 {
    let broad = sin(angle * (2.0 + layer) - phase * 5.0 + layer * 1.71);
    let medium = sin(angle * (7.0 + 2.0 * layer) + phase * 13.0 - layer * 0.93);
    let detail = sin(angle * (19.0 + layer) - phase * 29.0 + layer * 2.37);
    return clamp(0.50 + 0.25 * broad + 0.16 * medium + 0.09 * detail,
        0.0, 1.0);
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32,
        @builtin(instance_index) instance: u32) -> VsOut {
    var out: VsOut;
    let segment = vertex >> 1u;
    let vertical = f32(vertex & 1u);
    let angle = f32(segment) / f32(SEGMENTS) * TAU;
    let layer_index = instance % CURTAINS_PER_HEMISPHERE;
    let hemisphere = select(1.0, -1.0,
        instance >= CURTAINS_PER_HEMISPHERE);
    let layer = f32(layer_index);

    let magnetic_north = normalize(MAGNETIC_AXIS);
    let magnetic_x = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), magnetic_north));
    let magnetic_y = normalize(cross(magnetic_north, magnetic_x));
    let around = magnetic_x * cos(angle) + magnetic_y * sin(angle);

    // Every temporal term completes an integer number of cycles before the
    // modulo-4096 UTC clock wraps, keeping animation continuous at the wrap.
    let phase = g.sky.z * (TAU / 4096.0);
    let broad_fold = sin(angle * (3.0 + f32(layer_index % 3u))
        + phase * (11.0 + 2.0 * f32(layer_index % 2u)));
    let medium_fold = sin(angle * (11.0 + 2.0 * layer)
        - phase * (23.0 + 2.0 * f32(layer_index % 3u)));
    let fine_fold = sin(angle * (29.0 + layer)
        + phase * (47.0 + 2.0 * f32(layer_index % 2u)));
    let fold_degrees = broad_fold * 0.55 + medium_fold * 0.22
        + fine_fold * 0.08;

    // Several closely spaced sheets fill a roughly six-degree-wide quiet
    // auroral oval. A single folded latitude is used through the full height;
    // the ray direction below supplies the physically important field-line
    // alignment instead of leaning the top independently from the base.
    let base_latitude_degrees = 66.0 + layer * 1.25;
    let magnetic_latitude = hemisphere
        * radians(base_latitude_degrees + fold_degrees);
    let up = normalize(around * cos(magnetic_latitude)
        + magnetic_north * sin(magnetic_latitude));

    // Keep the mesh safely below the luminous lower border. The fragment
    // shader fades against a separate, locally varying boundary; ending the
    // geometry at that boundary exposes a ruler-straight triangle-strip edge.
    // Each sheet gets its own altitude and fine structure so neighbouring
    // curtains do not appear to have been clipped by one shared plane.
    let flux = precipitation_flux(angle, phase, layer);
    let lower_boundary_m = 108000.0
        + (layer - 2.5) * 650.0
        + 900.0 * sin(layer * 2.13)
        - 10500.0 * flux
        + 2300.0 * sin(angle * 11.0 + layer * 1.7)
        + 900.0 * sin(angle * 31.0 - layer * 0.83 - phase * 7.0);
    let upper_boundary_m = 242000.0 + 76000.0 * flux
        + 9000.0 * sin(angle * 5.0 - layer * 0.9 + phase * 11.0);
    let mesh_lower_m = lower_boundary_m - 18000.0;
    let mesh_upper_m = upper_boundary_m + 45000.0;
    let requested_altitude = mix(mesh_lower_m, mesh_upper_m, vertical);

    // Dipole field direction B ∝ 3(m·r)r − m. Particles and the visible rays
    // follow this direction, which is nearly vertical near the magnetic poles
    // but noticeably inclined around the equatorward edge of the oval.
    let dipole_field = 3.0 * dot(magnetic_north, up) * up - magnetic_north;
    let field_outward = normalize(dipole_field
        * select(-1.0, 1.0, dot(dipole_field, up) >= 0.0));
    let field_vertical = max(dot(field_outward, up), 0.35);
    let surface_radius = ellipsoid_radius(up);
    let surface_pos = g.planet_center.xyz + up * surface_radius;
    let world_pos = surface_pos
        + field_outward * (requested_altitude / field_vertical);
    let world_up = normalize(world_pos - g.planet_center.xyz);
    let altitude = length(world_pos - g.planet_center.xyz)
        - ellipsoid_radius(world_up);
    let tangent = normalize(cross(magnetic_north, up)) * hemisphere;
    let sheet_normal = normalize(cross(field_outward, tangent));
    let edge_distance = abs((layer - 2.5) / 2.5);

    out.pos = g.view_proj * vec4<f32>(world_pos, 1.0);
    out.world_pos = world_pos;
    out.up_ecef = world_up;
    out.sheet_normal = sheet_normal;
    out.magnetic_angle = angle;
    out.altitude_m = altitude;
    out.strength = mix(1.0, 0.30, edge_distance);
    out.layer = layer;
    out.lower_boundary_m = lower_boundary_m;
    out.upper_boundary_m = upper_boundary_m;
    return out;
}

fn planet_blocks(point: vec3<f32>) -> bool {
    let distance = length(point);
    let direction = point / max(distance, 1.0);
    let axes = vec3<f32>(WGS84_A, WGS84_A, WGS84_B);
    // Earth-centred scaled-space ray. g.planet_center is the Earth centre
    // relative to the camera, so the camera's ECEF position is its negation.
    let origin = -g.planet_center.xyz / axes;
    let ray = direction / axes;
    let a = dot(ray, ray);
    let b = dot(origin, ray);
    let c = dot(origin, origin) - 1.0;
    let discriminant = b * b - a * c;
    if discriminant <= 0.0 {
        return false;
    }
    let ground_distance = (-b - sqrt(discriminant)) / a;
    return ground_distance > 0.0 && ground_distance < distance - 1000.0;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if planet_blocks(in.world_pos) {
        discard;
    }

    // Each point on the global oval decides from its own solar altitude, so
    // orbital views show only the night-side arc and polar summer stays dark.
    let solar_altitude = dot(normalize(in.up_ecef), normalize(g.sun_dir.xyz));
    let darkness = 1.0 - smoothstep(sin(radians(-12.0)),
        sin(radians(-4.0)), solar_altitude);
    if darkness <= 0.001 {
        discard;
    }

    let phase = g.sky.z * (TAU / 4096.0);
    let flux = precipitation_flux(in.magnetic_angle, phase, in.layer);
    let slow_pulse = 0.84 + 0.16 * sin(in.magnetic_angle * 7.0
        - phase * 11.0 + in.layer * 1.31);
    let active_arc = smoothstep(0.22, 0.72, flux);
    let longitudinal_activity = 0.07 + 0.93 * active_arc;

    // Broad folds form a connected luminous sheet; two finer scales add the
    // field-aligned striations. Derivative filtering converges to each
    // pattern's mean at orbital distance rather than aliasing or shimmering.
    let cluster_coord = in.magnetic_angle * (149.0 + in.layer * 11.0)
        + 1.35 * sin(in.magnetic_angle * 17.0 - phase * 11.0)
        + phase * (17.0 + in.layer * 2.0);
    let cluster_secondary_coord = in.magnetic_angle
        * (233.0 + in.layer * 13.0)
        - 0.9 * sin(in.magnetic_angle * 31.0 + phase * 7.0)
        - phase * (23.0 + in.layer * 2.0);
    let cluster_primary = 0.5 + 0.5 * sin(cluster_coord);
    let cluster_secondary = 0.5 + 0.5 * sin(cluster_secondary_coord);
    let cluster_wave = cluster_primary * (0.62 + 0.38 * cluster_secondary);
    let resolved_cluster = smoothstep(0.08, 0.94, cluster_wave);
    let cluster_footprint = max(fwidth(cluster_coord),
        fwidth(cluster_secondary_coord));
    let cluster_blur = smoothstep(0.45, 1.8, cluster_footprint);
    let cluster = mix(resolved_cluster, 0.43, cluster_blur);

    let strand_coord = in.magnetic_angle * (997.0 + in.layer * 73.0)
        + 3.1 * sin(in.magnetic_angle * 43.0 - phase * 31.0)
        - phase * (59.0 + in.layer * 4.0);
    let strand_wave = 0.5 + 0.5 * sin(strand_coord);
    let resolved_strand = smoothstep(0.30, 0.96, strand_wave);
    let strand_blur = smoothstep(0.35, 1.7, fwidth(strand_coord));
    let strand = mix(resolved_strand, 0.42, strand_blur);
    // Rays are internal texture within a luminous sheet, not isolated beams.
    // Keeping most energy in the diffuse body produces the soft photographic
    // blend while leaving enough structure to read as an auroral curtain.
    let ray_structure = min(0.58 + 0.30 * cluster + 0.12 * strand, 1.0);

    let h = in.altitude_m;
    let height_above_lower = h - in.lower_boundary_m;
    // A discrete auroral arc can have a definite lower border, but the light
    // still falls away over an altitude band rather than ending at a polygon.
    // The mesh begins four kilometres below the first possible contribution,
    // guaranteeing that its geometric edge remains fully transparent.
    let bottom_fade = smoothstep(-14000.0, 14000.0, height_above_lower);
    let height_above_upper = h - in.upper_boundary_m;
    let top_fade = 1.0 - smoothstep(-35000.0, 35000.0, height_above_upper);
    let vertical_fade = bottom_fade * top_fade;
    // Broad, overlapping profiles approximate the line-of-sight integration
    // through a deep emitting layer. Their overlap blends green through cyan
    // and violet rather than stacking visibly separate colour bands.
    let green_profile = exp(-pow((h - 132000.0) / 62000.0, 2.0));
    let red_profile = smoothstep(165000.0, 210000.0, h)
        * exp(-pow((h - 265000.0) / 120000.0, 2.0));
    let blue_profile = exp(-pow((h - 165000.0) / 100000.0, 2.0));
    let upper_violet_profile = smoothstep(170000.0, 225000.0, h)
        * exp(-pow((h - 250000.0) / 115000.0, 2.0));
    let violet_profile = exp(-pow((height_above_lower + 1500.0) / 10500.0, 2.0));
    let fringe_wave = 0.5 + 0.5 * sin(in.magnetic_angle
        * (13.0 + in.layer) - phase * 7.0 + in.layer * 2.37);
    let violet_activity = smoothstep(0.72, 0.93, fringe_wave);

    // The lower green band is relatively continuous while energetic,
    // high-altitude emission resolves into taller individual rays.
    let ray_separation = smoothstep(135000.0, 245000.0, h);
    let structure = mix(0.82 + 0.18 * ray_structure,
        0.52 + 0.48 * ray_structure, ray_separation);

    // Thin emissive sheets become a little brighter edge-on, but the bounded
    // gain prevents a neon outline when a curtain crosses the horizon.
    let view_direction = normalize(-in.world_pos);
    let edge_on = 1.0 - abs(dot(normalize(in.sheet_normal), view_direction));
    let path_gain = 0.72 + 0.58 * edge_on * edge_on;
    let observer_up = normalize(-g.planet_center.xyz);
    let observer_altitude = max(length(g.planet_center.xyz)
        - ellipsoid_radius(observer_up), 0.0);
    let orbital_restraint = 1.0
        - 0.55 * smoothstep(400000.0, 2500000.0, observer_altitude);
    let intensity = darkness * vertical_fade * structure * slow_pulse
        * longitudinal_activity * in.strength * path_gain * orbital_restraint;

    let green = vec3<f32>(0.055, 0.68, 0.20) * green_profile * 0.060;
    let red = vec3<f32>(0.55, 0.060, 0.055) * red_profile * 0.018;
    let blue = vec3<f32>(0.045, 0.10, 0.50) * blue_profile * 0.010;
    let upper_violet = vec3<f32>(0.30, 0.10, 0.65)
        * upper_violet_profile * (0.028 + 0.010 * cluster);
    let violet = vec3<f32>(0.32, 0.035, 0.38)
        * violet_profile * violet_activity * 0.010;
    let color = (green + red + blue + upper_violet + violet) * intensity;
    if max(color.r, max(color.g, color.b)) < 0.00002 {
        discard;
    }
    return vec4<f32>(color, 1.0);
}
