//! Exact wet-footprint subdivision of the source terrain surface. The original
//! ground remains authoritative outside water, including islands and holes.

use crate::{
    N,
    road_surface::{canonical, intersection},
    water_surface::WaterTriangle,
};
use glam::{DVec2, DVec3};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use vearth_core::{
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex, decode_surface_normal, encode_surface_normal},
};

const EPS: f64 = 1e-8;

struct WaterPlane {
    points: [DVec3; 3],
    polygon: Vec<DVec2>,
    depth: f64,
    ocean: bool,
}
impl WaterPlane {
    fn height(&self, p: DVec2) -> f64 {
        height(self.points, p) - self.depth
    }
}
struct Shore {
    a: DVec2,
    b: DVec2,
    water: usize,
}

pub(super) struct Conditioned {
    pub mesh: MeshBuffers,
    pub boundary: Vec<[SurfaceVertex; 2]>,
}

struct Buckets {
    lists: Vec<Vec<usize>>,
}
impl Buckets {
    fn new() -> Self {
        Self {
            lists: vec![Vec::new(); N * N],
        }
    }
    fn range(lo: DVec2, hi: DVec2) -> (usize, usize, usize, usize) {
        let at = |v: f64| v.floor().clamp(0.0, (N - 1) as f64) as usize;
        (
            at(lo.x - EPS),
            at(lo.y - EPS),
            at(hi.x + EPS),
            at(hi.y + EPS),
        )
    }
    fn insert(&mut self, index: usize, lo: DVec2, hi: DVec2) {
        let (x0, z0, x1, z1) = Self::range(lo, hi);
        for z in z0..=z1 {
            for x in x0..=x1 {
                self.lists[z * N + x].push(index);
            }
        }
    }
    fn candidates(&self, lo: DVec2, hi: DVec2) -> Vec<usize> {
        let (x0, z0, x1, z1) = Self::range(lo, hi);
        let mut result = Vec::new();
        for z in z0..=z1 {
            for x in x0..=x1 {
                result.extend(&self.lists[z * N + x]);
            }
        }
        result.sort_unstable();
        result.dedup();
        result
    }

    fn terrain_polygon_candidates<const UNIT_BIN: bool>(
        &self,
        points: [DVec3; 3],
        lo: DVec2,
        hi: DVec2,
    ) -> Cow<'_, [usize]> {
        if UNIT_BIN && let Some(bin) = unit_triangle_bin(points, lo, hi) {
            // Insertion includes all polygon bounds touching this unit cell.
            // IDs are inserted once per bin in source order, matching the
            // general query's sorted, unique ownership order. This shortcut
            // is unsuitable for tolerant shore-segment or offset-probe queries.
            Cow::Borrowed(&self.lists[bin])
        } else {
            Cow::Owned(self.candidates(lo, hi))
        }
    }
}

fn unit_triangle_bin(points: [DVec3; 3], lo: DVec2, hi: DVec2) -> Option<usize> {
    if !points.into_iter().all(DVec3::is_finite)
        || lo != lo.floor()
        || hi != lo + DVec2::ONE
        || lo.min_element() < 0.0
        || hi.max_element() > N as f64
    {
        return None;
    }
    let mut corners = 0_u8;
    for point in points.map(xz) {
        if (point.x != lo.x && point.x != hi.x) || (point.y != lo.y && point.y != hi.y) {
            return None;
        }
        let corner = u8::from(point.x == hi.x) | (u8::from(point.y == hi.y) << 1);
        corners |= 1 << corner;
    }
    (corners.count_ones() == 3).then_some(lo.y as usize * N + lo.x as usize)
}
fn xz(p: DVec3) -> DVec2 {
    DVec2::new(p.x, p.z)
}
fn vec(v: SurfaceVertex) -> DVec3 {
    DVec3::from_array(v.pos.map(f64::from))
}
fn bounds(points: impl IntoIterator<Item = DVec2>) -> (DVec2, DVec2) {
    points.into_iter().fold(
        (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    )
}
fn weights(points: [DVec3; 3], p: DVec2) -> [f64; 3] {
    let a = xz(points[1] - points[0]);
    let b = xz(points[2] - points[0]);
    let v = p - xz(points[0]);
    let det = a.perp_dot(b);
    let u = v.perp_dot(b) / det;
    let w = a.perp_dot(v) / det;
    [1.0 - u - w, u, w]
}
fn height(points: [DVec3; 3], p: DVec2) -> f64 {
    let w = weights(points, p);
    (0..3).map(|i| w[i] * points[i].y).sum()
}
fn contains(polygon: &[DVec2], p: DVec2) -> bool {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .all(|(&a, &b)| (b - a).perp_dot(p - a) >= -1e-11)
}
fn edge_key(a: SurfaceVertex, b: SurfaceVertex) -> [[u32; 3]; 2] {
    let a = a.pos.map(f32::to_bits);
    let b = b.pos.map(f32::to_bits);
    if a < b { [a, b] } else { [b, a] }
}
fn perimeter(mesh: &MeshBuffers) -> Vec<[SurfaceVertex; 2]> {
    let mut edges = BTreeMap::new();
    for ids in mesh.indices.chunks_exact(3) {
        for i in 0..3 {
            let a = mesh.vertices[ids[i] as usize];
            let b = mesh.vertices[ids[(i + 1) % 3] as usize];
            let entry = edges.entry(edge_key(a, b)).or_insert((0, [a, b]));
            entry.0 += 1;
        }
    }
    edges
        .into_values()
        .filter_map(|(count, edge)| (count == 1).then_some(edge))
        .collect()
}
fn on_segment(a: DVec2, b: DVec2, p: DVec2) -> bool {
    let edge = b - a;
    let length = edge.length();
    length > EPS
        && edge.perp_dot(p - a).abs() <= length * 2e-5
        && (p - a).dot(edge) >= -2e-5 * length
        && (p - b).dot(edge) <= 2e-5 * length
}
struct Perimeter {
    edges: Vec<[SurfaceVertex; 2]>,
    buckets: Buckets,
}
impl Perimeter {
    fn new(edges: Vec<[SurfaceVertex; 2]>) -> Self {
        let mut buckets = Buckets::new();
        for (i, edge) in edges.iter().enumerate() {
            let a = xz(vec(edge[0]));
            let b = xz(vec(edge[1]));
            // The broad phase includes the exact predicate's endpoint and
            // lateral tolerance, including across terrain-grid boundaries.
            buckets.insert(
                i,
                a.min(b) - DVec2::splat(4e-5),
                a.max(b) + DVec2::splat(4e-5),
            );
        }
        Self { edges, buckets }
    }
    fn contains(&self, a: DVec2, b: DVec2) -> bool {
        let (x0, z0, x1, z1) = Buckets::range(a.min(b), a.max(b));
        // This is an existential query: duplicate IDs and traversal order do
        // not affect the result, so the bucket lists can be borrowed directly.
        for z in z0..=z1 {
            for x in x0..=x1 {
                for &id in &self.buckets.lists[z * N + x] {
                    let edge = self.edges[id];
                    let p = xz(vec(edge[0]));
                    let q = xz(vec(edge[1]));
                    if on_segment(p, q, a) && on_segment(p, q, b) {
                        return true;
                    }
                }
            }
        }
        false
    }
}
type FootprintEdge = [[u64; 2]; 2];

fn footprint_edge(a: DVec2, b: DVec2) -> FootprintEdge {
    let key = |p: DVec2| p.to_array().map(|v| if v == 0.0 { 0 } else { v.to_bits() });
    let a = key(a);
    let b = key(b);
    if a < b { [a, b] } else { [b, a] }
}

fn interior_edges(water: &[WaterPlane]) -> HashSet<FootprintEdge> {
    let mut sides = HashMap::<FootprintEdge, u8>::with_capacity(water.len() * 2);
    for plane in water {
        for i in 0..3 {
            let a = xz(plane.points[i]);
            let b = xz(plane.points[(i + 1) % 3]);
            let c = xz(plane.points[(i + 2) % 3]);
            let key = footprint_edge(a, b);
            let p = DVec2::from_array(key[0].map(f64::from_bits));
            let q = DVec2::from_array(key[1].map(f64::from_bits));
            let edge = q - p;
            let side = edge.perp_dot(c - p);
            // Near-degenerate slivers retain the arrangement's existing probe
            // tolerance. Heights and winding do not affect wet-footprint union.
            if side.abs() > edge.length() * 2e-6 {
                *sides.entry(key).or_default() |= if side > 0.0 { 1 } else { 2 };
            }
        }
    }
    sides
        .into_iter()
        .filter_map(|(key, sides)| (sides == 3).then_some(key))
        .collect()
}

fn shorelines(water: &[WaterPlane], buckets: &Buckets, perimeter: &Perimeter) -> Vec<Shore> {
    // Both sides of an identical edge are wet, so it cannot be a shoreline.
    // Cancel these interior triangulation edges before the segment arrangement;
    // unmatched, partial, and T-junction edges still use the full union test.
    let interior = interior_edges(water);
    shoreline_arrangement(water, buckets, perimeter, &interior)
}

fn shoreline_arrangement(
    water: &[WaterPlane],
    buckets: &Buckets,
    perimeter: &Perimeter,
    interior: &HashSet<FootprintEdge>,
) -> Vec<Shore> {
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    for (id, plane) in water.iter().enumerate() {
        for i in 0..3 {
            let a = xz(plane.points[i]);
            let b = xz(plane.points[(i + 1) % 3]);
            let delta = b - a;
            if delta.length_squared() < EPS * EPS || interior.contains(&footprint_edge(a, b)) {
                continue;
            }
            let candidates = buckets.candidates(a.min(b), a.max(b));
            let mut cuts = vec![0.0, 1.0];
            for &j in &candidates {
                for k in 0..3 {
                    let p = xz(water[j].points[k]);
                    let q = xz(water[j].points[(k + 1) % 3]);
                    let edge = q - p;
                    let det = delta.perp_dot(edge);
                    if det.abs() > 1e-12 {
                        let t = (p - a).perp_dot(edge) / det;
                        let u = (p - a).perp_dot(delta) / det;
                        if t > EPS && t < 1.0 - EPS && (-EPS..=1.0 + EPS).contains(&u) {
                            cuts.push(t);
                        }
                    } else if delta.perp_dot(p - a).abs() < EPS * delta.length() {
                        for p in [p, q] {
                            let t = (p - a).dot(delta) / delta.length_squared();
                            if t > EPS && t < 1.0 - EPS {
                                cuts.push(t);
                            }
                        }
                    }
                }
            }
            cuts.sort_by(f64::total_cmp);
            cuts.dedup_by(|a, b| (*a - *b).abs() < EPS);
            for span in cuts.windows(2) {
                let mut p = a + delta * span[0];
                let mut q = a + delta * span[1];
                if perimeter.contains(p, q) {
                    continue;
                }
                let mid = (p + q) * 0.5;
                let side = DVec2::new(-delta.y, delta.x).normalize() * 1e-6;
                let covered = |point| {
                    candidates
                        .iter()
                        .any(|&j| contains(&water[j].polygon, point))
                };
                let (left, right) = (covered(mid + side), covered(mid - side));
                if left == right {
                    continue;
                }
                if left {
                    std::mem::swap(&mut p, &mut q);
                }
                let key = |p: DVec2| [(p.x * 1e7).round() as i64, (p.y * 1e7).round() as i64];
                let (kp, kq) = (key(p), key(q));
                let key = if kp < kq { [kp, kq] } else { [kq, kp] };
                if seen.insert(key) {
                    output.push(Shore {
                        a: p,
                        b: q,
                        water: id,
                    });
                }
            }
        }
    }
    output
}
fn clip_height(poly: &[DVec2], distance: impl Fn(DVec2) -> f64) -> Vec<DVec2> {
    let mut out = Vec::new();
    for (&a, &b) in poly
        .iter()
        .zip(poly.iter().cycle().skip(1))
        .take(poly.len())
    {
        let da = distance(a);
        let db = distance(b);
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            out.push(a.lerp(b, da / (da - db)));
        }
    }
    canonical(out)
}
fn vertex(source: [SurfaceVertex; 3], p: DVec2, y: f64) -> SurfaceVertex {
    let points = source.map(vec);
    let w = weights(points, p);
    let owner = (0..3).max_by(|&a, &b| w[a].total_cmp(&w[b])).unwrap();
    let rgba = std::array::from_fn(|channel| {
        (0..3)
            .map(|i| w[i] * f64::from(source[i].rgba[channel]))
            .sum::<f64>()
            .round()
            .clamp(0.0, 255.0) as u8
    });
    let n = (0..3)
        .map(|i| {
            DVec3::from_array(
                decode_surface_normal(source[i].face_ao)
                    .unwrap_or([0.0, 1.0, 0.0])
                    .map(f64::from),
            ) * w[i]
        })
        .sum::<DVec3>()
        .normalize_or_zero();
    SurfaceVertex {
        pos: [p.x as f32, y as f32, p.y as f32],
        face_ao: encode_surface_normal(source[owner].face_ao, n.as_vec3().to_array()),
        rgba,
    }
}
fn emit(
    mesh: &mut MeshBuffers,
    polygon: &[DVec2],
    source: [SurfaceVertex; 3],
    height: impl Fn(DVec2) -> f64,
    flat: bool,
) {
    for i in 1..polygon.len().saturating_sub(1) {
        let mut vertices =
            [polygon[0], polygon[i + 1], polygon[i]].map(|p| vertex(source, p, height(p)));
        let p = vertices.map(vec);
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        if !n.is_finite() || n.length_squared() < 1e-20 {
            continue;
        }
        if flat {
            for v in &mut vertices {
                v.face_ao = encode_surface_normal(v.face_ao, n.normalize().as_vec3().to_array());
            }
        }
        let first = mesh.vertices.len() as u32;
        mesh.vertices.extend(vertices);
        mesh.indices.extend([first, first + 1, first + 2]);
    }
}
fn clip_segment(a: DVec2, b: DVec2, polygon: &[DVec2]) -> Option<(DVec2, DVec2)> {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for (&p, &q) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        let da = (q - p).perp_dot(a - p);
        let db = (q - p).perp_dot(b - p);
        if da < -EPS && db < -EPS {
            return None;
        }
        if (da < 0.0) != (db < 0.0) {
            let t = da / (da - db);
            if da < 0.0 {
                lo = lo.max(t);
            } else {
                hi = hi.min(t);
            }
        }
    }
    (hi - lo > EPS).then(|| (a.lerp(b, lo), a.lerp(b, hi)))
}

