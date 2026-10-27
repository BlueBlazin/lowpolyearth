// World surface shader: camera-relative positions, face-id normals resolved
// through the chunk frame basis, per-vertex AO, single sun + sky ambient.

struct Globals {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,     // ECEF, normalized
    moon_dir: vec4<f32>,    // xyz = ECEF, w = illuminated fraction
    ambient: vec4<f32>,     // rgb = sky ambient, w = exposure
    inv_view_proj: mat4x4<f32>,
    planet_center: vec4<f32>, // xyz = center (camera-relative), w = ground R
    viewport: vec4<f32>,      // xy = pixels, z = tan(fov_y / 2), w = building lights
    sky: vec4<f32>,
    horizon_glow_0: vec4<f32>,
    horizon_glow_1: vec4<f32>,
    weather0: vec4<f32>, // temperature, humidity, visibility, precipitation
    weather1: vec4<f32>, // wetness, snow, cloud coverage, convection
    wind: vec4<f32>,     // east/north/up/gust m/s
    surface: vec4<f32>,  // vegetation phase, sea state, aerosol, quality
    weather2: vec4<f32>, // time, seed phase, camera effects, lightning
    weather3: vec4<f32>, // cloud base/top, sea ice, weather mode
    weather4: vec4<f32>, // drought, turbulence, strike latitude/longitude
    weather5: vec4<f32>, // low/mid/high cloud fraction, optical depth
    weather6: vec4<f32>, // dust fraction, wet fraction, conditional rate, year phase
    wind_low: vec4<f32>,
    wind_mid: vec4<f32>,
    wind_upper: vec4<f32>,
    prev_view_proj: mat4x4<f32>,
    camera_to_prev: vec4<f32>,
    temporal: vec4<f32>, // w = simulation seconds since the hour mark
    material_origin: vec4<f32>, // camera ECEF modulo 8192 metres
    aerial_sunward: vec4<f32>,
    aerial_side: vec4<f32>,
    aerial_away: vec4<f32>,
    cloud_drift: vec4<f32>, // xyz wrapped ECEF cloud advection, w history sample cap
};

struct TerrainU {
    model: mat4x4<f32>,     // cell-frame coordinates -> camera-relative ECEF
    bx: vec4<f32>,          // xyz: unit face normal; w: signed X cofactor magnitude
    by: vec4<f32>,          // xyz: unit face normal; w: construction height phase
    bz: vec4<f32>,          // xyz: unit face normal; w: signed Z cofactor magnitude
    fade: vec4<f32>,        // LOD fade, near-tree blend, signed Y cofactor magnitude, pitch
};

struct LandmarkU {
    model: mat4x4<f32>,
    bx: vec4<f32>,
    by: vec4<f32>,
    bz: vec4<f32>,
    fade: vec4<f32>,
    landmark_model: mat4x4<f32>, // tagged landmark coordinates -> camera-relative ECEF
};

// Dedicated 128-byte near-tree record. Trees retain the exact precomputed
// normal frame used by the general surface path, but need none of its
// night-light or calibration-landmark data.
struct TreeU {
    model: mat4x4<f32>,
    bx: vec4<f32>,
    by: vec4<f32>,
    bz: vec4<f32>,
    fade: vec4<f32>,
};

struct SunShadow {
    light_view_proj_0: mat4x4<f32>,
    light_view_proj_1: mat4x4<f32>,
    // x = near end, y = far end, z = near blend start, w = far fade start.
    splits: vec4<f32>,
    // x = inverse resolution, y = shadow strength (0 disables), z/w = receiver bias.
    tuning: vec4<f32>,
};


@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<storage, read> chunks: array<TerrainU>;
@group(1) @binding(1) var<storage, read> trees: array<TreeU>;
@group(1) @binding(2) var<storage, read> landmark_chunks: array<LandmarkU>;
// Physical material atlas: tone, two signed height derivatives, roughness.
@group(2) @binding(0) var atlas_tex: texture_2d_array<f32>;
@group(2) @binding(1) var atlas_samp: sampler;
@group(2) @binding(2) var vehicle_tex: texture_2d_array<f32>;
@group(3) @binding(0) var sun_shadow_tex: texture_depth_2d_array;
@group(3) @binding(1) var sun_shadow_samp: sampler_comparison;
@group(3) @binding(2) var<uniform> sun_shadow: SunShadow;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) face_ao: u32,
    @location(2) rgba: vec4<u32>,
};

struct PlayerVsIn {
    @location(0) pos: vec3<f32>,
    @location(1) face_ao: u32,
    @location(2) rgba: vec4<u32>,
    @location(3) uv: vec2<f32>,
};

struct PackedVsIn {
    @location(0) pos_xy: u32,
    @location(1) pos_z_face: u32,
    @location(2) rgba: vec4<u32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) ao: f32,
    @location(3) world_pos: vec3<f32>, // camera-relative
    @location(4) sample_pitch_m: f32,
    @location(5) fade: f32,
    // 1 = emissive (booster flames): skip sun/AO/fog shading. World
    // geometry always writes vertex alpha 255 (→ 0 here).
    @location(6) emissive: f32,
    // Cell-local position in grid units. Facade aperture masks use the
    // fractional in-plane coordinates of the submitted surface triangles.
    @location(7) local_pos: vec3<f32>,
    @location(8) @interpolate(flat) face_mat: u32,
    // Chunk vertices carry near-tree blend in [0, 1]. Near-tree vertices use
    // -(1 + blend), preserving an unambiguous representation at blend zero.
    @location(9) @interpolate(flat) tree_transition: f32,
    // Base surface normal remains available for shadow bias, snow and wetting;
    // the material normal adds relief only to illumination.
    @location(10) base_normal: vec3<f32>,
    @location(11) @interpolate(flat) tree_lod_alpha: f32,
    @location(12) construction_height: f32,
    @location(13) vehicle_uv: vec2<f32>,
    @location(14) terrain_weights: vec4<f32>,
};

// Authored vertex colours have only 256 possible values per channel. Baking
// the exact IEC 61966-2-1 conversion avoids three power operations for every
// submitted surface vertex without reducing colour precision.
const SRGB_TO_LINEAR: array<f32, 256> = array<f32, 256>(
    0.00000000,
    0.000303526991,
    0.000607053982,
    0.000910580973,
    0.00121410796,
    0.00151763496,
    0.00182116195,
    0.00212468882,
    0.00242821593,
    0.00273174280,
    0.00303526991,
    0.00334653584,
    0.00367650739,
    0.00402471703,
    0.00439144205,
    0.00477695325,
    0.00518151652,
    0.00560539169,
    0.00604883302,
    0.00651209056,
    0.00699541019,
    0.00749903219,
    0.00802319311,
    0.00856812578,
    0.00913405884,
    0.00972121768,
    0.0103298230,
    0.0109600937,
    0.0116122449,
    0.0122864880,
    0.0129830325,
    0.0137020834,
    0.0144438436,
    0.0152085144,
    0.0159962941,
    0.0168073755,
    0.0176419541,
    0.0185002200,
    0.0193823613,
    0.0202885624,
    0.0212190095,
    0.0221738853,
    0.0231533665,
    0.0241576321,
    0.0251868591,
    0.0262412224,
    0.0273208916,
    0.0284260400,
    0.0295568351,
    0.0307134446,
    0.0318960324,
    0.0331047662,
    0.0343398079,
    0.0356013142,
    0.0368894488,
    0.0382043719,
    0.0395462364,
    0.0409151986,
    0.0423114114,
    0.0437350310,
    0.0451862030,
    0.0466650873,
    0.0481718257,
    0.0497065671,
    0.0512694567,
    0.0528606474,
    0.0544802770,
    0.0561284907,
    0.0578054301,
    0.0595112368,
    0.0612460524,
    0.0630100146,
    0.0648032650,
    0.0666259378,
    0.0684781671,
    0.0703600943,
    0.0722718537,
    0.0742135718,
    0.0761853829,
    0.0781874210,
    0.0802198201,
    0.0822827071,
    0.0843762085,
    0.0865004584,
    0.0886555836,
    0.0908417106,
    0.0930589661,
    0.0953074694,
    0.0975873470,
    0.0998987257,
    0.102241732,
    0.104616486,
    0.107023105,
    0.109461710,
    0.111932427,
    0.114435375,
    0.116970666,
    0.119538426,
    0.122138776,
    0.124771819,
    0.127437681,
    0.130136475,
    0.132868320,
    0.135633335,
    0.138431609,
    0.141263291,
    0.144128472,
    0.147027269,
    0.149959788,
    0.152926147,
    0.155926466,
    0.158960834,
    0.162029371,
    0.165132195,
    0.168269396,
    0.171441108,
    0.174647406,
    0.177888423,
    0.181164250,
    0.184474990,
    0.187820777,
    0.191201687,
    0.194617838,
    0.198069319,
    0.201556250,
    0.205078736,
    0.208636865,
    0.212230757,
    0.215860501,
    0.219526201,
    0.223227963,
    0.226965874,
    0.230740055,
    0.234550580,
    0.238397568,
    0.242281124,
    0.246201321,
    0.250158280,
    0.254152089,
    0.258182853,
    0.262250662,
    0.266355604,
    0.270497799,
    0.274677306,
    0.278894275,
    0.283148736,
    0.287440836,
    0.291770637,
    0.296138257,
    0.300543785,
    0.304987311,
    0.309468925,
    0.313988715,
    0.318546772,
    0.323143214,
    0.327778101,
    0.332451522,
    0.337163627,
    0.341914415,
    0.346704066,
    0.351532608,
    0.356400132,
    0.361306787,
    0.366252601,
    0.371237695,
    0.376262128,
    0.381326020,
    0.386429429,
    0.391572475,
    0.396755219,
    0.401977777,
    0.407240212,
    0.412542611,
    0.417885065,
    0.423267663,
    0.428690493,
    0.434153646,
    0.439657182,
    0.445201188,
    0.450785786,
    0.456411034,
    0.462076992,
    0.467783809,
    0.473531485,
    0.479320168,
    0.485149950,
    0.491020858,
    0.496932983,
    0.502886474,
    0.508881330,
    0.514917672,
    0.520995557,
    0.527115107,
    0.533276379,
    0.539479494,
    0.545724452,
    0.552011430,
    0.558340371,
    0.564711511,
    0.571124852,
    0.577580452,
    0.584078431,
    0.590618849,
    0.597201765,
    0.603827357,
    0.610495567,
    0.617206573,
    0.623960376,
    0.630757153,
    0.637596846,
    0.644479692,
    0.651405632,
    0.658374846,
    0.665387273,
    0.672443151,
    0.679542482,
    0.686685324,
    0.693871737,
    0.701101899,
    0.708375752,
    0.715693474,
    0.723055124,
    0.730460763,
    0.737910390,
    0.745404184,
    0.752942204,
    0.760524511,
    0.768151164,
    0.775822222,
    0.783537805,
    0.791297913,
    0.799102724,
    0.806952238,
    0.814846575,
    0.822785735,
    0.830769897,
    0.838799000,
    0.846873224,
    0.854992628,
    0.863157213,
    0.871367097,
    0.879622400,
    0.887923121,
    0.896269381,
    0.904661179,
    0.913098633,
    0.921581864,
    0.930110872,
    0.938685715,
    0.947306514,
    0.955973327,
    0.964686275,
    0.973445296,
    0.982250571,
    0.991102099,
    1.00000000,
);

