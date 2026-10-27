// Camera-local frame shared by environment generation and sampling. The
// explicit polar fallback avoids normalizing a zero east vector.
fn sky_environment_frame() -> mat3x3<f32> {
    let up = normalize(-g.planet_center.xyz);
    var east = vec3<f32>(-up.y, up.x, 0.0);
    if dot(east, east) < 0.000000000001 {
        east = vec3<f32>(0.0, 1.0, 0.0);
    }
    east = normalize(east);
    return mat3x3<f32>(east, cross(up, east), up);
}

fn sky_environment_sign(v: vec2<f32>) -> vec2<f32> {
    return select(vec2<f32>(-1.0), vec2<f32>(1.0), v >= vec2<f32>(0.0));
}

fn sky_environment_encode(direction: vec3<f32>) -> vec2<f32> {
    // The analytic dome changes most quickly just above the horizon. Warp
    // elevation before octahedral mapping to reserve more texels there.
    let warped_z = sign(direction.z)
        * (sqrt(abs(direction.z) + 0.0025) - 0.05) / 0.95124922;
    let horizontal = length(direction.xy);
    var warped = vec3<f32>(0.0, 0.0, warped_z);
    if horizontal > 0.000001 {
        warped = vec3<f32>(direction.xy / horizontal * sqrt(max(1.0 - warped_z * warped_z, 0.0)), warped_z);
    }
    let unit = warped / (abs(warped.x) + abs(warped.y) + abs(warped.z));
    var p = unit.xy;
    if unit.z < 0.0 {
        p = (vec2<f32>(1.0) - abs(p.yx)) * sky_environment_sign(p);
    }
    return (p * 0.5 + 0.5) * (62.0 / 64.0) + (1.0 / 64.0);
}

fn sky_environment_decode(texel: vec2<u32>) -> vec3<f32> {
    var p = (vec2<f32>(texel) + 0.5 - 1.0) * (2.0 / 62.0) - 1.0;
    // Reflect the one-texel gutter across the octahedral edge. Clamping its
    // colors would leave an unrelated sample on the other side of the seam.
    if abs(p.x) > 1.0 {
        p = vec2<f32>(sign(p.x) * (2.0 - abs(p.x)), -p.y);
    }
    if abs(p.y) > 1.0 {
        p = vec2<f32>(-p.x, sign(p.y) * (2.0 - abs(p.y)));
    }
    var direction = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    if direction.z < 0.0 {
        direction = vec3<f32>((vec2<f32>(1.0) - abs(p.yx)) * sky_environment_sign(p), direction.z);
    }
    let warped = normalize(direction);
    let unwarped = abs(warped.z) * 0.95124922 + 0.05;
    let z = sign(warped.z) * (unwarped * unwarped - 0.0025);
    let horizontal = length(warped.xy);
    if horizontal < 0.000001 { return vec3<f32>(0.0, 0.0, z); }
    return vec3<f32>(warped.xy / horizontal * sqrt(max(1.0 - z * z, 0.0)), z);
}
