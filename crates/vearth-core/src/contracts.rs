//! Stable semantic fingerprints for surface geometry and GPU payloads.
//!
//! These encodings are deliberately independent of Rust enum layout, pointer
//! identity, allocation order, and host endianness. Changing an encoding is a
//! contract/schema change and must be accompanied by an intentional fixture
//! update.

use crate::mesh::{MeshBuffers, PackedSurfaceVertex, SurfaceVertex};
use std::fmt;

/// Version of the canonical encodings in this module.
pub const CONTRACT_SCHEMA_VERSION: u32 = 2;

/// A compact deterministic digest. Two independently seeded FNV-1a lanes make
/// accidental fixture collisions far less likely than a single diagnostic
/// checksum while keeping the contract dependency-free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(pub [u64; 2]);

impl Fingerprint {
    pub fn of_bytes(domain: &str, bytes: &[u8]) -> Self {
        let mut hasher = SemanticHasher::new(domain);
        hasher.bytes(bytes);
        hasher.finish()
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}{:016x}", self.0[0], self.0[1])
    }
}

/// Explicit little-endian semantic encoder.
pub struct SemanticHasher {
    lanes: [u64; 2],
}

impl SemanticHasher {
    pub fn new(domain: &str) -> Self {
        let mut hasher = Self {
            lanes: [0xcbf2_9ce4_8422_2325, 0x8422_2325_cbf2_9ce4],
        };
        hasher.u32(CONTRACT_SCHEMA_VERSION);
        hasher.bytes(domain.as_bytes());
        hasher
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.raw_bytes(&(value.len() as u64).to_le_bytes());
        self.raw_bytes(value);
    }

