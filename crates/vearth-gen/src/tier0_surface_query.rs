use super::{FeaturePack, GroundKind, SurfaceLayer, point_in_rings, source_surface_with};
use vearth_data::vectors::Rings;

/// One immutable query index serves every terrain corner in the source pack.
/// Layer and feature ordering remain in `source_surface_with`. A bounds index
/// rejects unrelated polygons before their unchanged horizontal-ray tests.
pub(crate) struct SourceSurfaceQuery<'a> {
    pack: &'a FeaturePack,
    layers: [Vec<PolygonQuery<'a>>; 4],
    bounds: [PolygonBoundsIndex; 4],
}

impl<'a> SourceSurfaceQuery<'a> {
    pub(crate) fn new(pack: &'a FeaturePack) -> Self {
        let layers: [Vec<PolygonQuery<'a>>; 4] = [
            pack.regional_landcover
                .iter()
                .map(|region| PolygonQuery::new(&region.rings))
                .collect(),
            pack.forest.iter().map(PolygonQuery::new).collect(),
            pack.ground
                .iter()
                .map(|(_, rings)| PolygonQuery::new(rings))
                .collect(),
            pack.flora
                .iter()
                .map(|region| PolygonQuery::new(&region.rings))
                .collect(),
        ];
        let bounds = std::array::from_fn(|layer| PolygonBoundsIndex::new(&layers[layer]));
        Self {
            pack,
            layers,
            bounds,
        }
    }

    pub(crate) fn pack(&self) -> &'a FeaturePack {
        self.pack
    }

    pub(crate) fn at(&self, point: [f32; 2]) -> (Option<GroundKind>, bool) {
        let candidates = point.into_iter().all(f32::is_finite).then(|| {
            std::array::from_fn(|layer| self.bounds[layer].candidates(point, &self.layers[layer]))
        });
        source_surface_with(self.pack, candidates.as_ref(), |layer, index, _| {
            let layer = match layer {
                SurfaceLayer::Regional => 0,
                SurfaceLayer::Forest => 1,
                SurfaceLayer::Ground => 2,
                SurfaceLayer::Flora => 3,
            };
            self.layers[layer][index].contains(point)
        })
    }

    /// Independent mapped forest evidence survives broad developed-land fills.
    /// The boolean returned by `at` is cultivation, not forest coverage.
    pub(crate) fn contains_forest(&self, point: [f32; 2]) -> bool {
        point.into_iter().all(f32::is_finite)
            && self.bounds[1]
                .candidates(point, &self.layers[1])
                .into_iter()
                .any(|index| self.layers[1][index].contains(point))
    }
}

struct PolygonBoundsNode {
    bounds: [f32; 4],
    range: std::ops::Range<usize>,
    children: Option<[usize; 2]>,
}

/// Storage grows linearly with source polygons, including very broad regions.
/// Polygons whose existing ray query cannot reject by finite bounds bypass it.
struct PolygonBoundsIndex {
    nodes: Vec<PolygonBoundsNode>,
    indices: Vec<usize>,
    fallback: Vec<usize>,
}

fn bounds_contain(bounds: [f32; 4], point: [f32; 2]) -> bool {
    point[0] >= bounds[0] && point[1] >= bounds[1] && point[0] <= bounds[2] && point[1] <= bounds[3]
}

