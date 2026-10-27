// Depth-only directional Sun shadow rendering. Terrain vertices stay in their
// resident arena; the frame uploads only camera-relative model transforms.

struct ShadowPass {
    light_view_proj: mat4x4<f32>,
};

struct ShadowModel {
    model: mat4x4<f32>,
    landmark_model: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> shadow_pass: ShadowPass;
@group(1) @binding(0) var<storage, read> shadow_models: array<ShadowModel>;

struct FullVsIn {
    @location(0) pos: vec3<f32>,
    @location(1) face_ao: u32,
};

struct PackedVsIn {
    @location(0) pos_xy: u32,
    @location(1) pos_z_face: u32,
};

fn unpack_fixed_i16(bits: u32) -> f32 {
    return f32(bitcast<i32>(bits << 16u) >> 16) * (1.0 / 256.0);
}

fn unpack_fixed_i14(bits: u32) -> f32 {
    return f32(bitcast<i32>(bits << 18u) >> 18) * (1.0 / 64.0);
}

fn unpack_packed_position(in: PackedVsIn) -> vec3<f32> {
    if ((in.pos_z_face & 0x80000000u) != 0u) {
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
    if ((in.pos_z_face & 0x80000000u) != 0u) {
        // The shadow transform only consumes the landmark ownership tag;
        // normal bits live partly in alpha, which this pass does not load.
        return (in.pos_z_face >> 14u) & 0x1fffu;
    }
    return in.pos_z_face >> 16u;
}

fn projected_position(pos: vec3<f32>, face_ao: u32, model_index: u32) -> vec4<f32> {
    let model = shadow_models[model_index];
    let landmark_tag = 128u << 5u;
    var transform = model.model;
    if ((face_ao & landmark_tag) != 0u) {
        transform = model.landmark_model;
    }
    return shadow_pass.light_view_proj * transform * vec4<f32>(pos, 1.0);
}

@vertex
fn vs_full(in: FullVsIn, @builtin(instance_index) model_index: u32) -> @builtin(position) vec4<f32> {
    return projected_position(in.pos, in.face_ao, model_index);
}

@vertex
fn vs_packed(in: PackedVsIn, @builtin(instance_index) model_index: u32) -> @builtin(position) vec4<f32> {
    return projected_position(unpack_packed_position(in), unpack_packed_face(in), model_index);
}
