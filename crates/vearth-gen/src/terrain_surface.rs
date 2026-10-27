//! Direct terrain geometry and collision from shared geographic height samples.

use crate::{SurfaceAppearance, TerrainSurfaceData, material};
use glam::{DVec3, Vec3};
use std::cell::RefCell;
use std::collections::HashMap;
use vearth_core::cell::{CellId, ChunkAddr};
use vearth_core::chunk::CHUNK_DIM as N;
use vearth_core::frame::ChunkFrame;
use vearth_core::mesh::{
    MeshBuffers, SURFACE_NORMAL_FLAG, SurfaceVertex, TERRAIN_BLEND_FACE, decode_surface_normal,
    encode_surface_normal,
};

#[path = "terrain_water.rs"]
mod water_bed;

const EDGE: usize = N + 1;
const SMOOTH_CREASE_COS: f32 = 0.70710677;

/// Source-derived appearance at a shared terrain corner. This sample stays in
/// generated cell data; rendered vertices retain the packed world ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainCornerShading {
    /// Fourteen-bit octahedral normal in the cell's unscaled local frame.
    pub normal_oct: u16,
    pub material: u8,
    /// Natural corners already include their terrain pigment.
    pub tint: [u8; 3],
    /// Linear-light pigment ratio, 128 = neutral, for surfaces whose colour is
    /// owned per column (cultivated fields).
    pub pigment: [u8; 3],
}

impl TerrainCornerShading {
    pub fn new(normal: [f32; 3], material: u8, tint: [u8; 3]) -> Self {
        Self {
            normal_oct: ((encode_surface_normal(0, normal) >> 16) & 0x3fff) as u16,
            material,
            tint,
            pigment: crate::terrain_pigment::NEUTRAL,
        }
    }

    fn normal(self) -> Vec3 {
        Vec3::from_array(
            decode_surface_normal(SURFACE_NORMAL_FLAG | (u32::from(self.normal_oct) << 16))
                .unwrap(),
        )
    }
}

struct RootOwner {
    frame: ChunkFrame,
    corners: HashMap<usize, (f32, f32)>,
    landmarks: Vec<crate::landmarks::ActiveLandmark>,
}

/// Generation-only root sampler. Owner and halo use the same source terrain
/// plane; the current cell reuses its already fetched geographic corners.
pub(crate) struct SourceTreeGround<'a> {
    frame: ChunkFrame,
    corners: &'a [f32],
    margin: usize,
    eligible: [u32; N],
    elevation: &'a dyn vearth_data::ElevationSource,
    owners: RefCell<HashMap<CellId, RootOwner>>,
}

impl<'a> SourceTreeGround<'a> {
    pub(crate) fn new(
        frame: ChunkFrame,
        corners: &'a [f32],
        margin: usize,
        eligible: [u32; N],
        elevation: &'a dyn vearth_data::ElevationSource,
    ) -> Self {
        Self {
            frame,
            corners,
            margin,
            eligible,
            elevation,
            owners: RefCell::new(HashMap::new()),
        }
    }

    fn corner(&self, owner: &mut RootOwner, x: usize, z: usize) -> (f32, f32) {
        *owner.corners.entry(z * EDGE + x).or_insert_with(|| {
            let source = if owner.frame.addr.cell == self.frame.addr.cell {
                let edge = EDGE + self.margin * 2;
                self.corners[(z + self.margin) * edge + x + self.margin]
            } else {
                let (lat, lon) = owner
                    .frame
                    .column_latlon(x as f64 / N as f64, z as f64 / N as f64);
                self.elevation.elevation_m(
                    lat,
                    lon,
                    owner.frame.grid_scale.y * crate::SAMPLE_ZOOM_FACTOR,
                )
            };
            let (_, offset) = owner
                .frame
                .column_surface(x as f64 / N as f64, z as f64 / N as f64);
            let local = ((f64::from(source) + offset) / owner.frame.grid_scale.y) as f32;
            (source, local)
        })
    }

