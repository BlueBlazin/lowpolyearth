//! Surface vertices, explicit normals, and lossless GPU packing.
//!
//! Positions use the owning render frame's local coordinates. This module
//! defines buffer contracts only; source generators provide indexed geometry.

use bytemuck::{Pod, Zeroable};

/// Face ids: 0:+X 1:-X 2:+Y 3:-Y 4:+Z 5:-Z (chunk-local axes).
/// [`TERRAIN_BLEND_FACE`] identifies smooth natural ground with explicit normals.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct SurfaceVertex {
    pub pos: [f32; 3],
    /// bits 0..=2: face id; bits 3..=4: AO level (0 darkest ..= 3 open);
    /// bits 5..=12: material id. On [`TERRAIN_BLEND_FACE`], the AO bits instead
    /// select a corner's physical family; the terrain surface is unoccluded.
    /// Bits 13..=14 carry material-scoped context: construction finish variant
    /// or water kind. Vehicle meshes own all three bits 13..=15 instead.
    pub face_ao: u32,
    /// RGB is authored tint. Terrain alpha encodes a renderer-owned macro
    /// lighting normal; [`TERRAIN_GEOMETRIC_NORMAL`] selects the geometric
    /// face normal. Authored/player meshes retain their alpha/emission ABI.
    pub rgba: [u8; 4],
}

/// Terrain-vertex alpha value that keeps the six-direction geometric normal.
/// Values whose nibbles are both below 15 encode signed macro slopes.
pub const TERRAIN_GEOMETRIC_NORMAL: u8 = u8::MAX;

/// Explicit object-space normal in the high bits of the face payload. The
/// low sixteen bits retain face, AO, material and landmark ownership. The
/// packed format retains the normal when the low fifteen payload bits and
/// its signed 1/64-grid position lanes can represent the vertex exactly.
pub const SURFACE_NORMAL_FLAG: u32 = 1 << 31;

/// Natural terrain top with an explicit normal. Bits 3..=4 carry granular,
/// vegetated, rock, or frozen response (0..=3), while the material id retains
/// the source column's semantic ownership. This fits the packed 12-byte ABI.
pub const TERRAIN_BLEND_FACE: u32 = 6;

/// Encode a unit direction using two seven-bit octahedral coordinates.
pub fn encode_surface_normal(face_ao: u32, normal: [f32; 3]) -> u32 {
    let n = glam::Vec3::from_array(normal).normalize_or_zero();
    let sum = n.abs().element_sum();
    let n = if sum > 0.0 { n / sum } else { glam::Vec3::Y };
    let mut p = n.truncate();
    if n.z < 0.0 {
        p = glam::Vec2::new(
            (1.0 - p.y.abs()) * if p.x >= 0.0 { 1.0 } else { -1.0 },
            (1.0 - p.x.abs()) * if p.y >= 0.0 { 1.0 } else { -1.0 },
        );
    }
    let q = ((p * 0.5 + glam::Vec2::splat(0.5)) * 127.0).round();
    (face_ao & 0xffff) | SURFACE_NORMAL_FLAG | (q.x as u32) << 16 | (q.y as u32) << 23
}

/// Decode an explicit normal, or return `None` for axis-aligned faces.
pub fn decode_surface_normal(face_ao: u32) -> Option<[f32; 3]> {
    if face_ao & SURFACE_NORMAL_FLAG == 0 {
        return None;
    }
    let p = glam::Vec2::new(
        ((face_ao >> 16) & 127) as f32,
        ((face_ao >> 23) & 127) as f32,
    ) * (2.0 / 127.0)
        - glam::Vec2::ONE;
    let mut n = glam::Vec3::new(p.x, p.y, 1.0 - p.x.abs() - p.y.abs());
    let t = (-n.z).max(0.0);
    n.x += if n.x >= 0.0 { -t } else { t };
    n.y += if n.y >= 0.0 { -t } else { t };
    Some(n.normalize().to_array())
}

impl SurfaceVertex {
    pub fn ao(&self) -> u32 {
        if self.face_ao & SURFACE_NORMAL_FLAG != 0 && self.face_ao & 7 == TERRAIN_BLEND_FACE {
            3
        } else {
            (self.face_ao >> 3) & 0b11
        }
    }
}

