//! Direct typed water surfaces. Inland heights are a continuous, inferred DEM
//! profile; source footprints do not imply measured water levels or bathymetry.

use crate::{N, SurfaceAppearance, TerrainSurfaceData, material};
use glam::{DVec2, DVec3};
use std::collections::BTreeMap;
use vearth_core::{
    ellipsoid::MEAN_RADIUS,
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex},
};
use vearth_data::vectors::{
    FeaturePack, WaterRegionKind, Watercourse, WatercourseKind, WatercourseOutputMode,
};

/// Reach of the inferred datum beyond a sampled vertex: one 64 m lattice step
/// plus the lattice node's ±32 m neighbourhood, with slack for bilinear taps.
pub(crate) const DATUM_MARGIN_M: f64 = 128.0;

/// Fixed-detail elevation coverage required by the inferred water datum.
///
/// Datum reads happen only at vertices of water pieces clipped to the cell,
/// so each surface contributes its clipped column bounds plus
/// [`DATUM_MARGIN_M`], built with the same projection and ribbon offsets as
/// [`build_corrected`]. Geometry that cannot be bounded keeps the whole cell.
pub(crate) fn add_datum_cover(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    cover: &mut crate::DatumCover,
) {
    let (center_lat, center_lon) = frame.column_latlon(0.5, 0.5);
    let projection = crate::tier0::GridProj::of(frame, center_lat, center_lon);
    let mut add = |points: &mut dyn Iterator<Item = [f32; 2]>| {
        let mut lo = DVec2::splat(f64::INFINITY);
        let mut hi = DVec2::splat(f64::NEG_INFINITY);
        let mut bounded = true;
        for point in points {
            let column = projection.to_cols(point);
            if column.iter().any(|value| !value.is_finite()) {
                bounded = false;
                break;
            }
            let column = DVec2::new(f64::from(column[0]), f64::from(column[1]));
            lo = lo.min(column);
            hi = hi.max(column);
        }
        let (lo, hi) = if bounded && lo.x <= hi.x {
            (lo.max(DVec2::ZERO), hi.min(DVec2::splat(N as f64)))
        } else {
            (DVec2::ZERO, DVec2::splat(N as f64))
        };
        if lo.x > hi.x || lo.y > hi.y {
            return;
        }
        let corners = [(lo.x, lo.y), (hi.x, lo.y), (hi.x, hi.y), (lo.x, hi.y)]
            .map(|(x, z)| frame.column_latlon(x / N as f64, z / N as f64));
        if !cover.add(&corners, DATUM_MARGIN_M) {
            let cell = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
                .map(|(x, z)| frame.column_latlon(x, z));
            cover.add(&cell, DATUM_MARGIN_M);
        }
    };
    for water in &pack.water {
        if !water.is_ocean() && water.creates_surface_water() {
            add(&mut water.rings.iter().flatten().copied());
        }
    }
    for water in &pack.watercourses {
        if !watercourse_creates_surface(pack.watercourse_output_mode, water, frame.addr.cell.level)
            || !water.width_m.is_finite()
            || water.width_m <= 0.0
        {
            continue;
        }
        for path in &water.paths {
            let mut points: Vec<_> = path
                .iter()
                .map(|p| DVec2::from_array(p.map(f64::from)))
                .collect();
            if points.iter().any(|p| !p.is_finite()) {
                continue;
            }
            points.dedup_by(|a, b| a.distance_squared(*b) < 1e-12);
            if points.len() < 2 {
                continue;
            }
            let half = f64::from(water.width_m) * 0.5;
            let left = crate::road_surface::offset_polyline(&points, -half);
            let right = crate::road_surface::offset_polyline(&points, half);
            add(&mut left
                .into_iter()
                .chain(right)
                .map(|p| [p.x as f32, p.y as f32]));
        }
    }
}

/// Raster ecology additionally applies a column-width gate; direct water does not.
fn watercourse_creates_surface(
    mode: WatercourseOutputMode,
    water: &Watercourse,
    level: u8,
) -> bool {
    mode == WatercourseOutputMode::RiversAndCanals
        && !water.intermittent
        && !water.is_bridge
        && water.source_tile.0 == water.authority_zoom
        && water.visible_at_terrain_level(level)
        && matches!(water.kind, WatercourseKind::River | WatercourseKind::Canal)
}

const DATUM_STEP_M: f64 = 64.0;
const MAX_TRIANGLES: usize = 131_072;
const MAX_PIECES: usize = 2048;

/// Vertices share the cell-k0 render grid exactly. Depth is an inferred optical
/// and bed-conditioning value, not a claim of measured navigable depth.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterTriangle {
    pub vertices: [[f32; 3]; 3],
    pub kind: WaterRegionKind,
    pub depth_m: f32,
}

pub(crate) struct WaterGeometry {
    pub mesh: MeshBuffers,
    pub triangles: Vec<WaterTriangle>,
    pub complete: bool,
}

struct Region {
    polygon: Vec<DVec2>,
    kind: WaterRegionKind,
    line: bool,
}

struct RegionIndex {
    offsets: Vec<usize>,
    ids: Vec<usize>,
}

impl RegionIndex {
    fn new(regions: &[Region]) -> Self {
        let ranges: Vec<_> = regions
            .iter()
            .map(|region| {
                let mut lo = DVec2::splat(f64::INFINITY);
                let mut hi = DVec2::splat(f64::NEG_INFINITY);
                for &point in &region.polygon {
                    if !point.is_finite() {
                        return (0, 0, N - 1, N - 1);
                    }
                    lo = lo.min(point);
                    hi = hi.max(point);
                }
                // Keep touching squares and a full grid-square margin. The
                // narrow phase remains authoritative at rounded source edges.
                let lower = |v: f64| (v.floor() - 1.0).clamp(0.0, (N - 1) as f64) as usize;
                let upper = |v: f64| (v.floor() + 1.0).clamp(0.0, (N - 1) as f64) as usize;
                (lower(lo.x), lower(lo.y), upper(hi.x), upper(hi.y))
            })
            .collect();
        let mut offsets = vec![0; N * N + 1];
        for &(x0, z0, x1, z1) in &ranges {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    offsets[z * N + x + 1] += 1;
                }
            }
        }
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }
        let mut ids = vec![0; offsets[N * N]];
        let mut next = offsets[..N * N].to_vec();
        // Source order determines ownership where differently typed regions
        // overlap. Each square retains that order, including equal priorities.
        for (id, &(x0, z0, x1, z1)) in ranges.iter().enumerate() {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let square = z * N + x;
                    ids[next[square]] = id;
                    next[square] += 1;
                }
            }
        }
        Self { offsets, ids }
    }

    fn candidates(&self, x: usize, z: usize) -> &[usize] {
        let square = z * N + x;
        &self.ids[self.offsets[square]..self.offsets[square + 1]]
    }
}

fn water_class(kind: WaterRegionKind) -> u32 {
    if kind.is_ocean() {
        1
    } else if matches!(kind, WaterRegionKind::River | WaterRegionKind::Canal) {
        3
    } else {
        2
    }
}

/// Global angular keys make the inferred datum independent of the caller's
/// cell, source tile clipping, and rendering level. Longitude keys wrap.
struct Datum<'a> {
    source: &'a dyn Fn(f64, f64) -> Option<f32>,
    values: BTreeMap<(i64, i64), f64>,
    complete: bool,
}
impl Datum<'_> {
    fn sample(&mut self, lat: f64, lon: f64) -> Option<f64> {
        let count = (std::f64::consts::TAU * MEAN_RADIUS / DATUM_STEP_M).ceil() as i64;
        let angle = std::f64::consts::TAU / count as f64;
        let x = lon / angle;
        let y = lat / angle;
        let ix = x.floor() as i64;
        let iy = y.floor() as i64;
        let mut h = [0.0; 4];
        for (i, (dx, dy)) in [(0, 0), (1, 0), (1, 1), (0, 1)].into_iter().enumerate() {
            let key = ((ix + dx).rem_euclid(count), iy + dy);
            h[i] = if let Some(&v) = self.values.get(&key) {
                v
            } else {
                let a = ((iy + dy) as f64 * angle)
                    .clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
                let b = ((ix + dx) as f64 * angle + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
                let mut values = Vec::with_capacity(9);
                for north in [-1.0, 0.0, 1.0] {
                    for east in [-1.0, 0.0, 1.0] {
                        let latitude = (a + north * 32.0 / MEAN_RADIUS)
                            .clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
                        let longitude = (b
                            + east * 32.0 / (MEAN_RADIUS * a.cos().abs().max(0.01))
                            + std::f64::consts::PI)
                            .rem_euclid(std::f64::consts::TAU)
                            - std::f64::consts::PI;
                        match (self.source)(latitude, longitude).filter(|h| h.is_finite()) {
                            Some(value) => values.push(f64::from(value)),
                            None => self.complete = false,
                        }
                    }
                }
                if values.is_empty() {
                    return None;
                }
                values.sort_by(f64::total_cmp);
                // A low quantile rejects bank/building outliers without the
                // severe terrain depressions produced by a minimum filter.
                let value = values[(values.len() - 1) / 5];
                self.values.insert(key, value);
                value
            };
        }
        let (u, v) = (x - ix as f64, y - iy as f64);
        Some(if v <= u {
            h[0] + (h[1] - h[0]) * u + (h[2] - h[1]) * v
        } else {
            h[0] + (h[2] - h[3]) * u + (h[3] - h[0]) * v
        })
    }
}