    fn raw_bytes(&mut self, value: &[u8]) {
        for &byte in value {
            self.lanes[0] = (self.lanes[0] ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
            self.lanes[1] = (self.lanes[1] ^ u64::from(byte).wrapping_add(0x9d))
                .wrapping_mul(0x0000_0100_0000_01e7);
        }
    }

    pub fn bool(&mut self, value: bool) {
        self.bytes(&[u8::from(value)]);
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    pub fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }

    pub fn f64(&mut self, value: f64) {
        self.u64(value.to_bits());
    }

    pub fn fingerprint(&mut self, value: Fingerprint) {
        self.u64(value.0[0]);
        self.u64(value.0[1]);
    }

    pub fn finish(self) -> Fingerprint {
        Fingerprint(self.lanes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalVertex {
    pos: [u32; 3],
    face_ao: u32,
    rgba: [u8; 4],
}

impl From<SurfaceVertex> for CanonicalVertex {
    fn from(vertex: SurfaceVertex) -> Self {
        Self {
            pos: vertex.pos.map(f32::to_bits),
            face_ao: vertex.face_ao,
            rgba: vertex.rgba,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalTriangle {
    /// Dominant signed geometric normal: +X, -X, +Y, -Y, +Z, -Z.
    geometric_face: u8,
    vertices: [CanonicalVertex; 3],
}

fn canonical_triangle(vertices: [SurfaceVertex; 3]) -> CanonicalTriangle {
    let edge0 = [
        vertices[1].pos[0] - vertices[0].pos[0],
        vertices[1].pos[1] - vertices[0].pos[1],
        vertices[1].pos[2] - vertices[0].pos[2],
    ];
    let edge1 = [
        vertices[2].pos[0] - vertices[0].pos[0],
        vertices[2].pos[1] - vertices[0].pos[1],
        vertices[2].pos[2] - vertices[0].pos[2],
    ];
    let normal = [
        edge0[1] * edge1[2] - edge0[2] * edge1[1],
        edge0[2] * edge1[0] - edge0[0] * edge1[2],
        edge0[0] * edge1[1] - edge0[1] * edge1[0],
    ];
    let (axis, _) = normal
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
        .expect("a triangle has three normal components");
    let geometric_face = (axis * 2 + usize::from(normal[axis] < 0.0)) as u8;
    let raw = vertices.map(CanonicalVertex::from);
    // Preserve winding while removing the arbitrary choice of first vertex.
    let rotations = [raw, [raw[1], raw[2], raw[0]], [raw[2], raw[0], raw[1]]];
    CanonicalTriangle {
        geometric_face,
        vertices: *rotations.iter().min().unwrap(),
    }
}

/// Mesh semantics canonicalized as a sorted multiset of oriented triangles.
/// Sorting removes allocator/emission order while retaining winding, geometric
/// and shading faces, material/tint, vertex payload, and exact bounds.
pub fn mesh_semantic_fingerprint(mesh: &MeshBuffers) -> Fingerprint {
    let mut triangles = Vec::with_capacity(mesh.indices.len() / 3);
    for indices in mesh.indices.chunks_exact(3) {
        let vertices = [
            mesh.vertices[indices[0] as usize],
            mesh.vertices[indices[1] as usize],
            mesh.vertices[indices[2] as usize],
        ];
        triangles.push(canonical_triangle(vertices));
    }
    triangles.sort_unstable();

    let mut hasher = SemanticHasher::new("vearth.mesh-semantic.triangles.v2");
    hasher.u64(mesh.vertices.len() as u64);
    hasher.u64(mesh.indices.len() as u64);
    hasher.u64(triangles.len() as u64);
    let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
    for vertex in &mesh.vertices {
        for (axis, position) in vertex.pos.into_iter().enumerate() {
            bounds[0][axis] = bounds[0][axis].min(position);
            bounds[1][axis] = bounds[1][axis].max(position);
        }
    }
    for bound in bounds.into_iter().flatten() {
        hasher.f32(bound);
    }
    for triangle in triangles {
        hasher.u8(triangle.geometric_face);
        for vertex in triangle.vertices {
            for position in vertex.pos {
                hasher.u32(position);
            }
            hasher.u32(vertex.face_ao);
            hasher.bytes(&vertex.rgba);
        }
    }
    hasher.finish()
}

/// Exact packed-render byte contract. Both vertex and index layout are tagged;
/// callers must choose the tag that matches the bytes supplied.
pub fn packed_render_fingerprint(
    vertices: &[PackedSurfaceVertex],
    indices_u16: Option<&[u16]>,
    indices_u32: Option<&[u32]>,
) -> Fingerprint {
    assert!(indices_u16.is_some() ^ indices_u32.is_some());
    let mut hasher = SemanticHasher::new("vearth.packed-render.v1");
    hasher.u64(vertices.len() as u64);
    for vertex in vertices {
        hasher.u32(vertex.pos_xy);
        hasher.u32(vertex.pos_z_face);
        hasher.bytes(&vertex.rgba);
    }
    if let Some(indices) = indices_u16 {
        hasher.u8(16);
        hasher.u64(indices.len() as u64);
        for &index in indices {
            hasher.bytes(&index.to_le_bytes());
        }
    } else if let Some(indices) = indices_u32 {
        hasher.u8(32);
        hasher.u64(indices.len() as u64);
        for &index in indices {
            hasher.u32(index);
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::pack_surface_vertices;
    fn mesh() -> MeshBuffers {
        MeshBuffers {
            vertices: [[0., 0., 0.], [2., 0., 0.], [2., 3., 0.], [0., 3., 0.]]
                .map(|pos| SurfaceVertex {
                    pos,
                    face_ao: 2 | (3 << 3) | (7 << 5),
                    rgba: [12, 34, 56, 255],
                })
                .to_vec(),
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }
    #[test]
    fn mesh_hash_ignores_triangle_order_and_start_but_preserves_winding() {
        let mesh = mesh();
        let mut reordered = mesh.clone();
        reordered.indices = vec![2, 3, 0, 1, 2, 0];
        assert_eq!(
            mesh_semantic_fingerprint(&mesh),
            mesh_semantic_fingerprint(&reordered)
        );
        reordered.indices.swap(0, 1);
        assert_ne!(
            mesh_semantic_fingerprint(&mesh),
            mesh_semantic_fingerprint(&reordered)
        );
    }
    #[test]
    fn mesh_hash_detects_geometry_and_material_payload() {
        let mesh = mesh();
        let mut changed = mesh.clone();
        changed.vertices[0].pos[0] = 0.001;
        assert_ne!(
            mesh_semantic_fingerprint(&mesh),
            mesh_semantic_fingerprint(&changed)
        );
        let mut changed = mesh.clone();
        changed.vertices[0].rgba[0] ^= 1;
        assert_ne!(
            mesh_semantic_fingerprint(&mesh),
            mesh_semantic_fingerprint(&changed)
        );
    }
    #[test]
    fn packed_hash_detects_layout_and_payload_changes() {
        let mesh = mesh();
        let packed = pack_surface_vertices(&mesh.vertices).unwrap();
        let a = packed_render_fingerprint(&packed, None, Some(&mesh.indices));
        let u16_indices = mesh.indices.iter().map(|&i| i as u16).collect::<Vec<_>>();
        assert_ne!(
            a,
            packed_render_fingerprint(&packed, Some(&u16_indices), None)
        );
        let mut changed = packed;
        changed[0].rgba[0] ^= 1;
        assert_ne!(
            a,
            packed_render_fingerprint(&changed, None, Some(&mesh.indices))
        );
    }
}
