// Cell-instanced procedural flora. There is deliberately no vertex buffer:
// Indexed vertex_index selects a canonical field candidate and one of the 24
// opaque face vertices, while instance_index selects one
// FloraInstance.

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
    weather0: vec4<f32>,
    weather1: vec4<f32>,
    wind: vec4<f32>,
    surface: vec4<f32>,
    weather2: vec4<f32>,
    weather3: vec4<f32>,
    weather4: vec4<f32>,
    weather5: vec4<f32>,
    weather6: vec4<f32>,
    wind_low: vec4<f32>,
    wind_mid: vec4<f32>,
    wind_upper: vec4<f32>,
    prev_view_proj: mat4x4<f32>,
    camera_to_prev: vec4<f32>,
    temporal: vec4<f32>,
    material_origin: vec4<f32>,
    aerial_sunward: vec4<f32>,
    aerial_side: vec4<f32>,
    aerial_away: vec4<f32>,
    cloud_drift: vec4<f32>,
};

struct FloraInstance {
    // Camera-relative affine transform as three rows; the fourth row is fixed.
    model_x: vec4<f32>,
    model_y: vec4<f32>,
    model_z: vec4<f32>,
    // seed lo/hi, owning cell i/j
    identity: vec4<u32>,
    // packed archetype/phenology/style/calendar, plant LOD, candidate limit, face/level
    state: vec4<u32>,
    // row heading, base density, signed terrain fade, signed plant-LOD fade
    params: vec4<f32>,
    coverage: array<u32, 8>,
    // Owning-cell k=0 base and signed offsets, in centimetres, row-major 5x5.
    ground_base_cm: i32,
    // Twenty-five signed 13-bit centimetre offsets.
    ground_offsets_cm: array<u32, 11>,
    // Bit-cast/packed row spacing and reduced phases at the cell's local origin.
    crop_rows: array<u32, 3>,
    // Explicitly matches FloraInstance's 192-byte storage-array stride.
    padding: u32,
};

struct SunShadow {
    light_view_proj_0: mat4x4<f32>,
    light_view_proj_1: mat4x4<f32>,
    splits: vec4<f32>,
    // x = inverse resolution, y = shadow strength (0 disables), z/w = receiver bias.
    tuning: vec4<f32>,
};


@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<storage, read> flora: array<FloraInstance>;
@group(2) @binding(0) var sun_shadow_tex: texture_depth_2d_array;
@group(2) @binding(1) var sun_shadow_samp: sampler_comparison;
@group(2) @binding(2) var<uniform> sun_shadow: SunShadow;
@group(3) @binding(0) var leaf_atlas: texture_2d_array<f32>;
@group(3) @binding(1) var leaf_sampler: sampler;

struct FloraOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_pos: vec3<f32>,
    @location(3) terrain_fade: f32,
    @location(4) plant_fade: f32,
    @location(5) @interpolate(flat) archetype: u32,
    @location(6) field_uv: vec2<f32>,
    @location(7) @interpolate(flat) field_instance: u32,
};

fn hash_u32(value: u32) -> u32 {
    var x = value;
    x = (x ^ (x >> 16u)) * 0x7feb352du;
    x = (x ^ (x >> 15u)) * 0x846ca68bu;
    return x ^ (x >> 16u);
}

fn candidate_hash(f: FloraInstance, gx: u32, gz: u32, layer: u32, salt: u32) -> u32 {
    var h = f.identity.x ^ (f.identity.y * 0x9e3779b9u);
    h = hash_u32(h ^ (gx * 0x85ebca6bu));
    h = hash_u32(h ^ (gz * 0xc2b2ae35u));
    return hash_u32(h ^ (layer * 0x27d4eb2du) ^ salt);
}

fn hash_unit(value: u32) -> f32 {
    return f32(value >> 8u) * (1.0 / 16777216.0);
}

fn cascade_sun_visibility(world_pos: vec3<f32>, ndl: f32, cascade: u32) -> f32 {
    var light_view_proj = sun_shadow.light_view_proj_0;
    if (cascade == 1u) {
        light_view_proj = sun_shadow.light_view_proj_1;
    }
    let projected = light_view_proj * vec4<f32>(world_pos, 1.0);
    let uv = projected.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if (projected.z <= 0.0 || projected.z >= 1.0
        || any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return 1.0;
    }
    let reference = projected.z
        - sun_shadow.tuning.z
        - sun_shadow.tuning.w * (1.0 - ndl);
    let visibility = textureSampleCompare(
        sun_shadow_tex, sun_shadow_samp, uv, i32(cascade), reference,
    );
    // Match terrain so vegetation receives the same shadow contrast.
    return mix(1.0, visibility, sun_shadow.tuning.y);
}

fn sun_visibility(world_pos: vec3<f32>, ndl: f32) -> f32 {
    if (sun_shadow.tuning.y <= 0.0 || ndl <= 0.0) {
        return 1.0;
    }
    let distance_m = length(world_pos);
    if (distance_m >= sun_shadow.splits.y) {
        return 1.0;
    }
    if (distance_m < sun_shadow.splits.z) {
        return cascade_sun_visibility(world_pos, ndl, 0u);
    }
    if (distance_m < sun_shadow.splits.x) {
        let near_visibility = cascade_sun_visibility(world_pos, ndl, 0u);
        let far_visibility = cascade_sun_visibility(world_pos, ndl, 1u);
        let blend = smoothstep(sun_shadow.splits.z, sun_shadow.splits.x, distance_m);
        return mix(near_visibility, far_visibility, blend);
    }
    let far_visibility = cascade_sun_visibility(world_pos, ndl, 1u);
    let fade = smoothstep(sun_shadow.splits.w, sun_shadow.splits.y, distance_m);
    return mix(far_visibility, 1.0, fade);
}

fn coverage_at(f: FloraInstance, x: u32, z: u32) -> bool {
    let bit = z * 16u + x;
    return (f.coverage[bit >> 5u] & (1u << (bit & 31u))) != 0u;
}

fn model_point(f: FloraInstance, local: vec3<f32>) -> vec3<f32> {
    let point = vec4<f32>(local, 1.0);
    return vec3<f32>(
        dot(f.model_x, point),
        dot(f.model_y, point),
        dot(f.model_z, point),
    );
}

fn model_axis_x(f: FloraInstance) -> vec3<f32> {
    return vec3<f32>(f.model_x.x, f.model_y.x, f.model_z.x);
}

fn model_axis_y(f: FloraInstance) -> vec3<f32> {
    return vec3<f32>(f.model_x.y, f.model_y.y, f.model_z.y);
}

fn model_axis_z(f: FloraInstance) -> vec3<f32> {
    return vec3<f32>(f.model_x.z, f.model_y.z, f.model_z.z);
}

fn ground_offset_cm(f: FloraInstance, index: u32) -> i32 {
    let bit = index * 13u;
    let word = bit >> 5u;
    let shift = bit & 31u;
    var packed = f.ground_offsets_cm[word] >> shift;
    if (shift > 19u) {
        packed |= f.ground_offsets_cm[word + 1u] << (32u - shift);
    }
    let value = packed & 0x1fffu;
    return i32(value << 19u) >> 19;
}

fn ground_height(f: FloraInstance, fx: f32, fz: f32) -> f32 {
    let ground_cm = f32(f.ground_base_cm);
    let grid = clamp(vec2<f32>(fx, fz), vec2<f32>(0.0), vec2<f32>(1.0)) * 4.0;
    let p0 = vec2<u32>(floor(grid));
    let p1 = min(p0 + vec2<u32>(1u), vec2<u32>(4u));
    let t = fract(grid);
    let i00 = p0.y * 5u + p0.x;
    let i10 = p0.y * 5u + p1.x;
    let i01 = p1.y * 5u + p0.x;
    let i11 = p1.y * 5u + p1.x;
    let o00 = ground_offset_cm(f, i00);
    let o10 = ground_offset_cm(f, i10);
    let o01 = ground_offset_cm(f, i01);
    let o11 = ground_offset_cm(f, i11);
    let h00 = (ground_cm + f32(o00)) * 0.01;
    let h10 = (ground_cm + f32(o10)) * 0.01;
    let h01 = (ground_cm + f32(o01)) * 0.01;
    let h11 = (ground_cm + f32(o11)) * 0.01;
    if (t.y <= t.x) {
        return h00 + (h10 - h00) * t.x + (h11 - h10) * t.y;
    }
    return h00 + (h11 - h01) * t.x + (h01 - h00) * t.y;
}

struct CropForm {
    height_min_m: f32,
    height_range_m: f32,
    stem_width_m: f32,
    leaf_width_m: f32,
    head_diameter_m: f32,
    max_head_enlargement: f32,
};

fn crop_form(archetype: u32, lod: u32) -> CropForm {
    var form = CropForm(0.65, 0.45, 0.035, 0.075, 0.12, 2.20);
    if (lod == 0u) {
        switch archetype {
            case 1u: { form = CropForm(0.40, 0.38, 0.055, 0.18, 0.22, 2.00); }
            case 2u: { form = CropForm(1.40, 0.80, 0.060, 0.13, 0.18, 2.10); }
            case 3u: { form = CropForm(0.28, 0.27, 0.050, 0.16, 0.20, 1.90); }
            default: {}
        }
    } else if (lod == 1u) {
        form = CropForm(0.78, 0.34, 0.10, 0.24, 0.36, 1.80);
        switch archetype {
            case 1u: { form = CropForm(0.48, 0.34, 0.12, 0.30, 0.46, 1.70); }
            case 2u: { form = CropForm(1.45, 0.70, 0.12, 0.28, 0.42, 1.80); }
            case 3u: { form = CropForm(0.34, 0.28, 0.10, 0.28, 0.48, 1.65); }
            default: {}
        }
    } else {
        form = CropForm(0.68, 0.28, 0.18, 0.55, 1.30, 1.50);
        switch archetype {
            case 1u: { form = CropForm(0.48, 0.30, 0.18, 0.60, 1.35, 1.45); }
            case 2u: { form = CropForm(0.95, 0.50, 0.20, 0.65, 1.40, 1.45); }
            case 3u: { form = CropForm(0.38, 0.26, 0.18, 0.58, 1.32, 1.45); }
            default: {}
        }
    }
    return form;
}

