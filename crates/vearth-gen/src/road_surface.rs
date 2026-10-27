//! Metric road ribbons clipped to the existing graded terrain triangles.

use crate::{N, SurfaceAppearance, TerrainSurfaceData, tier0};
use glam::{DVec2, DVec3};
use vearth_core::{
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal},
};
use vearth_data::vectors::{FeaturePack, RoadClass};

#[cfg(test)]
fn road_appearance(class: RoadClass, jitter: f32, scale: f32) -> SurfaceAppearance {
    let color = crate::road_appearance(class, jitter, scale);
    SurfaceAppearance {
        material: color.material,
        tint: color.tint,
    }
}

const MAX_PRIMITIVES: usize = 8_192;
const MAX_PIECES_PER_TRIANGLE: usize = 256;
const MAX_TRIANGLES: usize = 196_608;
const POLYGON_EPS: f64 = 1.0e-10;
const ROAD_LIFT_M: f64 = 0.035;
const RAIL_LIFT_M: f64 = 0.09;
const RAIL_OFFSET_M: f64 = 1.435 * 0.5;
const RAIL_HALF_WIDTH_M: f64 = 0.075;
/// Roads narrower than this fraction of the terrain sample pitch are omitted.
/// Selection shows a sample at roughly 1.5-4.5 px, so an omitted ribbon is at
/// most about half a pixel wide. Its clipped pieces would only add sub-pixel
/// triangles, which multiply fragment shading under MSAA. Every mapped way
/// remains from L15. L14 omits 1.8 m footways and sidewalks, L13 also
/// service ways, alleys and paths, L12 keeps major roads and L11 highways.
const MIN_ROAD_WIDTH_SAMPLE_FRACTION: f64 = 0.12;

pub(crate) struct RoadSurfaceData {
    /// Positions are in the cell's k=0 frame, before chunk rebasing.
    pub mesh: MeshBuffers,
    pub complete: bool,
}

struct Primitive {
    polygon: Vec<DVec2>,
    appearance: SurfaceAppearance,
    priority: u8,
    lift_m: f64,
}

/// Surface eligibility resolves mapped water, structure and curated ownership
/// before the metric ribbons are clipped to the terrain planes.
pub(crate) fn build(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    terrain: &TerrainSurfaceData,
    exclusions: &[Vec<DVec2>],
    water: &[crate::water_surface::WaterTriangle],
) -> Option<RoadSurfaceData> {
    build_bounded(
        frame,
        pack,
        terrain,
        exclusions,
        water,
        crate::built_surface::ROAD_BASE_BUDGET_BYTES,
        MAX_PIECES_PER_TRIANGLE,
    )
}

fn build_bounded(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    terrain: &TerrainSurfaceData,
    exclusions: &[Vec<DVec2>],
    water: &[crate::water_surface::WaterTriangle],
    budget: usize,
    piece_limit: usize,
) -> Option<RoadSurfaceData> {
    build_bounded_with_reuse::<true>(frame, pack, terrain, exclusions, water, budget, piece_limit)
}

fn build_bounded_with_reuse<const REUSE: bool>(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    terrain: &TerrainSurfaceData,
    exclusions: &[Vec<DVec2>],
    water: &[crate::water_surface::WaterTriangle],
    budget: usize,
    piece_limit: usize,
) -> Option<RoadSurfaceData> {
    let mut result =
        build_ribbons::<REUSE>(frame, pack, terrain, exclusions, water, false, piece_limit)?;
    if crate::built_surface::mesh_bytes(&result.mesh) > budget {
        crate::built_surface::compact_mesh(&mut result.mesh);
    }
    if crate::built_surface::mesh_bytes(&result.mesh) > budget {
        result =
            build_ribbons::<REUSE>(frame, pack, terrain, exclusions, water, true, piece_limit)?;
        crate::built_surface::compact_mesh(&mut result.mesh);
    }
    result.complete &= crate::built_surface::mesh_bytes(&result.mesh) <= 128 * 1024 * 1024;
    Some(result)
}

