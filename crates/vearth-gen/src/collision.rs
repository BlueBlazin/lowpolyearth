//! Continuous solid collision in a cell-local metric frame.

use glam::DVec3;
use std::sync::OnceLock;
use vearth_core::{contracts::SemanticHasher, frame::ChunkFrame, mesh::MeshBuffers};

/// Shared generation, admission and persistent-codec allocation limits.
pub const MAX_COLLISION_TRIANGLES: usize = 2_000_000;
pub const MAX_COLLISION_CAPSULES: usize = 65_536;

fn restore_coarse_ground(terrain: &mut Option<crate::TerrainCollision>) {
    if let Some(terrain) = terrain {
        // Carved banks normally replace disabled heightfield columns with
        // detailed triangles. If those triangles are omitted, re-enable the
        // sampled ground so an optional physics budget cannot create holes.
        // Only collision changes; the rendered banks and water stay exact.
        terrain.source_columns.fill(u32::MAX);
    }
}

/// Orthogonal metres relative to the cell anchor. Render-grid coordinates are
/// skewed and anisotropic, so distances and sphere radii cannot be used there.
#[derive(Clone, Copy, Debug)]
pub struct MetricFrame {
    render: ChunkFrame,
    axes: [DVec3; 3],
}
impl MetricFrame {
    pub fn of(frame: &ChunkFrame) -> Self {
        Self {
            render: *frame,
            axes: [
                frame.x_axis,
                frame.y_axis,
                frame.x_axis.cross(frame.y_axis).normalize(),
            ],
        }
    }
    pub fn point_from_ecef(&self, p: DVec3) -> [f64; 3] {
        self.direction_from_ecef(p - self.render.anchor)
    }
    pub fn direction_from_ecef(&self, d: DVec3) -> [f64; 3] {
        self.axes.map(|axis| d.dot(axis))
    }
    pub fn point_from_local(&self, p: [f32; 3]) -> [f32; 3] {
        self.point_from_ecef(
            self.render
                .local_to_ecef(DVec3::from_array(p.map(f64::from))),
        )
        .map(|v| v as f32)
    }
    pub fn point_to_ecef(&self, p: [f64; 3]) -> DVec3 {
        self.render.anchor + self.axes[0] * p[0] + self.axes[1] * p[1] + self.axes[2] * p[2]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionTriangle {
    pub vertices: [[f32; 3]; 3],
}
impl CollisionTriangle {
    /// Only finite, exactly collapsed stored geometry is degenerate. A small
    /// but representable clipping sliver remains a physical surface.
    pub(crate) fn is_degenerate(&self) -> bool {
        if self.vertices.iter().flatten().any(|p| !p.is_finite()) {
            return false;
        }
        let p = self.vertices.map(|p| DVec3::from_array(p.map(f64::from)));
        (p[1] - p[0]).cross(p[2] - p[0]).length_squared() == 0.0
    }
}

/// Transform actual emitted vertices before testing collapse. Clipping can
/// retain a render-space sliver whose points coincide in metric f32 storage.
/// Non-finite inputs remain present so strict cell/cache validation rejects them.
pub(crate) fn triangle_from_local(
    vertices: [[f32; 3]; 3],
    metric: &MetricFrame,
) -> Option<CollisionTriangle> {
    let triangle = CollisionTriangle {
        vertices: vertices.map(|p| metric.point_from_local(p)),
    };
    (!triangle.is_degenerate()).then_some(triangle)
}

/// A conservative capsule for a tapered segment. Its enclosing radius
/// is the larger endpoint radius; no raster occupancy is constructed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub radius_a: f32,
    pub radius_b: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    lo: DVec3,
    hi: DVec3,
}
impl Bounds {
    fn points(points: impl IntoIterator<Item = DVec3>) -> Self {
        points.into_iter().fold(
            Self {
                lo: DVec3::splat(f64::INFINITY),
                hi: DVec3::splat(f64::NEG_INFINITY),
            },
            |b, p| Self {
                lo: b.lo.min(p),
                hi: b.hi.max(p),
            },
        )
    }
    fn union(self, b: Self) -> Self {
        Self {
            lo: self.lo.min(b.lo),
            hi: self.hi.max(b.hi),
        }
    }
    fn overlaps(self, b: Self) -> bool {
        self.lo.cmple(b.hi).all() && b.lo.cmple(self.hi).all()
    }
    fn ray(self, o: DVec3, d: DVec3, max: f64) -> bool {
        let (mut lo, mut hi) = (0.0_f64, max);
        for axis in 0..3 {
            if d[axis].abs() < 1e-15 {
                if o[axis] < self.lo[axis] || o[axis] > self.hi[axis] {
                    return false;
                }
            } else {
                let a = (self.lo[axis] - o[axis]) / d[axis];
                let b = (self.hi[axis] - o[axis]) / d[axis];
                lo = lo.max(a.min(b));
                hi = hi.min(a.max(b));
            }
        }
        lo <= hi
    }
}
#[derive(Clone, Debug, PartialEq)]
struct Node {
    bounds: Bounds,
    start: usize,
    len: usize,
    children: Option<[usize; 2]>,
}

/// Source geometry shared by cache validation and runtime shape queries.
/// The derived index is initialized once on first contact query; generation,
/// validation, serialization and uploads do not build a physics index.
#[derive(Clone, Debug, Default)]
pub struct SurfaceCollision {
    triangles: Vec<CollisionTriangle>,
    capsules: Vec<Capsule>,
    index: OnceLock<CollisionIndex>,
}

impl PartialEq for SurfaceCollision {
    fn eq(&self, other: &Self) -> bool {
        self.triangles == other.triangles && self.capsules == other.capsules
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CollisionIndex {
    order: Vec<usize>,
    nodes: Vec<Node>,
}
impl SurfaceCollision {
    /// Bound generated BVH work before allocating its index and node arrays.
    /// Exceptional oversized cells restore the supplied coarse ground, if any.
    /// Generation supplies it at collision-enabled levels; coarser cells have
    /// no heightfield and the runtime does not query their colliders.
    pub fn from_bounded_triangles(
        triangles: Vec<CollisionTriangle>,
        terrain: &mut Option<crate::TerrainCollision>,
    ) -> Self {
        if triangles.len() > MAX_COLLISION_TRIANGLES {
            restore_coarse_ground(terrain);
            Self::default()
        } else {
            Self::from_triangles(triangles)
        }
    }

    /// Apply the same budget to externally supplied or cached source products.
    /// Visual validity is independent of this optional detailed collider.
    /// A supplied ground heightfield is restored when detail is omitted.
    pub fn limit_complexity(&mut self, terrain: &mut Option<crate::TerrainCollision>) {
        if self.triangles.len() > MAX_COLLISION_TRIANGLES
            || self.capsules.len() > MAX_COLLISION_CAPSULES
        {
            *self = Self::default();
            restore_coarse_ground(terrain);
        }
    }

    pub fn from_triangles(triangles: Vec<CollisionTriangle>) -> Self {
        Self::from_parts(triangles, Vec::new())
    }
    pub fn from_parts(triangles: Vec<CollisionTriangle>, capsules: Vec<Capsule>) -> Self {
        Self {
            triangles,
            capsules,
            index: OnceLock::new(),
        }
    }
    fn index(&self) -> &CollisionIndex {
        self.index.get_or_init(|| {
            let len = self.triangles.len() + self.capsules.len();
            let mut index = CollisionIndex {
                order: (0..len).collect(),
                nodes: Vec::new(),
            };
            if len > 0 {
                let bounds: Vec<_> = (0..len).map(|id| self.primitive_bounds(id)).collect();
                index.build_node(0, len, &bounds);
            }
            index
        })
    }
    pub fn triangles(&self) -> &[CollisionTriangle] {
        &self.triangles
    }
    pub fn capsules(&self) -> &[Capsule] {
        &self.capsules
    }
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty() && self.capsules.is_empty()
    }
    fn primitive_bounds(&self, id: usize) -> Bounds {
        if let Some(t) = self.triangles.get(id) {
            Bounds::points(t.vertices.map(|p| DVec3::from_array(p.map(f64::from))))
        } else {
            let c = self.capsules[id - self.triangles.len()];
            let r = DVec3::splat(f64::from(c.radius_a.max(c.radius_b)));
            let b = Bounds::points([c.a, c.b].map(|p| DVec3::from_array(p.map(f64::from))));
            Bounds {
                lo: b.lo - r,
                hi: b.hi + r,
            }
        }
    }
    fn visit(
        &self,
        accept: impl Fn(Bounds) -> bool,
        mut primitive: impl FnMut(usize) -> bool,
    ) -> bool {
        if self.is_empty() {
            return false;
        }
        let index = self.index();
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            let n = &index.nodes[i];
            if !accept(n.bounds) {
                continue;
            }
            if let Some([a, b]) = n.children {
                stack.push(b);
                stack.push(a);
            } else {
                for &id in &index.order[n.start..n.start + n.len] {
                    if primitive(id) {
                        return true;
                    }
                }
            }
        }
        false
    }
    /// Vertical winding for closed source envelopes; ground is queried from its
    /// own height field. A half-open projected triangle rule avoids double hits
    /// on triangulation edges and preserves overlapping volumes.
    pub fn contains(&self, x: f64, y: f64, z: f64) -> bool {
        let p = DVec3::new(x, y, z);
        if !p.is_finite() {
            return false;
        }
        let mut winding = 0_i64;
        let capsule = self.visit(
            |b| x >= b.lo.x && x <= b.hi.x && z >= b.lo.z && z <= b.hi.z && y <= b.hi.y,
            |id| {
                if let Some(t) = self.triangles.get(id) {
                    if let Some((height, sign)) = vertical_hit(t, x, z) {
                        if height >= y {
                            winding += sign;
                        }
                    }
                    false
                } else {
                    capsule_contains(self.capsules[id - self.triangles.len()], p)
                }
            },
        );
        capsule || winding > 0
    }
    pub fn top(&self, x: f64, z: f64) -> Option<f64> {
        if !x.is_finite() || !z.is_finite() {
            return None;
        }
        let mut top: Option<f64> = None;
        self.visit(
            |b| x >= b.lo.x && x <= b.hi.x && z >= b.lo.z && z <= b.hi.z,
            |id| {
                let height = if let Some(t) = self.triangles.get(id) {
                    vertical_hit(t, x, z)
                        .filter(|(_, sign)| *sign > 0)
                        .map(|(h, _)| h)
                } else {
                    let c = self.capsules[id - self.triangles.len()];
                    let b = self.primitive_bounds(id);
                    let o = DVec3::new(x, b.hi.y + 1.0, z);
                    ray_capsule(c, o, -DVec3::Y).map(|t| o.y - t)
                };
                if let Some(h) = height {
                    top = Some(top.map_or(h, |old| old.max(h)));
                }
                false
            },
        );
        top
    }
    pub fn intersects_aabb(&self, min: [f64; 3], max: [f64; 3]) -> bool {
        let bounds = Bounds {
            lo: DVec3::from_array(min),
            hi: DVec3::from_array(max),
        };
        if !bounds.lo.is_finite() || !bounds.hi.is_finite() || !bounds.lo.cmple(bounds.hi).all() {
            return false;
        }
        self.visit(
            |b| b.overlaps(bounds),
            |id| {
                if let Some(t) = self.triangles.get(id) {
                    triangle_box(t, bounds)
                } else {
                    let c = self.capsules[id - self.triangles.len()];
                    let r = DVec3::splat(f64::from(c.radius_a.max(c.radius_b)));
                    let expanded = Bounds {
                        lo: bounds.lo - r,
                        hi: bounds.hi + r,
                    };
                    let a = DVec3::from_array(c.a.map(f64::from));
                    let b = DVec3::from_array(c.b.map(f64::from));
                    expanded.ray(a, b - a, 1.0)
                }
            },
        )
    }
    /// Intersect geometry with the overlap of an AABB and inward halfspaces.
    /// Planes use this collider's metric frame: `n.dot(point) + d >= 0` is
    /// inside. Triangle clipping is exact up to 0.1 micrometre contact tolerance;
    /// capsules use a conservative expansion by their maximum endpoint radius.
    pub fn intersects_region(&self, min: [f64; 3], max: [f64; 3], planes: &[[f64; 4]]) -> bool {
        let bounds = Bounds {
            lo: DVec3::from_array(min),
            hi: DVec3::from_array(max),
        };
        if !bounds.lo.is_finite()
            || !bounds.hi.is_finite()
            || !bounds.lo.cmple(bounds.hi).all()
            || planes.iter().flatten().any(|v| !v.is_finite())
        {
            return false;
        }
        let expanded: Vec<_> = (0..3)
            .flat_map(|axis| {
                let mut positive = [0.0; 4];
                positive[axis] = 1.0;
                positive[3] = -bounds.lo[axis];
                let mut negative = [0.0; 4];
                negative[axis] = -1.0;
                negative[3] = bounds.hi[axis];
                [positive, negative]
            })
            .chain(planes.iter().copied())
            .collect();
        self.visit(
            |b| {
                b.overlaps(bounds)
                    && planes.iter().all(|plane| {
                        let n = DVec3::new(plane[0], plane[1], plane[2]);
                        let support = DVec3::new(
                            if n.x >= 0.0 { b.hi.x } else { b.lo.x },
                            if n.y >= 0.0 { b.hi.y } else { b.lo.y },
                            if n.z >= 0.0 { b.hi.z } else { b.lo.z },
                        );
                        n.dot(support) + plane[3] >= -REGION_CONTACT_M * n.length()
                    })
            },
            |id| {
                if let Some(t) = self.triangles.get(id) {
                    let mut polygon = t
                        .vertices
                        .map(|p| DVec3::from_array(p.map(f64::from)))
                        .to_vec();
                    for plane in &expanded {
                        polygon = clip_region_polygon(&polygon, *plane);
                        if polygon.is_empty() {
                            return false;
                        }
                    }
                    true
                } else {
                    let c = self.capsules[id - self.triangles.len()];
                    let a = DVec3::from_array(c.a.map(f64::from));
                    let b = DVec3::from_array(c.b.map(f64::from));
                    let radius = f64::from(c.radius_a.max(c.radius_b));
                    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
                    for plane in &expanded {
                        let n = DVec3::new(plane[0], plane[1], plane[2]);
                        let margin = (radius + REGION_CONTACT_M) * n.length();
                        let da = n.dot(a) + plane[3] + margin;
                        let db = n.dot(b) + plane[3] + margin;
                        if da < 0.0 && db < 0.0 {
                            return false;
                        }
                        if (da < 0.0) != (db < 0.0) {
                            let t = da / (da - db);
                            if da < 0.0 {
                                lo = lo.max(t);
                            } else {
                                hi = hi.min(t);
                            }
                            if lo > hi {
                                return false;
                            }
                        }
                    }
                    true
                }
            },
        )
    }
    pub fn raycast(&self, origin: [f64; 3], direction: [f64; 3], max_t: f64) -> Option<f64> {
        let (o, d) = (DVec3::from_array(origin), DVec3::from_array(direction));
        if !o.is_finite()
            || !d.is_finite()
            || d.length_squared() <= 1e-24
            || max_t < 0.0
            || max_t.is_nan()
        {
            return None;
        }
        let mut nearest = max_t;
        let mut hit = false;
        self.visit(
            |b| b.ray(o, d, max_t),
            |id| {
                let t = if let Some(t) = self.triangles.get(id) {
                    ray_triangle(t, o, d)
                } else {
                    ray_capsule(self.capsules[id - self.triangles.len()], o, d)
                };
                if let Some(t) = t.filter(|t| *t <= nearest) {
                    nearest = t;
                    hit = true;
                }
                false
            },
        );
        hit.then_some(nearest)
    }
    pub fn heap_bytes(&self) -> usize {
        std::mem::size_of_val(self.triangles.as_slice())
            + std::mem::size_of_val(self.capsules.as_slice())
            + self.index.get().map_or(0, |index| {
                std::mem::size_of_val(index.order.as_slice())
                    + std::mem::size_of_val(index.nodes.as_slice())
            })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.triangles.len() > MAX_COLLISION_TRIANGLES
            || self.capsules.len() > MAX_COLLISION_CAPSULES
        {
            return Err("surface collision primitive count exceeds limit".into());
        }
        for t in &self.triangles {
            if t.vertices.iter().flatten().any(|p| !p.is_finite()) {
                return Err("surface collision contains non-finite triangle".into());
            }
            if t.is_degenerate() {
                return Err("surface collision contains degenerate triangle".into());
            }
        }
        for c in &self.capsules {
            if c.a.iter().chain(&c.b).any(|p| !p.is_finite())
                || !c.radius_a.is_finite()
                || !c.radius_b.is_finite()
                || c.radius_a < 0.0
                || c.radius_b < 0.0
                || c.radius_a.max(c.radius_b) <= 0.0
            {
                return Err("surface collision contains invalid capsule".into());
            }
        }
        Ok(())
    }
}

impl CollisionIndex {
    fn build_node(&mut self, start: usize, len: usize, primitive_bounds: &[Bounds]) -> usize {
        let bounds = self.order[start..start + len]
            .iter()
            .map(|&i| primitive_bounds[i])
            .reduce(Bounds::union)
            .unwrap();
        let index = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            start,
            len,
            children: None,
        });
        if len > 8 {
            let size = bounds.hi - bounds.lo;
            let axis = if size.x >= size.y && size.x >= size.z {
                0
            } else if size.y >= size.z {
                1
            } else {
                2
            };
            let compare = |&a: &usize, &b: &usize| {
                let a_bounds = primitive_bounds[a];
                let b_bounds = primitive_bounds[b];
                (a_bounds.lo[axis] + a_bounds.hi[axis])
                    .total_cmp(&(b_bounds.lo[axis] + b_bounds.hi[axis]))
                    .then(a.cmp(&b))
            };
            // Median selection preserves the exact partition without sorting
            // every internal range. Only terminal ranges need their parent's
            // total order to retain the same primitive traversal, including ties.
            let middle = len / 2;
            let (left_order, _, _) =
                self.order[start..start + len].select_nth_unstable_by(middle, compare);
            if middle <= 8 {
                left_order.sort_unstable_by(compare);
            }
            if len - middle <= 8 {
                // Include the median in the right child, as in the full sort.
                self.order[start + middle..start + len].sort_unstable_by(compare);
            }
            let left = self.build_node(start, len / 2, primitive_bounds);
            let right = self.build_node(start + len / 2, len - len / 2, primitive_bounds);
            self.nodes[index].children = Some([left, right]);
        }
        index
    }
}