fn flower_form(archetype: u32, lod: u32) -> CropForm {
    var form = CropForm(0.38, 0.40, 0.045, 0.13, 0.18, 2.20);
    if (lod == 0u) {
        switch archetype {
            case 5u: { form = CropForm(0.30, 0.42, 0.042, 0.14, 0.16, 2.25); }
            case 6u: { form = CropForm(0.22, 0.36, 0.032, 0.10, 0.12, 2.35); }
            default: {}
        }
    } else if (lod == 1u) {
        form = CropForm(0.46, 0.36, 0.11, 0.30, 0.55, 1.75);
        switch archetype {
            case 5u: { form = CropForm(0.38, 0.38, 0.10, 0.28, 0.50, 1.80); }
            case 6u: { form = CropForm(0.30, 0.34, 0.085, 0.24, 0.44, 1.85); }
            default: {}
        }
    } else {
        form = CropForm(0.50, 0.32, 0.22, 0.64, 1.38, 1.45);
        switch archetype {
            case 5u: { form = CropForm(0.44, 0.32, 0.20, 0.60, 1.32, 1.50); }
            case 6u: { form = CropForm(0.36, 0.32, 0.18, 0.54, 1.25, 1.55); }
            default: {}
        }
    }
    return form;
}

struct OrchardForm {
    trunk_height_m: f32,
    trunk_width_m: f32,
    crown_height_m: f32,
    crown_along_m: f32,
    crown_across_m: f32,
};

fn orchard_form(lod: u32) -> OrchardForm {
    if (lod == 0u) {
        return OrchardForm(1.15, 0.18, 1.85, 2.45, 2.15);
    } else if (lod == 1u) {
        return OrchardForm(1.05, 0.28, 1.75, 3.10, 2.45);
    }
    return OrchardForm(0.78, 0.42, 1.62, 5.20, 2.70);
}

struct VineyardForm {
    mass_bottom_m: f32,
    mass_height_m: f32,
    mass_along_m: f32,
    mass_across_m: f32,
    post_height_m: f32,
    post_width_m: f32,
    post_period: u32,
};

fn vineyard_form(lod: u32) -> VineyardForm {
    if (lod == 0u) {
        return VineyardForm(0.52, 0.92, 2.25, 0.62, 1.72, 0.075, 4u);
    } else if (lod == 1u) {
        return VineyardForm(0.46, 0.98, 3.80, 0.78, 1.68, 0.11, 6u);
    }
    return VineyardForm(0.34, 0.90, 6.40, 1.02, 0.0, 0.0, 0u);
}

fn crop_head_scale(form: CropForm, clip_w: f32) -> f32 {
    // g.viewport.y is the physical framebuffer height. This keeps the floor
    // stable when output/render scale changes instead of using CSS points.
    let projected_px = form.head_diameter_m * g.viewport.y
        / (2.0 * max(g.viewport.z, 0.0001) * max(abs(clip_w), 0.001));
    return clamp(1.75 / max(projected_px, 0.0001), 1.0, form.max_head_enlargement);
}

fn face_corner(vertex: u32) -> vec2<f32> {
    let corner = vertex % 4u;
    switch corner {
        case 0u: { return vec2<f32>(-1.0, 0.0); }
        case 1u: { return vec2<f32>( 1.0, 0.0); }
        case 2u: { return vec2<f32>( 1.0, 1.0); }
        default: { return vec2<f32>(-1.0, 1.0); }
    }
}

struct BoxVertex {
    position: vec3<f32>,
    normal: vec3<f32>,
};

// Three crossed, double-sided tapered ribbons reuse the six-quad topology.
// No alpha texture, blending, animation, or per-plant upload is required.
fn crop_ribbon_vertex(vertex: u32, center: vec3<f32>, half_extent: vec3<f32>) -> BoxVertex {
    let face = vertex / 4u;
    let back = (face & 1u) != 0u;
    let blade = face / 2u;
    let corner = face_corner(vertex);
    let u = select(corner.x, -corner.x, back);
    let t = corner.y;
    let axes = array<vec2<f32>, 3>(
        vec2<f32>(1.0, 0.0), vec2<f32>(0.5, 0.8660254), vec2<f32>(-0.5, 0.8660254),
    );
    let axis = axes[blade];
    let width = mix(0.88, 0.24, t);
    let bend = (t * t) * (f32(blade) - 0.7) * 0.16;
    let p = vec3<f32>(axis.x * u * width + bend, t * 2.0 - 1.0,
        axis.y * u * width);
    let n = normalize(vec3<f32>(-axis.y, 0.30, axis.x));
    return BoxVertex(center + p * half_extent, select(n, -n, back));
}

// A narrow crown and sloped shoulders read as a planted row at low angles.
// Keep the ordinary closed topology so distant fields benefit from early Z.
fn crop_canopy_vertex(vertex: u32, center: vec3<f32>, half_extent: vec3<f32>) -> BoxVertex {
    var shape = box_vertex(vertex, vec3<f32>(0.0), vec3<f32>(1.0));
    let top = shape.position.y > 0.0;
    shape.position.z *= select(0.90, 0.22, top);
    shape.position.x *= select(1.0, 0.98, top);
    shape.position.y += select(0.0, -abs(shape.position.x) * 0.12, top);
    shape.position = center + shape.position * half_extent;
    shape.normal = normalize(shape.normal + vec3<f32>(0.0,
        select(0.0, 0.68 * half_extent.z / max(half_extent.y * 2.0, 0.01), abs(shape.normal.z) > 0.5),
        0.0));
    return shape;
}

fn plant_mass_vertex(vertex: u32, center: vec3<f32>, half_extent: vec3<f32>) -> BoxVertex {
    // Six diamond sectors form a closed twelve-triangle plant mass using
    // the same index topology as the stem boxes. Pointed ends and a hexagonal
    // silhouette avoid rectangular flower bands without extra instances.
    let ring = array<vec2<f32>, 6>(
        vec2<f32>(1.0, 0.0), vec2<f32>(0.5, 0.8660254),
        vec2<f32>(-0.5, 0.8660254), vec2<f32>(-1.0, 0.0),
        vec2<f32>(-0.5, -0.8660254), vec2<f32>(0.5, -0.8660254),
    );
    let sector = vertex / 4u;
    let corner = vertex % 4u;
    var p = vec3<f32>(ring[(sector + 1u) % 6u].x, 0.0, ring[(sector + 1u) % 6u].y);
    if (corner == 1u) { p = vec3<f32>(0.0, -1.0, 0.0); }
    if (corner == 2u) { p = vec3<f32>(ring[sector].x, 0.0, ring[sector].y); }
    if (corner == 3u) { p = vec3<f32>(0.0, 1.0, 0.0); }
    return BoxVertex(center + p * half_extent,
        normalize(p / max(half_extent, vec3<f32>(0.001))));
}

// Clipped tea foliage has a broad picking surface and rounded shoulders.
// Keep the closed twelve-triangle topology used by the other crop masses.
fn tea_hedge_vertex(vertex: u32, center: vec3<f32>, half_extent: vec3<f32>) -> BoxVertex {
    var shape = box_vertex(vertex, vec3<f32>(0.0), vec3<f32>(1.0));
    let top = shape.position.y > 0.0;
    shape.position.z *= select(1.0, 0.80, top);
    if (top) {
        shape.normal = normalize(vec3<f32>(shape.normal.x * 0.25, 1.0, shape.position.z * 0.50));
    } else {
        shape.normal = normalize(shape.normal + vec3<f32>(0.0, 0.25, 0.0));
    }
    shape.position = center + shape.position * half_extent;
    return shape;
}

