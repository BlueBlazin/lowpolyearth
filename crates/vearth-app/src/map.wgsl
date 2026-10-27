struct MapUniform {
    center_zoom_aspect: vec4<f32>,
    viewport_ppp: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> map: MapUniform;

fn project_world(world: vec2<f32>) -> vec2<f32> {
    let center = map.center_zoom_aspect.xy;
    let zoom = map.center_zoom_aspect.z;
    let aspect = map.center_zoom_aspect.w;

    return vec2<f32>(
        2.0 * (world.x - center.x) * zoom,
        -2.0 * (world.y - center.y) * zoom * aspect,
    );
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

struct ColorVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn fill_vs(
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @builtin(instance_index) instance_index: u32,
) -> ColorVertexOutput {
    let wrap = f32(instance_index) - 1.0;
    var output: ColorVertexOutput;
    output.position = vec4<f32>(project_world(position + vec2<f32>(wrap, 0.0)), 0.0, 1.0);
    output.color = vec4<f32>(srgb_to_linear(color.rgb), color.a);
    return output;
}

@vertex
fn line_vs(
    @location(0) start: vec2<f32>,
    @location(1) end: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) width_points: f32,
    @builtin(vertex_index) vertex_index: u32,
) -> ColorVertexOutput {
    let endpoint = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    let side = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let t = endpoint[vertex_index];

    let start_ndc = project_world(start);
    let end_ndc = project_world(end);
    let half_viewport_px = max(map.viewport_ppp.xy * 0.5, vec2<f32>(1.0));
    let segment_px = (end_ndc - start_ndc) * half_viewport_px;
    let direction = segment_px / max(length(segment_px), 0.0001);
    let normal = vec2<f32>(-direction.y, direction.x);
    let half_width_px = 0.5 * width_points * map.viewport_ppp.z;
    let extended = direction * ((2.0 * t - 1.0) * half_width_px);
    let offset_px = extended + normal * (side[vertex_index] * half_width_px);
    let position_ndc = mix(start_ndc, end_ndc, t) + offset_px / half_viewport_px;

    var output: ColorVertexOutput;
    output.position = vec4<f32>(position_ndc, 0.0, 1.0);
    output.color = vec4<f32>(srgb_to_linear(color.rgb), color.a);
    return output;
}

@fragment
fn color_fs(input: ColorVertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
