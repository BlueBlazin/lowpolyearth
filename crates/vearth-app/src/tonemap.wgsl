// Full-resolution HDR resolve. The world and atmosphere compose in linear
// Rgba16Float; this final pass applies exposure and a compact filmic curve,
// then finishes the creative grade before the surface's sRGB view performs
// gamma encoding.

struct Globals {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    moon_dir: vec4<f32>,
    ambient: vec4<f32>,     // w = exposure
    inv_view_proj: mat4x4<f32>,
    planet_center: vec4<f32>,
    viewport: vec4<f32>,
    sky: vec4<f32>,
    horizon_glow_0: vec4<f32>,
    horizon_glow_1: vec4<f32>,
};

struct PostSettings {
    // color-grade LUT, filmic saturation, unused, unused
    flags: vec4<u32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var hdr_tex: texture_2d<f32>;
@group(1) @binding(1) var grade_lut: texture_3d<f32>;
@group(1) @binding(2) var grade_sampler: sampler;
@group(1) @binding(3) var scene_depth: texture_depth_2d;
@group(2) @binding(0) var<uniform> post: PostSettings;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, y, 0.5, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

fn filmic_curve(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((color * (a * color + b)) /
        (color * (c * color + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn filmic_aces(color: vec3<f32>) -> vec3<f32> {
    let mapped = filmic_curve(color);
    let peak = max(color.r, max(color.g, color.b));
    // A shared highlight gain retains pigment hue as the brightest channel
    // enters the shoulder. Neutral highlights and the dark toe stay unchanged.
    let highlight = color * (max(mapped.r, max(mapped.g, mapped.b)) / max(peak, 1e-5));
    return mix(mapped, highlight, 0.65 * smoothstep(0.25, 0.90, peak));
}

fn linear_rgb_to_oklab(color: vec3<f32>) -> vec3<f32> {
    let lms = vec3<f32>(
        dot(color, vec3<f32>(0.4122214708, 0.5363325363, 0.0514459929)),
        dot(color, vec3<f32>(0.2119034982, 0.6806995451, 0.1073969566)),
        dot(color, vec3<f32>(0.0883024619, 0.2817188376, 0.6299787005)),
    );
    let root = pow(max(lms, vec3<f32>(0.0)), vec3<f32>(1.0 / 3.0));
    return vec3<f32>(
        dot(root, vec3<f32>(0.2104542553, 0.7936177850, -0.0040720468)),
        dot(root, vec3<f32>(1.9779984951, -2.4285922050, 0.4505937099)),
        dot(root, vec3<f32>(0.0259040371, 0.7827717662, -0.8086757660)),
    );
}

fn oklab_to_linear_rgb(lab: vec3<f32>) -> vec3<f32> {
    let root = vec3<f32>(
        lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z,
        lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z,
        lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z,
    );
    let lms = root * root * root;
    return vec3<f32>(
        dot(lms, vec3<f32>(4.0767416621, -3.3077115913, 0.2309699292)),
        dot(lms, vec3<f32>(-1.2684380046, 2.6097574011, -0.3413193965)),
        dot(lms, vec3<f32>(-0.0041960863, -0.7034186147, 1.7076147010)),
    );
}

fn compress_gamut(color: vec3<f32>) -> vec3<f32> {
    let luminance = clamp(
        dot(color, vec3<f32>(0.2126, 0.7152, 0.0722)),
        0.0,
        1.0,
    );
    let minimum = min(color.r, min(color.g, color.b));
    let maximum = max(color.r, max(color.g, color.b));
    var scale = 1.0;
    if minimum < 0.0 {
        scale = min(scale, luminance / max(luminance - minimum, 1e-5));
    }
    if maximum > 1.0 {
        scale = min(scale, (1.0 - luminance) / max(maximum - luminance, 1e-5));
    }
    return vec3<f32>(luminance) + (color - vec3<f32>(luminance)) * clamp(scale, 0.0, 1.0);
}

fn filmic_saturation(color: vec3<f32>) -> vec3<f32> {
    var lab = linear_rgb_to_oklab(max(color, vec3<f32>(0.0)));
    let chroma = length(lab.yz);
    let midtone = smoothstep(0.08, 0.32, lab.x)
        * (1.0 - smoothstep(0.72, 0.98, lab.x));
    let unsaturated = 1.0 - smoothstep(0.055, 0.26, chroma);
    let highlight_rolloff = 1.0 - smoothstep(0.78, 1.0, lab.x);
    // Preserve the authored hue range; only lift subdued midtones, and roll
    // already vivid colors gently inward instead of amplifying field greens.
    let vivid = smoothstep(0.12, 0.27, chroma);
    let gain = 1.0 + 0.06 * midtone * unsaturated * highlight_rolloff - 0.10 * vivid;
    lab = vec3<f32>(lab.x, lab.yz * gain);
    return clamp(compress_gamut(oklab_to_linear_rgb(lab)), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Edge detection uses display-referred contrast; averaging HDR first would
// re-saturate bright silhouettes when the filmic curve is applied afterward.
fn exposed_color(pixel: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(hdr_tex));
    let address = clamp(pixel, vec2<i32>(0), size - vec2<i32>(1));
    let hdr = max(textureLoad(hdr_tex, address, 0).rgb, vec3<f32>(0.0));
    let ndc = (vec2<f32>(address) + vec2<f32>(0.5)) / vec2<f32>(size) * 2.0 - 1.0;
    let vignette = 1.0 - 0.045 * smoothstep(0.30, 1.45, dot(ndc, ndc));
    return filmic_aces(hdr * g.ambient.w * vignette);
}

fn perceptual_luma(color: vec3<f32>) -> f32 {
    return sqrt(max(dot(color, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0));
}

fn creative_grade(color: vec3<f32>) -> vec3<f32> {
    var mapped = color;
    if post.flags.x != 0u {
        let lut_size = vec3<f32>(textureDimensions(grade_lut));
        let lut_coordinate = (clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0))
            * (lut_size - vec3<f32>(1.0)) + vec3<f32>(0.5)) / lut_size;
        mapped = textureSampleLevel(grade_lut, grade_sampler, lut_coordinate, 0.0).rgb;
    }
    if post.flags.y != 0u {
        mapped = filmic_saturation(mapped);
    }
    return mapped;
}

fn edge_pair_luma(pixel: vec2<i32>, normal: vec2<i32>) -> f32 {
    return 0.5 * (perceptual_luma(exposed_color(pixel))
        + perceptual_luma(exposed_color(pixel + normal)));
}

// Independent directional resolve, informed by Timothy Lottes' FXAA paper:
// https://developer.download.nvidia.com/assets/gamedev/files/sdk/11/FXAA_WhitePaper.pdf
// Five HDR reads on low-contrast pixels; at most 21 on an edge. Search and
// interpolation stay in this pass and require no history or HDR sampler.
fn resolve_edges(pixel: vec2<i32>, center: vec3<f32>) -> vec3<f32> {
    let north = exposed_color(pixel + vec2<i32>(0, -1));
    let south = exposed_color(pixel + vec2<i32>(0, 1));
    let west = exposed_color(pixel + vec2<i32>(-1, 0));
    let east = exposed_color(pixel + vec2<i32>(1, 0));
    let m = perceptual_luma(center);
    let n = perceptual_luma(north);
    let s = perceptual_luma(south);
    let w = perceptual_luma(west);
    let e = perceptual_luma(east);
    let low = min(m, min(min(n, s), min(w, e)));
    let high = max(m, max(max(n, s), max(w, e)));
    if high - low < max(0.03125, high * 0.125) {
        return creative_grade(center);
    }

    let nw = perceptual_luma(exposed_color(pixel + vec2<i32>(-1, -1)));
    let ne = perceptual_luma(exposed_color(pixel + vec2<i32>(1, -1)));
    let sw = perceptual_luma(exposed_color(pixel + vec2<i32>(-1, 1)));
    let se = perceptual_luma(exposed_color(pixel + vec2<i32>(1, 1)));
    let bend_y = abs(nw - 2.0 * w + sw) + 2.0 * abs(n - 2.0 * m + s)
        + abs(ne - 2.0 * e + se);
    let bend_x = abs(nw - 2.0 * n + ne) + 2.0 * abs(w - 2.0 * m + e)
        + abs(sw - 2.0 * s + se);
    let horizontal = bend_y >= bend_x;
    let before = select(w, n, horizontal);
    let after = select(e, s, horizontal);
    let negative = abs(before - m) >= abs(after - m);
    let neighbor_luma = select(after, before, negative);
    let normal = select(vec2<i32>(1, 0), vec2<i32>(0, 1), horizontal)
        * select(1, -1, negative);
    let tangent = select(vec2<i32>(0, 1), vec2<i32>(1, 0), horizontal);
    let neighbor = select(select(east, west, negative),
        select(south, north, negative), horizontal);
    let reference = 0.5 * (m + neighbor_luma);
    let gradient = abs(neighbor_luma - m);
    if gradient < 0.00001 {
        return creative_grade(center);
    }

    // Reuse the 3x3 neighborhood for the first search step on both sides.
    let pair_before = 0.5 * select(n + select(ne, nw, negative),
        w + select(sw, nw, negative), horizontal);
    let pair_after = 0.5 * select(s + select(se, sw, negative),
        e + select(se, ne, negative), horizontal);
    let local_before = pair_before - reference;
    let local_after = pair_after - reference;
    var delta_before = local_before;
    var delta_after = local_after;
    let end_threshold = gradient * 0.25;
    var found_before = abs(delta_before) >= end_threshold;
    var found_after = abs(delta_after) >= end_threshold;
    var distance_before = 1.0;
    var distance_after = 1.0;
    for (var step = 0u; step < 3u; step += 1u) {
        if found_before && found_after {
            break;
        }
        let distance = i32(2u << step);
        if !found_before {
            delta_before = edge_pair_luma(pixel - tangent * distance, normal) - reference;
            distance_before = f32(distance);
            found_before = abs(delta_before) >= end_threshold;
        }
        if !found_after {
            delta_after = edge_pair_luma(pixel + tangent * distance, normal) - reference;
            distance_after = f32(distance);
            found_after = abs(delta_after) >= end_threshold;
        }
    }

    let nearest_before = distance_before <= distance_after;
    let endpoint_delta = select(delta_after, delta_before, nearest_before);
    let found_endpoint = select(found_after, found_before, nearest_before);
    var coverage = 0.0;
    if found_endpoint && (endpoint_delta < 0.0) != (m - reference < 0.0) {
        coverage = 0.5 - min(distance_before, distance_after)
            / (distance_before + distance_after);
    }
    // A diagonal passing between opposite corners needs fractional coverage
    // even when both endpoints are equally close. Opposite signs reject
    // aligned edges, checkerboards and isolated emissive points.
    if abs(local_before) >= end_threshold && abs(local_after) >= end_threshold
        && local_before * local_after < 0.0 {
        let diagonal = 0.375 * min(abs(local_before), abs(local_after)) / gradient;
        coverage = max(coverage, min(diagonal, 0.1875));
    }
    if coverage <= 0.0 {
        return creative_grade(center);
    }
    // Convex interpolation after the grade cannot introduce ringing, alter
    // hue through a second tone-map, or spread a highlight beyond one pixel.
    return mix(creative_grade(center), creative_grade(neighbor), min(coverage, 0.4375));
}

// Camera-relative positions are in metres. Zero reversed-Z is sky; samples
// beyond the viewport are absent geometry, never clamped foreground walls.
fn contact_position(pixel: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(scene_depth));
    if any(pixel < vec2<i32>(0)) || any(pixel >= size) {
        return vec4<f32>(0.0);
    }
    let depth = textureLoad(scene_depth, pixel, 0);
    if depth <= 0.0 {
        return vec4<f32>(0.0);
    }
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / vec2<f32>(size);
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let point = g.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
    if abs(point.w) < 1e-8 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(point.xyz / point.w, 1.0);
}

// A restrained post contact approximation, not an indirect-light solution.
// Surface-normal bias and true metre-distance rejection follow the geometric
// obscurance model in McGuire et al., Alchemy (HPG 2011):
// https://research.nvidia.com/publication/2011-08_alchemy-screen-space-ambient-obscurance-algorithm
// No history or blur crosses silhouettes. At most thirteen depth reads; only
// eight samples estimate occlusion, and the remainder reconstruct a normal.
fn contact_visibility(pixel: vec2<i32>, hdr: vec3<f32>) -> f32 {
    let brightness = max(hdr.r, max(hdr.g, hdr.b)) * g.ambient.w;
    let emissive_guard = 1.0 - smoothstep(0.65, 1.4, brightness);
    if emissive_guard <= 0.0 {
        return 1.0;
    }
    let center = contact_position(pixel);
    if center.w == 0.0 {
        return 1.0;
    }
    let distance_fade = 1.0 - smoothstep(120.0, 320.0, length(center.xyz));
    if distance_fade <= 0.0 {
        return 1.0;
    }
    let left = contact_position(pixel + vec2<i32>(-1, 0));
    let right = contact_position(pixel + vec2<i32>(1, 0));
    let above = contact_position(pixel + vec2<i32>(0, -1));
    let below = contact_position(pixel + vec2<i32>(0, 1));
    let dl = center.xyz - left.xyz;
    let dr = right.xyz - center.xyz;
    let da = center.xyz - above.xyz;
    let db = below.xyz - center.xyz;
    let left_length = select(1e20, dot(dl, dl), left.w > 0.0);
    let right_length = select(1e20, dot(dr, dr), right.w > 0.0);
    let above_length = select(1e20, dot(da, da), above.w > 0.0);
    let below_length = select(1e20, dot(db, db), below.w > 0.0);
    // One-sided derivatives stay on the closest surface at a depth edge.
    if min(left_length, right_length) >= 4.0
        || min(above_length, below_length) >= 4.0 {
        return 1.0;
    }
    let dx = select(dr, dl, left_length <= right_length);
    let dy = select(db, da, above_length <= below_length);
    let unnormalized = cross(dx, dy);
    if dot(unnormalized, unnormalized) < 1e-12 {
        return 1.0;
    }
    var normal = normalize(unnormalized);
    if dot(normal, center.xyz) > 0.0 {
        normal = -normal;
    }
    let footprint = max(min(length(dx), length(dy)), 0.0001);
    let radius_pixels = clamp(2.0 / footprint, 2.0, 48.0);
    let bias = max(0.025, footprint * 0.06);
    let taps = array<vec2<f32>, 8>(
        vec2<f32>(0.35, 0.0), vec2<f32>(-0.35, 0.0),
        vec2<f32>(0.0, 0.35), vec2<f32>(0.0, -0.35),
        vec2<f32>(0.60, 0.60), vec2<f32>(-0.60, 0.60),
        vec2<f32>(0.60, -0.60), vec2<f32>(-0.60, -0.60),
    );
    var obscurance = 0.0;
    for (var index = 0u; index < 8u; index += 1u) {
        let offset = vec2<i32>(round(taps[index] * radius_pixels));
        let sample = contact_position(pixel + offset);
        if sample.w == 0.0 {
            continue;
        }
        let difference = sample.xyz - center.xyz;
        let distance_squared = dot(difference, difference);
        // A nearby screen silhouette is not necessarily nearby geometry.
        if distance_squared <= 0.000001 || distance_squared >= 4.0 {
            continue;
        }
        let projected = max(dot(normal, difference) - bias, 0.0)
            * inverseSqrt(distance_squared);
        let falloff = 1.0 - distance_squared * 0.25;
        obscurance += projected * falloff * falloff;
    }
    return 1.0 - min(obscurance * (0.60 / 8.0), 0.14)
        * distance_fade * emissive_guard;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.pos.xy);
    let hdr = max(textureLoad(hdr_tex, pixel, 0).rgb, vec3<f32>(0.0));
    let vignette = 1.0 - 0.045 * smoothstep(0.30, 1.45, dot(in.ndc, in.ndc));
    let center = filmic_aces(hdr * g.ambient.w * vignette);
    var mapped = resolve_edges(pixel, center);
    mapped *= contact_visibility(pixel, hdr);

    // Dither is bounded in display codes, not linear light. A conservative
    // inverse-sRGB-slope approximation prevents amplified noise in dark roof
    // pigments while retaining band suppression in sky gradients.
    let noise = fract(52.9829189 * fract(dot(floor(in.pos.xy),
        vec2<f32>(0.06711056, 0.00583715))));
    let dither_weight = smoothstep(0.002, 0.04, max(hdr.r, max(hdr.g, hdr.b)));
    let display_step = max(vec3<f32>(1.0 / 12.92),
        1.3 * sqrt(max(mapped, vec3<f32>(0.0))) + 0.9749 * mapped);
    mapped += display_step * ((noise - 0.5) / 255.0 * dither_weight);
    return vec4<f32>(clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