fn build_ribbons<const REUSE: bool>(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    terrain: &TerrainSurfaceData,
    exclusions: &[Vec<DVec2>],
    water: &[crate::water_surface::WaterTriangle],
    simple: bool,
    piece_limit: usize,
) -> Option<RoadSurfaceData> {
    if frame.addr.cell.level < tier0::ROADS_MIN_LEVEL || pack.roads.is_empty() {
        return None;
    }
    let (lat, lon) = frame.column_latlon(0.5, 0.5);
    let projection = tier0::GridProj::of(frame, lat, lon);
    let mut complete = true;
    let mut primitives = Vec::new();
    let materials = crate::road_materials::Resolver::new(frame, pack);
    let min_width_m = MIN_ROAD_WIDTH_SAMPLE_FRACTION * frame.addr.cell.sample_pitch_m();
    for road in &pack.roads {
        if road.is_bridge || road.pts.len() < 2 {
            continue;
        }
        if road.class != RoadClass::Rail && f64::from(road.width_m()) < min_width_m {
            continue;
        }
        let points: Vec<_> = road
            .pts
            .iter()
            .map(|&point| DVec2::new(f64::from(point[0]), f64::from(point[1])))
            .collect();
        if points.iter().any(|point| !point.is_finite()) {
            complete = false;
            continue;
        }
        let mut points = points;
        points.dedup_by(|a, b| a.distance_squared(*b) < 1.0e-8);
        if points.len() < 2 {
            continue;
        }
        let appearance = materials.appearance(road);
        if road.class == RoadClass::Rail {
            for offset in [-RAIL_OFFSET_M, RAIL_OFFSET_M] {
                let track = offset_polyline(&points, offset);
                stroke(
                    &track,
                    RAIL_HALF_WIDTH_M,
                    appearance,
                    7,
                    RAIL_LIFT_M,
                    &projection,
                    &mut primitives,
                    simple,
                );
            }
        } else {
            stroke(
                &points,
                f64::from(road.width_m()) * 0.5,
                appearance,
                priority(road.class),
                ROAD_LIFT_M,
                &projection,
                &mut primitives,
                simple,
            );
        }
        if primitives.len() > MAX_PRIMITIVES {
            return Some(RoadSurfaceData {
                mesh: MeshBuffers::default(),
                complete: false,
            });
        }
    }
    if primitives.is_empty() {
        return (!complete).then_some(RoadSurfaceData {
            mesh: MeshBuffers::default(),
            complete,
        });
    }
    // Union ownership is independent of source feature order and duplicate
    // MVT fragments. Rails remain visible through at-grade road crossings.
    primitives.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.appearance.material.cmp(&b.appearance.material))
            .then(a.appearance.tint.cmp(&b.appearance.tint))
            .then_with(|| polygon_cmp(&a.polygon, &b.polygon))
    });
    primitives.dedup_by(|a, b| {
        a.priority == b.priority && a.appearance == b.appearance && a.polygon == b.polygon
    });

    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); N * N];
    for (index, primitive) in primitives.iter().enumerate() {
        let (lo, hi) = bounds(&primitive.polygon);
        let x0 = lo.x.floor().max(0.0) as usize;
        let z0 = lo.y.floor().max(0.0) as usize;
        let x1 = (hi.x.ceil() as usize).min(N);
        let z1 = (hi.y.ceil() as usize).min(N);
        for z in z0..z1 {
            for x in x0..x1 {
                buckets[z * N + x].push(index);
            }
        }
    }
    let mut blockers = exclusions.to_vec();
    blockers.extend(water.iter().map(|triangle| {
        canonical(
            triangle
                .vertices
                .iter()
                .map(|p| DVec2::new(f64::from(p[0]), f64::from(p[2])))
                .collect(),
        )
    }));

    for rings in pack
        .buildings
        .iter()
        .filter(|b| b.min_height_m <= 0.5)
        .map(|b| &b.rings)
    {
        let columns: vearth_data::vectors::Rings = rings
            .iter()
            .map(|ring| ring.iter().map(|&p| projection.to_cols(p)).collect())
            .collect();
        if let Some(cells) = crate::building_surface::source_polygon_cells_clipped(
            &columns,
            DVec2::ZERO,
            DVec2::splat(N as f64),
            1e9,
        ) {
            blockers.extend(
                cells
                    .into_iter()
                    .map(|cell| canonical(cell.into_iter().collect()))
                    .filter(|p| p.len() >= 3),
            );
        }
    }
    let mut blocked_buckets = vec![Vec::<usize>::new(); N * N];
    for (index, polygon) in blockers.iter().enumerate() {
        let (lo, hi) = bounds(polygon);
        let x0 = (lo.x.floor().max(0.0) as usize).min(N);
        let z0 = (lo.y.floor().max(0.0) as usize).min(N);
        let x1 = (hi.x.ceil().max(0.0) as usize).min(N);
        let z1 = (hi.y.ceil().max(0.0) as usize).min(N);
        for z in z0..z1 {
            for x in x0..x1 {
                blocked_buckets[z * N + x].push(index);
            }
        }
    }
    let mut heights = [0.0_f64; (N + 1) * (N + 1)];
    for z in 0..=N {
        for x in 0..=N {
            let (_, offset) = frame.column_surface(x as f64 / N as f64, z as f64 / N as f64);
            let height =
                (f64::from(terrain.heights_m[z * (N + 1) + x]) + offset) / frame.grid_scale.y;
            if !height.is_finite() {
                return Some(RoadSurfaceData {
                    mesh: MeshBuffers::default(),
                    complete: false,
                });
            }
            heights[z * (N + 1) + x] = f64::from(height as f32);
        }
    }
    let mut mesh = MeshBuffers::default();
    let mut subtraction = SubtractionScratch::default();
    for z in 0..N {
        for x in 0..N {
            let bucket = &buckets[z * N + x];
            if bucket.is_empty() {
                continue;
            }
            let p = [
                DVec2::new(x as f64, z as f64),
                DVec2::new(x as f64 + 1.0, z as f64),
                DVec2::new(x as f64 + 1.0, z as f64 + 1.0),
                DVec2::new(x as f64, z as f64 + 1.0),
            ];
            let h = [
                heights[z * (N + 1) + x],
                heights[z * (N + 1) + x + 1],
                heights[(z + 1) * (N + 1) + x + 1],
                heights[(z + 1) * (N + 1) + x],
            ];
            for ids in [[0, 1, 2], [0, 2, 3]] {
                let triangle = ids.map(|i| p[i]);
                let elevation = ids.map(|i| h[i]);
                let vertex_start = mesh.vertices.len();
                let index_start = mesh.indices.len();
                let blocked: Vec<Vec<DVec2>> = blocked_buckets[z * N + x]
                    .iter()
                    .map(|&i| intersection(&blockers[i], &triangle))
                    .filter(|p| p.len() >= 3)
                    .collect();
                let mut accepted = blocked.clone();
                let mut fallback = simple;
                'union: for &index in bucket {
                    if fallback {
                        break;
                    }
                    let primitive = &primitives[index];
                    let clipped = intersection(&primitive.polygon, &triangle);
                    if clipped.len() < 3 {
                        continue;
                    }
                    let mut remaining = vec![clipped];
                    for covered in &accepted {
                        subtraction.subtract::<REUSE>(&mut remaining, covered);
                        if remaining.len() + accepted.len() > piece_limit {
                            fallback = true;
                            break 'union;
                        }
                        if remaining.is_empty() {
                            break;
                        }
                    }
                    for polygon in remaining {
                        emit(
                            &mut mesh,
                            &polygon,
                            triangle,
                            elevation,
                            primitive.appearance,
                            primitive.lift_m / frame.grid_scale.y,
                        );
                        accepted.push(polygon);
                    }
                    if mesh.indices.len() / 3 > MAX_TRIANGLES || accepted.len() > piece_limit {
                        fallback = true;
                        break;
                    }
                }
                if fallback {
                    mesh.vertices.truncate(vertex_start);
                    mesh.indices.truncate(index_start);
                    // Emergency topology is the original metric ribbons on the
                    // very same terrain planes. Class ordering gets a fixed
                    // sub-centimetre lift instead of an expensive planar union.
                    for &index in bucket.iter().rev() {
                        let primitive = &primitives[index];
                        let polygon = intersection(&primitive.polygon, &triangle);
                        let mut remaining = vec![polygon];
                        for cover in &blocked {
                            subtraction.subtract::<REUSE>(&mut remaining, cover);
                        }
                        for polygon in remaining.into_iter().filter(|p| p.len() >= 3) {
                            emit(
                                &mut mesh,
                                &polygon,
                                triangle,
                                elevation,
                                primitive.appearance,
                                (primitive.lift_m + f64::from(primitive.priority) * 0.001)
                                    / frame.grid_scale.y,
                            );
                        }
                    }
                }
            }
        }
    }
    (!mesh.indices.is_empty() || !complete).then_some(RoadSurfaceData { mesh, complete })
}

fn priority(class: RoadClass) -> u8 {
    match class {
        RoadClass::Highway => 6,
        RoadClass::Major => 5,
        RoadClass::Medium => 4,
        RoadClass::Minor => 3,
        RoadClass::Service => 2,
        RoadClass::Rail => 1,
        RoadClass::Path => 0,
    }
}

pub(crate) fn offset_polyline(points: &[DVec2], offset: f64) -> Vec<DVec2> {
    let normals: Vec<_> = points
        .windows(2)
        .map(|pair| {
            let direction = (pair[1] - pair[0]).normalize();
            DVec2::new(-direction.y, direction.x)
        })
        .collect();
    let closed = points.len() > 3 && points[0].distance_squared(*points.last().unwrap()) < 1.0e-8;
    points
        .iter()
        .enumerate()
        .map(|(index, &point)| {
            if closed && (index == 0 || index == points.len() - 1) {
                let previous = *normals.last().unwrap();
                let bisector = previous + normals[0];
                let denominator = bisector.dot(normals[0]);
                point
                    + if denominator > 0.25 {
                        bisector * (offset / denominator)
                    } else {
                        normals[0] * offset
                    }
            } else if index == 0 {
                point + normals[0] * offset
            } else if index == points.len() - 1 {
                point + normals[index - 1] * offset
            } else {
                let bisector = normals[index - 1] + normals[index];
                let denominator = bisector.dot(normals[index]);
                if denominator > 0.25 {
                    point + bisector * (offset / denominator)
                } else {
                    // Near reversals cannot create an unbounded offset spike.
                    point + normals[index] * offset
                }
            }
        })
        .collect()
}