fn srgb_to_linear_u8(c: vec3<u32>) -> vec3<f32> {
    return vec3<f32>(
        SRGB_TO_LINEAR[c.r],
        SRGB_TO_LINEAR[c.g],
        SRGB_TO_LINEAR[c.b],
    );
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
    // Linear comparison sampling provides a hardware-filtered 2x2 PCF
    // footprint with one shader lookup.
    let visibility = textureSampleCompare(
        sun_shadow_tex, sun_shadow_samp, uv, i32(cascade), reference,
    );
    // Reduce obstruction contrast without changing fully sunlit surfaces.
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

const TERRAIN_SLOPE_LEVELS: array<f32, 8> = array<f32, 8>(
    0.0, 0.125, 0.25, 0.4, 0.6, 0.9, 1.3, 2.0,
);

fn terrain_slope_component(nibble: u32) -> f32 {
    let signed = i32(nibble) - 7;
    let magnitude = TERRAIN_SLOPE_LEVELS[u32(abs(signed))];
    return select(magnitude, -magnitude, signed < 0);
}

fn terrain_lighting_normal(
    encoded: u32,
    model: mat4x4<f32>,
    geometric: vec3<f32>,
) -> vec3<f32> {
    if (encoded == 255u) {
        return geometric;
    }
    let slope_x = terrain_slope_component(encoded & 15u);
    let slope_z = terrain_slope_component((encoded >> 4u) & 15u);
    // The encoded derivatives live in cell-grid coordinates. Building
    // tangents from the model columns preserves anisotropic scale and the
    // intentionally skewed cube-sphere horizontal basis exactly.
    let tangent_x = model[0].xyz + model[1].xyz * slope_x;
    let tangent_z = model[2].xyz + model[1].xyz * slope_z;
    return normalize(cross(tangent_z, tangent_x));
}

// Explicit normals share the packed material/AO payload. Cofactors
// transform their normals correctly through the skewed cube-sphere frame.
fn surface_normal(payload: u32, model: mat4x4<f32>, fallback: vec3<f32>) -> vec3<f32> {
    if ((payload & 0x80000000u) == 0u) { return fallback; }
    let p = vec2<f32>(f32((payload >> 16u) & 127u), f32((payload >> 23u) & 127u))
        * (2.0 / 127.0) - vec2<f32>(1.0);
    var n = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    let t = max(-n.z, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.y += select(t, -t, n.y >= 0.0);
    let cofactor_normal = cross(model[1].xyz, model[2].xyz) * n.x
        + cross(model[2].xyz, model[0].xyz) * n.y
        + cross(model[0].xyz, model[1].xyz) * n.z;
    let determinant = dot(model[0].xyz, cross(model[1].xyz, model[2].xyz));
    // The inverse transpose retains the determinant sign for reflected
    // transforms; omitting it points their lit surfaces inward.
    return normalize(cofactor_normal) * select(1.0, -1.0, determinant < 0.0);
}

// World-frame directions already occupy this record. Their cofactor
// magnitudes and pitch are immutable and supplied in its spare scalar lanes.
fn world_surface_normal(
    payload: u32,
    bx: vec4<f32>, by: vec4<f32>, bz: vec4<f32>, y_weight: f32,
    fallback: vec3<f32>,
) -> vec3<f32> {
    if ((payload & 0x80000000u) == 0u) { return fallback; }
    let p = vec2<f32>(f32((payload >> 16u) & 127u), f32((payload >> 23u) & 127u))
        * (2.0 / 127.0) - vec2<f32>(1.0);
    var n = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    let t = max(-n.z, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.y += select(t, -t, n.y >= 0.0);
    return normalize(bx.xyz * (n.x * bx.w) + by.xyz * (n.y * y_weight)
        + bz.xyz * (n.z * bz.w));
}

// Face 6 is reserved for source terrain tops. Its AO lane carries the
// physical family while the flat material id keeps geographic ownership.
fn terrain_corner_weights(payload: u32) -> vec4<f32> {
    var weights = vec4<f32>(0.0);
    if ((payload & 0x80000007u) == 0x80000006u) {
        weights[(payload >> 3u) & 3u] = 1.0;
    }
    return weights;
}

fn surface_vertex(
    pos: vec3<f32>,
    face_ao: u32,
    rgba: vec4<u32>,
    chunk_index: u32,
    terrain_vertex: bool,
) -> VsOut {
    return surface_vertex_with_height(pos, face_ao, rgba, chunk_index, terrain_vertex, true);
}

fn surface_vertex_with_height(
    pos: vec3<f32>,
    face_ao: u32,
    rgba: vec4<u32>,
    chunk_index: u32,
    terrain_vertex: bool,
    compute_roof_height: bool,
) -> VsOut {
    var out: VsOut;
    let c = chunks[chunk_index];
    let landmark_tag = 128u << 5u;
    let world = c.model * vec4<f32>(pos, 1.0);
    out.clip = g.view_proj * world;
    out.world_pos = world.xyz;
    out.sample_pitch_m = c.fade.w;
    if !terrain_vertex { out.sample_pitch_m = length(c.model[0].xyz); }
    let face = face_ao & 7u;
    let water_context = ((face_ao >> 5u) & 127u) == 1u && ((face_ao >> 13u) & 3u) != 0u;
    var n: vec3<f32>;
    switch face {
        case 0u: { n = c.bx.xyz; }
        case 1u: { n = -c.bx.xyz; }
        case 2u: { n = c.by.xyz; }
        case 3u: { n = -c.by.xyz; }
        case 4u: { n = c.bz.xyz; }
        default: { n = -c.bz.xyz; }
    }
    if terrain_vertex {
        n = world_surface_normal(face_ao, c.bx, c.by, c.bz, c.fade.z, n);
    } else {
        n = surface_normal(face_ao, c.model, n);
    }
    out.normal = n;
    if terrain_vertex && !water_context && (face_ao & 0x80000000u) == 0u {
        out.normal = terrain_lighting_normal(rgba.a, c.model, n);
    }
    out.base_normal = out.normal;
    out.terrain_weights = terrain_corner_weights(face_ao);
    out.ao = select(f32((face_ao >> 3u) & 3u) / 3.0, 1.0,
        dot(out.terrain_weights, vec4<f32>(1.0)) > 0.0);
    // Tints are authored in sRGB; all lighting is linear.
    out.color = srgb_to_linear_u8(rgba.rgb);
    out.fade = c.fade.x;
    out.emissive = select(select(0.0, 1.0, rgba.a < 128u), 0.0, terrain_vertex);
    out.local_pos = pos;
    out.construction_height = 0.0;
    out.vehicle_uv = vec2<f32>(0.0);
    if water_context {
        out.emissive = 0.0;
    }
    let construction_id = (face_ao >> 5u) & 127u;
    if ((construction_id >= 81u && construction_id <= 93u) || (construction_id >= 103u && construction_id <= 118u) || construction_id == 119u) {
        // These roof layers always replace height-based UVs with world-position
        // UVs before sampling in construction_material. Wall courses retain
        // their exact geodetic height; the reference path computes both.
        let roof = construction_id == 85u || construction_id == 86u || construction_id == 88u
            || construction_id == 90u || construction_id == 91u || construction_id == 92u;
        if (compute_roof_height || !roof) {
            out.construction_height = construction_height(pos, c.model, c.by.w);
        }
    }
    out.face_mat = face_ao & ~landmark_tag;
    out.tree_transition = c.fade.y;
    out.tree_lod_alpha = 1.0;
    return out;
}

fn landmark_vertex(pos: vec3<f32>, face_ao: u32, rgba: vec4<u32>, chunk_index: u32) -> VsOut {
    var out: VsOut;
    let c = landmark_chunks[chunk_index];
    let landmark_tag = 128u << 5u;
    let is_landmark = (face_ao & landmark_tag) != 0u;
    let water_context = !is_landmark && ((face_ao >> 5u) & 127u) == 1u
        && ((face_ao >> 13u) & 3u) != 0u;
    var world = c.model * vec4<f32>(pos, 1.0);
    var sample_pitch_m = length(c.model[0].xyz);
    var bx = c.bx.xyz;
    var by = c.by.xyz;
    var bz = c.bz.xyz;
    // Only the rare tagged vertices enter this branch. Ordinary chunks use
    // surface_vertex and never fetch or evaluate the landmark transform.
    if (is_landmark) {
        world = c.landmark_model * vec4<f32>(pos, 1.0);
        sample_pitch_m = length(c.landmark_model[0].xyz);
        bx = normalize(c.landmark_model[0].xyz);
        by = normalize(c.landmark_model[1].xyz);
        bz = normalize(c.landmark_model[2].xyz);
    }
    out.clip = g.view_proj * world;
    out.world_pos = world.xyz;
    out.sample_pitch_m = sample_pitch_m;
    let face = face_ao & 7u;
    var n: vec3<f32>;
    switch face {
        case 0u: { n = bx; }
        case 1u: { n = -bx; }
        case 2u: { n = by; }
        case 3u: { n = -by; }
        case 4u: { n = bz; }
        default: { n = -bz; }
    }
    var normal_model = c.model;
    if (is_landmark) { normal_model = c.landmark_model; }
    n = surface_normal(face_ao, normal_model, n);
    out.normal = n;
    if !is_landmark && !water_context && (face_ao & 0x80000000u) == 0u {
        out.normal = terrain_lighting_normal(rgba.a, c.model, n);
    }
    out.base_normal = out.normal;
    out.terrain_weights = terrain_corner_weights(face_ao);
    out.ao = select(f32((face_ao >> 3u) & 3u) / 3.0, 1.0,
        dot(out.terrain_weights, vec4<f32>(1.0)) > 0.0);
    out.color = srgb_to_linear_u8(rgba.rgb);
    out.fade = c.fade.x;
    // Cell terrain uses alpha for its macro normal, while the tagged authored
    // landmark retains the existing alpha/emission convention.
    out.emissive = select(0.0, select(0.0, 1.0, rgba.a < 128u), is_landmark);
    out.local_pos = pos;
    out.construction_height = 0.0;
    out.vehicle_uv = vec2<f32>(0.0);
    let construction_id = (face_ao >> 5u) & 127u;
    if ((construction_id >= 81u && construction_id <= 93u) || (construction_id >= 103u && construction_id <= 118u) || construction_id == 119u) {
        out.construction_height = construction_height(pos, c.model, c.by.w);
    }
    out.face_mat = face_ao & ~landmark_tag;
    out.tree_transition = c.fade.y;
    out.tree_lod_alpha = 1.0;
    return out;
}

@vertex
fn vs_main(in: VsIn, @builtin(instance_index) chunk_index: u32) -> VsOut {
    return surface_vertex(in.pos, in.face_ao, in.rgba, chunk_index, true);
}

@vertex
fn vs_player(in: PlayerVsIn, @builtin(instance_index) chunk_index: u32) -> VsOut {
    var out = surface_vertex(in.pos, in.face_ao, in.rgba, chunk_index, false);
    out.vehicle_uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
    return out;
}

@vertex
fn vs_landmark(in: VsIn, @builtin(instance_index) chunk_index: u32) -> VsOut {
    return landmark_vertex(in.pos, in.face_ao, in.rgba, chunk_index);
}

fn tree_vertex(pos: vec3<f32>, face_ao: u32, rgba: vec4<u32>, tree_index: u32) -> VsOut {
    var out: VsOut;
    let t = trees[tree_index];
    let world = t.model * vec4<f32>(pos, 1.0);
    out.clip = g.view_proj * world;
    out.world_pos = world.xyz;
    out.sample_pitch_m = length(t.model[0].xyz) * t.fade.w;
    let face = face_ao & 7u;
    var n: vec3<f32>;
    switch face {
        case 0u: { n = t.bx.xyz; }
        case 1u: { n = -t.bx.xyz; }
        case 2u: { n = t.by.xyz; }
        case 3u: { n = -t.by.xyz; }
        case 4u: { n = t.bz.xyz; }
        default: { n = -t.bz.xyz; }
    }
    n = surface_normal(face_ao, t.model, n);
    out.normal = n;
    out.base_normal = n;
    out.terrain_weights = vec4<f32>(0.0);
    out.ao = f32((face_ao >> 3u) & 3u) / 3.0;
    out.color = srgb_to_linear_u8(rgba.rgb) * t.fade.z;
    out.fade = t.fade.x;
    out.emissive = select(0.0, 1.0, rgba.a < 128u);
    // Object-mesh vertices are stored in metres; normalize them to the
    // material sampling pitch used by the shared surface shader.
    out.local_pos = pos / t.fade.w;
    out.construction_height = 0.0;
    out.vehicle_uv = vec2<f32>(0.0);
    out.face_mat = face_ao & ~(128u << 5u);
    out.tree_transition = -(1.0 + t.fade.y);
    out.tree_lod_alpha = t.bx.w;
    return out;
}

@vertex
fn vs_tree(in: VsIn, @builtin(instance_index) tree_index: u32) -> VsOut {
    return tree_vertex(in.pos, in.face_ao, in.rgba, tree_index);
}

fn unpack_fixed_i16(value: u32) -> f32 {
    // Shift the selected 16-bit lane into the sign position, bitcast, then
    // arithmetic-shift it back before decoding signed 8.8 fixed point.
    return f32(bitcast<i32>(value << 16u) >> 16) * (1.0 / 256.0);
}

fn packed_surface_normal(in: PackedVsIn) -> bool {
    return (in.pos_z_face & 0x80000000u) != 0u;
}

fn unpack_fixed_i14(value: u32) -> f32 {
    return f32(bitcast<i32>(value << 18u) >> 18) * (1.0 / 64.0);
}

fn unpack_packed_position(in: PackedVsIn) -> vec3<f32> {
    if (packed_surface_normal(in)) {
        return vec3<f32>(
            unpack_fixed_i14(in.pos_xy),
            unpack_fixed_i14(in.pos_xy >> 14u),
            unpack_fixed_i14(in.pos_z_face),
        );
    }
    return vec3<f32>(
        unpack_fixed_i16(in.pos_xy),
        unpack_fixed_i16(in.pos_xy >> 16u),
        unpack_fixed_i16(in.pos_z_face),
    );
}

fn unpack_packed_face(in: PackedVsIn) -> u32 {
    if (packed_surface_normal(in)) {
        let normal = (in.pos_xy >> 28u)
            | (((in.pos_z_face >> 27u) & 15u) << 4u)
            | ((in.rgba.a & 63u) << 8u);
        return ((in.pos_z_face >> 14u) & 0x1fffu) | ((in.rgba.a >> 6u) << 13u)
            | (normal << 16u) | 0x80000000u;
    }
    return in.pos_z_face >> 16u;
}

fn unpack_packed_rgba(in: PackedVsIn) -> vec4<u32> {
    return vec4<u32>(in.rgba.rgb, select(in.rgba.a, 255u, packed_surface_normal(in)));
}

@vertex
fn vs_packed(in: PackedVsIn, @builtin(instance_index) chunk_index: u32) -> VsOut {
    return surface_vertex(
        unpack_packed_position(in), unpack_packed_face(in), unpack_packed_rgba(in), chunk_index, true,
    );
}

@vertex
fn vs_packed_landmark(in: PackedVsIn, @builtin(instance_index) chunk_index: u32) -> VsOut {
    return landmark_vertex(
        unpack_packed_position(in), unpack_packed_face(in), unpack_packed_rgba(in), chunk_index,
    );
}

fn pixel_hash(pixel: vec2<u32>) -> f32 {
    // A compact integer avalanche avoids directional screen-space patterns.
    var h = pixel.x * 1664525u + pixel.y * 1013904223u + 0x9e3779b9u;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h & 0x00ffffffu) * (1.0 / 16777216.0);
}

// Analytic soft transitions keep a readable core shadow and a broad lit
// region while retaining the material normal's response to moving sunlight.
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

struct MaterialSample {
    tone: f32,
    normal: vec3<f32>,
    roughness: f32,
};

fn material_hash(value: u32) -> u32 {
    var x = value;
    x = (x ^ (x >> 16u)) * 0x7feb352du;
    x = (x ^ (x >> 15u)) * 0x846ca68bu;
    return x ^ (x >> 16u);
}

fn material_cell_noise(cell: vec3<i32>) -> f32 {
    // The field repeats only with the camera's 8192 m origin. Modulo-32
    // addressing keeps the pattern continuous when that origin wraps.
    let wrapped = vec3<u32>(cell & vec3<i32>(31));
    return f32(material_hash(wrapped.x + wrapped.y * 32u + wrapped.z * 1024u) & 65535u)
        / 65535.0;
}

fn material_selector(position_m: vec3<f32>) -> f32 {
    let p = position_m / 256.0;
    let cell = vec3<i32>(floor(p));
    let f = fract(p);
    let w = f * f * (vec3<f32>(3.0) - 2.0 * f);
    let low = mix(mix(material_cell_noise(cell), material_cell_noise(cell + vec3<i32>(1, 0, 0)), w.x),
        mix(material_cell_noise(cell + vec3<i32>(0, 1, 0)), material_cell_noise(cell + vec3<i32>(1, 1, 0)), w.x), w.y);
    let high = mix(mix(material_cell_noise(cell + vec3<i32>(0, 0, 1)), material_cell_noise(cell + vec3<i32>(1, 0, 1)), w.x),
        mix(material_cell_noise(cell + vec3<i32>(0, 1, 1)), material_cell_noise(cell + vec3<i32>(1, 1, 1)), w.x), w.y);
    return mix(low, high, w.z) * 7.0;
}

fn material_offset(index: u32) -> vec2<f32> {
    let h = material_hash(index + 1879u);
    return vec2<f32>(f32(h & 65535u), f32(h >> 16u)) / 65536.0;
}

fn varied_material(uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, layer: u32, selector: f32) -> vec4<f32> {
    let index = u32(floor(selector));
    let blend = smoothstep(0.15, 0.85, fract(selector));
    // Translation leaves height derivatives in the same basis. Explicit
    // texture gradients prevent the changing offset index from selecting a
    // coarse mip at a blend boundary, including inside the endpoint branches.
    // The RGBA8Unorm atlas is finite, so a zero-weight endpoint contributes
    // nothing and its texture sample can be omitted.
    if (blend <= 0.0) {
        return textureSampleGrad(atlas_tex, atlas_samp, uv + material_offset(index), i32(layer), dx, dy);
    }
    if (blend >= 1.0) {
        return textureSampleGrad(atlas_tex, atlas_samp, uv + material_offset(index + 1u), i32(layer), dx, dy);
    }
    let a = textureSampleGrad(atlas_tex, atlas_samp, uv + material_offset(index), i32(layer), dx, dy);
    let b = textureSampleGrad(atlas_tex, atlas_samp, uv + material_offset(index + 1u), i32(layer), dx, dy);
    return mix(a, b, blend);
}

fn sample_material(position_m: vec3<f32>, normal: vec3<f32>, layer: u32,
                   period_m: f32, detail: f32) -> MaterialSample {
    return sample_material_grad(position_m, normal, layer, period_m, detail,
        dpdx(position_m), dpdy(position_m));
}

fn sample_material_grad(position_m: vec3<f32>, normal: vec3<f32>, layer: u32,
                        period_m: f32, detail: f32, world_dx: vec3<f32>,
                        world_dy: vec3<f32>) -> MaterialSample {
    if (detail <= 0.0) { return MaterialSample(0.0, normal, 1.0); }
    // Unit paving keeps one world-anchored bond. Crossfading arbitrary tile
    // offsets would double the joints and make slabs slide across the surface.
    var selector = 0.0;
    if (layer < 39u || layer == 42u || layer > 44u) {
        selector = material_selector(position_m);
    }
    return sample_material_selected_grad(position_m, normal, layer, period_m, detail,
        world_dx, world_dy, selector);
}

fn sample_material_selected_grad(position_m: vec3<f32>, normal: vec3<f32>, layer: u32,
                                 period_m: f32, detail: f32, world_dx: vec3<f32>,
                                 world_dy: vec3<f32>, selector: f32) -> MaterialSample {
    var p = position_m / period_m;
    if (layer >= 39u && layer <= 44u && layer != 42u) {
        // Reduce the repeated bond before adding its fractional atlas offset.
        // Otherwise large UVs round that offset differently at origin wraps.
        // Explicit derivatives retain the same filtering across tile seams.
        p = fract(p);
    }
    let pixel_dx = world_dx / period_m;
    let pixel_dy = world_dy / period_m;
    let squared = normal * normal;
    let weights = squared * squared / max(dot(squared, squared), 1e-5);
    let x = varied_material(p.yz, pixel_dx.yz, pixel_dy.yz, layer, selector);
    let y = varied_material(p.xz, pixel_dx.xz, pixel_dy.xz, layer, selector);
    let z = varied_material(p.xy, pixel_dx.xy, pixel_dy.xy, layer, selector);
    let painted = x * weights.x + y * weights.y + z * weights.z;
    // Blending height gradients in the tangent plane avoids the opposing-axis
    // seams of ordinary triplanar normal-vector blending (Mikkelsen, JCGT 2020).
    let dx = (x.gb - vec2<f32>(0.5)) * (4.0 / period_m);
    let dy = (y.gb - vec2<f32>(0.5)) * (4.0 / period_m);
    let dz = (z.gb - vec2<f32>(0.5)) * (4.0 / period_m);
    let gradient = vec3<f32>(0.0, dx) * weights.x
        + vec3<f32>(dy.x, 0.0, dy.y) * weights.y
        + vec3<f32>(dz, 0.0) * weights.z;
    let tangent_gradient = gradient - normal * dot(normal, gradient);
    var result: MaterialSample;
    result.tone = ((painted.r - 0.5) * 1.4 + (selector / 7.0 - 0.5) * 0.12) * detail;
    result.normal = normalize(normal - tangent_gradient * (1.5 * detail));
    result.roughness = painted.a;
    return result;
}

struct VehicleMaterial {
    roughness: f32,
    metallic: f32,
    reflectance: f32,
    clearcoat: f32,
    emission: f32,
};

// Vehicle ids belong to transient authored meshes, outside persistent terrain
// and construction semantics. Panel boundaries and bevels come from geometry;
// these surfaces do not inherit the world's stone/grain atlas.
fn vehicle_material(material_id: u32) -> VehicleMaterial {
    switch material_id {
        case 97u, 102u: { return VehicleMaterial(0.30, 1.0, 0.04, 0.0, 0.0); }
        case 98u: { return VehicleMaterial(0.72, 0.0, 0.035, 0.0, 0.0); }
        case 99u: { return VehicleMaterial(0.16, 0.0, 0.055, 0.0, 0.0); }
        case 100u: { return VehicleMaterial(0.40, 0.0, 0.04, 0.0, 3.6); }
        case 101u: { return VehicleMaterial(0.40, 0.0, 0.04, 0.0, 2.1); }
        default: { return VehicleMaterial(0.36, 0.0, 0.04, 0.32, 0.0); }
    }
}

fn vehicle_fresnel(f0: vec3<f32>, cosine: f32) -> vec3<f32> {
    let grazing = pow(1.0 - clamp(cosine, 0.0, 1.0), 5.0);
    return f0 + (vec3<f32>(1.0) - f0) * grazing;
}

fn vehicle_specular_lobe(roughness: f32, normal_variance: f32,
                        ndv: f32, ndl: f32, ndh: f32) -> f32 {
    // GGX distribution with correlated Smith visibility. Unresolved normal
    // variance broadens the lobe before evaluation, keeping small bevels stable.
    let alpha_squared = clamp(pow(roughness, 4.0) + normal_variance, 0.0005, 1.0);
    let denominator = ndh * ndh * (alpha_squared - 1.0) + 1.0;
    let distribution = alpha_squared / (3.14159265 * denominator * denominator);
    let visibility_v = ndl * sqrt(ndv * ndv * (1.0 - alpha_squared) + alpha_squared);
    let visibility_l = ndv * sqrt(ndl * ndl * (1.0 - alpha_squared) + alpha_squared);
    return distribution * 0.5 / max(visibility_v + visibility_l, 0.0001);
}

fn vehicle_direct_response(material: VehicleMaterial, albedo: vec3<f32>,
                           n: vec3<f32>, view: vec3<f32>, light: vec3<f32>,
                           normal_variance: f32) -> vec3<f32> {
    let ndl = max(dot(n, light), 0.0);
    let ndv = max(dot(n, view), 0.001);
    let sum = view + light;
    let half_direction = sum / max(length(sum), 0.0001);
    let ndh = max(dot(n, half_direction), 0.0);
    let vdh = max(dot(view, half_direction), 0.0);
    let f0 = mix(vec3<f32>(material.reflectance), albedo, material.metallic);
    let fresnel = vehicle_fresnel(f0, vdh);
    let specular = fresnel * vehicle_specular_lobe(material.roughness,
        normal_variance, ndv, ndl, ndh);
    let diffuse = albedo * (vec3<f32>(1.0) - fresnel)
        * ((1.0 - material.metallic) / 3.14159265);
    let coat_fresnel = vehicle_fresnel(vec3<f32>(0.04), vdh).r * material.clearcoat;
    let coat = coat_fresnel * vehicle_specular_lobe(0.22,
        normal_variance, ndv, ndl, ndh);
    // The world key is irradiance divided by pi; compensate here so the
    // vehicle's Lambert term remains in the same exposure range as terrain.
    return ((diffuse + specular) * (1.0 - coat_fresnel) + vec3<f32>(coat))
        * (3.14159265 * ndl);
}

fn vehicle_environment_radiance(sky: vec3<f32>) -> vec3<f32> {
    let radiance = max(sky, vec3<f32>(0.0));
    let luminance = dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722));
    // Diffuse sky fill uses this scene exposure range on the CPU. Reflections
    // need the same compression, or HDR sky erases dark painted albedo.
    // A common gain preserves sky hue and leaves faint night reflections alone.
    return radiance * min(1.0, 0.32 / (0.09 + luminance));
}

