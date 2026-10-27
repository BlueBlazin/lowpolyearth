//! Geographically streamed Great Wall geometry from the pinned OSM network.
//!
//! Source coordinates and dimensions are geographic/metres, independent of
//! terrain cells. Shared stations use one DEM resolution so adjacent cells
//! clip the same wall even when their terrain LOD differs.
use std::{collections::HashMap, io::Read, sync::OnceLock};

use glam::{DMat3, DVec2, DVec3};
use vearth_core::{
    ellipsoid::{self, MEAN_RADIUS},
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal},
};

use crate::collision::{CollisionTriangle, MetricFrame};

pub(crate) const SAMPLE_M: f64 = 8.0;
const BIN: f64 = 128.0;
const MAX_PITCH_M: f64 = 48.0;
const GUARD_M: f64 = 45.0;

#[derive(Clone, Copy, Debug)]
struct Point {
    lat: f64,
    lon: f64,
}
impl Point {
    fn offset(self, en: DVec2) -> Self {
        Self {
            lat: self.lat + en.y / MEAN_RADIUS,
            lon: self.lon + en.x / (MEAN_RADIUS * self.lat.cos()),
        }
    }
    fn delta(self, other: Self) -> DVec2 {
        DVec2::new(
            (other.lon - self.lon) * MEAN_RADIUS * ((other.lat + self.lat) * 0.5).cos(),
            (other.lat - self.lat) * MEAN_RADIUS,
        )
    }
    fn lerp(self, other: Self, t: f64) -> Self {
        Self {
            lat: self.lat + (other.lat - self.lat) * t,
            lon: self.lon + (other.lon - self.lon) * t,
        }
    }
    fn world(self, height: f64) -> DVec3 {
        ellipsoid::geodetic_to_ecef(self.lat, self.lon, height)
    }
    fn bin(self) -> (i32, i32) {
        (
            (self.lat.to_degrees() * BIN).floor() as i32,
            (self.lon.to_degrees() * BIN).floor() as i32,
        )
    }
}

#[derive(Clone, Debug)]
struct Segment {
    a: Point,
    b: Point,
    na: DVec2,
    nb: DVec2,
    width: f64,
    height: f64,
    phase: f64,
    seed: u32,
    style: u8,
}
#[derive(Clone, Debug)]
struct Tower {
    p: Point,
    width: f64,
    depth: f64,
    angle: f64,
    height: f64,
    seed: u32,
    style: u8,
}
impl Tower {
    fn point(&self, x: f64, z: f64) -> Point {
        let (s, c) = self.angle.sin_cos();
        self.p.offset(DVec2::new(x * c - z * s, x * s + z * c))
    }
    fn contains(&self, p: Point, margin: f64) -> bool {
        let d = self.p.delta(p);
        let (s, c) = self.angle.sin_cos();
        (d.x * c + d.y * s).abs() <= self.width * 0.5 + margin
            && (-d.x * s + d.y * c).abs() <= self.depth * 0.5 + margin
    }
    fn samples(&self) -> [Point; 5] {
        [
            self.p,
            self.point(-self.width * 0.5, -self.depth * 0.5),
            self.point(self.width * 0.5, -self.depth * 0.5),
            self.point(self.width * 0.5, self.depth * 0.5),
            self.point(-self.width * 0.5, self.depth * 0.5),
        ]
    }
}
struct Network {
    segments: Vec<Segment>,
    towers: Vec<Tower>,
    bins: HashMap<(i32, i32), Vec<u32>>,
}

