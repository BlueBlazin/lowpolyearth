//! Physical terrain samples and their immutable descendant range summary.

use crate::TerrainSurfaceData;
use vearth_core::{
    cell::CellId,
    contracts::{Fingerprint, SemanticHasher},
};

const SIDE: usize = 32;
const CORNERS: usize = (SIDE + 1) * (SIDE + 1);
const DEPTH: u8 = 5;
const NODES: usize = (4usize.pow(DEPTH as u32 + 1) - 1) / 3;
/// Share of a region's owned quads whose forward step reaches its broad step.
/// Isolated walls, cuttings and embankments occupy fewer quads than this, so
/// they cannot raise the value; slopes covering the region do.
const BROAD_STEP_QUANTILE: f64 = 0.9;

/// Geodetic terrain heights on the cell's shared 33×33 corner lattice.
/// Rows follow local +z, opposite cube-face +v. Ownership addresses 32×32 quads.
/// Separate structures, visual skirts and cell-frame curvature are excluded.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainDetailGrid {
    pub heights_m: Box<[f32; CORNERS]>,
    pub source_columns: [u32; SIDE],
}

impl TerrainDetailGrid {
    /// Copy owned physical corners; inactive corners have canonical zero bits.
    /// A nonfinite corner incident to an owned quad makes the grid unavailable.
    pub fn from_source(source: &TerrainSurfaceData) -> Option<Self> {
        let mut heights_m = Box::new([0.0; CORNERS]);
        for z in 0..SIDE {
            for x in 0..SIDE {
                if source.source_columns[z] & (1 << x) == 0 {
                    continue;
                }
                for index in corner_indices(x, z) {
                    let height = source.heights_m[index];
                    if !height.is_finite() {
                        return None;
                    }
                    heights_m[index] = height;
                }
            }
        }
        Some(Self {
            heights_m,
            source_columns: source.source_columns,
        })
    }

    /// Validate the complete serialized payload, including inactive samples.
    pub fn validate(&self) -> Result<(), String> {
        if self.heights_m.iter().any(|height| !height.is_finite()) {
            return Err("terrain detail grid contains a nonfinite height".into());
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Fingerprint {
        let mut hasher = SemanticHasher::new("vearth.terrain-detail-grid.v1");
        for &height in self.heights_m.iter() {
            hasher.f32(height);
        }
        for mask in self.source_columns {
            hasher.u32(mask);
        }
        hasher.finish()
    }
}

/// Owned corner extrema and the existing two-forward-edge step proxy, in metres.
/// Finite `min_m > max_m` with zero step denotes known-empty terrain ownership.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainDetailRange {
    pub min_m: f32,
    pub max_m: f32,
    pub step_m: f32,
}

impl TerrainDetailRange {
    const EMPTY: Self = Self {
        min_m: f32::MAX,
        max_m: -f32::MAX,
        step_m: 0.0,
    };

    fn include(&mut self, other: Self) {
        self.min_m = self.min_m.min(other.min_m);
        self.max_m = self.max_m.max(other.max_m);
        self.step_m = self.step_m.max(other.step_m);
    }
}

/// Five dyadic subdivision levels plus their root, built once from owned samples.
/// A finer query returns its containing measured quad without inventing samples.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainDetailSummary {
    nodes: Box<[TerrainDetailRange; NODES]>,
    /// Per node, the step reached by the steepest tenth of its owned quads.
    broad_steps: Box<[f32; NODES]>,
}