fn vehicle_lighting(in: VsOut, material_id: u32, n: vec3<f32>, up: vec3<f32>,
                    ambient: vec3<f32>, direct_color: vec3<f32>,
                    key_visibility: f32, sun_horizon: f32, moon_visibility: f32,
                    ao: f32, normal_variance: f32, uv_dx: vec2<f32>, uv_dy: vec2<f32>) -> vec3<f32> {
    let material = vehicle_material(material_id);
    let view = -normalize(in.world_pos);
    var albedo = in.color;
    // Only authored player meshes own bits 13..15. Layer zero is untextured.
    let texture_layer = (in.face_mat >> 13u) & 7u;
    if (texture_layer > 0u) {
        albedo *= textureSampleGrad(vehicle_tex, atlas_samp, in.vehicle_uv, i32(texture_layer - 1u), uv_dx, uv_dy).rgb;
    }
    if (material.metallic > 0.5) {
        albedo = clamp(albedo, vec3<f32>(0.12), vec3<f32>(0.95));
    }
    if (material_id == 99u) { albedo *= 0.14; }
    let f0 = mix(vec3<f32>(material.reflectance), albedo, material.metallic);
    let ndv = max(dot(n, view), 0.0);
    let grazing = pow(1.0 - ndv, 5.0);
    let environment_fresnel = f0
        + (max(vec3<f32>(1.0 - material.roughness), f0) - f0) * grazing;
    let reflected_direction = reflect(-view, n);
    let sharp_environment = vehicle_environment_radiance(sample_reflected_environment(reflected_direction));
    // The cached sky has no reflected terrain. Fade its lower hemisphere to
    // a diffuse ground estimate; rough finishes soften toward the same sky fill.
    let sky_weight = smoothstep(-0.18, 0.06, dot(reflected_direction, up));
    let environment = mix(ambient * vec3<f32>(0.45, 0.38, 0.32),
        mix(sharp_environment, ambient * 1.8, material.roughness * material.roughness * 0.72),
        sky_weight);
    let diffuse = albedo * ambient * (vec3<f32>(1.0) - environment_fresnel)
        * (1.0 - material.metallic);
    let coat_fresnel = (0.04 + 0.96 * grazing) * material.clearcoat;
    var lit = (diffuse + environment * environment_fresnel)
        * (1.0 - coat_fresnel) * ao
        + environment * coat_fresnel * ao;
    lit += vehicle_direct_response(material, albedo, n, view,
        normalize(g.sun_dir.xyz), normal_variance)
        * direct_color * key_visibility * sun_horizon;
    lit += vehicle_direct_response(material, albedo, n, view,
        normalize(g.moon_dir.xyz), normal_variance)
        * vec3<f32>(0.070, 0.100, 0.180) * moon_visibility;
    // Actual nozzle and navigation surfaces remain opaque and depth-writing.
    // The separate exhaust pass supplies the transparent light around them.
    return lit + in.color * material.emission;
}


