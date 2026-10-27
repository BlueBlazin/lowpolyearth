// WorldScalar is selected when assembling the opaque-world module.
// Positions retain f32 precision in both variants; only bounded appearance
// values may use half precision between vertex and fragment stages.
const WORLD_SKIP_UNUSED_ROOF_HEIGHT: bool = true;

struct WorldVsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<WorldScalar>,
    @location(1) base_normal: vec3<WorldScalar>,
    @location(2) ao: WorldScalar,
    @location(3) world_pos: vec3<f32>,
    @location(4) sample_pitch_m: f32,
    @location(5) local_pos: vec3<f32>,
    @location(6) @interpolate(flat) face_mat: u32,
    @location(7) construction_height: f32,
    @location(8) terrain_weights: vec4<WorldScalar>,
};

fn world_surface_output(source: VsOut) -> WorldVsOut {
    var out: WorldVsOut;
    out.clip = source.clip;
    out.color = vec3<WorldScalar>(source.color);
    out.base_normal = vec3<WorldScalar>(source.base_normal);
    out.ao = WorldScalar(source.ao);
    out.world_pos = source.world_pos;
    out.sample_pitch_m = source.sample_pitch_m;
    out.local_pos = source.local_pos;
    out.face_mat = source.face_mat;
    out.construction_height = source.construction_height;
    out.terrain_weights = vec4<WorldScalar>(source.terrain_weights);
    return out;
}

@vertex
fn vs_world(in: VsIn, @builtin(instance_index) chunk_index: u32) -> WorldVsOut {
    return world_surface_output(surface_vertex_with_height(
        in.pos, in.face_ao, in.rgba, chunk_index, true, !WORLD_SKIP_UNUSED_ROOF_HEIGHT,
    ));
}

@vertex
fn vs_world_packed(in: PackedVsIn, @builtin(instance_index) chunk_index: u32) -> WorldVsOut {
    return world_surface_output(surface_vertex_with_height(
        unpack_packed_position(in), unpack_packed_face(in), unpack_packed_rgba(in), chunk_index,
        true, !WORLD_SKIP_UNUSED_ROOF_HEIGHT,
    ));
}

@fragment
fn fs_world_opaque(in: WorldVsOut) -> @location(0) vec4<f32> {
    var surface: VsOut;
    surface.clip = in.clip;
    surface.color = vec3<f32>(in.color);
    surface.base_normal = vec3<f32>(in.base_normal);
    surface.normal = surface.base_normal;
    surface.ao = f32(in.ao);
    surface.world_pos = in.world_pos;
    surface.sample_pitch_m = in.sample_pitch_m;
    surface.local_pos = in.local_pos;
    surface.face_mat = in.face_mat;
    surface.construction_height = in.construction_height;
    surface.terrain_weights = vec4<f32>(in.terrain_weights);
    // The ordinary world vertex path supplies these constants. Stable opaque
    // chunks cannot enter the screen-door or tree transition fragment paths.
    surface.fade = 1.0;
    surface.emissive = 0.0;
    surface.vehicle_uv = vec2<f32>(0.0);
    surface.tree_transition = 0.0;
    surface.tree_lod_alpha = 1.0;
    return shade_surface(surface, vec2<f32>(0.0), vec2<f32>(0.0));
}