impl PolygonBoundsIndex {
    fn new(polygons: &[PolygonQuery<'_>]) -> Self {
        let mut index = Self {
            nodes: Vec::new(),
            indices: Vec::new(),
            fallback: Vec::new(),
        };
        for (id, polygon) in polygons.iter().enumerate() {
            if !polygon.buckets.is_empty() && polygon.bounds.into_iter().all(f32::is_finite) {
                index.indices.push(id);
            } else {
                index.fallback.push(id);
            }
        }
        if !index.indices.is_empty() {
            Self::build(polygons, &mut index.indices, 0, &mut index.nodes);
        }
        index
    }

    fn build(
        polygons: &[PolygonQuery<'_>],
        indices: &mut [usize],
        first: usize,
        nodes: &mut Vec<PolygonBoundsNode>,
    ) -> usize {
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for &id in indices.iter() {
            let other = polygons[id].bounds;
            bounds[0] = bounds[0].min(other[0]);
            bounds[1] = bounds[1].min(other[1]);
            bounds[2] = bounds[2].max(other[2]);
            bounds[3] = bounds[3].max(other[3]);
        }
        let node = nodes.len();
        nodes.push(PolygonBoundsNode {
            bounds,
            range: first..first + indices.len(),
            children: None,
        });
        if indices.len() > 8 {
            let axis = usize::from(
                f64::from(bounds[3]) - f64::from(bounds[1])
                    > f64::from(bounds[2]) - f64::from(bounds[0]),
            );
            let middle = indices.len() / 2;
            let center = |id: usize| {
                let b = polygons[id].bounds;
                f64::from(b[axis]) + f64::from(b[axis + 2])
            };
            indices.select_nth_unstable_by(middle, |&a, &b| {
                center(a).total_cmp(&center(b)).then(a.cmp(&b))
            });
            let (left, right) = indices.split_at_mut(middle);
            let children = [
                Self::build(polygons, left, first, nodes),
                Self::build(polygons, right, first + middle, nodes),
            ];
            nodes[node].children = Some(children);
        }
        node
    }

    fn candidates(&self, point: [f32; 2], polygons: &[PolygonQuery<'_>]) -> Vec<usize> {
        let mut candidates = self.fallback.clone();
        if !self.nodes.is_empty() {
            self.visit(0, point, polygons, &mut candidates);
        }
        // Traversal order is spatial; source ordinals alone control precedence.
        candidates.sort_unstable();
        candidates
    }

    fn visit(
        &self,
        node: usize,
        point: [f32; 2],
        polygons: &[PolygonQuery<'_>],
        candidates: &mut Vec<usize>,
    ) {
        let node = &self.nodes[node];
        if !bounds_contain(node.bounds, point) {
            return;
        }
        if let Some(children) = node.children {
            for child in children {
                self.visit(child, point, polygons, candidates);
            }
        } else {
            candidates.extend(
                self.indices[node.range.clone()]
                    .iter()
                    .copied()
                    .filter(|&id| bounds_contain(polygons[id].bounds, point)),
            );
        }
    }
}

struct PolygonQuery<'a> {
    rings: &'a Rings,
    bounds: [f32; 4],
    edges: Vec<[[f32; 2]; 2]>,
    buckets: Vec<Vec<usize>>,
}

impl<'a> PolygonQuery<'a> {
    fn new(rings: &'a Rings) -> Self {
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        let mut edges = Vec::new();
        let mut finite = true;
        for ring in rings {
            for index in 0..ring.len() {
                let a = ring[index];
                let b = ring[(index + 1) % ring.len()];
                finite &= a.into_iter().all(f32::is_finite);
                bounds[0] = bounds[0].min(a[0]);
                bounds[1] = bounds[1].min(a[1]);
                bounds[2] = bounds[2].max(a[0]);
                bounds[3] = bounds[3].max(a[1]);
                if a[1] != b[1] {
                    // The ray test interpolates in f32. Its rounded endpoint
                    // can extend beyond b.x after cancellation, so bound that
                    // arithmetic too rather than only the source vertices.
                    let end_x = a[0] + (b[0] - a[0]);
                    bounds[0] = bounds[0].min(end_x);
                    bounds[2] = bounds[2].max(end_x);
                    edges.push([a, b]);
                }
            }
        }
        let mut query = Self {
            rings,
            bounds,
            edges,
            buckets: Vec::new(),
        };
        let extent = f64::from(bounds[3]) - f64::from(bounds[1]);
        if finite && extent.is_finite() && extent > 0.0 {
            query.buckets = vec![Vec::new(); (query.edges.len() / 8).clamp(1, 32)];
            for (index, [a, b]) in query.edges.iter().enumerate() {
                // Include a bucket on either side of each endpoint. Boundary
                // rounding can add candidates, never remove a crossing edge.
                let first = query.bucket(a[1].min(b[1])).saturating_sub(1);
                let last = (query.bucket(a[1].max(b[1])) + 1).min(query.buckets.len() - 1);
                for bucket in &mut query.buckets[first..=last] {
                    bucket.push(index);
                }
            }
        }
        query
    }

    fn bucket(&self, y: f32) -> usize {
        let relative = (f64::from(y) - f64::from(self.bounds[1]))
            / (f64::from(self.bounds[3]) - f64::from(self.bounds[1]));
        ((relative * self.buckets.len() as f64) as usize).min(self.buckets.len() - 1)
    }

    fn contains(&self, point: [f32; 2]) -> bool {
        if self.buckets.is_empty() || !point.into_iter().all(f32::is_finite) {
            return point_in_rings(point, self.rings);
        }
        if point[0] < self.bounds[0]
            || point[1] < self.bounds[1]
            || point[0] > self.bounds[2]
            || point[1] > self.bounds[3]
        {
            return false;
        }
        let mut inside = false;
        for &index in &self.buckets[self.bucket(point[1])] {
            let [a, b] = self.edges[index];
            if (a[1] > point[1]) != (b[1] > point[1]) {
                let t = (point[1] - a[1]) / (b[1] - a[1]);
                if point[0] < a[0] + t * (b[0] - a[0]) {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

#[cfg(test)]
mod tests {
    use super::super::{RegionalLandcoverMode, tree_source_surface};
    use super::*;
    use vearth_data::{
        protomaps_semantics::LandcoverKind,
        vectors::{FloraRegion, FloraRegionKind, RegionalLandcoverRegion},
    };

    fn square(x: f32, y: f32, size: f32) -> Rings {
        vec![vec![
            [x, y],
            [x + size, y],
            [x + size, y + size],
            [x, y + size],
        ]]
    }

    #[test]
    fn indexed_crossings_preserve_holes_boundaries_and_float_arithmetic() {
        let mut rings = vec![
            (0..113)
                .map(|index| {
                    let angle = index as f32 * std::f32::consts::TAU / 113.0;
                    let radius = if index % 3 == 0 { 15.0 } else { 40.0 };
                    [radius * angle.cos() + 1234.0, radius * angle.sin() - 18.0]
                })
                .collect(),
        ];
        rings.extend(square(1225.0, -24.0, 9.0));
        for rings in [
            rings,
            vec![vec![[1.0e20, -1.0], [-100.0, 1.0], [-200.0, -1.0]]],
            vec![vec![[f32::MAX, -2.0], [-f32::MAX, 2.0], [0.0, 4.0]]],
            vec![vec![[f32::NAN, 0.0], [5.0, 10.0], [-5.0, 10.0]]],
            vec![vec![], vec![[0.0, 0.0]]],
        ] {
            let query = PolygonQuery::new(&rings);
            let mut points = vec![
                [f32::NAN, 0.0],
                [f32::INFINITY, 0.0],
                [0.0, f32::NEG_INFINITY],
            ];
            for ring in &rings {
                for index in 0..ring.len() {
                    let a = ring[index];
                    let b = ring[(index + 1) % ring.len()];
                    for t in [0.0, 0.125, 0.5, 0.875, 1.0] {
                        let p = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                        points.extend([
                            p,
                            [p[0].next_up(), p[1]],
                            [p[0].next_down(), p[1]],
                            [p[0], p[1].next_up()],
                            [p[0], p[1].next_down()],
                        ]);
                    }
                }
            }
            for y in -200..200 {
                for x in -150..150 {
                    points.push([1234.0 + x as f32 * 0.375, -18.0 + y as f32 * 0.25]);
                }
            }
            for point in points {
                assert_eq!(
                    query.contains(point),
                    point_in_rings(point, &rings),
                    "{point:?}"
                );
            }
        }
    }

    fn layered_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        for index in 0..32 {
            let x = (index % 8) as f32 * 18.0;
            let y = (index / 8) as f32 * 18.0;
            pack.regional_landcover.push(RegionalLandcoverRegion {
                location_rad: None,
                rings: square(x - 10.0, y - 10.0, 42.0),
                kind: if index % 2 == 0 {
                    LandcoverKind::Farmland
                } else {
                    LandcoverKind::Forest
                },
                source_id: Some(index),
                stable_identity: index,
            });
            pack.forest.push(square(x, y, 12.0));
            for (offset, kind) in [
                GroundKind::NaturalGrass,
                GroundKind::Green,
                GroundKind::Paved,
            ]
            .into_iter()
            .enumerate()
            {
                pack.ground
                    .push((kind, square(x + offset as f32 * 3.0, y + 4.0, 8.0)));
            }
            pack.flora.push(FloraRegion {
                location_rad: None,
                rings: square(x + 5.0, y + 5.0, 14.0),
                kind: FloraRegionKind::Farmland,
                seed: index,
            });
        }
        pack
    }

    fn without_polygon_bounds(
        query: &SourceSurfaceQuery<'_>,
        point: [f32; 2],
    ) -> (Option<GroundKind>, bool) {
        source_surface_with(query.pack, None, |layer, index, _| {
            let layer = match layer {
                SurfaceLayer::Regional => 0,
                SurfaceLayer::Forest => 1,
                SurfaceLayer::Ground => 2,
                SurfaceLayer::Flora => 3,
            };
            query.layers[layer][index].contains(point)
        })
    }

    #[test]
    fn polygon_bounds_candidates_keep_numeric_fallbacks_and_source_order() {
        let mut rings: Vec<_> = (0..129)
            .map(|index| square(((index * 67) % 129) as f32 * 9.0 - 600.0, -30.0, 7.0))
            .collect();
        rings.push(vec![vec![[1.0e20, -1.0], [-100.0, 1.0], [-200.0, -1.0]]]);
        rings.push(vec![vec![[f32::MAX, -2.0], [-f32::MAX, 2.0], [0.0, 4.0]]]);
        rings.push(vec![vec![[f32::NAN, 0.0], [5.0, 10.0], [-5.0, 10.0]]]);
        rings.push(vec![vec![], vec![[0.0, 0.0]]]);
        let mut hole = square(-40.0, -40.0, 80.0);
        hole.extend(square(-10.0, -10.0, 20.0));
        rings.push(hole);
        let queries: Vec<_> = rings.iter().map(PolygonQuery::new).collect();
        let index = PolygonBoundsIndex::new(&queries);
        assert!(index.fallback.len() >= 3);
        assert!(index.nodes.len() < 2 * queries.len());
        assert_eq!(index.indices.len() + index.fallback.len(), queries.len());
        let mut points = vec![[0.0, 0.0], [-0.0, -0.0], [f32::MAX, 0.0], [-f32::MAX, 0.0]];
        for rings in &rings {
            for ring in rings {
                for &point in ring {
                    if point.into_iter().all(f32::is_finite) {
                        points.extend([
                            point,
                            [point[0].next_up(), point[1]],
                            [point[0].next_down(), point[1]],
                            [point[0], point[1].next_up()],
                            [point[0], point[1].next_down()],
                        ]);
                    }
                }
            }
        }
        for point in points
            .into_iter()
            .filter(|p| p.into_iter().all(|v| v.is_finite()))
        {
            let candidates = index.candidates(point, &queries);
            assert!(candidates.windows(2).all(|pair| pair[0] < pair[1]));
            for (id, query) in queries.iter().enumerate() {
                if query.contains(point) {
                    assert!(
                        candidates.binary_search(&id).is_ok(),
                        "omitted {id} at {point:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn polygon_candidate_queries_preserve_mixed_holes_and_layer_precedence() {
        let mut pack = layered_pack();
        for ring in &mut pack.forest {
            let origin = ring[0][0];
            ring.extend(square(origin[0] + 3.0, origin[1] + 3.0, 5.0));
        }
        pack.ground
            .push((GroundKind::Paved, square(-100.0, -100.0, 8.0)));
        pack.ground
            .push((GroundKind::Urban, square(-100.0, -100.0, 8.0)));
        pack.flora.push(FloraRegion {
            location_rad: None,
            rings: square(-100.0, -100.0, 8.0),
            kind: FloraRegionKind::Farmland,
            seed: 123,
        });
        pack.ground.push((
            GroundKind::Sand,
            vec![vec![[f32::NAN, 0.0], [5.0, 10.0], [-5.0, 10.0]]],
        ));
        for mode in [
            RegionalLandcoverMode::ExistingFarmlandOnly,
            RegionalLandcoverMode::SelectiveRegionalTerrain,
        ] {
            pack.regional_landcover_mode = mode;
            let query = SourceSurfaceQuery::new(&pack);
            assert_eq!(query.at([-96.0, -96.0]), (Some(GroundKind::Urban), false));
            let mut points = vec![
                [f32::NAN, 0.0],
                [f32::INFINITY, 0.0],
                [0.0, f32::NEG_INFINITY],
            ];
            for layer in &query.layers {
                for polygon in layer {
                    for ring in polygon.rings {
                        for &p in ring {
                            points.extend([p, [p[0].next_up(), p[1]], [p[0], p[1].next_down()]]);
                        }
                    }
                }
            }
            for point in points {
                assert_eq!(
                    query.at(point),
                    without_polygon_bounds(&query, point),
                    "{mode:?} {point:?}"
                );
                assert_eq!(
                    query.at(point),
                    tree_source_surface(&pack, point),
                    "raw {mode:?} {point:?}"
                );
            }
        }
    }

    fn scattered_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        for index in 0..4096 {
            let position = (index * 2081) % 4096;
            pack.ground.push((
                match index % 3 {
                    0 => GroundKind::NaturalGrass,
                    1 => GroundKind::Farmyard,
                    _ => GroundKind::Green,
                },
                square(
                    (position % 64) as f32 * 8.0,
                    (position / 64) as f32 * 8.0,
                    6.0,
                ),
            ));
        }
        pack
    }

    #[test]
    fn polygon_candidate_queries_avoid_scanning_unrelated_source_features() {
        let pack = scattered_pack();
        let query = SourceSurfaceQuery::new(&pack);
        let mut visited = 0;
        for index in 0..1024 {
            let position = (index * 41) % 4096;
            let point = [
                (position % 64) as f32 * 8.0 + 3.0,
                (position / 64) as f32 * 8.0 + 3.0,
            ];
            let candidates = query.bounds[2].candidates(point, &query.layers[2]);
            assert_eq!(candidates.len(), 1);
            visited += candidates.len();
            assert_eq!(query.at(point), without_polygon_bounds(&query, point));
        }
        assert_eq!(visited, 1024);
        assert_eq!(query.bounds[2].indices.len(), 4096);
        assert!(query.bounds[2].nodes.len() < 8192);
    }

    #[test]
    #[ignore = "bounded polygon-candidate lookup benchmark with previous indexed-ray oracle"]
    fn profile_polygon_candidate_queries() {
        let pack = scattered_pack();
        let points: Vec<_> = (0..4096)
            .map(|index| {
                let position = (index * 41) % 4096;
                [
                    (position % 64) as f32 * 8.0 + 3.0,
                    (position / 64) as f32 * 8.0 + 3.0,
                ]
            })
            .collect();
        let query = SourceSurfaceQuery::new(&pack);
        let start = vearth_platform::time::Instant::now();
        let expected: Vec<_> = points
            .iter()
            .map(|&p| without_polygon_bounds(&query, p))
            .collect();
        let reference = start.elapsed();
        let start = vearth_platform::time::Instant::now();
        let query = SourceSurfaceQuery::new(&pack);
        let actual: Vec<_> = points.iter().map(|&p| query.at(p)).collect();
        let candidate = start.elapsed();
        assert_eq!(actual, expected);
        eprintln!(
            "polygon candidate lookup: {reference:?} previous indexed-ray query, {candidate:?} including new index construction; {} source polygons and {} points",
            pack.ground.len(),
            points.len()
        );
    }

    #[test]
    fn prepared_source_queries_preserve_layer_precedence_and_cultivation() {
        let mut pack = layered_pack();
        for mode in [
            RegionalLandcoverMode::ExistingFarmlandOnly,
            RegionalLandcoverMode::SelectiveRegionalTerrain,
        ] {
            pack.regional_landcover_mode = mode;
            let query = SourceSurfaceQuery::new(&pack);
            for y in -20..190 {
                for x in -20..340 {
                    let point = [x as f32 * 0.5, y as f32 * 0.5];
                    assert_eq!(
                        query.at(point),
                        tree_source_surface(&pack, point),
                        "{mode:?} {point:?}"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "bounded source-membership microbenchmark"]
    fn profile_prepared_source_queries() {
        let pack = layered_pack();
        let points: Vec<_> = (0..50_000)
            .map(|index| {
                [
                    ((index * 41) % 370) as f32 * 0.5 - 20.0,
                    ((index * 71) % 210) as f32 * 0.5 - 20.0,
                ]
            })
            .collect();
        let start = vearth_platform::time::Instant::now();
        let expected: Vec<_> = points
            .iter()
            .map(|&point| tree_source_surface(&pack, point))
            .collect();
        let reference = start.elapsed();
        let start = vearth_platform::time::Instant::now();
        let query = SourceSurfaceQuery::new(&pack);
        let actual: Vec<_> = points.iter().map(|&point| query.at(point)).collect();
        let prepared = start.elapsed();
        assert_eq!(actual, expected);
        eprintln!("source query: {reference:?} reference, {prepared:?} prepared (including index)");
    }
}