// One closed cuboid: six independently shaded faces, two triangles per face.
// The static index buffer emits each face as 0,1,2,0,2,3. `face_corner` is CCW
// in the local (u,v) frame; every face frame below satisfies
// cross(u, v) == outward normal, so ordinary backface culling works.
fn box_vertex(vertex: u32, center: vec3<f32>, half_extent: vec3<f32>) -> BoxVertex {
    let face = vertex / 4u;
    let corner = face_corner(vertex);
    let u = corner.x;
    let v = corner.y * 2.0 - 1.0;
    var normal = vec3<f32>(1.0, 0.0, 0.0);
    var axis_u = vec3<f32>(0.0, 1.0, 0.0);
    var axis_v = vec3<f32>(0.0, 0.0, 1.0);
    var extent_u = half_extent.y;
    var extent_v = half_extent.z;
    var normal_extent = half_extent.x;
    switch face {
        case 1u: {
            normal = vec3<f32>(-1.0, 0.0, 0.0);
            axis_v = vec3<f32>(0.0, 0.0, -1.0);
        }
        case 2u: {
            normal = vec3<f32>(0.0, 1.0, 0.0);
            axis_u = vec3<f32>(0.0, 0.0, 1.0);
            axis_v = vec3<f32>(1.0, 0.0, 0.0);
            extent_u = half_extent.z;
            extent_v = half_extent.x;
            normal_extent = half_extent.y;
        }
        case 3u: {
            normal = vec3<f32>(0.0, -1.0, 0.0);
            axis_u = vec3<f32>(0.0, 0.0, 1.0);
            axis_v = vec3<f32>(-1.0, 0.0, 0.0);
            extent_u = half_extent.z;
            extent_v = half_extent.x;
            normal_extent = half_extent.y;
        }
        case 4u: {
            normal = vec3<f32>(0.0, 0.0, 1.0);
            axis_u = vec3<f32>(1.0, 0.0, 0.0);
            axis_v = vec3<f32>(0.0, 1.0, 0.0);
            extent_u = half_extent.x;
            extent_v = half_extent.y;
            normal_extent = half_extent.z;
        }
        case 5u: {
            normal = vec3<f32>(0.0, 0.0, -1.0);
            axis_u = vec3<f32>(1.0, 0.0, 0.0);
            axis_v = vec3<f32>(0.0, -1.0, 0.0);
            extent_u = half_extent.x;
            extent_v = half_extent.y;
            normal_extent = half_extent.z;
        }
        default: {}
    }
    return BoxVertex(
        center + normal * normal_extent + axis_u * u * extent_u + axis_v * v * extent_v,
        normal,
    );
}

// Mirrors flower_destinations::FLOWER_DESTINATIONS. Keeping the three-color
// authored palette behind the existing style byte adds no instance payload,
// binding, draw, texture or candidate work.
fn curated_flower_color(style: u32, variant: u32) -> vec3<f32> {
    var a = vec3<f32>(0.88, 0.16, 0.28);
    var b = vec3<f32>(0.96, 0.76, 0.16);
    var c = vec3<f32>(0.48, 0.20, 0.70);
    switch style {
        case 1u: { a=vec3<f32>(0.392,0.176,0.608); b=vec3<f32>(0.961,0.804,0.176); c=vec3<f32>(0.882,0.275,0.451); }
        case 2u: { a=vec3<f32>(0.933,0.204,0.345); b=vec3<f32>(0.973,0.824,0.176); c=vec3<f32>(0.455,0.255,0.706); }
        case 3u: { a=vec3<f32>(0.314,0.569,0.882); b=vec3<f32>(0.882,0.196,0.176); c=vec3<f32>(0.961,0.902,0.804); }
        case 4u: { a=vec3<f32>(0.439,0.243,0.667); b=vec3<f32>(0.804,0.608,0.882); c=vec3<f32>(0.961,0.863,0.949); }
        case 5u: { a=vec3<f32>(0.902,0.137,0.314); b=vec3<f32>(0.973,0.745,0.118); c=vec3<f32>(0.518,0.216,0.706); }
        case 6u: { a=vec3<f32>(0.882,0.137,0.275); b=vec3<f32>(0.980,0.804,0.137); c=vec3<f32>(0.471,0.176,0.686); }
        case 7u: { a=vec3<f32>(0.361,0.188,0.569); b=vec3<f32>(0.494,0.294,0.686); c=vec3<f32>(0.725,0.529,0.804); }
        case 8u: { a=vec3<f32>(0.337,0.176,0.529); b=vec3<f32>(0.478,0.290,0.647); c=vec3<f32>(0.698,0.518,0.765); }
        case 9u: { a=vec3<f32>(0.863,0.157,0.275); b=vec3<f32>(0.949,0.557,0.137); c=vec3<f32>(0.961,0.843,0.216); }
        case 10u: { a=vec3<f32>(0.878,0.137,0.255); b=vec3<f32>(0.961,0.745,0.098); c=vec3<f32>(0.529,0.165,0.686); }
        case 11u: { a=vec3<f32>(0.882,0.149,0.275); b=vec3<f32>(0.980,0.804,0.118); c=vec3<f32>(0.455,0.196,0.706); }
        case 12u: { a=vec3<f32>(0.882,0.137,0.282); b=vec3<f32>(0.961,0.729,0.125); c=vec3<f32>(0.522,0.188,0.698); }
        case 13u: { a=vec3<f32>(0.922,0.176,0.373); b=vec3<f32>(0.980,0.706,0.137); c=vec3<f32>(0.557,0.216,0.725); }
        case 14u: { a=vec3<f32>(0.882,0.216,0.412); b=vec3<f32>(0.949,0.725,0.216); c=vec3<f32>(0.431,0.282,0.686); }
        case 15u: { a=vec3<f32>(0.886,0.149,0.282); b=vec3<f32>(0.961,0.773,0.137); c=vec3<f32>(0.463,0.204,0.706); }
        case 16u: { a=vec3<f32>(0.965,0.804,0.094); b=vec3<f32>(0.922,0.698,0.071); c=vec3<f32>(1.000,0.882,0.243); }
        case 17u: { a=vec3<f32>(0.965,0.812,0.098); b=vec3<f32>(0.910,0.690,0.063); c=vec3<f32>(1.000,0.894,0.275); }
        case 18u: { a=vec3<f32>(0.969,0.812,0.102); b=vec3<f32>(0.922,0.710,0.078); c=vec3<f32>(1.000,0.898,0.294); }
        case 19u: { a=vec3<f32>(0.784,0.157,0.275); b=vec3<f32>(0.922,0.765,0.165); c=vec3<f32>(0.373,0.353,0.725); }
        case 20u: { a=vec3<f32>(0.933,0.439,0.094); b=vec3<f32>(0.957,0.745,0.137); c=vec3<f32>(0.902,0.275,0.282); }
        case 21u: { a=vec3<f32>(0.933,0.408,0.098); b=vec3<f32>(0.961,0.753,0.157); c=vec3<f32>(0.878,0.275,0.322); }
        case 22u: { a=vec3<f32>(0.361,0.361,0.784); b=vec3<f32>(0.878,0.314,0.529); c=vec3<f32>(0.957,0.745,0.188); }
        case 23u: { a=vec3<f32>(0.463,0.298,0.745); b=vec3<f32>(0.886,0.314,0.557); c=vec3<f32>(0.961,0.769,0.216); }
        case 24u: { a=vec3<f32>(0.949,0.400,0.055); b=vec3<f32>(0.969,0.616,0.094); c=vec3<f32>(0.933,0.776,0.216); }
        case 25u: { a=vec3<f32>(0.933,0.439,0.071); b=vec3<f32>(0.965,0.773,0.137); c=vec3<f32>(0.439,0.298,0.737); }
        case 26u: { a=vec3<f32>(0.933,0.373,0.086); b=vec3<f32>(0.961,0.745,0.165); c=vec3<f32>(0.714,0.282,0.659); }
        case 27u: { a=vec3<f32>(0.188,0.298,0.690); b=vec3<f32>(0.345,0.439,0.804); c=vec3<f32>(0.933,0.902,0.863); }
        case 28u: { a=vec3<f32>(0.400,0.251,0.706); b=vec3<f32>(0.824,0.412,0.686); c=vec3<f32>(0.922,0.745,0.863); }
        case 29u: { a=vec3<f32>(0.824,0.294,0.510); b=vec3<f32>(0.922,0.765,0.212); c=vec3<f32>(0.361,0.384,0.765); }
        case 30u: { a=vec3<f32>(0.804,0.243,0.529); b=vec3<f32>(0.933,0.412,0.682); c=vec3<f32>(0.949,0.745,0.216); }
        default: {}
    }
    return select(select(b, c, variant == 2u), a, variant == 0u);
}

// Mirrors palette::field::FLOWER and flora::generic_flower_palette_linear.
// The third band is the exact midpoint of the two authored colors.
fn generic_flower_color(palette: u32, variant: u32) -> vec3<f32> {
    var a = vec3<f32>(0.796078, 0.227451, 0.254902);
    var b = vec3<f32>(0.925490, 0.690196, 0.211765);
    var c = vec3<f32>(0.860784, 0.458824, 0.233333);
    switch palette & 3u {
        case 1u: {
            a = vec3<f32>(0.592157, 0.325490, 0.682353);
            b = vec3<f32>(0.909804, 0.670588, 0.752941);
            c = vec3<f32>(0.750980, 0.498039, 0.717647);
        }
        case 2u: {
            a = vec3<f32>(0.905882, 0.760784, 0.243137);
            b = vec3<f32>(0.898039, 0.913725, 0.862745);
            c = vec3<f32>(0.901961, 0.837255, 0.552941);
        }
        case 3u: {
            a = vec3<f32>(0.847059, 0.333333, 0.188235);
            b = vec3<f32>(0.466667, 0.356863, 0.686275);
            c = vec3<f32>(0.656863, 0.345098, 0.437255);
        }
        default: {}
    }
    return select(select(b, c, variant == 2u), a, variant == 0u);
}

