//! Compact internal addressing for quadtree indexing.
//!
//! [`CellId`] remains the public geometric address. `CellKey` is an in-memory
//! indexing form only: it is not a cache, archive, network, or on-disk format.
//! Its Morton bits are correlated, so maps keyed by `CellKey` must not use an
//! identity hasher.

use crate::cell::{CellId, MAX_LEVEL};
use crate::cubesphere::Face;

const FACE_SHIFT: u32 = 61;
const LEVEL_SHIFT: u32 = 56;
const LEVEL_MASK: u64 = 0x1f << LEVEL_SHIFT;
const FACE_MASK: u64 = 0x7 << FACE_SHIFT;
/// The app admits one landmark-biased refinement level beyond the ordinary
/// global stream ceiling. `CellId` can represent it, so the in-memory key
/// must remain lossless for it as well.
pub const MAX_KEY_LEVEL: u8 = MAX_LEVEL + 1;
const PATH_BITS: u32 = 2 * MAX_KEY_LEVEL as u32;
const PATH_MASK: u64 = (1 << PATH_BITS) - 1;

/// Compact, lossless in-memory key for a canonical [`CellId`].
///
/// The low 40 bits hold the Morton path, with `i` in each even bit and `j` in
/// each odd bit. The next 16 bits are reserved, bits 56–60 hold the level, and
/// bits 61–63 hold the cube face. Keeping the face at the top also gives each
/// face a disjoint spatial range domain.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct CellKey(u64);

/// Half-open Morton interval at [`MAX_KEY_LEVEL`] resolution.
///
/// Values from different cube faces have a reserved gap between them. This
/// means ordinary overlap/adjacency checks cannot accidentally combine ranges
/// across a face boundary.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct CellRange {
    start: u64,
    end: u64,
}

impl CellKey {
    /// Encode a canonical geometric cell.
    ///
    /// Panics for a face outside `0..Face::COUNT`, a level above
    /// [`MAX_KEY_LEVEL`], or coordinates outside that level's face grid. Invalid
    /// values are rejected rather than silently aliasing a valid key.
    #[inline]
    pub fn new(cell: CellId) -> Self {
        assert!(
            cell.face.0 < Face::COUNT,
            "cell face {} is outside 0..{}",
            cell.face.0,
            Face::COUNT
        );
        assert!(
            cell.level <= MAX_KEY_LEVEL,
            "cell level {} exceeds MAX_KEY_LEVEL {MAX_KEY_LEVEL}",
            cell.level
        );
        let edge = 1u32 << cell.level;
        assert!(
            cell.i < edge && cell.j < edge,
            "cell coordinates ({}, {}) are outside level {}",
            cell.i,
            cell.j,
            cell.level
        );

        let path = interleave_axis(cell.i) | (interleave_axis(cell.j) << 1);
        CellKey(
            (u64::from(cell.face.0) << FACE_SHIFT) | (u64::from(cell.level) << LEVEL_SHIFT) | path,
        )
    }

    /// The packed value, for well-mixed hashing and compact storage.
    ///
    /// This is intentionally one-way: packed values are not a persistence
    /// format and cannot be used to construct unchecked keys.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn face(self) -> Face {
        Face((self.0 >> FACE_SHIFT) as u8)
    }

    #[inline]
    pub const fn level(self) -> u8 {
        ((self.0 & LEVEL_MASK) >> LEVEL_SHIFT) as u8
    }

    #[inline]
    const fn path(self) -> u64 {
        self.0 & PATH_MASK
    }

    /// Decode the original geometric cell.
    #[inline]
    pub fn cell(self) -> CellId {
        CellId {
            face: self.face(),
            level: self.level(),
            i: compact_axis(self.path()),
            j: compact_axis(self.path() >> 1),
        }
    }

    /// Parent key, or `None` for a face root.
    #[inline]
    pub const fn parent(self) -> Option<Self> {
        let level = self.level();
        if level == 0 {
            return None;
        }
        Some(CellKey(
            (self.0 & FACE_MASK) | ((level as u64 - 1) << LEVEL_SHIFT) | (self.path() >> 2),
        ))
    }

    /// Child in Morton quadrant order:
    /// `(i, j) = (0,0), (1,0), (0,1), (1,1)`.
    #[inline]
    pub fn child(self, quadrant: u8) -> Option<Self> {
        assert!(quadrant < 4, "quadrant {quadrant} is outside 0..4");
        let level = self.level();
        if level == MAX_KEY_LEVEL {
            return None;
        }
        Some(CellKey(
            (self.0 & FACE_MASK)
                | (u64::from(level + 1) << LEVEL_SHIFT)
                | (self.path() << 2)
                | u64::from(quadrant),
        ))
    }

    /// All four children in the same order as [`CellId::children`].
    #[inline]
    pub fn children(self) -> Option<[Self; 4]> {
        if self.level() == MAX_KEY_LEVEL {
            return None;
        }
        Some([
            self.child(0).unwrap(),
            self.child(1).unwrap(),
            self.child(2).unwrap(),
            self.child(3).unwrap(),
        ])
    }

    /// This cell's quadrant within its parent, or `None` for a face root.
    #[inline]
    pub const fn quadrant(self) -> Option<u8> {
        if self.level() == 0 {
            None
        } else {
            Some((self.path() & 0x3) as u8)
        }
    }

    /// Spatial interval occupied by this cell at [`MAX_KEY_LEVEL`] resolution.
    #[inline]
    pub const fn range(self) -> CellRange {
        let suffix_bits = 2 * (MAX_KEY_LEVEL - self.level()) as u32;
        let start = (self.0 & FACE_MASK) | (self.path() << suffix_bits);
        CellRange {
            start,
            end: start + (1 << suffix_bits),
        }
    }
}