/// Lossless 12-byte GPU representation. Positions use signed 8.8 fixed
/// point, or signed fourteen-bit 1/64-grid coordinates with explicit normals.
/// Callers retain full floats whenever any payload or position cannot round
/// trip exactly; packing never quantizes geometry.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq, Eq)]
pub struct PackedSurfaceVertex {
    /// Ordinary mode: signed 8.8 X/Y halves. Normal mode: fourteen-bit X/Y,
    /// followed by the low four normal bits.
    pub pos_xy: u32,
    /// Ordinary mode: signed 8.8 Z and fifteen payload bits. Bit 31 selects
    /// normal mode: fourteen-bit Z, thirteen payload bits, four normal bits.
    pub pos_z_face: u32,
    /// Normal mode stores six normal bits and payload bits 13..=14 in alpha; decoded
    /// alpha is always [`TERRAIN_GEOMETRIC_NORMAL`].
    pub rgba: [u8; 4],
}

impl PackedSurfaceVertex {
    pub const POSITION_SCALE: f32 = 256.0;
    pub const SURFACE_POSITION_SCALE: f32 = 64.0;
    const NORMAL_MODE_FLAG: u32 = 1 << 31;

    pub fn try_from_full(vertex: SurfaceVertex) -> Option<Self> {
        fn encode(value: f32, scale: f32, min: i32, max: i32) -> Option<u32> {
            let scaled = value * scale;
            if !scaled.is_finite()
                || scaled.fract() != 0.0
                || scaled < min as f32
                || scaled > max as f32
            {
                return None;
            }
            let encoded = scaled as i32;
            let decoded = encoded as f32 / scale;
            (decoded == value).then_some(encoded as u32)
        }

        if vertex.face_ao & SURFACE_NORMAL_FLAG != 0 {
            const ALLOWED_PAYLOAD: u32 = SURFACE_NORMAL_FLAG | (0x3fff << 16) | 0x7fff;
            if vertex.face_ao & !ALLOWED_PAYLOAD != 0 || vertex.rgba[3] != TERRAIN_GEOMETRIC_NORMAL
            {
                return None;
            }
            let fixed = |value| {
                encode(value, Self::SURFACE_POSITION_SCALE, -8192, 8191)
                    .map(|encoded| encoded & 0x3fff)
            };
            let [x, y, z] = [
                fixed(vertex.pos[0])?,
                fixed(vertex.pos[1])?,
                fixed(vertex.pos[2])?,
            ];
            let normal = (vertex.face_ao >> 16) & 0x3fff;
            return Some(Self {
                pos_xy: x | (y << 14) | ((normal & 0xf) << 28),
                pos_z_face: z
                    | ((vertex.face_ao & 0x1fff) << 14)
                    | (((normal >> 4) & 0xf) << 27)
                    | Self::NORMAL_MODE_FLAG,
                rgba: [
                    vertex.rgba[0],
                    vertex.rgba[1],
                    vertex.rgba[2],
                    ((normal >> 8) | (((vertex.face_ao >> 13) & 3) << 6)) as u8,
                ],
            });
        }
        // Payload bit fifteen becomes the per-vertex format marker. Retain
        // full vertices for unsupported ownership bits rather than aliasing
        // them to an explicit-normal vertex.
        if vertex.face_ao & !0x7fff != 0 {
            return None;
        }
        let fixed = |value| {
            encode(
                value,
                Self::POSITION_SCALE,
                i16::MIN as i32,
                i16::MAX as i32,
            )
            .map(|encoded| encoded & 0xffff)
        };
        let [x, y, z] = [
            fixed(vertex.pos[0])?,
            fixed(vertex.pos[1])?,
            fixed(vertex.pos[2])?,
        ];
        Some(Self {
            pos_xy: x | (y << 16),
            pos_z_face: z | (vertex.face_ao << 16),
            rgba: vertex.rgba,
        })
    }

    pub fn to_full(self) -> SurfaceVertex {
        if self.pos_z_face & Self::NORMAL_MODE_FLAG != 0 {
            let decode =
                |value: u32| ((value << 18) as i32 >> 18) as f32 / Self::SURFACE_POSITION_SCALE;
            let normal = (self.pos_xy >> 28)
                | (((self.pos_z_face >> 27) & 0xf) << 4)
                | (u32::from(self.rgba[3] & 0x3f) << 8);
            return SurfaceVertex {
                pos: [
                    decode(self.pos_xy),
                    decode(self.pos_xy >> 14),
                    decode(self.pos_z_face),
                ],
                face_ao: ((self.pos_z_face >> 14) & 0x1fff)
                    | (u32::from(self.rgba[3] >> 6) << 13)
                    | (normal << 16)
                    | SURFACE_NORMAL_FLAG,
                rgba: [
                    self.rgba[0],
                    self.rgba[1],
                    self.rgba[2],
                    TERRAIN_GEOMETRIC_NORMAL,
                ],
            };
        }
        fn decode(value: u16) -> f32 {
            f32::from(value as i16) / PackedSurfaceVertex::POSITION_SCALE
        }
        SurfaceVertex {
            pos: [
                decode(self.pos_xy as u16),
                decode((self.pos_xy >> 16) as u16),
                decode(self.pos_z_face as u16),
            ],
            face_ao: self.pos_z_face >> 16,
            rgba: self.rgba,
        }
    }
}

