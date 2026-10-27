//! Coverage-safe LOD transition state.
//!
//! Screen-door fades cover exactly when an outgoing cell starts opaque and
//! its replacement fades in at the same rate. Camera motion and streaming can
//! retarget a region before that pair finishes, so phases are deliberately
//! non-interruptible:
//!
//! - incoming cells always reach alpha 1 before they may leave;
//! - outgoing cells always reach alpha 0 before they may re-enter;
//! - newly arrived target cells still start fading immediately.
//!
//! The active pair therefore remains a complete cover while a newer target is
//! layered over it. This module is independent of the app frame loop so the
//! coverage invariant can be exhaustively sampled in unit tests before it is
//! wired into rendering.
//!
//! Integration must keep every active draw resident until it leaves this
//! state, and should apply the coverage assertion only to spatial regions
//! shared by successive cuts (a horizon exit intentionally fades to sky).

use std::collections::{HashMap, HashSet};
use vearth_core::cell::CellId;
use vearth_core::cell_key::{CellKey, CellRange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionPhase {
    Incoming,
    Stable,
    Outgoing,
}

/// Renderer-facing transition value. Positive fade is incoming/opaque;
/// negative fade is outgoing, matching `gfx::DrawCell` and `shader.wgsl`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionDraw {
    pub cell: CellId,
    pub fade: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransitionStats {
    pub started: u64,
    pub completed: u64,
    pub deferred_reversals: u64,
    pub active: usize,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    alpha: f32,
    phase: TransitionPhase,
    target_epoch: u64,
}

/// Sorted, merged Morton coverage for one previously drawn cut.
///
/// Construction is `O(n log n)` once per transition retarget. Queries are a
/// binary search followed by one interval-containment check, replacing the
/// recursive prefix sets previously rebuilt for every target cell.
#[derive(Clone, Debug, Default)]
pub struct CoverageIndex {
    ranges: Vec<CellRange>,
}

impl CoverageIndex {
    pub fn from_cells(cells: impl IntoIterator<Item = CellId>) -> Self {
        let mut index = Self::default();
        index.rebuild(cells);
        index
    }

    pub fn rebuild(&mut self, cells: impl IntoIterator<Item = CellId>) {
        self.ranges.clear();
        self.ranges
            .extend(cells.into_iter().map(|cell| CellKey::from(cell).range()));
        self.ranges.sort_unstable_by_key(|range| range.start());

        let mut merged_len = 0usize;
        for read in 0..self.ranges.len() {
            let range = self.ranges[read];
            if merged_len > 0
                && let Some(combined) = self.ranges[merged_len - 1].merged(range)
            {
                self.ranges[merged_len - 1] = combined;
            } else {
                self.ranges[merged_len] = range;
                merged_len += 1;
            }
        }
        self.ranges.truncate(merged_len);
    }

    #[inline]
    pub fn fully_covers(&self, cell: CellId) -> bool {
        let target = CellKey::from(cell).range();
        let index = self
            .ranges
            .partition_point(|range| range.start() <= target.start());
        index > 0 && self.ranges[index - 1].contains(target)
    }

    #[cfg(test)]
    fn ranges(&self) -> &[CellRange] {
        &self.ranges
    }
}

/// Stateful, coverage-safe conversion from successive drawable cuts to
/// signed renderer fades.
pub struct LodTransitions {
    duration_s: f32,
    entries: HashMap<CellKey, Entry>,
    remove_scratch: Vec<CellKey>,
    target_epoch: u64,
    // Stable obsolete cells still need an advance to begin outgoing. This
    // flag therefore includes target membership, not just active phases.
    settled: bool,
    #[cfg(test)]
    advance_scans: usize,
    started: u64,
    completed: u64,
    deferred_reversals: u64,
}

impl LodTransitions {
    pub fn new(duration_s: f32) -> Self {
        assert!(duration_s.is_finite() && duration_s > 0.0);
        LodTransitions {
            duration_s,
            entries: HashMap::new(),
            remove_scratch: Vec::new(),
            target_epoch: 1,
            settled: true,
            #[cfg(test)]
            advance_scans: 0,
            started: 0,
            completed: 0,
            deferred_reversals: 0,
        }
    }

    /// Construct an already-covered initial cut. Useful for app startup and
    /// tests; later retargets use normal dither transitions.
    pub fn with_opaque(duration_s: f32, cells: &[CellId]) -> Self {
        let mut transitions = Self::new(duration_s);
        transitions.reset_opaque(cells);
        transitions
    }

    pub fn reset_opaque(&mut self, cells: &[CellId]) {
        self.entries.clear();
        self.target_epoch = 1;
        self.settled = true;
        #[cfg(test)]
        {
            self.advance_scans = 0;
        }
        self.started = 0;
        self.completed = 0;
        self.deferred_reversals = 0;
        for &cell in cells {
            self.entries.insert(
                CellKey::from(cell),
                Entry {
                    alpha: 1.0,
                    phase: TransitionPhase::Stable,
                    target_epoch: self.target_epoch,
                },
            );
        }
    }

    /// Replace the desired drawable cut. `initial_alpha` is called only for
    /// target cells with no active transition. The app can return 0 for a
    /// covered arrival and 1 for a fresh silhouette with no backdrop, avoiding
    /// a fade against the sky.
    ///
    /// Existing phases are never reversed here. In particular, an incoming
    /// cell removed by the new target finishes incoming before it can become
    /// outgoing; this is the key nested-transition coverage invariant.
    pub fn retarget<F>(&mut self, cells: &[CellId], mut initial_alpha: F)
    where
        F: FnMut(CellId) -> f32,
    {
        // Preserve target membership across the practically unreachable u64
        // wrap so an old epoch can never become current again.
        if self.target_epoch == u64::MAX {
            for entry in self.entries.values_mut() {
                entry.target_epoch = u64::from(entry.target_epoch == u64::MAX);
            }
            self.target_epoch = 1;
        }
        let previous_epoch = self.target_epoch;
        let next_epoch = previous_epoch + 1;

        for &cell in cells {
            let key = CellKey::from(cell);
            if let Some(entry) = self.entries.get_mut(&key) {
                // Duplicate cells do not create a false membership change.
                if entry.target_epoch == next_epoch {
                    continue;
                }
                if entry.target_epoch != previous_epoch && entry.phase != TransitionPhase::Stable {
                    self.deferred_reversals += 1;
                }
                entry.target_epoch = next_epoch;
                continue;
            }

            let raw_alpha = initial_alpha(cell);
            let alpha = if raw_alpha.is_finite() {
                raw_alpha.clamp(0.0, 1.0)
            } else {
                1.0
            };
            let phase = if alpha >= 1.0 {
                TransitionPhase::Stable
            } else {
                TransitionPhase::Incoming
            };
            if phase == TransitionPhase::Incoming {
                self.started += 1;
            }
            self.entries.insert(
                key,
                Entry {
                    alpha,
                    phase,
                    target_epoch: next_epoch,
                },
            );
        }

        self.settled = true;
        for entry in self.entries.values() {
            if entry.target_epoch == previous_epoch && entry.phase != TransitionPhase::Stable {
                self.deferred_reversals += 1;
            }
            self.settled &=
                entry.phase == TransitionPhase::Stable && entry.target_epoch == next_epoch;
        }
        self.target_epoch = next_epoch;
    }

    /// Advance all active phases. Phase reversals are deferred until the
    /// current phase reaches its endpoint; target changes may still add new
    /// incoming cells immediately through `retarget`.
    pub fn advance(&mut self, dt_s: f32) {
        if self.settled || !dt_s.is_finite() || dt_s <= 0.0 {
            return;
        }
        #[cfg(test)]
        {
            self.advance_scans += 1;
        }
        let step = (dt_s / self.duration_s).min(1.0);
        self.remove_scratch.clear();
        let mut unsettled = 0usize;

        for (&cell, entry) in &mut self.entries {
            match entry.phase {
                TransitionPhase::Incoming => {
                    entry.alpha = (entry.alpha + step).min(1.0);
                    if entry.alpha >= 1.0 {
                        // If this cell is already obsolete it intentionally
                        // remains stable until the next advance. It may only
                        // begin outgoing from the fully opaque endpoint.
                        entry.phase = TransitionPhase::Stable;
                        self.completed += 1;
                    }
                }
                TransitionPhase::Stable => {
                    if entry.target_epoch != self.target_epoch {
                        debug_assert_eq!(entry.alpha, 1.0);
                        entry.phase = TransitionPhase::Outgoing;
                        entry.alpha = (1.0 - step).max(0.0);
                        if entry.alpha <= 0.0 {
                            self.remove_scratch.push(cell);
                        }
                    }
                }
                TransitionPhase::Outgoing => {
                    // Even if the cell becomes wanted again, completing this
                    // outgoing phase preserves the old complementary pair.
                    // It re-enters below after reaching zero.
                    entry.alpha = (entry.alpha - step).max(0.0);
                    if entry.alpha <= 0.0 {
                        self.remove_scratch.push(cell);
                    }
                }
            }
            unsettled += usize::from(
                entry.phase != TransitionPhase::Stable || entry.target_epoch != self.target_epoch,
            );
        }

        for cell in self.remove_scratch.drain(..) {
            unsettled -= 1;
            let was_targeted = self
                .entries
                .remove(&cell)
                .is_some_and(|entry| entry.target_epoch == self.target_epoch);
            self.completed += 1;
            if was_targeted {
                unsettled += 1;
                self.started += 1;
                self.entries.insert(
                    cell,
                    Entry {
                        alpha: 0.0,
                        phase: TransitionPhase::Incoming,
                        target_epoch: self.target_epoch,
                    },
                );
            }
        }
        self.settled = unsettled == 0;
    }

    pub fn phase(&self, cell: CellId) -> Option<TransitionPhase> {
        self.entries
            .get(&CellKey::from(cell))
            .map(|entry| entry.phase)
    }

    pub fn alpha(&self, cell: CellId) -> Option<f32> {
        self.entries
            .get(&CellKey::from(cell))
            .map(|entry| entry.alpha)
    }

    pub fn stats(&self) -> TransitionStats {
        TransitionStats {
            started: self.started,
            completed: self.completed,
            deferred_reversals: self.deferred_reversals,
            active: if self.settled {
                0
            } else {
                self.entries
                    .values()
                    .filter(|entry| entry.phase != TransitionPhase::Stable)
                    .count()
            },
        }
    }

    /// Active renderer draws. Alpha-zero incoming entries are state-only and
    /// omitted until they affect a pixel. The renderer applies its own stable
    /// front-to-back + CellId order, so sorting this full set here would be
    /// redundant frame-critical work.
    pub fn iter_draws(&self) -> impl Iterator<Item = TransitionDraw> + '_ {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.alpha > 0.0)
            .map(|(&key, entry)| TransitionDraw {
                cell: key.cell(),
                fade: if entry.phase == TransitionPhase::Outgoing {
                    -entry.alpha
                } else {
                    entry.alpha
                },
            })
    }

    pub fn draws(&self) -> Vec<TransitionDraw> {
        self.iter_draws().collect()
    }

    /// Count active fragments covering a fine sample cell for a dither hash
    /// in [0, 1). `sample` must be at least as fine as the active draw cells;
    /// this is intended for invariants, diagnostics, and deterministic tests.
    pub fn sampled_coverage(&self, sample: CellId, hash: f32) -> usize {
        debug_assert!((0.0..1.0).contains(&hash));
        self.entries
            .iter()
            .filter(|(key, entry)| {
                entry.alpha > 0.0
                    && contains_cell(key.cell(), sample)
                    && match entry.phase {
                        TransitionPhase::Outgoing => hash >= 1.0 - entry.alpha,
                        TransitionPhase::Incoming | TransitionPhase::Stable => hash < entry.alpha,
                    }
            })
            .count()
    }

    /// Count target quadtree regions whose active dither intervals do not
    /// cover the full `[0, 1)` hash domain. This is a diagnostic invariant,
    /// not part of presentation: callers can sample it periodically while a
    /// transition is active without changing transition state.
    ///
    /// At one spatial point, incoming entries cover `[0, alpha)` and outgoing
    /// entries cover `[1 - alpha, 1)`. Their union is complete exactly when
    /// the largest incoming endpoint reaches the earliest outgoing start.
    /// Recursing only where an active descendant exists partitions the target
    /// into regions with a constant set of covering entries.
    pub fn uncovered_target_regions(&self) -> usize {
        let mut descendant_prefixes = HashSet::new();
        for &key in self.entries.keys() {
            let mut current = key.cell();
            while let Some(parent) = current.parent() {
                descendant_prefixes.insert(CellKey::from(parent));
                current = parent;
            }
        }

        fn include(entry: Entry, incoming_end: &mut f32, outgoing_start: &mut f32) {
            match entry.phase {
                TransitionPhase::Outgoing => {
                    *outgoing_start = outgoing_start.min(1.0 - entry.alpha);
                }
                TransitionPhase::Incoming | TransitionPhase::Stable => {
                    *incoming_end = incoming_end.max(entry.alpha);
                }
            }
        }

        fn count(
            transitions: &LodTransitions,
            prefixes: &HashSet<CellKey>,
            node: CellId,
            mut incoming_end: f32,
            mut outgoing_start: f32,
        ) -> usize {
            if let Some(&entry) = transitions.entries.get(&CellKey::from(node)) {
                include(entry, &mut incoming_end, &mut outgoing_start);
            }
            if incoming_end >= outgoing_start {
                return 0;
            }
            if !prefixes.contains(&CellKey::from(node)) {
                return 1;
            }
            node.children()
                .into_iter()
                .map(|child| count(transitions, prefixes, child, incoming_end, outgoing_start))
                .sum()
        }

        self.entries
            .iter()
            .filter(|(_, entry)| entry.target_epoch == self.target_epoch)
            .map(|(&target, _)| {
                let target_cell = target.cell();
                let mut incoming_end = 0.0f32;
                let mut outgoing_start = 1.0f32;
                let mut ancestors = Vec::new();
                let mut current = target;
                while let Some(parent) = current.parent() {
                    ancestors.push(parent);
                    current = parent;
                }
                for ancestor in ancestors.into_iter().rev() {
                    if let Some(&entry) = self.entries.get(&ancestor) {
                        include(entry, &mut incoming_end, &mut outgoing_start);
                    }
                }
                count(
                    self,
                    &descendant_prefixes,
                    target_cell,
                    incoming_end,
                    outgoing_start,
                )
            })
            .sum()
    }
}