fn flower_head_color(
    f: FloraInstance,
    archetype: u32,
    band_phase: f32,
    gx: u32,
    gz: u32,
    layer: u32,
) -> vec3<f32> {
    var variant = 0u;
    if (archetype == 4u) {
        variant = min(u32(floor(fract(band_phase) * 3.0)), 2u);
    } else if (archetype == 5u) {
        variant = candidate_hash(f, gx, gz, layer, 0x70616c65u) % 3u;
    } else {
        // A whole irregular cluster shares a restrained color identity.
        variant = candidate_hash(f, gx >> 3u, gz >> 3u, 0u, 0x636c7573u) % 3u;
    }
    let curated_style = (f.state.x >> 16u) & 0xffu;
    if (curated_style != 0u) {
        return curated_flower_color(curated_style, variant);
    }
    let palette = (f.state.w >> 16u) & 0xffu;
    var color = generic_flower_color(palette, variant);
    if (archetype == 6u) {
        // Natural flowers stay quieter than commercial strips and formal beds.
        color = mix(color, vec3<f32>(0.76, 0.70, 0.48), 0.24);
    }
    return color;
}

struct FloraSeasonState {
    height: f32,
    density: f32,
    bloom: f32,
    maturity: f32,
    senescence: f32,
    stubble: f32,
    dormancy: f32,
};

fn cyclic_window(phase_in: f32, start_in: f32, end_in: f32, feather: f32) -> f32 {
    let phase = fract(phase_in);
    let start = fract(start_in);
    let end = fract(end_in);
    if (start <= end) {
        return smoothstep(start - feather, start + feather, phase)
            * (1.0 - smoothstep(end - feather, end + feather, phase));
    }
    return clamp(
        smoothstep(start - feather, start + feather, phase)
            + (1.0 - smoothstep(end - feather, end + feather, phase)),
        0.0,
        1.0,
    );
}

fn calendar_offset(calendar: u32, encoded: u32) -> f32 {
    var maximum = 0.055;
    switch calendar {
        case 1u: { maximum = 0.035; }
        case 2u: { maximum = 0.045; }
        case 3u: { maximum = 0.035; }
        case 4u: { maximum = 0.025; }
        case 5u: { maximum = 0.045; }
        case 6u: { maximum = 0.045; }
        case 7u: { maximum = 0.5; }
        default: {}
    }
    return (f32(encoded) * (2.0 / 255.0) - 1.0) * maximum;
}

// Mirrors vearth_gen::flora_season_state. Temperate/perennial calendars use
// the already hemisphere-correct local phase. Monsoonal and tropical fields
// use the astronomical clock plus observed moisture, avoiding a latitude-only
// six-month inversion.
fn flora_season_state(calendar: u32, encoded_offset: u32) -> FloraSeasonState {
    let offset = calendar_offset(calendar, encoded_offset);
    let local = fract(g.surface.x + offset + 1.0);
    let astronomical = fract(g.weather6.w + offset + 1.0);
    // Wet-hour frequency is not soil moisture. Humid growing regions retain
    // plant water between rain events; explicit drought still dries them.
    let humid_water = smoothstep(0.42, 0.78, g.weather0.y) * (1.0 - g.weather4.x);
    let wet = clamp(max(max(g.weather1.x, g.weather6.y), humid_water), 0.0, 1.0);
    var emergence = 0.0;
    var vegetative = 0.0;
    var bloom = 0.0;
    var maturity = 0.0;
    var senescence = 0.0;
    var stubble = 0.0;
    var dormancy = 0.0;
    switch calendar {
        case 1u: { // cool short season
            emergence = cyclic_window(local, 0.20, 0.34, 0.035);
            vegetative = cyclic_window(local, 0.27, 0.56, 0.040);
            bloom = cyclic_window(local, 0.40, 0.56, 0.035);
            maturity = cyclic_window(local, 0.50, 0.67, 0.035);
            senescence = cyclic_window(local, 0.63, 0.77, 0.035);
            stubble = cyclic_window(local, 0.72, 0.93, 0.035);
            dormancy = cyclic_window(local, 0.86, 0.23, 0.040);
        }
        case 2u: { // Mediterranean winter crop
            emergence = cyclic_window(local, 0.68, 0.84, 0.040);
            vegetative = cyclic_window(local, 0.76, 0.30, 0.055);
            bloom = cyclic_window(local, 0.12, 0.31, 0.040);
            maturity = cyclic_window(local, 0.25, 0.44, 0.040);
            senescence = cyclic_window(local, 0.40, 0.57, 0.040);
            stubble = cyclic_window(local, 0.52, 0.72, 0.040);
            dormancy = cyclic_window(local, 0.58, 0.76, 0.045);
        }
        case 3u: { // monsoonal wet-season crop
            let wet_growth = smoothstep(0.22, 0.62, wet);
            let wet_bloom = smoothstep(0.34, 0.72, wet);
            let drying = 1.0 - smoothstep(0.16, 0.46, wet);
            emergence = wet_growth * cyclic_window(astronomical, 0.02, 0.26, 0.08);
            vegetative = wet_growth;
            bloom = wet_bloom * cyclic_window(astronomical, 0.16, 0.48, 0.10);
            maturity = wet_growth * cyclic_window(astronomical, 0.38, 0.72, 0.08);
            senescence = drying;
            stubble = drying * cyclic_window(astronomical, 0.58, 0.96, 0.08);
            dormancy = drying;
        }
        case 4u: { // tropical multi-cycle
            let cycle = fract(astronomical * 2.0);
            let moisture_gate = smoothstep(0.12, 0.48, wet);
            emergence = cyclic_window(cycle, 0.00, 0.20, 0.06) * moisture_gate;
            vegetative = cyclic_window(cycle, 0.10, 0.62, 0.07) * moisture_gate;
            bloom = cyclic_window(cycle, 0.32, 0.56, 0.055) * moisture_gate;
            maturity = cyclic_window(cycle, 0.50, 0.76, 0.055) * moisture_gate;
            senescence = cyclic_window(cycle, 0.72, 0.92, 0.055);
            stubble = cyclic_window(cycle, 0.86, 0.08, 0.055);
            dormancy = (1.0 - moisture_gate) * 0.80;
        }
        case 5u: { // perennial orchard
            emergence = cyclic_window(local, 0.10, 0.24, 0.035);
            vegetative = cyclic_window(local, 0.18, 0.72, 0.055);
            bloom = cyclic_window(local, 0.12, 0.29, 0.035);
            maturity = cyclic_window(local, 0.42, 0.70, 0.055);
            senescence = cyclic_window(local, 0.66, 0.86, 0.045);
            dormancy = cyclic_window(local, 0.84, 0.18, 0.050);
        }
        case 6u: { // perennial vineyard
            emergence = cyclic_window(local, 0.16, 0.30, 0.035);
            vegetative = cyclic_window(local, 0.23, 0.75, 0.055);
            bloom = cyclic_window(local, 0.34, 0.50, 0.040);
            maturity = cyclic_window(local, 0.52, 0.72, 0.050);
            senescence = cyclic_window(local, 0.68, 0.88, 0.045);
            dormancy = cyclic_window(local, 0.86, 0.20, 0.050);
        }
        default: { // temperate summer and curated bloom
            emergence = cyclic_window(local, 0.10, 0.28, 0.045);
            vegetative = cyclic_window(local, 0.18, 0.58, 0.055);
            bloom = cyclic_window(local, 0.30, 0.54, 0.045);
            maturity = cyclic_window(local, 0.48, 0.70, 0.045);
            senescence = cyclic_window(local, 0.65, 0.84, 0.045);
            stubble = cyclic_window(local, 0.78, 0.96, 0.035);
            dormancy = cyclic_window(local, 0.90, 0.14, 0.045);
        }
    }
    let live = clamp(emergence * 0.35 + max(vegetative, maturity) + bloom * 0.18, 0.0, 1.0);
    let drought = clamp(g.weather4.x, 0.0, 1.0);
    let snow = clamp(g.weather1.y, 0.0, 1.0);
    // Lifecycle, not the current hourly temperature, owns planted structure.
    // Emergence is dense and short; senescence keeps stalks; harvest keeps
    // stubble. Frost and snow remain transient appearance inputs.
    let emerging_structure = smoothstep(0.04, 0.22, emergence);
    let standing_structure = max(
        max(max(emerging_structure, vegetative), max(bloom, maturity)),
        max(senescence, stubble),
    );
    let structural_height = max(max(live, senescence * 0.82), stubble * 0.15);
    return FloraSeasonState(
        (0.08 + structural_height * 0.92) * (1.0 - drought * 0.45) * (1.0 - snow * 0.75),
        (0.08 + standing_structure * 0.92) * (1.0 - drought * 0.70),
        bloom * (1.0 - drought * 0.85) * (1.0 - snow),
        maturity * (1.0 - snow * 0.88),
        max(senescence, drought * 0.72),
        stubble,
        max(dormancy, snow),
    );
}

// Row colors from palette::field::CROP. They are carried by the same palette
// byte as the terrain treatment, so generic crop crowns cannot select an
// unrelated shader-only family.
fn crop_field_color(palette: u32) -> vec3<f32> {
    var color = vec3<f32>(0.423529, 0.501961, 0.298039);
    switch palette & 3u {
        case 1u: { color = vec3<f32>(0.568627, 0.564706, 0.360784); }
        case 2u: { color = vec3<f32>(0.337255, 0.454902, 0.329412); }
        case 3u: { color = vec3<f32>(0.454902, 0.533333, 0.364706); }
        default: {}
    }
    return color;
}