// Persistent sculptures share the existing GGX and sky cache, but never read
// player texture payloads. The sky has no buildings; its lower hemisphere is
// a restrained ground estimate. No additional capture, pass or binding is used.
fn sculpture_metal_lighting(in: VsOut, n: vec3<f32>, up: vec3<f32>,
                           ambient: vec3<f32>, direct_color: vec3<f32>,
                           key_visibility: f32, sun_horizon: f32, moon_visibility: f32,
                           ao: f32, normal_variance: f32) -> vec3<f32> {
    let material = VehicleMaterial(0.28, 1.0, 0.04, 0.0, 0.0);
    let albedo = clamp(in.color, vec3<f32>(0.02), vec3<f32>(0.95));
    let view = -normalize(in.world_pos);
    let reflected = reflect(-view, n);
    let sky = vehicle_environment_radiance(sample_reflected_environment(reflected));
    let sky_weight = smoothstep(-0.18, 0.06, dot(reflected, up));
    let environment = mix(ambient * vec3<f32>(0.45, 0.38, 0.32),
        mix(sky, ambient * 1.8, material.roughness * material.roughness * 0.72), sky_weight);
    let fresnel = vehicle_fresnel(albedo, max(dot(n, view), 0.0));
    var lit = environment * fresnel * ao;
    lit += vehicle_direct_response(material, albedo, n, view,
        normalize(g.sun_dir.xyz), normal_variance)
        * direct_color * key_visibility * sun_horizon;
    lit += vehicle_direct_response(material, albedo, n, view,
        normalize(g.moon_dir.xyz), normal_variance)
        * vec3<f32>(0.070, 0.100, 0.180) * moon_visibility;
    return lit;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vehicle_uv_dx = dpdx(in.vehicle_uv);
    let vehicle_uv_dy = dpdy(in.vehicle_uv);
    let pixel_noise = pixel_hash(vec2<u32>(floor(in.clip.xy)));
    // LOD crossfade: screen-door dither on direction-free pixel noise.
    // Fading in (a > 0) keeps pixels where h < a; fading out (a < 0,
    // weight |a|) keeps the exact complement h >= 1-|a|, so a paired
    // refine/merge swap writes every pixel exactly once — no z-fight, no
    // punch-through. a == 1 (steady state) skips the whole block.
    let a = in.fade;
    if (a < 1.0) {
        let keep = select(pixel_noise >= 1.0 + a, pixel_noise < a, 0.0 <= a);
        if (!keep) {
            discard;
        }
    }
    // Emissive (booster flames): self-lit, no sun/AO/fog.
    if (in.emissive > 0.5) {
        return vec4<f32>(in.color * 1.4, 1.0);
    }
    let material_id = (in.face_mat >> 5u) & 255u;
    // Semantic foliage ids retain season and motion while sharing one leaf-art
    // layer. Material id is not necessarily the atlas layer.
    let is_foliage = material_id >= 22u && material_id < 66u;
    let is_tree_material = material_id == 13u || is_foliage;
    let is_near_tree = in.tree_transition < 0.0;
    if (is_tree_material) {
        let near_blend = select(
            clamp(in.tree_transition, 0.0, 1.0),
            clamp(-in.tree_transition - 1.0, 0.0, 1.0),
            is_near_tree,
        );
        // Complementary masks retain exactly one representation at every
        // pixel while the detailed mesh replaces the streamed fallback.
        let keep_tree = select(
            pixel_noise >= near_blend,
            pixel_noise < near_blend,
            is_near_tree,
        );
        if (!keep_tree) {
            discard;
        }
        if (in.tree_lod_alpha < 1.0) {
            // An independent screen-space mask preserves both adjacent-LOD
            // coverage and the streamed/instanced tree transition above.
            let lod_noise = pixel_hash(vec2<u32>(floor(in.clip.xy)) + vec2<u32>(173u, 941u));
            let alpha = in.tree_lod_alpha;
            let keep_lod = select(lod_noise >= 1.0 + alpha, (lod_noise < alpha), 0.0 <= alpha);
            if (!keep_lod) { discard; }
        }
    }
    return shade_surface(in, vehicle_uv_dx, vehicle_uv_dy);
}

