use glam::DVec2;
use std::{collections::HashMap, sync::OnceLock};

type Polygon = &'static [[f64; 2]];

struct Bounds {
    low: DVec2,
    high: DVec2,
}

impl Bounds {
    fn points(points: impl Iterator<Item = DVec2>) -> Self {
        points.fold(
            Self {
                low: DVec2::splat(f64::INFINITY),
                high: DVec2::splat(f64::NEG_INFINITY),
            },
            |a, p| Self {
                low: a.low.min(p),
                high: a.high.max(p),
            },
        )
    }

    fn intersects(&self, other: &Self) -> bool {
        self.low.cmple(other.high).all() && self.high.cmpge(other.low).all()
    }
}

enum Node {
    Leaf {
        bounds: Bounds,
        polygons: Vec<Polygon>,
    },
    Branch {
        bounds: Bounds,
        children: [Box<Node>; 2],
    },
}

impl Node {
    fn build(mut polygons: Vec<Polygon>) -> Self {
        let bounds = Bounds::points(
            polygons
                .iter()
                .flat_map(|p| p.iter().copied().map(DVec2::from)),
        );
        if polygons.len() <= 8 {
            return Self::Leaf { bounds, polygons };
        }
        let axis = usize::from(bounds.high.y - bounds.low.y > bounds.high.x - bounds.low.x);
        polygons.sort_by(|a, b| {
            let center = |p: &Polygon| {
                let b = Bounds::points(p.iter().copied().map(DVec2::from));
                b.low[axis] + b.high[axis]
            };
            center(a).total_cmp(&center(b))
        });
        let right = polygons.split_off(polygons.len() / 2);
        Self::Branch {
            bounds,
            children: [
                Box::new(Self::build(polygons)),
                Box::new(Self::build(right)),
            ],
        }
    }

    fn any(&self, query: &Bounds, predicate: &impl Fn(Polygon) -> bool) -> bool {
        match self {
            Self::Leaf { bounds, polygons } => {
                bounds.intersects(query) && polygons.iter().any(|p| predicate(p))
            }
            Self::Branch { bounds, children } => {
                bounds.intersects(query) && children.iter().any(|node| node.any(query, predicate))
            }
        }
    }
}

pub(super) struct FootprintIndex(Node);

impl FootprintIndex {
    pub fn contains(&self, point: DVec2, pad: f64) -> bool {
        let pad = pad.max(0.0);
        self.0.any(
            &Bounds {
                low: point - pad,
                high: point + pad,
            },
            &|p| contains(p, point, pad),
        )
    }

    pub fn overlaps(&self, rings: &[Vec<DVec2>]) -> bool {
        let bounds = Bounds::points(rings.iter().flatten().copied());
        self.0.any(&bounds, &|p| overlaps(p, rings))
    }
}

pub(super) fn source_footprint(name: &str) -> Option<&'static FootprintIndex> {
    // Long defensive circuits own many narrow polygons. Decode their spatial
    // hierarchy once, shared by every terrain cell and source-building query.
    static INDEX: OnceLock<HashMap<&'static str, FootprintIndex>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            super::curated::models()
                .filter_map(|lm| {
                    super::curated::source_exclusion_polygons(lm.name)
                        .map(|polygons| (lm.name, FootprintIndex(Node::build(polygons.to_vec()))))
                })
                .collect()
        })
        .get(name)
}

pub(super) struct TerrainCut {
    pub polygon: &'static [[f64; 2]],
    // Model-space x/z slopes and a height relative to the landmark's base.
    // This correction deliberately does not change the mesh anchoring datum.
    pub plane: [f64; 3],
    pub feather_m: f64,
}

fn edge_distance(polygon: &[[f64; 2]], point: DVec2) -> f64 {
    let a = DVec2::from(polygon[0]);
    let b = DVec2::from(polygon[1]);
    let c = DVec2::from(polygon[2]);
    let winding = (b - a).perp_dot(c - b).signum();
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(&a, &b)| {
            let a = DVec2::from(a);
            let edge = DVec2::from(b) - a;
            winding * edge.perp_dot(point - a) / edge.length()
        })
        .fold(f64::INFINITY, f64::min)
}