fn network() -> &'static Network {
    static DATA: OnceLock<Network> = OnceLock::new();
    DATA.get_or_init(|| {
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(vearth_platform::asset_bytes!(
            "../assets/great-wall/network.gwl"
        ))
        .read_to_end(&mut raw)
        .expect("bundled Great Wall pack decompresses");
        assert_eq!(&raw[..4], b"GWL1");
        let mut cursor = 4;
        fn u32_at(raw: &[u8], cursor: &mut usize) -> u32 {
            let n = u32::from_le_bytes(raw[*cursor..*cursor + 4].try_into().unwrap());
            *cursor += 4;
            n
        }
        fn f(raw: &[u8], cursor: &mut usize) -> f64 {
            f64::from(f32::from_bits(u32_at(raw, cursor)))
        }
        fn point(raw: &[u8], cursor: &mut usize) -> Point {
            Point {
                lat: (f64::from(u32_at(raw, cursor) as i32) * 1e-7).to_radians(),
                lon: (f64::from(u32_at(raw, cursor) as i32) * 1e-7).to_radians(),
            }
        }
        let ns = u32_at(&raw, &mut cursor) as usize;
        let nt = u32_at(&raw, &mut cursor) as usize;
        assert_eq!(raw.len(), 12 + ns * 49 + nt * 29);
        let mut result = Network {
            segments: Vec::with_capacity(ns),
            towers: Vec::with_capacity(nt),
            bins: HashMap::new(),
        };
        for _ in 0..ns {
            let a = point(&raw, &mut cursor);
            let b = point(&raw, &mut cursor);
            let na = DVec2::new(f(&raw, &mut cursor), f(&raw, &mut cursor));
            let nb = DVec2::new(f(&raw, &mut cursor), f(&raw, &mut cursor));
            let width = f(&raw, &mut cursor);
            let height = f(&raw, &mut cursor);
            let phase = f(&raw, &mut cursor);
            let seed = u32_at(&raw, &mut cursor);
            let style = raw[cursor];
            cursor += 1;
            result.segments.push(Segment {
                a,
                b,
                na,
                nb,
                width,
                height,
                phase,
                seed,
                style,
            });
        }
        for _ in 0..nt {
            let p = point(&raw, &mut cursor);
            let width = f(&raw, &mut cursor);
            let depth = f(&raw, &mut cursor);
            let angle = f(&raw, &mut cursor);
            let height = f(&raw, &mut cursor);
            let seed = u32_at(&raw, &mut cursor);
            let style = raw[cursor];
            cursor += 1;
            result.towers.push(Tower {
                p,
                width,
                depth,
                angle,
                height,
                seed,
                style,
            });
        }
        for (i, s) in result.segments.iter().enumerate() {
            let a = s.a.bin();
            let b = s.b.bin();
            for y in a.0.min(b.0)..=a.0.max(b.0) {
                for x in a.1.min(b.1)..=a.1.max(b.1) {
                    result.bins.entry((y, x)).or_default().push(i as u32);
                }
            }
        }
        for (i, t) in result.towers.iter().enumerate() {
            result
                .bins
                .entry(t.p.bin())
                .or_default()
                .push((ns + i) as u32);
        }
        result
    })
}