fn stroke(
    points: &[DVec2],
    radius: f64,
    appearance: SurfaceAppearance,
    priority: u8,
    lift_m: f64,
    projection: &tier0::GridProj,
    primitives: &mut Vec<Primitive>,
    simple: bool,
) {
    let mut add = |polygon: Vec<DVec2>| {
        if primitives.len() > MAX_PRIMITIVES {
            return false;
        }
        let polygon: Vec<_> = polygon
            .into_iter()
            .map(|point| {
                let p = projection.to_cols([point.x as f32, point.y as f32]);
                DVec2::new(f64::from(p[0]), f64::from(p[1]))
            })
            .collect();
        let polygon = canonical(polygon);
        let square = [
            DVec2::ZERO,
            DVec2::new(N as f64, 0.0),
            DVec2::splat(N as f64),
            DVec2::new(0.0, N as f64),
        ];
        let polygon = canonical(intersection(&polygon, &square));
        if polygon.len() >= 3 {
            primitives.push(Primitive {
                polygon,
                appearance,
                priority,
                lift_m,
            });
        }
        primitives.len() <= MAX_PRIMITIVES
    };
    for pair in points.windows(2) {
        let direction = (pair[1] - pair[0]).normalize();
        let side = DVec2::new(-direction.y, direction.x) * radius;
        if !add(vec![
            pair[0] - side,
            pair[1] - side,
            pair[1] + side,
            pair[0] + side,
        ]) {
            return;
        }
    }
    // A shared round join has bounded reach regardless of turn angle and
    // survives reversed ways, duplicate fragments, and short segments.
    let sides = if simple {
        8
    } else {
        ((std::f64::consts::TAU * radius / 1.25).ceil() as usize).clamp(12, 32)
    };
    for &point in points {
        if !add((0..sides)
            .map(|index| {
                let angle = index as f64 / sides as f64 * std::f64::consts::TAU;
                point + DVec2::new(angle.cos(), angle.sin()) * radius
            })
            .collect())
        {
            return;
        }
    }
}

fn area(polygon: &[DVec2]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f64>()
        * 0.5
}

pub(crate) fn canonical(mut polygon: Vec<DVec2>) -> Vec<DVec2> {
    canonical_in_place(&mut polygon);
    polygon
}

fn canonical_in_place(polygon: &mut Vec<DVec2>) -> bool {
    polygon.dedup_by(|a, b| a.distance_squared(*b) <= POLYGON_EPS * POLYGON_EPS);
    let closed = polygon.len() > 1
        && polygon[0].distance_squared(*polygon.last().unwrap()) <= POLYGON_EPS * POLYGON_EPS;
    if closed {
        polygon.pop();
    }
    let signed = area(polygon);
    if signed.abs() <= POLYGON_EPS || polygon.iter().any(|point| !point.is_finite()) {
        polygon.clear();
        return true;
    }
    if signed < 0.0 {
        polygon.reverse();
    }
    let first = (0..polygon.len())
        .min_by(|&a, &b| point_cmp(polygon[a], polygon[b]))
        .unwrap();
    polygon.rotate_left(first);
    // Area summation and near-duplicate cleanup must remain bit-identical to
    // another legacy canonical pass. Reordering or closing-point removal can
    // change that pass for nearly degenerate inputs, so only certify an
    // unchanged ordering whose closure needed no cleanup.
    signed > 0.0 && first == 0 && !closed
}

fn point_cmp(a: DVec2, b: DVec2) -> std::cmp::Ordering {
    a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y))
}