pub(super) fn contains(polygon: &[[f64; 2]], point: DVec2, pad: f64) -> bool {
    if edge_distance(polygon, point) >= 0.0 {
        return true;
    }
    // Euclidean padding also bounds acute corners by the query's padded AABB.
    // Offsetting infinite edge lines would extend their mitres beyond it.
    pad > 0.0
        && polygon
            .iter()
            .zip(polygon.iter().cycle().skip(1))
            .any(|(&a, &b)| {
                let a = DVec2::from(a);
                let edge = DVec2::from(b) - a;
                let t = ((point - a).dot(edge) / edge.length_squared()).clamp(0.0, 1.0);
                point.distance_squared(a + t * edge) <= pad * pad
            })
}

pub(super) fn cut_height(
    cuts: &[TerrainCut],
    point: DVec2,
    raw: f32,
    base: f32,
) -> Option<(f32, f64)> {
    height_from_cuts(cuts.iter(), point, raw, base)
}

fn height_from_cuts<'a>(
    cuts: impl Iterator<Item = &'a TerrainCut>,
    point: DVec2,
    raw: f32,
    base: f32,
) -> Option<(f32, f64)> {
    cuts.filter_map(|cut| {
        let weight = patch_weight(cut, point)?;
        let target = base + (cut.plane[0] * point.x + cut.plane[1] * point.y + cut.plane[2]) as f32;
        Some((raw + (raw.min(target) - raw) * weight as f32, weight))
    })
    .min_by(|a, b| a.0.total_cmp(&b.0))
}

fn patch_weight(cut: &TerrainCut, point: DVec2) -> Option<f64> {
    let mut distance = edge_distance(cut.polygon, point);
    if distance <= -cut.feather_m {
        return None;
    }
    if distance < 0.0 {
        // Feather by physical distance to the finite polygon. Extended
        // edge lines create unbounded mitres at acute survey triangles.
        distance = -cut
            .polygon
            .iter()
            .zip(cut.polygon.iter().cycle().skip(1))
            .map(|(&a, &b)| {
                let a = DVec2::from(a);
                let edge = DVec2::from(b) - a;
                let t = ((point - a).dot(edge) / edge.length_squared()).clamp(0.0, 1.0);
                point.distance(a + edge * t)
            })
            .fold(f64::INFINITY, f64::min);
    }
    if distance <= -cut.feather_m {
        return None;
    }
    let t = (1.0 + distance / cut.feather_m).clamp(0.0, 1.0);
    Some(t * t * (3.0 - 2.0 * t))
}