fn in_region(p: Point) -> bool {
    (26.9..48.8).contains(&p.lat.to_degrees()) && (91.1..124.8).contains(&p.lon.to_degrees())
}
fn local_candidates(p: Point, half: DVec2) -> Vec<u32> {
    if !in_region(p) {
        return Vec::new();
    }
    let low = p.offset(-half).bin();
    let high = p.offset(half).bin();
    let data = network();
    let mut ids = Vec::new();
    for y in low.0..=high.0 {
        for x in low.1..=high.1 {
            if let Some(bin) = data.bins.get(&(y, x)) {
                ids.extend_from_slice(bin);
            }
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Only local source records; no country-sized landmark bounds or LOD lock.
pub(crate) struct Plan {
    ids: Vec<u32>,
}
impl Plan {
    pub(crate) fn for_frame(frame: &ChunkFrame) -> Self {
        if frame.sample_pitch_m() > MAX_PITCH_M {
            return Self { ids: Vec::new() };
        }
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let p = Point { lat, lon };
        let radius = frame.local_box_radius(DVec3::new(16., 0., 16.)) + GUARD_M;
        let ids = local_candidates(p, DVec2::splat(radius));
        if ids.is_empty() {
            return Self { ids };
        }
        let data = network();
        let margin = DVec2::new(GUARD_M / frame.grid_scale.x, GUARD_M / frame.grid_scale.z);
        let project = |p: Point| {
            let q = frame.ecef_to_local(p.world(0.));
            DVec2::new(q.x, q.z)
        };
        let ids = ids
            .into_iter()
            .filter(|&id| {
                let (a, b) = if (id as usize) < data.segments.len() {
                    let s = &data.segments[id as usize];
                    (project(s.a), project(s.b))
                } else {
                    let t = &data.towers[id as usize - data.segments.len()];
                    (project(t.p), project(t.p))
                };
                let low = a.min(b);
                let high = a.max(b);
                high.x >= -margin.x
                    && high.y >= -margin.y
                    && low.x <= 32. + margin.x
                    && low.y <= 32. + margin.y
            })
            .collect();
        Self { ids }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub(crate) fn sample_points(&self) -> Vec<(f64, f64)> {
        if self.is_empty() {
            return Vec::new();
        }
        let data = network();
        let mut points = Vec::new();
        for &id in &self.ids {
            if (id as usize) < data.segments.len() {
                let s = &data.segments[id as usize];
                for i in 0..=s.steps() {
                    points.extend(s.samples(i as f64 / s.steps() as f64));
                }
            } else {
                points.extend(data.towers[id as usize - data.segments.len()].samples());
            }
        }
        points.into_iter().map(|p| (p.lat, p.lon)).collect()
    }
    pub(crate) fn replaces_building(&self, lat: f64, lon: f64) -> bool {
        if self.is_empty() {
            return false;
        }
        let data = network();
        self.ids.iter().any(|&id| {
            id as usize >= data.segments.len()
                && data.towers[id as usize - data.segments.len()].contains(Point { lat, lon }, 1.)
        })
    }
}

/// Suppress roots only inside the wall/tower, retaining the surrounding forest.
pub(crate) fn excludes_tree(lat: f64, lon: f64) -> bool {
    let p = Point { lat, lon };
    let ids = local_candidates(p, DVec2::splat(GUARD_M));
    if ids.is_empty() {
        return false;
    }
    let data = network();
    ids.into_iter().any(|id| {
        if (id as usize) < data.segments.len() {
            let s = &data.segments[id as usize];
            let ab = s.a.delta(s.b);
            let ap = s.a.delta(p);
            let t = (ap.dot(ab) / ab.length_squared()).clamp(0., 1.);
            (ap - ab * t).length() < s.width * 0.5 + 4.5
        } else {
            data.towers[id as usize - data.segments.len()].contains(p, 4.5)
        }
    })
}
impl Segment {
    fn steps(&self) -> usize {
        (self.a.delta(self.b).length() / 4.).ceil().max(1.) as usize
    }
    fn samples(&self, t: f64) -> [Point; 3] {
        let p = self.a.lerp(self.b, t);
        let n = self.na.lerp(self.nb, t) * self.width * 0.5;
        [p, p.offset(-n), p.offset(n)]
    }
}

pub(crate) struct Geometry {
    pub mesh: MeshBuffers,
    pub collision: Vec<CollisionTriangle>,
}
struct Emitter<'a> {
    frame: &'a ChunkFrame,
    normal_to_local: DMat3,
    metric: MetricFrame,
    result: Geometry,
}
fn clip(poly: &[DVec3], axis: usize, limit: f64, greater: bool) -> Vec<DVec3> {
    let mut out = Vec::new();
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let inside = |p: DVec3| {
            if greater {
                p[axis] >= limit
            } else {
                p[axis] <= limit
            }
        };
        if inside(a) {
            out.push(a);
        }
        if inside(a) != inside(b) {
            let mut p = a.lerp(b, (limit - a[axis]) / (b[axis] - a[axis]));
            p[axis] = limit;
            out.push(p);
        }
    }
    out
}
impl<'a> Emitter<'a> {
    fn new(frame: &'a ChunkFrame) -> Self {
        Self {
            frame,
            normal_to_local: DMat3::from_mat4(frame.model_matrix()).transpose(),
            metric: MetricFrame::of(frame),
            result: Geometry {
                mesh: MeshBuffers::default(),
                collision: Vec::new(),
            },
        }
    }
    fn triangle(&mut self, mut p: [DVec3; 3], outward: DVec3, color: [u8; 3], material: u8) {
        let mut normal = (p[1] - p[0]).cross(p[2] - p[0]);
        if normal.length_squared() < 1e-12 {
            return;
        }
        if normal.dot(outward) < 0. {
            p.swap(1, 2);
            normal = -normal;
        }
        let normal = (self.normal_to_local * normal.normalize()).normalize();
        let mut polygon = p.map(|p| self.frame.ecef_to_local(p)).to_vec();
        for (axis, limit, greater) in [
            (0, 0., true),
            (0, 32., false),
            (2, 0., true),
            (2, 32., false),
        ] {
            polygon = clip(&polygon, axis, limit, greater);
            if polygon.len() < 3 {
                return;
            }
        }
        let base = self.result.mesh.vertices.len() as u32;
        self.result
            .mesh
            .vertices
            .extend(polygon.iter().map(|p| SurfaceVertex {
                pos: p.as_vec3().to_array(),
                face_ao: encode_surface_normal(
                    2 | (3 << 3) | (u32::from(material) << 5),
                    normal.as_vec3().to_array(),
                ),
                rgba: [color[0], color[1], color[2], 255],
            }));
        for i in 1..polygon.len() - 1 {
            let positions = [0, i, i + 1].map(|j| self.result.mesh.vertices[base as usize + j].pos);
            if let Some(t) = crate::collision::triangle_from_local(positions, &self.metric) {
                self.result
                    .mesh
                    .indices
                    .extend([base, base + i as u32, base + i as u32 + 1]);
                self.result.collision.push(t);
            }
        }
    }
    fn quad(&mut self, p: [DVec3; 4], out: DVec3, color: [u8; 3], material: u8) {
        self.triangle([p[0], p[1], p[2]], out, color, material);
        self.triangle([p[0], p[2], p[3]], out, color, material);
    }
    fn prism(&mut self, bottom: [DVec3; 4], top: [DVec3; 4], color: [u8; 3], material: u8) {
        let center = (bottom.into_iter().sum::<DVec3>() + top.into_iter().sum::<DVec3>()) / 8.;
        self.quad(
            top,
            top.into_iter().sum::<DVec3>() / 4. - center,
            color,
            material,
        );
        for i in 0..4 {
            let j = (i + 1) % 4;
            let q = [bottom[i], bottom[j], top[j], top[i]];
            self.quad(
                q,
                q.into_iter().sum::<DVec3>() / 4. - center,
                color,
                material,
            );
        }
    }
}
fn palette(style: u8, seed: u32) -> ([u8; 3], u8) {
    let variation = (seed.wrapping_mul(1664525) >> 28) as i16 - 7;
    let base: [i16; 3] = if style & 1 != 0 {
        [161, 128, 83]
    } else {
        [122, 120, 109]
    };
    (
        base.map(|v| (v + variation).clamp(0, 255) as u8),
        if style & 1 != 0 {
            crate::material::ROCK
        } else {
            crate::material::STONE_WALL
        },
    )
}

#[derive(Clone, Copy)]
struct Station {
    p: Point,
    n: DVec2,
    ground: [f64; 3],
    deck: f64,
}
impl Station {
    fn point(self, lateral: f64, height: f64) -> DVec3 {
        self.p.offset(self.n * lateral).world(height)
    }
    fn lerp(self, b: Self, t: f64) -> Self {
        Self {
            p: self.p.lerp(b.p, t),
            n: self.n.lerp(b.n, t),
            ground: std::array::from_fn(|i| self.ground[i] + (b.ground[i] - self.ground[i]) * t),
            deck: self.deck + (b.deck - self.deck) * t,
        }
    }
}
fn strip(
    em: &mut Emitter<'_>,
    a: Station,
    b: Station,
    low: f64,
    high: f64,
    bottom_a: f64,
    bottom_b: f64,
    top_a: f64,
    top_b: f64,
    color: [u8; 3],
    material: u8,
) {
    em.prism(
        [
            a.point(low, bottom_a),
            b.point(low, bottom_b),
            b.point(high, bottom_b),
            a.point(high, bottom_a),
        ],
        [
            a.point(low, top_a),
            b.point(low, top_b),
            b.point(high, top_b),
            a.point(high, top_a),
        ],
        color,
        material,
    );
}
fn wall(em: &mut Emitter<'_>, s: &Segment, elevation: &dyn Fn(f64, f64) -> f64) {
    let (color, material) = palette(s.style, s.seed);
    let intact = s.style == 0;
    let half = s.width * 0.5;
    let top = if s.style & 1 != 0 {
        half * 0.42
    } else {
        (half - 0.7).max(0.7)
    };
    let steps = s.steps();
    let stations = (0..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            let points = s.samples(t);
            let ground = points.map(|p| elevation(p.lat, p.lon));
            Station {
                p: points[0],
                n: s.na.lerp(s.nb, t),
                ground,
                deck: ground.into_iter().fold(f64::NEG_INFINITY, f64::max) + s.height
                    - if intact { 1.5 } else { 0. },
            }
        })
        .collect::<Vec<_>>();
    let length = s.a.delta(s.b).length();
    for (i, pair) in stations.windows(2).enumerate() {
        let [a, b] = [pair[0], pair[1]];
        em.prism(
            [
                a.point(-half, a.ground[1] - 1.2),
                b.point(-half, b.ground[1] - 1.2),
                b.point(half, b.ground[2] - 1.2),
                a.point(half, a.ground[2] - 1.2),
            ],
            [
                a.point(-top, a.deck),
                b.point(-top, b.deck),
                b.point(top, b.deck),
                a.point(top, a.deck),
            ],
            color,
            material,
        );
        if !intact {
            continue;
        }
        let coping = [157, 153, 137];
        for side in [-1., 1.] {
            let lo = if side < 0. { -top } else { top - 0.48 };
            let hi = lo + 0.48;
            strip(
                em,
                a,
                b,
                lo,
                hi,
                a.deck,
                b.deck,
                a.deck + 0.65,
                b.deck + 0.65,
                color,
                crate::material::BRICK_WALL,
            );
            // Crenellation phase belongs to the source way, never the cell/LOD.
            let start = s.phase + length * i as f64 / steps as f64;
            let end = s.phase + length * (i + 1) as f64 / steps as f64;
            let mut k = (start / 2.4).floor() as i32;
            while k as f64 * 2.4 < end {
                let l = start.max(k as f64 * 2.4);
                let r = end.min(k as f64 * 2.4 + 1.15);
                if r > l {
                    let ca = a.lerp(b, (l - start) / (end - start));
                    let cb = a.lerp(b, (r - start) / (end - start));
                    strip(
                        em,
                        ca,
                        cb,
                        lo,
                        hi,
                        ca.deck + 0.65,
                        cb.deck + 0.65,
                        ca.deck + 1.5,
                        cb.deck + 1.5,
                        color,
                        crate::material::BRICK_WALL,
                    );
                    if em.frame.sample_pitch_m() < 6. {
                        strip(
                            em,
                            ca,
                            cb,
                            lo - 0.05,
                            hi + 0.05,
                            ca.deck + 1.5,
                            cb.deck + 1.5,
                            ca.deck + 1.64,
                            cb.deck + 1.64,
                            coping,
                            crate::material::STONE_WALL,
                        );
                    }
                }
                k += 1;
            }
        }
    }
}

fn tower(em: &mut Emitter<'_>, t: &Tower, elevation: &dyn Fn(f64, f64) -> f64) {
    let samples = t.samples();
    let grounds = samples.map(|p| elevation(p.lat, p.lon));
    let base = grounds.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let (color, material) = palette(t.style, t.seed);
    let halfx = t.width * 0.5;
    let halfz = t.depth * 0.5;
    let top = base + t.height;
    let point = |x: f64, y: f64, z: f64| t.point(x, z).world(y);
    let box_at = |em: &mut Emitter<'_>,
                  x0: f64,
                  z0: f64,
                  x1: f64,
                  z1: f64,
                  y0: f64,
                  y1: f64,
                  col: [u8; 3]| {
        em.prism(
            [
                point(x0, y0, z0),
                point(x1, y0, z0),
                point(x1, y0, z1),
                point(x0, y0, z1),
            ],
            [
                point(x0, y1, z0),
                point(x1, y1, z0),
                point(x1, y1, z1),
                point(x0, y1, z1),
            ],
            col,
            material,
        );
    };
    if t.style != 0 {
        let inset = (if t.style & 1 != 0 { 1.3_f64 } else { 0.45 }).min(halfx.min(halfz) * 0.45);
        em.prism(
            [
                point(-halfx, grounds[1] - 1., -halfz),
                point(halfx, grounds[2] - 1., -halfz),
                point(halfx, grounds[3] - 1., halfz),
                point(-halfx, grounds[4] - 1., halfz),
            ],
            [
                point(-halfx + inset, top, -halfz + inset),
                point(halfx - inset, top, -halfz + inset),
                point(halfx - inset, top, halfz - inset),
                point(-halfx + inset, top, halfz - inset),
            ],
            color,
            material,
        );
        return;
    }
    let shoulder = top - 4.1;
    em.prism(
        [
            point(-halfx, grounds[1] - 1., -halfz),
            point(halfx, grounds[2] - 1., -halfz),
            point(halfx, grounds[3] - 1., halfz),
            point(-halfx, grounds[4] - 1., halfz),
        ],
        [
            point(-halfx + 0.3, shoulder, -halfz + 0.3),
            point(halfx - 0.3, shoulder, -halfz + 0.3),
            point(halfx - 0.3, shoulder, halfz - 0.3),
            point(-halfx + 0.3, shoulder, halfz - 0.3),
        ],
        color,
        material,
    );
    // A hollow chamber with real arched openings; the dark recess is geometry.
    let hx = (halfx - 0.35).max(0.8);
    let hz = (halfz - 0.35).max(0.8);
    let roof = top - 1.35;
    for side in 0..4 {
        let along = if side < 2 { hx } else { hz };
        let depth = if side < 2 { hz } else { hx };
        let map = |u: f64, v: f64, y: f64| {
            if side < 2 {
                point(u, y, if side == 0 { -v } else { v })
            } else {
                point(if side == 2 { -v } else { v }, y, u)
            }
        };
        let facebox = |em: &mut Emitter<'_>, u0: f64, u1: f64, y0: f64, y1: f64| {
            em.prism(
                [
                    map(u0, depth - 0.65, y0),
                    map(u1, depth - 0.65, y0),
                    map(u1, depth, y0),
                    map(u0, depth, y0),
                ],
                [
                    map(u0, depth - 0.65, y1),
                    map(u1, depth - 0.65, y1),
                    map(u1, depth, y1),
                    map(u0, depth, y1),
                ],
                color,
                crate::material::BRICK_WALL,
            );
        };
        let openings = if along > 4. { 3 } else { 1 };
        let spacing = 2. * along / (openings + 1) as f64;
        let radius = (spacing * 0.28).min(0.65);
        let sill = shoulder + 0.35;
        let spring = roof - 0.85;
        let mut previous = -along;
        for i in 1..=openings {
            let center = -along + i as f64 * spacing;
            facebox(em, previous, center - radius, shoulder, roof);
            facebox(em, center - radius, center + radius, shoulder, sill);
            for k in 0..8 {
                let x0 = -radius + 2. * radius * k as f64 / 8.;
                let x1 = -radius + 2. * radius * (k + 1) as f64 / 8.;
                let y0 = spring + (radius * radius - x0 * x0).max(0.).sqrt();
                let y1 = spring + (radius * radius - x1 * x1).max(0.).sqrt();
                let bottom = [
                    map(center + x0, depth - 0.65, y0),
                    map(center + x1, depth - 0.65, y1),
                    map(center + x1, depth, y1),
                    map(center + x0, depth, y0),
                ];
                let upper = [
                    map(center + x0, depth - 0.65, roof),
                    map(center + x1, depth - 0.65, roof),
                    map(center + x1, depth, roof),
                    map(center + x0, depth, roof),
                ];
                em.prism(bottom, upper, color, crate::material::BRICK_WALL);
                let inside =
                    (bottom[0] + bottom[1] + bottom[2] + bottom[3]) / 4. - t.p.world(roof + 1.);
                em.quad(bottom, inside, color, crate::material::BRICK_WALL);
            }
            previous = center + radius;
        }
        facebox(em, previous, along, shoulder, roof);
    }
    box_at(
        em,
        -hx - 0.12,
        -hz - 0.12,
        hx + 0.12,
        hz + 0.12,
        roof,
        roof + 0.25,
        [157, 151, 133],
    );
    for side in 0..4 {
        let length = if side < 2 { 2. * hx } else { 2. * hz };
        let count = (length / 2.).round().max(2.) as usize;
        for i in 0..count {
            let u0 = -length * 0.5 + length * i as f64 / count as f64;
            let u1 = u0 + length / count as f64 * 0.52;
            let (x0, z0, x1, z1) = match side {
                0 => (u0, -hz, u1, -hz + 0.55),
                1 => (u0, hz - 0.55, u1, hz),
                2 => (-hx, u0, -hx + 0.55, u1),
                _ => (hx - 0.55, u0, hx, u1),
            };
            box_at(em, x0, z0, x1, z1, roof + 0.25, top, color);
            box_at(
                em,
                x0 - 0.05,
                z0 - 0.05,
                x1 + 0.05,
                z1 + 0.05,
                top,
                top + 0.12,
                [157, 151, 133],
            );
        }
    }
}