impl TerrainDetailSummary {
    pub fn from_grid(grid: &TerrainDetailGrid) -> Option<Self> {
        grid.validate().ok()?;
        let mut nodes = Box::new([TerrainDetailRange::EMPTY; NODES]);
        for z in 0..SIDE {
            for x in 0..SIDE {
                if grid.source_columns[z] & (1 << x) == 0 {
                    continue;
                }
                let heights = corner_indices(x, z).map(|index| grid.heights_m[index]);
                let step = (f64::from(heights[1]) - f64::from(heights[0]))
                    .abs()
                    .max((f64::from(heights[2]) - f64::from(heights[0])).abs());
                nodes[node_index(DEPTH, x, z)] = TerrainDetailRange {
                    min_m: heights.into_iter().fold(f32::INFINITY, f32::min),
                    max_m: heights.into_iter().fold(f32::NEG_INFINITY, f32::max),
                    // Finite input differences can exceed f32. The scalar
                    // convention already saturates LOD preference at that scale.
                    step_m: step.min(f64::from(f32::MAX)) as f32,
                };
            }
        }
        for depth in (0..DEPTH).rev() {
            let side = 1usize << depth;
            for z in 0..side {
                for x in 0..side {
                    let mut range = TerrainDetailRange::EMPTY;
                    for dz in 0..2 {
                        for dx in 0..2 {
                            range.include(nodes[node_index(depth + 1, x * 2 + dx, z * 2 + dz)]);
                        }
                    }
                    nodes[node_index(depth, x, z)] = range;
                }
            }
        }
        // Quantiles do not compose from children, so each node ranks the
        // owned leaf steps it covers. Known-empty nodes keep a zero step.
        let mut broad_steps = Box::new([0.0; NODES]);
        let mut steps = Vec::with_capacity(SIDE * SIDE);
        for depth in 0..=DEPTH {
            let side = 1usize << depth;
            let span = SIDE >> depth;
            for z in 0..side {
                for x in 0..side {
                    steps.clear();
                    for leaf_z in z * span..(z + 1) * span {
                        for leaf_x in x * span..(x + 1) * span {
                            if grid.source_columns[leaf_z] & (1 << leaf_x) != 0 {
                                steps.push(nodes[node_index(DEPTH, leaf_x, leaf_z)].step_m);
                            }
                        }
                    }
                    if steps.is_empty() {
                        continue;
                    }
                    let rank = ((steps.len() - 1) as f64 * BROAD_STEP_QUANTILE).round() as usize;
                    let (_, broad, _) = steps.select_nth_unstable_by(rank, f32::total_cmp);
                    broad_steps[node_index(depth, x, z)] = *broad;
                }
            }
        }
        Some(Self { nodes, broad_steps })
    }

    /// Read a same-face descendant's range in constant time. Invalid addresses
    /// or ancestry return `None`; known-empty ownership remains a valid range.
    pub fn query(&self, source: CellId, target: CellId) -> Option<TerrainDetailRange> {
        Some(self.nodes[node_for(source, target)?])
    }

    /// The step reached by the steepest tenth of the owned quads in a
    /// same-face descendant's region, in metres. Unlike [`TerrainDetailRange`]'s
    /// maximum it ignores isolated steep features. Known-empty regions return zero.
    pub fn broad_step(&self, source: CellId, target: CellId) -> Option<f32> {
        Some(self.broad_steps[node_for(source, target)?])
    }

    pub fn heap_bytes(&self) -> usize {
        std::mem::size_of_val(self.nodes.as_ref())
            + std::mem::size_of_val(self.broad_steps.as_ref())
    }

    pub fn fingerprint(&self) -> Fingerprint {
        let mut hasher = SemanticHasher::new("vearth.terrain-detail-summary.v1");
        for range in self.nodes.iter() {
            hasher.f32(range.min_m);
            hasher.f32(range.max_m);
            hasher.f32(range.step_m);
        }
        hasher.finish()
    }
}

fn node_for(source: CellId, target: CellId) -> Option<usize> {
    if !valid_cell(source)
        || !valid_cell(target)
        || source.face != target.face
        || target.level < source.level
    {
        return None;
    }
    let delta = target.level - source.level;
    if target.i >> delta != source.i || target.j >> delta != source.j {
        return None;
    }
    let depth = delta.min(DEPTH);
    let mask = (1u32 << depth) - 1;
    let x = (target.i >> (delta - depth)) & mask;
    let v = (target.j >> (delta - depth)) & mask;
    // column_dir samples cell.uv_at(fx, 1-fz). Integer dyadic addressing
    // avoids any tangent-frame or inverse-projection approximation here.
    let z = mask - v;
    Some(node_index(depth, x as usize, z as usize))
}

fn valid_cell(cell: CellId) -> bool {
    cell.face.0 < 6 && cell.level <= 30 && cell.i < 1 << cell.level && cell.j < 1 << cell.level
}

fn corner_indices(x: usize, z: usize) -> [usize; 4] {
    let first = z * (SIDE + 1) + x;
    [first, first + 1, first + SIDE + 1, first + SIDE + 2]
}