// Explicit surveyed grades can fill a false DEM depression. They do not
// participate in the model datum; the strongest feather selects overlapping
// patches, with source order breaking ties.
fn grade_height<'a>(
    grades: impl IntoIterator<Item = &'a TerrainCut>,
    point: DVec2,
    raw: f32,
    base: f32,
) -> Option<(f32, f64)> {
    grades
        .into_iter()
        .filter_map(|grade| {
            let weight = patch_weight(grade, point)?;
            let target = base
                + (grade.plane[0] * point.x + grade.plane[1] * point.y + grade.plane[2]) as f32;
            Some((raw + (target - raw) * weight as f32, weight))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

const CUT_BIN_M: f64 = 32.0;

struct CutIndex {
    cells: HashMap<(i32, i32), Vec<&'static TerrainCut>>,
}

impl CutIndex {
    fn key(point: DVec2) -> (i32, i32) {
        (
            (point.x / CUT_BIN_M).floor() as i32,
            (point.y / CUT_BIN_M).floor() as i32,
        )
    }

    fn build(cuts: &'static [TerrainCut]) -> Self {
        let mut cells: HashMap<_, Vec<_>> = HashMap::new();
        for cut in cuts {
            let bounds = Bounds::points(cut.polygon.iter().copied().map(DVec2::from));
            let low = Self::key(bounds.low - cut.feather_m);
            let high = Self::key(bounds.high + cut.feather_m);
            for z in low.1..=high.1 {
                for x in low.0..=high.0 {
                    cells.entry((x, z)).or_default().push(cut);
                }
            }
        }
        Self { cells }
    }

    fn height(&self, point: DVec2, raw: f32, base: f32) -> Option<(f32, f64)> {
        height_from_cuts(
            self.cells.get(&Self::key(point))?.iter().copied(),
            point,
            raw,
            base,
        )
    }
}

pub(super) fn indexed_height(
    name: &str,
    water: bool,
    point: DVec2,
    raw: f32,
    base: f32,
) -> Option<(f32, f64)> {
    let source = if water {
        super::curated::water_cuts
    } else {
        super::curated::terrain_cuts
    };
    let cuts = source(name);
    if cuts.len() <= 16 {
        return cut_height(cuts, point, raw, base);
    }
    // Detailed relief is immutable and shared across cells. Keep source order
    // in each bin so overlapping patches retain the same minimum/tie behavior.
    static TERRAIN: OnceLock<HashMap<&'static str, CutIndex>> = OnceLock::new();
    static WATER: OnceLock<HashMap<&'static str, CutIndex>> = OnceLock::new();
    let indices = if water { &WATER } else { &TERRAIN };
    indices
        .get_or_init(|| {
            super::curated::models()
                .filter_map(|lm| {
                    let cuts = source(lm.name);
                    (cuts.len() > 16).then(|| (lm.name, CutIndex::build(cuts)))
                })
                .collect()
        })
        .get(name)?
        .height(point, raw, base)
}

pub(super) fn indexed_grade_height(
    name: &str,
    point: DVec2,
    raw: f32,
    base: f32,
) -> Option<(f32, f64)> {
    let grades = super::curated::terrain_grades(name);
    if grades.len() <= 16 {
        return grade_height(grades, point, raw, base);
    }
    // Detailed island relief has many small triangles. Reuse the immutable
    // spatial bins so a terrain sample visits only nearby grades.
    static GRADES: OnceLock<HashMap<&'static str, CutIndex>> = OnceLock::new();
    let index = GRADES
        .get_or_init(|| {
            super::curated::models()
                .filter_map(|lm| {
                    let grades = super::curated::terrain_grades(lm.name);
                    (grades.len() > 16).then(|| (lm.name, CutIndex::build(grades)))
                })
                .collect()
        })
        .get(name)?;
    grade_height(
        index.cells.get(&CutIndex::key(point))?.iter().copied(),
        point,
        raw,
        base,
    )
}

fn in_building(rings: &[Vec<DVec2>], point: DVec2) -> bool {
    let mut inside = false;
    for ring in rings {
        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            if (a.y > point.y) != (b.y > point.y)
                && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
            {
                inside = !inside;
            }
        }
    }
    inside
}

pub(super) fn overlaps(polygon: &[[f64; 2]], rings: &[Vec<DVec2>]) -> bool {
    if rings.iter().flatten().any(|&p| contains(polygon, p, 1e-7))
        || polygon.iter().any(|&p| in_building(rings, p.into()))
    {
        return true;
    }
    for (&a, &b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        let a = DVec2::from(a);
        let b = DVec2::from(b);
        for ring in rings {
            for (&c, &d) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let ab = b - a;
                let cd = d - c;
                let den = ab.perp_dot(cd);
                if den.abs() > 1e-10 {
                    let t = (c - a).perp_dot(cd) / den;
                    let u = (c - a).perp_dot(ab) / den;
                    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_island_grades_match_direct_evaluation_at_edges_and_interiors() {
        let name = "bled-island";
        let grades = super::super::curated::terrain_grades(name);
        assert!(grades.len() > 16);
        for x in (-60..=45).step_by(3) {
            for z in (-114..=78).step_by(3) {
                let p = DVec2::new(x as f64 + 0.017, z as f64 - 0.013);
                assert_eq!(
                    indexed_grade_height(name, p, 474.8, 475.58548),
                    grade_height(grades, p, 474.8, 475.58548),
                    "({x}, {z})"
                );
            }
        }
        for grade in grades.iter().step_by(13) {
            for &p in grade.polygon {
                let p = DVec2::from(p);
                assert_eq!(
                    indexed_grade_height(name, p, 474.8, 475.58548),
                    grade_height(grades, p, 474.8, 475.58548)
                );
            }
        }
    }

    #[test]
    fn surveyed_grade_fills_depression_and_feathers_within_bounds() {
        let grades = [TerrainCut {
            polygon: &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            plane: [0.0, 0.0, -9.0],
            feather_m: 4.0,
        }];
        assert_eq!(
            grade_height(&grades, DVec2::splat(5.0), -6.0, 13.5),
            Some((4.5, 1.0))
        );
        assert_eq!(
            grade_height(&grades, DVec2::new(12.0, 5.0), -6.0, 13.5),
            Some((-0.75, 0.5))
        );
        assert_eq!(
            grade_height(&grades, DVec2::new(14.0, 5.0), -6.0, 13.5),
            None
        );
        assert_eq!(
            grade_height(&grades, DVec2::splat(5.0), 9.0, 13.5),
            Some((4.5, 1.0))
        );
        assert_eq!(
            cut_height(&grades, DVec2::splat(5.0), -6.0, 13.5),
            Some((-6.0, 1.0))
        );
    }

    #[test]
    fn indexed_cuts_match_full_scan_at_interiors_edges_and_bin_boundaries() {
        let cuts: &'static [TerrainCut] = Box::leak(
            (0..256)
                .map(|i| {
                    let x = f64::from(i % 16) * 16.0;
                    let z = f64::from(i / 16) * 16.0;
                    TerrainCut {
                        polygon: Box::leak(Box::new([[x, z], [x + 16.0, z], [x + 16.0, z + 16.0]])),
                        plane: [0.01, -0.02, -f64::from(i % 9)],
                        feather_m: 3.0,
                    }
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let index = CutIndex::build(cuts);
        assert!(index.cells.values().map(Vec::len).max().unwrap() <= 16);
        for i in 0..4096 {
            let point = DVec2::new(
                f64::from(i % 64) * 4.25 - 8.0,
                f64::from(i / 64) * 4.25 - 8.0,
            );
            for raw in [80.0, 100.0, 140.0] {
                assert_eq!(
                    index.height(point, raw, 100.0),
                    cut_height(cuts, point, raw, 100.0)
                );
            }
        }
    }

    #[test]
    fn acute_cut_feather_cannot_escape_its_activation_padding() {
        let cuts = [TerrainCut {
            polygon: &[[0.0, 0.0], [100.0, 0.0], [0.0, 1.0]],
            plane: [0.0, 0.0, -10.0],
            feather_m: 2.0,
        }];
        assert_eq!(
            cut_height(&cuts, DVec2::new(103.0, 0.0), 100.0, 100.0),
            None
        );
        assert_eq!(
            cut_height(&cuts, DVec2::new(101.0, 0.0), 100.0, 100.0),
            Some((95.0, 0.5))
        );
    }

    const STRIP: &[[f64; 2]] = &[[0.0, 0.0], [10.0, 0.0], [10.0, 2.0], [0.0, 2.0]];

    #[test]
    fn spatial_queries_match_full_scan_across_branches_and_padding() {
        let polygons: Vec<Polygon> = (0..24)
            .map(|i| {
                let x = f64::from(i % 6) * 17.0;
                let z = f64::from(i / 6) * 19.0;
                let mut polygon = STRIP
                    .iter()
                    .map(|p| [p[0] + x, p[1] + z])
                    .collect::<Vec<_>>();
                if i % 2 == 0 {
                    polygon.reverse();
                }
                &*Box::leak(polygon.into_boxed_slice())
            })
            .collect();
        let index = FootprintIndex(Node::build(polygons.clone()));
        for x in -2..102 {
            for z in -2..63 {
                let p = DVec2::new(f64::from(x) + 0.3, f64::from(z) + 0.2);
                for pad in [0.0, 0.5, 2.0] {
                    assert_eq!(
                        index.contains(p, pad),
                        polygons.iter().any(|q| contains(q, p, pad))
                    );
                }
                let rings = vec![vec![
                    p,
                    p + DVec2::new(12.0, 0.0),
                    p + DVec2::new(12.0, 5.0),
                    p + DVec2::new(0.0, 5.0),
                ]];
                assert_eq!(
                    index.overlaps(&rings),
                    polygons.iter().any(|q| overlaps(q, &rings))
                );
            }
        }
        assert!(!index.overlaps(&[]));
        let empty = FootprintIndex(Node::build(Vec::new()));
        assert!(!empty.contains(DVec2::ZERO, 3.0));
    }

    #[test]
    fn padding_is_bounded_even_at_acute_corners() {
        let triangle = &[[0.0, 0.0], [10.0, 0.0], [0.0, 1.0]];
        assert!(contains(triangle, DVec2::new(10.5, 0.0), 1.0));
        assert!(!contains(triangle, DVec2::new(11.1, 0.0), 1.0));
        assert!(!contains(STRIP, DVec2::new(10.8, 2.8), 1.0));
    }

    #[test]
    fn wall_corridor_preserves_courtyard_and_crossing_sources_are_found() {
        let courtyard = vec![vec![
            DVec2::new(3.0, 3.0),
            DVec2::new(6.0, 3.0),
            DVec2::new(6.0, 6.0),
            DVec2::new(3.0, 6.0),
        ]];
        assert!(!overlaps(STRIP, &courtyard));
        let crossing = vec![vec![
            DVec2::new(4.0, -3.0),
            DVec2::new(5.0, -3.0),
            DVec2::new(5.0, 4.0),
            DVec2::new(4.0, 4.0),
        ]];
        assert!(overlaps(STRIP, &crossing));
        let mut reverse = STRIP.to_vec();
        reverse.reverse();
        assert!(contains(&reverse, DVec2::new(3.0, 1.0), 0.0));
    }

    #[test]
    fn source_holes_do_not_count_as_building_mass() {
        let rings = vec![
            vec![
                DVec2::new(-5.0, -5.0),
                DVec2::new(15.0, -5.0),
                DVec2::new(15.0, 15.0),
                DVec2::new(-5.0, 15.0),
            ],
            vec![
                DVec2::new(-1.0, -1.0),
                DVec2::new(11.0, -1.0),
                DVec2::new(11.0, 3.0),
                DVec2::new(-1.0, 3.0),
            ],
        ];
        assert!(!overlaps(STRIP, &rings));
        assert!(overlaps(
            &[[12.0, 1.0], [13.0, 1.0], [13.0, 2.0], [12.0, 2.0]],
            &rings
        ));
    }

    #[test]
    fn excavation_is_local_lower_only_and_relative_to_rim_datum() {
        let cuts = [TerrainCut {
            polygon: STRIP,
            plane: [0.0, 0.0, -12.0],
            feather_m: 1.0,
        }];
        assert_eq!(
            cut_height(&cuts, DVec2::new(5.0, 1.0), 100.0, 100.0),
            Some((88.0, 1.0))
        );
        assert_eq!(
            cut_height(&cuts, DVec2::new(5.0, 1.0), 80.0, 100.0),
            Some((80.0, 1.0))
        );
        assert_eq!(cut_height(&cuts, DVec2::new(5.0, 4.0), 100.0, 100.0), None);
        assert_eq!(
            cut_height(&cuts, DVec2::new(5.0, 2.5), 100.0, 100.0),
            Some((94.0, 0.5))
        );
    }
}