// Selected only for positively opaque ordinary chunks without tree materials.
// Keep this entry point's reachable call graph free of discard, so opaque
// hidden-surface removal can run before material shading.
@fragment
fn fs_opaque(in: VsOut) -> @location(0) vec4<f32> {
    let vehicle_uv_dx = dpdx(in.vehicle_uv);
    let vehicle_uv_dy = dpdy(in.vehicle_uv);
    return shade_surface(in, vehicle_uv_dx, vehicle_uv_dy);
}


// Analytic box filtering preserves glazing coverage when bays become subpixel.
// Pattern periods divide the floating-origin period; height uses the CPU's
// common 256 m phase so neighbouring chunks agree for both 3.2 m and 4 m floors.
struct HighriseSample {
    glass: f32,
    color: vec3<f32>,
    roughness: f32,
    emission: vec3<f32>,
};
fn highrise_integral(x: f32, width: f32) -> f32 {
    return floor(x) * width + min(fract(x), width);
}
fn highrise_coverage(x: f32, low: f32, high: f32, footprint: f32) -> f32 {
    let radius = max(footprint, 0.001) * 0.5;
    let p = x - low;
    return clamp((highrise_integral(p + radius, high - low)
        - highrise_integral(p - radius, high - low)) / (2.0 * radius), 0.0, 1.0);
}
fn highrise_sample(wall: vec3<f32>, id: u32, variant: u32, floor_coord: f32, footprint: f32, profile: HighriseProfile) -> HighriseSample {
    let system = (id - 103u) % 8u;
    let residential = system >= 5u;
    // Variant 1 is an area-integrated linear-light pigment encoded
    // into existing vertex RGB. Reflectance/emission retain a bounded typical
    // bay estimate; the coarse mesh has no spatial aperture information.
    if variant == 1u {
        return HighriseSample(profile.coarse_glass, wall, mix(0.76, profile.roughness, profile.coarse_glass),
            select(vec3<f32>(0.55, 0.58, 0.55) * 0.28, vec3<f32>(0.86, 0.58, 0.29) * 0.38, residential) * profile.coarse_glass * 0.16 * select(0.0, 1.0, id >= 111u));
    }
    let opening = highrise_coverage(floor_coord, profile.glazing.x, profile.glazing.y, footprint);
    let resolved = 1.0 - smoothstep(0.18, 0.65, footprint);
    // The mesh fits vertical frames to source edges. Only geodetic floor
    // divisions are shaded here: world-axis projection would slope up walls.
    let filtered_opening = mix(profile.glazing.y - profile.glazing.x, opening, resolved);
    let glass = select(0.72 + 0.28 * filtered_opening, filtered_opening, residential);
    let row_period = select(64.0, 80.0, residential);
    let row = u32(floor(fract(floor_coord / row_period) * row_period));
    var state = row * 1013904223u + system * 747796405u;
    state = (state ^ (state >> 16u)) * 2246822519u;
    let room = f32((state ^ (state >> 13u)) & 65535u) / 65536.0;
    let occupancy = select(0.28, 0.38, residential);
    let lit_room = mix(occupancy, select(0.0, 1.0, room < occupancy), resolved);
    let interior = mix(0.93, 1.03, smoothstep(0.0, 0.6, fract(floor_coord)));
    let glazing = profile.glass * mix(1.0, interior, resolved);
    let spandrel = wall;
    let emission_color = select(vec3<f32>(0.55, 0.58, 0.55), vec3<f32>(0.86, 0.58, 0.29), residential);
    return HighriseSample(glass, mix(spandrel, glazing, glass), mix(0.76, profile.roughness, glass),
        emission_color * lit_room * select(opening, glass, residential) * 0.16 * select(0.0, 1.0, id >= 111u));
}
fn highrise_material(in: VsOut, id: u32) -> HighriseSample {
    let profile = highrise_profile((id - 103u) % 8u);
    let floor_coord = in.construction_height / profile.floor_m;
    let footprint = abs(dpdx(floor_coord)) + abs(dpdy(floor_coord));
    return highrise_sample(in.color, id, (in.face_mat >> 13u) & 3u, floor_coord, footprint, profile);
}

// Only the authored player pipeline draws vehicle materials. World pipelines
// override this to false so their register allocation excludes the unused
// vehicle lighting path.
override VEHICLE_SURFACES: bool = true;
// Highrise facades, sculpture metal and landmark fixtures are rare, but their
// paths raise register allocation for every fragment. Chunks without them
// (see prepared_render::material_has_rare_surface) use pipelines that
// override this to false.
override RARE_SURFACES: bool = true;