fn node_index(depth: u8, x: usize, z: usize) -> usize {
    ((1usize << (2 * depth)) - 1) / 3 + (z << depth) + x
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::{cell::ChunkAddr, cubesphere::Face, frame::ChunkFrame};

    fn varied_grid() -> TerrainDetailGrid {
        TerrainDetailGrid {
            heights_m: Box::new(std::array::from_fn(|i| {
                let (x, z) = (i % 33, i / 33);
                ((x * 173 + z * 67 + x * z * 3) % 2003) as f32 * 0.125 - 95.0
            })),
            source_columns: std::array::from_fn(|z| 0xa59e_f01du32.rotate_left(z as u32)),
        }
    }

    fn brute_range(
        grid: &TerrainDetailGrid,
        x0: usize,
        z0: usize,
        side: usize,
    ) -> TerrainDetailRange {
        let mut range = TerrainDetailRange::EMPTY;
        for z in z0..z0 + side {
            for x in x0..x0 + side {
                if grid.source_columns[z] & (1 << x) == 0 {
                    continue;
                }
                let h = corner_indices(x, z).map(|i| grid.heights_m[i]);
                for value in h {
                    range.min_m = range.min_m.min(value);
                    range.max_m = range.max_m.max(value);
                }
                range.step_m = range
                    .step_m
                    .max((h[1] - h[0]).abs().max((h[2] - h[0]).abs()));
            }
        }
        range
    }

    #[test]
    fn terrain_detail_all_nodes_match_owned_quad_oracle() {
        let grid = varied_grid();
        let summary = TerrainDetailSummary::from_grid(&grid).unwrap();
        assert_eq!(summary.heap_bytes(), 21_840);
        for face in 0..6 {
            for level in [0, 3, 11, 25] {
                let source = CellId {
                    face: Face(face),
                    level,
                    i: (1 << level) - 1,
                    j: 0,
                };
                for depth in 0..=DEPTH {
                    let width = 1usize << depth;
                    let side = SIDE / width;
                    for v in 0..width {
                        for x in 0..width {
                            let target = CellId {
                                face: source.face,
                                level: source.level + depth,
                                i: (source.i << depth) + x as u32,
                                j: (source.j << depth) + v as u32,
                            };
                            assert_eq!(
                                summary.query(source, target),
                                Some(brute_range(&grid, x * side, (width - v - 1) * side, side,)),
                                "{source:?} -> {target:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    fn brute_broad_step(grid: &TerrainDetailGrid, x0: usize, z0: usize, side: usize) -> f32 {
        let mut steps = Vec::new();
        for z in z0..z0 + side {
            for x in x0..x0 + side {
                if grid.source_columns[z] & (1 << x) != 0 {
                    let h = corner_indices(x, z).map(|i| grid.heights_m[i]);
                    steps.push((h[1] - h[0]).abs().max((h[2] - h[0]).abs()));
                }
            }
        }
        if steps.is_empty() {
            return 0.0;
        }
        steps.sort_by(f32::total_cmp);
        steps[((steps.len() - 1) as f64 * BROAD_STEP_QUANTILE).round() as usize]
    }

    #[test]
    fn terrain_detail_broad_steps_match_owned_quad_quantile_oracle() {
        let grid = varied_grid();
        let summary = TerrainDetailSummary::from_grid(&grid).unwrap();
        let source = CellId {
            face: Face(2),
            level: 9,
            i: 311,
            j: 17,
        };
        for depth in 0..=DEPTH {
            let width = 1usize << depth;
            let side = SIDE / width;
            for v in 0..width {
                for x in 0..width {
                    let target = CellId {
                        face: source.face,
                        level: source.level + depth,
                        i: (source.i << depth) + x as u32,
                        j: (source.j << depth) + v as u32,
                    };
                    assert_eq!(
                        summary.broad_step(source, target),
                        Some(brute_broad_step(
                            &grid,
                            x * side,
                            (width - v - 1) * side,
                            side
                        )),
                        "{target:?}"
                    );
                }
            }
        }
        assert!(
            summary
                .broad_step(source, CellId { i: 0, ..source })
                .is_none()
        );
    }

    #[test]
    fn terrain_detail_broad_step_ignores_isolated_walls_but_keeps_broad_slopes() {
        let owned = [u32::MAX; SIDE];
        // A 20 m quay wall along one lattice column is the maximum step, yet
        // covers only 1/32 of the quads.
        let wall = TerrainDetailGrid {
            heights_m: Box::new(std::array::from_fn(
                |i| if i % 33 > 16 { 20.0 } else { 0.0 },
            )),
            source_columns: owned,
        };
        let root = CellId::root(Face(0));
        let summary = TerrainDetailSummary::from_grid(&wall).unwrap();
        assert_eq!(summary.query(root, root).unwrap().step_m, 20.0);
        assert_eq!(summary.broad_step(root, root), Some(0.0));
        // Every quad of a uniform slope rises by the same step.
        let slope = TerrainDetailGrid {
            heights_m: Box::new(std::array::from_fn(|i| (i / 33) as f32 * 3.0)),
            source_columns: owned,
        };
        let summary = TerrainDetailSummary::from_grid(&slope).unwrap();
        assert_eq!(summary.broad_step(root, root), Some(3.0));
        // Known-empty ownership has no measured steepness.
        let empty = TerrainDetailGrid {
            heights_m: Box::new([0.0; CORNERS]),
            source_columns: [0; SIDE],
        };
        let summary = TerrainDetailSummary::from_grid(&empty).unwrap();
        assert_eq!(summary.broad_step(root, root), Some(0.0));
    }

    #[test]
    fn terrain_detail_descendants_use_exact_source_uv_and_containing_quad() {
        let grid = varied_grid();
        let summary = TerrainDetailSummary::from_grid(&grid).unwrap();
        for face in 0..6 {
            for (level, i, j) in [
                (0, 0, 0),
                (3, 7, 0),
                (11, 431, 1790),
                (22, 4_194_303, 4_194_303),
            ] {
                let source = CellId {
                    face: Face(face),
                    level,
                    i,
                    j,
                };
                let frame = ChunkFrame::of(ChunkAddr {
                    cell: source,
                    k: -2,
                });
                for depth in [1, 3, 5, 8] {
                    let width = 1u32 << depth;
                    for (di, dj) in [
                        (0, 0),
                        (width - 1, 0),
                        (0, width - 1),
                        (width / 2, width / 2),
                        (width - 1, width - 1),
                    ] {
                        let target = CellId {
                            face: source.face,
                            level: level + depth,
                            i: (i << depth) + di,
                            j: (j << depth) + dj,
                        };
                        let child = ChunkFrame::of(ChunkAddr { cell: target, k: 3 });
                        for (x, z) in [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (1.0, 1.0)] {
                            let sx = (f64::from(di) + x) / f64::from(width);
                            let sz = (f64::from(width - 1 - dj) + z) / f64::from(width);
                            assert_eq!(frame.column_dir(sx, sz), child.column_dir(x, z));
                        }
                        let measured_depth = depth.min(DEPTH);
                        let mx = (di >> (depth - measured_depth)) as usize;
                        let mz = ((width - 1 - dj) >> (depth - measured_depth)) as usize;
                        let side = SIDE >> measured_depth;
                        assert_eq!(
                            summary.query(source, target),
                            Some(brute_range(&grid, mx * side, mz * side, side))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn terrain_detail_empty_invalid_and_finite_extrema_remain_distinct() {
        let mut source = TerrainSurfaceData {
            heights_m: Box::new([f32::NAN; CORNERS]),
            shading: None,
            source_columns: [0; SIDE],
        };
        let empty = TerrainDetailGrid::from_source(&source).unwrap();
        assert!(empty.heights_m.iter().all(|h| h.to_bits() == 0));
        let root = CellId::root(Face(0));
        assert_eq!(
            TerrainDetailSummary::from_grid(&empty)
                .unwrap()
                .query(root, root),
            Some(TerrainDetailRange::EMPTY)
        );
        source.source_columns[31] = 1 << 31;
        assert!(TerrainDetailGrid::from_source(&source).is_none());
        for (index, value) in
            corner_indices(31, 31)
                .into_iter()
                .zip([-f32::MAX, f32::MAX, -0.0, 9000.0])
        {
            source.heights_m[index] = value;
        }
        let valid = TerrainDetailGrid::from_source(&source).unwrap();
        assert_eq!(
            valid.heights_m[corner_indices(31, 31)[2]].to_bits(),
            (-0.0f32).to_bits()
        );
        let summary = TerrainDetailSummary::from_grid(&valid).unwrap();
        assert_eq!(
            summary.query(root, root),
            Some(TerrainDetailRange {
                min_m: -f32::MAX,
                max_m: f32::MAX,
                step_m: f32::MAX
            })
        );
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut invalid = valid.clone();
            invalid.heights_m[0] = bad;
            assert!(invalid.validate().is_err());
            assert!(TerrainDetailSummary::from_grid(&invalid).is_none());
        }
        let child = CellId { level: 1, ..root };
        for bad in [
            CellId {
                face: Face(6),
                ..root
            },
            CellId { level: 31, ..root },
            CellId { i: 1, ..root },
            CellId { j: 1, ..root },
            CellId {
                face: Face(1),
                ..child
            },
        ] {
            assert!(summary.query(root, bad).is_none());
            assert!(summary.query(bad, root).is_none());
        }
        assert!(summary.query(child, root).is_none());
        assert!(summary.query(child, CellId { i: 1, ..child }).is_none());
    }

    #[test]
    fn terrain_detail_fingerprints_cover_samples_ownership_and_summary() {
        let grid = varied_grid();
        let summary = TerrainDetailSummary::from_grid(&grid).unwrap();
        assert_eq!(summary, TerrainDetailSummary::from_grid(&grid).unwrap());
        assert_eq!(summary.fingerprint(), summary.clone().fingerprint());
        let mut changed = grid.clone();
        changed.heights_m[0] = -2000.0;
        assert_ne!(grid.fingerprint(), changed.fingerprint());
        assert_ne!(
            summary.fingerprint(),
            TerrainDetailSummary::from_grid(&changed)
                .unwrap()
                .fingerprint()
        );
        let mut changed = grid.clone();
        changed.source_columns[0] ^= 1;
        assert_ne!(grid.fingerprint(), changed.fingerprint());
        assert_ne!(
            summary.fingerprint(),
            TerrainDetailSummary::from_grid(&changed)
                .unwrap()
                .fingerprint()
        );
    }
}