    pub(crate) fn elevation_at(&self, lat: f64, lon: f64) -> Option<f32> {
        let surface = vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
        if !surface.is_finite() {
            return None;
        }
        let cell = CellId::from_dir(surface, self.frame.addr.cell.level);
        let mut owners = self.owners.borrow_mut();
        let owner = owners.entry(cell).or_insert_with(|| {
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            RootOwner {
                frame,
                corners: HashMap::new(),
                // Only horizontal ownership is queried. Resolving its frame
                // does not require fetching the landmark's terrain-base DEM.
                landmarks: crate::landmarks::active_for(cell, &frame, &|_, _| 0.0),
            }
        });
        let origin = owner.frame.ecef_to_local(surface);
        let up = vearth_core::ellipsoid::geodetic_normal(surface);
        let direction = owner.frame.ecef_to_local(surface + up) - origin;
        let mut altitude = 0.0;
        for _ in 0..4 {
            let p = origin + direction * altitude;
            if !(0.0..=N as f64).contains(&p.x) || !(0.0..=N as f64).contains(&p.z) {
                return None;
            }
            let x = (p.x.floor() as usize).min(N - 1);
            let z = (p.z.floor() as usize).min(N - 1);
            if cell == self.frame.addr.cell && self.eligible[z] & (1 << x) == 0 {
                return None;
            }
            let centre = owner
                .frame
                .local_to_ecef(DVec3::new(x as f64 + 0.5, 0.0, z as f64 + 0.5));
            if owner
                .landmarks
                .iter()
                .any(|landmark| landmark.excludes_source_at(centre, owner.frame.grid_scale.x * 0.5))
            {
                return None;
            }
            let samples = [(x, z), (x + 1, z), (x, z + 1), (x + 1, z + 1)]
                .map(|(x, z)| self.corner(owner, x, z));
            // Below-sea source columns can be carved or explicitly restored
            // by coast ownership. Owner and halo both retain that fallback.
            if samples.iter().map(|sample| sample.0).sum::<f32>() <= 0.0 {
                return None;
            }
            let [h00, h10, h01, h11] = samples.map(|sample| f64::from(sample.1));
            if [h00, h10, h01, h11].iter().any(|h| !h.is_finite()) {
                return None;
            }
            let (a, b) = if p.z - z as f64 <= p.x - x as f64 {
                (h10 - h00, h11 - h10)
            } else {
                (h11 - h01, h01 - h00)
            };
            let c = h00 - a * x as f64 - b * z as f64;
            let denominator = direction.y - a * direction.x - b * direction.z;
            if denominator <= 1.0e-12 {
                return None;
            }
            let next = (c + a * origin.x + b * origin.z - origin.y) / denominator;
            if !next.is_finite() {
                return None;
            }
            if (next - altitude).abs() < 1.0e-5 {
                return Some(next as f32);
            }
            altitude = next;
        }
        None
    }
}

/// Shared piecewise-planar terrain; every height is a rendered vertex value.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainCollision {
    pub heights_local: Box<[f32; EDGE * EDGE]>,
    pub source_columns: [u32; N],
}

impl TerrainCollision {
    pub fn sample_local(&self, x: f64, z: f64) -> Option<f64> {
        if !x.is_finite()
            || !z.is_finite()
            || !(0.0..=N as f64).contains(&x)
            || !(0.0..=N as f64).contains(&z)
        {
            return None;
        }
        let cx = (x.floor() as usize).min(N - 1);
        let cz = (z.floor() as usize).min(N - 1);
        if self.source_columns[cz] & (1 << cx) == 0 {
            return None;
        }
        let fx = x - cx as f64;
        let fz = z - cz as f64;
        let a = f64::from(self.heights_local[cz * EDGE + cx]);
        let b = f64::from(self.heights_local[cz * EDGE + cx + 1]);
        let c = f64::from(self.heights_local[(cz + 1) * EDGE + cx]);
        let d = f64::from(self.heights_local[(cz + 1) * EDGE + cx + 1]);
        Some(if fz <= fx {
            a + (b - a) * fx + (d - b) * fz
        } else {
            a + (d - c) * fx + (c - a) * fz
        })
    }
    pub fn heap_bytes(&self) -> usize {
        std::mem::size_of_val(self.heights_local.as_ref())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.heights_local.iter().any(|h| !h.is_finite()) {
            Err("terrain has a non-finite corner".into())
        } else {
            Ok(())
        }
    }
}

pub(crate) struct TerrainGeometry {
    pub mesh: MeshBuffers,
    pub collision: Option<TerrainCollision>,
    pub triangles: Vec<crate::collision::CollisionTriangle>,
}