pub fn pack_surface_vertices(vertices: &[SurfaceVertex]) -> Option<Vec<PackedSurfaceVertex>> {
    vertices
        .iter()
        .copied()
        .map(PackedSurfaceVertex::try_from_full)
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct MeshBuffers {
    pub vertices: Vec<SurfaceVertex>,
    pub indices: Vec<u32>,
}

/// GPU payload schema, shared with the renderer's packed vertex decoder.
pub const RENDER_SCHEMA_VERSION: u32 = 4;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surface_normals_preserve_payload_and_round_trip_directions() {
        use glam::Vec3;
        let payload = 2 | (3 << 3) | (150 << 5);
        for x in -8..=8 {
            for y in -8..=8 {
                for z in -8..=8 {
                    let n = Vec3::new(x as f32, y as f32, z as f32).normalize_or_zero();
                    if n == Vec3::ZERO {
                        continue;
                    }
                    let encoded = super::encode_surface_normal(payload, n.to_array());
                    assert_eq!(encoded & 0xffff, payload);
                    let decoded = Vec3::from_array(super::decode_surface_normal(encoded).unwrap());
                    assert!(n.dot(decoded) > 0.999, "{n:?} -> {decoded:?}");
                }
            }
        }
        assert!(super::decode_surface_normal(payload).is_none());
    }
    #[test]
    fn packed_terrain_vertices_are_12_bytes_and_round_trip_exactly() {
        assert_eq!(std::mem::size_of::<PackedSurfaceVertex>(), 12);
        for position in [
            [-6.0, 0.0, 32.0],
            [-0.75, 32.75, 16.0],
            [0.0, -6.0, 0.75],
            [32.0, 32.0, 32.0],
        ] {
            let full = SurfaceVertex {
                pos: position,
                face_ao: 5 | (3 << 3) | (255 << 5),
                rgba: [1, 127, 255, 255],
            };
            let packed = PackedSurfaceVertex::try_from_full(full).unwrap();
            assert_eq!(packed.to_full(), full);
        }
    }

    #[test]
    fn packed_surface_normals_round_trip_every_encoded_direction_and_material_tag() {
        for normal in 0..=0x3fff {
            let full = SurfaceVertex {
                pos: [-0.75, 32.015625, -128.0],
                face_ao: SURFACE_NORMAL_FLAG | (normal << 16) | 5 | (3 << 3) | (255 << 5),
                rgba: [17, 127, 243, TERRAIN_GEOMETRIC_NORMAL],
            };
            let packed = PackedSurfaceVertex::try_from_full(full).unwrap();
            assert_ne!(packed.pos_z_face & PackedSurfaceVertex::NORMAL_MODE_FLAG, 0);
            assert_eq!(packed.to_full(), full);
            assert_eq!(packed.rgba[3], (normal >> 8) as u8);
        }
    }

    #[test]
    fn packed_terrain_blends_keep_semantic_owner_normal_and_family() {
        for material in [2, 3, 4, 5, 66, 67, 68, 69, 70] {
            for family in 0..4 {
                let vertex = SurfaceVertex {
                    pos: [32.0, -12.125, 0.0],
                    face_ao: encode_surface_normal(
                        TERRAIN_BLEND_FACE | family << 3 | material << 5,
                        [-0.25, 1.0, 0.4],
                    ),
                    rgba: [83, 117, 54, TERRAIN_GEOMETRIC_NORMAL],
                };
                let decoded = PackedSurfaceVertex::try_from_full(vertex)
                    .unwrap()
                    .to_full();
                assert_eq!(decoded, vertex);
                assert_eq!(decoded.ao(), 3);
                assert_eq!((decoded.face_ao >> 3) & 3, family);
                assert_eq!((decoded.face_ao >> 5) & 255, material);
            }
        }
    }

    #[test]
    fn packed_modes_share_one_buffer_without_changing_ordinary_vertex_bytes() {
        let ordinary = SurfaceVertex {
            pos: [-1.0 / 256.0, 32.75, -6.0],
            face_ao: 0x7fff,
            rgba: [1, 2, 3, 4],
        };
        let explicit = SurfaceVertex {
            pos: [-0.75, 32.75, -6.0],
            face_ao: encode_surface_normal(2 | (3 << 3) | (204 << 5), [0.3, 0.9, -0.1]),
            rgba: [4, 5, 6, TERRAIN_GEOMETRIC_NORMAL],
        };
        let packed = pack_surface_vertices(&[ordinary, explicit]).unwrap();
        assert_eq!(packed[0].pos_xy, 0x20c0_ffff);
        assert_eq!(packed[0].pos_z_face, 0x7fff_fa00);
        assert_eq!(packed[0].rgba, ordinary.rgba);
        assert_eq!(packed[0].to_full(), ordinary);
        assert_eq!(packed[1].to_full(), explicit);
        assert_eq!(packed[1].to_full().face_ao & (128 << 5), 128 << 5);
    }

    #[test]
    fn packed_surface_context_preserves_normals_tint_and_ownership() {
        assert_eq!(std::mem::size_of::<PackedSurfaceVertex>(), 12);
        for material in [1, 82, 84, 85, 87, 88, 90, 210] {
            for variant in 0..4 {
                for normal in [[0.0, 1.0, 0.0], [-0.31, 0.51, -0.79], [0.91, -0.2, 0.1]] {
                    let original = SurfaceVertex {
                        pos: [-127.0, 4.0, 127.984375],
                        face_ao: encode_surface_normal(
                            2 | (3 << 3) | (material << 5) | (variant << 13),
                            normal,
                        ),
                        rgba: [31, 107, 201, TERRAIN_GEOMETRIC_NORMAL],
                    };
                    let packed = PackedSurfaceVertex::try_from_full(original).unwrap();
                    assert_eq!(packed.to_full(), original);
                    assert_eq!(u32::from(packed.rgba[3] >> 6), variant);
                    assert_eq!(
                        decode_surface_normal(packed.to_full().face_ao),
                        decode_surface_normal(original.face_ao)
                    );
                }
            }
        }
    }

    #[test]
    fn packed_surface_positions_are_exact_at_signed_fourteen_bit_limits() {
        let base = SurfaceVertex {
            pos: [0.0; 3],
            face_ao: encode_surface_normal(2, [0.0, 1.0, 0.0]),
            rgba: [7, 8, 9, TERRAIN_GEOMETRIC_NORMAL],
        };
        for axis in 0..3 {
            for coordinate in [-128.0, -1.0 / 64.0, 0.0, 1.0 / 64.0, 127.984375] {
                let mut full = base;
                full.pos[axis] = coordinate;
                let packed = PackedSurfaceVertex::try_from_full(full).unwrap();
                assert_eq!(packed.to_full(), full);
            }
            for coordinate in [
                -128.015625,
                128.0,
                127.99609375,
                1.0 / 256.0,
                1.0 / 3.0,
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ] {
                let mut full = base;
                full.pos[axis] = coordinate;
                assert!(PackedSurfaceVertex::try_from_full(full).is_none());
            }
        }
    }

    #[test]
    fn packed_surface_mode_rejects_unsupported_payload_and_alpha() {
        let base = SurfaceVertex {
            pos: [1.0, 2.0, 3.0],
            face_ao: encode_surface_normal(2 | (3 << 3) | (255 << 5), [0.3, 0.4, 0.5]),
            rgba: [7, 8, 9, TERRAIN_GEOMETRIC_NORMAL],
        };
        for bit in [15, 30] {
            assert!(
                PackedSurfaceVertex::try_from_full(SurfaceVertex {
                    face_ao: base.face_ao | (1 << bit),
                    ..base
                })
                .is_none()
            );
        }
        for alpha in [0, 1, 127, 128, 254] {
            assert!(
                PackedSurfaceVertex::try_from_full(SurfaceVertex {
                    rgba: [7, 8, 9, alpha],
                    ..base
                })
                .is_none()
            );
        }
        let marker_collision = SurfaceVertex {
            face_ao: 1 << 15,
            ..base
        };
        assert!(PackedSurfaceVertex::try_from_full(marker_collision).is_none());
        assert!(pack_surface_vertices(&[base, marker_collision]).is_none());
    }

    #[test]
    fn compact_vertex_refuses_any_loss_and_preserves_full_float_fallback() {
        let fractional = SurfaceVertex {
            pos: [1.0 / 3.0, 2.0, 3.0],
            face_ao: 2,
            rgba: [4, 5, 6, 255],
        };
        let out_of_range = SurfaceVertex {
            pos: [128.0, 0.0, 0.0],
            ..fractional
        };
        let oversized_payload = SurfaceVertex {
            pos: [1.0, 2.0, 3.0],
            face_ao: u32::MAX,
            ..fractional
        };
        assert!(PackedSurfaceVertex::try_from_full(fractional).is_none());
        assert!(PackedSurfaceVertex::try_from_full(out_of_range).is_none());
        assert!(PackedSurfaceVertex::try_from_full(oversized_payload).is_none());
        assert!(pack_surface_vertices(&[fractional]).is_none());
    }
}