fn natural_bank(vertex: SurfaceVertex) -> bool {
    use crate::material;
    matches!(
        ((vertex.face_ao >> 5) & 255) as u8,
        material::SAND
            | material::GRASS
            | material::ROCK
            | material::SEABED
            | material::SOIL
            | material::VOLCANIC
            | material::WETLAND
    )
}

fn emit_bank_quad(mesh: &mut MeshBuffers, vertices: [SurfaceVertex; 4]) {
    for ids in [[0, 1, 2], [0, 2, 3]] {
        let mut triangle = ids.map(|i| vertices[i]);
        let p = triangle.map(vec);
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        if n.length_squared() < 1e-20 {
            continue;
        }
        for v in &mut triangle {
            v.face_ao = encode_surface_normal(v.face_ao, n.normalize().as_vec3().to_array());
        }
        let first = mesh.vertices.len() as u32;
        mesh.vertices.extend(triangle);
        mesh.indices.extend([first, first + 1, first + 2]);
    }
}

fn emit_bank(
    mesh: &mut MeshBuffers,
    top: [SurfaceVertex; 2],
    bottom: [SurfaceVertex; 2],
    water_height: [f64; 2],
    scale_y: f64,
    ocean: bool,
) {
    if !top.into_iter().all(natural_bank) {
        emit_bank_quad(mesh, [top[0], top[1], bottom[1], bottom[0]]);
        return;
    }
    let upper_m = if ocean { 0.65 } else { 0.35 };
    let shade = |mut vertex: SurfaceVertex, i: usize| {
        let relative_m = (f64::from(vertex.pos[1]) - water_height[i]) * scale_y;
        let t = ((relative_m + 0.15) / (upper_m + 0.15)).clamp(0.0, 1.0);
        let tint = 0.76 + 0.24 * t * t * (3.0 - 2.0 * t);
        for channel in &mut vertex.rgba[..3] {
            *channel = (f64::from(*channel) * tint).round() as u8;
        }
        vertex
    };
    // Only subdivide the existing vertical bank plane. The wet strip follows
    // the local water datum and cannot spread up a tall cliff or across land.
    let at = |relative_m: f64| {
        std::array::from_fn(|i| {
            let mut vertex = top[i];
            vertex.pos[1] = (water_height[i] + relative_m / scale_y)
                .clamp(f64::from(bottom[i].pos[1]), f64::from(top[i].pos[1]))
                as f32;
            shade(vertex, i)
        })
    };
    let rows = [
        [shade(bottom[0], 0), shade(bottom[1], 1)],
        at(-0.15),
        at(upper_m),
        [shade(top[0], 0), shade(top[1], 1)],
    ];
    for pair in rows.windows(2) {
        emit_bank_quad(mesh, [pair[1][0], pair[1][1], pair[0][1], pair[0][0]]);
    }
}

struct WetRegion {
    polygon: Vec<DVec2>,
    water: usize,
}
struct OwnedPatch {
    polygon: Vec<DVec2>,
    water: Option<usize>,
}
#[derive(Clone, Copy)]
struct SweepEdge {
    a: DVec2,
    b: DVec2,
    owner: usize,
    enter: bool,
}
impl SweepEdge {
    fn at(&self, x: f64) -> DVec2 {
        if x == self.a.x {
            self.a
        } else if x == self.b.x {
            self.b
        } else {
            DVec2::new(
                x,
                self.a.y + (self.b.y - self.a.y) * ((x - self.a.x) / (self.b.x - self.a.x)),
            )
        }
    }
}
struct SweepPatch {
    x0: f64,
    x1: f64,
    lower: usize,
    upper: usize,
    owner: usize,
}

/// Partition only at finite polygon boundaries. Interior water triangulation
/// never extends clipping planes into the remaining dry domain. Each convex
/// input is CCW; the lowest wet-region index preserves source water authority.
fn ownership_sweep(source: &[DVec2], regions: &[WetRegion]) -> Vec<OwnedPatch> {
    if regions.is_empty() {
        return vec![OwnedPatch {
            polygon: source.to_vec(),
            water: None,
        }];
    }
    if regions[0].polygon == source {
        return vec![OwnedPatch {
            polygon: source.to_vec(),
            water: Some(regions[0].water),
        }];
    }
    let source_owner = regions.len();
    let mut edges = Vec::<SweepEdge>::new();
    let mut cuts = Vec::new();
    for (owner, polygon) in regions
        .iter()
        .map(|r| r.polygon.as_slice())
        .chain([source])
        .enumerate()
    {
        for (&a, &b) in polygon
            .iter()
            .zip(polygon.iter().cycle().skip(1))
            .take(polygon.len())
        {
            cuts.push(a.x);
            if a.x == b.x {
                continue;
            }
            let enter = a.x < b.x;
            let (a, b) = if enter { (a, b) } else { (b, a) };
            edges.push(SweepEdge { a, b, owner, enter });
        }
    }
    // Polygon vertices suffice for disjoint water faces. Genuine overlapping
    // boundaries additionally need their crossing abscissae; an active x-range
    // sweep and y bounds avoid comparing unrelated render-triangle edges.
    let mut starts: Vec<_> = (0..edges.len()).collect();
    starts.sort_unstable_by(|&a, &b| edges[a].a.x.total_cmp(&edges[b].a.x).then(a.cmp(&b)));
    let mut active_edges = Vec::<usize>::new();
    for &id in &starts {
        let edge = edges[id];
        active_edges.retain(|&i| edges[i].b.x > edge.a.x);
        for &i in &active_edges {
            let other = edges[i];
            if edge.owner == other.owner
                || edge.a.y.min(edge.b.y) > other.a.y.max(other.b.y)
                || other.a.y.min(other.b.y) > edge.a.y.max(edge.b.y)
            {
                continue;
            }
            let a = edge.b - edge.a;
            let b = other.b - other.a;
            let det = a.perp_dot(b);
            if det == 0.0 {
                continue;
            }
            let delta = other.a - edge.a;
            let t = delta.perp_dot(b) / det;
            let u = delta.perp_dot(a) / det;
            if t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0 {
                cuts.push(edge.a.x + a.x * t);
            }
        }
        active_edges.push(id);
    }
    cuts.sort_unstable_by(f64::total_cmp);
    cuts.dedup();
    let mut patches = Vec::<SweepPatch>::new();
    // Keys contain only local edge/owner ordinals, and are never iterated to
    // determine geometry order. Reuse both sweep frontiers between strips.
    let mut previous = rustc_hash::FxHashMap::<(usize, usize, usize), usize>::default();
    let mut next = rustc_hash::FxHashMap::default();
    let mut events = Vec::<(f64, usize)>::new();
    let mut active = BTreeSet::new();
    active_edges.clear();
    let mut start = 0;
    for span in cuts.windows(2) {
        let (x0, x1) = (span[0], span[1]);
        let mid = x0 + (x1 - x0) * 0.5;
        // Adjacent floating-point abscissae contain no representable interior.
        if mid <= x0 || mid >= x1 {
            continue;
        }
        events.clear();
        while start < starts.len() && edges[starts[start]].a.x < mid {
            active_edges.push(starts[start]);
            start += 1;
        }
        active_edges.retain(|&id| edges[id].b.x > mid);
        for &i in &active_edges {
            events.push((edges[i].at(mid).y, i));
        }
        events.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        active.clear();
        let mut owner = None;
        let mut lower = 0;
        next.clear();
        for &(_, edge_id) in &events {
            let edge = edges[edge_id];
            if edge.enter {
                active.insert(edge.owner);
            } else {
                active.remove(&edge.owner);
            }
            let new_owner = active
                .contains(&source_owner)
                .then(|| *active.first().unwrap());
            if new_owner == owner {
                continue;
            }
            if let Some(owner) = owner {
                let key = (owner, lower, edge_id);
                let index = if let Some(&index) = previous.get(&key) {
                    patches[index].x1 = x1;
                    index
                } else {
                    let index = patches.len();
                    patches.push(SweepPatch {
                        x0,
                        x1,
                        lower,
                        upper: edge_id,
                        owner,
                    });
                    index
                };
                next.insert(key, index);
            }
            lower = edge_id;
            owner = new_owner;
        }
        std::mem::swap(&mut previous, &mut next);
    }
    patches
        .into_iter()
        .filter_map(|patch| {
            let bottom = edges[patch.lower];
            let top = edges[patch.upper];
            let polygon = canonical(vec![
                bottom.at(patch.x0),
                bottom.at(patch.x1),
                top.at(patch.x1),
                top.at(patch.x0),
            ]);
            (polygon.len() >= 3).then(|| OwnedPatch {
                polygon,
                water: (patch.owner != source_owner).then(|| regions[patch.owner].water),
            })
        })
        .collect()
}

