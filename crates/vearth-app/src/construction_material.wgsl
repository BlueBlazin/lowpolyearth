// Pattern coverage is linear data so filtering preserves the two-pigment mean.
// The region owns ink RGB; the ceramic ground is sRGB(232,234,225).
fn azulejo_pattern_color(ink: vec3<f32>, coverage: f32) -> vec3<f32> {
    return mix(vec3<f32>(0.806952258, 0.822785754, 0.752942217), ink,
        clamp(coverage, 0.0, 1.0));
}

// Geodetic altitude keeps masonry courses level at every latitude. The
// CPU uploads its 256-metre phase, a common period of masonry and tower floors,
// in f64. A local ellipsoid gradient and curvature correction preserve it
// across chunks without large f32 heights.
fn construction_height(position: vec3<f32>, model: mat4x4<f32>, phase: f32) -> f32 {
    let origin = model[3].xyz - g.planet_center.xyz;
    let up = normalize(origin * vec3<f32>(1.0, 1.0, 1.0067395));
    let delta = (model * vec4<f32>(position, 0.0)).xyz;
    let rise = dot(delta, up);
    let curvature = (dot(delta, delta) - rise * rise) / (2.0 * max(length(origin), 1.0));
    return phase + rise + curvature;
}

fn construction_material(height: f32, position: vec3<f32>, world: vec3<f32>, normal: vec3<f32>,
                         layer: u32, material_id: u32, world_dx: vec3<f32>, world_dy: vec3<f32>,
                         detail: f32) -> MaterialSample {
    let up = normalize((world - g.planet_center.xyz) * vec3<f32>(1.0, 1.0, 1.0067395));
    let tangent = abs(cross(up, normal));
    // Project onto the strongest horizontal world axis. Unlike longitude UVs,
    // this has no dateline seam or polar squeezing; projected scale differs
    // by at most sqrt(3) with wall orientation, as with ordinary box mapping.
    var horizontal = position.x;
    var horizontal_dx = world_dx.x;
    var horizontal_dy = world_dy.x;
    if (tangent.y > tangent.x && tangent.y >= tangent.z) {
        horizontal = position.y;
        horizontal_dx = world_dx.y;
        horizontal_dy = world_dy.y;
    }
    if (tangent.z > tangent.x && tangent.z > tangent.y) {
        horizontal = position.z;
        horizontal_dx = world_dx.z;
        horizontal_dy = world_dy.z;
    }
    var uv = vec2<f32>(horizontal, height) / 4.0;
    // The position contains an up-to-8192 m texture phase. Differencing it
    // quantizes subpixel slopes, causing speckled relief at close range.
    // Project the camera-relative derivatives before adding that phase.
    var uv_dx = vec2<f32>(horizontal_dx, dpdx(height)) / 4.0;
    var uv_dy = vec2<f32>(horizontal_dy, dpdy(height)) / 4.0;
    if (material_id == 12u || material_id == 85u || material_id == 86u || material_id == 88u || material_id == 90u || material_id == 91u || material_id == 92u || abs(dot(normal, up)) > 0.72) {
        let axes = abs(normal);
        uv = position.xy / 4.0;
        uv_dx = world_dx.xy / 4.0;
        uv_dy = world_dy.xy / 4.0;
        if (axes.x >= axes.y && axes.x >= axes.z) {
            uv = position.yz / 4.0;
            uv_dx = world_dx.yz / 4.0;
            uv_dy = world_dy.yz / 4.0;
        } else if (axes.y >= axes.z) {
            uv = position.xz / 4.0;
            uv_dx = world_dx.xz / 4.0;
            uv_dy = world_dy.xz / 4.0;
        }
    }
    let sample = textureSampleGrad(atlas_tex, atlas_samp, uv, i32(layer), uv_dx, uv_dy);
    let derivative = (sample.gb - vec2<f32>(0.5)) * 4.0;
    let r1 = cross(world_dy, normal);
    let r2 = cross(normal, world_dx);
    let determinant = dot(world_dx, r1);
    var gradient = vec3<f32>(0.0);
    if (abs(determinant) > 0.00000001) {
        gradient = (dot(derivative, uv_dx) * r1 + dot(derivative, uv_dy) * r2) / determinant;
    }
    // Patterned glaze carries coverage instead of centered pigment variation.
    // Retain its filtered mean at all distances; only relief uses detail fading.
    let tone = select((sample.r - 0.5) * 1.6 * detail, sample.r, layer == 54u);
    return MaterialSample(tone,
        normalize(normal - gradient * detail), sample.a);
}