pub(crate) fn generate(
    frame: &ChunkFrame,
    plan: &Plan,
    elevation: &dyn Fn(f64, f64) -> f64,
) -> Geometry {
    let mut em = Emitter::new(frame);
    if plan.is_empty() {
        return em.result;
    }
    let data = network();
    for &id in &plan.ids {
        if (id as usize) < data.segments.len() {
            wall(&mut em, &data.segments[id as usize], elevation);
        } else {
            tower(
                &mut em,
                &data.towers[id as usize - data.segments.len()],
                elevation,
            );
        }
    }
    crate::built_surface::compact_mesh(&mut em.result.mesh);
    em.result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use vearth_core::cell::{CellId, ChunkAddr};

    fn frame(lat: f64, lon: f64, level: u8) -> ChunkFrame {
        let p = Point {
            lat: lat.to_radians(),
            lon: lon.to_radians(),
        };
        ChunkFrame::of(ChunkAddr {
            cell: CellId::from_dir(p.world(0.).normalize(), level),
            k: 0,
        })
    }
    #[test]
    fn national_source_is_metric_and_has_eastern_and_western_routes() {
        let data = network();
        assert!(data.segments.len() > 200_000 && data.towers.len() > 5_000);
        let mut total = 0.;
        let mut styles = [0usize; 4];
        for s in &data.segments {
            let length = s.a.delta(s.b).length();
            assert!(length > 0. && length < 40.1);
            assert!((0.8..=12.).contains(&s.height));
            assert!((0.5..=20.).contains(&s.width));
            assert!(s.na.is_finite() && s.nb.is_finite());
            assert!(s.na.length() < 2.1 && s.nb.length() < 2.1);
            total += length;
            styles[s.style as usize] += 1;
        }
        assert!((6_700_000.0..6_950_000.0).contains(&total));
        assert!(styles.into_iter().all(|n| n > 1000));
        // Independent, widely separated mapped destinations, not one tourist section.
        for (lat, lon) in [
            (40.3544, 116.006),
            (40.4311994, 116.5715615),
            (39.8227976, 98.2073979),
            (40.0096249, 119.7535064),
        ] {
            let plan = Plan::for_frame(&frame(lat, lon, 15));
            assert!(!plan.is_empty(), "missing route near {lat},{lon}");
            assert!(plan.ids.len() < 2500);
        }
        for (lat, lon) in [(35.68, 139.76), (40.75, -73.98), (39.9, 116.4)] {
            assert!(Plan::for_frame(&frame(lat, lon, 17)).is_empty());
            assert!(!excludes_tree(lat.to_radians(), lon.to_radians()));
        }
    }
    #[test]
    fn all_runtime_dem_reads_are_planned_at_their_actual_resolution() {
        use vearth_data::terrarium::{TerrariumSource, sampling_tiles};
        for (lat, lon) in [
            (40.3554, 116.008),
            (40.4311994, 116.5715615),
            (39.8227976, 98.2073979),
        ] {
            for level in [15, 17, 19] {
                let frame = frame(lat, lon, level);
                let plan = Plan::for_frame(&frame);
                let points: HashSet<_> = plan
                    .sample_points()
                    .into_iter()
                    .map(|(a, b)| (a.to_bits(), b.to_bits()))
                    .collect();
                let footprint = crate::source_query_footprint(frame.addr.cell);
                let tiles: HashSet<_> =
                    crate::required_tiles(frame.addr.cell).into_iter().collect();
                generate(&frame, &plan, &|a, b| {
                    assert!(points.contains(&(a.to_bits(), b.to_bits())));
                    assert!(footprint.contains(a, b));
                    let zoom = TerrariumSource::zoom_for(SAMPLE_M, a);
                    assert!(
                        sampling_tiles(a, b, zoom)
                            .into_iter()
                            .all(|t| tiles.contains(&t))
                    );
                    100.
                });
            }
        }
    }
    #[test]
    fn wall_follows_slope_at_real_width_and_collision_matches_visible_surface() {
        let f = frame(40.3544, 116.006, 16);
        let (lat, lon) = f.column_latlon(0.5, 0.5);
        let center = Point { lat, lon };
        let s = Segment {
            a: center.offset(DVec2::new(0., -18.)),
            b: center.offset(DVec2::new(0., 18.)),
            na: DVec2::X,
            nb: DVec2::X,
            width: 6.8,
            height: 7.5,
            phase: 0.,
            seed: 1,
            style: 0,
        };
        let mut em = Emitter::new(&f);
        let elevation = |lat, lon| 100. + center.delta(Point { lat, lon }).y * 0.3;
        wall(&mut em, &s, &elevation);
        let mesh = &em.result.mesh;
        assert!(!mesh.indices.is_empty());
        assert_eq!(mesh.indices.len() / 3, em.result.collision.len());
        let mut width: f64 = 0.;
        let mut low_deck = Vec::new();
        let mut high_deck = Vec::new();
        for v in &mesh.vertices {
            assert!(v.pos.iter().all(|x| x.is_finite()));
            assert!((0.0..=32.).contains(&v.pos[0]) && (0.0..=32.).contains(&v.pos[2]));
            let world = f.local_to_ecef(DVec3::from_array(v.pos.map(f64::from)));
            let (lat, lon, h) = ellipsoid::ecef_to_geodetic(world);
            let d = center.delta(Point { lat, lon });
            width = width.max(d.x.abs() * 2.);
            let above = h - elevation(lat, lon);
            assert!((-1.21..7.66).contains(&above), "height {above}");
            if (above - 6.).abs() < 0.02 {
                if d.y < -17.9 {
                    low_deck.push(h);
                }
                if d.y > 17.9 {
                    high_deck.push(h);
                }
            }
        }
        assert!((width - 6.8).abs() < 0.02, "width {width}");
        assert!(!low_deck.is_empty() && !high_deck.is_empty());
        assert!((high_deck[0] - low_deck[0] - 10.8).abs() < 0.03);
        for (tri, collision) in mesh.indices.chunks_exact(3).zip(&em.result.collision) {
            assert_eq!(
                std::array::from_fn(|i| em
                    .metric
                    .point_from_local(mesh.vertices[tri[i] as usize].pos)),
                collision.vertices
            );
        }
    }
    #[test]
    fn neighboring_cells_clip_the_same_sloping_wall() {
        let left = frame(40.3544, 116.006, 17);
        let edge = left.local_to_ecef(DVec3::new(32., 0., 16.));
        let (lat, lon, _) = ellipsoid::ecef_to_geodetic(edge);
        let p = Point { lat, lon };
        let right = ChunkFrame::of(ChunkAddr {
            cell: CellId::from_dir((edge + left.x_axis * 2.).normalize(), 17),
            k: 0,
        });
        assert_ne!(left.addr.cell, right.addr.cell);
        let east = DVec3::new(-lon.sin(), lon.cos(), 0.);
        let north = DVec3::new(-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos());
        let direction = DVec2::new(left.x_axis.dot(east), left.x_axis.dot(north)).normalize();
        let normal = DVec2::new(-direction.y, direction.x);
        let segment = Segment {
            a: p.offset(-direction * 20.),
            b: p.offset(direction * 20.),
            na: normal,
            nb: normal,
            width: 6.8,
            height: 7.5,
            phase: 1.1,
            seed: 7,
            style: 0,
        };
        let elevation = |lat, lon| 120. + p.delta(Point { lat, lon }).dot(direction) * 0.2;
        let mut a = Emitter::new(&left);
        let mut b = Emitter::new(&right);
        wall(&mut a, &segment, &elevation);
        wall(&mut b, &segment, &elevation);
        let other: Vec<_> = b
            .result
            .mesh
            .vertices
            .iter()
            .map(|v| right.local_to_ecef(DVec3::from_array(v.pos.map(f64::from))))
            .collect();
        let mut boundary_count = 0;
        for v in &a.result.mesh.vertices {
            if (v.pos[0] - 32.).abs() > 0.00001 {
                continue;
            }
            let world = left.local_to_ecef(DVec3::from_array(v.pos.map(f64::from)));
            assert!(
                other.iter().any(|p| p.distance(world) < 0.025),
                "unmatched boundary vertex"
            );
            boundary_count += 1;
        }
        assert!(boundary_count > 10);
    }

    #[test]
    fn tree_and_building_exclusions_are_narrow_and_geographic() {
        let data = network();
        let s = data.segments.iter().find(|s| s.seed == 89351141).unwrap();
        let p = s.a.lerp(s.b, 0.5);
        assert!(excludes_tree(p.lat, p.lon));
        let away = p.offset(s.na.normalize() * 60.);
        assert!(!excludes_tree(away.lat, away.lon));
        let t = data.towers.iter().find(|t| t.seed == 89351131).unwrap();
        let plan = Plan::for_frame(&frame(t.p.lat.to_degrees(), t.p.lon.to_degrees(), 19));
        assert!(plan.replaces_building(t.p.lat, t.p.lon));
        let away = t.p.offset(DVec2::splat(60.));
        assert!(!plan.replaces_building(away.lat, away.lon));
    }
}