fn emit_conditioned_patch(
    mesh: &mut MeshBuffers,
    original: [SurfaceVertex; 3],
    patch: &[DVec2],
    plane: Option<&WaterPlane>,
) -> bool {
    let points = original.map(vec);
    let Some(plane) = plane else {
        emit(mesh, patch, original, |p| height(points, p), false);
        return false;
    };
    let delta = |p| height(points, p) - plane.height(p);
    let (minimum, maximum) = patch
        .iter()
        .map(|&p| delta(p))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), d| {
            (a.min(d), b.max(d))
        });
    if maximum <= 1e-10 {
        emit(mesh, patch, original, |p| height(points, p), false);
        return false;
    }
    // The bed is the minimum of two affine planes. Its crossing is a real
    // surface crease, independent of the ownership arrangement's topology.
    if minimum >= 0.0 {
        emit(mesh, patch, original, |p| plane.height(p), true);
    } else {
        emit(
            mesh,
            &clip_height(patch, delta),
            original,
            |p| plane.height(p),
            true,
        );
        emit(
            mesh,
            &clip_height(patch, |p| -delta(p)),
            original,
            |p| height(points, p),
            false,
        );
    }
    true
}

pub(super) fn carve(
    frame: &ChunkFrame,
    source: &MeshBuffers,
    water: &[WaterTriangle],
    heightfield_columns: &mut [u32; N],
) -> Option<Conditioned> {
    carve_with_query::<true>(frame, source, water, heightfield_columns)
}

fn carve_with_query<const UNIT_BIN: bool>(
    frame: &ChunkFrame,
    source: &MeshBuffers,
    water: &[WaterTriangle],
    heightfield_columns: &mut [u32; N],
) -> Option<Conditioned> {
    let water: Vec<_> = water
        .iter()
        .filter_map(|triangle| {
            let points = triangle
                .vertices
                .map(|p| DVec3::from_array(p.map(f64::from)));
            let polygon = canonical(points.map(xz).to_vec());
            (polygon.len() >= 3).then_some(WaterPlane {
                points,
                polygon,
                depth: if triangle.kind.is_ocean() { 0.05 } else { 1.2 } / frame.grid_scale.y,
                ocean: triangle.kind.is_ocean(),
            })
        })
        .collect();
    let mut buckets = Buckets::new();
    for (i, plane) in water.iter().enumerate() {
        let (lo, hi) = bounds(plane.polygon.iter().copied());
        buckets.insert(i, lo, hi);
    }
    // Existing bathymetry needs no subdivision. This cheap overlap proof
    // avoids shoreline arrangement and preserves its original vertex payloads.
    let needs_carve = source.indices.chunks_exact(3).any(|ids| {
        let points = [0, 1, 2].map(|i| vec(source.vertices[ids[i] as usize]));
        let polygon = canonical(points.map(xz).to_vec());
        if polygon.len() < 3 {
            return false;
        }
        let (lo, hi) = bounds(polygon.iter().copied());
        buckets
            .terrain_polygon_candidates::<UNIT_BIN>(points, lo, hi)
            .iter()
            .copied()
            .any(|id| {
                let plane = &water[id];
                if points.iter().all(|&p| p.y <= plane.height(xz(p)) + 1e-10) {
                    return false;
                }
                intersection(&polygon, &plane.polygon)
                    .iter()
                    .any(|&p| height(points, p) > plane.height(p) + 1e-10)
            })
    });
    if !needs_carve {
        return None;
    }
    let source_edges = Perimeter::new(perimeter(source));
    let shores = shorelines(&water, &buckets, &source_edges);
    let mut shore_buckets = Buckets::new();
    for (i, shore) in shores.iter().enumerate() {
        shore_buckets.insert(i, shore.a.min(shore.b), shore.a.max(shore.b));
    }
    let mut mesh = MeshBuffers::default();
    let mut bank_seen = HashSet::new();
    for ids in source.indices.chunks_exact(3) {
        let original = [0, 1, 2].map(|i| source.vertices[ids[i] as usize]);
        let points = original.map(vec);
        let polygon = canonical(points.map(xz).to_vec());
        if polygon.len() < 3 {
            continue;
        }
        let (lo, hi) = bounds(polygon.iter().copied());
        let candidates = buckets.terrain_polygon_candidates::<UNIT_BIN>(points, lo, hi);
        let mut regions = Vec::new();
        for &id in candidates.iter() {
            let plane = &water[id];
            let clipped = intersection(&polygon, &plane.polygon);
            if clipped.len() >= 3
                && clipped
                    .iter()
                    .any(|&p| height(points, p) > plane.height(p) + 1e-10)
            {
                regions.push(WetRegion {
                    polygon: clipped,
                    water: id,
                });
            }
        }
        let mut changed = false;
        if regions.is_empty() {
            let first = mesh.vertices.len() as u32;
            mesh.vertices.extend(original);
            mesh.indices.extend([first, first + 1, first + 2]);
        } else {
            for patch in ownership_sweep(&polygon, &regions) {
                changed |= emit_conditioned_patch(
                    &mut mesh,
                    original,
                    &patch.polygon,
                    patch.water.map(|id| &water[id]),
                );
            }
        }
        if changed {
            let center = points.iter().copied().sum::<DVec3>() / 3.0;
            let x = center.x.floor().clamp(0.0, (N - 1) as f64) as usize;
            let z = center.z.floor().clamp(0.0, (N - 1) as f64) as usize;
            heightfield_columns[z] &= !(1 << x);
        }
        for id in shore_buckets.candidates(lo, hi) {
            let shore = &shores[id];
            let Some((mut a, mut b)) = clip_segment(shore.a, shore.b, &polygon) else {
                continue;
            };
            let plane = &water[shore.water];
            let da = height(points, a) - plane.height(a);
            let db = height(points, b) - plane.height(b);
            if da <= 1e-10 && db <= 1e-10 {
                continue;
            }
            if da < 0.0 {
                a = a.lerp(b, da / (da - db));
            } else if db < 0.0 {
                b = a.lerp(b, da / (da - db));
            }
            let top = [
                vertex(original, a, height(points, a)),
                vertex(original, b, height(points, b)),
            ];
            let bottom = [
                vertex(original, a, plane.height(a).min(height(points, a))),
                vertex(original, b, plane.height(b).min(height(points, b))),
            ];
            let key = [edge_key(top[0], top[1]), edge_key(bottom[0], bottom[1])];
            if !bank_seen.insert(key) {
                continue;
            }
            if original.into_iter().all(natural_bank) {
                emit_bank(
                    &mut mesh,
                    top,
                    bottom,
                    [plane.height(a) + plane.depth, plane.height(b) + plane.depth],
                    frame.grid_scale.y,
                    plane.ocean,
                );
            } else {
                emit_bank_quad(&mut mesh, [top[0], top[1], bottom[1], bottom[0]]);
            }
        }
    }
    let mut boundary = Vec::new();
    let mut seen = HashSet::new();
    for ids in mesh.indices.chunks_exact(3) {
        let vertices = [0, 1, 2].map(|i| mesh.vertices[ids[i] as usize]);
        let p = vertices.map(vec);
        if (p[1] - p[0]).cross(p[2] - p[0]).y <= 1e-12 {
            continue;
        }
        for i in 0..3 {
            let a = vertices[i];
            let b = vertices[(i + 1) % 3];
            if source_edges.contains(xz(vec(a)), xz(vec(b))) && seen.insert(edge_key(a, b)) {
                boundary.push([b, a]);
            }
        }
    }
    Some(Conditioned { mesh, boundary })
}