fn shade_surface(in: VsOut, vehicle_uv_dx: vec2<f32>, vehicle_uv_dy: vec2<f32>) -> vec4<f32> {
    if (in.emissive > 0.5) {
        return vec4<f32>(in.color * 1.4, 1.0);
    }
    let material_id = (in.face_mat >> 5u) & 255u;
    let is_highrise = RARE_SURFACES && material_id >= 103u && material_id <= 118u;
    let blended_terrain = dot(in.terrain_weights, vec4<f32>(1.0)) > 0.0;
    let is_vehicle = VEHICLE_SURFACES && material_id >= 96u && material_id <= 102u;
    let is_foliage = material_id >= 22u && material_id < 66u;
    var layer = select(min(material_id, 22u), 14u, is_foliage);
    // Expanded persistent surface semantics reuse physical art rather than
    // consuming one atlas layer per authored colour family. Keep this mapping
    // in sync with atlas::material_traits.
    switch material_id {
        case 66u: { layer = 2u; } // soil -> sand/aggregate grain
        case 67u: { layer = 2u; } // salt/gypsum -> fine granular crust
        case 68u: { layer = 5u; } // ice -> snow/ice grain
        case 69u: { layer = 4u; } // volcanic -> rock grain
        case 70u: { layer = 3u; } // wetland/peat -> organic ground
        case 71u: { layer = 16u; } // aviation beacon -> painted metal
        case 72u: { layer = 3u; } // growing crop -> vegetated ground
        case 73u: { layer = 3u; } // flower field -> vegetated ground
        case 74u: { layer = 3u; } // orchard surface -> vegetated ground
        case 75u: { layer = 3u; } // vineyard surface -> vegetated ground
        case 76u: { layer = 3u; } // meadow surface -> vegetated ground
        case 77u: { layer = 2u; } // dormant field -> dry aggregate
        case 78u: { layer = 2u; } // harvested field -> soil/stubble
        case 79u: { layer = 16u; } // landmark lamp -> painted metal
        case 80u: { layer = 16u; } // landmark sparkle fixture -> painted metal
        case 81u: { layer = 23u; }
        case 82u: { layer = 24u; }
        case 83u: { layer = 25u; }
        case 84u: { layer = 26u; }
        case 85u: { layer = 27u; }
        case 86u: { layer = 28u; }
        case 87u: { layer = 29u; }
        case 88u: { layer = 30u; }
        case 89u: { layer = 31u; }
        case 90u: { layer = 32u; }
        case 91u: { layer = 33u; }
        case 92u: { layer = 34u; }
        case 93u: { layer = 35u; }
        case 94u, 95u: { layer = 16u; }
        case 119u: { layer = 36u; }
        case 120u: { layer = 37u; }
        case 121u: { layer = 38u; }
        case 122u: { layer = 39u; }
        case 123u: { layer = 40u; }
        case 124u: { layer = 41u; }
        case 125u: { layer = 42u; }
        case 126u: { layer = 43u; }
        case 127u: { layer = 44u; }
        default: {}
    }
    // Bits 13..14 are material-scoped; water and vehicle context is unchanged.
    if (((in.face_mat >> 13u) & 3u) == 1u) {
        switch material_id {
            case 87u, 88u: { layer = 45u; }
            case 85u: { layer = 46u; }
            case 82u: { layer = 47u; }
            case 84u: { layer = 48u; }
            case 90u: { layer = 49u; }
            case 93u: { layer = 53u; }
            default: {}
        }
    }
    let patterned_azulejo = ((in.face_mat >> 13u) & 3u) == 2u && material_id == 93u;
    if patterned_azulejo { layer = 54u; }
    let matte_corrugated = ((in.face_mat >> 13u) & 3u) == 2u
        && (material_id == 87u || material_id == 88u);
    if matte_corrugated { layer = 45u; }
    if ((in.face_mat >> 13u) & 3u) == 2u && material_id == 84u { layer = 50u; }
    let named_roof_finish = material_id == 12u && ((in.face_mat >> 13u) & 3u) != 0u;
    if named_roof_finish {
        switch ((in.face_mat >> 13u) & 3u) {
            case 1u: { layer = 51u; }
            case 2u: { layer = 52u; }
            case 3u: { layer = 33u; }
            default: {}
        }
    }
    let tile_stamped_metal = ((in.face_mat >> 13u) & 3u) == 3u && material_id == 88u;
    if tile_stamped_metal { layer = 27u; }
    var period_m = 16.0;
    switch layer {
        case 3u: { period_m = 32.0; }
        case 4u: { period_m = 64.0; }
        case 14u: { period_m = 8.0; }
        case 11u, 12u: { period_m = 4.0; }
        case 13u: { period_m = 2.0; }
        case 37u: { period_m = 8.0; }
        case 38u: { period_m = 4.0; }
        case 7u: { period_m = 8.0; }
        case 10u, 39u, 42u, 44u: { period_m = 4.0; }
        case 40u, 41u, 43u: { period_m = 2.0; }
        default: {}
    }
    let material_detail = 1.0 - smoothstep(48.0, 512.0, in.sample_pitch_m);
    var base_normal = normalize(in.base_normal);
    if (material_id == 1u) {
        let ellipsoid = in.world_pos - g.planet_center.xyz;
        base_normal = normalize(ellipsoid * vec3<f32>(1.0, 1.0, 1.0067395));
    }
    let material_position = in.world_pos + g.material_origin.xyz;
    let world_dx = dpdx(in.world_pos);
    let world_dy = dpdy(in.world_pos);
    let water_footprint = max(length(world_dx), length(world_dy));
    var tower = HighriseSample(0.0, in.color, 0.75, vec3<f32>(0.0));
    if is_highrise { tower = highrise_material(in, material_id); }
    var surface_material: MaterialSample;
    var water = WaterSurface(base_normal, 0.15, 0.5);
    let water_kind = (in.face_mat >> 13u) & 3u;
    let water_shelter = water_exposure(water_kind);
    if (material_id == 94u) {
        surface_material = MaterialSample(0.0, base_normal, 1.0);
    } else if (is_highrise) {
        surface_material = MaterialSample(0.0, base_normal, tower.roughness);
    } else if (RARE_SURFACES && material_id == 95u) {
        surface_material = MaterialSample(0.0, base_normal, 0.28);
    } else if (is_vehicle) {
        surface_material = MaterialSample(0.0, base_normal, vehicle_material(material_id).roughness);
    } else if (material_id == 1u) {
        if (material_detail > 0.0) {
            water = water_surface(material_position, base_normal,
                (length(g.wind.xy) + g.wind.w * 0.25) * water_shelter.x,
                g.surface.y * water_shelter.y, g.weather2.x, water_footprint);
            // Constant binary-rational drift moves by whole 8192 m periods at
            // the 65536 s weather-clock wrap, including the material selector.
            let drift = vec3<f32>(0.125, -0.25, 0.125) * g.weather2.x;
            let residual = sample_material(material_position + drift,
                base_normal, 1u, 16.0, material_detail);
            let wind_energy = clamp((length(g.wind.xy) + g.wind.w * 0.25) * water_shelter.x / 22.0, 0.0, 1.0);
            let sea_energy = clamp(g.surface.y * water_shelter.y, 0.0, 1.0);
            let residual_strength = (0.30 + 1.15 * sea_energy + 0.55 * wind_energy) * 0.50;
            let carrier_gradient = base_normal
                - water.normal / max(dot(water.normal, base_normal), 0.01);
            let residual_gradient = base_normal
                - residual.normal / max(dot(residual.normal, base_normal), 0.01);
            water.normal = normalize(base_normal - carrier_gradient * (0.60 * material_detail)
                - residual_gradient * residual_strength);
            let base_roughness = 0.095 + 0.075 * wind_energy + 0.10 * sea_energy;
            let carrier_variance = max(pow(water.roughness, 4.0) - pow(base_roughness, 4.0), 0.0)
                * (0.36 * material_detail * material_detail);
            // Atlas alpha includes unresolved residual slope variance. Its
            // base is the encoded 31/255 value of authored roughness 0.12.
            let residual_variance = max(pow(residual.roughness, 4.0) - pow(31.0 / 255.0, 4.0), 0.0);
            water.roughness = clamp(water_roughness_from_slope_variance(base_roughness,
                carrier_variance + residual_variance * residual_strength * residual_strength
                    * material_detail * material_detail), 0.095, 0.50);
        }
        surface_material = MaterialSample(0.0, water.normal, water.roughness);
    } else if (blended_terrain) {
        surface_material = terrain_material(material_position, base_normal,
            in.terrain_weights, material_detail, world_dx, world_dy);
    } else if ((material_id >= 81u && material_id <= 93u) || material_id == 119u || named_roof_finish) {
        surface_material = construction_material(in.construction_height, material_position, in.world_pos,
            base_normal, layer, material_id, world_dx, world_dy, material_detail);
        if matte_corrugated {
            surface_material.roughness = max(surface_material.roughness, 0.9);
        }
        if tile_stamped_metal {
            // A continuous coated sheet shares the crown profile, not ceramic firing.
            surface_material.tone *= 0.25;
            let filtered_variance = max(pow(surface_material.roughness, 4.0) - pow(0.67, 4.0), 0.0);
            surface_material.roughness = pow(pow(0.51, 4.0) + filtered_variance, 0.25);
        }
    } else {
        surface_material = sample_material(material_position,
            base_normal, layer, period_m, material_detail);
    }
    let tone = surface_material.tone;
    let tonal_detail = vec3<f32>(1.0)
        + min(tone, 0.0) * vec3<f32>(0.68, 0.58, 0.42)
        + max(tone, 0.0) * vec3<f32>(0.55, 0.42, 0.20);
    let n = surface_material.normal;
    let sun_dir = normalize(g.sun_dir.xyz);
    let moon_dir = normalize(g.moon_dir.xyz);
    let pc = g.planet_center.xyz;
    let stable_ecef = in.world_pos - pc;
    // The uniform weather sample describes the camera locality. Transient
    // wet/snow/wind effects must not recolor a continent when this fragment
    // belongs to terrain sampled at kilometre-scale orbital spacing.
    let local_weather_detail = 1.0 - smoothstep(48.0, 512.0, in.sample_pitch_m);

    // Local solar elevation is evaluated per fragment. From orbit this lets
    // noon, a warm terminator and night coexist on the same visible globe;
    // it also prevents east/west faces on the night side seeing the Sun.
    let up = normalize(in.world_pos - pc);
    let solar_alt = dot(up, sun_dir);
    let horizon = smoothstep(-0.015, 0.025, solar_alt);
    let ndl = max(dot(n, sun_dir), 0.0) * horizon;
    let base_ndl = max(dot(base_normal, sun_dir), 0.0) * horizon;
    let lunar_alt = dot(up, moon_dir);
    let moon_horizon = smoothstep(-0.01, 0.025, lunar_alt);
    let moon_phase = g.moon_dir.w;
    let moon_visibility = moon_horizon * moon_phase;
    // Keep creases readable without turning already-shadowed cliffs black.
    var ao = mix(0.62, 1.0, in.ao);
    if ((is_foliage || material_id == 14u) && (in.face_mat & 0x80000000u) != 0u) {
        ao = mix(0.68, 1.0, in.ao);
    }

    // Blue wavelengths travel the longest atmospheric path when the Sun
    // is low, leaving warm direct light. The curve is art-directed to
    // keep golden hour legible across the surface palette.
    let sun_height = max(solar_alt, 0.0);
    let daylight = smoothstep(0.035, 0.32, sun_height);
    let low_sun = vec3<f32>(1.42, 0.66, 0.24);
    var direct_color = mix(low_sun, vec3<f32>(1.68, 1.59, 1.36), daylight);
    direct_color *= mix(0.62, 1.0, smoothstep(0.0, 0.20, sun_height));

    // Warm light stays directional; unlit faces retain a cool sky fill.
    // Civil twilight fades into a deliberately faint blue night floor.
    let day_fill = smoothstep(-0.10, 0.16, solar_alt);
    let twilight = smoothstep(-0.22, -0.015, solar_alt)
        * (1.0 - smoothstep(0.06, 0.28, solar_alt));
    // Preserve a navigable blue-hour floor even without Moon light; the
    // scene should read as night without crushing terrain silhouettes.
    var ambient = mix(vec3<f32>(0.020, 0.028, 0.058), g.ambient.rgb, day_fill)
        + twilight * vec3<f32>(0.025, 0.010, 0.040);
    // The Moon remains a restrained cool key rather than a second Sun.
    // A small hemispheric lift keeps surface silhouettes readable while
    // the directional term gives moon-facing facets a crisp blue edge.
    ambient += vec3<f32>(0.020, 0.032, 0.070) * moon_visibility;
    let local_overcast = smoothstep(2.4, 8.5, g.weather5.w) * local_weather_detail;
    direct_color *= cloud_sunlight(in.world_pos, local_weather_detail,
        mix(1.0, 0.18, local_overcast));
    ambient = mix(
        ambient,
        vec3<f32>(0.13, 0.15, 0.18) * (0.38 + 0.62 * max(solar_alt, 0.0)),
        local_overcast * 0.72,
    );
    // A single shadowed key light gives terrain, structures and tree facets
    // the same form. Hemisphere fill keeps downward and side-facing surfaces
    // readable without flattening the crown's baked local occlusion.
    var shadow_ndl = base_ndl;
    if ((material_id >= 81u && material_id <= 93u) || (material_id >= 119u && material_id <= 121u) || named_roof_finish) {
        // Quantized facade normals can turn a backlit face toward grazing
        // sunlight. Shadow maps contain the actual triangle, so use its plane
        // for visibility while the authored normal retains its lighting role.
        let plane = cross(world_dx, world_dy);
        let facing = select(-1.0, 1.0, dot(plane, base_normal) >= 0.0);
        shadow_ndl = max(dot(plane, sun_dir) * facing, 0.0)
            / max(length(plane), 0.00000001) * horizon;
    }
    let key_visibility = sun_visibility(in.world_pos, shadow_ndl);
    let style_detail = 1.0 - smoothstep(48.0, 512.0, in.sample_pitch_m);
    let diffuse = mix(base_ndl,
        illustrative_diffuse(dot(n, sun_dir)) * horizon, style_detail);
    let base_sun = direct_color * diffuse * mix(0.08, 1.0, key_visibility);
    let fill_daylight = smoothstep(-0.10, 0.16, solar_alt);
    ambient = mix(ambient, illustrative_fill(ambient, dot(n, up), fill_daylight), style_detail);
    let base_moon = vec3<f32>(0.070, 0.100, 0.180)
        * max(dot(base_normal, moon_dir), 0.0) * moon_visibility;
    let normal_dx = dpdx(n);
    let normal_dy = dpdy(n);
    let normal_variance = min(0.25, max(dot(normal_dx, normal_dx), dot(normal_dy, normal_dy)));
    if (is_vehicle) {
        let vehicle_lit = vehicle_lighting(in, material_id, n, up, ambient, direct_color,
            key_visibility, horizon, moon_visibility, ao, normal_variance, vehicle_uv_dx, vehicle_uv_dy);
        return vec4<f32>(apply_aerial_perspective(vehicle_lit, in.world_pos, in.sample_pitch_m), 1.0);
    }
    if (RARE_SURFACES && material_id == 95u) {
        let metal_lit = sculpture_metal_lighting(in, n, up, ambient, direct_color,
            key_visibility, horizon, moon_visibility, ao, normal_variance);
        return vec4<f32>(apply_aerial_perspective(metal_lit, in.world_pos, in.sample_pitch_m), 1.0);
    }
    var albedo = select(in.color * tonal_detail, tower.color, is_highrise);
    if patterned_azulejo { albedo = azulejo_pattern_color(in.color, surface_material.tone); }
    if material_id == 1u {
        albedo = water_bed_color(albedo, material_position, water_footprint, water_kind);
    }
    var motion_light = 1.0;
    if (is_foliage) {
        let semantic = material_id - 22u;
        let season = semantic / 4u;
        let motion = semantic % 4u;
        let phase = g.surface.x;
        let autumn = smoothstep(0.56, 0.69, phase)
            * (1.0 - smoothstep(0.84, 0.94, phase));
        let winter = smoothstep(0.80, 0.96, phase)
            * (1.0 - smoothstep(0.08, 0.20, phase));
        let spring = smoothstep(0.08, 0.18, phase)
            * (1.0 - smoothstep(0.28, 0.38, phase));
        switch season {
            case 0u: { // evergreen needle
                albedo *= mix(vec3<f32>(0.82, 0.92, 0.80), vec3<f32>(1.0), 1.0 - winter * 0.18);
            }
            case 1u: { // evergreen broadleaf
                albedo *= mix(vec3<f32>(0.82, 0.90, 0.76), vec3<f32>(1.02, 1.04, 0.98), g.weather1.x);
            }
            case 2u: { albedo = mix(albedo, vec3<f32>(0.50, 0.29, 0.065), autumn * 0.82); }
            case 3u: { albedo = mix(albedo, vec3<f32>(0.62, 0.20, 0.055), autumn * 0.82); }
            case 4u: { albedo = mix(albedo, vec3<f32>(0.42, 0.055, 0.028), autumn * 0.88); }
            case 5u: { albedo = mix(albedo, vec3<f32>(0.55, 0.32, 0.06), autumn * 0.90); }
            case 6u: { // drought-deciduous follows moisture, not latitude
                let dry = clamp((1.0 - g.weather0.y) * (1.0 - g.weather1.x), 0.0, 1.0);
                albedo = mix(albedo, vec3<f32>(0.34, 0.26, 0.105), dry * 0.72);
            }
            case 7u: { albedo = mix(albedo, vec3<f32>(1.0, 0.48, 0.66), spring * 0.70); }
            case 8u: { albedo = mix(albedo, vec3<f32>(1.0, 0.72, 0.78), spring * 0.78); }
            case 9u: { albedo *= mix(vec3<f32>(0.78, 0.84, 0.65), vec3<f32>(1.04, 1.08, 0.98), g.weather1.x); }
            default: { albedo *= mix(vec3<f32>(0.90, 0.88, 0.72), vec3<f32>(1.0), g.weather0.y); }
        }
        // Stable shading shimmer: geometry, depth and collision never move.
        let wind_speed = length(g.wind.xy) + g.wind.w * 0.35;
        var amplitude = 0.018;
        switch motion {
            case 1u: { amplitude = 0.040; }
            case 2u: { amplitude = 0.060; }
            case 3u: { amplitude = 0.085; }
            default: {}
        }
        let distant_fade = 1.0 - smoothstep(18.0, 90.0, in.sample_pitch_m);
        let phase_motion = dot(stable_ecef * 0.0025, vec3<f32>(0.71, 0.37, 0.53))
            + g.weather2.x * (0.7 + wind_speed * 0.08);
        motion_light += sin(phase_motion) * amplitude
            * smoothstep(0.8, 12.0, wind_speed) * distant_fade;
    }
    var field_runtime_presence = 1.0;
    if (material_id >= 72u && material_id <= 78u) { // semantic field surfaces
        let local_phase = g.surface.x;
        let astronomical_phase = g.weather6.w;
        let moisture = clamp(max(g.weather1.x, g.weather6.y), 0.0, 1.0);
        let monsoonal = smoothstep(0.58, 0.78, g.weather0.y)
            * smoothstep(0.20, 0.54, moisture);
        let tropical = smoothstep(18.0, 24.0, g.weather0.x) * (1.0 - monsoonal);
        let tropical_cycle = fract(astronomical_phase * 2.0);
        let temperate_growth = smoothstep(0.10, 0.25, local_phase)
            * (1.0 - smoothstep(0.67, 0.84, local_phase));
        let temperate_bloom = smoothstep(0.24, 0.36, local_phase)
            * (1.0 - smoothstep(0.52, 0.62, local_phase));
        let temperate_mature = smoothstep(0.46, 0.57, local_phase)
            * (1.0 - smoothstep(0.70, 0.80, local_phase));
        let temperate_senescent = smoothstep(0.62, 0.76, local_phase)
            * (1.0 - smoothstep(0.91, 0.98, local_phase));
        let temperate_stubble = smoothstep(0.74, 0.82, local_phase)
            * (1.0 - smoothstep(0.94, 0.995, local_phase));
        let tropical_growth = smoothstep(0.05, 0.18, tropical_cycle)
            * (1.0 - smoothstep(0.66, 0.82, tropical_cycle)) * moisture;
        let tropical_bloom = smoothstep(0.27, 0.38, tropical_cycle)
            * (1.0 - smoothstep(0.54, 0.64, tropical_cycle)) * moisture;
        let monsoon_growth = smoothstep(0.20, 0.58, moisture);
        let growth = mix(
            temperate_growth,
            mix(tropical_growth, monsoon_growth, monsoonal),
            max(tropical, monsoonal),
        );
        let bloom = mix(
            temperate_bloom,
            mix(tropical_bloom, monsoon_growth * 0.72, monsoonal),
            max(tropical, monsoonal),
        );
        let mature = mix(temperate_mature, 1.0 - growth, max(tropical, monsoonal));
        let senescent = max(
            temperate_senescent * (1.0 - max(tropical, monsoonal)),
            g.weather4.x * 0.76,
        );
        let stubble = temperate_stubble * (1.0 - max(tropical, monsoonal));
        let annual_structure = max(max(growth, mature), max(senescent, stubble));
        field_runtime_presence = annual_structure;
        // Seasonal material targets are linear albedo, matching decoded vertex colors.
        if (material_id == 72u) { // annual crop
            // The regional catalog owns soil color; field descriptors own the
            // crop calendar. An independent terrain clock would turn a green
            // rice/vegetable field into ripe cereal beneath its living canopy.
            field_runtime_presence = 0.0;
        } else if (material_id == 73u) { // commercial/formal flower surface
            // Exact flower calendar/palette state lives in FloraCell, not the
            // terrain vertex. Keep the observed/procedural substrate neutral;
            // close plants and distant aggregates provide all synthetic
            // seasonal flower colour without a mismatched terrain clock.
            albedo = mix(albedo, vec3<f32>(0.12596739, 0.08898153, 0.04696420), 0.08);
            field_runtime_presence = 0.0;
        } else if (material_id == 74u) { // orchard
            albedo = mix(albedo, vec3<f32>(0.76776876, 0.46236137, 0.50707851), temperate_bloom * 0.48);
            albedo = mix(albedo, vec3<f32>(0.38005634, 0.10048151, 0.04323364), senescent * 0.72);
            albedo = mix(vec3<f32>(0.12596739, 0.08898153, 0.04696420), albedo, 0.18 + growth * 0.82);
            field_runtime_presence = max(growth, max(temperate_bloom, senescent));
        } else if (material_id == 75u) { // vineyard
            // Alleys retain their substrate through the vine's leaf cycle.
            field_runtime_presence = 0.0;
        } else if (material_id == 76u) { // meadow
            albedo = mix(albedo, vec3<f32>(0.17886750, 0.26327341, 0.09462963), growth * 0.26);
            albedo = mix(albedo, vec3<f32>(0.33034371, 0.27383842, 0.11928019), senescent * 0.46);
            field_runtime_presence = max(growth, senescent);
        } else if (material_id == 77u) { // dormant rows
            albedo = mix(albedo, vec3<f32>(0.14731900, 0.22341399, 0.07828832), growth * 0.38);
            field_runtime_presence = growth;
        } else { // harvested/stubble
            albedo = mix(albedo, vec3<f32>(0.24286681, 0.15487249, 0.06372380), 0.28 + (1.0 - growth) * 0.52);
            field_runtime_presence = max(stubble, growth);
        }
    }
    if (blended_terrain || material_id == 3u || material_id == 70u
        || (material_id >= 72u && material_id <= 76u)) { // vegetated ground response
        let vegetation_weight = select(1.0, in.terrain_weights.y, blended_terrain);
        // Humid, drought-free vegetation stays green between rain events.
        // Fields still retain their lifecycle presence above.
        let growing_season = smoothstep(0.06, 0.22, g.surface.x)
            * (1.0 - smoothstep(0.74, 0.94, g.surface.x));
        let upward_vegetation = smoothstep(0.12, 0.55, dot(base_normal, up));
        let climate_green = g.weather0.y * (1.0 - g.weather4.x)
            * smoothstep(-8.0, 8.0, g.weather0.x) * growing_season
            * upward_vegetation;
        let green_flush = max(g.weather1.x * g.weather0.y, climate_green * 0.60)
            * local_weather_detail
            * select(1.0, field_runtime_presence, material_id >= 72u && material_id <= 78u);
        let dry = select(
            clamp((1.0 - g.weather0.y) * max(g.weather0.x / 32.0, 0.0), 0.0, 1.0),
            0.0,
            material_id == 70u && !blended_terrain,
        ) * local_weather_detail;
        // Pigment response preserves source variation and dark grass detail.
        // Crop lifecycle colours retain their separate, restrained flush.
        if (blended_terrain || material_id == 3u || material_id == 70u) {
            let healthy_grass = smoothstep(0.08, 0.42, green_flush) * vegetation_weight;
            albedo *= mix(vec3<f32>(1.0), vec3<f32>(0.86, 0.97, 0.78), healthy_grass);
        } else {
            albedo = mix(albedo, vec3<f32>(0.25, 0.34, 0.12), green_flush * 0.12 * vegetation_weight);
        }
        albedo = mix(albedo, vec3<f32>(0.40, 0.30, 0.14), dry * 0.26 * vegetation_weight);
    }
    let base_lighting = (ambient + base_sun + base_moon) * ao;
    var lit = albedo * motion_light * base_lighting;

    let view = -normalize(in.world_pos);
    let half_direction = normalize(view + sun_dir);
    let roughness = clamp(surface_material.roughness, 0.12, 1.0);
    // Unresolved normal variation broadens the lobe instead of producing
    // isolated hot pixels that spatial edge smoothing cannot reconstruct.
    let specular_power = clamp(2.0 / (pow(roughness, 4.0) + normal_variance) - 2.0, 2.0, select(192.0, 2048.0, material_id == 1u));
    let fresnel = select(0.035 + 0.965 * pow(1.0 - max(dot(view, half_direction), 0.0), 5.0),
        water_fresnel(dot(view, half_direction)), material_id == 1u);
    let specular = pow(max(dot(n, half_direction), 0.0), specular_power)
        * ((specular_power + 2.0) * 0.125) * fresnel
        * ndl * key_visibility * material_detail;
    // Rough stone and plaster have a broad, weak response; metal and water
    // preserve a tighter highlight. Relief changes light, not geometry.
    if (!(is_foliage || material_id == 14u) && material_id != 1u) {
        let reflectance = select(vec3<f32>(0.55), albedo * 3.0, material_id == 16u);
        lit += direct_color * reflectance * specular;
    }
    if (is_foliage || material_id == 14u) {
        let backlight = pow(max(dot(-view, sun_dir), 0.0), 3.0) * horizon;
        lit += albedo * direct_color * backlight * 0.085 * ao
            * mix(0.35, 1.0, key_visibility);
    }

    // Runtime surface response never changes chunks or cache keys.
    let emissive_surface = material_id == 15u || (material_id >= 18u && material_id <= 21u)
        || material_id == 71u || material_id == 79u || material_id == 80u || material_id == 94u;
    let glass_surface = material_id == 15u || material_id == 18u || is_highrise;
    let submerged = material_id == 1u;
    // Smooth terrain uses the same continuous frozen coverage for weather;
    // a flat owner id must not reintroduce a seam through rain or snow.
    let weather_coverage = clamp(1.0 - in.terrain_weights.w, 0.0, 1.0);
    let wettable_surface = blended_terrain || (material_id != 67u && material_id != 68u);
    let snow_eligible_surface = blended_terrain || material_id != 68u;
    let wet_coverage = select(1.0, weather_coverage, blended_terrain);
    if (!emissive_surface && !submerged && wettable_surface) {
        let wet = g.weather1.x * local_weather_detail * wet_coverage;
        lit *= mix(1.0, 0.70, wet);
        let broad_reflection = pow(
            max(dot(reflect(-normalize(in.world_pos), base_normal), up), 0.0),
            10.0,
        );
        lit += vec3<f32>(0.16, 0.22, 0.30) * broad_reflection * wet;
    }
    if (!emissive_surface && !submerged && snow_eligible_surface) {
        let snow = g.weather1.y * local_weather_detail * wet_coverage
            * smoothstep(0.35, 0.78, dot(base_normal, up));
        lit = mix(
            lit,
            vec3<f32>(0.72, 0.80, 0.88)
                * (ambient + base_sun * 0.72),
            snow,
        );
    }
    if (submerged) {
        let sea = g.surface.y * water_shelter.y * local_weather_detail;
        var reflected_sky = vec3<f32>(0.0);
        if (material_detail > 0.0) {
            reflected_sky = sample_reflected_environment(reflect(-view, n));
        }
        let reflection = water_fresnel(max(dot(n, view), 0.0));
        // Depth/sediment colours describe the opaque water body. Fresnel
        // replaces that response with the sky as the viewing angle grazes it.
        lit *= mix(1.0, 0.76, sea);
        lit += water_crest_scattering(albedo, water.crest, dot(n, view), dot(-view, sun_dir),
            direct_color * horizon * key_visibility) * material_detail;
        lit = mix(lit, reflected_sky, reflection * material_detail);
        lit += direct_color * specular * 0.35;
    }
    if (glass_surface && material_detail > 0.0) {
        let environment = sample_reflected_environment(reflect(-view, n));
        let glass_fresnel = 0.04 + 0.96 * pow(1.0 - max(dot(n, view), 0.0), 5.0);
        // Reflection replaces part of the dark interior response. A minimum
        // sky-colour floor would brighten every distant window independently
        // of incidence, erasing the authored facade and joinery palette.
        let coverage = select(1.0, tower.glass, is_highrise);
        let reflection = select(0.06 + 0.72 * glass_fresnel, 0.025 + 0.30 * glass_fresnel, is_highrise);
        lit = mix(lit, environment, reflection * material_detail * coverage);
    }

    if is_highrise && g.viewport.w > 0.5 {
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        lit += tower.emission * night;
    }

    // Treat the discharge as a localized cloud-height source instead of a
    // global color lift. Nearby faces turn toward it, range falls off through
    // the storm, and the existing fog below attenuates the result.
    if g.weather2.w > 0.002 {
        let strike_latitude = g.weather4.z;
        let strike_longitude = g.weather4.w;
        let strike_cos_latitude = cos(strike_latitude);
        let strike_direction = vec3<f32>(
            strike_cos_latitude * cos(strike_longitude),
            strike_cos_latitude * sin(strike_longitude),
            sin(strike_latitude),
        );
        let strike_ground = g.planet_center.xyz
            + strike_direction * g.planet_center.w;
        let electrical_source = strike_ground + strike_direction
            * mix(g.weather3.x, g.weather3.y, 0.42);
        let toward_source = electrical_source - in.world_pos;
        let source_distance = length(toward_source);
        let facing = max(dot(n, toward_source / max(source_distance, 1.0)), 0.0);
        let range_falloff = exp(-source_distance / 24000.0);
        let electrical_light = (0.10 + facing * 0.52) * range_falloff * g.weather2.w;
        lit += albedo * vec3<f32>(0.46, 0.57, 0.88) * electrical_light;
    }
    // Lit panes already have source-fitted opening geometry. Emission fills
    // that opening without a second grid inside it. Legacy facade proxies
    // retain their area coverage; horizontal surfaces never emit.
    var window_coverage = 0.0;
    switch layer {
        case 18u: { window_coverage = 1.0; }
        case 19u: { window_coverage = 0.1008; }
        case 20u: { window_coverage = 0.20; }
        case 21u: { window_coverage = 0.3456; }
        default: {}
    }
    if (g.viewport.w > 0.5 && window_coverage > 0.0) {
        let vertical_face = 1.0 - smoothstep(0.30, 0.72, abs(dot(n, up)));
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        // Match the warm residential highrise light's color and radiance.
        lit += vec3<f32>(0.86, 0.58, 0.29) * 0.16 * window_coverage * night * vertical_face;
    }

    // Observation-tower obstruction lamp. Unlike facade apertures, the
    // entire tiny cap emits so it remains readable from aircraft distance;
    // daylight retains the physical red-painted fixture without a glow.
    if (g.viewport.w > 0.5 && material_id == 71u) {
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        lit += vec3<f32>(1.15, 0.012, 0.006) * night;
    }

    // Curated-landmark fixtures retain their authored surface colors by day and
    // become emissive only at night. Eiffel sparkle fixtures join the warm
    // architectural wash between shows, then switch to a sparse on/off
    // multi-colour duty cycle for five minutes from every simulation-clock
    // hour mark. Each fixture has a stable phase but re-rolls several times
    // per second, producing glitter rather than a synchronized tower pulse.
    if (RARE_SURFACES && g.viewport.w > 0.5 && (material_id == 79u || material_id == 80u)) {
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        let hourly_show = material_id == 80u && g.temporal.w < 300.0;
        if (!hourly_show) {
            let steady_strength = select(0.92, 0.54, material_id == 80u);
            lit += vec3<f32>(1.24, 0.43, 0.055) * night * steady_strength;
        } else {
            let fixture_phase = dot(
                floor(abs(in.local_pos) * 0.5),
                vec3<f32>(12.9898, 78.233, 37.719),
            );
            let flash_tick = floor(g.weather2.x * 8.0);
            let flash_hash = fract(sin(fixture_phase + flash_tick * 17.17) * 43758.5453);
            let colour_hash = fract(sin(fixture_phase * 1.71 + flash_tick * 3.11) * 24634.6345);
            var sparkle_colour = vec3<f32>(1.30, 0.18, 0.62);
            if (colour_hash < 0.25) {
                sparkle_colour = vec3<f32>(0.12, 0.82, 1.42);
            } else if (colour_hash < 0.50) {
                sparkle_colour = vec3<f32>(0.24, 1.24, 0.38);
            } else if (colour_hash < 0.75) {
                sparkle_colour = vec3<f32>(1.48, 0.72, 0.08);
            }
            // A fixture below the threshold contributes no emission at all.
            // Roughly one third are lit on any tick; the independently
            // hashed next tick turns some off while lighting others.
            let flash = smoothstep(0.67, 0.78, flash_hash)
                * (1.35 + smoothstep(0.78, 0.98, flash_hash) * 1.25);
            lit += sparkle_colour * night * flash;
        }
    }

    // Catalog fixtures retain the artist's color and remain perfectly steady.
    // Their emission is attenuated by the same atmosphere as their housing.
    if (g.viewport.w > 0.5 && material_id == 94u) {
        let night = 1.0 - smoothstep(-0.10, 0.07, solar_alt);
        lit += in.color * (2.4 * night);
    }
    return vec4<f32>(apply_aerial_perspective(lit, in.world_pos, in.sample_pitch_m), 1.0);
}