impl From<CellId> for CellKey {
    #[inline]
    fn from(cell: CellId) -> Self {
        Self::new(cell)
    }
}

impl From<CellKey> for CellId {
    #[inline]
    fn from(key: CellKey) -> Self {
        key.cell()
    }
}

impl CellRange {
    #[inline]
    pub const fn face(self) -> Face {
        Face((self.start >> FACE_SHIFT) as u8)
    }

    #[inline]
    pub const fn start(self) -> u64 {
        self.start
    }

    #[inline]
    pub const fn end(self) -> u64 {
        self.end
    }

    #[inline]
    pub const fn len(self) -> u64 {
        self.end - self.start
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Whether `other` is fully contained in this half-open interval.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    #[inline]
    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Whether the intervals overlap or are directly adjacent.
    ///
    /// Face domains are separated by construction, so ranges on different
    /// faces always return `false`.
    #[inline]
    pub const fn can_merge(self, other: Self) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    /// Union overlapping or adjacent ranges.
    #[inline]
    pub const fn merged(self, other: Self) -> Option<Self> {
        if !self.can_merge(other) {
            return None;
        }
        Some(CellRange {
            start: if self.start < other.start {
                self.start
            } else {
                other.start
            },
            end: if self.end > other.end {
                self.end
            } else {
                other.end
            },
        })
    }
}

#[inline]
const fn interleave_axis(value: u32) -> u64 {
    let mut x = value as u64;
    x = (x | (x << 16)) & 0x0000_ffff_0000_ffff;
    x = (x | (x << 8)) & 0x00ff_00ff_00ff_00ff;
    x = (x | (x << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    x = (x | (x << 2)) & 0x3333_3333_3333_3333;
    (x | (x << 1)) & 0x5555_5555_5555_5555
}

#[inline]
const fn compact_axis(value: u64) -> u32 {
    let mut x = value & 0x5555_5555_5555_5555;
    x = (x | (x >> 1)) & 0x3333_3333_3333_3333;
    x = (x | (x >> 2)) & 0x0f0f_0f0f_0f0f_0f0f;
    x = (x | (x >> 4)) & 0x00ff_00ff_00ff_00ff;
    x = (x | (x >> 8)) & 0x0000_ffff_0000_ffff;
    ((x | (x >> 16)) & 0x0000_0000_ffff_ffff) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn cell(face: u8, level: u8, i: u32, j: u32) -> CellId {
        CellId {
            face: Face(face),
            level,
            i,
            j,
        }
    }

    #[test]
    fn cell_key_is_one_u64() {
        assert_eq!(std::mem::size_of::<CellKey>(), std::mem::size_of::<u64>());
    }

    #[test]
    fn round_trips_every_cell_through_level_eight() {
        for face in 0..Face::COUNT {
            for level in 0..=8 {
                let edge = 1u32 << level;
                for i in 0..edge {
                    for j in 0..edge {
                        let original = cell(face, level, i, j);
                        let key = CellKey::from(original);
                        assert_eq!(CellId::from(key), original);
                        assert_eq!(key.face(), original.face);
                        assert_eq!(key.level(), original.level);
                    }
                }
            }
        }
    }

    #[test]
    fn randomized_maximum_level_round_trips() {
        let edge_mask = (1u32 << MAX_KEY_LEVEL) - 1;
        let mut state = 0x6a09_e667_f3bc_c909u64;
        for _ in 0..100_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let face = (state % u64::from(Face::COUNT)) as u8;
            let i = (state as u32) & edge_mask;
            let j = ((state >> 32) as u32) & edge_mask;
            let original = cell(face, MAX_KEY_LEVEL, i, j);
            assert_eq!(CellKey::from(original).cell(), original);
        }
    }

    #[test]
    fn boundary_coordinates_round_trip() {
        for face in 0..Face::COUNT {
            for level in [0, 1, 3, 8, 12, MAX_LEVEL, MAX_KEY_LEVEL] {
                let last = (1u32 << level) - 1;
                for (i, j) in [(0, 0), (last, 0), (0, last), (last, last)] {
                    let original = cell(face, level, i, j);
                    assert_eq!(CellKey::from(original).cell(), original);
                }
            }
        }
    }

    #[test]
    fn sampled_faces_and_levels_have_unique_keys() {
        let mut keys = HashSet::new();
        for face in 0..Face::COUNT {
            for level in 0..=7 {
                let edge = 1u32 << level;
                for i in 0..edge {
                    for j in 0..edge {
                        let key = CellKey::from(cell(face, level, i, j));
                        assert!(keys.insert(key), "duplicate key for {:?}", key.cell());
                    }
                }
            }
        }
    }

    #[test]
    fn key_parent_children_and_quadrants_match_cell_id() {
        for face in 0..Face::COUNT {
            let parent_cell = cell(face, 12, 1234, 2987);
            let parent = CellKey::from(parent_cell);
            let key_children = parent.children().unwrap();
            let cell_children = parent_cell.children();
            for quadrant in 0..4 {
                let child = key_children[quadrant];
                assert_eq!(child.cell(), cell_children[quadrant]);
                assert_eq!(child.parent(), Some(parent));
                assert_eq!(child.quadrant(), Some(quadrant as u8));
            }
        }
        assert_eq!(CellKey::from(cell(0, 0, 0, 0)).parent(), None);
        assert_eq!(CellKey::from(cell(0, 0, 0, 0)).quadrant(), None);
        assert!(
            CellKey::from(cell(0, MAX_KEY_LEVEL, 0, 0))
                .children()
                .is_none()
        );
    }

    #[test]
    fn parent_range_contains_every_child_range() {
        for face in 0..Face::COUNT {
            for level in 0..MAX_KEY_LEVEL {
                let edge = 1u32 << level;
                let samples = [
                    (0, 0),
                    (edge - 1, 0),
                    (0, edge - 1),
                    (edge - 1, edge - 1),
                    (edge / 2, edge / 3),
                ];
                for (i, j) in samples {
                    let parent = CellKey::from(cell(face, level, i, j));
                    for child in parent.children().unwrap() {
                        assert!(parent.range().contains(child.range()));
                    }
                }
            }
        }
    }

    #[test]
    fn randomized_descendant_ranges_stay_inside_every_ancestor() {
        let edge_mask = (1u32 << MAX_KEY_LEVEL) - 1;
        let mut state = 0xbb67_ae85_84ca_a73bu64;
        for _ in 0..10_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let leaf = CellKey::from(cell(
                (state % u64::from(Face::COUNT)) as u8,
                MAX_KEY_LEVEL,
                (state as u32) & edge_mask,
                ((state >> 32) as u32) & edge_mask,
            ));
            let leaf_range = leaf.range();
            let mut ancestor = Some(leaf);
            while let Some(key) = ancestor {
                assert!(key.range().contains(leaf_range));
                ancestor = key.parent();
            }
        }
    }

    #[test]
    fn sibling_ranges_are_disjoint_and_partition_the_parent() {
        for face in 0..Face::COUNT {
            for level in 0..MAX_KEY_LEVEL {
                let edge = 1u32 << level;
                for (i, j) in [(0, 0), (edge - 1, edge - 1), (edge / 2, edge / 3)] {
                    let parent = CellKey::from(cell(face, level, i, j));
                    let children = parent.children().unwrap();
                    let ranges = children.map(CellKey::range);

                    assert_eq!(ranges[0].start(), parent.range().start());
                    assert_eq!(ranges[3].end(), parent.range().end());
                    assert_eq!(
                        ranges.iter().map(|range| range.len()).sum::<u64>(),
                        parent.range().len()
                    );
                    for pair in ranges.windows(2) {
                        assert!(!pair[0].overlaps(pair[1]));
                        assert_eq!(pair[0].end(), pair[1].start());
                    }
                }
            }
        }
    }

    #[test]
    fn six_face_ranges_never_overlap_or_merge() {
        let roots: Vec<_> = (0..Face::COUNT)
            .map(|face| CellKey::from(cell(face, 0, 0, 0)).range())
            .collect();
        for (face, range) in roots.iter().copied().enumerate() {
            assert_eq!(range.face(), Face(face as u8));
        }
        for a in 0..roots.len() {
            for b in a + 1..roots.len() {
                assert!(!roots[a].overlaps(roots[b]));
                assert!(!roots[a].can_merge(roots[b]));
                assert_eq!(roots[a].merged(roots[b]), None);
            }
        }
    }

    #[test]
    fn adjacent_ranges_merge_only_within_a_face() {
        let root = CellKey::from(cell(3, 0, 0, 0));
        let ranges = root.children().unwrap().map(CellKey::range);
        assert_eq!(
            ranges[0].merged(ranges[1]),
            Some(CellRange {
                start: ranges[0].start(),
                end: ranges[1].end(),
            })
        );
        assert!(!ranges[0].is_empty());
    }

    #[test]
    #[should_panic(expected = "exceeds MAX_KEY_LEVEL")]
    fn rejects_levels_above_maximum() {
        let _ = CellKey::from(cell(0, MAX_KEY_LEVEL + 1, 0, 0));
    }

    #[test]
    #[should_panic(expected = "outside level")]
    fn rejects_noncanonical_coordinates() {
        let _ = CellKey::from(cell(0, 4, 16, 0));
    }

    #[test]
    #[should_panic(expected = "outside 0..6")]
    fn rejects_noncanonical_faces() {
        let _ = CellKey::from(cell(Face::COUNT, 0, 0, 0));
    }
}