pub(super) fn append_skirts(mesh: &mut MeshBuffers, boundary: Vec<[SurfaceVertex; 2]>, depth: f32) {
    for [a, b] in boundary {
        let mut vertices = [a, b, b, a];
        vertices[2].pos[1] -= depth;
        vertices[3].pos[1] -= depth;
        let p = vertices.map(vec);
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        if n.length_squared() < 1e-20 {
            continue;
        }
        for v in &mut vertices {
            v.face_ao = encode_surface_normal(v.face_ao, n.normalize().as_vec3().to_array());
        }
        let first = mesh.vertices.len() as u32;
        mesh.vertices.extend(vertices);
        mesh.indices
            .extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SurfaceAppearance, TerrainSurfaceData, material, terrain_surface, water_surface};
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        cubesphere::Face,
    };
    use vearth_data::vectors::{
        FeaturePack, Water, Watercourse, WatercourseKind, WatercourseOutputMode,
    };

    fn query_source(heights: &[f32]) -> MeshBuffers {
        assert_eq!(heights.len(), (N + 1) * (N + 1));
        let vertices = heights
            .iter()
            .enumerate()
            .map(|(i, &height)| {
                let x = i % (N + 1);
                let z = i / (N + 1);
                let payload = vearth_core::mesh::TERRAIN_BLEND_FACE
                    | (((x + z) % 4) as u32) << 3
                    | u32::from(if x % 5 == 0 {
                        material::ROCK
                    } else {
                        material::GRASS
                    }) << 5;
                SurfaceVertex {
                    pos: [x as f32, height, z as f32],
                    face_ao: if i % 7 == 0 {
                        payload
                    } else {
                        encode_surface_normal(payload, [x as f32 * 0.01, 1.0, z as f32 * -0.02])
                    },
                    rgba: [60 + x as u8, 110 + z as u8, 50 + ((x * z) % 31) as u8, 255],
                }
            })
            .collect();
        let mut indices = Vec::new();
        for z in 0..N {
            for x in 0..N {
                let a = (z * (N + 1) + x) as u32;
                let b = a + 1;
                let d = a + (N + 1) as u32;
                let c = d + 1;
                indices.extend([a, c, b, a, d, c]);
            }
        }
        MeshBuffers { vertices, indices }
    }

    fn assert_same_conditioning(
        old: &Option<Conditioned>,
        new: &Option<Conditioned>,
        old_columns: &[u32; N],
        new_columns: &[u32; N],
    ) {
        assert_eq!(old_columns, new_columns);
        match (old, new) {
            (Some(old), Some(new)) => {
                let key = |v: SurfaceVertex| (v.pos.map(f32::to_bits), v.face_ao, v.rgba);
                assert_eq!(old.mesh.vertices.len(), new.mesh.vertices.len());
                for (index, (&a, &b)) in
                    old.mesh.vertices.iter().zip(&new.mesh.vertices).enumerate()
                {
                    assert_eq!(key(a), key(b), "vertex {index}");
                }
                assert_eq!(old.mesh.indices, new.mesh.indices);
                assert_eq!(old.boundary.len(), new.boundary.len());
                for (index, (a, b)) in old.boundary.iter().zip(&new.boundary).enumerate() {
                    assert_eq!(a.map(key), b.map(key), "boundary {index}");
                }
            }
            (None, None) => {}
            _ => panic!("unit-bin query changed whether conditioning was needed"),
        }
    }

    #[test]
    fn unit_water_query_certifies_only_integer_unit_right_triangles() {
        let buckets = Buckets::new();
        for z in 0..N {
            for x in 0..N {
                let lo = DVec2::new(x as f64, z as f64);
                let hi = lo + DVec2::ONE;
                let corners = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
                for omitted in 0..4 {
                    let triangle: Vec<_> = corners
                        .iter()
                        .enumerate()
                        .filter_map(|(i, p)| (i != omitted).then_some(*p))
                        .collect();
                    for start in 0..3 {
                        for reverse in [false, true] {
                            let points = std::array::from_fn(|i| {
                                let p = triangle[(start + if reverse { 3 - i } else { i }) % 3];
                                DVec3::new(p.x, i as f64 * -3.5, p.y)
                            });
                            assert_eq!(unit_triangle_bin(points, lo, hi), Some(z * N + x));
                            assert!(matches!(
                                buckets.terrain_polygon_candidates::<true>(points, lo, hi),
                                Cow::Borrowed(_)
                            ));
                        }
                    }
                }
            }
        }
        let base = [DVec3::ZERO, DVec3::X, DVec3::Z];
        for points in [
            base.map(|p| p * 0.01),
            base.map(|p| p * 2.0),
            base.map(|p| p + DVec3::new(0.5, 0.0, 0.0)),
            base.map(|p| p - DVec3::X),
            base.map(|p| p + DVec3::splat(N as f64)),
            [base[0], base[1], DVec3::new(0.3, 0.0, 1.0)],
            [base[0], base[0], base[2]],
            [base[0], base[1], DVec3::new(0.0, f64::NAN, 1.0)],
            [base[0], base[1], DVec3::new(f64::INFINITY, 0.0, 1.0)],
        ] {
            let (lo, hi) = bounds(points.map(xz));
            assert_eq!(unit_triangle_bin(points, lo, hi), None, "{points:?}");
            let actual = buckets.terrain_polygon_candidates::<true>(points, lo, hi);
            assert!(matches!(actual, Cow::Owned(_)));
            assert_eq!(actual.as_ref(), buckets.candidates(lo, hi));
        }
        let signed_zero = [DVec3::new(-0.0, 0.0, -0.0), DVec3::X, DVec3::Z];
        assert_eq!(
            unit_triangle_bin(signed_zero, DVec2::ZERO, DVec2::ONE),
            Some(0)
        );
    }

    #[test]
    fn unit_water_query_preserves_padded_boundary_intersections_and_owner_order() {
        for origin in [DVec2::ZERO, DVec2::splat(8.0), DVec2::splat(31.0)] {
            let mut buckets = Buckets::new();
            let mut water = Vec::new();
            for delta in [-2.0 * EPS, -EPS, -EPS * 0.5, 0.0, EPS * 0.5, EPS, EPS * 2.0] {
                for side in [0.03, 0.4, 1.0, 2.3] {
                    for corner in [DVec2::ZERO, DVec2::X, DVec2::Y, DVec2::ONE] {
                        let p = origin + corner + DVec2::splat(delta);
                        for triangle in [
                            [p, p + DVec2::new(side, 0.0), p + DVec2::new(0.0, side)],
                            [p, p - DVec2::new(side, 0.0), p - DVec2::new(0.0, side)],
                        ] {
                            let polygon = canonical(triangle.to_vec());
                            let (lo, hi) = bounds(polygon.iter().copied());
                            buckets.insert(water.len(), lo, hi);
                            water.push(polygon);
                        }
                    }
                }
            }
            let corners = [
                origin,
                origin + DVec2::X,
                origin + DVec2::ONE,
                origin + DVec2::Y,
            ];
            for omit in 0..4 {
                let mut polygon: Vec<_> = corners
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| (i != omit).then_some(*p))
                    .collect();
                for _ in 0..2 {
                    polygon.reverse();
                    let points =
                        std::array::from_fn(|i| DVec3::new(polygon[i].x, 5.0, polygon[i].y));
                    let polygon = canonical(polygon.clone());
                    let old = buckets.terrain_polygon_candidates::<false>(
                        points,
                        origin,
                        origin + DVec2::ONE,
                    );
                    let new = buckets.terrain_polygon_candidates::<true>(
                        points,
                        origin,
                        origin + DVec2::ONE,
                    );
                    assert!(new.windows(2).all(|ids| ids[0] < ids[1]));
                    let clipped = |ids: &[usize]| {
                        ids.iter()
                            .filter_map(|&id| {
                                let result = intersection(&polygon, &water[id]);
                                (result.len() >= 3).then_some((id, result))
                            })
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(clipped(&old), clipped(&new));
                    for (id, water) in water.iter().enumerate() {
                        let (lo, hi) = bounds(water.iter().copied());
                        // The polygon predicate has no tolerance. Near-EPS
                        // disjoint bounds are covered by the differential
                        // clipping assertion, not required as candidates.
                        if lo.cmple(origin + DVec2::ONE).all() && hi.cmpge(origin).all() {
                            assert!(new.contains(&id), "touching water bounds omitted");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn unit_water_query_does_not_replace_tolerant_shore_segment_queries() {
        let lo = DVec2::splat(8.0);
        let hi = lo + DVec2::ONE;
        let polygon = vec![lo, lo + DVec2::X, hi];
        let a = lo + DVec2::new(-1.5 * EPS, -0.75 * EPS);
        let b = lo + DVec2::new(-1.4 * EPS, -0.7 * EPS);
        assert!(clip_segment(a, b, &polygon).is_some());
        let mut buckets = Buckets::new();
        buckets.insert(0, a.min(b), a.max(b));
        assert_eq!(buckets.candidates(lo, hi), vec![0]);
        // Two tolerant half-planes extend the accepted corner beyond the
        // polygon insertion padding; shore queries must retain their route.
        assert!(!buckets.lists[8 * N + 8].contains(&0));
    }

    #[test]
    fn unit_water_query_keeps_full_conditioning_identical_for_overlaps_and_fallbacks() {
        use vearth_data::vectors::WaterRegionKind;
        let (frame, _) = fixture(17, 0.0);
        let heights: Vec<_> = (0..(N + 1) * (N + 1))
            .map(|i| 5.0 + (i % (N + 1)) as f32 * 0.13 - (i / (N + 1)) as f32 * 0.06)
            .collect();
        let source = query_source(&heights);
        let mut water = Vec::new();
        for (x0, z0, x1, z1, y, kind) in [
            (0.0, 0.0, 4.0, 2.5, 3.0, WaterRegionKind::Ocean),
            (31.5, 28.0, 32.0, 32.0, 4.0, WaterRegionKind::Canal),
            (8.25, 8.3, 8.4, 8.8, 2.0, WaterRegionKind::Lake),
            (7.9, 7.0, 10.1, 11.0, 4.0, WaterRegionKind::Canal),
            (9.0, 9.0, 12.0, 12.0, 7.0, WaterRegionKind::Lake),
            (14.0, 14.0, 17.0, 15.0, 3.0, WaterRegionKind::Lake),
            (14.0, 16.0, 17.0, 17.0, 3.0, WaterRegionKind::Lake),
            (14.0, 15.0, 15.0, 16.0, 3.0, WaterRegionKind::Lake),
            (16.0, 15.0, 17.0, 16.0, 3.0, WaterRegionKind::Lake),
            (-0.25, 15.0, 0.25, 15.05, 2.0, WaterRegionKind::River),
        ] {
            let points = [
                [x0, y, z0],
                [x1, y, z0],
                [x1, y + 0.2, z1],
                [x0, y + 0.2, z1],
            ];
            for indices in [[0, 1, 2], [0, 2, 3]] {
                water.push(WaterTriangle {
                    vertices: indices.map(|i| points[i]),
                    kind,
                    depth_m: 2.0,
                });
            }
        }
        water.push(water[6].clone());
        water.last_mut().unwrap().vertices.reverse();
        for mode in 0..4 {
            let mut source = source.clone();
            for vertex in &mut source.vertices {
                match mode {
                    1 => {
                        vertex.pos[0] += vertex.pos[2] * 0.013;
                    }
                    2 => {
                        vertex.pos[1] -= 100.0;
                    }
                    _ => {}
                }
            }
            if mode == 3 {
                // A missing source column must keep the same exposed boundary.
                let first = (8 * N + 8) * 6;
                source.indices.drain(first..first + 6);
            }
            let mut old_columns = [u32::MAX; N];
            let mut new_columns = old_columns;
            let old = carve_with_query::<false>(&frame, &source, &water, &mut old_columns);
            let new = carve_with_query::<true>(&frame, &source, &water, &mut new_columns);
            assert_same_conditioning(&old, &new, &old_columns, &new_columns);
            assert_eq!(old.is_none(), mode == 2);
        }
    }

    #[test]
    #[ignore = "actual C19 cached water/height input; requires VEARTH_WATER_QUERY_FIXTURES"]
    fn unit_water_query_actual_cache_benchmark() {
        use std::{path::Path, time::Instant};
        use vearth_data::vectors::WaterRegionKind;
        let directory =
            vearth_platform::env::var("VEARTH_WATER_QUERY_FIXTURES").expect("fixture directory");
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
        for name in [
            "4-12-3770-2228.water.txt",
            "4-12-3765-2218.water.txt",
            "4-14-15059-8877.water.txt",
        ] {
            let text = std::fs::read_to_string(Path::new(&directory).join(name)).unwrap();
            let mut cell = None;
            let mut heights = Vec::new();
            let mut water = Vec::new();
            for line in text
                .lines()
                .filter(|line| !line.starts_with('#') && !line.is_empty())
            {
                let mut words = line.split_whitespace();
                match words.next().unwrap() {
                    "cell" => {
                        let values: Vec<u32> = words.map(|word| word.parse().unwrap()).collect();
                        cell = Some(CellId {
                            face: Face(values[0] as u8),
                            level: values[1] as u8,
                            i: values[2],
                            j: values[3],
                        });
                    }
                    "heights" => heights.extend(words.map(|word| word.parse::<f32>().unwrap())),
                    "water" => {
                        let kind: usize = words.next().unwrap().parse().unwrap();
                        let depth_m = words.next().unwrap().parse().unwrap();
                        let coordinates: Vec<f32> =
                            words.map(|word| word.parse().unwrap()).collect();
                        assert_eq!(coordinates.len(), 9);
                        water.push(WaterTriangle {
                            vertices: std::array::from_fn(|i| {
                                std::array::from_fn(|j| coordinates[i * 3 + j])
                            }),
                            kind: kinds[kind],
                            depth_m,
                        });
                    }
                    _ => panic!("unknown fixture record"),
                }
            }
            let frame = ChunkFrame::of(ChunkAddr {
                cell: cell.unwrap(),
                k: 0,
            });
            let source = query_source(&heights);
            let mut buckets = Buckets::new();
            let mut accepted = 0;
            for triangle in &water {
                let polygon = canonical(
                    triangle
                        .vertices
                        .map(|p| DVec2::new(f64::from(p[0]), f64::from(p[2])))
                        .to_vec(),
                );
                if polygon.len() < 3 {
                    continue;
                }
                let (lo, hi) = bounds(polygon);
                buckets.insert(accepted, lo, hi);
                accepted += 1;
            }
            let mut visits = [0_usize; 2];
            for ids in source.indices.chunks_exact(3) {
                let points = std::array::from_fn(|i| vec(source.vertices[ids[i] as usize]));
                let (lo, hi) = bounds(points.map(xz));
                visits[0] += buckets
                    .terrain_polygon_candidates::<false>(points, lo, hi)
                    .len();
                visits[1] += buckets
                    .terrain_polygon_candidates::<true>(points, lo, hi)
                    .len();
            }
            assert!(visits[1] * 2 < visits[0]);
            let mut times = [Vec::new(), Vec::new()];
            let mut output_triangles = 0;
            for repeat in 0..4 {
                let mut old = None;
                let mut new = None;
                let mut old_columns = [u32::MAX; N];
                let mut new_columns = old_columns;
                for variant in if repeat % 2 == 0 { [0, 1] } else { [1, 0] } {
                    let start = Instant::now();
                    if variant == 0 {
                        old = carve_with_query::<false>(&frame, &source, &water, &mut old_columns);
                    } else {
                        new = carve_with_query::<true>(&frame, &source, &water, &mut new_columns);
                    }
                    if repeat > 0 {
                        times[variant].push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                assert_same_conditioning(&old, &new, &old_columns, &new_columns);
                output_triangles = new
                    .as_ref()
                    .expect("actual water cell conditions terrain")
                    .mesh
                    .indices
                    .len()
                    / 3;
            }
            for values in &mut times {
                values.sort_by(f64::total_cmp);
            }
            eprintln!(
                "unit water query {name}: water={}, source_tris={}, output_tris={output_triangles}, polygon_candidates={}->{}, median_carve_ms={:.3}->{:.3}; exact vertices/indices/boundary/columns",
                water.len(),
                source.indices.len() / 3,
                visits[0],
                visits[1],
                times[0][1],
                times[1][1]
            );
        }
    }

    #[test]
    fn wet_bank_strips_preserve_plane_extent_and_keep_tall_banks_dry_above_water() {
        let v = |x, y| SurfaceVertex {
            pos: [x, y, 0.0],
            face_ao: 2 | (3 << 3) | (u32::from(material::GRASS) << 5),
            rgba: [100, 140, 90, 255],
        };
        let top = [v(0.0, 5.0), v(10.0, 8.0)];
        let bottom = [v(0.0, -1.0), v(10.0, 0.0)];
        for ocean in [false, true] {
            let mut mesh = MeshBuffers::default();
            emit_bank(&mut mesh, top, bottom, [0.0, 1.0], 1.0, ocean);
            assert!(mesh.indices.len() <= 18);
            let area: f64 = mesh
                .indices
                .chunks_exact(3)
                .map(|ids| {
                    let p = [0, 1, 2].map(|i| vec(mesh.vertices[ids[i] as usize]));
                    (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5
                })
                .sum();
            assert!((area - 70.0).abs() < 1e-5);
            let upper = if ocean { 0.65 } else { 0.35 };
            assert!(mesh.vertices.iter().any(|v| v.rgba[0] < 100));
            for vertex in &mesh.vertices {
                assert_eq!(vertex.pos[2], 0.0);
                assert_eq!(vertex.rgba[3], 255);
                assert!(decode_surface_normal(vertex.face_ao).is_some());
                let above_water = f64::from(vertex.pos[1] - vertex.pos[0] * 0.1);
                if above_water >= upper + 1e-6 {
                    assert_eq!(vertex.rgba, [100, 140, 90, 255]);
                }
            }
        }
    }

    #[test]
    fn managed_and_frozen_banks_keep_original_geometry_and_color() {
        for material in [
            material::ASPHALT,
            material::PAVED,
            material::WALL,
            material::SNOW,
            material::ICE,
            material::FIELD_CROP,
        ] {
            let v = |x, y| SurfaceVertex {
                pos: [x, y, 0.0],
                face_ao: u32::from(material) << 5,
                rgba: [100, 110, 120, 255],
            };
            let top = [v(0.0, 2.0), v(10.0, 2.0)];
            let bottom = [v(0.0, -1.0), v(10.0, -1.0)];
            let mut expected = MeshBuffers::default();
            emit_bank_quad(&mut expected, [top[0], top[1], bottom[1], bottom[0]]);
            let mut actual = MeshBuffers::default();
            emit_bank(&mut actual, top, bottom, [0.0; 2], 1.0, false);
            assert_eq!(actual.vertices, expected.vertices);
            assert_eq!(actual.indices, expected.indices);
        }
    }

    fn fixture(level: u8, slope: f64) -> (ChunkFrame, TerrainSurfaceData) {
        let frame = ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face: Face(0),
                level,
                i: 1 << (level - 1),
                j: 1 << (level - 1),
            },
            k: 0,
        });
        let source = TerrainSurfaceData {
            heights_m: Box::new(std::array::from_fn(|i| {
                (35.0 + slope * (i % (N + 1)) as f64) as f32
            })),
            shading: None,
            source_columns: [u32::MAX; N],
        };
        (frame, source)
    }
    fn en(frame: &ChunkFrame, p: [f64; 2]) -> [f32; 2] {
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = crate::tier0::GridProj::of(frame, lat, lon);
        let q = |p| DVec2::from_array(projection.to_cols(p).map(f64::from));
        let origin = q([0.0, 0.0]);
        let a = q([1.0, 0.0]) - origin;
        let b = q([0.0, 1.0]) - origin;
        let p = DVec2::from_array(p) - origin;
        let det = a.perp_dot(b);
        [(p.perp_dot(b) / det) as f32, (a.perp_dot(p) / det) as f32]
    }
    fn ring(frame: &ChunkFrame, x0: f64, z0: f64, x1: f64, z1: f64) -> Vec<[f32; 2]> {
        [[x0, z0], [x1, z0], [x1, z1], [x0, z1]]
            .map(|p| en(frame, p))
            .to_vec()
    }
    fn appearance() -> [[SurfaceAppearance; N]; N] {
        [[SurfaceAppearance {
            material: material::GRASS,
            tint: [77, 105, 62],
        }; N]; N]
    }
    fn geometry(
        frame: &ChunkFrame,
        source: &TerrainSurfaceData,
        pack: &FeaturePack,
    ) -> (
        water_surface::WaterGeometry,
        terrain_surface::TerrainGeometry,
    ) {
        let before = source.heights_m.clone();
        let water = water_surface::build(
            frame,
            source,
            Some(pack),
            &|_, _| Some(30.0),
            &|_, _, _, _, _| SurfaceAppearance {
                material: material::WATER,
                tint: [30, 90, 125],
            },
        );
        assert!(water.complete);
        assert!(!water.triangles.is_empty());
        assert_eq!(
            source.heights_m, before,
            "hydrology must not move dry source corners"
        );
        let terrain =
            terrain_surface::build_with_water(frame, source, &appearance(), &water.triangles);
        (water, terrain)
    }
    fn top(mesh: &MeshBuffers, point: DVec2) -> Option<f64> {
        mesh.indices
            .chunks_exact(3)
            .filter_map(|ids| {
                let p = [0, 1, 2].map(|i| vec(mesh.vertices[ids[i] as usize]));
                if (p[1] - p[0]).cross(p[2] - p[0]).y <= 1e-12 {
                    return None;
                }
                weights(p, point)
                    .iter()
                    .all(|w| *w >= -1e-9)
                    .then(|| height(p, point))
            })
            .max_by(f64::total_cmp)
    }
    fn sequential_ownership(source: &[DVec2], regions: &[WetRegion]) -> Vec<OwnedPatch> {
        let mut dry = vec![source.to_vec()];
        let mut out = Vec::new();
        for region in regions {
            for piece in &dry {
                let polygon = intersection(&region.polygon, piece);
                if polygon.len() >= 3 {
                    out.push(OwnedPatch {
                        polygon,
                        water: Some(region.water),
                    });
                }
            }
            dry = dry
                .into_iter()
                .flat_map(|piece| crate::road_surface::difference(&piece, &region.polygon))
                .collect();
        }
        out.extend(dry.into_iter().map(|polygon| OwnedPatch {
            polygon,
            water: None,
        }));
        out
    }
    fn polygon_area(p: &[DVec2]) -> f64 {
        p.iter()
            .zip(p.iter().cycle().skip(1))
            .take(p.len())
            .map(|(a, b)| a.perp_dot(*b))
            .sum::<f64>()
            * 0.5
    }
    fn strict_inside(p: &[DVec2], point: DVec2) -> bool {
        p.iter()
            .zip(p.iter().cycle().skip(1))
            .take(p.len())
            .all(|(&a, &b)| (b - a).perp_dot(point - a) > 1e-9)
    }
    fn check_ownership(source: &[DVec2], regions: &[WetRegion]) -> Vec<OwnedPatch> {
        let actual = ownership_sweep(source, regions);
        let reference = sequential_ownership(source, regions);
        let areas = |patches: &[OwnedPatch]| {
            let mut result = BTreeMap::<Option<usize>, f64>::new();
            for patch in patches {
                let area = polygon_area(&patch.polygon);
                assert!(area > 0.0);
                *result.entry(patch.water).or_default() += area;
            }
            result
        };
        let expected_area = areas(&reference);
        let actual_area = areas(&actual);
        for key in expected_area.keys().chain(actual_area.keys()) {
            assert!(
                (expected_area.get(key).copied().unwrap_or_default()
                    - actual_area.get(key).copied().unwrap_or_default())
                .abs()
                    < 1e-7,
                "owner {key:?}: {actual_area:?} != {expected_area:?}"
            );
        }
        assert!((actual_area.values().sum::<f64>() - polygon_area(source)).abs() < 1e-7);
        for y in 0..61 {
            for x in 0..61 {
                let p = DVec2::new(
                    (x as f64 + 0.371) * 8.0 / 61.0,
                    (y as f64 + 0.619) * 8.0 / 61.0,
                );
                if !strict_inside(source, p) {
                    continue;
                }
                let found: Vec<_> = actual
                    .iter()
                    .filter(|patch| strict_inside(&patch.polygon, p))
                    .collect();
                if found.is_empty() {
                    assert!(
                        actual.iter().any(|patch| contains(&patch.polygon, p)),
                        "uncovered {p:?}"
                    );
                    continue;
                }
                assert_eq!(found.len(), 1, "overlapping patches at {p:?}");
                let owner = regions
                    .iter()
                    .find(|region| contains(&region.polygon, p))
                    .map(|region| region.water);
                assert_eq!(found[0].water, owner, "wrong authority at {p:?}");
            }
        }
        actual
    }
    #[test]
    fn ownership_sweep_preserves_holes_crossings_duplicates_and_t_junction_authority() {
        let source = canonical(vec![
            DVec2::ZERO,
            DVec2::new(8.0, 0.0),
            DVec2::new(0.0, 8.0),
        ]);
        let rectangle = |x0, y0, x1, y1| {
            vec![
                DVec2::new(x0, y0),
                DVec2::new(x1, y0),
                DVec2::new(x1, y1),
                DVec2::new(x0, y1),
            ]
        };
        let ring = vec![
            rectangle(1.0, 1.0, 4.0, 1.5),
            rectangle(1.0, 3.5, 4.0, 4.0),
            rectangle(1.0, 1.5, 1.5, 3.5),
            rectangle(3.5, 1.5, 4.0, 3.5),
        ];
        let mut overlap = ring.clone();
        overlap.extend([
            vec![
                DVec2::new(0.2, 2.0),
                DVec2::new(5.5, 1.0),
                DVec2::new(2.5, 5.0),
            ],
            rectangle(1.0, 1.0, 4.0, 1.5),
            rectangle(4.0, 1.5, 4.3, 2.0),
            rectangle(4.0, 2.0, 4.3, 3.0),
        ]);
        for polygons in [
            Vec::new(),
            ring,
            overlap,
            vec![rectangle(2.0, 2.0, 2.0001, 2.0001)],
        ] {
            for reverse in [false, true] {
                let regions: Vec<_> = polygons
                    .iter()
                    .enumerate()
                    .filter_map(|(water, p)| {
                        let mut p = p.clone();
                        if reverse {
                            p.reverse();
                        }
                        let polygon = intersection(&source, &canonical(p));
                        (polygon.len() >= 3).then_some(WetRegion { polygon, water })
                    })
                    .collect();
                check_ownership(&source, &regions);
            }
        }
    }
    #[test]
    fn internal_water_refinement_does_not_fragment_dry_terrain() {
        let source = canonical(vec![
            DVec2::ZERO,
            DVec2::new(8.0, 0.0),
            DVec2::new(0.0, 8.0),
        ]);
        let corners = [
            DVec2::new(1.0, 1.0),
            DVec2::new(3.0, 1.0),
            DVec2::new(3.0, 3.0),
            DVec2::new(1.0, 3.0),
        ];
        let mut triangles = vec![
            [corners[0], corners[1], corners[2]],
            [corners[0], corners[2], corners[3]],
        ];
        let mut baseline = None;
        for _ in 0..4 {
            let regions: Vec<_> = triangles
                .iter()
                .enumerate()
                .map(|(water, p)| WetRegion {
                    polygon: canonical(p.to_vec()),
                    water,
                })
                .collect();
            let patches = check_ownership(&source, &regions);
            let count: usize = patches
                .iter()
                .filter(|p| p.water.is_none())
                .map(|p| p.polygon.len() - 2)
                .sum();
            if let Some(expected) = baseline {
                assert_eq!(
                    count,
                    expected,
                    "{} wet faces changed the dry domain complexity",
                    regions.len()
                );
            } else {
                baseline = Some(count);
            }
            triangles = triangles
                .into_iter()
                .flat_map(|p| {
                    let center = (p[0] + p[1] + p[2]) / 3.0;
                    [
                        [p[0], p[1], center],
                        [p[1], p[2], center],
                        [p[2], p[0], center],
                    ]
                })
                .collect();
        }
    }
    #[test]
    fn ownership_sweep_preserves_bed_crossings_and_interpolated_source_attributes() {
        let original = [
            SurfaceVertex {
                pos: [0.0, 20.0, 0.0],
                face_ao: 2 | (3 << 3) | (u32::from(material::WETLAND) << 5),
                rgba: [70, 100, 50, 255],
            },
            SurfaceVertex {
                pos: [0.0, 44.0, 8.0],
                face_ao: 2 | (3 << 3) | (u32::from(material::WETLAND) << 5),
                rgba: [100, 160, 100, 255],
            },
            SurfaceVertex {
                pos: [8.0, 36.0, 0.0],
                face_ao: 2 | (3 << 3) | (u32::from(material::WETLAND) << 5),
                rgba: [130, 120, 80, 255],
            },
        ];
        let points = original.map(vec);
        let source = canonical(points.map(xz).to_vec());
        let planes: Vec<_> = [
            (
                [[0.5, 29.0, 0.5], [6.0, 30.0, 0.5], [0.5, 31.0, 6.0]],
                1.2,
                false,
            ),
            (
                [[0.0, 27.0, 2.0], [6.0, 32.0, 0.0], [1.0, 35.0, 7.0]],
                0.05,
                true,
            ),
        ]
        .into_iter()
        .map(|(p, depth, ocean)| {
            let points = p.map(DVec3::from_array);
            WaterPlane {
                points,
                polygon: canonical(points.map(xz).to_vec()),
                depth,
                ocean,
            }
        })
        .collect();
        let regions: Vec<_> = planes
            .iter()
            .enumerate()
            .map(|(water, plane)| WetRegion {
                polygon: intersection(&source, &plane.polygon),
                water,
            })
            .collect();
        let actual = check_ownership(&source, &regions);
        let reference = sequential_ownership(&source, &regions);
        let build = |patches: Vec<OwnedPatch>| {
            let mut mesh = MeshBuffers::default();
            for patch in patches {
                emit_conditioned_patch(
                    &mut mesh,
                    original,
                    &patch.polygon,
                    patch.water.map(|id| &planes[id]),
                );
            }
            mesh
        };
        let actual = build(actual);
        let reference = build(reference);
        for y in 0..47 {
            for x in 0..47 {
                let p = DVec2::new(
                    (x as f64 + 0.37) * 8.0 / 47.0,
                    (y as f64 + 0.61) * 8.0 / 47.0,
                );
                if !strict_inside(&source, p) {
                    continue;
                }
                let expected = regions
                    .iter()
                    .find(|r| contains(&r.polygon, p))
                    .map_or_else(
                        || height(points, p),
                        |r| height(points, p).min(planes[r.water].height(p)),
                    );
                assert!((top(&actual, p).unwrap() - expected).abs() < 1e-5);
                assert!((top(&actual, p).unwrap() - top(&reference, p).unwrap()).abs() < 1e-5);
            }
        }
        for &v in &actual.vertices {
            let p = xz(vec(v));
            let expected = vertex(original, p, f64::from(v.pos[1]));
            for i in 0..4 {
                assert!(v.rgba[i].abs_diff(expected.rgba[i]) <= 1);
            }
            assert_eq!((v.face_ao >> 5) & 255, u32::from(material::WETLAND));
        }
    }
    #[test]
    fn indexed_perimeter_preserves_segment_tolerance() {
        let v = |x, z| SurfaceVertex {
            pos: [x, 0.0, z],
            face_ao: 0,
            rgba: [0; 4],
        };
        let edges = vec![
            [v(1.0, 0.0), v(1.0, 3.0)],
            [v(0.0, 2.0), v(4.0, 2.0)],
            [v(30.0, 30.0), v(32.0, 32.0)],
        ];
        let index = Perimeter::new(edges.clone());
        for edge in &edges {
            let a = xz(vec(edge[0]));
            let b = xz(vec(edge[1]));
            let normal = DVec2::new(-(b - a).y, (b - a).x).normalize();
            for offset in [-2.1e-5, -1.9e-5, 0.0, 1.9e-5, 2.1e-5] {
                for (s, t) in [(-1e-7, 0.5), (0.2, 0.8), (0.5, 1.0000001)] {
                    let p = a.lerp(b, s) + normal * offset;
                    let q = a.lerp(b, t) + normal * offset;
                    let expected = edges.iter().any(|edge| {
                        on_segment(xz(vec(edge[0])), xz(vec(edge[1])), p)
                            && on_segment(xz(vec(edge[0])), xz(vec(edge[1])), q)
                    });
                    assert_eq!(index.contains(p, q), expected, "{p:?} {q:?}");
                }
            }
        }
    }

    #[test]
    fn perimeter_bucket_scan_matches_sorted_query_at_borders_and_source_holes() {
        let mut source = query_source(&vec![0.0; (N + 1) * (N + 1)]);
        source.indices = source
            .indices
            .chunks_exact(6)
            .enumerate()
            .filter(|(i, _)| {
                let (x, z) = (i % N, i / N);
                !((8..12).contains(&x) && (7..11).contains(&z) || x < 3 && (18..21).contains(&z))
            })
            .flat_map(|(_, indices)| indices.iter().copied())
            .collect();
        for skew in [0.0, 0.013] {
            let mut mesh = source.clone();
            for vertex in &mut mesh.vertices {
                vertex.pos[0] += skew * vertex.pos[2];
            }
            let edges = perimeter(&mesh);
            let index = Perimeter::new(edges.clone());
            let check = |a: DVec2, b: DVec2| {
                let matches = |edge: &[SurfaceVertex; 2]| {
                    let p = xz(vec(edge[0]));
                    let q = xz(vec(edge[1]));
                    on_segment(p, q, a) && on_segment(p, q, b)
                };
                let sorted = index
                    .buckets
                    .candidates(a.min(b), a.max(b))
                    .into_iter()
                    .any(|id| matches(&index.edges[id]));
                let exhaustive = edges.iter().any(matches);
                assert_eq!(sorted, exhaustive, "broad phase: {a:?} {b:?}");
                assert_eq!(index.contains(a, b), sorted, "bucket scan: {a:?} {b:?}");
                assert_eq!(index.contains(b, a), sorted, "reversed: {a:?} {b:?}");
            };
            for edge in &edges {
                let a = xz(vec(edge[0]));
                let b = xz(vec(edge[1]));
                let tangent = (b - a).normalize();
                let normal = DVec2::new(-tangent.y, tangent.x);
                for offset in [-2.1e-5, -2e-5, -EPS, 0.0, EPS, 2e-5, 2.1e-5] {
                    check(a + normal * offset, b + normal * offset);
                    check(a - tangent * offset, a.lerp(b, 0.5));
                    check(a.lerp(b, 0.5), b + tangent * offset);
                }
            }
            for (a, b) in [
                ([0.0, 0.0], [N as f64, 0.0]),
                ([-0.25, 0.0], [0.25, 0.0]),
                ([N as f64, 31.0], [N as f64, 32.0]),
                ([8.0, 7.25], [8.0, 7.75]),
                ([8.0, 7.25], [8.0, 10.75]),
                ([8.25, 7.25], [11.75, 10.75]),
                ([0.0, 18.0], [3.0, 18.0]),
                ([3.0, 18.25], [3.0, 18.75]),
                ([15.25, 15.25], [16.75, 16.75]),
            ] {
                check(DVec2::from_array(a), DVec2::from_array(b));
            }
        }
        assert!(!Perimeter::new(Vec::new()).contains(DVec2::ZERO, DVec2::splat(N as f64)));
    }

    #[test]
    fn independent_terrain_pieces_keep_matching_water_boundaries_and_closed_skirts() {
        let (frame, _) = fixture(14, 0.0);
        let scale = frame.grid_scale.y;
        let v = |x: f32, z: f32| SurfaceVertex {
            pos: [x, (35.0 / scale) as f32, z],
            face_ao: 2 | (3 << 3) | (u32::from(material::GRASS) << 5),
            rgba: [77, 105, 62, 255],
        };
        let source = |x: f32| MeshBuffers {
            vertices: vec![v(x, 0.0), v(x + 1.0, 0.0), v(x + 1.0, 1.0), v(x, 1.0)],
            indices: vec![0, 3, 2, 0, 2, 1],
        };
        let corners = [
            [0.25_f32, 0.25_f32],
            [1.75, 0.4375],
            [1.75, 0.6875],
            [0.25, 0.5],
        ];
        let water: Vec<_> = [[0, 2, 1], [0, 3, 2]]
            .into_iter()
            .map(|ids| WaterTriangle {
                vertices: ids.map(|i| {
                    [
                        corners[i][0],
                        ((30.0 + 0.2 * f64::from(corners[i][1])) / scale) as f32,
                        corners[i][1],
                    ]
                }),
                kind: vearth_data::vectors::WaterRegionKind::Canal,
                depth_m: 1.2,
            })
            .collect();
        let left = carve(&frame, &source(0.0), &water, &mut [u32::MAX; N]).unwrap();
        let right = carve(&frame, &source(1.0), &water, &mut [u32::MAX; N]).unwrap();
        let shared = |result: &Conditioned| {
            result
                .boundary
                .iter()
                .copied()
                .filter(|edge| edge.iter().all(|v| v.pos[0] == 1.0))
                .collect::<Vec<_>>()
        };
        let left_edges = shared(&left);
        let right_edges = shared(&right);
        for edges in [&left_edges, &right_edges] {
            let total = edges
                .iter()
                .map(|edge| f64::from((edge[0].pos[2] - edge[1].pos[2]).abs()))
                .sum::<f64>();
            assert!((total - 1.0).abs() < 1e-6, "boundary coverage {total}");
        }
        let boundary_height = |edges: &[[SurfaceVertex; 2]], z: f64| {
            edges
                .iter()
                .find_map(|edge| {
                    let [a, b] = edge.map(vec);
                    if z < a.z.min(b.z) || z > a.z.max(b.z) || a.z == b.z {
                        return None;
                    }
                    Some(a.y + (b.y - a.y) * (z - a.z) / (b.z - a.z))
                })
                .expect("shared boundary gap")
        };
        for i in 0..100 {
            let z = (i as f64 + 0.31) / 100.0;
            let a = boundary_height(&left_edges, z) * scale;
            let b = boundary_height(&right_edges, z) * scale;
            let expected = if z > 0.34375 && z < 0.59375 {
                28.8 + 0.2 * z
            } else {
                35.0
            };
            assert!(
                (a - b).abs() < 1e-4 && (a - expected).abs() < 1e-4,
                "boundary at {z}: {a}, {b}, expected {expected}"
            );
        }
        for mut result in [left, right] {
            let before = result.mesh.vertices.len();
            let boundaries = result.boundary.clone();
            append_skirts(&mut result.mesh, result.boundary, 0.25);
            assert_eq!(result.mesh.vertices.len(), before + boundaries.len() * 4);
            for (quad, edge) in result.mesh.vertices[before..]
                .chunks_exact(4)
                .zip(boundaries)
            {
                assert_eq!(quad[0].pos, edge[0].pos);
                assert_eq!(quad[1].pos, edge[1].pos);
                assert_eq!(quad[2].pos[1], edge[1].pos[1] - 0.25);
                assert_eq!(quad[3].pos[1], edge[0].pos[1] - 0.25);
            }
        }
    }

    fn raster_family_weights(mesh: &MeshBuffers, point: DVec2) -> Option<[f64; 4]> {
        mesh.indices.chunks_exact(3).find_map(|ids| {
            let vertices = [0, 1, 2].map(|i| mesh.vertices[ids[i] as usize]);
            let points = vertices.map(vec);
            if (points[1] - points[0]).cross(points[2] - points[0]).y <= 1e-12 {
                return None;
            }
            let w = weights(points, point);
            if w.iter().any(|&w| w < -1e-8) {
                return None;
            }
            let mut lanes = [0.0; 4];
            for i in 0..3 {
                lanes[((vertices[i].face_ao >> 3) & 3) as usize] += w[i];
            }
            Some(lanes)
        })
    }
    #[test]
    fn mixed_terrain_families_keep_the_source_palette_across_water_partitioning() {
        let vertex = |x, z, family: u32| SurfaceVertex {
            pos: [x, 10.0, z],
            face_ao: vearth_core::mesh::TERRAIN_BLEND_FACE
                | (family << 3)
                | (u32::from(material::GRASS) << 5),
            rgba: [77, 105, 62, 255],
        };
        // The independently conditioned triangles share a horizontal 0→1
        // family edge. Interior pond events can subdivide only its lower side.
        let sources = [
            [
                vertex(0.0, 0.0, 2),
                vertex(0.0, 1.0, 0),
                vertex(1.0, 1.0, 1),
            ],
            [
                vertex(0.0, 1.0, 0),
                vertex(0.0, 2.0, 3),
                vertex(1.0, 1.0, 1),
            ],
        ];
        let rectangle = |x0: f64, z0: f64, x1: f64, z1: f64| {
            let p = [
                DVec2::new(x0, z0),
                DVec2::new(x1, z0),
                DVec2::new(x1, z1),
                DVec2::new(x0, z1),
            ];
            [[p[0], p[1], p[2]], [p[0], p[2], p[3]]]
        };
        for (name, triangles) in [
            ("interior_pond", rectangle(0.2, 0.65, 0.45, 0.85).to_vec()),
            ("crossing_canal", rectangle(0.2, 0.4, 0.3, 1.6).to_vec()),
        ] {
            let water: Vec<_> = triangles
                .into_iter()
                .map(|triangle| WaterPlane {
                    points: triangle.map(|p| DVec3::new(p.x, 5.0, p.y)),
                    polygon: canonical(triangle.to_vec()),
                    depth: 1.2,
                    ocean: false,
                })
                .collect();
            let build = |source: [SurfaceVertex; 3], sweep: bool| {
                let points = source.map(vec);
                let domain = canonical(points.map(xz).to_vec());
                let regions: Vec<_> = water
                    .iter()
                    .enumerate()
                    .filter_map(|(id, plane)| {
                        let polygon = intersection(&domain, &plane.polygon);
                        (polygon.len() >= 3).then_some(WetRegion { polygon, water: id })
                    })
                    .collect();
                let patches = if sweep {
                    ownership_sweep(&domain, &regions)
                } else {
                    sequential_ownership(&domain, &regions)
                };
                let mut mesh = MeshBuffers::default();
                for patch in patches {
                    emit_conditioned_patch(
                        &mut mesh,
                        source,
                        &patch.polygon,
                        patch.water.map(|id| &water[id]),
                    );
                }
                let palette = source
                    .iter()
                    .fold(0_u32, |mask, v| mask | (1 << ((v.face_ao >> 3) & 3)));
                let emitted = mesh
                    .vertices
                    .iter()
                    .fold(0_u32, |mask, v| mask | (1 << ((v.face_ao >> 3) & 3)));
                assert_eq!(emitted, palette, "source family palette changed");
                for &v in &mesh.vertices {
                    assert_eq!(v.face_ao & 7, vearth_core::mesh::TERRAIN_BLEND_FACE);
                    assert_eq!((v.face_ao >> 5) & 255, u32::from(material::GRASS));
                }
                mesh
            };
            let old = sources.map(|source| build(source, false));
            let new = sources.map(|source| build(source, true));
            let mut maximum = 0.0_f64;
            let mut squared = 0.0;
            let mut count = 0;
            for i in 0..2 {
                let domain = canonical(sources[i].map(vec).map(xz).to_vec());
                for z in 0..80 {
                    for x in 0..80 {
                        let p = DVec2::new((x as f64 + 0.371) / 80.0, (z as f64 + 0.619) / 40.0);
                        if !strict_inside(&domain, p) {
                            continue;
                        }
                        let a =
                            raster_family_weights(&old[i], p).expect("reference family coverage");
                        let b = raster_family_weights(&new[i], p).expect("sweep family coverage");
                        assert!((b.iter().sum::<f64>() - 1.0).abs() < 1e-7);
                        assert!(b.iter().all(|&v| (-1e-7..=1.0000001).contains(&v)));
                        for channel in 0..4 {
                            let delta = (a[channel] - b[channel]).abs();
                            maximum = maximum.max(delta);
                            squared += delta * delta;
                            count += 1;
                        }
                    }
                }
            }
            let mut seam = [0.0_f64; 2];
            for x in 0..160 {
                let p = DVec2::new((x as f64 + 0.371) / 160.0, 1.0);
                for (i, meshes) in [&old, &new].into_iter().enumerate() {
                    let a =
                        raster_family_weights(&meshes[0], p).expect("lower family edge coverage");
                    let b =
                        raster_family_weights(&meshes[1], p).expect("upper family edge coverage");
                    for channel in 0..4 {
                        seam[i] = seam[i].max((a[channel] - b[channel]).abs());
                    }
                }
            }
            println!(
                "terrain family {name}: max old/new lane delta={maximum:.6}, RMS={:.6}; shared-edge old/new maxima={:.6}/{:.6}",
                (squared / f64::from(count)).sqrt(),
                seam[0],
                seam[1]
            );
        }
    }

    #[test]
    fn exact_interior_edge_cancellation_preserves_union_shorelines() {
        let triangle = |p: [[f64; 2]; 3]| p.map(DVec2::from_array);
        let rectangle = |x0, z0, x1, z1| {
            [
                triangle([[x0, z0], [x1, z0], [x1, z1]]),
                triangle([[x0, z0], [x1, z1], [x0, z1]]),
            ]
        };
        let square = rectangle(2.0, 2.0, 4.0, 4.0).to_vec();
        let mut duplicate = square.clone();
        duplicate.extend(square.iter().copied());
        let mut junction = rectangle(2.0, 2.0, 3.0, 4.0).to_vec();
        junction.extend(rectangle(3.0, 2.0, 4.0, 3.0));
        junction.extend(rectangle(3.0, 3.0, 4.0, 4.0));
        let mut overlap = square.clone();
        overlap.push(triangle([[3.0, 1.0], [5.0, 3.0], [3.0, 5.0]]));
        let mut partial = square.clone();
        partial.extend(rectangle(4.0, 2.5, 5.0, 3.5));
        let sliver = vec![
            triangle([[2.0, 2.0], [4.0, 2.0], [3.0, 4.0]]),
            triangle([[2.0, 2.0], [4.0, 2.0], [3.0, 2.0 - 1e-8]]),
        ];
        for triangles in [square, duplicate, junction, overlap, partial, sliver] {
            for reversed in [false, true] {
                let water: Vec<_> = triangles
                    .iter()
                    .enumerate()
                    .map(|(i, triangle)| {
                        let mut triangle = *triangle;
                        if reversed && i % 2 == 0 {
                            triangle.swap(0, 2);
                        }
                        WaterPlane {
                            points: triangle.map(|p| DVec3::new(p.x, i as f64, p.y)),
                            polygon: canonical(triangle.to_vec()),
                            depth: if i % 2 == 0 { 0.05 } else { 1.2 },
                            ocean: i % 2 == 0,
                        }
                    })
                    .collect();
                let mut buckets = Buckets::new();
                for (i, plane) in water.iter().enumerate() {
                    let (lo, hi) = bounds(plane.polygon.iter().copied());
                    buckets.insert(i, lo, hi);
                }
                let reference = shoreline_arrangement(
                    &water,
                    &buckets,
                    &Perimeter::new(Vec::new()),
                    &HashSet::new(),
                );
                let actual = shorelines(&water, &buckets, &Perimeter::new(Vec::new()));
                let signature = |shores: Vec<Shore>| {
                    shores
                        .into_iter()
                        .map(|s| {
                            (
                                s.a.to_array().map(f64::to_bits),
                                s.b.to_array().map(f64::to_bits),
                                s.water,
                            )
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(signature(actual), signature(reference));
                if triangles.len() > 2 {
                    assert!(!interior_edges(&water).is_empty());
                }
            }
        }
    }

    #[test]
    fn finely_partitioned_canal_preserves_wetland_plane_coverage_and_bed_height() {
        let (frame, _) = fixture(14, 0.0);
        let local_height = |meters: f64| (meters / frame.grid_scale.y) as f32;
        let vertex = |x, z| SurfaceVertex {
            pos: [x, local_height(35.0), z],
            face_ao: 2 | (3 << 3) | (u32::from(material::WETLAND) << 5),
            rgba: [77, 105, 62, 255],
        };
        let source = MeshBuffers {
            vertices: vec![
                vertex(0.0, 0.0),
                vertex(1.0, 0.0),
                vertex(1.0, 1.0),
                vertex(0.0, 1.0),
            ],
            indices: vec![0, 3, 2, 0, 2, 1],
        };
        let mut water = Vec::new();
        for i in 0..64 {
            let point = |t: f32, offset| [t, local_height(30.0), t * 0.7 + 0.1 + offset];
            let a = i as f32 / 64.0;
            let b = (i + 1) as f32 / 64.0;
            let quad = [
                point(a, 0.0),
                point(b, 0.0),
                point(b, 0.025),
                point(a, 0.025),
            ];
            for ids in [[0, 2, 1], [0, 3, 2]] {
                water.push(WaterTriangle {
                    vertices: ids.map(|j| quad[j]),
                    kind: vearth_data::vectors::WaterRegionKind::Canal,
                    depth_m: 1.2,
                });
            }
        }
        let mut columns = [u32::MAX; N];
        let result = carve(&frame, &source, &water, &mut columns).unwrap();
        let area: f64 = result
            .mesh
            .indices
            .chunks_exact(3)
            .map(|ids| {
                let p = [0, 1, 2].map(|i| vec(result.mesh.vertices[ids[i] as usize]));
                (p[1] - p[0]).cross(p[2] - p[0]).y.max(0.0) * 0.5
            })
            .sum();
        assert!((area - 1.0).abs() < 2e-5, "surface coverage {area}");
        for p in [DVec2::new(0.05, 0.05), DVec2::new(0.95, 0.95)] {
            assert!((top(&result.mesh, p).unwrap() * frame.grid_scale.y - 35.0).abs() < 1e-4);
        }
        let bed = top(&result.mesh, DVec2::new(0.5, 0.4625)).unwrap();
        assert!((bed * frame.grid_scale.y - 28.8).abs() < 1e-4);
        assert!(
            result
                .mesh
                .vertices
                .iter()
                .all(|v| ((v.face_ao >> 5) & 255) == u32::from(material::WETLAND))
        );
        assert_eq!(columns[0], u32::MAX & !1);
        assert!(columns[1..].iter().all(|&row| row == u32::MAX));
    }
    fn assert_visible(
        frame: &ChunkFrame,
        water: &water_surface::WaterGeometry,
        terrain: &terrain_surface::TerrainGeometry,
    ) {
        let collider =
            crate::collision::SurfaceCollision::from_triangles(terrain.triangles.clone());
        let metric = crate::collision::MetricFrame::of(frame);
        for triangle in &water.triangles {
            let p = triangle
                .vertices
                .map(|p| DVec3::from_array(p.map(f64::from)));
            let center = (p[0] + p[1] + p[2]) / 3.0;
            let bed = top(&terrain.mesh, xz(center)).expect("water must have a closed terrain bed");
            assert!(
                (center.y - bed) * frame.grid_scale.y > 1.18,
                "hidden water at L{}: {:?}, bed={bed}",
                frame.addr.cell.level,
                center
            );
            let at = metric.point_from_local([center.x as f32, bed as f32, center.z as f32]);
            let heightfield = terrain.collision.as_ref().and_then(|terrain| {
                terrain
                    .raycast(
                        frame,
                        [f64::from(at[0]), f64::from(at[1]) + 4.0, f64::from(at[2])],
                        [0.0, -1.0, 0.0],
                        8.0,
                    )
                    .map(|t| f64::from(at[1]) + 4.0 - t)
            });
            let physical = collider
                .top(f64::from(at[0]), f64::from(at[2]))
                .into_iter()
                .chain(heightfield)
                .reduce(f64::max)
                .expect("visible bed must remain collidable");
            assert!(
                (physical - f64::from(at[1])).abs() < 0.003,
                "render and exact terrain collision disagree"
            );
        }
    }
    fn assert_dry(
        frame: &ChunkFrame,
        source: &TerrainSurfaceData,
        terrain: &terrain_surface::TerrainGeometry,
        points: &[[f64; 2]],
    ) {
        let original = terrain_surface::build(frame, source, &appearance());
        for &point in points {
            let p = DVec2::from_array(point);
            let a = top(&original.mesh, p).unwrap();
            let b = top(&terrain.mesh, p).unwrap();
            assert!(
                (a - b).abs() * frame.grid_scale.y < 0.0001,
                "dry ground moved at {point:?}: {a} -> {b}"
            );
        }
    }
    #[test]
    fn tiny_pond_inside_one_quad_carves_only_its_exact_footprint() {
        for level in [14, 17, 19] {
            let (frame, source) = fixture(level, 0.0);
            let mut pack = FeaturePack::empty();
            pack.water
                .push(Water::inland(vec![ring(&frame, 15.2, 15.2, 15.8, 15.8)]));
            let (water, terrain) = geometry(&frame, &source, &pack);
            assert_visible(&frame, &water, &terrain);
            assert_dry(
                &frame,
                &source,
                &terrain,
                &[[15.1, 15.5], [15.5, 15.9], [15.9, 15.5], [15.5, 15.1]],
            );
            let mask = &terrain.collision.as_ref().unwrap().source_columns;
            assert_eq!(mask[15], u32::MAX & !(1 << 15));
            assert_eq!(mask[14], u32::MAX);
            // No terrain-grid corner lies within this pond: a corner-only bed
            // adjustment cannot satisfy any of the visibility assertions.
            assert!(
                water
                    .triangles
                    .iter()
                    .all(|triangle| triangle
                        .vertices
                        .iter()
                        .all(|p| p[0] > 15.0 && p[0] < 16.0 && p[2] > 15.0 && p[2] < 16.0))
            );
        }
    }
    #[test]
    fn narrow_diagonal_river_has_visible_bed_without_lowering_adjacent_banks() {
        for level in [14, 17, 19] {
            let (frame, source) = fixture(level, 0.0);
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = crate::tier0::GridProj::of(&frame, lat, lon);
            let mut pack = FeaturePack::empty();
            pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
            pack.watercourses.push(Watercourse {
                paths: vec![vec![en(&frame, [14.2, 14.38]), en(&frame, [17.8, 17.98])]],
                kind: WatercourseKind::River,
                width_m: (projection.step_m * 0.1) as f32,
                identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(7),
                style_identity: 7,
                source_tile: (15, 0, 0),
                authority_zoom: 15,
                source_feature_index: 0,
                source_min_zoom: None,
                fragment_count: 1,
                intermittent: false,
                is_bridge: false,
            });
            let (water, terrain) = geometry(&frame, &source, &pack);
            assert_visible(&frame, &water, &terrain);
            assert_dry(
                &frame,
                &source,
                &terrain,
                &[[15.5, 15.3], [15.5, 15.9], [16.5, 16.3], [16.5, 16.9]],
            );
        }
    }
    #[test]
    fn lake_holes_remain_dry_and_source_shorelines_have_no_internal_bank_walls() {
        for level in [14, 17, 19] {
            let (frame, source) = fixture(level, 0.0);
            let mut pack = FeaturePack::empty();
            pack.water.push(Water::inland(vec![
                ring(&frame, 14.2, 14.2, 16.8, 16.8),
                ring(&frame, 15.2, 15.2, 15.8, 15.8),
            ]));
            let (water, terrain) = geometry(&frame, &source, &pack);
            assert_visible(&frame, &water, &terrain);
            assert_dry(
                &frame,
                &source,
                &terrain,
                &[[15.5, 15.5], [14.1, 15.5], [16.9, 15.5]],
            );
            for ids in terrain.mesh.indices.chunks_exact(3) {
                let p = [0, 1, 2].map(|i| vec(terrain.mesh.vertices[ids[i] as usize]));
                let n = (p[1] - p[0]).cross(p[2] - p[0]);
                if n.y.abs() > 1e-10 {
                    continue;
                }
                let center = (p[0] + p[1] + p[2]) / 3.0;
                if center.x <= 0.01 || center.x >= 31.99 || center.z <= 0.01 || center.z >= 31.99 {
                    continue;
                }
                assert!(
                    [14.2, 16.8, 15.2, 15.8]
                        .iter()
                        .any(|&edge| (center.x - edge).abs() < 0.001
                            || (center.z - edge).abs() < 0.001),
                    "internal water triangulation created a bank wall at {center:?}"
                );
            }
        }
    }
    #[test]
    fn sloped_banks_keep_dry_planes_and_existing_deep_bathymetry() {
        for level in [14, 17, 19] {
            let (frame, mut source) = fixture(level, 0.0);
            for i in 0..source.heights_m.len() {
                source.heights_m[i] = (25.0 + ((i % (N + 1)) as f64 - 15.0) * 5.0) as f32;
            }
            let mut pack = FeaturePack::empty();
            pack.water
                .push(Water::inland(vec![ring(&frame, 14.2, 15.2, 16.8, 15.8)]));
            let (water, terrain) = geometry(&frame, &source, &pack);
            assert_visible(&frame, &water, &terrain);
            assert_dry(
                &frame,
                &source,
                &terrain,
                &[[14.5, 15.1], [16.5, 15.9], [14.5, 15.5]],
            );
            let original = terrain_surface::build(&frame, &source, &appearance());
            let p = DVec2::new(16.5, 15.5);
            assert!(
                top(&original.mesh, p).unwrap() - top(&terrain.mesh, p).unwrap()
                    > 1.0 / frame.grid_scale.y
            );
        }
    }
    #[test]
    fn submerged_ground_fast_path_preserves_vertices_skirts_and_collision_exactly() {
        for level in [5, 14, 19] {
            let (frame, mut source) = fixture(level, 0.0);
            source.heights_m.fill(-100.0);
            let water = water_surface::build(
                &frame,
                &source,
                None,
                &|_, _| Some(-100.0),
                &|_, _, _, _, _| SurfaceAppearance {
                    material: material::WATER,
                    tint: [30, 90, 125],
                },
            );
            let original = terrain_surface::build(&frame, &source, &appearance());
            let mut mask = source.source_columns;
            assert!(carve(&frame, &original.mesh, &water.triangles, &mut mask).is_none());
            assert_eq!(mask, source.source_columns);
            let output =
                terrain_surface::build_with_water(&frame, &source, &appearance(), &water.triangles);
            assert_eq!(output.mesh.vertices, original.mesh.vertices);
            assert_eq!(output.mesh.indices, original.mesh.indices);
            assert_eq!(output.triangles, original.triangles);
            assert_eq!(output.collision, original.collision);
        }
    }

    #[test]
    fn repeated_river_vertices_keep_geometry_and_invalid_supported_paths_are_incomplete() {
        let (frame, source) = fixture(17, 0.0);
        let mut pack = FeaturePack::empty();
        pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
        pack.watercourses.push(Watercourse {
            paths: vec![vec![en(&frame, [14.2, 14.4]), en(&frame, [17.8, 18.0])]],
            kind: WatercourseKind::River,
            width_m: 0.5,
            identity: vearth_data::protomaps_semantics::SourceFeatureIdentity::Provided(8),
            style_identity: 8,
            source_tile: (15, 0, 0),
            authority_zoom: 15,
            source_feature_index: 0,
            source_min_zoom: None,
            fragment_count: 1,
            intermittent: false,
            is_bridge: false,
        });
        let render = |pack: &FeaturePack| {
            water_surface::build(
                &frame,
                &source,
                Some(pack),
                &|_, _| Some(30.0),
                &|_, _, _, _, _| SurfaceAppearance {
                    material: material::WATER,
                    tint: [30, 90, 125],
                },
            )
        };
        let clean = render(&pack);
        assert!(clean.complete);
        assert!(!clean.triangles.is_empty());
        let path = &mut pack.watercourses[0].paths[0];
        path.insert(0, path[0]);
        path.push(*path.last().unwrap());
        let repeated = render(&pack);
        assert!(repeated.complete);
        assert_eq!(repeated.triangles, clean.triangles);
        assert_eq!(repeated.mesh.vertices, clean.mesh.vertices);
        pack.watercourses[0]
            .paths
            .push(vec![[f32::NAN, 0.0], [1.0, 0.0]]);
        let invalid = render(&pack);
        assert!(!invalid.complete);
        assert_eq!(
            invalid.triangles, clean.triangles,
            "one malformed path must not discard another valid river fragment"
        );
        pack.watercourses[0].paths.pop();
        pack.watercourses[0].width_m = f32::NAN;
        let invalid = render(&pack);
        assert!(!invalid.complete);
        assert!(invalid.triangles.is_empty());
        pack.watercourses[0].intermittent = true;
        assert!(
            render(&pack).complete,
            "semantically excluded water does not require valid surface geometry"
        );
    }
}
