use std::collections::BTreeMap;
use std::fmt;

use glam::Vec3;
use vearth_core::mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal};

const MAGIC: &[u8; 8] = b"VEARTHV1";
const VERSION: u16 = 1;
const VERTEX_BYTES: usize = 40;
const MAX_PARTS: usize = 32;
const MAX_VERTICES: usize = 1_000_000;
const MAX_INDICES: usize = 3_000_000;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct AssetError(&'static str);

impl fmt::Display for AssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for AssetError {}

pub(super) struct VehicleAsset {
    parts: BTreeMap<String, AssetPart>,
}

#[derive(Clone, Debug)]
pub(super) struct AssetPart {
    pub(super) mesh: MeshBuffers,
    pub(super) uvs: Vec<[f32; 2]>,
}

impl VehicleAsset {
    pub(super) fn take_part(&mut self, name: &str) -> Result<AssetPart, AssetError> {
        self.parts
            .remove(name)
            .ok_or(AssetError("vehicle asset is missing the requested part"))
    }

    pub(super) fn part_names(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8], AssetError> {
        if size > self.bytes.len() {
            return Err(AssetError("truncated vehicle asset"));
        }
        let (head, tail) = self.bytes.split_at(size);
        self.bytes = tail;
        Ok(head)
    }

    fn u16(&mut self) -> Result<u16, AssetError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, AssetError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn vec3(&mut self) -> Result<Vec3, AssetError> {
        let mut values = [0.0; 3];
        for value in &mut values {
            *value = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        }
        Ok(Vec3::from_array(values))
    }

    fn vec2(&mut self) -> Result<[f32; 2], AssetError> {
        let mut values = [0.0; 2];
        for value in &mut values {
            *value = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        }
        Ok(values)
    }
}

fn face(normal: Vec3) -> u32 {
    let magnitude = normal.abs();
    if magnitude.x >= magnitude.y && magnitude.x >= magnitude.z {
        u32::from(normal.x < 0.0)
    } else if magnitude.y >= magnitude.z {
        2 + u32::from(normal.y < 0.0)
    } else {
        4 + u32::from(normal.z < 0.0)
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<VehicleAsset, AssetError> {
    let mut reader = Reader { bytes };
    if reader.take(8)? != MAGIC || reader.u16()? != VERSION || reader.u16()? != 0 {
        return Err(AssetError("unsupported vehicle asset header"));
    }
    let part_count = reader.u32()? as usize;
    if !(1..=MAX_PARTS).contains(&part_count) {
        return Err(AssetError("invalid vehicle part count"));
    }
    let mut parts = BTreeMap::new();
    let mut total_vertices = 0;
    let mut total_indices = 0;
    for _ in 0..part_count {
        let name_length = reader.u16()? as usize;
        if !(1..=63).contains(&name_length) || reader.u16()? != 0 {
            return Err(AssetError("invalid vehicle part header"));
        }
        let vertex_count = reader.u32()? as usize;
        let index_count = reader.u32()? as usize;
        if vertex_count > MAX_VERTICES || index_count > MAX_INDICES {
            return Err(AssetError("vehicle mesh exceeds asset limits"));
        }
        total_vertices += vertex_count;
        total_indices += index_count;
        if vertex_count < 3
            || index_count < 3
            || index_count % 3 != 0
            || total_vertices > MAX_VERTICES
            || total_indices > MAX_INDICES
        {
            return Err(AssetError("invalid vehicle mesh counts"));
        }
        let name_bytes = reader.take(name_length)?;
        if !name_bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        {
            return Err(AssetError("invalid vehicle part name"));
        }
        let name = std::str::from_utf8(name_bytes).unwrap().to_owned();
        if parts.contains_key(&name) {
            return Err(AssetError("duplicate vehicle part name"));
        }
        // Check the complete payload before allocating from untrusted counts.
        if vertex_count * VERTEX_BYTES + index_count * 4 > reader.bytes.len() {
            return Err(AssetError("truncated vehicle mesh payload"));
        }
        let mut vertices = Vec::with_capacity(vertex_count);
        let mut uvs = Vec::with_capacity(vertex_count);
        let mut normals = Vec::with_capacity(vertex_count);
        for _ in 0..vertex_count {
            let position = reader.vec3()?;
            let normal = reader.vec3()?;
            let uv = reader.vec2()?;
            let appearance = reader.take(8)?;
            if !position.is_finite() || position.abs().max_element() > 256.0 {
                return Err(AssetError("invalid vehicle position in metres"));
            }
            if !normal.is_finite() || (normal.length_squared() - 1.0).abs() > 0.002 {
                return Err(AssetError("invalid vehicle corner normal"));
            }
            if !uv
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 10_000.0)
            {
                return Err(AssetError("invalid vehicle UV coordinate"));
            }
            let material = u32::from(appearance[3]);
            if !(96..=102).contains(&material) {
                return Err(AssetError("invalid vehicle material"));
            }
            let texture_layer = u32::from(appearance[4]);
            if texture_layer > 3 || appearance[5..] != [0, 0, 0] {
                return Err(AssetError(
                    "invalid vehicle texture layer or reserved bytes",
                ));
            }
            vertices.push(SurfaceVertex {
                pos: position.to_array(),
                face_ao: encode_surface_normal(
                    face(normal) | (3 << 3) | (material << 5) | (texture_layer << 13),
                    normal.to_array(),
                ),
                rgba: [appearance[0], appearance[1], appearance[2], 255],
            });
            uvs.push(uv);
            normals.push(normal);
        }
        let mut indices = Vec::with_capacity(index_count);
        for _ in 0..index_count {
            let index = reader.u32()?;
            if index as usize >= vertex_count {
                return Err(AssetError("vehicle index is out of bounds"));
            }
            indices.push(index);
        }
        for triangle in indices.chunks_exact(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|index| index as usize);
            let point = |index: usize| Vec3::from_array(vertices[index].pos);
            let cross = (point(b) - point(a)).cross(point(c) - point(a));
            if cross.length_squared() <= 1.0e-16 {
                return Err(AssetError("degenerate vehicle triangle"));
            }
            let geometric = cross.normalize();
            let material = (vertices[a].face_ao >> 5) & 255;
            let texture_layer = (vertices[a].face_ao >> 13) & 7;
            for index in [a, b, c] {
                if normals[index].dot(geometric) <= 0.001 {
                    return Err(AssetError("vehicle normal opposes triangle winding"));
                }
                if (vertices[index].face_ao >> 5) & 255 != material {
                    return Err(AssetError("vehicle triangle crosses material boundary"));
                }
                if (vertices[index].face_ao >> 13) & 7 != texture_layer {
                    return Err(AssetError("vehicle triangle crosses texture boundary"));
                }
            }
        }
        parts.insert(
            name,
            AssetPart {
                mesh: MeshBuffers { vertices, indices },
                uvs,
            },
        );
    }
    if !reader.bytes.is_empty() {
        return Err(AssetError("unexpected trailing vehicle asset data"));
    }
    Ok(VehicleAsset { parts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::mesh::decode_surface_normal;

    const VERTEX_START: usize = 32;
    const INDEX_START: usize = VERTEX_START + 3 * VERTEX_BYTES;

    fn triangle() -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(VERSION.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend(1_u32.to_le_bytes());
        bytes.extend(4_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend(3_u32.to_le_bytes());
        bytes.extend(3_u32.to_le_bytes());
        bytes.extend(b"body");
        for (position, normal) in [
            (Vec3::ZERO, Vec3::new(0.1, 0.0, 1.0).normalize()),
            (Vec3::X, Vec3::new(-0.1, 0.0, 1.0).normalize()),
            (Vec3::Y, Vec3::Z),
        ] {
            for value in position.to_array().into_iter().chain(normal.to_array()) {
                bytes.extend(value.to_le_bytes());
            }
            bytes.extend(0.25_f32.to_le_bytes());
            bytes.extend(0.75_f32.to_le_bytes());
            bytes.extend([240, 220, 180, 96, 2, 0, 0, 0]);
        }
        for index in 0_u32..3 {
            bytes.extend(index.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn retains_split_corner_normals_and_authored_finish() {
        let mut asset = decode(&triangle()).unwrap();
        assert_eq!(asset.part_names().collect::<Vec<_>>(), ["body"]);
        let part = asset.take_part("body").unwrap();
        let mesh = part.mesh;
        assert_eq!(part.uvs, [[0.25, 0.75]; 3]);
        assert!(asset.take_part("body").is_err());
        assert_eq!(mesh.indices, [0, 1, 2]);
        let normals: Vec<_> = mesh
            .vertices
            .iter()
            .map(|vertex| {
                assert_eq!(vertex.rgba, [240, 220, 180, 255]);
                assert_eq!((vertex.face_ao >> 5) & 255, 96);
                assert_eq!((vertex.face_ao >> 13) & 7, 2);
                Vec3::from_array(decode_surface_normal(vertex.face_ao).unwrap())
            })
            .collect();
        assert!(normals[0].x > 0.07 && normals[1].x < -0.07);
        assert!(normals.iter().all(|normal| normal.z > 0.99));
    }

    #[test]
    fn rejects_corrupt_and_unsupported_assets() {
        let valid = triangle();
        for length in 0..valid.len() {
            assert!(decode(&valid[..length]).is_err());
        }
        for (offset, value) in [
            (0, b'X'),
            (8, 2),
            (10, 1),
            (12, 0),
            (16, 0),
            (18, 1),
            (20, 2),
            (24, 4),
            (28, b'/'),
            (VERTEX_START + 35, 95),
            (VERTEX_START + 36, 4),
            (VERTEX_START + 37, 1),
            (INDEX_START, 3),
        ] {
            let mut bytes = valid.clone();
            bytes[offset] = value;
            assert!(decode(&bytes).is_err(), "accepted corrupt byte {offset}");
        }
        for (offset, value) in [
            (VERTEX_START, f32::NAN),
            (VERTEX_START, f32::INFINITY),
            (VERTEX_START, 257.0),
            (VERTEX_START + 12, 2.0),
            (VERTEX_START + 12, f32::NAN),
            (VERTEX_START + 24, f32::NAN),
            (VERTEX_START + 28, 10_001.0),
        ] {
            let mut bytes = valid.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode(&bytes).is_err());
        }
        let mut trailing = valid.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        let mut huge = valid;
        huge[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&huge).is_err());
    }

    #[test]
    fn rejects_degenerate_reversed_and_mixed_material_triangles() {
        let valid = triangle();
        let mut degenerate = valid.clone();
        degenerate[INDEX_START + 4..INDEX_START + 8].copy_from_slice(&0_u32.to_le_bytes());
        assert!(decode(&degenerate).is_err());
        let mut reversed = valid.clone();
        reversed[INDEX_START + 4..INDEX_START + 8].copy_from_slice(&2_u32.to_le_bytes());
        reversed[INDEX_START + 8..INDEX_START + 12].copy_from_slice(&1_u32.to_le_bytes());
        assert!(decode(&reversed).is_err());
        let mut mixed = valid.clone();
        mixed[VERTEX_START + VERTEX_BYTES + 35] = 97;
        assert!(decode(&mixed).is_err());
        let mut mixed_texture = valid.clone();
        mixed_texture[VERTEX_START + VERTEX_BYTES + 36] = 1;
        assert!(decode(&mixed_texture).is_err());
        let mut duplicate = valid.clone();
        duplicate[12..16].copy_from_slice(&2_u32.to_le_bytes());
        duplicate.extend_from_slice(&valid[16..]);
        assert!(decode(&duplicate).is_err());
    }
}