fn polygon_cmp(a: &[DVec2], b: &[DVec2]) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| {
        a.iter()
            .zip(b)
            .map(|(&a, &b)| point_cmp(a, b))
            .find(|order| !order.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn bounds(polygon: &[DVec2]) -> (DVec2, DVec2) {
    polygon.iter().fold(
        (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
        |(lo, hi), &point| (lo.min(point), hi.max(point)),
    )
}

fn halfplane_into(
    polygon: &[DVec2],
    a: DVec2,
    b: DVec2,
    inside: bool,
    canonical_input: bool,
    output: &mut Vec<DVec2>,
) -> bool {
    output.clear();
    if polygon.len() < 3 {
        return true;
    }
    let side = if inside { 1.0 } else { -1.0 };
    let distance = |p: DVec2| (b - a).perp_dot(p - a) * side;
    let mut unchanged = true;
    let mut previous = *polygon.last().unwrap();
    let mut previous_distance = distance(previous);
    for &current in polygon {
        let current_distance = distance(current);
        let (previous_in, current_in) = (previous_distance >= 0.0, current_distance >= 0.0);
        if output.is_empty() && (current_in || previous_in != current_in) {
            output.reserve(polygon.len() + 1);
        }
        if previous_in != current_in {
            let t = previous_distance / (previous_distance - current_distance);
            output.push(previous + (current - previous) * t);
        }
        if current_in {
            output.push(current);
        } else {
            unchanged = false;
        }
        previous = current;
        previous_distance = current_distance;
    }
    // Later planes often retain the entire already-canonical polygon. Its
    // identical point sequence needs neither another area scan nor rotation.
    if !canonical_input || !unchanged {
        canonical_in_place(output)
    } else {
        true
    }
}

pub(crate) fn intersection(polygon: &[DVec2], clip: &[DVec2]) -> Vec<DVec2> {
    let mut edges = clip
        .iter()
        .zip(clip.iter().cycle().skip(1))
        .take(clip.len());
    let Some((&a, &b)) = edges.next() else {
        return polygon.to_vec();
    };
    // The first cut can read the caller's polygon directly. A rejected cut
    // needs no copied input or allocated output at all.
    let mut result = Vec::new();
    let mut canonical_input = halfplane_into(polygon, a, b, true, false, &mut result);
    if result.len() < 3 {
        return result;
    }
    let mut scratch = Vec::new();
    for (&a, &b) in edges {
        canonical_input = halfplane_into(&result, a, b, true, canonical_input, &mut scratch);
        std::mem::swap(&mut result, &mut scratch);
        if result.len() < 3 {
            break;
        }
    }
    result
}

pub(crate) fn difference(polygon: &[DVec2], clip: &[DVec2]) -> Vec<Vec<DVec2>> {
    let (p0, p1) = bounds(polygon);
    let (c0, c1) = bounds(clip);
    if p1.x <= c0.x || c1.x <= p0.x || p1.y <= c0.y || c1.y <= p0.y {
        return vec![polygon.to_vec()];
    }
    // The inputs are convex, counterclockwise pieces. A bounding-box overlap
    // does not imply a polygon overlap: running a half-plane difference in
    // that case partitions unchanged land along extended water/road edges.
    // Repeated adjacent clips would retain and recursively split those seams.
    if separated_by_edge(polygon, clip) || separated_by_edge(clip, polygon) {
        return vec![polygon.to_vec()];
    }
    difference_partition(polygon, clip)
}

#[derive(Default)]
struct SubtractionScratch {
    pieces: Vec<Vec<DVec2>>,
    inside: Vec<DVec2>,
    outside: Vec<DVec2>,
}

impl SubtractionScratch {
    fn subtract<const REUSE: bool>(&mut self, pieces: &mut Vec<Vec<DVec2>>, clip: &[DVec2]) {
        if !REUSE {
            *pieces = std::mem::take(pieces)
                .into_iter()
                .flat_map(|polygon| difference(&polygon, clip))
                .collect();
            return;
        }
        let (c0, c1) = bounds(clip);
        self.pieces.clear();
        for polygon in pieces.drain(..) {
            let (p0, p1) = bounds(&polygon);
            if p1.x <= c0.x
                || c1.x <= p0.x
                || p1.y <= c0.y
                || c1.y <= p0.y
                || separated_by_edge(&polygon, clip)
                || separated_by_edge(clip, &polygon)
            {
                self.pieces.push(polygon);
                continue;
            }
            // Unchanged pieces retain their allocation. Only the half-plane
            // outputs that survive subtraction transfer ownership to the result.
            let mut remaining = polygon;
            let mut canonical_input = false;
            for (&a, &b) in clip
                .iter()
                .zip(clip.iter().cycle().skip(1))
                .take(clip.len())
            {
                halfplane_into(&remaining, a, b, false, canonical_input, &mut self.outside);
                if self.outside.len() >= 3 {
                    self.pieces.push(std::mem::take(&mut self.outside));
                }
                canonical_input =
                    halfplane_into(&remaining, a, b, true, canonical_input, &mut self.inside);
                std::mem::swap(&mut remaining, &mut self.inside);
                if remaining.len() < 3 {
                    break;
                }
            }
            if remaining.capacity() > self.outside.capacity() {
                self.outside = remaining;
            }
        }
        std::mem::swap(pieces, &mut self.pieces);
    }
}

fn separated_by_edge(polygon: &[DVec2], other: &[DVec2]) -> bool {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .any(|(&a, &b)| {
            let edge = b - a;
            edge.length_squared() > 0.0 && other.iter().all(|&p| edge.perp_dot(p - a) <= 0.0)
        })
}

fn difference_partition(polygon: &[DVec2], clip: &[DVec2]) -> Vec<Vec<DVec2>> {
    let mut remaining = polygon.to_vec();
    let mut scratch = Vec::new();
    let mut output = Vec::new();
    let mut outside = Vec::new();
    let mut canonical_input = false;
    for (&a, &b) in clip
        .iter()
        .zip(clip.iter().cycle().skip(1))
        .take(clip.len())
    {
        halfplane_into(&remaining, a, b, false, canonical_input, &mut outside);
        if outside.len() >= 3 {
            output.push(std::mem::take(&mut outside));
        }
        canonical_input = halfplane_into(&remaining, a, b, true, canonical_input, &mut scratch);
        std::mem::swap(&mut remaining, &mut scratch);
        if remaining.len() < 3 {
            break;
        }
    }
    output
}

fn emit(
    mesh: &mut MeshBuffers,
    polygon: &[DVec2],
    triangle: [DVec2; 3],
    heights: [f64; 3],
    appearance: SurfaceAppearance,
    lift: f64,
) {
    if polygon.len() < 3 {
        return;
    }
    let [a, b, c] = triangle;
    let denominator = (b - a).perp_dot(c - a);
    let point = |p: DVec2| {
        let v = (p - a).perp_dot(c - a) / denominator;
        let w = (b - a).perp_dot(p - a) / denominator;
        DVec3::new(
            p.x,
            heights[0] + (heights[1] - heights[0]) * v + (heights[2] - heights[0]) * w + lift,
            p.y,
        )
    };
    let normal = (point(c) - point(a)).cross(point(b) - point(a)).normalize();
    let payload = encode_surface_normal(
        2 | (3 << 3) | (u32::from(appearance.material) << 5),
        normal.as_vec3().to_array(),
    );
    let base = mesh.vertices.len() as u32;
    mesh.vertices.extend(polygon.iter().map(|&p| SurfaceVertex {
        pos: point(p).as_vec3().to_array(),
        face_ao: payload,
        rgba: [
            appearance.tint[0],
            appearance.tint[1],
            appearance.tint[2],
            255,
        ],
    }));
    for index in 1..polygon.len().saturating_sub(1) {
        mesh.indices
            .extend([base, base + index as u32 + 1, base + index as u32]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TerrainCollision, material};
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        ellipsoid::geodetic_to_ecef,
    };
    use vearth_data::vectors::Road;

    fn frame(level: u8, lat: f64, lon: f64) -> ChunkFrame {
        let cell = CellId::from_dir(
            geodetic_to_ecef(lat.to_radians(), lon.to_radians(), 0.0).normalize(),
            level,
        );
        ChunkFrame::of(ChunkAddr { cell, k: 0 })
    }

    fn terrain(frame: &ChunkFrame) -> TerrainSurfaceData {
        TerrainSurfaceData {
            heights_m: Box::new(std::array::from_fn(|index| {
                let (x, z) = (index % (N + 1), index / (N + 1));
                let local = 30.0
                    + x as f64 * 0.125
                    + z as f64 * 0.0625
                    + x.saturating_sub(16) as f64 * z.saturating_sub(16) as f64 / 64.0;
                let (_, offset) = frame.column_surface(x as f64 / N as f64, z as f64 / N as f64);
                (local * frame.grid_scale.y - offset) as f32
            })),
            shading: None,
            source_columns: [u32::MAX; N],
        }
    }

    fn build_test(
        frame: &ChunkFrame,
        pack: &FeaturePack,
        terrain: &TerrainSurfaceData,
        exclusions: &[Vec<DVec2>],
    ) -> Option<RoadSurfaceData> {
        let mut source = terrain.clone();
        let water = crate::water_surface::build(
            frame,
            &mut source,
            Some(pack),
            &|_, _| Some(0.0),
            &|_, _, _, _, _| SurfaceAppearance {
                material: crate::material::WATER,
                tint: [0, 80, 100],
            },
        );
        build(frame, pack, terrain, exclusions, &water.triangles)
    }

    fn road(pts: &[[f32; 2]], class: RoadClass) -> Road {
        Road {
            profile: Default::default(),
            pts: pts.to_vec(),
            class,
            is_bridge: false,
        }
    }

    fn projection(frame: &ChunkFrame) -> tier0::GridProj {
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        tier0::GridProj::of(frame, lat, lon)
    }

    fn to_en(projection: &tier0::GridProj, point: DVec2) -> DVec2 {
        let a = DVec2::from_array(projection.to_cols([0.0, 0.0]).map(f64::from));
        let e = DVec2::from_array(projection.to_cols([100.0, 0.0]).map(f64::from)) - a;
        let n = DVec2::from_array(projection.to_cols([0.0, 100.0]).map(f64::from)) - a;
        let p = point - a;
        DVec2::new(p.perp_dot(n), e.perp_dot(p)) * (100.0 / e.perp_dot(n))
    }

    fn covering(mesh: &MeshBuffers, point: DVec2) -> Vec<[u8; 4]> {
        mesh.indices
            .chunks_exact(3)
            .filter_map(|indices| {
                let vertices = indices
                    .iter()
                    .map(|&i| mesh.vertices[i as usize])
                    .collect::<Vec<_>>();
                let polygon: Vec<_> = vertices
                    .iter()
                    .map(|v| DVec2::new(f64::from(v.pos[0]), f64::from(v.pos[2])))
                    .collect();
                let positive = polygon
                    .iter()
                    .zip(polygon.iter().cycle().skip(1))
                    .take(3)
                    .all(|(&a, &b)| (b - a).perp_dot(point - a) <= -1.0e-9);
                positive.then_some(vertices[0].rgba)
            })
            .collect()
    }

    #[test]
    fn fully_clipped_road_polygons_have_no_geometry() {
        let mut mesh = MeshBuffers::default();
        for polygon in [vec![], vec![DVec2::ZERO], vec![DVec2::ZERO, DVec2::ONE]] {
            emit(
                &mut mesh,
                &polygon,
                [DVec2::ZERO, DVec2::X, DVec2::Y],
                [0.0; 3],
                SurfaceAppearance {
                    material: material::ASPHALT,
                    tint: [80; 3],
                },
                0.03,
            );
        }
        assert!(mesh.vertices.is_empty());
        assert!(mesh.indices.is_empty());
    }

    #[test]
    fn metric_width_survives_orientation_and_terrain_lod() {
        use vearth_data::road_profile::RoadDetail;
        for detail in [
            RoadDetail::Generic,
            RoadDetail::Footway,
            RoadDetail::Cycleway,
            RoadDetail::Pedestrian,
        ] {
            for level in [15, 19] {
                for (lat, lon) in [(47.0, 8.0), (60.0, 25.0), (-35.0, 150.0)] {
                    let frame = frame(level, lat, lon);
                    let projection = projection(&frame);
                    let mut pack = FeaturePack::empty();
                    pack.roads
                        .push(road(&[[-2000.0, 0.0], [2000.0, 0.0]], RoadClass::Path));
                    pack.roads[0].profile.detail = detail;
                    let half_width = f64::from(pack.roads[0].width_m()) * 0.5;
                    let surface = build_test(&frame, &pack, &terrain(&frame), &[]).unwrap();
                    let lateral: Vec<_> = surface
                        .mesh
                        .vertices
                        .iter()
                        .map(|v| {
                            to_en(
                                &projection,
                                DVec2::new(f64::from(v.pos[0]), f64::from(v.pos[2])),
                            )
                            .y
                        })
                        .collect();
                    let low = lateral.iter().copied().fold(f64::INFINITY, f64::min);
                    let high = lateral.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    assert!(
                        (low + half_width).abs() < 0.002 && (high - half_width).abs() < 0.002,
                        "L{level} {lat}/{lon}: {low}..{high}"
                    );
                }
            }
        }
    }

    #[test]
    fn coarse_levels_omit_only_subpixel_road_widths() {
        use vearth_data::road_profile::RoadDetail;
        let drawn = |level: u8, class: RoadClass, detail: RoadDetail| {
            let frame = frame(level, 39.74, -104.99);
            let mut pack = FeaturePack::empty();
            pack.roads
                .push(road(&[[-2000.0, 0.0], [2000.0, 0.0]], class));
            pack.roads[0].profile.detail = detail;
            build_test(&frame, &pack, &terrain(&frame), &[])
                .is_some_and(|surface| !surface.mesh.indices.is_empty())
        };
        let cases = [
            (11, RoadClass::Highway, RoadDetail::Generic, true),
            (11, RoadClass::Major, RoadDetail::Generic, false),
            (12, RoadClass::Major, RoadDetail::Generic, true),
            (12, RoadClass::Medium, RoadDetail::Generic, false),
            (13, RoadClass::Minor, RoadDetail::Generic, true),
            (13, RoadClass::Service, RoadDetail::Generic, false),
            (14, RoadClass::Path, RoadDetail::Generic, true),
            (14, RoadClass::Path, RoadDetail::Sidewalk, false),
            (15, RoadClass::Path, RoadDetail::Sidewalk, true),
        ];
        for (level, class, detail, expected) in cases {
            assert_eq!(
                drawn(level, class, detail),
                expected,
                "L{level} {class:?} {detail:?}"
            );
        }
    }

    #[test]
    fn road_vertices_follow_the_exact_graded_collision_triangles() {
        let frame = frame(19, 47.0, 8.0);
        let terrain = terrain(&frame);
        let collision = TerrainCollision {
            heights_local: Box::new(std::array::from_fn(|index| {
                let (x, z) = (index % (N + 1), index / (N + 1));
                let (_, offset) = frame.column_surface(x as f64 / N as f64, z as f64 / N as f64);
                ((f64::from(terrain.heights_m[index]) + offset) / frame.grid_scale.y) as f32
            })),
            source_columns: [u32::MAX; N],
        };
        let mut pack = FeaturePack::empty();
        pack.roads.push(road(
            &[[-100.0, -40.0], [0.0, 1.0], [100.0, 55.0]],
            RoadClass::Minor,
        ));
        let surface = build_test(&frame, &pack, &terrain, &[]).unwrap();
        for vertex in &surface.mesh.vertices {
            let [x, y, z] = vertex.pos.map(f64::from);
            let ground = collision.sample_local(x, z).unwrap();
            assert!(
                (y - ground - ROAD_LIFT_M / frame.grid_scale.y).abs() < 1.0e-5,
                "{x}/{z}: road {y}, ground {ground}"
            );
        }
    }

    #[test]
    fn crossing_ways_have_one_surface_and_stable_priority() {
        let frame = frame(17, 47.0, 8.0);
        let projection = projection(&frame);
        let mut pack = FeaturePack::empty();
        pack.roads
            .push(road(&[[-1000.0, 0.0], [1000.0, 0.0]], RoadClass::Minor));
        pack.roads
            .push(road(&[[0.0, -1000.0], [0.0, 1000.0]], RoadClass::Major));
        let surface = build_test(&frame, &pack, &terrain(&frame), &[]).unwrap();
        for row in 0..35 {
            for column in 0..35 {
                let point = DVec2::new(
                    (column as f64 + 0.371) / 35.0 * N as f64,
                    (row as f64 + 0.219) / 35.0 * N as f64,
                );
                let en = to_en(&projection, point);
                let covered = covering(&surface.mesh, point);
                let expected = en.y.abs() < f64::from(RoadClass::Minor.width_m()) * 0.5
                    || en.x.abs() < f64::from(RoadClass::Major.width_m()) * 0.5;
                assert_eq!(
                    covered.len(),
                    usize::from(expected),
                    "{en:?}: {} overlapping triangles",
                    covered.len()
                );
                if en.x.abs() < 5.9 && en.y.abs() < 3.1 {
                    let tint = road_appearance(RoadClass::Major, 0.5, 1.0).tint;
                    assert_eq!(covered[0], [tint[0], tint[1], tint[2], 255]);
                }
            }
        }
    }

    #[test]
    fn reversed_and_duplicate_source_ways_are_identical() {
        let frame = frame(17, 47.0, 8.0);
        let mut pack = FeaturePack::empty();
        let points = [[-100.0, -40.0], [0.0, 0.0], [7.0, 18.0], [90.0, 40.0]];
        pack.roads.push(road(&points, RoadClass::Minor));
        let original = build_test(&frame, &pack, &terrain(&frame), &[]).unwrap();
        pack.roads[0].pts.reverse();
        pack.roads.push(road(&points, RoadClass::Minor));
        let duplicated = build_test(&frame, &pack, &terrain(&frame), &[]).unwrap();
        assert_eq!(original.mesh.vertices, duplicated.mesh.vertices);
        assert_eq!(original.mesh.indices, duplicated.mesh.indices);
    }

    #[test]
    fn protected_columns_and_bridge_ways_keep_their_ownership() {
        let frame = frame(19, 47.0, 8.0);
        let mut pack = FeaturePack::empty();
        pack.roads
            .push(road(&[[-100.0, 0.0], [100.0, 0.0]], RoadClass::Minor));
        pack.roads[0].is_bridge = true;
        assert!(build_test(&frame, &pack, &terrain(&frame), &[]).is_none());
        pack.roads[0].is_bridge = false;
        let excluded = vec![vec![
            DVec2::splat(12.0),
            DVec2::new(20.0, 12.0),
            DVec2::splat(20.0),
            DVec2::new(12.0, 20.0),
        ]];
        let surface = build_test(&frame, &pack, &terrain(&frame), &excluded).unwrap();
        for triangle in surface.mesh.indices.chunks_exact(3) {
            let center = triangle
                .iter()
                .map(|&i| {
                    let p = surface.mesh.vertices[i as usize].pos;
                    DVec2::new(f64::from(p[0]), f64::from(p[2]))
                })
                .sum::<DVec2>()
                / 3.0;
            assert!(
                !(center.x > 12.00001
                    && center.x < 19.99999
                    && center.y > 12.00001
                    && center.y < 19.99999)
            );
        }
        pack.roads[0].pts[0][0] = f32::NAN;
        assert!(
            !build_test(&frame, &pack, &terrain(&frame), &excluded)
                .unwrap()
                .complete
        );
    }

    #[test]
    fn rail_pair_has_standard_gauge_and_no_synthesized_bed() {
        let frame = frame(19, 47.0, 8.0);
        let projection = projection(&frame);
        let mut pack = FeaturePack::empty();
        pack.roads
            .push(road(&[[-100.0, 0.0], [100.0, 0.0]], RoadClass::Rail));
        let surface = build_test(&frame, &pack, &terrain(&frame), &[]).unwrap();
        for vertex in &surface.mesh.vertices {
            assert_eq!((vertex.face_ao >> 5) & 255, u32::from(material::STEEL));
            let en = to_en(
                &projection,
                DVec2::new(f64::from(vertex.pos[0]), f64::from(vertex.pos[2])),
            );
            assert!(
                (en.y.abs() - RAIL_OFFSET_M).abs() <= RAIL_HALF_WIDTH_M + 0.0001,
                "{en:?}"
            );
        }
        let middle = projection.to_cols([0.123, 0.0]).map(f64::from);
        assert!(covering(&surface.mesh, DVec2::from_array(middle)).is_empty());
        let loop_points = [
            DVec2::ZERO,
            DVec2::new(20.0, 0.0),
            DVec2::splat(20.0),
            DVec2::new(0.0, 20.0),
            DVec2::ZERO,
        ];
        let offset = offset_polyline(&loop_points, RAIL_OFFSET_M);
        assert_eq!(offset[0], *offset.last().unwrap());
        assert!(offset.iter().all(|point| point.is_finite()));
    }

    #[test]
    fn one_invalid_way_does_not_discard_other_source_roads() {
        let frame = frame(17, 47.0, 8.0);
        let mut pack = FeaturePack::empty();
        pack.roads
            .push(road(&[[-1000.0, 0.0], [1000.0, 0.0]], RoadClass::Minor));
        let before = build_test(&frame, &pack, &terrain(&frame), &[]).unwrap();
        pack.roads
            .push(road(&[[f32::NAN, 4.0], [1000.0, 0.0]], RoadClass::Major));
        let after = build_test(&frame, &pack, &terrain(&frame), &[]).unwrap();
        assert_eq!(before.mesh.vertices, after.mesh.vertices);
        assert_eq!(before.mesh.indices, after.mesh.indices);
    }

    #[test]
    fn crossing_complexity_keeps_continuous_metric_ribbons() {
        let frame = frame(17, 47.0, 8.0);
        let projection = projection(&frame);
        let mut pack = FeaturePack::empty();
        pack.roads
            .push(road(&[[-1000.0, 0.0], [1000.0, 0.0]], RoadClass::Minor));
        pack.roads
            .push(road(&[[0.0, -1000.0], [0.0, 1000.0]], RoadClass::Major));
        let normal = build_test(&frame, &pack, &terrain(&frame), &[]).unwrap();
        let fallback = build_bounded(
            &frame,
            &pack,
            &terrain(&frame),
            &[],
            &[],
            crate::built_surface::ROAD_BASE_BUDGET_BYTES,
            0,
        )
        .unwrap();
        assert!(
            crate::built_surface::mesh_bytes(&fallback.mesh)
                < crate::built_surface::ROAD_BASE_BUDGET_BYTES
        );
        for y in 0..40 {
            for x in 0..40 {
                let point = DVec2::new((x as f64 + 0.237) * 0.8, (y as f64 + 0.613) * 0.8);
                let en = to_en(&projection, point);
                let expected = en.y.abs() < f64::from(RoadClass::Minor.width_m()) * 0.5
                    || en.x.abs() < f64::from(RoadClass::Major.width_m()) * 0.5;
                assert_eq!(!covering(&normal.mesh, point).is_empty(), expected);
                assert_eq!(
                    !covering(&fallback.mesh, point).is_empty(),
                    expected,
                    "{en:?}"
                );
            }
        }
    }

    #[test]
    fn convex_subtraction_conserves_area_without_overlapping_pieces() {
        let square = vec![
            DVec2::ZERO,
            DVec2::new(4.0, 0.0),
            DVec2::splat(4.0),
            DVec2::new(0.0, 4.0),
        ];
        let clip = vec![
            DVec2::splat(1.0),
            DVec2::new(3.0, 1.0),
            DVec2::splat(3.0),
            DVec2::new(1.0, 3.0),
        ];
        let pieces = difference(&square, &clip);
        assert!((pieces.iter().map(|polygon| area(polygon)).sum::<f64>() - 12.0).abs() < 1.0e-10);
        for (index, a) in pieces.iter().enumerate() {
            assert!(area(a) > 0.0);
            for b in &pieces[index + 1..] {
                assert!(area(&intersection(a, b)).abs() < 1.0e-10);
            }
            assert!(area(&intersection(a, &clip)).abs() < 1.0e-10);
        }
    }
    #[test]
    fn source_water_cuts_roads_at_exact_diagonal_bank_in_both_budget_modes() {
        let frame = frame(17, 47.0, 8.0);
        let projection = projection(&frame);
        let mut pack = FeaturePack::empty();
        pack.roads
            .push(road(&[[-1000.0, 0.0], [1000.0, 0.0]], RoadClass::Minor));
        pack.water
            .push(vearth_data::vectors::Water::inland(vec![vec![
                [-1000.0, -1000.0],
                [1000.0, 1000.0],
                [-1000.0, 1000.0],
            ]]));
        let source = terrain(&frame);
        let water = crate::water_surface::build(
            &frame,
            &source,
            Some(&pack),
            &|_, _| Some(0.0),
            &|_, _, _, _, _| SurfaceAppearance {
                material: crate::material::WATER,
                tint: [0, 80, 100],
            },
        );
        for simple in [false, true] {
            let surface =
                build_ribbons::<true>(&frame, &pack, &source, &[], &water.triangles, simple, 256)
                    .unwrap();
            for east in [-2.25, 2.25] {
                let p = DVec2::from_array(projection.to_cols([east, 0.27]).map(f64::from));
                assert_eq!(covering(&surface.mesh, p).is_empty(), east < 0.0);
            }
        }
    }

    fn dense_road_fixture(frame: &ChunkFrame) -> (FeaturePack, Vec<Vec<DVec2>>) {
        use vearth_data::vectors::{Building, BuildingKind, Water};
        let projection = projection(frame);
        let point = |x, z| {
            to_en(&projection, DVec2::new(x, z))
                .to_array()
                .map(|v| v as f32)
        };
        let mut pack = FeaturePack::empty();
        for i in 0..20 {
            let t = 1.0 + i as f64 * 1.5;
            let class = [RoadClass::Minor, RoadClass::Major, RoadClass::Path][i % 3];
            pack.roads.push(road(
                &[point(-2.0, t), point(13.3, t + 0.2), point(34.0, t - 0.1)],
                class,
            ));
            pack.roads.push(road(
                &[point(t, -2.0), point(t + 0.4, 17.1), point(t - 0.3, 34.0)],
                class,
            ));
        }
        for i in 0..8 {
            let t = i as f64 * 3.0 - 10.0;
            pack.roads.push(road(
                &[point(-2.0, t), point(34.0, t + 32.0)],
                RoadClass::Rail,
            ));
        }
        for z in 0..4 {
            for x in 0..4 {
                let (a, b) = (3.2 + x as f64 * 7.0, 2.3 + z as f64 * 7.0);
                pack.buildings.push(Building {
                    rings: vec![vec![
                        point(a, b),
                        point(a + 2.7, b),
                        point(a + 2.7, b + 3.1),
                        point(a, b + 3.1),
                    ]],
                    location_rad: None,
                    height_m: 12.0,
                    min_height_m: 0.0,
                    seed: (z * 4 + x) as u64,
                    kind: BuildingKind::Building,
                    roof_shape: None,
                    use_kind: Default::default(),
                });
            }
        }
        pack.water.push(Water::inland(vec![vec![
            point(-2.0, 7.1),
            point(34.0, 19.7),
            point(34.0, 21.4),
            point(-2.0, 8.3),
        ]]));
        let exclusions = vec![canonical(vec![
            DVec2::new(10.7, 6.4),
            DVec2::new(14.9, 8.2),
            DVec2::new(13.8, 13.3),
            DVec2::new(9.4, 12.1),
        ])];
        (pack, exclusions)
    }

    fn assert_same_road_surface(old: &RoadSurfaceData, new: &RoadSurfaceData) {
        assert_eq!(old.complete, new.complete);
        assert_eq!(old.mesh.indices, new.mesh.indices);
        assert_eq!(old.mesh.vertices.len(), new.mesh.vertices.len());
        for (old, new) in old.mesh.vertices.iter().zip(&new.mesh.vertices) {
            assert_eq!(old.pos.map(f32::to_bits), new.pos.map(f32::to_bits));
            assert_eq!(old.face_ao, new.face_ao);
            assert_eq!(old.rgba, new.rgba);
        }
    }

    #[test]
    fn owned_subtraction_keeps_generated_roads_and_budget_fallbacks_identical() {
        for level in [15, 17, 19] {
            let frame = frame(level, 51.9244, 4.4777);
            let (pack, exclusions) = dense_road_fixture(&frame);
            let source = terrain(&frame);
            let water = crate::water_surface::build(
                &frame,
                &source,
                Some(&pack),
                &|_, _| Some(0.0),
                &|_, _, _, _, _| SurfaceAppearance {
                    material: material::WATER,
                    tint: [0, 80, 100],
                },
            );
            assert!(!water.triangles.is_empty());
            for (budget, limit) in [(usize::MAX, 256), (usize::MAX, 0), (0, 256)] {
                let old = build_bounded_with_reuse::<false>(
                    &frame,
                    &pack,
                    &source,
                    &exclusions,
                    &water.triangles,
                    budget,
                    limit,
                )
                .unwrap();
                let new = build_bounded_with_reuse::<true>(
                    &frame,
                    &pack,
                    &source,
                    &exclusions,
                    &water.triangles,
                    budget,
                    limit,
                )
                .unwrap();
                assert!(!old.mesh.indices.is_empty());
                assert_same_road_surface(&old, &new);
            }
        }
    }

    #[test]
    #[ignore = "paired release timing of dense road generation"]
    fn owned_subtraction_dense_road_benchmark() {
        use std::{hint::black_box, time::Instant};
        let frame = frame(17, 51.9244, 4.4777);
        let (pack, exclusions) = dense_road_fixture(&frame);
        let source = terrain(&frame);
        let water = crate::water_surface::build(
            &frame,
            &source,
            Some(&pack),
            &|_, _| Some(0.0),
            &|_, _, _, _, _| SurfaceAppearance {
                material: material::WATER,
                tint: [0, 80, 100],
            },
        );
        for simple in [false, true] {
            let build_old = || {
                build_ribbons::<false>(
                    &frame,
                    &pack,
                    &source,
                    &exclusions,
                    &water.triangles,
                    simple,
                    256,
                )
                .unwrap()
            };
            let build_new = || {
                build_ribbons::<true>(
                    &frame,
                    &pack,
                    &source,
                    &exclusions,
                    &water.triangles,
                    simple,
                    256,
                )
                .unwrap()
            };
            let old = build_old();
            assert_same_road_surface(&old, &build_new());
            let mut times = [Vec::new(), Vec::new()];
            for i in 0..24 {
                for variant in [i % 2, 1 - i % 2] {
                    let start = Instant::now();
                    let output = if variant == 0 {
                        build_old()
                    } else {
                        build_new()
                    };
                    let elapsed = start.elapsed().as_nanos();
                    black_box(output);
                    if i >= 4 {
                        times[variant].push(elapsed);
                    }
                }
            }
            times.iter_mut().for_each(|time| time.sort_unstable());
            eprintln!(
                "dense road simple={simple} roads={} blockers={} water={} vertices={} old_ns={} reuse_ns={}",
                pack.roads.len(),
                pack.buildings.len() + exclusions.len(),
                water.triangles.len(),
                old.mesh.vertices.len(),
                times[0][10],
                times[1][10],
            );
        }
    }

    // Frozen clipping kernels establish exact ordering and floating-point output.
    fn reference_canonical(mut polygon: Vec<DVec2>) -> Vec<DVec2> {
        polygon.dedup_by(|a, b| a.distance_squared(*b) <= POLYGON_EPS * POLYGON_EPS);
        if polygon.len() > 1
            && polygon[0].distance_squared(*polygon.last().unwrap()) <= POLYGON_EPS * POLYGON_EPS
        {
            polygon.pop();
        }
        let signed = area(&polygon);
        if signed.abs() <= POLYGON_EPS || polygon.iter().any(|point| !point.is_finite()) {
            return Vec::new();
        }
        if signed < 0.0 {
            polygon.reverse();
        }
        let first = (0..polygon.len())
            .min_by(|&a, &b| point_cmp(polygon[a], polygon[b]))
            .unwrap();
        polygon.rotate_left(first);
        polygon
    }

    fn reference_halfplane(polygon: &[DVec2], a: DVec2, b: DVec2, inside: bool) -> Vec<DVec2> {
        if polygon.len() < 3 {
            return Vec::new();
        }
        let side = if inside { 1.0 } else { -1.0 };
        let distance = |p: DVec2| (b - a).perp_dot(p - a) * side;
        let mut output = Vec::new();
        let mut previous = *polygon.last().unwrap();
        let mut previous_distance = distance(previous);
        for &current in polygon {
            let current_distance = distance(current);
            let (previous_in, current_in) = (previous_distance >= 0.0, current_distance >= 0.0);
            if previous_in != current_in {
                let t = previous_distance / (previous_distance - current_distance);
                output.push(previous + (current - previous) * t);
            }
            if current_in {
                output.push(current);
            }
            previous = current;
            previous_distance = current_distance;
        }
        reference_canonical(output)
    }

    fn reference_intersection(polygon: &[DVec2], clip: &[DVec2]) -> Vec<DVec2> {
        let mut result = polygon.to_vec();
        for (&a, &b) in clip
            .iter()
            .zip(clip.iter().cycle().skip(1))
            .take(clip.len())
        {
            result = reference_halfplane(&result, a, b, true);
            if result.len() < 3 {
                break;
            }
        }
        result
    }

    fn reference_difference(polygon: &[DVec2], clip: &[DVec2]) -> Vec<Vec<DVec2>> {
        let (p0, p1) = bounds(polygon);
        let (c0, c1) = bounds(clip);
        if p1.x <= c0.x || c1.x <= p0.x || p1.y <= c0.y || c1.y <= p0.y {
            return vec![polygon.to_vec()];
        }
        let mut remaining = polygon.to_vec();
        let mut output = Vec::new();
        for (&a, &b) in clip
            .iter()
            .zip(clip.iter().cycle().skip(1))
            .take(clip.len())
        {
            let outside = reference_halfplane(&remaining, a, b, false);
            if outside.len() >= 3 {
                output.push(outside);
            }
            remaining = reference_halfplane(&remaining, a, b, true);
            if remaining.len() < 3 {
                break;
            }
        }
        output
    }

    fn clipping_bits(polygons: &[Vec<DVec2>]) -> Vec<Vec<[u64; 2]>> {
        polygons
            .iter()
            .map(|p| p.iter().map(|v| v.to_array().map(f64::to_bits)).collect())
            .collect()
    }

    #[test]
    fn disjoint_convex_subtraction_preserves_subject_despite_overlapping_bounds() {
        let subject = canonical(vec![DVec2::ZERO, DVec2::X, DVec2::Y]);
        let clip = canonical(vec![
            DVec2::new(0.1, 1.5),
            DVec2::new(0.4, 0.8),
            DVec2::new(1.2, 0.2),
        ]);
        assert!(intersection(&subject, &clip).is_empty());
        assert_eq!(reference_difference(&subject, &clip).len(), 2);
        for (a, b) in [(&subject, &clip), (&clip, &subject)] {
            assert_eq!(
                clipping_bits(&difference(a, b)),
                clipping_bits(&[a.clone()])
            );
        }

        // Touching along a diagonal is another zero-area overlap; neither
        // source triangle should acquire a seam from its neighbor.
        let neighbor = canonical(vec![DVec2::X, DVec2::ONE, DVec2::Y]);
        assert_eq!(difference(&subject, &neighbor), vec![subject]);
    }

    #[test]
    fn adjacent_canal_triangles_do_not_recursively_partition_unchanged_land() {
        let square = canonical(vec![DVec2::ZERO, DVec2::X, DVec2::ONE, DVec2::Y]);
        let mut dry = vec![square.clone()];
        let mut legacy = vec![square];
        let at = |t: f64| DVec2::new(t, t * 0.7 + 0.1);
        for i in 0..64 {
            let a = at(i as f64 / 64.0);
            let b = at((i + 1) as f64 / 64.0);
            let quad = [a, b, b + DVec2::Y * 0.025, a + DVec2::Y * 0.025];
            for ids in [[0, 1, 2], [0, 2, 3]] {
                let clip = canonical(ids.map(|j| quad[j]).to_vec());
                dry = dry
                    .into_iter()
                    .flat_map(|p| difference(&p, &clip))
                    .collect();
                legacy = legacy
                    .into_iter()
                    .flat_map(|p| reference_difference(&p, &clip))
                    .collect();
            }
        }
        let triangle_count = |pieces: &[Vec<DVec2>]| {
            pieces
                .iter()
                .map(|p| p.len().saturating_sub(2))
                .sum::<usize>()
        };
        assert!(triangle_count(&dry) <= 140);
        assert!(triangle_count(&legacy) >= 250);
        assert!((dry.iter().map(|p| area(p)).sum::<f64>() - 0.975).abs() < 1e-10);
        for z in 0..32 {
            for x in 0..32 {
                let p = DVec2::new((x as f64 + 0.3) / 32.0, (z as f64 + 0.7) / 32.0);
                let canal = p.y > p.x * 0.7 + 0.1 && p.y < p.x * 0.7 + 0.125;
                let covers = dry
                    .iter()
                    .filter(|polygon| {
                        polygon
                            .iter()
                            .zip(polygon.iter().cycle().skip(1))
                            .take(polygon.len())
                            .all(|(&a, &b)| (b - a).perp_dot(p - a) >= -1e-12)
                    })
                    .count();
                assert_eq!(covers, usize::from(!canal), "coverage at {p:?}");
            }
        }
    }

    #[test]
    fn reused_clipping_buffers_match_legacy_order_and_coordinate_bits() {
        let mut cases = vec![
            vec![],
            vec![DVec2::ZERO],
            vec![DVec2::ZERO, DVec2::ONE],
            vec![DVec2::ZERO, DVec2::X, DVec2::Y],
            vec![
                DVec2::ZERO,
                DVec2::X,
                DVec2::X,
                DVec2::ONE,
                DVec2::Y,
                DVec2::ZERO,
            ],
            vec![
                DVec2::ZERO,
                DVec2::new(0.0, 0.8e-10),
                DVec2::new(0.8e-10, 0.0),
            ],
            vec![
                DVec2::ZERO,
                DVec2::X,
                DVec2::ONE,
                DVec2::Y,
                DVec2::new(0.0, 0.9 * POLYGON_EPS),
                DVec2::new(0.9 * POLYGON_EPS, 0.0),
            ],
            vec![
                DVec2::new(-0.0, 0.0),
                DVec2::X,
                DVec2::ONE,
                DVec2::new(0.0, 1.0),
                DVec2::new(0.0, -0.0),
            ],
            vec![DVec2::ZERO, DVec2::new(f64::NAN, 1.0), DVec2::ONE],
            vec![DVec2::ZERO, DVec2::new(f64::INFINITY, 1.0), DVec2::ONE],
        ];
        let mut state = 0x9752b163efad04ce_u64;
        let mut random = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / ((1_u64 << 53) as f64)
        };
        for index in 0..192 {
            let count = 3 + index % 9;
            let scale = [1.0e-6, 0.01, 1.0, 32.0, 1.0e6][index % 5];
            let center = DVec2::new(random() - 0.5, random() - 0.5) * scale * 4.0;
            let radius = 0.1 + random() * scale;
            let angle = random() * std::f64::consts::TAU;
            let mut polygon: Vec<_> = (0..count)
                .map(|i| {
                    let a = angle + (i as f64 / count as f64) * std::f64::consts::TAU;
                    center + DVec2::new(a.cos(), a.sin()) * radius
                })
                .collect();
            if index % 3 == 0 {
                polygon.reverse();
            }
            if index % 7 == 0 {
                polygon.push(polygon[0]);
            }
            cases.push(polygon);
        }
        let mut subtraction = SubtractionScratch::default();
        for (i, polygon) in cases.iter().enumerate() {
            assert_eq!(
                clipping_bits(&[canonical(polygon.clone())]),
                clipping_bits(&[reference_canonical(polygon.clone())]),
                "canonical {i}"
            );
            for (j, clip) in cases.iter().enumerate() {
                let mut pieces = vec![polygon.clone()];
                subtraction.subtract::<true>(&mut pieces, clip);
                assert_eq!(
                    clipping_bits(&pieces),
                    clipping_bits(&difference(polygon, clip)),
                    "owned subtraction {i}/{j}"
                );
                assert_eq!(
                    clipping_bits(&[intersection(polygon, clip)]),
                    clipping_bits(&[reference_intersection(polygon, clip)]),
                    "intersection {i}/{j}"
                );
                let (p0, p1) = bounds(polygon);
                let (c0, c1) = bounds(clip);
                let partition = if p1.x <= c0.x || c1.x <= p0.x || p1.y <= c0.y || c1.y <= p0.y {
                    vec![polygon.to_vec()]
                } else {
                    difference_partition(polygon, clip)
                };
                assert_eq!(
                    clipping_bits(&partition),
                    clipping_bits(&reference_difference(polygon, clip)),
                    "difference partition {i}/{j}"
                );
            }
        }
    }
}