#[cfg(test)]
pub(crate) fn build(
    frame: &ChunkFrame,
    terrain: &TerrainSurfaceData,
    pack: Option<&FeaturePack>,
    elev_geo: &dyn Fn(f64, f64) -> Option<f32>,
    appearance: &dyn Fn(f64, f64, f32, f32, WaterRegionKind) -> SurfaceAppearance,
) -> WaterGeometry {
    build_with_policy::<true>(frame, terrain, pack, elev_geo, appearance)
}

#[cfg(test)]
fn build_with_policy<const COALESCED: bool>(
    frame: &ChunkFrame,
    terrain: &TerrainSurfaceData,
    pack: Option<&FeaturePack>,
    elev_geo: &dyn Fn(f64, f64) -> Option<f32>,
    appearance: &dyn Fn(f64, f64, f32, f32, WaterRegionKind) -> SurfaceAppearance,
) -> WaterGeometry {
    build_corrected::<COALESCED>(frame, terrain, pack, elev_geo, appearance, &|_, _, h| h)
}

pub(crate) fn build_corrected<const COALESCED: bool>(
    frame: &ChunkFrame,
    terrain: &TerrainSurfaceData,
    pack: Option<&FeaturePack>,
    elev_geo: &dyn Fn(f64, f64) -> Option<f32>,
    appearance: &dyn Fn(f64, f64, f32, f32, WaterRegionKind) -> SurfaceAppearance,
    inland_height: &dyn Fn(f64, f64, f64) -> f64,
) -> WaterGeometry {
    use crate::road_surface::{canonical, difference, intersection};
    let mut result = WaterGeometry {
        mesh: MeshBuffers::default(),
        triangles: Vec::new(),
        complete: true,
    };
    let (lat, lon) = frame.column_latlon(0.5, 0.5);
    let projection = crate::tier0::GridProj::of(frame, lat, lon);
    let mut regions = Vec::<Region>::new();
    if let Some(pack) = pack {
        let mut add = |rings: &vearth_data::vectors::Rings, kind, line| {
            let columns: vearth_data::vectors::Rings = rings
                .iter()
                .map(|ring| ring.iter().map(|&p| projection.to_cols(p)).collect())
                .collect();
            let cells = if COALESCED {
                crate::building_surface::source_polygon_cells_clipped_coalesced(
                    &columns,
                    DVec2::ZERO,
                    DVec2::splat(N as f64),
                    1e9,
                )
            } else {
                crate::building_surface::source_polygon_cells_clipped(
                    &columns,
                    DVec2::ZERO,
                    DVec2::splat(N as f64),
                    1e9,
                )
            };
            let Some(cells) = cells else {
                return false;
            };
            for quad in cells {
                let polygon = canonical(quad.into_iter().collect());
                if !polygon.is_empty() {
                    regions.push(Region {
                        polygon,
                        kind,
                        line,
                    });
                }
            }
            true
        };
        for water in &pack.water {
            if water.creates_surface_water() {
                result.complete &= add(&water.rings, water.kind, false);
            }
        }
        if pack.watercourse_output_mode == WatercourseOutputMode::RiversAndCanals {
            for water in &pack.watercourses {
                if !watercourse_creates_surface(
                    pack.watercourse_output_mode,
                    water,
                    frame.addr.cell.level,
                ) {
                    continue;
                }
                if !water.width_m.is_finite() || water.width_m <= 0.0 || water.paths.is_empty() {
                    result.complete = false;
                    continue;
                }
                let kind = if water.kind == WatercourseKind::River {
                    WaterRegionKind::River
                } else {
                    WaterRegionKind::Canal
                };
                for path in &water.paths {
                    let mut points: Vec<_> = path
                        .iter()
                        .map(|p| DVec2::from_array(p.map(f64::from)))
                        .collect();
                    if points.iter().any(|p| !p.is_finite()) {
                        result.complete = false;
                        continue;
                    }
                    // Tile fragments can repeat their first/last coordinate.
                    // Such vertices must not create zero-length offset normals.
                    points.dedup_by(|a, b| a.distance_squared(*b) < 1e-12);
                    if points.len() < 2 {
                        result.complete = false;
                        continue;
                    }
                    let mut ring = crate::road_surface::offset_polyline(
                        &points,
                        -f64::from(water.width_m) * 0.5,
                    );
                    ring.extend(
                        crate::road_surface::offset_polyline(
                            &points,
                            f64::from(water.width_m) * 0.5,
                        )
                        .into_iter()
                        .rev(),
                    );
                    result.complete &= add(
                        &vec![ring.into_iter().map(|p| [p.x as f32, p.y as f32]).collect()],
                        kind,
                        true,
                    );
                }
            }
        }
    }
    regions.sort_by_key(|r| (r.line, std::cmp::Reverse(r.kind.precedence())));
    let region_index = RegionIndex::new(&regions);
    let mut datum = Datum {
        source: elev_geo,
        values: BTreeMap::new(),
        complete: true,
    };
    let original = &terrain.heights_m;
    let terrain_h = |p: DVec2| {
        let x = (p.x.floor().max(0.0) as usize).min(N - 1);
        let z = (p.y.floor().max(0.0) as usize).min(N - 1);
        let u = (p.x - x as f64).clamp(0.0, 1.0);
        let v = (p.y - z as f64).clamp(0.0, 1.0);
        let [a, b, c, d] = [(x, z), (x + 1, z), (x + 1, z + 1), (x, z + 1)]
            .map(|(x, z)| f64::from(original[z * (N + 1) + x]));
        if v <= u {
            a + (b - a) * u + (c - b) * v
        } else {
            a + (c - d) * u + (d - a) * v
        }
    };
    let inferred_ocean = pack.is_none();
    for z in 0..N {
        for x in 0..N {
            let square = [
                DVec2::new(x as f64, z as f64),
                DVec2::new(x as f64 + 1.0, z as f64),
                DVec2::new(x as f64 + 1.0, z as f64 + 1.0),
                DVec2::new(x as f64, z as f64 + 1.0),
            ];
            let mut accepted = Vec::<Vec<DVec2>>::new();
            let mut pieces = Vec::<(Vec<DVec2>, WaterRegionKind)>::new();
            for &region_id in region_index.candidates(x, z) {
                let region = &regions[region_id];
                let clipped = intersection(&region.polygon, &square);
                if clipped.len() < 3 {
                    continue;
                }
                let mut remaining = vec![clipped];
                for cover in &accepted {
                    remaining = remaining
                        .into_iter()
                        .flat_map(|p| difference(&p, cover))
                        .collect();
                    if remaining.len() > MAX_PIECES {
                        result.complete = false;
                        remaining.clear();
                        break;
                    }
                }
                for piece in remaining {
                    if piece.len() < 3 {
                        continue;
                    }
                    accepted.push(piece.clone());
                    pieces.push((piece, region.kind));
                }
            }
            if inferred_ocean {
                for ids in [[0, 1, 2], [0, 2, 3]] {
                    let polygon: Vec<_> = ids.map(|i| square[i]).to_vec();
                    let mut clipped = Vec::new();
                    for (&p, &q) in polygon.iter().zip(polygon.iter().cycle().skip(1)).take(3) {
                        let a = terrain_h(p);
                        let b = terrain_h(q);
                        if a <= 0.0 {
                            clipped.push(p);
                        }
                        if (a <= 0.0) != (b <= 0.0) {
                            clipped.push(p.lerp(q, a / (a - b)));
                        }
                    }
                    let clipped = canonical(clipped);
                    if clipped.len() >= 3 {
                        pieces.push((clipped, WaterRegionKind::Ocean));
                    }
                }
            }
            for (polygon, kind) in pieces {
                if polygon.len() < 3 {
                    continue;
                }
                if result.triangles.len() >= MAX_TRIANGLES {
                    result.complete = false;
                    break;
                }
                let mut vertices = Vec::new();
                let mut colors = Vec::new();
                let mut depths = Vec::new();
                for &p in &polygon {
                    let (a, b) = frame.column_latlon(p.x / N as f64, p.y / N as f64);
                    let surface = if kind.is_ocean() {
                        Some(0.0)
                    } else {
                        datum.sample(a, b)
                    };
                    let Some(surface) = surface else {
                        result.complete = false;
                        break;
                    };
                    // Correct the inferred datum at shared geographic vertices,
                    // before both the visible surface and riverbed use it.
                    let surface = if kind.is_ocean() {
                        surface
                    } else {
                        inland_height(a, b, surface)
                    };
                    let ground = terrain_h(p);
                    let depth =
                        (surface - ground).max(if kind.is_ocean() { 0.0 } else { 1.2 }) as f32;
                    let (_, offset) = frame.column_surface(p.x / N as f64, p.y / N as f64);
                    let position = if frame.addr.cell.level < 11 {
                        frame
                            .ecef_to_local(vearth_core::ellipsoid::geodetic_to_ecef(a, b, surface))
                            .as_vec3()
                            .to_array()
                    } else {
                        [
                            p.x as f32,
                            ((surface + offset) / frame.grid_scale.y) as f32,
                            p.y as f32,
                        ]
                    };
                    vertices.push(position);
                    colors.push(appearance(
                        a,
                        b,
                        depth,
                        (1.0 - depth / 8.0).clamp(0.0, 1.0),
                        kind,
                    ));
                    depths.push(depth);
                }
                if vertices.len() < 3 || vertices.len() != polygon.len() {
                    continue;
                }
                for (i, _) in vertices[1..].windows(2).enumerate() {
                    let ids = [0, i + 2, i + 1];
                    let triangle = ids.map(|j| vertices[j]);
                    let p = triangle.map(|p| DVec3::from_array(p.map(f64::from)));
                    if (p[1] - p[0]).cross(p[2] - p[0]).length_squared() < 1e-20 {
                        continue;
                    }
                    let first = result.mesh.vertices.len() as u32;
                    result.mesh.vertices.extend(ids.map(|j| SurfaceVertex {
                        pos: vertices[j],
                        // Ordinary packing retains the typed water class in
                        // two spare bits without another vertex attribute.
                        face_ao: 2
                            | (3 << 3)
                            | (u32::from(material::WATER) << 5)
                            | (water_class(kind) << 13),
                        rgba: [colors[j].tint[0], colors[j].tint[1], colors[j].tint[2], 255],
                    }));
                    result.mesh.indices.extend([first, first + 1, first + 2]);
                    result.triangles.push(WaterTriangle {
                        vertices: triangle,
                        kind,
                        depth_m: ids.iter().map(|&j| depths[j]).sum::<f32>() / 3.0,
                    });
                }
            }
        }
    }
    result.complete &= datum.complete;
    result
}
#[cfg(test)]
fn inside(polygon: &[DVec2], p: DVec2) -> bool {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .all(|(&a, &b)| (b - a).perp_dot(p - a) >= -1e-7)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cover(frame: &ChunkFrame, pack: &FeaturePack) -> crate::DatumCover {
        let pitch = crate::datum_pitch_m(frame.column_latlon(0.5, 0.5).0);
        let mut cover = crate::DatumCover::new(frame, pitch);
        add_datum_cover(frame, pack, &mut cover);
        cover
    }
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        cubesphere::Face,
    };
    fn fixture() -> (ChunkFrame, TerrainSurfaceData) {
        let frame = ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face: Face(0),
                level: 17,
                i: 65000,
                j: 65000,
            },
            k: 0,
        });
        (
            frame,
            TerrainSurfaceData {
                heights_m: Box::new([-10.0; (N + 1) * (N + 1)]),
                shading: None,
                source_columns: [u32::MAX; N],
            },
        )
    }
    fn color(_: f64, _: f64, _: f32, _: f32, _: WaterRegionKind) -> SurfaceAppearance {
        SurfaceAppearance {
            material: material::WATER,
            tint: [35, 100, 135],
        }
    }

    fn preparation_line(frame: &ChunkFrame, kind: WatercourseKind) -> Watercourse {
        Watercourse {
            paths: vec![vec![
                component_en(frame, [4.0, 16.0]),
                component_en(frame, [28.0, 16.0]),
            ]],
            kind,
            width_m: kind.fallback_width_m(),
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(42),
            style_identity: 42,
            source_tile: (15, 0, 0),
            authority_zoom: 15,
            source_feature_index: 0,
            source_min_zoom: None,
            fragment_count: 1,
            intermittent: false,
            is_bridge: false,
        }
    }

    #[test]
    fn corrected_inland_datum_reaches_mesh_and_bed_without_moving_ocean() {
        use vearth_data::vectors::Water;
        for kind in [WaterRegionKind::River, WaterRegionKind::Ocean] {
            let (frame, mut terrain) = fixture();
            if !kind.is_ocean() {
                terrain.heights_m.fill(20.0);
            }
            let mut pack = FeaturePack::empty();
            let mut region = Water::inland(vec![
                [[-4.0, -4.0], [36.0, -4.0], [36.0, 36.0], [-4.0, 36.0]]
                    .map(|p| component_en(&frame, p))
                    .to_vec(),
            ]);
            region.kind = kind;
            pack.water.push(region);
            let corrected = build_corrected::<true>(
                &frame,
                &terrain,
                Some(&pack),
                &|_, _| Some(30.0),
                &color,
                &|_, _, inferred| {
                    assert!(!kind.is_ocean(), "ocean bypasses architectural corrections");
                    assert!((inferred - 30.0).abs() < 0.01);
                    5.0
                },
            );
            assert!(corrected.complete && !corrected.triangles.is_empty());
            for triangle in &corrected.triangles {
                for vertex in triangle.vertices {
                    let (_, offset) = frame.column_surface(
                        f64::from(vertex[0]) / N as f64,
                        f64::from(vertex[2]) / N as f64,
                    );
                    let altitude = f64::from(vertex[1]) * frame.grid_scale.y - offset;
                    assert!((altitude - if kind.is_ocean() { 0.0 } else { 5.0 }).abs() < 0.01);
                }
            }
            let appearances = [[color(0.0, 0.0, 0.0, 0.0, kind); N]; N];
            let ground = crate::terrain_surface::build_with_water(
                &frame,
                &terrain,
                &appearances,
                &corrected.triangles,
            );
            let probe = ComponentMeshProbe::new(&ground.mesh);
            let point = DVec2::new(16.13, 15.62);
            let height = probe.at(point).expect("river bed remains solid").0;
            let (_, offset) = frame.column_surface(point.x / N as f64, point.y / N as f64);
            let altitude = height * frame.grid_scale.y - offset;
            assert!(altitude <= if kind.is_ocean() { -9.9 } else { 3.81 });
        }
    }

    #[test]
    fn water_preparation_excludes_lines_that_cannot_create_surface() {
        use std::cell::Cell;
        let (frame, terrain) = fixture();
        let baseline = preparation_line(&frame, WatercourseKind::River);
        let mut cases = Vec::new();
        cases.push((WatercourseOutputMode::DisabledG96, baseline.clone()));
        let mode = WatercourseOutputMode::RiversAndCanals;
        for kind in [
            WatercourseKind::Stream,
            WatercourseKind::Drain,
            WatercourseKind::Ditch,
        ] {
            cases.push((mode, preparation_line(&frame, kind)));
        }
        let mut intermittent = baseline.clone();
        intermittent.intermittent = true;
        cases.push((mode, intermittent));
        let mut bridge = baseline.clone();
        bridge.is_bridge = true;
        cases.push((mode, bridge));
        let mut wrong_zoom = baseline.clone();
        wrong_zoom.source_tile.0 = 14;
        cases.push((mode, wrong_zoom));
        for (mode, line) in cases {
            let mut pack = FeaturePack::empty();
            pack.watercourse_output_mode = mode;
            pack.watercourses.push(line);
            assert!(cover(&frame, &pack).is_empty());
            let calls = Cell::new(0);
            let water = build(
                &frame,
                &terrain,
                Some(&pack),
                &|_, _| {
                    calls.set(calls.get() + 1);
                    Some(1.0)
                },
                &color,
            );
            assert!(water.complete);
            assert!(water.triangles.is_empty());
            assert_eq!(calls.get(), 0);
            assert_eq!(
                pack.watercourses.len(),
                1,
                "raw source facts remain available"
            );
        }
        for kind in [WatercourseKind::River, WatercourseKind::Canal] {
            for level in 11..kind.minimum_terrain_level() {
                let mut cell = frame.addr.cell;
                while cell.level > level {
                    cell = cell.parent().unwrap();
                }
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                let mut pack = FeaturePack::empty();
                pack.watercourse_output_mode = mode;
                pack.watercourses.push(preparation_line(&frame, kind));
                assert!(cover(&frame, &pack).is_empty(), "{kind:?} L{level}");
                let calls = Cell::new(0);
                let water = build(
                    &frame,
                    &terrain,
                    Some(&pack),
                    &|_, _| {
                        calls.set(calls.get() + 1);
                        Some(1.0)
                    },
                    &color,
                );
                assert!(water.complete);
                assert!(water.triangles.is_empty());
                assert_eq!(calls.get(), 0);
            }
        }
    }

    #[test]
    fn water_preparation_keeps_first_admitted_lines_and_covers_every_datum_read() {
        use std::cell::Cell;
        let (_, terrain) = fixture();
        for kind in [WatercourseKind::River, WatercourseKind::Canal] {
            let level = kind.minimum_terrain_level();
            let frame = ChunkFrame::of(ChunkAddr {
                cell: CellId {
                    face: Face(0),
                    level,
                    i: 1 << (level - 1),
                    j: 1 << (level - 1),
                },
                k: 0,
            });
            let mut pack = FeaturePack::empty();
            pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
            pack.watercourses.push(preparation_line(&frame, kind));
            let cover = cover(&frame, &pack);
            let caps = cover.footprints();
            assert!(!caps.is_empty());
            let pitch = crate::datum_pitch_m(frame.column_latlon(0.5, 0.5).0);
            assert!(caps.iter().all(|cap| cap.sample_hint_m == pitch));
            let calls = Cell::new(0);
            let water = build(
                &frame,
                &terrain,
                Some(&pack),
                &|lat, lon| {
                    assert!(
                        cover.contains(lat, lon),
                        "{kind:?} datum read outside cover"
                    );
                    assert!(caps.iter().any(|cap| cap.contains(lat, lon)));
                    calls.set(calls.get() + 1);
                    Some(1.0)
                },
                &color,
            );
            assert!(water.complete);
            assert!(!water.triangles.is_empty());
            assert!(calls.get() > 0);
        }
    }

    #[test]
    fn water_preparation_rejects_only_proven_disjoint_polygons() {
        use std::cell::Cell;
        use vearth_data::vectors::Water;
        let (frame, terrain) = fixture();
        let rectangle = |x0, z0, x1, z1| {
            vec![
                component_en(&frame, [x0, z0]),
                component_en(&frame, [x1, z0]),
                component_en(&frame, [x1, z1]),
                component_en(&frame, [x0, z1]),
            ]
        };
        for ring in [
            rectangle(-4.0, 1.0, -1.0, 4.0),
            rectangle(33.0, 1.0, 36.0, 4.0),
            rectangle(1.0, -4.0, 4.0, -1.0),
            rectangle(1.0, 33.0, 4.0, 36.0),
        ] {
            let mut pack = FeaturePack::empty();
            pack.water.push(Water::inland(vec![ring]));
            assert!(cover(&frame, &pack).is_empty());
            let calls = Cell::new(0);
            let water = build(
                &frame,
                &terrain,
                Some(&pack),
                &|_, _| {
                    calls.set(calls.get() + 1);
                    Some(1.0)
                },
                &color,
            );
            assert!(water.complete);
            assert!(water.triangles.is_empty());
            assert_eq!(calls.get(), 0);
        }
        for rings in [
            vec![rectangle(-4.0, -4.0, 36.0, 36.0)],
            vec![
                rectangle(-4.0, -4.0, 36.0, 36.0),
                rectangle(4.0, 4.0, 28.0, 28.0),
            ],
            vec![rectangle(-1.0, 1.0, 0.01, 4.0)],
            vec![
                rectangle(1.0, 1.0, 3.0, 3.0),
                rectangle(40.0, 40.0, 44.0, 44.0),
            ],
        ] {
            let mut pack = FeaturePack::empty();
            pack.water.push(Water::inland(rings));
            assert!(!cover(&frame, &pack).is_empty());
            let calls = Cell::new(0);
            let water = build(
                &frame,
                &terrain,
                Some(&pack),
                &|_, _| {
                    calls.set(calls.get() + 1);
                    Some(1.0)
                },
                &color,
            );
            assert!(water.complete);
            assert!(!water.triangles.is_empty());
            assert!(calls.get() > 0);
        }
        for rings in [
            vec![],
            vec![vec![]],
            vec![vec![[1.0, 1.0]; 2]],
            vec![vec![[f32::NAN, 1.0]; 3]],
            vec![vec![[f32::INFINITY, 1.0]; 3]],
        ] {
            // Geometry that cannot be bounded keeps whole-cell coverage.
            let mut pack = FeaturePack::empty();
            pack.water.push(Water::inland(rings));
            assert!(!cover(&frame, &pack).is_empty());
        }
    }

    #[test]
    fn water_preparation_preserves_mixed_geometry_and_datum_queries() {
        use std::cell::RefCell;
        use vearth_data::vectors::Water;
        let (frame, terrain) = fixture();
        let mut pack = FeaturePack::empty();
        pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
        pack.watercourses
            .push(preparation_line(&frame, WatercourseKind::River));
        let sample = |pack: &FeaturePack| {
            let keys = RefCell::new(Vec::new());
            let water = build(
                &frame,
                &terrain,
                Some(pack),
                &|lat, lon| {
                    keys.borrow_mut().push((lat.to_bits(), lon.to_bits()));
                    Some(1.0)
                },
                &color,
            );
            (water, keys.into_inner())
        };
        let caps = cover(&frame, &pack).footprints();
        let (expected, keys) = sample(&pack);
        pack.watercourses
            .push(preparation_line(&frame, WatercourseKind::Ditch));
        pack.water.push(Water::inland(vec![vec![
            component_en(&frame, [40.0, 40.0]),
            component_en(&frame, [44.0, 40.0]),
            component_en(&frame, [44.0, 44.0]),
            component_en(&frame, [40.0, 44.0]),
        ]]));
        assert_eq!(cover(&frame, &pack).footprints(), caps);
        let (actual, actual_keys) = sample(&pack);
        assert_eq!(actual.complete, expected.complete);
        assert_eq!(actual.triangles, expected.triangles);
        assert_eq!(actual.mesh.vertices, expected.mesh.vertices);
        assert_eq!(actual.mesh.indices, expected.mesh.indices);
        assert_eq!(actual_keys, keys);
    }

    #[test]
    fn water_region_index_preserves_all_intersections_in_source_order() {
        use crate::road_surface::{canonical, intersection};
        let mut regions = Vec::new();
        for i in 0..256 {
            let x = ((i * 73) % 347) as f64 * 0.1 - 1.0;
            let z = ((i * 47) % 337) as f64 * 0.1 - 1.0;
            let width = [0.00001, 0.17, 0.9, 4.2, 40.0][i % 5];
            let height = [0.2, 3.7, 0.00001, 1.0][i % 4];
            regions.push(Region {
                polygon: canonical(vec![
                    DVec2::new(x, z),
                    DVec2::new(x + width, z),
                    DVec2::new(x + width * 0.3, z + height),
                ]),
                kind: WaterRegionKind::Lake,
                line: false,
            });
        }
        for edge in 0..=N {
            let edge = edge as f64;
            for x in [edge.next_down(), edge, edge.next_up()] {
                regions.push(Region {
                    polygon: canonical(vec![
                        DVec2::new(x, 0.0),
                        DVec2::new(x + 0.0001, 0.0),
                        DVec2::new(x + 0.0001, N as f64),
                        DVec2::new(x, N as f64),
                    ]),
                    kind: WaterRegionKind::Canal,
                    line: true,
                });
            }
        }
        let index = RegionIndex::new(&regions);
        for z in 0..N {
            for x in 0..N {
                let square = [
                    DVec2::new(x as f64, z as f64),
                    DVec2::new(x as f64 + 1.0, z as f64),
                    DVec2::new(x as f64 + 1.0, z as f64 + 1.0),
                    DVec2::new(x as f64, z as f64 + 1.0),
                ];
                let clipped = |id: usize| {
                    let polygon = intersection(&regions[id].polygon, &square);
                    (polygon.len() >= 3).then_some((id, polygon))
                };
                let expected: Vec<_> = (0..regions.len()).filter_map(clipped).collect();
                let actual: Vec<_> = index
                    .candidates(x, z)
                    .iter()
                    .copied()
                    .filter_map(clipped)
                    .collect();
                assert_eq!(actual, expected, "square {x},{z}");
            }
        }
        assert!(index.ids.len() < regions.len() * N * N / 8);
        assert!(RegionIndex::new(&[]).candidates(0, 0).is_empty());
    }

    #[test]
    fn typed_water_class_survives_ordinary_gpu_packing_without_extra_attributes() {
        use vearth_core::mesh::{PackedSurfaceVertex, SURFACE_NORMAL_FLAG};
        use vearth_data::vectors::Water;
        let (frame, terrain) = fixture();
        for (kind, class) in [
            (WaterRegionKind::Ocean, 1),
            (WaterRegionKind::Lake, 2),
            (WaterRegionKind::River, 3),
            (WaterRegionKind::Canal, 3),
        ] {
            let vertex = SurfaceVertex {
                pos: [1.0, 0.0, 2.0],
                face_ao: 2
                    | (3 << 3)
                    | (u32::from(material::WATER) << 5)
                    | (water_class(kind) << 13),
                rgba: [35, 100, 135, 255],
            };
            assert_eq!(
                PackedSurfaceVertex::try_from_full(vertex)
                    .unwrap()
                    .to_full(),
                vertex
            );
            let mut source = Water::inland(vec![vec![
                [-100.0, -100.0],
                [100.0, -100.0],
                [100.0, 100.0],
                [-100.0, 100.0],
            ]]);
            source.kind = kind;
            let mut pack = FeaturePack::empty();
            pack.water.push(source);
            let water = build(&frame, &terrain, Some(&pack), &|_, _| Some(1.0), &color);
            assert!(water.complete);
            assert!(!water.mesh.vertices.is_empty());
            for vertex in &water.mesh.vertices {
                assert_eq!((vertex.face_ao >> 13) & 3, class);
                assert_eq!(vertex.face_ao & SURFACE_NORMAL_FLAG, 0);
                assert_eq!(vertex.face_ao & 7, 2);
                assert_eq!((vertex.face_ao >> 3) & 3, 3);
                assert_eq!(vertex.rgba, [35, 100, 135, 255]);
            }
        }
    }
    #[test]
    fn direct_ocean_is_geodetic_zero_and_shares_render_query_vertices() {
        let (frame, mut terrain) = fixture();
        let water = build(&frame, &mut terrain, None, &|_, _| Some(-10.0), &color);
        assert!(water.complete);
        assert_eq!(water.triangles.len(), N * N * 2);
        for (triangle, indices) in water
            .triangles
            .iter()
            .zip(water.mesh.indices.chunks_exact(3))
        {
            assert_eq!(
                triangle.vertices,
                [0, 1, 2].map(|i| water.mesh.vertices[indices[i] as usize].pos)
            );
            for p in triangle.vertices {
                let (_, offset) =
                    frame.column_surface(f64::from(p[0]) / N as f64, f64::from(p[2]) / N as f64);
                assert!((f64::from(p[1]) * frame.grid_scale.y - offset).abs() < 0.0001);
            }
        }
    }
    #[test]
    fn typed_complete_dry_land_does_not_flood_below_sea_level() {
        let (frame, mut terrain) = fixture();
        let pack = FeaturePack::empty();
        let water = build(
            &frame,
            &mut terrain,
            Some(&pack),
            &|_, _| Some(-10.0),
            &color,
        );
        assert!(water.mesh.indices.is_empty());
        assert_eq!(terrain.heights_m[0], -10.0);
    }
    #[test]
    fn datum_is_continuous_wrapped_and_missing_samples_are_uncacheable() {
        let source = |a: f64, b: f64| Some((a * 20.0 + b.cos() * 2.0) as f32);
        let mut a = Datum {
            source: &source,
            values: BTreeMap::new(),
            complete: true,
        };
        let mut b = Datum {
            source: &source,
            values: BTreeMap::new(),
            complete: true,
        };
        assert!(
            (a.sample(0.2, std::f64::consts::PI).unwrap()
                - b.sample(0.2, -std::f64::consts::PI).unwrap())
            .abs()
                < 1e-7
        );
        let mut bad = Datum {
            source: &|_, _| None,
            values: BTreeMap::new(),
            complete: true,
        };
        assert!(bad.sample(0.2, 0.3).is_none());
        assert!(!bad.complete);
    }
    #[test]
    fn typed_polygon_holes_and_dry_domains_survive_direct_tessellation() {
        use vearth_data::vectors::Water;
        let (frame, mut terrain) = fixture();
        let mut pack = FeaturePack::empty();
        pack.water.push(Water::inland(vec![
            vec![[-90.0, -90.0], [90.0, -90.0], [90.0, 90.0], [-90.0, 90.0]],
            vec![[-15.0, -15.0], [-15.0, 15.0], [15.0, 15.0], [15.0, -15.0]],
        ]));
        let water = build(
            &frame,
            &mut terrain,
            Some(&pack),
            &|_, _| Some(12.25),
            &color,
        );
        assert!(water.complete);
        assert!(!water.triangles.is_empty());
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = crate::tier0::GridProj::of(&frame, lat, lon);
        let hole = glam::DVec2::from_array(projection.to_cols([0.0, 0.0]).map(f64::from));
        for triangle in &water.triangles {
            let mut polygon = triangle
                .vertices
                .map(|p| DVec2::new(f64::from(p[0]), f64::from(p[2])))
                .to_vec();
            polygon.reverse();
            assert!(!inside(&polygon, hole));
            for p in triangle.vertices {
                let (_, offset) =
                    frame.column_surface(f64::from(p[0]) / N as f64, f64::from(p[2]) / N as f64);
                assert!((f64::from(p[1]) * frame.grid_scale.y - offset - 12.25).abs() < 0.0001);
            }
        }
        pack.water[0].kind = WaterRegionKind::Playa;
        assert!(
            build(
                &frame,
                &mut terrain,
                Some(&pack),
                &|_, _| Some(12.25),
                &color
            )
            .triangles
            .is_empty()
        );
    }
    #[test]
    fn coarse_ocean_uses_true_ellipsoid_positions() {
        let frame = ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face: Face(0),
                level: 5,
                i: 16,
                j: 19,
            },
            k: 0,
        });
        let mut terrain = TerrainSurfaceData {
            heights_m: Box::new([-100.0; (N + 1) * (N + 1)]),
            shading: None,
            source_columns: [u32::MAX; N],
        };
        let water = build(&frame, &mut terrain, None, &|_, _| Some(-100.0), &color);
        assert!(water.complete);
        assert_eq!(water.triangles.len(), N * N * 2);
        for triangle in &water.triangles {
            for p in triangle.vertices {
                let world = frame.local_to_ecef(DVec3::from_array(p.map(f64::from)));
                let (_, _, height) = vearth_core::ellipsoid::ecef_to_geodetic(world);
                assert!(height.abs() < frame.grid_scale.max_element() * 5e-6);
            }
        }
    }
    #[test]
    fn coastline_touching_only_a_dry_corner_does_not_admit_an_empty_fan() {
        for level in [14, 17, 19] {
            for (x, z) in [(0, 0), (7, 11), (16, 16), (31, 30)] {
                let frame = ChunkFrame::of(ChunkAddr {
                    cell: CellId {
                        face: Face(0),
                        level,
                        i: (1 << (level - 1)) + 3,
                        j: (1 << (level - 1)) + 1,
                    },
                    k: 0,
                });
                let mut terrain = TerrainSurfaceData {
                    heights_m: Box::new([1.0; (N + 1) * (N + 1)]),
                    shading: None,
                    source_columns: [u32::MAX; N],
                };
                terrain.heights_m[z * (N + 1) + x] = 0.0;
                // Adjacent wet/dry half-plane clips can produce three copies
                // of this point before canonicalization removes its zero area.
                let dry = build(&frame, &terrain, None, &|_, _| Some(1.0), &color);
                assert!(dry.complete);
                assert!(
                    dry.mesh.indices.is_empty(),
                    "a zero-area coast contact is not a water surface"
                );
                terrain.heights_m[z * (N + 1) + x + 1] = -1.0;
                let wet = build(&frame, &terrain, None, &|_, _| Some(1.0), &color);
                assert!(wet.complete);
                assert!(
                    !wet.triangles.is_empty(),
                    "a neighboring nonzero wet footprint must survive"
                );
                for triangle in wet.triangles {
                    let p = triangle
                        .vertices
                        .map(|p| DVec3::from_array(p.map(f64::from)));
                    assert!(p.iter().all(|p| p.is_finite()));
                    assert!((p[1] - p[0]).cross(p[2] - p[0]).y > 0.0);
                }
            }
        }
    }

    #[test]
    fn translated_overlapping_typed_regions_discard_fully_clipped_pieces_and_keep_dry_holes() {
        use vearth_data::vectors::Water;
        for level in [14, 17, 19] {
            for shift in [0.0, 0.000001, 0.237, 0.999999] {
                let frame = ChunkFrame::of(ChunkAddr {
                    cell: CellId {
                        face: Face(0),
                        level,
                        i: (1 << (level - 1)) + 2,
                        j: (1 << (level - 1)) + 4,
                    },
                    k: 0,
                });
                let terrain = TerrainSurfaceData {
                    heights_m: Box::new([10.0; (N + 1) * (N + 1)]),
                    shading: None,
                    source_columns: [u32::MAX; N],
                };
                let (lat, lon) = frame.column_latlon(0.5, 0.5);
                let projection = crate::tier0::GridProj::of(&frame, lat, lon);
                let q = |p| DVec2::from_array(projection.to_cols(p).map(f64::from));
                let origin = q([0.0, 0.0]);
                let east = q([100.0, 0.0]) - origin;
                let north = q([0.0, 100.0]) - origin;
                let to_en = |x: f64, z: f64| {
                    let p = DVec2::new(x, z) - origin;
                    let det = east.perp_dot(north);
                    [
                        (p.perp_dot(north) * 100.0 / det) as f32,
                        (east.perp_dot(p) * 100.0 / det) as f32,
                    ]
                };
                let (x, z) = (12.0 + shift, 14.0 + shift);
                let rect = |a: f64, b: f64, c: f64, d: f64| {
                    vec![
                        to_en(x + a, z + b),
                        to_en(x + c, z + b),
                        to_en(x + c, z + d),
                        to_en(x + a, z + d),
                    ]
                };
                let mut pack = FeaturePack::empty();
                pack.water
                    .push(Water::ocean(vec![rect(-0.3, -0.2, 1.5, 3.2)]));
                pack.water.push(Water::inland(vec![
                    rect(0.0, 0.0, 3.0, 3.0),
                    rect(0.8, 0.8, 2.2, 2.2),
                ]));
                let baseline = build(&frame, &terrain, Some(&pack), &|_, _| Some(12.0), &color);
                assert!(baseline.complete);
                // This valid source polygon is completely covered by the
                // higher-priority ocean; subtraction must contribute no fan.
                pack.water
                    .push(Water::inland(vec![rect(-0.1, 0.1, 1.3, 2.9)]));
                let combined = build(&frame, &terrain, Some(&pack), &|_, _| Some(12.0), &color);
                assert!(combined.complete);
                assert_eq!(combined.triangles, baseline.triangles);
                let cover = |point: DVec2| {
                    combined.triangles.iter().any(|triangle| {
                        let polygon = crate::road_surface::canonical(
                            triangle
                                .vertices
                                .iter()
                                .map(|p| DVec2::new(f64::from(p[0]), f64::from(p[2])))
                                .collect(),
                        );
                        polygon.len() >= 3 && inside(&polygon, point)
                    })
                };
                assert!(
                    !cover(DVec2::new(x + 1.8, z + 1.5)),
                    "the dry lake hole outside the ocean stays open"
                );
                assert!(cover(DVec2::new(x + 0.5, z + 1.5)));
                assert!(cover(DVec2::new(x + 2.6, z + 1.5)));
                assert!(!cover(DVec2::new(x + 3.4, z + 1.5)));
            }
        }
    }
    struct WaterComponentFixture {
        kind: WaterRegionKind,
        original: Option<vearth_data::vectors::Rings>,
        groups: Vec<vearth_data::vectors::Rings>,
    }

    fn component_pack(sources: &[WaterComponentFixture], separated: bool) -> FeaturePack {
        let mut pack = FeaturePack::empty();
        for source in sources {
            let rings = if separated {
                source.groups.clone()
            } else {
                vec![
                    source
                        .original
                        .clone()
                        .unwrap_or_else(|| source.groups.iter().flatten().cloned().collect()),
                ]
            };
            for rings in rings {
                let mut water = vearth_data::vectors::Water::inland(rings);
                water.kind = source.kind;
                pack.water.push(water);
            }
        }
        pack
    }

    fn assert_component_water_classes(a: &WaterGeometry, b: &WaterGeometry) {
        let areas = |water: &WaterGeometry| {
            let mut areas = BTreeMap::<WaterRegionKind, f64>::new();
            for triangle in &water.triangles {
                let p = triangle
                    .vertices
                    .map(|p| DVec2::new(f64::from(p[0]), f64::from(p[2])));
                *areas.entry(triangle.kind).or_default() +=
                    (p[1] - p[0]).perp_dot(p[2] - p[0]).abs() * 0.5;
            }
            areas
        };
        let a = areas(a);
        let b = areas(b);
        assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
        for (kind, area) in a {
            assert!(
                (area - b[&kind]).abs() < area.max(1.0) * 1.0e-5,
                "typed ownership changed: {kind:?}"
            );
        }
    }

    struct ComponentMeshProbe<'a> {
        mesh: &'a MeshBuffers,
        bins: Vec<Vec<usize>>,
        area: f64,
    }
    impl<'a> ComponentMeshProbe<'a> {
        fn new(mesh: &'a MeshBuffers) -> Self {
            let mut bins = vec![Vec::new(); N * N];
            let mut area = 0.0;
            for (index, ids) in mesh.indices.chunks_exact(3).enumerate() {
                let p = ids.map_vertices(mesh);
                let determinant = (p[1] - p[0]).cross(p[2] - p[0]).y;
                if determinant <= 1.0e-12 {
                    continue;
                }
                area += determinant * 0.5;
                let lo = p[0].min(p[1]).min(p[2]);
                let hi = p[0].max(p[1]).max(p[2]);
                let column = |v: f64| v.floor().clamp(0.0, (N - 1) as f64) as usize;
                for z in column(lo.z)..=column(hi.z) {
                    for x in column(lo.x)..=column(hi.x) {
                        bins[z * N + x].push(index);
                    }
                }
            }
            Self { mesh, bins, area }
        }
        fn at(&self, p: DVec2) -> Option<(f64, [f64; 3])> {
            let x = p.x.floor().clamp(0.0, (N - 1) as f64) as usize;
            let z = p.y.floor().clamp(0.0, (N - 1) as f64) as usize;
            self.bins[z * N + x].iter().find_map(|&index| {
                let ids = &self.mesh.indices[index * 3..index * 3 + 3];
                let v = ids.map_vertices(self.mesh);
                let xz = |p: DVec3| DVec2::new(p.x, p.z);
                let a = xz(v[1] - v[0]);
                let b = xz(v[2] - v[0]);
                let delta = p - xz(v[0]);
                let det = a.perp_dot(b);
                let u = delta.perp_dot(b) / det;
                let w = a.perp_dot(delta) / det;
                let weights = [1.0 - u - w, u, w];
                if weights.iter().any(|&weight| weight < -1.0e-9) {
                    return None;
                }
                Some((
                    (0..3).map(|i| weights[i] * v[i].y).sum(),
                    std::array::from_fn(|channel| {
                        (0..3)
                            .map(|i| {
                                weights[i]
                                    * f64::from(self.mesh.vertices[ids[i] as usize].rgba[channel])
                            })
                            .sum()
                    }),
                ))
            })
        }
    }

    trait ComponentTrianglePoints {
        fn map_vertices(&self, mesh: &MeshBuffers) -> [DVec3; 3];
    }
    impl ComponentTrianglePoints for [u32] {
        fn map_vertices(&self, mesh: &MeshBuffers) -> [DVec3; 3] {
            std::array::from_fn(|i| {
                DVec3::from_array(mesh.vertices[self[i] as usize].pos.map(f64::from))
            })
        }
    }

    fn compare_component_meshes(
        frame: &ChunkFrame,
        a: &MeshBuffers,
        b: &MeshBuffers,
        count: usize,
    ) -> (usize, f64, f64) {
        let a = ComponentMeshProbe::new(a);
        let b = ComponentMeshProbe::new(b);
        assert!(
            (a.area - b.area).abs() < a.area.max(1.0) * 1.0e-5,
            "source coverage changed: {} -> {}",
            a.area,
            b.area
        );
        let (mut sampled, mut height_error, mut color_error) = (0, 0.0_f64, 0.0_f64);
        for i in 0..count {
            let p = DVec2::new(
                ((i * 719) % 10007) as f64 / 10007.0,
                ((i * 313) % 9973) as f64 / 9973.0,
            ) * N as f64;
            match (a.at(p), b.at(p)) {
                (Some((ha, ca)), Some((hb, cb))) => {
                    sampled += 1;
                    height_error = height_error.max((ha - hb).abs() * frame.grid_scale.y);
                    for channel in 0..3 {
                        color_error = color_error.max((ca[channel] - cb[channel]).abs());
                    }
                }
                (None, None) => {}
                _ => panic!("source coverage changed at {p:?}"),
            }
        }
        (sampled, height_error, color_error)
    }

    fn compare_component_boundaries(
        frame: &ChunkFrame,
        a: &MeshBuffers,
        b: &MeshBuffers,
    ) -> (usize, f64) {
        let a = ComponentMeshProbe::new(a);
        let b = ComponentMeshProbe::new(b);
        let mut wet = 0;
        let mut error = 0.0_f64;
        for i in 0..256 {
            let t = (i as f64 + 0.37) * N as f64 / 256.0;
            for p in [
                DVec2::new(0.0, t),
                DVec2::new(N as f64, t),
                DVec2::new(t, 0.0),
                DVec2::new(t, N as f64),
            ] {
                match (a.at(p), b.at(p)) {
                    (Some((ha, _)), Some((hb, _))) => {
                        wet += 1;
                        error = error.max((ha - hb).abs() * frame.grid_scale.y);
                    }
                    (None, None) => {}
                    _ => panic!("wet cell-edge coverage changed at {p:?}"),
                }
            }
        }
        (wet, error)
    }

    fn component_en(frame: &ChunkFrame, p: [f64; 2]) -> [f32; 2] {
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = crate::tier0::GridProj::of(frame, lat, lon);
        let q = |p| DVec2::from_array(projection.to_cols(p).map(f64::from));
        let origin = q([0.0, 0.0]);
        let a = (q([100.0, 0.0]) - origin) / 100.0;
        let b = (q([0.0, 100.0]) - origin) / 100.0;
        let delta = DVec2::from_array(p) - origin;
        let det = a.perp_dot(b);
        [
            (delta.perp_dot(b) / det) as f32,
            (a.perp_dot(delta) / det) as f32,
        ]
    }

    #[test]
    fn water_components_corner_and_edge_contacts_are_complete_empty_domains() {
        use crate::building_surface::source_polygon_cells_clipped;
        let inside = vec![[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]];
        let clip = |rings: &vearth_data::vectors::Rings| {
            source_polygon_cells_clipped(rings, DVec2::ZERO, DVec2::splat(N as f64), 1.0e9)
        };
        let expected = clip(&vec![inside.clone()]).unwrap();
        for outside in [
            vec![[-1.0, -1.0], [-1.0, 1.0], [0.0, 0.0]],
            vec![[-1.0, 2.0], [0.0, 2.0], [0.0, 4.0], [-1.0, 4.0]],
            vec![[-2.0, -2.0], [-1.0, -2.0], [-1.0, -1.0], [-2.0, -1.0]],
        ] {
            assert!(
                clip(&vec![outside.clone()]).is_some_and(|cells| cells.is_empty()),
                "a source contact cannot create an incomplete cell"
            );
            let compound = clip(&vec![inside.clone(), outside]).unwrap();
            let area = |cells: &Vec<[DVec2; 4]>| {
                cells
                    .iter()
                    .map(|p| {
                        (0..4)
                            .map(|i| p[i].perp_dot(p[(i + 1) % 4]))
                            .sum::<f64>()
                            .abs()
                            * 0.5
                    })
                    .sum::<f64>()
            };
            assert!((area(&compound) - area(&expected)).abs() < 1.0e-10);
        }
        assert!(
            clip(&vec![vec![[2.0, 2.0]; 3]]).is_none(),
            "invalid interior source cannot become a complete empty component"
        );
        let narrow = vec![vec![[0.0, 0.0], [0.01, 0.0], [0.0, 0.01]]];
        assert!(
            clip(&narrow).is_some_and(|cells| !cells.is_empty()),
            "nonzero boundary-adjacent source must survive"
        );
        assert!(clip(&vec![vec![[0.0, 0.0], [1.0, 1.0]]]).is_none());
        assert!(clip(&vec![vec![[0.0, 0.0], [1.0, f32::NAN], [2.0, 0.0]]]).is_none());
    }

    #[test]
    fn water_components_preserve_disconnected_holes_typed_priority_and_datum() {
        for level in [14, 17] {
            let frame = ChunkFrame::of(ChunkAddr {
                cell: CellId {
                    face: Face(0),
                    level,
                    i: (1 << (level - 1)) + 3,
                    j: (1 << (level - 1)) + 2,
                },
                k: 0,
            });
            let terrain = TerrainSurfaceData {
                heights_m: Box::new([35.0; (N + 1) * (N + 1)]),
                shading: None,
                source_columns: [u32::MAX; N],
            };
            let rect = |x: f64, z: f64, w: f64, h: f64| {
                [[x, z], [x + w, z], [x + w, z + h], [x, z + h]]
                    .map(|p| component_en(&frame, p))
                    .to_vec()
            };
            let mut sources = vec![WaterComponentFixture {
                kind: WaterRegionKind::Lake,
                original: None,
                groups: Vec::new(),
            }];
            let mut checks = Vec::new();
            for i in 0..24 {
                let x = 0.7 + (i % 6) as f64 * 5.1;
                let z = 0.6 + (i / 6) as f64 * 7.4 + i as f64 * 0.019;
                let mut rings = vec![rect(x, z, 3.4, 4.7)];
                if i % 3 == 0 {
                    rings.push(rect(x + 1.1, z + 1.2, 0.8, 1.4));
                    checks.push((DVec2::new(x + 1.5, z + 1.9), false));
                }
                checks.push((DVec2::new(x + 0.5, z + 0.5), true));
                sources[0].groups.push(rings);
            }
            // Source clipping and an independently typed lower-priority overlap.
            sources[0].groups.push(vec![rect(-1.2, 30.1, 4.0, 3.0)]);
            sources.push(WaterComponentFixture {
                kind: WaterRegionKind::Canal,
                original: None,
                groups: vec![
                    vec![rect(0.8, 0.8, 0.5, 0.5)],
                    vec![rect(5.0, 5.0, 0.4, 8.0)],
                ],
            });
            let old = component_pack(&sources, false);
            let new = component_pack(&sources, true);
            let a = build_with_policy::<false>(
                &frame,
                &terrain,
                Some(&old),
                &|_, _| Some(30.0),
                &color,
            );
            let b = build(&frame, &terrain, Some(&new), &|_, _| Some(30.0), &color);
            assert!(a.complete && b.complete);
            assert_component_water_classes(&a, &b);
            let (samples, error, color_error) =
                compare_component_meshes(&frame, &a.mesh, &b.mesh, 4096);
            assert!(samples > 1000);
            assert!(error < 0.02, "constant-datum interpolation error {error}m");
            assert!(color_error < 1.0e-7);
            let (wet_boundary, boundary_error) =
                compare_component_boundaries(&frame, &a.mesh, &b.mesh);
            assert!(
                wet_boundary > 20,
                "crossing lake must exercise the cell boundary"
            );
            assert!(
                boundary_error < 0.02,
                "constant datum boundary changed {boundary_error}m"
            );
            let probe = ComponentMeshProbe::new(&b.mesh);
            for (point, wet) in checks {
                assert_eq!(probe.at(point).is_some(), wet, "hole/patch at {point:?}");
            }
            assert!(
                b.triangles.len() < a.triangles.len(),
                "component partition should remove remote slab cuts"
            );
            let appearances = [[SurfaceAppearance {
                material: material::GRASS,
                tint: [70, 105, 61],
            }; N]; N];
            let ta = crate::terrain_surface::build_with_water(
                &frame,
                &terrain,
                &appearances,
                &a.triangles,
            );
            let tb = crate::terrain_surface::build_with_water(
                &frame,
                &terrain,
                &appearances,
                &b.triangles,
            );
            let (samples, error, _) = compare_component_meshes(&frame, &ta.mesh, &tb.mesh, 4096);
            assert_eq!(samples, 4096);
            assert!(error < 0.02, "constant bed difference {error}m");
            assert!(tb.mesh.indices.len() < ta.mesh.indices.len());
        }
    }

    #[test]
    fn coalesced_water_slabs_preserve_connected_lake_holes_and_boundary_datum() {
        for level in [14, 17] {
            let frame = ChunkFrame::of(ChunkAddr {
                cell: CellId {
                    face: Face(0),
                    level,
                    i: (1 << (level - 1)) + 3,
                    j: (1 << (level - 1)) + 2,
                },
                k: 0,
            });
            let terrain = TerrainSurfaceData {
                heights_m: Box::new([35.0; (N + 1) * (N + 1)]),
                shading: None,
                source_columns: [u32::MAX; N],
            };
            let rect = |x: f64, z: f64, w: f64, h: f64| {
                [[x, z], [x + w, z], [x + w, z + h], [x, z + h]]
                    .map(|p| component_en(&frame, p))
                    .to_vec()
            };
            let mut rings = vec![rect(-2.0, -2.0, 36.0, 36.0)];
            let mut island_centers = Vec::new();
            for i in 0..24 {
                let x = 1.8 + (i % 6) as f64 * 5.0;
                let z = 1.4 + (i / 6) as f64 * 7.1 + i as f64 * 0.027;
                rings.push(rect(x, z, 2.1, 2.8));
                island_centers.push(DVec2::new(x + 1.05, z + 1.4));
            }
            let pack = component_pack(
                &[WaterComponentFixture {
                    kind: WaterRegionKind::Lake,
                    original: None,
                    groups: vec![rings],
                }],
                true,
            );
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            for varying in [false, true] {
                let elev = |a: f64, b: f64| {
                    Some(if varying {
                        (30.0
                            + ((a - lat) * MEAN_RADIUS / 250.0).sin() * 0.3
                            + ((b - lon) * MEAN_RADIUS * lat.cos() / 250.0).sin() * 0.3)
                            as f32
                    } else {
                        30.0
                    })
                };
                let old = build_with_policy::<false>(&frame, &terrain, Some(&pack), &elev, &color);
                let new = build(&frame, &terrain, Some(&pack), &elev, &color);
                assert!(old.complete && new.complete);
                assert_component_water_classes(&old, &new);
                let (samples, error, color_error) =
                    compare_component_meshes(&frame, &old.mesh, &new.mesh, 4096);
                assert!(samples > 3000);
                assert!(error < 0.02, "L{level} datum varying={varying}: {error}m");
                assert!(color_error < 1.0e-7);
                let (wet, error) = compare_component_boundaries(&frame, &old.mesh, &new.mesh);
                assert_eq!(wet, 1024, "the connected lake crosses every cell edge");
                assert!(
                    error < 0.02,
                    "L{level} boundary datum varying={varying}: {error}m"
                );
                let probe = ComponentMeshProbe::new(&new.mesh);
                for &center in &island_centers {
                    assert!(probe.at(center).is_none());
                }
                assert!(new.triangles.len() < old.triangles.len());
            }
        }
    }

    #[test]
    #[ignore = "actual cached source comparison; requires VEARTH_WATER_COMPONENT_FIXTURE"]
    fn water_components_actual_source_geometry_benchmark() {
        use vearth_platform::time::Instant;
        let text = std::fs::read_to_string(
            vearth_platform::env::var("VEARTH_WATER_COMPONENT_FIXTURE").expect("fixture path"),
        )
        .unwrap();
        let kinds = [
            WaterRegionKind::Ocean,
            WaterRegionKind::InlandWater,
            WaterRegionKind::Lake,
            WaterRegionKind::Reservoir,
            WaterRegionKind::River,
            WaterRegionKind::Canal,
            WaterRegionKind::Stream,
            WaterRegionKind::Drainage,
            WaterRegionKind::Basin,
            WaterRegionKind::Dock,
            WaterRegionKind::Decorative,
            WaterRegionKind::Playa,
            WaterRegionKind::Reef,
            WaterRegionKind::Other,
            WaterRegionKind::CompatibilityUnknown,
        ];
        let mut sources = Vec::<WaterComponentFixture>::new();
        let mut original = false;
        for line in text
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let words: Vec<_> = line.split_whitespace().collect();
            match words[0] {
                "source" => {
                    original = false;
                    sources.push(WaterComponentFixture {
                        kind: kinds[words[2].parse::<usize>().unwrap()],
                        original: None,
                        groups: Vec::new(),
                    });
                }
                "original" => {
                    original = true;
                    sources.last_mut().unwrap().original = Some(Vec::new());
                }
                "component" => {
                    original = false;
                    sources.last_mut().unwrap().groups.push(Vec::new());
                }
                _ => {
                    let source = sources.last_mut().unwrap();
                    let rings = if original {
                        source.original.as_mut().unwrap()
                    } else {
                        source.groups.last_mut().unwrap()
                    };
                    if words[0] == "ring" {
                        rings.push(Vec::new());
                    } else {
                        rings
                            .last_mut()
                            .unwrap()
                            .push([words[0].parse().unwrap(), words[1].parse().unwrap()]);
                    }
                }
            }
        }

        let frame = ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face: Face(4),
                level: 12,
                i: 3770,
                j: 2228,
            },
            k: 0,
        });
        let terrain = TerrainSurfaceData {
            heights_m: Box::new([35.0; (N + 1) * (N + 1)]),
            shading: None,
            source_columns: [u32::MAX; N],
        };
        let appearances = [[SurfaceAppearance {
            material: material::GRASS,
            tint: [70, 105, 61],
        }; N]; N];
        let old = component_pack(&sources, true);
        let new = component_pack(&sources, true);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        for varying in [false, true] {
            let elevation = |a: f64, b: f64| {
                Some(if varying {
                    (30.0
                        + ((a - lat) * MEAN_RADIUS / 250.0).sin() * 0.3
                        + ((b - lon) * MEAN_RADIUS * lat.cos() / 250.0).sin() * 0.3)
                        as f32
                } else {
                    30.0
                })
            };
            let started = Instant::now();
            let a = build_with_policy::<false>(&frame, &terrain, Some(&old), &elevation, &color);
            let old_water_ms = started.elapsed().as_secs_f64() * 1000.0;
            let started = Instant::now();
            let b = build(&frame, &terrain, Some(&new), &elevation, &color);
            let new_water_ms = started.elapsed().as_secs_f64() * 1000.0;
            assert!(a.complete && b.complete);
            assert_component_water_classes(&a, &b);
            let water_diff = compare_component_meshes(&frame, &a.mesh, &b.mesh, 8192);
            let boundary_diff = compare_component_boundaries(&frame, &a.mesh, &b.mesh);
            let started = Instant::now();
            let ta = crate::terrain_surface::build_with_water(
                &frame,
                &terrain,
                &appearances,
                &a.triangles,
            );
            let old_terrain_ms = started.elapsed().as_secs_f64() * 1000.0;
            let started = Instant::now();
            let tb = crate::terrain_surface::build_with_water(
                &frame,
                &terrain,
                &appearances,
                &b.triangles,
            );
            let new_terrain_ms = started.elapsed().as_secs_f64() * 1000.0;
            let bed_diff = compare_component_meshes(&frame, &ta.mesh, &tb.mesh, 8192);
            eprintln!(
                "actual cached grouped source, C14 slabs -> coalesced slabs, constant35m terrain, varying_datum={varying}: features={}->{}, water_tris={}->{}, terrain_tris={}->{}, water_ms={old_water_ms:.3}->{new_water_ms:.3}, terrain_ms={old_terrain_ms:.3}->{new_terrain_ms:.3}, water_samples/error_m/color={water_diff:?}, boundary_samples/error_m={boundary_diff:?}, bed_samples/error_m/color={bed_diff:?}",
                old.water.len(),
                new.water.len(),
                a.triangles.len(),
                b.triangles.len(),
                ta.mesh.indices.len() / 3,
                tb.mesh.indices.len() / 3
            );
            assert!(water_diff.0 > 0);
            assert_eq!(bed_diff.0, 8192);
        }
    }
}