impl crate::TerrainCollision {
    fn visit_metric_triangles(
        &self,
        frame: &ChunkFrame,
        lo: glam::DVec2,
        hi: glam::DVec2,
        mut visit: impl FnMut(CollisionTriangle) -> bool,
    ) -> bool {
        const N: usize = crate::N;
        const EDGE: usize = N + 1;
        // Metric f32 vertices can move a few ulps across a grid edge. The
        // broadphase includes that fringe; exact triangle tests decide hits.
        const PAD: f64 = 1e-4;
        if !lo.is_finite()
            || !hi.is_finite()
            || !lo.cmple(hi).all()
            || hi.x < -PAD
            || hi.y < -PAD
            || lo.x > N as f64 + PAD
            || lo.y > N as f64 + PAD
        {
            return false;
        }
        let lower = |v: f64| (v - PAD).floor().clamp(0.0, (N - 1) as f64) as usize;
        let upper = |v: f64| (v + PAD).floor().clamp(0.0, (N - 1) as f64) as usize;
        let metric = MetricFrame::of(frame);
        for z in lower(lo.y)..=upper(hi.y) {
            for x in lower(lo.x)..=upper(hi.x) {
                if self.source_columns[z] & (1 << x) == 0 {
                    continue;
                }
                let points = [
                    [x as f32, self.heights_local[z * EDGE + x], z as f32],
                    [
                        (x + 1) as f32,
                        self.heights_local[z * EDGE + x + 1],
                        z as f32,
                    ],
                    [
                        (x + 1) as f32,
                        self.heights_local[(z + 1) * EDGE + x + 1],
                        (z + 1) as f32,
                    ],
                    [
                        x as f32,
                        self.heights_local[(z + 1) * EDGE + x],
                        (z + 1) as f32,
                    ],
                ];
                for ids in [[0, 3, 2], [0, 2, 1]] {
                    if let Some(triangle) = triangle_from_local(ids.map(|i| points[i]), &metric)
                        && visit(triangle)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Raycast the shared terrain lattice without a stored triangle BVH.
    /// Coordinates and distances use the same metric frame as `SurfaceCollision`.
    pub fn raycast(
        &self,
        frame: &ChunkFrame,
        origin: [f64; 3],
        direction: [f64; 3],
        max_t: f64,
    ) -> Option<f64> {
        let origin_metric = DVec3::from_array(origin);
        let direction_metric = DVec3::from_array(direction);
        if !origin_metric.is_finite()
            || !direction_metric.is_finite()
            || direction_metric.length_squared() <= 1e-24
            || max_t < 0.0
            || max_t.is_nan()
        {
            return None;
        }
        let metric = MetricFrame::of(frame);
        let local = frame.ecef_to_local(metric.point_to_ecef(origin));
        let next = frame
            .ecef_to_local(metric.point_to_ecef((origin_metric + direction_metric).to_array()));
        let delta = next - local;
        let (mut low, mut high) = (0.0_f64, max_t);
        for axis in [0, 2] {
            if delta[axis].abs() < 1e-15 {
                if local[axis] < -1e-4 || local[axis] > crate::N as f64 + 1e-4 {
                    return None;
                }
            } else {
                let a = (-1e-4 - local[axis]) / delta[axis];
                let b = (crate::N as f64 + 1e-4 - local[axis]) / delta[axis];
                low = low.max(a.min(b));
                high = high.min(a.max(b));
            }
        }
        if low > high {
            return None;
        }
        let at = |t: f64| {
            glam::DVec2::new(
                if delta.x.abs() < 1e-15 {
                    local.x
                } else {
                    local.x + delta.x * t
                },
                if delta.z.abs() < 1e-15 {
                    local.z
                } else {
                    local.z + delta.z * t
                },
            )
        };
        let (a, b) = (at(low), at(high));
        let mut nearest = max_t;
        let mut hit = false;
        self.visit_metric_triangles(frame, a.min(b), a.max(b), |triangle| {
            if let Some(t) =
                ray_triangle(&triangle, origin_metric, direction_metric).filter(|t| *t <= nearest)
            {
                nearest = t;
                hit = true;
            }
            false
        });
        hit.then_some(nearest)
    }

    /// Test terrain against a metric AABB intersected with inward halfspaces.
    /// Masked shore columns are left to their separate water/bank colliders.
    pub fn intersects_region(
        &self,
        frame: &ChunkFrame,
        min: [f64; 3],
        max: [f64; 3],
        planes: &[[f64; 4]],
    ) -> bool {
        let bounds = Bounds {
            lo: DVec3::from_array(min),
            hi: DVec3::from_array(max),
        };
        if !bounds.lo.is_finite()
            || !bounds.hi.is_finite()
            || !bounds.lo.cmple(bounds.hi).all()
            || planes.iter().flatten().any(|v| !v.is_finite())
        {
            return false;
        }
        let metric = MetricFrame::of(frame);
        let mut lo = glam::DVec2::splat(f64::INFINITY);
        let mut hi = glam::DVec2::splat(f64::NEG_INFINITY);
        for i in 0..8 {
            let p = std::array::from_fn(|axis| {
                if i & (1 << axis) == 0 {
                    min[axis]
                } else {
                    max[axis]
                }
            });
            let local = frame.ecef_to_local(metric.point_to_ecef(p));
            let p = glam::DVec2::new(local.x, local.z);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        self.visit_metric_triangles(frame, lo, hi, |triangle| {
            let b = Bounds::points(
                triangle
                    .vertices
                    .map(|p| DVec3::from_array(p.map(f64::from))),
            );
            b.overlaps(bounds) && triangle_intersects_region(&triangle, bounds, planes)
        })
    }
}

fn box_planes(bounds: Bounds) -> [[f64; 4]; 6] {
    std::array::from_fn(|i| {
        let axis = i / 2;
        let mut plane = [0.0; 4];
        if i % 2 == 0 {
            plane[axis] = 1.0;
            plane[3] = -bounds.lo[axis];
        } else {
            plane[axis] = -1.0;
            plane[3] = bounds.hi[axis];
        }
        plane
    })
}

fn triangle_intersects_region(
    triangle: &CollisionTriangle,
    bounds: Bounds,
    planes: &[[f64; 4]],
) -> bool {
    let box_planes = box_planes(bounds);
    let clipping_planes = || box_planes.iter().chain(planes);
    // A clipped triangle adds at most one vertex per plane. Normal cell
    // queries use ten planes and stay on the stack; unusual caller-defined
    // regions may use the unbounded reference path instead.
    if planes.len().saturating_add(9) > 64 {
        let mut polygon = triangle
            .vertices
            .map(|p| DVec3::from_array(p.map(f64::from)))
            .to_vec();
        for &plane in clipping_planes() {
            polygon = clip_region_polygon(&polygon, plane);
            if polygon.is_empty() {
                return false;
            }
        }
        return true;
    }
    let mut current = [DVec3::ZERO; 64];
    let mut scratch = [DVec3::ZERO; 64];
    current[..3].copy_from_slice(
        &triangle
            .vertices
            .map(|p| DVec3::from_array(p.map(f64::from))),
    );
    let mut len = 3;
    for &plane in clipping_planes() {
        let n = DVec3::new(plane[0], plane[1], plane[2]);
        let margin = REGION_CONTACT_M * n.length();
        let distance = |p: DVec3| n.dot(p) + plane[3] + margin;
        let mut output = 0;
        for i in 0..len {
            let a = current[i];
            let b = current[(i + 1) % len];
            let da = distance(a);
            let db = distance(b);
            if da >= 0.0 {
                scratch[output] = a;
                output += 1;
            }
            if (da >= 0.0) != (db >= 0.0) {
                scratch[output] = a.lerp(b, (da / (da - db)).clamp(0.0, 1.0));
                output += 1;
            }
        }
        if output == 0 {
            return false;
        }
        std::mem::swap(&mut current, &mut scratch);
        len = output;
    }
    true
}

pub fn triangles_from_mesh(mesh: &MeshBuffers, frame: &ChunkFrame) -> Vec<CollisionTriangle> {
    let metric = MetricFrame::of(frame);
    mesh.indices
        .chunks_exact(3)
        .filter_map(|indices| {
            triangle_from_local(
                [indices[0], indices[1], indices[2]].map(|i| mesh.vertices[i as usize].pos),
                &metric,
            )
        })
        .collect()
}

pub fn hash_collision(h: &mut SemanticHasher, collision: &SurfaceCollision) {
    h.u64(collision.triangles.len() as u64);
    for t in &collision.triangles {
        for p in t.vertices {
            for v in p {
                h.f32(v);
            }
        }
    }
    h.u64(collision.capsules.len() as u64);
    for c in &collision.capsules {
        for v in c.a.into_iter().chain(c.b) {
            h.f32(v);
        }
        h.f32(c.radius_a);
        h.f32(c.radius_b);
    }
}

const REGION_CONTACT_M: f64 = 1e-7;
fn clip_region_polygon(polygon: &[DVec3], plane: [f64; 4]) -> Vec<DVec3> {
    if polygon.is_empty() {
        return Vec::new();
    }
    let n = DVec3::new(plane[0], plane[1], plane[2]);
    let margin = REGION_CONTACT_M * n.length();
    let distance = |p: DVec3| n.dot(p) + plane[3] + margin;
    let mut output = Vec::with_capacity(polygon.len() + 1);
    for (&a, &b) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        let da = distance(a);
        let db = distance(b);
        if da >= 0.0 {
            output.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            output.push(a.lerp(b, (da / (da - db)).clamp(0.0, 1.0)));
        }
    }
    output
}

fn vertical_hit(t: &CollisionTriangle, x: f64, z: f64) -> Option<(f64, i64)> {
    let mut p = t.vertices.map(|p| DVec3::from_array(p.map(f64::from)));
    let normal = (p[1] - p[0]).cross(p[2] - p[0]);
    if normal.y.abs() < 1e-14 {
        return None;
    }
    let sign = if normal.y > 0.0 { 1 } else { -1 };
    if normal.y > 0.0 {
        p.swap(1, 2);
    }
    for i in 0..3 {
        let a = p[i];
        let b = p[(i + 1) % 3];
        let (dx, dz) = (b.x - a.x, b.z - a.z);
        let edge = dx * (z - a.z) - dz * (x - a.x);
        if edge < 0.0 || (edge == 0.0 && !(dz > 0.0 || (dz == 0.0 && dx < 0.0))) {
            return None;
        }
    }
    let height = t.vertices[0][1] as f64
        - (normal.x * (x - t.vertices[0][0] as f64) + normal.z * (z - t.vertices[0][2] as f64))
            / normal.y;
    Some((height, sign))
}
fn capsule_contains(c: Capsule, p: DVec3) -> bool {
    let a = DVec3::from_array(c.a.map(f64::from));
    let b = DVec3::from_array(c.b.map(f64::from));
    let d = b - a;
    let t = if d.length_squared() > 1e-24 {
        ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance_squared(a + d * t) <= f64::from(c.radius_a.max(c.radius_b)).powi(2)
}
fn ray_triangle(t: &CollisionTriangle, o: DVec3, d: DVec3) -> Option<f64> {
    let p = t.vertices.map(|p| DVec3::from_array(p.map(f64::from)));
    let e1 = p[1] - p[0];
    let e2 = p[2] - p[0];
    let h = d.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-14 {
        return None;
    }
    let s = o - p[0];
    let u = s.dot(h) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let hit = e2.dot(q) / det;
    (hit >= 0.0).then_some(hit)
}
fn ray_capsule(c: Capsule, o: DVec3, direction: DVec3) -> Option<f64> {
    let length = direction.length();
    if length <= 1e-15 {
        return None;
    }
    let d = direction / length;
    let a = DVec3::from_array(c.a.map(f64::from));
    let b = DVec3::from_array(c.b.map(f64::from));
    let ba = b - a;
    let oa = o - a;
    let r = f64::from(c.radius_a.max(c.radius_b));
    if capsule_contains(c, o) {
        return Some(0.0);
    }
    let baba = ba.length_squared();
    let bard = ba.dot(d);
    let baoa = ba.dot(oa);
    let aa = baba - bard * bard;
    let bb = baba * oa.dot(d) - baoa * bard;
    let cc = baba * oa.length_squared() - baoa * baoa - r * r * baba;
    let mut result: Option<f64> = None;
    let disc = bb * bb - aa * cc;
    if aa.abs() > 1e-15 && disc >= 0.0 {
        let t = (-bb - disc.sqrt()) / aa;
        let y = baoa + t * bard;
        if t >= 0.0 && y >= 0.0 && y <= baba {
            result = Some(t);
        }
    }
    for centre in [a, b] {
        let q = o - centre;
        let b = q.dot(d);
        let disc = b * b - q.length_squared() + r * r;
        if disc >= 0.0 {
            let t = -b - disc.sqrt();
            if t >= 0.0 {
                result = Some(result.map_or(t, |v| v.min(t)));
            }
        }
    }
    result.map(|t| t / length)
}
fn triangle_box(t: &CollisionTriangle, b: Bounds) -> bool {
    let centre = (b.lo + b.hi) * 0.5;
    let half = (b.hi - b.lo) * 0.5;
    let p = t
        .vertices
        .map(|p| DVec3::from_array(p.map(f64::from)) - centre);
    let edges = [p[1] - p[0], p[2] - p[1], p[0] - p[2]];
    let separated = |axis: DVec3| {
        let proj = p.map(|p| p.dot(axis));
        let lo = proj[0].min(proj[1]).min(proj[2]);
        let hi = proj[0].max(proj[1]).max(proj[2]);
        let r = half.dot(axis.abs());
        lo > r || hi < -r
    };
    for axis in [DVec3::X, DVec3::Y, DVec3::Z, edges[0].cross(edges[1])] {
        if separated(axis) {
            return false;
        }
    }
    for e in edges {
        for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
            if separated(e.cross(axis)) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_validation_and_identity_do_not_build_a_collision_index() {
        let collision = SurfaceCollision::from_triangles(slab(1.0, 3.0, false));
        let source = collision.clone();
        collision.validate().unwrap();
        assert!(!collision.is_empty());
        assert_eq!(
            collision.heap_bytes(),
            std::mem::size_of_val(collision.triangles())
        );
        assert!(collision.capsules().is_empty());
        assert!(collision.index.get().is_none());
        vearth_platform::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    assert_eq!(collision.top(0.0, 0.0), Some(3.0));
                    assert!(collision.contains(0.0, 2.0, 0.0));
                    assert_eq!(
                        collision.raycast([0.0, 10.0, 0.0], [0.0, -1.0, 0.0], 20.0),
                        Some(7.0)
                    );
                });
            }
        });
        assert!(collision.index.get().is_some());
        assert!(source.index.get().is_none());
        assert_eq!(
            collision, source,
            "query acceleration is not source identity"
        );
    }

    #[test]
    fn oversized_generated_collision_skips_bvh_and_keeps_coarse_ground() {
        let mut terrain = Some(crate::TerrainCollision {
            heights_local: Box::new([4.0; 33 * 33]),
            source_columns: [0; 32],
        });
        let triangle = CollisionTriangle {
            vertices: [[0.0, 4.0, 0.0], [0.0, 4.0, 1.0], [1.0, 4.0, 0.0]],
        };
        let collision = SurfaceCollision::from_bounded_triangles(
            vec![triangle; MAX_COLLISION_TRIANGLES + 1],
            &mut terrain,
        );
        assert!(collision.is_empty());
        assert_eq!(collision.heap_bytes(), 0);
        collision.validate().unwrap();
        let terrain = terrain.unwrap();
        for z in 0..=32 {
            for x in 0..=32 {
                assert_eq!(terrain.sample_local(x as f64, z as f64), Some(4.0));
            }
        }
    }
    fn slab(lo: f32, hi: f32, diagonal: bool) -> Vec<CollisionTriangle> {
        let plane = |y: f32, up: bool| {
            let p = [[-2., y, -2.], [2., y, -2.], [2., y, 2.], [-2., y, 2.]];
            let ids = if diagonal {
                [[0, 3, 2], [0, 2, 1]]
            } else {
                [[0, 3, 1], [1, 3, 2]]
            };
            ids.map(|mut ids| {
                if !up {
                    ids.swap(1, 2);
                }
                CollisionTriangle {
                    vertices: ids.map(|i| p[i]),
                }
            })
        };
        plane(hi, true)
            .into_iter()
            .chain(plane(lo, false))
            .collect()
    }
    #[test]
    fn closed_overlapping_envelopes_and_shared_edges_keep_open_air() {
        let mut triangles = slab(2., 4., true);
        triangles.extend(slab(3., 5., false));
        let c = SurfaceCollision::from_triangles(triangles);
        for (x, z) in [(0., 0.), (1., 1.), (-1., 1.), (0.2, -0.7)] {
            assert!(!c.contains(x, 1., z));
            assert!(c.contains(x, 2.5, z));
            assert!(c.contains(x, 4.5, z));
            assert!(!c.contains(x, 5.1, z));
            assert_eq!(c.top(x, z), Some(5.));
        }
    }
    #[test]
    fn thin_slabs_use_shape_and_ray_queries_without_sampling_gaps() {
        let c = SurfaceCollision::from_triangles(sab());
        assert_eq!(c.raycast([0., 10., 0.], [0., -1., 0.], 20.), Some(5.));
        assert!(c.intersects_aabb([-0.2, 4.9, -0.2], [0.2, 5.1, 0.2]));
        assert!(!c.intersects_aabb([3., 4.9, 3.], [4., 5.1, 4.]));
        fn sab() -> Vec<CollisionTriangle> {
            slab(4.99, 5., true)
        }
    }
    #[test]
    fn metric_frame_preserves_distances_off_cube_face_axes() {
        let cell = vearth_core::cell::CellId::from_dir(DVec3::new(1., 0.6, 0.4), 18);
        let frame = ChunkFrame::of(vearth_core::cell::ChunkAddr { cell, k: 0 });
        let metric = MetricFrame::of(&frame);
        let a = frame.anchor + DVec3::new(3., 7., 11.);
        let b = frame.anchor + DVec3::new(-2., 9., 4.);
        let ma = DVec3::from_array(metric.point_from_ecef(a));
        let mb = DVec3::from_array(metric.point_from_ecef(b));
        assert!((ma.distance(mb) - a.distance(b)).abs() < 1e-10);
        assert!(metric.point_to_ecef(ma.to_array()).distance(a) < 1e-8);
    }
    #[test]
    fn capsules_retain_radial_gaps_and_ray_distance() {
        let c = SurfaceCollision::from_parts(
            Vec::new(),
            vec![Capsule {
                a: [0., 0., 0.],
                b: [0., 10., 0.],
                radius_a: 0.2,
                radius_b: 0.1,
            }],
        );
        assert!(c.contains(0., 5., 0.));
        assert!(!c.contains(0.5, 5., 0.));
        assert!((c.raycast([2., 5., 0.], [-1., 0., 0.], 4.).unwrap() - 1.8).abs() < 1e-6);
    }
    #[test]
    fn coverage_planes_reject_hidden_parent_geometry_inside_query_aabb() {
        let collision = SurfaceCollision::from_triangles(vec![CollisionTriangle {
            vertices: [[-3.0, 0.0, -1.0], [-1.0, 0.0, -1.0], [-2.0, 0.0, 1.0]],
        }]);
        let min = [-4.0, -1.0, -2.0];
        let max = [4.0, 1.0, 2.0];
        assert!(collision.intersects_aabb(min, max));
        assert!(!collision.intersects_region(min, max, &[[1.0, 0.0, 0.0, 0.0]]));
        assert!(collision.intersects_region(min, max, &[[-1.0, 0.0, 0.0, 0.0]]));
        assert_eq!(
            collision.intersects_region(min, max, &[]),
            collision.intersects_aabb(min, max)
        );
    }
    #[test]
    fn region_clipping_requires_common_triangle_box_and_cone_overlap() {
        let collision = SurfaceCollision::from_triangles(vec![CollisionTriangle {
            vertices: [[-3.0, 0.0, -3.0], [3.0, 0.0, -3.0], [-3.0, 0.0, 3.0]],
        }]);
        let min = [-4.0, -1.0, -4.0];
        let max = [4.0, 1.0, 4.0];
        // Each plane separately intersects; their intersection does not.
        assert!(!collision.intersects_region(
            min,
            max,
            &[[1.0, 0.0, 0.0, -1.0], [0.0, 0.0, 1.0, -1.0]]
        ));
        assert!(collision.intersects_region(
            min,
            max,
            &[[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]]
        ));
        assert!(!collision.intersects_region([1.0, -1.0, 1.0], [2.0, 1.0, 2.0], &[]));
        assert!(collision.intersects_region([0.0, -1.0, 0.0], [0.1, 1.0, 0.1], &[]));
    }
    #[test]
    fn capsule_region_rejection_preserves_radius_and_halfspace_scaling() {
        let collision = SurfaceCollision::from_parts(
            Vec::new(),
            vec![Capsule {
                a: [-2.0, 0.0, 0.0],
                b: [-2.0, 3.0, 0.0],
                radius_a: 0.5,
                radius_b: 0.2,
            }],
        );
        let min = [-4.0, -1.0, -1.0];
        let max = [4.0, 4.0, 1.0];
        assert!(!collision.intersects_region(min, max, &[[1.0, 0.0, 0.0, 1.4]]));
        assert!(collision.intersects_region(min, max, &[[1.0, 0.0, 0.0, 1.6]]));
        assert!(collision.intersects_region(min, max, &[[10.0, 0.0, 0.0, 16.0]]));
        assert!(!collision.intersects_region(min, max, &[[0.0, 0.0, 0.0, -1.0]]));
        assert!(!collision.intersects_region(min, max, &[[f64::NAN, 0.0, 0.0, 0.0]]));
    }
    #[test]
    fn clipped_render_slivers_are_checked_after_metric_f32_storage() {
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            ellipsoid::geodetic_to_ecef,
            mesh::SurfaceVertex,
        };
        let frame = ChunkFrame::of(ChunkAddr {
            cell: CellId::from_dir(
                geodetic_to_ecef(18.5207_f64.to_radians(), 73.8524_f64.to_radians(), 0.0),
                15,
            ),
            k: 0,
        });
        let metric = MetricFrame::of(&frame);
        let vertices = [[0.0, 100.0, 1.0], [1e-8, 100.0, 1.0], [0.0, 100.0, 2.0]];
        let source = vertices.map(|p| DVec3::from_array(p.map(f64::from)));
        assert!(
            (source[1] - source[0])
                .cross(source[2] - source[0])
                .length_squared()
                > 0.0
        );
        let stored = vertices.map(|p| metric.point_from_local(p));
        assert_eq!(
            stored[0], stored[1],
            "the fixture must exercise an actual metric f32 collapse"
        );
        assert!(triangle_from_local(vertices, &metric).is_none());
        let valid = [[0.0, 100.0, 1.0], [0.001, 100.0, 1.0], [0.0, 100.0, 2.0]];
        let mut mesh = MeshBuffers::default();
        for triangle in [vertices, valid] {
            let index = mesh.vertices.len() as u32;
            mesh.vertices.extend(triangle.map(|pos| SurfaceVertex {
                pos,
                face_ao: 0,
                rgba: [255; 4],
            }));
            mesh.indices.extend([index, index + 1, index + 2]);
        }
        let triangles = triangles_from_mesh(&mesh, &frame);
        assert_eq!(triangles.len(), 1);
        assert_eq!(triangles[0], triangle_from_local(valid, &metric).unwrap());
        SurfaceCollision::from_triangles(triangles)
            .validate()
            .unwrap();
    }

