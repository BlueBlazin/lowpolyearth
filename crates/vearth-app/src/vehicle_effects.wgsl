// Prefix of the shared scene uniform. Effect positions are rebased on CPU.
struct Globals {
    view_proj: mat4x4<f32>, sun_dir: vec4<f32>, moon_dir: vec4<f32>,
    ambient: vec4<f32>, inv_view_proj: mat4x4<f32>,
    planet_center: vec4<f32>, viewport: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var scene_depth: texture_depth_2d;
struct Input {
    @location(0) start_radius: vec4<f32>,
    @location(1) end_radius: vec4<f32>,
    @location(2) color_energy: vec4<f32>,
    @location(3) shape: vec4<f32>,
};
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) @interpolate(flat) color_energy: vec4<f32>,
    @location(3) @interpolate(flat) shape: vec4<f32>,
    @location(4) @interpolate(flat) end_on: f32,
};
@vertex
fn vs_main(input: Input, @builtin(vertex_index) index: u32) -> Output {
    let corners = array<vec2<f32>,6>(vec2<f32>(-1,0),vec2<f32>(1,0),vec2<f32>(1,1),vec2<f32>(-1,0),vec2<f32>(1,1),vec2<f32>(-1,1));
    let uv = corners[index];
    let center = (input.start_radius.xyz + input.end_radius.xyz) * 0.5;
    let segment = input.end_radius.xyz - input.start_radius.xyz;
    let camera_right = normalize(g.inv_view_proj[0].xyz);
    let camera_up = normalize(g.inv_view_proj[1].xyz);
    var toward_camera = normalize(cross(camera_right, camera_up));
    if length(center) > 0.0001 { toward_camera = -normalize(center); }
    let projected = segment - toward_camera * dot(segment, toward_camera);
    let projected_length = length(projected);
    var along = normalize(camera_up - toward_camera * dot(camera_up, toward_camera));
    if projected_length > 0.00001 { along = projected / projected_length; }
    let across = normalize(cross(toward_camera, along));
    // A ribbon alone has zero projected area when viewed along its axis.
    // Expand its end-on footprint into a round cross section, retaining the
    // actual segment depth for intersection fading against the craft.
    let end_on = 1.0 - smoothstep(input.start_radius.w * 0.25,
        input.start_radius.w * 2.0, projected_length);
    let width = mix(mix(input.start_radius.w, input.end_radius.w, uv.y),
        input.start_radius.w, end_on);
    var world = mix(input.start_radius.xyz, input.end_radius.xyz, uv.y)
        + across * uv.x * width
        + along * (uv.y * 2.0 - 1.0) * input.start_radius.w * end_on;
    if input.shape.x == 1.0 {
        world = center + (camera_right * uv.x + camera_up * (uv.y * 2.0 - 1.0)) * input.start_radius.w;
    }
    var out: Output;
    out.position = g.view_proj * vec4<f32>(world,1);
    out.uv = uv; out.world = world;
    out.color_energy = input.color_energy; out.shape = input.shape;
    out.end_on = end_on;
    return out;
}
@fragment
fn fs_main(input: Output) -> @location(0) vec4<f32> {
    let pixel = clamp(vec2<i32>(input.position.xy), vec2<i32>(0), vec2<i32>(textureDimensions(scene_depth))-1);
    let depth = textureLoad(scene_depth,pixel,0);
    var intersection_fade = 1.0;
    if depth > 0.0 {
        let ndc = vec2<f32>(input.position.x/g.viewport.x*2.0-1.0,1.0-input.position.y/g.viewport.y*2.0);
        let surface = g.inv_view_proj * vec4<f32>(ndc,depth,1);
        let separation = length(surface.xyz / surface.w) - length(input.world);
        intersection_fade = smoothstep(0.0,max(input.shape.z * 0.8,0.025),separation);
    }
    let x = input.uv.x;
    let age = input.uv.y;
    let edge = pow(max(1.0-x*x,0.0),2.2);
    var shape = edge * (1.0-smoothstep(0.15,1.0,age));
    let diamonds = 0.83 + 0.17 * cos(age * 30.0 - input.shape.y);
    shape *= diamonds;
    if input.shape.x == 1.0 {
        let r2 = x*x + pow(age*2.0-1.0,2.0);
        shape = pow(max(1.0-r2,0.0),3.0);
    } else if input.shape.x == 2.0 {
        shape = edge * sin(age * 3.14159265);
    }
    if input.shape.x != 1.0 {
        let r2 = x*x + pow(age*2.0-1.0,2.0);
        let cross_section = pow(max(1.0-r2,0.0),2.2);
        shape = mix(shape, cross_section, input.end_on);
    }
    let energy = shape * intersection_fade * input.color_energy.w;
    return vec4<f32>(input.color_energy.rgb * energy,0.0);
}