#[cfg(test)]
pub(crate) fn build(
    frame: &ChunkFrame,
    source: &TerrainSurfaceData,
    appearance: &[[SurfaceAppearance; N]; N],
) -> TerrainGeometry {
    build_with_water(frame, source, appearance, &[])
}

pub(crate) fn build_with_water(
    frame: &ChunkFrame,
    source: &TerrainSurfaceData,
    appearance: &[[SurfaceAppearance; N]; N],
    water: &[crate::water_surface::WaterTriangle],
) -> TerrainGeometry {
    let exact_globe = frame.addr.cell.level < 11;
    let mut projected = exact_globe.then(|| Box::new([Vec3::ZERO; EDGE * EDGE]));
    let heights_local = Box::new(std::array::from_fn(|i| {
        let (x, z) = (i % EDGE, i / EDGE);
        let (surface, offset) = frame.column_surface(x as f64 / N as f64, z as f64 / N as f64);
        if let Some(projected) = &mut projected {
            let normal = vearth_core::ellipsoid::geodetic_normal(surface);
            projected[i] = frame
                .ecef_to_local(surface + normal * f64::from(source.heights_m[i]))
                .as_vec3();
        }
        ((f64::from(source.heights_m[i]) + offset) / frame.grid_scale.y) as f32
    }));
    let mut collision = TerrainCollision {
        heights_local,
        source_columns: source.source_columns,
    };
    let full_grid = collision.source_columns.iter().all(|&row| row == u32::MAX)
        && collision
            .heights_local
            .iter()
            .all(|height| height.is_finite());
    // Corner normals are shared by up to six emitted vertices. Keep the
    // decode/re-encode arithmetic, including octahedral boundary rounding.
    let shading_normals = source
        .shading
        .as_ref()
        .filter(|_| {
            full_grid
                && appearance
                    .iter()
                    .flatten()
                    .any(|owner| natural_family(owner.material).is_some())
        })
        .map(|shading| {
            Box::new(std::array::from_fn::<_, { EDGE * EDGE }, _>(|i| {
                let normal = shading[i].normal();
                (normal, encode_surface_normal(0, normal.to_array()))
            }))
        });
    let mut mesh = if full_grid {
        // Match Vec's growth headroom: streamed trees can append to this batch.
        MeshBuffers {
            vertices: Vec::with_capacity((N * N * 6 + N * 4 * 4).next_power_of_two()),
            indices: Vec::with_capacity((N * N * 6 + N * 4 * 6).next_power_of_two()),
        }
    } else {
        // Missing or sparse terrain retains demand-driven allocation.
        MeshBuffers::default()
    };
    for z in 0..N {
        for x in 0..N {
            if collision.source_columns[z] & (1 << x) == 0 {
                continue;
            }
            let corners = [
                z * EDGE + x,
                z * EDGE + x + 1,
                (z + 1) * EDGE + x + 1,
                (z + 1) * EDGE + x,
            ];
            if corners
                .iter()
                .any(|i| !collision.heights_local[*i].is_finite())
            {
                collision.source_columns[z] &= !(1 << x);
                continue;
            }
            let points = corners.map(|i| {
                projected.as_ref().map_or_else(
                    || {
                        Vec3::new(
                            (i % EDGE) as f32,
                            collision.heights_local[i],
                            (i / EDGE) as f32,
                        )
                    },
                    |p| p[i],
                )
            });
            let owner = appearance[z][x];
            let owner_family = natural_family(owner.material);
            for triangle in [[0, 3, 2], [0, 2, 1]] {
                let geometric = (points[triangle[1]] - points[triangle[0]])
                    .cross(points[triangle[2]] - points[triangle[0]])
                    .normalize();
                let base = mesh.vertices.len() as u32;
                let mut geometric_encoded = None;
                for i in triangle {
                    let source_corner = source.shading.as_ref().map(|s| s[corners[i]]);
                    let (normal, tint, family) = if owner_family.is_some() {
                        source_corner.map_or((None, owner.tint, owner_family), |s| {
                            let prepared =
                                shading_normals.as_ref().map(|normals| normals[corners[i]]);
                            let n = prepared.map_or_else(|| s.normal(), |prepared| prepared.0);
                            (
                                (n.dot(geometric) >= SMOOTH_CREASE_COS).then(|| {
                                    prepared.map_or_else(
                                        || encode_surface_normal(0, n.to_array()),
                                        |prepared| prepared.1,
                                    )
                                }),
                                s.tint,
                                natural_family(s.material),
                            )
                        })
                    } else {
                        let tint = source_corner
                            .filter(|_| material::is_field_surface(owner.material))
                            .map_or(owner.tint, |s| {
                                crate::terrain_pigment::apply_multiplier(owner.tint, s.pigment)
                            });
                        (None, tint, None)
                    };
                    let payload = family
                        .map_or(2 | (3 << 3), |family| TERRAIN_BLEND_FACE | (family << 3))
                        | (u32::from(owner.material) << 5);
                    mesh.vertices.push(SurfaceVertex {
                        pos: points[i].to_array(),
                        face_ao: payload
                            | normal.unwrap_or_else(|| {
                                *geometric_encoded.get_or_insert_with(|| {
                                    encode_surface_normal(0, geometric.to_array())
                                })
                            }),
                        rgba: [tint[0], tint[1], tint[2], 255],
                    });
                }
                mesh.indices.extend([base, base + 1, base + 2]);
            }
        }
    }
    let conditioned_boundary = if water.is_empty() || mesh.indices.is_empty() {
        None
    } else {
        water_bed::carve(frame, &mesh, water, &mut collision.source_columns).map(|conditioned| {
            mesh = conditioned.mesh;
            conditioned.boundary
        })
    };
    // Normal columns are already represented by the compact heightfield.
    // Only altered shore/bank geometry and the true coarse globe need stored
    // metric triangles; visual skirts never participate in collision.
    let triangles = if !exact_globe && collision.source_columns.iter().all(|&row| row == u32::MAX) {
        Vec::new()
    } else {
        let metric = crate::collision::MetricFrame::of(frame);
        mesh.indices
            .chunks_exact(3)
            .filter_map(|ids| {
                let points = [0, 1, 2].map(|i| mesh.vertices[ids[i] as usize].pos);
                if !exact_globe {
                    let p = points.map(|p| DVec3::from_array(p.map(f64::from)));
                    let center = (p[0] + p[1] + p[2]) / 3.0;
                    let x = center.x.floor().clamp(0.0, (N - 1) as f64) as usize;
                    let z = center.z.floor().clamp(0.0, (N - 1) as f64) as usize;
                    if (p[1] - p[0]).cross(p[2] - p[0]).y > 0.0
                        && collision.source_columns[z] & (1 << x) != 0
                    {
                        return None;
                    }
                }
                crate::collision::triangle_from_local(points, &metric)
            })
            .collect()
    };
    // Direct skirts seal mixed-resolution T junctions. They are visibility
    // closures only; ground queries continue to use the shared height field.
    let lo = source
        .heights_m
        .iter()
        .copied()
        .filter(|h| h.is_finite())
        .fold(f32::INFINITY, f32::min);
    let hi = source
        .heights_m
        .iter()
        .copied()
        .filter(|h| h.is_finite())
        .fold(f32::NEG_INFINITY, f32::max);
    let depth = ((f64::from(hi - lo) * 1.25)
        .max(frame.grid_scale.x.max(frame.grid_scale.z) * 2.0)
        .max(0.25)
        / frame.grid_scale.y) as f32;
    if let Some(boundary) = conditioned_boundary {
        water_bed::append_skirts(&mut mesh, boundary, depth);
    } else {
        for i in 0..N {
            for (a, b, x, z) in [
                ((i, 0), (i + 1, 0), i, 0),
                ((N, i), (N, i + 1), N - 1, i),
                ((i + 1, N), (i, N), i, N - 1),
                ((0, i + 1), (0, i), 0, i),
            ] {
                if collision.source_columns[z] & (1 << x) == 0 {
                    continue;
                }
                let indices = [a.1 * EDGE + a.0, b.1 * EDGE + b.0];
                let top = indices.map(|j| {
                    projected.as_ref().map_or_else(
                        || {
                            Vec3::new(
                                (j % EDGE) as f32,
                                collision.heights_local[j],
                                (j / EDGE) as f32,
                            )
                        },
                        |p| p[j],
                    )
                });
                if top.iter().any(|p| !p.is_finite()) {
                    continue;
                }
                let points = [
                    top[0],
                    top[1],
                    top[1] - Vec3::Y * depth,
                    top[0] - Vec3::Y * depth,
                ];
                let normal = (points[1] - points[0])
                    .cross(points[2] - points[0])
                    .normalize();
                let owner = appearance[z][x];
                let face_ao = encode_surface_normal(
                    2 | (3 << 3) | (u32::from(owner.material) << 5),
                    normal.to_array(),
                );
                let base = mesh.vertices.len() as u32;
                for (j, point) in points.into_iter().enumerate() {
                    let tint = source.shading.as_ref().map_or(owner.tint, |s| {
                        s[indices[usize::from(j == 1 || j == 2)]].tint
                    });
                    mesh.vertices.push(SurfaceVertex {
                        pos: point.to_array(),
                        face_ao,
                        rgba: [tint[0], tint[1], tint[2], 255],
                    });
                }
                mesh.indices
                    .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
    }
    TerrainGeometry {
        mesh,
        collision: (!exact_globe).then_some(collision),
        triangles,
    }
}

fn natural_family(mat: u8) -> Option<u32> {
    match mat {
        material::SAND | material::SOIL | material::SALT => Some(0),
        material::GRASS | material::WETLAND => Some(1),
        material::ROCK | material::VOLCANIC => Some(2),
        material::SNOW | material::ICE => Some(3),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::{cell::CellId, cubesphere::Face};
    fn frame() -> ChunkFrame {
        ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face: Face(0),
                level: 17,
                i: 65536,
                j: 65536,
            },
            k: 0,
        })
    }

    #[test]
    fn missing_and_sparse_terrain_preserve_demand_driven_mesh_allocation() {
        use crate::water_surface::WaterTriangle;
        use vearth_data::vectors::WaterRegionKind;

        let frame = frame();
        let appearance = [[SurfaceAppearance {
            material: material::GRASS,
            tint: [83, 127, 62],
        }; N]; N];
        let mut source = TerrainSurfaceData {
            heights_m: Box::new([40.0; EDGE * EDGE]),
            shading: Some(Box::new(
                [TerrainCornerShading::new([0.2, 0.9, 0.1], material::GRASS, [70, 120, 80]);
                    EDGE * EDGE],
            )),
            source_columns: [0; N],
        };
        for kind in [WaterRegionKind::Ocean, WaterRegionKind::InlandWater] {
            let water = [WaterTriangle {
                vertices: [[0.0, 30.0, 0.0], [0.0, 30.0, 32.0], [32.0, 30.0, 32.0]],
                kind,
                depth_m: 3.0,
            }];
            let missing = build_with_water(&frame, &source, &appearance, &water);
            assert!(missing.mesh.vertices.is_empty() && missing.mesh.indices.is_empty());
            assert_eq!(missing.mesh.vertices.capacity(), 0);
            assert_eq!(missing.mesh.indices.capacity(), 0);
            assert!(missing.triangles.is_empty());
            assert_eq!(missing.collision.unwrap().source_columns, [0; N]);
        }

        source.source_columns.fill(u32::MAX);
        let full = build(&frame, &source, &appearance);
        for (x, z) in [(3, 5), (0, 0), (N - 1, N - 1)] {
            source.source_columns.fill(0);
            source.source_columns[z] = 1 << x;
            let sparse = build(&frame, &source, &appearance);
            let first = (z * N + x) * 6;
            assert_eq!(
                &sparse.mesh.vertices[..6],
                &full.mesh.vertices[first..first + 6]
            );
            let mut original_vertices = Vec::new();
            for &vertex in &sparse.mesh.vertices {
                original_vertices.push(vertex);
            }
            let mut original_indices = Vec::new();
            for indices in sparse.mesh.indices[..6].chunks_exact(3) {
                original_indices.extend_from_slice(indices);
            }
            for indices in sparse.mesh.indices[6..].chunks_exact(6) {
                original_indices.extend_from_slice(indices);
            }
            assert_eq!(
                sparse.mesh.vertices.capacity(),
                original_vertices.capacity()
            );
            assert_eq!(sparse.mesh.indices.capacity(), original_indices.capacity());
        }

        source.source_columns.fill(u32::MAX);
        source.heights_m.fill(f32::NAN);
        let invalid = build(&frame, &source, &appearance);
        assert_eq!(invalid.mesh.vertices.capacity(), 0);
        assert_eq!(invalid.mesh.indices.capacity(), 0);
        assert_eq!(invalid.collision.unwrap().source_columns, [0; N]);
    }

    #[test]
    fn shared_corner_preparation_preserves_all_normal_codes_and_crease_payloads() {
        let materials = [
            material::GRASS,
            material::ROCK,
            material::SNOW,
            material::SAND,
            material::SOIL,
            material::ASPHALT,
            material::FIELD_CROP,
        ];
        let appearance = std::array::from_fn(|z| {
            std::array::from_fn(|x| SurfaceAppearance {
                material: materials[(z * N + x) % materials.len()],
                tint: [83, 127, 62],
            })
        });
        for block in 0..16 {
            let frame = frame();
            let source = TerrainSurfaceData {
                heights_m: Box::new(std::array::from_fn(|i| {
                    40.0 + ((i * 17 % 31) as f32 - 15.0) * 0.4
                })),
                shading: Some(Box::new(std::array::from_fn(|i| TerrainCornerShading {
                    normal_oct: ((block * 1024 + i) & 0x3fff) as u16,
                    material: materials[(i / 7) % materials.len()],
                    tint: [i as u8, (i / 3) as u8, (i / 5) as u8],
                    pigment: crate::terrain_pigment::NEUTRAL,
                }))),
                source_columns: [u32::MAX; N],
            };
            let geometry = build(&frame, &source, &appearance);
            for z in 0..N {
                for x in 0..N {
                    let corners = [
                        z * EDGE + x,
                        z * EDGE + x + 1,
                        (z + 1) * EDGE + x + 1,
                        (z + 1) * EDGE + x,
                    ];
                    let owner = appearance[z][x];
                    for (triangle_index, triangle) in [[0, 3, 2], [0, 2, 1]].into_iter().enumerate()
                    {
                        let first = (z * N + x) * 6 + triangle_index * 3;
                        let vertices = &geometry.mesh.vertices[first..first + 3];
                        let points = [0, 1, 2].map(|i| Vec3::from_array(vertices[i].pos));
                        let geometric = (points[1] - points[0])
                            .cross(points[2] - points[0])
                            .normalize();
                        for (vertex, corner) in vertices.iter().zip(triangle.map(|i| corners[i])) {
                            let s = source.shading.as_ref().unwrap()[corner];
                            let (normal, tint, family) = if natural_family(owner.material).is_some()
                            {
                                let n = s.normal();
                                (
                                    if n.dot(geometric) >= SMOOTH_CREASE_COS {
                                        n
                                    } else {
                                        geometric
                                    },
                                    s.tint,
                                    natural_family(s.material),
                                )
                            } else {
                                (geometric, owner.tint, None)
                            };
                            let payload = family
                                .map_or(2 | (3 << 3), |family| TERRAIN_BLEND_FACE | (family << 3))
                                | (u32::from(owner.material) << 5);
                            assert_eq!(
                                vertex.face_ao,
                                encode_surface_normal(payload, normal.to_array()),
                                "normal={}",
                                s.normal_oct
                            );
                            assert_eq!(vertex.rgba, [tint[0], tint[1], tint[2], 255]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn direct_terrain_collision_matches_every_rendered_triangle() {
        let frame = frame();
        let source = TerrainSurfaceData {
            heights_m: Box::new(std::array::from_fn(|i| {
                (i % EDGE) as f32 * 0.231 + (i / EDGE) as f32 * 0.112 + 12.34567
            })),
            shading: None,
            source_columns: [u32::MAX; N],
        };
        let geometry = build(
            &frame,
            &source,
            &[[SurfaceAppearance {
                material: material::GRASS,
                tint: [80, 120, 65],
            }; N]; N],
        );
        assert_eq!(geometry.mesh.indices.len(), N * N * 6 + N * 4 * 6);
        for indices in geometry.mesh.indices.chunks_exact(3) {
            let p = [0, 1, 2].map(|i| {
                DVec3::from_array(
                    geometry.mesh.vertices[indices[i] as usize]
                        .pos
                        .map(f64::from),
                )
            });
            if (p[1] - p[0]).cross(p[2] - p[0]).y.abs() < 1e-12 {
                continue;
            }
            let centroid = (p[0] + p[1] + p[2]) / 3.0;
            assert!(
                (geometry
                    .collision
                    .as_ref()
                    .unwrap()
                    .sample_local(centroid.x, centroid.z)
                    .unwrap()
                    - centroid.y)
                    .abs()
                    < 1.0e-6
            );
            assert!((p[1] - p[0]).cross(p[2] - p[0]).y > 0.0);
        }
        assert!(
            geometry
                .collision
                .as_ref()
                .unwrap()
                .heights_local
                .iter()
                .any(|h| (h * 64.0).fract().abs() > 0.001)
        );
    }
    #[test]
    fn terrain_masks_only_explicit_missing_surfaces() {
        let mut collision = TerrainCollision {
            heights_local: Box::new([2.0; EDGE * EDGE]),
            source_columns: [u32::MAX; N],
        };
        collision.source_columns[3] &= !(1 << 4);
        assert_eq!(collision.sample_local(4.5, 3.5), None);
        assert_eq!(collision.sample_local(5.5, 3.5), Some(2.0));
        assert_eq!(collision.sample_local(32.0, 32.0), Some(2.0));
        assert_eq!(collision.sample_local(f64::NAN, 0.0), None);
    }
    #[test]
    fn coarse_globe_borders_use_shared_ecef_positions_and_exact_triangle_queries() {
        for level in [3, 5, 8, 10] {
            let i = (1 << level) / 2;
            let j = (1 << level) * 3 / 5;
            let a = ChunkFrame::of(ChunkAddr {
                cell: CellId {
                    face: Face(0),
                    level,
                    i,
                    j,
                },
                k: 0,
            });
            let b = ChunkFrame::of(ChunkAddr {
                cell: CellId {
                    face: Face(0),
                    level,
                    i: i + 1,
                    j,
                },
                k: 0,
            });
            let source = TerrainSurfaceData {
                heights_m: Box::new([100.0; EDGE * EDGE]),
                shading: None,
                source_columns: [u32::MAX; N],
            };
            let appearance = [[SurfaceAppearance {
                material: material::GRASS,
                tint: [90, 120, 80],
            }; N]; N];
            let ga = build(&a, &source, &appearance);
            let gb = build(&b, &source, &appearance);
            assert!(ga.collision.is_none() && gb.collision.is_none());
            assert_eq!(ga.triangles.len(), N * N * 2);
            let top_count = N * N * 6;
            for z in [0, 7, 16, 25, 32] {
                let (p, _) = a.column_surface(1.0, z as f64 / N as f64);
                let expected = p + vearth_core::ellipsoid::geodetic_normal(p) * 100.0;
                let nearest = |frame: &ChunkFrame, g: &TerrainGeometry| {
                    g.mesh.vertices[..top_count]
                        .iter()
                        .map(|v| frame.local_to_ecef(DVec3::from_array(v.pos.map(f64::from))))
                        .min_by(|p, q| {
                            p.distance_squared(expected)
                                .total_cmp(&q.distance_squared(expected))
                        })
                        .unwrap()
                };
                let pa = nearest(&a, &ga);
                let pb = nearest(&b, &gb);
                let tolerance = a.grid_scale.max_element() * 8e-6;
                assert!(
                    pa.distance(pb) < tolerance,
                    "L{level}: {} metre gap",
                    pa.distance(pb)
                );
                assert!(pa.distance(expected) < tolerance);
            }
        }
    }

    #[test]
    fn direct_skirts_cover_mixed_lod_edge_height_variation() {
        let frame = frame();
        let source = TerrainSurfaceData {
            heights_m: Box::new(std::array::from_fn(|i| {
                100.0 + 30.0 * (i as f32 * 0.19).sin()
            })),
            shading: None,
            source_columns: [u32::MAX; N],
        };
        let geometry = build(
            &frame,
            &source,
            &[[SurfaceAppearance {
                material: material::ROCK,
                tint: [110, 115, 109],
            }; N]; N],
        );
        let skirts = &geometry.mesh.vertices[N * N * 6..];
        assert_eq!(skirts.len(), N * 4 * 4);
        for quad in skirts.chunks_exact(4) {
            assert_eq!(
                (quad[0].pos[0], quad[0].pos[2]),
                (quad[3].pos[0], quad[3].pos[2])
            );
            assert_eq!(
                (quad[1].pos[0], quad[1].pos[2]),
                (quad[2].pos[0], quad[2].pos[2])
            );
            assert!((quad[0].pos[1] - quad[3].pos[1]) * frame.grid_scale.y as f32 > 60.0);
        }
        assert!(geometry.triangles.is_empty());
        assert!(geometry.collision.is_some());
    }
}