fn archetype_color(
    f: FloraInstance,
    archetype: u32,
    head: bool,
    shade: f32,
    season: FloraSeasonState,
    band_phase: f32,
    gx: u32,
    gz: u32,
    layer: u32,
) -> vec3<f32> {
    var stem = vec3<f32>(0.27, 0.39, 0.23);
    var crown = vec3<f32>(0.49, 0.57, 0.34);
    let palette = (f.state.w >> 16u) & 0xffu;
    let field_crop = crop_field_color(palette);
    switch archetype {
        case 0u: { crown = mix(field_crop, vec3<f32>(0.69, 0.60, 0.36), 0.22); } // cereal
        case 1u: { crown = mix(field_crop, vec3<f32>(0.35, 0.50, 0.27), 0.18); }
        case 2u: {
            stem = vec3<f32>(0.24, 0.36, 0.22);
            crown = mix(field_crop, vec3<f32>(0.53, 0.48, 0.29), 0.20);
        }
        case 3u: { crown = mix(field_crop, vec3<f32>(0.38, 0.55, 0.30), 0.16); }
        case 4u: { stem = vec3<f32>(0.25, 0.39, 0.23); }
        case 5u: { stem = vec3<f32>(0.23, 0.36, 0.23); }
        case 6u: { stem = vec3<f32>(0.28, 0.37, 0.25); }
        case 7u: { stem = vec3<f32>(0.40, 0.33, 0.25); crown = vec3<f32>(0.34, 0.46, 0.28); }
        default: { stem = vec3<f32>(0.37, 0.31, 0.23); crown = vec3<f32>(0.30, 0.42, 0.26); }
    }
    if (head && archetype >= 4u && archetype <= 6u) {
        crown = flower_head_color(f, archetype, band_phase, gx, gz, layer);
    }
    stem = mix(stem, vec3<f32>(0.49, 0.48, 0.29), season.senescence * 0.62);
    crown = mix(crown, vec3<f32>(0.70, 0.60, 0.35), season.maturity * 0.46);
    crown = mix(crown, vec3<f32>(0.57, 0.40, 0.25), season.senescence * 0.78);
    if (archetype == 7u && head) {
        crown = mix(crown, vec3<f32>(0.94, 0.75, 0.80), season.bloom * 0.88);
        crown = mix(crown, vec3<f32>(0.67, 0.34, 0.21), season.senescence * 0.82);
    } else if (archetype == 8u && head) {
        crown = mix(crown, vec3<f32>(0.59, 0.32, 0.23), season.senescence * 0.76);
    }
    var color = select(stem, crown, head);
    if (head && archetype >= 4u && archetype <= 6u) {
        // Large mapped flower fields retain their authored hue bands without
        // giving every head the chroma reserved for a small visual accent.
        let value = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
        color = mix(vec3<f32>(value), color, 0.72);
    }
    color = mix(color, vec3<f32>(0.51, 0.44, 0.30), season.stubble * 0.72);
    color = mix(color, vec3<f32>(0.39, 0.33, 0.24), season.dormancy * 0.58);
    if (archetype <= 3u && palette >= 16u) {
        let treatment = crop_treatment(palette);
        color = mix(treatment.green, treatment.mature, season.maturity);
        color = mix(color, mix(treatment.mature, treatment.soil, 0.45), season.senescence * 0.72);
        color = mix(color, treatment.soil * 1.08, max(season.stubble * 0.78, season.dormancy * 0.92));
    }
    // These palettes, including the authored flower tables, are display sRGB.
    // Decode once before the shared linear lighting, as terrain vertices do.
    // Ian Taylor's cubic avoids three powers per procedural vertex:
    // https://chilliant.blogspot.com/2012/08/srgb-approximations-for-hlsl.html
    let srgb = clamp(color * mix(0.94, 1.06, shade), vec3<f32>(0.0), vec3<f32>(1.0));
    let decode_coefficients = vec3<f32>(0.305306011, 0.682171111, 0.012522878);
    return srgb * (srgb * (srgb * decode_coefficients.x
        + vec3<f32>(decode_coefficients.y)) + vec3<f32>(decode_coefficients.z));
}

fn absent_flora(terrain_fade: f32, plant_fade: f32, archetype: u32) -> FloraOut {
    var out: FloraOut;
    // All vertices of the rejected primitive share this finite position,
    // outside WebGPU's zero-to-one clip-depth interval.
    out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    out.color = vec3<f32>(0.0);
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
    out.world_pos = vec3<f32>(0.0);
    out.terrain_fade = terrain_fade;
    out.plant_fade = plant_fade;
    out.archetype = archetype;
    out.field_uv = vec2<f32>(0.0);
    out.field_instance = 0u;
    return out;
}