    #[test]
    fn representable_narrow_blockers_survive_without_relaxing_corrupt_cache_validation() {
        let narrow = CollisionTriangle {
            vertices: [[0.0, 2.0, 0.0], [0.0, 2.0, 1.0], [4e-13, 2.0, 0.0]],
        };
        assert!(!narrow.is_degenerate());
        let collision = SurfaceCollision::from_triangles(vec![narrow]);
        collision.validate().unwrap();
        assert_eq!(
            collision.raycast([1e-13, 3.0, 0.25], [0.0, -1.0, 0.0], 2.0),
            Some(1.0)
        );
        let collapsed = CollisionTriangle {
            vertices: [[0.0; 3]; 3],
        };
        assert!(collapsed.is_degenerate());
        let strict = SurfaceCollision::from_triangles(vec![collapsed]);
        assert_eq!(
            strict.triangles().len(),
            1,
            "decode/construction must never silently repair corrupt cached geometry"
        );
        assert!(strict.validate().unwrap_err().contains("degenerate"));
        let nonfinite = CollisionTriangle {
            vertices: [[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        };
        assert!(!nonfinite.is_degenerate());
        assert!(
            SurfaceCollision::from_triangles(vec![nonfinite])
                .validate()
                .unwrap_err()
                .contains("non-finite")
        );
    }
    fn reference_build_node(
        collision: &SurfaceCollision,
        index_data: &mut CollisionIndex,
        start: usize,
        len: usize,
    ) -> usize {
        let bounds = index_data.order[start..start + len]
            .iter()
            .map(|&i| collision.primitive_bounds(i))
            .reduce(Bounds::union)
            .unwrap();
        let index = index_data.nodes.len();
        index_data.nodes.push(Node {
            bounds,
            start,
            len,
            children: None,
        });
        if len > 8 {
            let size = bounds.hi - bounds.lo;
            let axis = if size.x >= size.y && size.x >= size.z {
                0
            } else if size.y >= size.z {
                1
            } else {
                2
            };
            // Sorting a local vector avoids aliasing geometry and order; the
            // primitive index resolves coincident-centre ties deterministically.
            let mut order = index_data.order[start..start + len].to_vec();
            order.sort_unstable_by(|&a, &b| {
                let a_bounds = collision.primitive_bounds(a);
                let b_bounds = collision.primitive_bounds(b);
                (a_bounds.lo[axis] + a_bounds.hi[axis])
                    .total_cmp(&(b_bounds.lo[axis] + b_bounds.hi[axis]))
                    .then(a.cmp(&b))
            });
            index_data.order[start..start + len].copy_from_slice(&order);
            let left = reference_build_node(collision, index_data, start, len / 2);
            let right = reference_build_node(collision, index_data, start + len / 2, len - len / 2);
            index_data.nodes[index].children = Some([left, right]);
        }
        index
    }

    #[test]
    fn cached_bvh_bounds_preserve_the_entire_index() {
        let triangles: Vec<_> = (0..8192)
            .map(|i| {
                let x = ((i * 719) % 97) as f32 - 40.0;
                let y = ((i * 311) % 37) as f32 - 15.0;
                let z = ((i * 227) % 113) as f32 - 55.0;
                CollisionTriangle {
                    vertices: [[x, y, z], [x + 0.7, y, z], [x, y + 1.3, z + 0.4]],
                }
            })
            .collect();
        let capsules: Vec<_> = (0..257)
            .map(|i| Capsule {
                a: [(i % 19) as f32, -2.0, (i % 23) as f32],
                b: [(i % 19) as f32 + 1.0, 4.0, (i % 23) as f32],
                radius_a: 0.2,
                radius_b: 0.7,
            })
            .collect();
        let reference = SurfaceCollision::from_parts(triangles.clone(), capsules.clone());
        let mut reference_index = CollisionIndex {
            order: (0..triangles.len() + capsules.len()).collect(),
            nodes: Vec::new(),
        };
        let start = vearth_platform::time::Instant::now();
        reference_build_node(
            &reference,
            &mut reference_index,
            0,
            triangles.len() + capsules.len(),
        );
        let reference_time = start.elapsed();
        let start = vearth_platform::time::Instant::now();
        let actual = SurfaceCollision::from_parts(triangles, capsules);
        assert!(actual.index.get().is_none());
        assert_eq!(actual.index(), &reference_index);
        let actual_time = start.elapsed();
        assert_eq!(actual, reference);
        eprintln!("BVH reference {reference_time:?}, lazy median index {actual_time:?}");
    }
    #[test]
    fn heightfield_queries_match_unstored_render_triangles_with_masked_columns() {
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            cubesphere::Face,
        };
        for level in [11, 16, 19] {
            for face in 0..6 {
                let frame = ChunkFrame::of(ChunkAddr {
                    cell: CellId {
                        face: Face(face),
                        level,
                        i: (1 << level) * 3 / 8,
                        j: (1 << level) * 5 / 8,
                    },
                    k: 0,
                });
                let mut source = crate::TerrainSurfaceData {
                    heights_m: Box::new(std::array::from_fn(|i| {
                        100.0 + ((i % 33) as f32 * 0.71).sin() * 7.0 + (i / 33) as f32 * 0.2
                    })),
                    shading: None,
                    source_columns: [u32::MAX; crate::N],
                };
                source.source_columns[7] &= !(1 << 8);
                let geometry = crate::terrain_surface::build(
                    &frame,
                    &source,
                    &[[crate::SurfaceAppearance {
                        material: crate::material::GRASS,
                        tint: [80, 110, 70],
                    }; crate::N]; crate::N],
                );
                let terrain = geometry.collision.unwrap();
                assert!(
                    geometry.triangles.is_empty(),
                    "unmodified columns must not duplicate their heightfield in the BVH"
                );
                let metric = MetricFrame::of(&frame);
                let reference = SurfaceCollision::from_triangles(
                    geometry
                        .mesh
                        .indices
                        .chunks_exact(3)
                        .filter_map(|ids| {
                            let points =
                                [0, 1, 2].map(|i| geometry.mesh.vertices[ids[i] as usize].pos);
                            let p = points.map(|p| DVec3::from_array(p.map(f64::from)));
                            ((p[1] - p[0]).cross(p[2] - p[0]).y > 0.0)
                                .then(|| triangle_from_local(points, &metric))
                                .flatten()
                        })
                        .collect(),
                );
                for i in 0..32 {
                    let x = ((i * 17) % 31) as f32 + 0.37;
                    let z = ((i * 23) % 31) as f32 + 0.63;
                    let height = terrain
                        .sample_local(f64::from(x), f64::from(z))
                        .unwrap_or(100.0 / frame.grid_scale.y);
                    let origin = metric
                        .point_from_local([x, (height + 20.0 / frame.grid_scale.y) as f32, z])
                        .map(f64::from);
                    for direction in [[0.0, -1.0, 0.0], [0.3, -1.0, 0.2], [-0.2, -1.0, -0.3]] {
                        assert_eq!(
                            terrain.raycast(&frame, origin, direction, 150.0),
                            reference.raycast(origin, direction, 150.0),
                            "L{level} face{face} ray{i}"
                        );
                    }
                    let center = metric
                        .point_from_local([x, height as f32, z])
                        .map(f64::from);
                    for lift in [-5.0, 0.0, 5.0] {
                        let mut center = DVec3::from_array(center);
                        center.y += lift;
                        let min = (center - DVec3::splat(1.1)).to_array();
                        let max = (center + DVec3::splat(1.1)).to_array();
                        let planes = [
                            [1.0, 0.0, 0.5, -center.x - center.z * 0.5],
                            [0.0, 1.0, 0.0, -center.y - 0.1],
                        ];
                        for planes in [&[][..], &planes[..]] {
                            assert_eq!(
                                terrain.intersects_region(&frame, min, max, planes),
                                reference.intersects_region(min, max, planes),
                                "L{level} face{face} region{i} lift{lift}"
                            );
                        }
                    }
                }
                let x = 8.5;
                let z = 7.5;
                let local_y =
                    f64::from(terrain.heights_local[7 * 33 + 8]) + 20.0 / frame.grid_scale.y;
                let origin = metric
                    .point_from_local([x, local_y as f32, z])
                    .map(f64::from);
                assert_eq!(
                    terrain.raycast(&frame, origin, [0.0, -1.0, 0.0], f64::INFINITY),
                    None,
                    "masked hole gained a floor"
                );
            }
        }
    }

    #[test]
    fn stack_region_clipping_matches_the_unbounded_reference() {
        let triangle = CollisionTriangle {
            vertices: [[-2.0, 0.0, -2.0], [2.0, 1.0, -2.0], [0.0, 3.0, 3.0]],
        };
        for count in [0, 4, 12, 64] {
            let planes: Vec<_> = (0..count)
                .map(|i| {
                    let angle = i as f64 / count.max(1) as f64 * std::f64::consts::TAU;
                    [angle.cos(), 0.0, angle.sin(), 1.7]
                })
                .collect();
            for offset in [-3.0, -1.0, 0.0, 1.0, 3.0] {
                let bounds = Bounds {
                    lo: DVec3::new(offset - 0.5, 0.4, -0.5),
                    hi: DVec3::new(offset + 0.5, 2.0, 0.5),
                };
                let mut expected = triangle
                    .vertices
                    .map(|p| DVec3::from_array(p.map(f64::from)))
                    .to_vec();
                for plane in box_planes(bounds).into_iter().chain(planes.iter().copied()) {
                    expected = clip_region_polygon(&expected, plane);
                }
                assert_eq!(
                    triangle_intersects_region(&triangle, bounds, &planes),
                    !expected.is_empty()
                );
            }
        }
    }
}