fn contains_cell(cover: CellId, sample: CellId) -> bool {
    if cover.face != sample.face || cover.level > sample.level {
        return false;
    }
    let shift = sample.level - cover.level;
    (sample.i >> shift) == cover.i && (sample.j >> shift) == cover.j
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::cell::MAX_LEVEL;
    use vearth_core::cubesphere::Face;

    fn cell(face: u8, level: u8, i: u32, j: u32) -> CellId {
        CellId {
            face: Face(face),
            level,
            i,
            j,
        }
    }

    fn next_random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    fn samples(cell: CellId) -> Vec<CellId> {
        cell.children()
            .into_iter()
            .flat_map(|child| child.children())
            .collect()
    }

    fn assert_full_coverage(transitions: &LodTransitions, samples: &[CellId]) {
        assert_eq!(
            transitions.uncovered_target_regions(),
            0,
            "runtime coverage diagnostic found a hole"
        );
        for &sample in samples {
            for i in 0..256 {
                let hash = (i as f32 + 0.5) / 256.0;
                assert!(
                    transitions.sampled_coverage(sample, hash) >= 1,
                    "coverage hole at {sample:?}, hash {hash:.4}; draws {:?}",
                    transitions.draws(),
                );
            }
        }
    }

    #[test]
    fn coverage_index_handles_ancestor_descendant_mixed_and_face_cases() {
        let target = cell(2, 9, 40, 80);
        assert!(!CoverageIndex::default().fully_covers(target));
        assert!(
            CoverageIndex::from_cells([target.parent().unwrap()]).fully_covers(target),
            "ancestor must cover"
        );

        let children = target.children();
        assert!(
            !CoverageIndex::from_cells(children[..3].iter().copied()).fully_covers(target),
            "partial child coverage must not cover"
        );
        let descendants = children[0]
            .children()
            .into_iter()
            .chain(children[1..].iter().copied());
        assert!(
            CoverageIndex::from_cells(descendants).fully_covers(target),
            "mixed descendant levels must cover"
        );
        assert!(
            !CoverageIndex::from_cells([cell(3, 8, 20, 40)]).fully_covers(target),
            "another face must not cover"
        );
    }

    #[test]
    fn coverage_index_merges_only_same_face_intervals() {
        let roots = (0..Face::COUNT).map(|face| cell(face, 0, 0, 0));
        let index = CoverageIndex::from_cells(roots);
        assert_eq!(index.ranges().len(), Face::COUNT as usize);

        let parent = cell(0, 8, 10, 20);
        let index = CoverageIndex::from_cells(parent.children());
        assert_eq!(index.ranges(), &[CellKey::from(parent).range()]);
    }

    #[test]
    fn randomized_coverage_index_matches_recursive_oracle() {
        let mut state = 0x3c6e_f372_fe94_f82bu64;
        for iteration in 0..20_000 {
            let level = 3 + (next_random(&mut state) % 13) as u8;
            let edge = 1u32 << level;
            let target = cell(
                (next_random(&mut state) % u64::from(Face::COUNT)) as u8,
                level,
                next_random(&mut state) as u32 & (edge - 1),
                next_random(&mut state) as u32 & (edge - 1),
            );
            let mut drawn = HashSet::new();
            let count = (next_random(&mut state) % 48) as usize;
            for _ in 0..count {
                let candidate = match next_random(&mut state) % 4 {
                    0 => {
                        let mut candidate = target;
                        let steps = (next_random(&mut state) % u64::from(target.level + 1)) as u8;
                        for _ in 0..steps {
                            candidate = candidate.parent().unwrap();
                        }
                        candidate
                    }
                    1 => {
                        let mut candidate = target;
                        let max_depth = (MAX_LEVEL - target.level).min(4);
                        let depth = if max_depth == 0 {
                            0
                        } else {
                            (next_random(&mut state) % u64::from(max_depth + 1)) as u8
                        };
                        for _ in 0..depth {
                            let quadrant = (next_random(&mut state) & 3) as usize;
                            candidate = candidate.children()[quadrant];
                        }
                        candidate
                    }
                    2 => {
                        let candidate_level = 3 + (next_random(&mut state) % 17) as u8;
                        let candidate_edge = 1u32 << candidate_level;
                        cell(
                            target.face.0,
                            candidate_level,
                            next_random(&mut state) as u32 & (candidate_edge - 1),
                            next_random(&mut state) as u32 & (candidate_edge - 1),
                        )
                    }
                    _ => {
                        let candidate_level = 3 + (next_random(&mut state) % 17) as u8;
                        let candidate_edge = 1u32 << candidate_level;
                        cell(
                            (target.face.0 + 1) % Face::COUNT,
                            candidate_level,
                            next_random(&mut state) as u32 & (candidate_edge - 1),
                            next_random(&mut state) as u32 & (candidate_edge - 1),
                        )
                    }
                };
                drawn.insert(candidate);
            }

            // Force both covered and partially covered cases regularly rather
            // than relying on sparse random cells to hit them.
            if iteration % 5 == 0 {
                drawn.extend(target.children());
            } else if iteration % 5 == 1 {
                drawn.extend(target.children()[..3].iter().copied());
            } else if iteration % 5 == 2 {
                drawn.insert(target.parent().unwrap());
            }

            let index = CoverageIndex::from_cells(drawn.iter().copied());
            assert_eq!(
                index.fully_covers(target),
                crate::streaming::fully_covered_by_drawn(target, &drawn),
                "coverage mismatch at iteration {iteration}, target {target:?}, drawn {drawn:?}"
            );
        }
    }

    #[derive(Clone, Copy)]
    struct ReferenceEntry {
        alpha: f32,
        phase: TransitionPhase,
    }

    struct ReferenceTransitions {
        duration_s: f32,
        target: HashSet<CellId>,
        entries: HashMap<CellId, ReferenceEntry>,
        started: u64,
        completed: u64,
        deferred_reversals: u64,
    }

    impl ReferenceTransitions {
        fn new(duration_s: f32) -> Self {
            Self {
                duration_s,
                target: HashSet::new(),
                entries: HashMap::new(),
                started: 0,
                completed: 0,
                deferred_reversals: 0,
            }
        }

        fn retarget(&mut self, cells: &[CellId], mut initial_alpha: impl FnMut(CellId) -> f32) {
            let next_target: HashSet<_> = cells.iter().copied().collect();
            for (&cell, entry) in &self.entries {
                if self.target.contains(&cell) != next_target.contains(&cell)
                    && entry.phase != TransitionPhase::Stable
                {
                    self.deferred_reversals += 1;
                }
            }
            self.target = next_target;
            for &cell in cells {
                if self.entries.contains_key(&cell) {
                    continue;
                }
                let raw_alpha = initial_alpha(cell);
                let alpha = if raw_alpha.is_finite() {
                    raw_alpha.clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let phase = if alpha >= 1.0 {
                    TransitionPhase::Stable
                } else {
                    TransitionPhase::Incoming
                };
                if phase == TransitionPhase::Incoming {
                    self.started += 1;
                }
                self.entries.insert(cell, ReferenceEntry { alpha, phase });
            }
        }

        fn advance(&mut self, dt_s: f32) {
            if !dt_s.is_finite() || dt_s <= 0.0 {
                return;
            }
            let step = (dt_s / self.duration_s).min(1.0);
            let mut remove = Vec::new();
            for (&cell, entry) in &mut self.entries {
                match entry.phase {
                    TransitionPhase::Incoming => {
                        entry.alpha = (entry.alpha + step).min(1.0);
                        if entry.alpha >= 1.0 {
                            entry.phase = TransitionPhase::Stable;
                            self.completed += 1;
                        }
                    }
                    TransitionPhase::Stable => {
                        if !self.target.contains(&cell) {
                            entry.phase = TransitionPhase::Outgoing;
                            entry.alpha = (1.0 - step).max(0.0);
                            if entry.alpha <= 0.0 {
                                remove.push(cell);
                            }
                        }
                    }
                    TransitionPhase::Outgoing => {
                        entry.alpha = (entry.alpha - step).max(0.0);
                        if entry.alpha <= 0.0 {
                            remove.push(cell);
                        }
                    }
                }
            }
            for cell in remove {
                self.entries.remove(&cell);
                self.completed += 1;
                if self.target.contains(&cell) {
                    self.started += 1;
                    self.entries.insert(
                        cell,
                        ReferenceEntry {
                            alpha: 0.0,
                            phase: TransitionPhase::Incoming,
                        },
                    );
                }
            }
        }
    }

    fn assert_epoch_matches_reference(actual: &LodTransitions, reference: &ReferenceTransitions) {
        let actual_target: HashSet<_> = actual
            .entries
            .iter()
            .filter(|(_, entry)| entry.target_epoch == actual.target_epoch)
            .map(|(&key, _)| key.cell())
            .collect();
        assert_eq!(actual_target, reference.target);
        assert_eq!(actual.entries.len(), reference.entries.len());
        for (&cell, expected) in &reference.entries {
            let actual = actual.entries.get(&CellKey::from(cell)).unwrap();
            assert_eq!(actual.phase, expected.phase, "phase mismatch for {cell:?}");
            assert_eq!(actual.alpha, expected.alpha, "alpha mismatch for {cell:?}");
        }
        assert_eq!(actual.started, reference.started);
        assert_eq!(actual.completed, reference.completed);
        assert_eq!(actual.deferred_reversals, reference.deferred_reversals);
        assert_eq!(
            actual.stats().active,
            reference
                .entries
                .values()
                .filter(|entry| entry.phase != TransitionPhase::Stable)
                .count()
        );
        assert_eq!(
            actual.settled,
            reference.entries.iter().all(|(cell, entry)| {
                entry.phase == TransitionPhase::Stable && reference.target.contains(cell)
            })
        );
    }

    #[test]
    fn unchanged_cut_retarget_reuse_matches_every_frame_reference() {
        let parent = cell(1, 9, 100, 200);
        let children = parent.children();
        let mut nested = children[0].children().to_vec();
        nested.extend_from_slice(&children[1..]);
        let cuts = [vec![parent], children.to_vec(), nested, Vec::new()];
        let mut actual = LodTransitions::new(0.3);
        let mut reference = ReferenceTransitions::new(0.3);
        let mut previous = None;
        let mut state = 0x9b05_688c_2b3e_6c1fu64;
        let mut reused_while_active = 0;
        let mut reused_while_settled = 0;
        for segment in 0..160 {
            let index = next_random(&mut state) as usize % cuts.len();
            let cut = &cuts[index];
            let frames = if segment % 3 == 0 { 1 } else { 32 };
            for frame in 0..frames {
                if previous != Some(index) {
                    let coverage = CoverageIndex::from_cells(actual.iter_draws().map(|d| d.cell));
                    actual.retarget(cut, |cell| {
                        if coverage.fully_covers(cell) {
                            0.0
                        } else {
                            1.0
                        }
                    });
                    previous = Some(index);
                } else if actual.settled {
                    reused_while_settled += 1;
                } else {
                    reused_while_active += 1;
                }
                let coverage = CoverageIndex::from_cells(
                    reference
                        .entries
                        .iter()
                        .filter(|(_, entry)| entry.alpha > 0.0)
                        .map(|(&cell, _)| cell),
                );
                reference.retarget(cut, |cell| {
                    if coverage.fully_covers(cell) {
                        0.0
                    } else {
                        1.0
                    }
                });
                assert_epoch_matches_reference(&actual, &reference);
                let dt = if frame % 13 == 0 { f32::NAN } else { 0.05 };
                actual.advance(dt);
                reference.advance(dt);
                assert_epoch_matches_reference(&actual, &reference);
            }
        }
        assert!(reused_while_active > 100);
        assert!(reused_while_settled > 100);
    }

    #[test]
    fn settled_fast_path_preserves_obsolete_opaque_endpoint_and_resets() {
        let target = cell(1, 9, 100, 200);
        let mut transitions = LodTransitions::new(1.0);
        transitions.retarget(&[target], |_| 0.0);
        transitions.advance(0.5);
        transitions.retarget(&[], |_| unreachable!());
        transitions.advance(0.5);
        assert_eq!(transitions.stats().active, 0);
        assert_eq!(transitions.phase(target), Some(TransitionPhase::Stable));
        assert_eq!(transitions.alpha(target), Some(1.0));
        assert!(!transitions.settled);
        transitions.advance(0.25);
        assert_eq!(transitions.phase(target), Some(TransitionPhase::Outgoing));
        assert_eq!(transitions.alpha(target), Some(0.75));
        transitions.advance(1.0);
        assert!(transitions.settled);
        assert!(transitions.draws().is_empty());
        let scans = transitions.advance_scans;
        let stats = transitions.stats();
        for _ in 0..32 {
            transitions.advance(0.1);
        }
        assert_eq!(transitions.advance_scans, scans);
        assert_eq!(transitions.stats(), stats);

        transitions.reset_opaque(&[target]);
        transitions.advance(1.0);
        assert_eq!(transitions.advance_scans, 0);
        assert!(transitions.settled);
        assert_eq!(transitions.stats(), TransitionStats::default());
        transitions.retarget(&[], |_| unreachable!());
        assert!(!transitions.settled);
        transitions.advance(0.25);
        assert_eq!(transitions.alpha(target), Some(0.75));
        assert_eq!(transitions.stats().active, 1);
    }

    #[test]
    fn target_epochs_match_hash_set_reference_through_randomized_retargets() {
        let parent = cell(1, 9, 100, 200);
        let children = parent.children();
        let grandchildren = children[0].children();
        let cuts = [
            vec![parent],
            children.to_vec(),
            grandchildren
                .into_iter()
                .chain(children[1..].iter().copied())
                .collect(),
            vec![children[2], children[2], children[3]],
            Vec::new(),
        ];
        let initial_alpha = |cell: CellId| {
            if (cell.i ^ cell.j ^ u32::from(cell.level)) & 1 == 0 {
                0.0
            } else {
                1.0
            }
        };

        let mut actual = LodTransitions::new(0.75);
        let mut reference = ReferenceTransitions::new(0.75);
        let mut state = 0xa54f_f53a_5f1d_36f1u64;
        for _ in 0..10_000 {
            let cut = &cuts[next_random(&mut state) as usize % cuts.len()];
            actual.retarget(cut, initial_alpha);
            reference.retarget(cut, initial_alpha);
            assert_epoch_matches_reference(&actual, &reference);

            let dt = match next_random(&mut state) % 6 {
                0 => 0.0,
                1 => f32::NAN,
                2 => 0.05,
                3 => 0.2,
                4 => 0.75,
                _ => 2.0,
            };
            actual.advance(dt);
            reference.advance(dt);
            assert_epoch_matches_reference(&actual, &reference);
        }
    }

    #[test]
    fn target_epoch_wrap_preserves_current_membership() {
        let parent = cell(0, 8, 10, 20);
        let child = parent.children()[0];
        let stale = parent.children()[1];
        let mut transitions = LodTransitions::with_opaque(1.0, &[parent, child]);
        transitions.entries.insert(
            CellKey::from(stale),
            Entry {
                alpha: 1.0,
                phase: TransitionPhase::Stable,
                target_epoch: 7,
            },
        );
        transitions.target_epoch = u64::MAX;
        for cell in [parent, child] {
            transitions
                .entries
                .get_mut(&CellKey::from(cell))
                .unwrap()
                .target_epoch = u64::MAX;
        }

        transitions.retarget(&[child], |_| 0.0);
        assert_eq!(transitions.target_epoch, 2);
        assert_eq!(
            transitions
                .entries
                .get(&CellKey::from(child))
                .unwrap()
                .target_epoch,
            transitions.target_epoch
        );
        assert_ne!(
            transitions
                .entries
                .get(&CellKey::from(parent))
                .unwrap()
                .target_epoch,
            transitions.target_epoch
        );
        assert_ne!(
            transitions
                .entries
                .get(&CellKey::from(stale))
                .unwrap()
                .target_epoch,
            transitions.target_epoch
        );
    }

    /// Release-mode comparison of per-target recursive resolution against one
    /// coverage index per retarget.
    #[test]
    #[ignore = "release-mode performance benchmark"]
    fn benchmark_coverage_retarget_old_vs_index() {
        use std::hint::black_box;
        use vearth_platform::time::{Duration, Instant};

        fn unique_cells(count: usize, state: &mut u64) -> Vec<CellId> {
            let mut cells = HashSet::with_capacity(count);
            while cells.len() < count {
                let level = 8 + (next_random(state) % 5) as u8;
                let edge = 1u32 << level;
                cells.insert(cell(
                    (next_random(state) % u64::from(Face::COUNT)) as u8,
                    level,
                    next_random(state) as u32 & (edge - 1),
                    next_random(state) as u32 & (edge - 1),
                ));
            }
            cells.into_iter().collect()
        }

        fn percentile(samples: &[Duration], percentile: f64) -> f64 {
            let mut micros: Vec<_> = samples
                .iter()
                .map(|sample| sample.as_secs_f64() * 1e6)
                .collect();
            micros.sort_by(f64::total_cmp);
            let index = ((micros.len() - 1) as f64 * percentile).ceil() as usize;
            micros[index]
        }

        let mut state = 0x510e_527f_ade6_82d1u64;
        let drawn_vec = unique_cells(4_096, &mut state);
        let drawn: HashSet<_> = drawn_vec.iter().copied().collect();
        let mut targets = unique_cells(3_600, &mut state);
        // Ensure the workload includes true ancestor, complete-descendant,
        // partial-descendant, and exact-cell answers.
        targets[..400].copy_from_slice(&drawn_vec[..400]);
        for index in 400..800 {
            targets[index] = drawn_vec[index].parent().unwrap();
        }

        let mut old_samples = Vec::new();
        let mut new_samples = Vec::new();
        let mut old_checksum = 0usize;
        let mut new_checksum = 0usize;
        for _ in 0..12 {
            let started = Instant::now();
            for &target in &targets {
                old_checksum = old_checksum.wrapping_add(usize::from(
                    crate::streaming::fully_covered_by_drawn(black_box(target), black_box(&drawn)),
                ));
            }
            old_samples.push(started.elapsed());

            let started = Instant::now();
            let coverage = CoverageIndex::from_cells(drawn.iter().copied());
            for &target in &targets {
                new_checksum = new_checksum
                    .wrapping_add(usize::from(coverage.fully_covers(black_box(target))));
            }
            new_samples.push(started.elapsed());
        }
        assert_eq!(old_checksum, new_checksum);

        eprintln!(
            "coverage-retarget-us old p50={:.1} p95={:.1} p99={:.1}; index p50={:.1} p95={:.1} p99={:.1}",
            percentile(&old_samples, 0.50),
            percentile(&old_samples, 0.95),
            percentile(&old_samples, 0.99),
            percentile(&new_samples, 0.50),
            percentile(&new_samples, 0.95),
            percentile(&new_samples, 0.99),
        );
    }

    /// Manual release-mode comparison of persistent target membership.
    #[test]
    #[ignore = "release-mode performance benchmark"]
    fn benchmark_target_epoch_vs_hash_set() {
        use std::hint::black_box;
        use vearth_platform::time::{Duration, Instant};

        fn percentile(samples: &[Duration], percentile: f64) -> f64 {
            let mut micros: Vec<_> = samples
                .iter()
                .map(|sample| sample.as_secs_f64() * 1e6)
                .collect();
            micros.sort_by(f64::total_cmp);
            let index = ((micros.len() - 1) as f64 * percentile).ceil() as usize;
            micros[index]
        }

        let mut pool = Vec::with_capacity(4_096);
        for i in 0..64 {
            for j in 0..64 {
                pool.push(cell(((i + j) % 6) as u8, 11, i * 16, j * 16));
            }
        }
        let cuts: Vec<Vec<_>> = (0..4)
            .map(|offset| {
                (0..3_600)
                    .map(|index| pool[(index + offset * 173) % pool.len()])
                    .collect()
            })
            .collect();

        let mut hash_set_samples = Vec::new();
        let mut epoch_samples = Vec::new();
        for _ in 0..12 {
            let mut reference = ReferenceTransitions::new(0.3);
            reference.retarget(&cuts[0], |_| 1.0);
            let started = Instant::now();
            for step in 0..64 {
                reference.retarget(black_box(&cuts[step % cuts.len()]), |_| 0.0);
                reference.advance(1.0);
            }
            hash_set_samples.push(started.elapsed());

            let mut actual = LodTransitions::with_opaque(0.3, &cuts[0]);
            let started = Instant::now();
            for step in 0..64 {
                actual.retarget(black_box(&cuts[step % cuts.len()]), |_| 0.0);
                actual.advance(1.0);
            }
            epoch_samples.push(started.elapsed());
            assert_epoch_matches_reference(&actual, &reference);
        }

        eprintln!(
            "transition-target-us hash-set p50={:.1} p95={:.1} p99={:.1}; epoch p50={:.1} p95={:.1} p99={:.1}",
            percentile(&hash_set_samples, 0.50),
            percentile(&hash_set_samples, 0.95),
            percentile(&hash_set_samples, 0.99),
            percentile(&epoch_samples, 0.50),
            percentile(&epoch_samples, 0.95),
            percentile(&epoch_samples, 0.99),
        );
    }

    #[test]
    fn refine_pair_is_exactly_complementary() {
        let parent = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        let children = parent.children();
        let samples = samples(parent);
        let mut transitions = LodTransitions::with_opaque(1.0, &[parent]);

        transitions.retarget(&children, |_| 0.0);
        transitions.advance(0.25);

        assert_eq!(transitions.phase(parent), Some(TransitionPhase::Outgoing));
        assert_eq!(transitions.alpha(parent), Some(0.75));
        for child in children {
            assert_eq!(transitions.phase(child), Some(TransitionPhase::Incoming));
            assert_eq!(transitions.alpha(child), Some(0.25));
        }
        assert_full_coverage(&transitions, &samples);
    }

    #[test]
    fn nested_retargets_never_reverse_a_partial_incoming_cell() {
        let parent = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        let children = parent.children();
        let grandchildren = children[0].children();
        let samples = samples(parent);
        let mut transitions = LodTransitions::with_opaque(1.0, &[parent]);

        transitions.retarget(&children, |_| 0.0);
        transitions.advance(0.25);
        assert_full_coverage(&transitions, &samples);

        let mut nested_target = grandchildren.to_vec();
        nested_target.extend_from_slice(&children[1..]);
        transitions.retarget(&nested_target, |_| 0.0);
        transitions.advance(0.25);

        // The first child is no longer targeted, but starting it outgoing at
        // alpha .25 would leave 75% of its area uncovered. It must finish the
        // original incoming phase while its grandchildren begin immediately.
        assert_eq!(
            transitions.phase(children[0]),
            Some(TransitionPhase::Incoming)
        );
        assert_eq!(transitions.alpha(children[0]), Some(0.5));
        for grandchild in grandchildren {
            assert_eq!(transitions.alpha(grandchild), Some(0.25));
        }
        assert_full_coverage(&transitions, &samples);

        // Reverse all the way back to the parent while both transitions are
        // active. Non-interruptible phases must keep coverage complete through
        // the reversal and eventual convergence.
        for _ in 0..8 {
            transitions.retarget(&[parent], |_| 0.0);
            transitions.advance(0.25);
            assert_full_coverage(&transitions, &samples);
        }
        assert_eq!(
            transitions.draws(),
            vec![TransitionDraw {
                cell: parent,
                fade: 1.0
            }]
        );
    }

    #[test]
    fn runtime_coverage_metric_detects_a_partial_sole_backdrop() {
        let cell = CellId {
            face: Face(0),
            level: 10,
            i: 100,
            j: 200,
        };
        let mut transitions = LodTransitions::new(1.0);
        transitions.entries.insert(
            CellKey::from(cell),
            Entry {
                alpha: 0.5,
                phase: TransitionPhase::Incoming,
                target_epoch: transitions.target_epoch,
            },
        );
        assert_eq!(transitions.uncovered_target_regions(), 1);

        transitions.entries.insert(
            CellKey::from(cell.parent().unwrap()),
            Entry {
                alpha: 0.5,
                phase: TransitionPhase::Outgoing,
                target_epoch: 0,
            },
        );
        assert_eq!(transitions.uncovered_target_regions(), 0);
    }

    #[test]
    fn uncovered_new_cells_can_enter_opaque() {
        let cell = CellId {
            face: Face(2),
            level: 8,
            i: 30,
            j: 40,
        };
        let mut transitions = LodTransitions::new(0.3);
        transitions.retarget(&[cell], |_| 1.0);
        transitions.advance(0.01);
        assert_eq!(transitions.phase(cell), Some(TransitionPhase::Stable));
        assert_eq!(
            transitions.draws(),
            vec![TransitionDraw { cell, fade: 1.0 }]
        );
    }
}