@vertex
fn vs_flora(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> FloraOut {
    let f = flora[instance_index];
    let packed = f.state.x;
    let archetype = packed & 0xffu;
    let phenology = (packed >> 8u) & 0xffu;
    let calendar = (packed >> 24u) & 0xffu;
    let lod = f.state.y;
    let compound_middle = archetype >= 7u && lod == 1u;
    let vertices_per_glyph = select(24u, 48u, lod == 0u || compound_middle);
    let glyph = vertex_index / vertices_per_glyph;
    if (glyph >= f.state.z) {
        return absent_flora(f.params.z, f.params.w, archetype);
    }
    let primitive_vertex = vertex_index % vertices_per_glyph;
    let level = (f.state.w >> 8u) & 0xffu;
    let shift = min(select(0u, 19u - level, level < 19u), 7u);
    let edge = 16u << shift;
    let total = edge * edge;
    // Odd multiplication permutes every power-of-two lattice. A parent
    // therefore enumerates a deterministic subset of the exact candidates
    // enumerated by its children instead of reorganizing the field.
    let flat = (glyph * 40503u) % total;
    let layer = glyph / total;
    let lx = flat % edge;
    let lz = flat / edge;
    let gx = (f.identity.z << shift) * 16u + lx;
    // Cell j follows cube-face +v while ChunkFrame local +z is -v.
    let reference_cells = 1u << (level + shift);
    let gz = (reference_cells - (f.identity.w + 1u) * (1u << shift)) * 16u + lz;
    // Multiple dense strata occupy stable, distinct subslots instead of
    // overlapping the same two positions when commercial density increases.
    let subslot = layer & 3u;
    let layer_offset_x = select(-0.27, 0.27, (subslot & 1u) != 0u);
    let layer_offset_z = select(-0.27, 0.27, (subslot & 2u) != 0u);
    var fx = (f32(lx) + 0.5 + layer_offset_x) / f32(edge);
    var fz = (f32(lz) + 0.5 + layer_offset_z) / f32(edge);
    let sx = max(length(model_axis_x(f)), 0.001);
    let sy = max(length(model_axis_y(f)), 0.001);
    let sz = max(length(model_axis_z(f)), 0.001);
    let heading = f.params.x;
    // Surface-field north follows +v; ChunkFrame local +z follows -v.
    let row_axis = vec2<f32>(cos(heading), -sin(heading));
    let cross_axis = vec2<f32>(-row_axis.y, row_axis.x);
    var row_tangent = row_axis;
    let crop = archetype <= 3u;
    let flower = archetype >= 4u && archetype <= 6u;
    let flower_crop = archetype == 4u;
    let orchard = archetype == 7u;
    let vineyard = archetype == 8u;
    let managed = orchard || vineyard;
    let canopy_surface = crop && lod >= 2u;
    let canopy_columns = select(8u, CROP_DISTANT_GRID_EDGE, lod == 3u);
    let canopy_rows = select(8u, CROP_DISTANT_GRID_EDGE, lod == 3u);
    if (canopy_surface) {
        fx = (f32(glyph % canopy_columns) + 0.5) / f32(canopy_columns);
        fz = (f32(glyph / canopy_columns) + 0.5) / f32(canopy_rows);
    }
    var season = flora_season_state(calendar, phenology);
    let palette = (f.state.w >> 16u) & 0xffu;
    let evergreen_hedge = crop && palette >= 16u && crop_treatment(palette).evergreen;
    if (evergreen_hedge) {
        // Repeated picking trims an evergreen tea hedge; it does not harvest
        // the woody plant to cereal stubble each winter.
        season = FloraSeasonState(
            0.94 * (1.0 - g.weather1.y * 0.45),
            1.0 - g.weather4.x * 0.40, 0.0, 0.0,
            g.weather4.x * 0.50, 0.0, g.weather1.y * 0.60,
        );
    }
    let cultivated_rows = crop || flower_crop || managed;
    let nominal_cell_m = 10007557.221 / exp2(f32(level));
    // Aggregate solids cover the same conceptual world-area density even
    // when the terrain cut supplies a coarser owner cell.
    let terrain_lod_scale =
        clamp(exp2(f32(19u - min(level, 19u)) * 0.5), 1.0, 4.0);
    let local_east_m = fx * nominal_cell_m;
    let local_north_m = -fz * nominal_cell_m;
    var flower_band_phase = 0.0;
    var managed_inside = true;
    var slot_gx = gx;
    var slot_gz = gz;
    if (cultivated_rows && !canopy_surface) {
        // Snap the canonical candidate to a gently perturbed field-wide row.
        // CPU-reduced origin phases retain the f64 surface equation without
        // performing multi-million-metre face-coordinate arithmetic in f32.
        let row_spacing_m = bitcast<f32>(f.crop_rows[0]);
        let row_origin_phase = bitcast<f32>(f.crop_rows[1]);
        let perturb_origin_phase = bitcast<f32>(f.crop_rows[2]);
        let along = local_east_m * cos(heading) + local_north_m * sin(heading);
        let across = -local_east_m * sin(heading) + local_north_m * cos(heading);
        var wobble_m = 0.0;
        var along_delta_m = 0.0;
        if (crop) {
            let bend_phase = (along / 48.0 + perturb_origin_phase) * 6.28318530718;
            let bend_m = select(0.16, 2.2, evergreen_hedge);
            wobble_m =
                sin(bend_phase) * bend_m;
            if (evergreen_hedge) {
                row_tangent = normalize(row_axis - cross_axis * cos(bend_phase) * bend_m * (6.28318530718 / 48.0));
            }
        } else if (flower_crop) {
            let packed_band = f.crop_rows[2];
            let band_spacing_m = f32(packed_band & 0xffffu) * 0.01;
            let band_origin_phase = f32(packed_band >> 16u) / 65536.0;
            flower_band_phase = fract(across / max(band_spacing_m, 0.01) + band_origin_phase);
        } else {
            let packed_along = f.crop_rows[2];
            let along_spacing_m = f32(packed_along & 0xffffu) * 0.01;
            let along_origin_phase = f32(packed_along >> 16u) / 65536.0;
            let along_center_m = (
                floor(along / along_spacing_m + along_origin_phase)
                + 0.5 - along_origin_phase
            ) * along_spacing_m;
            along_delta_m = along_center_m - along;
        }
        let row_center_m = (
            floor((across - wobble_m) / row_spacing_m + row_origin_phase)
            + 0.5 - row_origin_phase
        ) * row_spacing_m + wobble_m;
        let within_row_m = select(
            (hash_unit(candidate_hash(f, gx, gz, layer, 0x726f776fu)) - 0.5) * 0.68,
            0.0,
            managed || evergreen_hedge,
        );
        let row_delta_m = clamp(
            row_center_m + within_row_m - across,
            -row_spacing_m * 0.50,
            row_spacing_m * 0.50,
        );
        let east_delta_m =
            cos(heading) * along_delta_m - sin(heading) * row_delta_m;
        let north_delta_m =
            sin(heading) * along_delta_m + cos(heading) * row_delta_m;
        let snapped_fx = fx + east_delta_m / (sx * 32.0);
        let snapped_fz = fz - north_delta_m / (sz * 32.0);
        managed_inside = !managed
            || (snapped_fx >= 0.0 && snapped_fx <= 1.0
                && snapped_fz >= 0.0 && snapped_fz <= 1.0);
        fx = clamp(snapped_fx, 0.0, 1.0);
        fz = clamp(snapped_fz, 0.0, 1.0);
        if (managed) {
            // Hash the snapped world slot, not the source candidate. Duplicate
            // candidates collapse to one identical managed tree/vine form,
            // including at cell and terrain-LOD boundaries.
            slot_gx = (f.identity.z << shift) * 16u + u32(round(fx * f32(edge)));
            slot_gz = (reference_cells - (f.identity.w + 1u) * (1u << shift)) * 16u
                + u32(round(fz * f32(edge)));
        }
    } else if (flower) {
        // Formal beds and meadow flowers break the canonical lattice with a
        // stable sub-cell jitter. Their coverage remains cell/LOD coherent.
        let jitter_x = hash_unit(candidate_hash(f, gx, gz, layer, 0x6a697478u)) - 0.5;
        let jitter_z = hash_unit(candidate_hash(f, gx, gz, layer, 0x6a69747au)) - 0.5;
        fx = clamp(fx + jitter_x * 0.72 / f32(edge), 0.0, 1.0);
        fz = clamp(fz + jitter_z * 0.72 / f32(edge), 0.0, 1.0);
    }

    let mask_x = min(u32(floor(fx * 16.0)), 15u);
    let mask_z = min(u32(floor(fz * 16.0)), 15u);
    let visual_gx = select(gx, slot_gx, managed);
    let visual_gz = select(gz, slot_gz, managed);
    let visual_layer = select(layer, 0u, managed);
    let existence =
        hash_unit(candidate_hash(f, visual_gx, visual_gz, visual_layer, 0x65786973u));
    var existence_threshold = f.params.y;
    if (!managed) {
        existence_threshold *= season.density;
    }
    if (archetype == 6u) {
        let cluster =
            hash_unit(candidate_hash(f, gx >> 3u, gz >> 3u, 0u, 0x64656e73u));
        existence_threshold *= mix(0.12, 1.0, smoothstep(0.30, 0.82, cluster));
    }
    let exists = glyph < f.state.z
        && managed_inside
        && (canopy_surface || coverage_at(f, mask_x, mask_z))
        && (canopy_surface || existence < existence_threshold);
    if (!exists) {
        return absent_flora(f.params.z, f.params.w, archetype);
    }
    let root_y_m = ground_height(f, fx, fz) - 0.035;
    let individual =
        hash_unit(candidate_hash(f, visual_gx, visual_gz, visual_layer, 0x73686164u));
    // Coarse canonical-lattice patches add restrained spatial coherence
    // without unstable large-coordinate trigonometry.
    let height_patch =
        hash_unit(candidate_hash(f, visual_gx >> 4u, visual_gz >> 4u, 0u, 0x68656967u))
            * 2.0 - 1.0;
    let shade_patch =
        hash_unit(candidate_hash(f, visual_gx >> 5u, visual_gz >> 5u, 0u, 0x70617463u))
            * 2.0 - 1.0;
    let height_scale = clamp(0.88 + individual * 0.18 + height_patch * 0.045, 0.82, 1.12);
    let shade = clamp(0.42 + individual * 0.34 + shade_patch * 0.12, 0.22, 0.88);

    var height_m = (0.72 + individual * 0.38) * height_scale;
    var stem_width_m = 0.055;
    var leaf_width_m = 0.10;
    var head_diameter_m = 0.20;
    var head_enlargement = 1.0;
    if (crop || flower) {
        var form = crop_form(archetype, lod);
        if (flower) {
            form = flower_form(archetype, lod);
        }
        height_m = (form.height_min_m + individual * form.height_range_m) * height_scale;
        height_m *= season.height;
        if (canopy_surface || evergreen_hedge) {
            height_m = (form.height_min_m + form.height_range_m * 0.5) * season.height;
        }
        if (crop && ((f.state.w >> 16u) & 0xffu) >= 16u) {
            height_m *= crop_treatment((f.state.w >> 16u) & 0xffu).height;
        }
        stem_width_m = form.stem_width_m;
        leaf_width_m = form.leaf_width_m;
        head_diameter_m = form.head_diameter_m;
        let head_center_local = vec3<f32>(
            fx * 32.0,
            root_y_m / sy + (height_m - head_diameter_m * 0.10) / sy,
            fz * 32.0,
        );
        let head_center_world = model_point(f, head_center_local);
        head_enlargement = crop_head_scale(
            form,
            (g.view_proj * vec4<f32>(head_center_world, 1.0)).w,
        );
    } else if (orchard) {
        let form = orchard_form(lod);
        height_m = form.trunk_height_m + form.crown_height_m;
        stem_width_m = form.trunk_width_m;
        leaf_width_m = form.crown_across_m;
        head_diameter_m = form.crown_along_m;
    } else if (vineyard) {
        let form = vineyard_form(lod);
        height_m = form.mass_bottom_m + form.mass_height_m;
        stem_width_m = form.post_width_m;
        leaf_width_m = form.mass_across_m;
        head_diameter_m = form.mass_along_m;
    }

    let primitive = primitive_vertex / 24u;
    let box_corner_vertex = primitive_vertex % 24u;
    var horizontal = row_tangent;
    var shape_center_m = vec3<f32>(0.0, height_m * 0.5, 0.0);
    var shape_half_m = vec3<f32>(
        stem_width_m * 0.5,
        max(height_m * 0.5, 0.01),
        stem_width_m * 0.5,
    );
    var head = false;
    var primitive_exists = exists;
    let aggregate = lod >= 2u || ((crop || flower) && lod >= 1u);
    if (flower) {
        let form = flower_form(archetype, lod);
        let head_size = form.head_diameter_m * head_enlargement;
        if (aggregate) {
            let head_presence = season.bloom;
            head = hash_unit(candidate_hash(
                f, visual_gx, visual_gz, visual_layer, 0x68656164u,
            )) < head_presence;
            let distance_scale = select(1.45, 1.65, lod == 3u);
            let aggregate_along_m = select(
                head_size * distance_scale * terrain_lod_scale,
                clamp(
                    nominal_cell_m * 0.30,
                    head_size * distance_scale * terrain_lod_scale,
                    160.0,
                ),
                lod == 3u,
            );
            let aggregate_across_m = select(
                head_size * distance_scale * terrain_lod_scale * 0.32,
                clamp(
                    // Distant crop/flower cells use twelve rather than sixteen
                    // solids. The wider band preserves their total projected
                    // field coverage while reducing vertex work by 25%.
                    nominal_cell_m * 0.067,
                    head_size * distance_scale * terrain_lod_scale * 0.32,
                    28.0,
                ),
                lod == 3u,
            );
            shape_center_m = vec3<f32>(0.0, max(height_m * 0.55, 0.12), 0.0);
            shape_half_m = vec3<f32>(
                aggregate_along_m * 0.50,
                max(height_m * 0.34, 0.10),
                aggregate_across_m * 0.50,
            );
        } else {
            let side = select(-1.0, 1.0, primitive == 1u);
            let flower_head_present = hash_unit(candidate_hash(
                f, visual_gx, visual_gz, visual_layer,
                0x68656164u + primitive,
            )) < season.bloom;
            head = flower_head_present;
            if (flower_head_present) {
                shape_center_m = vec3<f32>(
                    side * head_size * 0.28,
                    height_m - head_size * 0.22,
                    -side * head_size * 0.10,
                );
                shape_half_m = vec3<f32>(
                    head_size * 0.34,
                    head_size * 0.28,
                    head_size * 0.30,
                );
            } else {
                // The same lifecycle structure represented by middle/coarse
                // aggregates must survive the close handoff. A non-blooming
                // candidate becomes a bounded stem/foliage cluster instead of
                // collapsing both close primitives to zero-area triangles.
                shape_center_m = vec3<f32>(
                    side * max(leaf_width_m * 0.26, 0.025),
                    height_m * 0.48,
                    -side * max(leaf_width_m * 0.10, 0.012),
                );
                shape_half_m = vec3<f32>(
                    max(leaf_width_m * 0.34, stem_width_m * 0.75),
                    max(height_m * 0.48, 0.04),
                    max(leaf_width_m * 0.22, stem_width_m * 0.60),
                );
            }
        }
    } else if (orchard) {
        let form = orchard_form(lod);
        if (aggregate) {
            head = true;
            let distance_scale = select(1.3, 1.7, lod == 3u);
            let aggregate_height_m = form.trunk_height_m + form.crown_height_m;
            shape_center_m = vec3<f32>(
                0.0,
                aggregate_height_m * 0.5,
                0.0,
            );
            shape_half_m = vec3<f32>(
                min(form.crown_along_m * distance_scale, 10.0) * 0.5,
                aggregate_height_m * 0.5,
                min(form.crown_across_m * distance_scale, 5.5) * 0.5,
            );
        } else if (primitive > 0u) {
            head = true;
            primitive_exists = primitive_exists
                && hash_unit(candidate_hash(
                    f, visual_gx, visual_gz, visual_layer, 0x73656173u,
                )) < season.density;
            shape_center_m = vec3<f32>(
                0.0,
                form.trunk_height_m + form.crown_height_m * 0.5,
                0.0,
            );
            shape_half_m = vec3<f32>(
                form.crown_along_m * 0.5,
                form.crown_height_m * 0.5,
                form.crown_across_m * 0.5,
            );
        } else {
            shape_center_m = vec3<f32>(0.0, form.trunk_height_m * 0.5, 0.0);
            shape_half_m = vec3<f32>(
                form.trunk_width_m * 0.5,
                form.trunk_height_m * 0.5,
                form.trunk_width_m * 0.5,
            );
        }
    } else if (vineyard) {
        let form = vineyard_form(lod);
        if (aggregate) {
            head = true;
            primitive_exists = primitive_exists
                && hash_unit(candidate_hash(
                    f, visual_gx, visual_gz, visual_layer, 0x73656173u,
                )) < season.density;
            let distance_scale = select(1.35, 1.8, lod == 3u);
            shape_center_m = vec3<f32>(
                0.0,
                form.mass_bottom_m + form.mass_height_m * 0.5,
                0.0,
            );
            shape_half_m = vec3<f32>(
                form.mass_along_m * distance_scale * terrain_lod_scale * 0.5,
                form.mass_height_m * 0.5,
                form.mass_across_m * terrain_lod_scale * 0.5,
            );
        } else if (primitive == 0u) {
            head = true;
            primitive_exists = primitive_exists
                && hash_unit(candidate_hash(
                    f, visual_gx, visual_gz, visual_layer, 0x73656173u,
                )) < season.density;
            shape_center_m = vec3<f32>(
                0.0,
                form.mass_bottom_m + form.mass_height_m * 0.5,
                0.0,
            );
            shape_half_m = vec3<f32>(
                form.mass_along_m * 0.56,
                form.mass_height_m * 0.5,
                form.mass_across_m * 0.5,
            );
        } else {
            let post_hash = hash_u32(visual_gx ^ (visual_gz * 0x9e3779b9u));
            primitive_exists = primitive_exists
                && form.post_period > 0u
                && post_hash % max(form.post_period, 1u) == 0u;
            shape_center_m = vec3<f32>(0.0, form.post_height_m * 0.5, 0.0);
            shape_half_m = vec3<f32>(
                form.post_width_m * 0.5,
                form.post_height_m * 0.5,
                form.post_width_m * 0.5,
            );
        }
    }

    if (crop) {
        // Every glyph carries living canopy; maturity changes its treatment,
        // not a random head's existence. Adjacent runs overlap into a field.
        head = true;
        primitive_exists = exists;
        let spacing = bitcast<f32>(f.crop_rows[0]);
        let stride = nominal_cell_m / sqrt(f32(max(f.state.z, 1u)));
        let along = max(stride * 1.65, select(select(1.35, 1.75, archetype == 2u), 3.2, evergreen_hedge));
        let across = min(spacing * 0.94, max(stride * 1.16, 0.72));
        shape_center_m = vec3<f32>(
            select(0.0, (f32(primitive) - 0.5) * along * 0.48, lod == 0u),
            height_m * 0.49,
            0.0,
        );
        shape_half_m = vec3<f32>(along * 0.54, max(height_m * 0.49, 0.045), across * 0.5);
        if (evergreen_hedge) {
            // A tea glyph represents a length of trimmed hedge, with enough
            // crown width to join its neighbors while leaving picking alleys.
            shape_half_m.z = spacing * 0.43;
        }
        if (canopy_surface) {
            // A tiled canopy becomes a continuous field at overview distance.
            // Fragment coverage cuts its actual source edges and exclusions.
            shape_center_m = vec3<f32>(0.0, height_m * 0.49, 0.0);
            shape_half_m = vec3<f32>(
                sx * 16.0 / f32(canopy_columns), max(height_m * 0.49, 0.035),
                sz * 16.0 / f32(canopy_rows),
            );
            horizontal = vec2<f32>(1.0, 0.0);
        }
    }
    if (vineyard && (aggregate || primitive == 0u) && !primitive_exists) {
        // Dormant rows retain their trained woody cordon between supports.
        primitive_exists = exists;
        head = false;
        shape_half_m.y = 0.045;
        shape_half_m.z = 0.035;
    }

    if (!primitive_exists) {
        return absent_flora(f.params.z, f.params.w, archetype);
    }
    var shape = box_vertex(box_corner_vertex, shape_center_m, shape_half_m);
    if (head || flower || (crop && primitive == 0u)) {
        shape = plant_mass_vertex(box_corner_vertex, shape_center_m, shape_half_m);
    }
    if (crop || (vineyard && head)) {
        shape = crop_canopy_vertex(box_corner_vertex, shape_center_m, shape_half_m);
        if (evergreen_hedge) {
            shape = tea_hedge_vertex(box_corner_vertex, shape_center_m, shape_half_m);
        }
        if (crop && !evergreen_hedge && CROP_RIBBONS && lod <= 1u) {
            shape = crop_ribbon_vertex(box_corner_vertex, shape_center_m, shape_half_m);
        }
    }
    if (canopy_surface) {
        shape = box_vertex(box_corner_vertex, shape_center_m, shape_half_m);
    }
    let side_axis = vec2<f32>(-horizontal.y, horizontal.x);
    let horizontal_offset_m =
        horizontal * shape.position.x + side_axis * shape.position.z;
    var local = vec3<f32>(
        fx * 32.0 + horizontal_offset_m.x / sx,
        root_y_m / sy + shape.position.y / sy,
        fz * 32.0 + horizontal_offset_m.y / sz,
    );
    if (crop || vineyard) {
        let anchor_uv = clamp(local.xz / 32.0, vec2<f32>(0.0), vec2<f32>(1.0));
        local.y = (ground_height(f, anchor_uv.x, anchor_uv.y) - 0.035 + shape.position.y) / sy;
    }
    let world = model_point(f, local);
    let tangent =
        normalize(model_axis_x(f) * horizontal.x + model_axis_z(f) * horizontal.y);
    let side = normalize(model_axis_x(f) * side_axis.x + model_axis_z(f) * side_axis.y);
    let up = normalize(model_axis_y(f));
    let normal = normalize(
        tangent * shape.normal.x + up * shape.normal.y + side * shape.normal.z,
    );
    var out: FloraOut;
    out.clip = g.view_proj * vec4<f32>(world, 1.0);
    out.color = archetype_color(
        f,
        archetype,
        head,
        select(shade, 0.5, canopy_surface || evergreen_hedge),
        season,
        flower_band_phase,
        visual_gx,
        visual_gz,
        visual_layer,
    );
    out.normal = normal;
    out.world_pos = world;
    out.terrain_fade = f.params.z;
    out.plant_fade = f.params.w;
    out.archetype = archetype;
    out.field_uv = local.xz / 32.0;
    out.field_instance = instance_index;
    return out;
}

fn pixel_hash(pixel: vec2<u32>) -> f32 {
    return hash_unit(hash_u32(pixel.x ^ (pixel.y * 0x9e3779b9u)));
}

fn dither_keeps(alpha: f32, noise: f32) -> bool {
    if (alpha >= 1.0) {
        return true;
    }
    return select(noise >= 1.0 + alpha, noise < alpha, 0.0 <= alpha);
}

// Broad shadow, midtone and highlight intervals preserve facet form without
// a hard toon edge. Solar visibility still controls all directional energy.
fn illustrative_diffuse(cosine: f32) -> f32 {
    return 0.04 + 0.76 * smoothstep(-0.08, 0.70, cosine)
        + 0.20 * smoothstep(0.70, 1.0, cosine);
}

fn illustrative_fill(color: vec3<f32>, upward: f32, daylight: f32) -> vec3<f32> {
    let value = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let sky = mix(color, value * vec3<f32>(0.78, 1.03, 1.35), 0.25 * daylight);
    let hemisphere = clamp(upward * 0.5 + 0.5, 0.0, 1.0);
    return sky * mix(0.38, 1.0, hemisphere)
        + vec3<f32>(0.075, 0.055, 0.026) * (1.0 - hemisphere) * daylight;
}

struct LeafSurface { normal: vec3<f32>, tone: f32 };

// Reuse the terrain/tree material atlas: three mip-filtered projections carry
// leaf-clump color and normal relief without new textures or plant geometry.
fn leaf_surface(position: vec3<f32>, normal: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>) -> LeafSurface {
    let period = 2.0;
    let p = (position + g.material_origin.xyz) / period;
    let tx = textureSampleGrad(leaf_atlas, leaf_sampler, p.yz, 14, dx.yz / period, dy.yz / period);
    let ty = textureSampleGrad(leaf_atlas, leaf_sampler, p.xz, 14, dx.xz / period, dy.xz / period);
    let tz = textureSampleGrad(leaf_atlas, leaf_sampler, p.xy, 14, dx.xy / period, dy.xy / period);
    let squared = normal * normal;
    let weights = squared * squared / max(dot(squared, squared), 0.0001);
    let painted = tx * weights.x + ty * weights.y + tz * weights.z;
    let gx = (tx.gb - vec2<f32>(0.5)) * 2.0;
    let gy = (ty.gb - vec2<f32>(0.5)) * 2.0;
    let gz = (tz.gb - vec2<f32>(0.5)) * 2.0;
    let gradient = vec3<f32>(0.0, gx) * weights.x
        + vec3<f32>(gy.x, 0.0, gy.y) * weights.y + vec3<f32>(gz, 0.0) * weights.z;
    let relief = gradient - normal * dot(normal, gradient);
    return LeafSurface(normalize(normal - relief * 0.45), 1.0 + (painted.r - 0.5) * 1.0);
}

@fragment
fn fs_flora(in: FloraOut) -> @location(0) vec4<f32> {
    let world_dx = dpdx(in.world_pos);
    let world_dy = dpdy(in.world_pos);
    let footprint_uv = max(length(dpdx(in.field_uv)), length(dpdy(in.field_uv)));
    if (in.archetype <= 3u || in.archetype == 8u) {
        // Canopy swathes stop at the same field/road/building mask as roots.
        // This also prevents adjacent owning cells from drawing each other's
        // enlarged overview representation.
        if (any(in.field_uv < vec2<f32>(0.0)) || any(in.field_uv >= vec2<f32>(1.0))) {
            discard;
        }
        let mask = vec2<u32>(floor(in.field_uv * 16.0));
        if (!coverage_at(flora[in.field_instance], mask.x, mask.y)) {
            discard;
        }
    }
    let noise = pixel_hash(vec2<u32>(floor(in.clip.xy)));
    // Terrain-cut and plant-LOD swaps use the same complementary predicate.
    // Each paired transition is a partition: one side keeps h < a and the
    // other keeps h >= a, so there is no density hole or double-write.
    if (!dither_keeps(in.terrain_fade, noise) || !dither_keeps(in.plant_fade, noise)) {
        discard;
    }

    var n = normalize(in.normal);
    let up = normalize(in.world_pos - g.planet_center.xyz);
    var leaf_tone = 1.0;
    if (in.archetype <= 3u || in.archetype == 8u) {
        let f = flora[in.field_instance];
        let palette = (f.state.w >> 16u) & 0xffu;
        if (in.archetype <= 3u && palette >= 16u && crop_treatment(palette).evergreen && f.state.y >= 2u) {
            let cell_m = 10007557.221 / exp2(f32((f.state.w >> 8u) & 0xffu));
            let p = in.field_uv * cell_m;
            let along = p.x * cos(f.params.x) - p.y * sin(f.params.x);
            let across = -p.x * sin(f.params.x) - p.y * cos(f.params.x);
            let bend = sin((along / 48.0 + bitcast<f32>(f.crop_rows[2])) * 6.28318530718) * 2.2;
            let phase = (across - bend) / bitcast<f32>(f.crop_rows[0]) + bitcast<f32>(f.crop_rows[1]);
            let side = normalize(model_axis_x(f) * sin(f.params.x) + model_axis_z(f) * cos(f.params.x));
            let resolved = 1.0 - smoothstep(0.35, 1.1, footprint_uv * cell_m / bitcast<f32>(f.crop_rows[0]));
            n = normalize(n - side * (fract(phase) - 0.5) * 2.4 * resolved);
        }
        let material = leaf_surface(in.world_pos, n, world_dx, world_dy);
        n = material.normal;
        leaf_tone = material.tone;
    }
    let sun_dir = normalize(g.sun_dir.xyz);
    let moon_dir = normalize(g.moon_dir.xyz);
    let solar_alt = dot(up, sun_dir);
    let daylight = smoothstep(-0.03, 0.12, solar_alt);
    let lunar_alt = dot(up, moon_dir);
    let moon_horizon = smoothstep(-0.01, 0.025, lunar_alt);
    let moon_phase = g.moon_dir.w;
    let moon_visibility = moon_horizon * moon_phase;
    let sun_height = max(solar_alt, 0.0);
    var direct_color = mix(vec3<f32>(1.42, 0.66, 0.24), vec3<f32>(1.68, 1.59, 1.36),
        smoothstep(0.035, 0.32, sun_height));
    direct_color *= mix(0.62, 1.0, smoothstep(0.0, 0.20, sun_height));
    // Match the terrain pass: the uploaded ambient is its daylight
    // baseline, not a light that should remain at full strength overnight.
    let day_fill = smoothstep(-0.10, 0.16, solar_alt);
    let twilight = smoothstep(-0.22, -0.015, solar_alt)
        * (1.0 - smoothstep(0.06, 0.28, solar_alt));
    var ambient = mix(vec3<f32>(0.020, 0.028, 0.058), g.ambient.rgb, day_fill)
        + twilight * vec3<f32>(0.025, 0.010, 0.040);
    ambient += vec3<f32>(0.020, 0.032, 0.070) * moon_visibility;
    let ndl = max(dot(n, sun_dir), 0.0);
    let local_overcast = smoothstep(2.4, 8.5, g.weather5.w);
    let cloud_transmission = cloud_sunlight(in.world_pos, 1.0, mix(1.0, 0.18, local_overcast));
    let key_visibility = sun_visibility(in.world_pos, ndl);
    let base_direct = direct_color * illustrative_diffuse(dot(n, sun_dir))
        * daylight * cloud_transmission * mix(0.08, 1.0, key_visibility);
    ambient = mix(ambient,
        vec3<f32>(0.13, 0.15, 0.18) * (0.38 + 0.62 * max(solar_alt, 0.0)),
        local_overcast * 0.72);
    let fill_daylight = smoothstep(-0.10, 0.16, solar_alt);
    ambient = illustrative_fill(ambient, dot(n, up), fill_daylight);
    let moon = vec3<f32>(0.04, 0.065, 0.13)
        * max(dot(n, moon_dir), 0.0) * moon_visibility;
    var albedo = in.color * leaf_tone;
    if (in.archetype <= 3u) {
        let f = flora[in.field_instance];
        let cell_m = 10007557.221 / exp2(f32((f.state.w >> 8u) & 0xffu));
        let spacing = bitcast<f32>(f.crop_rows[0]);
        let local_m = in.field_uv * cell_m;
        let across = -local_m.x * sin(f.params.x) - local_m.y * cos(f.params.x);
        let palette = (f.state.w >> 16u) & 0xffu;
        let tea = palette >= 16u && crop_treatment(palette).evergreen;
        let along = local_m.x * cos(f.params.x) - local_m.y * sin(f.params.x);
        let bend = sin((along / 48.0 + bitcast<f32>(f.crop_rows[2])) * 6.28318530718) * 2.2;
        let row_phase = (across - select(0.0, bend, tea)) / spacing + bitcast<f32>(f.crop_rows[1]);
        let resolved = 1.0 - smoothstep(0.35, 1.1, footprint_uv * cell_m / spacing);
        let row_shade = 0.96 + 0.04 * cos(row_phase * 6.28318530718);
        albedo *= mix(1.0, row_shade, resolved);
        if (tea) {
            // Overlapping hedge glyphs share a field-wide picking alley.
            // Unresolved rows converge to their area mean rather than shimmer.
            let distance_from_row = abs(fract(row_phase) - 0.5);
            if (distance_from_row > mix(0.5, 0.40, resolved)) { discard; }
            albedo *= mix(0.85, 1.0, resolved);
        }
    }
    let drought = clamp(g.weather4.x, 0.0, 1.0);
    // Phenology already carries senescence; weather adds a small straw tint
    // in linear albedo without replacing the authored crop/flower family.
    albedo = mix(albedo, vec3<f32>(0.27383842, 0.17886750, 0.06847817), drought * 0.18);
    var lit = albedo * (ambient + base_direct + moon);
    let wet = clamp(g.weather1.x, 0.0, 1.0);
    lit *= mix(1.0, 0.72, wet);
    let snow = clamp(g.weather1.y, 0.0, 1.0) * smoothstep(0.25, 0.8, dot(n, up));
    lit = mix(
        lit,
        vec3<f32>(0.72, 0.80, 0.88)
            * (ambient + base_direct * 0.7),
        snow,
    );

    // Flora is emitted only at fine terrain LODs, before orbital fog fades.
    return vec4<f32>(apply_aerial_perspective(lit, in.world_pos, 0.0), 1.0);
}
