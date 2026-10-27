//! Priority-aware staging between generation and GPU residency.
//!
//! Generation completion is bursty: one shared source tile can wake many
//! workers together. Keeping those meshes in a bounded CPU queue lets the
//! frame loop admit only a small, useful byte budget instead of allocating
//! every GPU buffer in the same frame.

use std::collections::{BinaryHeap, HashMap};
use vearth_core::cell::CellId;
use vearth_platform::time::Instant;

use crate::prepared_render::PreparedRenderCell;

/// Stable intent lanes. Ordering is part of the scheduler contract: visible
/// coverage must never queue behind speculative quality work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IntentClass {
    Background,
    DestinationDetail,
    DestinationCoverage,
    Predicted,
    Guard,
    VisibleQuality,
    VisibleCoverage,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UploadPriority {
    pub class: IntentClass,
    pub score: f64,
}

impl UploadPriority {
    pub const fn new(class: IntentClass, score: f64) -> Self {
        UploadPriority { class, score }
    }
}

pub struct StagedMesh {
    pub meshed: PreparedRenderCell,
    pub priority: UploadPriority,
    pub gpu_bytes: usize,
    pub cpu_bytes: usize,
    pub ready_at: Instant,
    sequence: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropStats {
    pub cells: usize,
    pub bytes: usize,
}

/// Bounded ready queue. Duplicate cells replace neither an existing staged
/// result nor a resident result; callers count the duplicate separately.
pub struct UploadStager {
    entries: HashMap<CellId, StagedMesh>,
    order: BinaryHeap<UploadRank>,
    bytes: usize,
    next_sequence: u64,
}

impl UploadStager {
    pub fn new() -> Self {
        UploadStager {
            entries: HashMap::new(),
            order: BinaryHeap::new(),
            bytes: 0,
            next_sequence: 0,
        }
    }

    /// Returns false for a duplicate staged cell.
    pub fn push(&mut self, meshed: PreparedRenderCell, priority: UploadPriority) -> bool {
        if self.entries.contains_key(&meshed.cell()) {
            return false;
        }
        let gpu_bytes = meshed.gpu_bytes();
        let cpu_bytes = meshed.owned_bytes();
        let cell = meshed.cell();
        let staged = StagedMesh {
            meshed,
            priority: sanitize_priority(priority),
            gpu_bytes,
            cpu_bytes,
            ready_at: Instant::now(),
            sequence: self.next_sequence,
        };
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.bytes = self.bytes.saturating_add(cpu_bytes);
        self.order.push(UploadRank::of(&staged));
        self.entries.insert(cell, staged);
        true
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, cell: CellId) -> bool {
        self.entries.contains_key(&cell)
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Discard every generated mesh that has not reached the GPU yet.
    /// Provider changes use this to prevent old-source completions from
    /// becoming resident after the visible world has been invalidated.
    pub fn clear(&mut self) -> DropStats {
        let dropped = DropStats {
            cells: self.entries.len(),
            bytes: self.bytes,
        };
        self.entries.clear();
        self.order.clear();
        self.bytes = 0;
        dropped
    }

    pub fn oldest_ready_ms(&self) -> f64 {
        self.entries
            .values()
            .map(|entry| entry.ready_at.elapsed().as_secs_f64() * 1e3)
            .fold(0.0, f64::max)
    }

    /// Estimated bytes of the same entry [`pop_next`](Self::pop_next) would
    /// return, without removing it.
    pub fn next_gpu_bytes(&self) -> Option<usize> {
        let cell = self.next_cell()?;
        self.entries.get(&cell).map(|entry| entry.gpu_bytes)
    }

    /// Refresh intent while reusing the mesh payloads and ranking storage. Entries
    /// unknown to the classifier retain their previous lane until explicit
    /// stale pruning, which avoids dropping a just-arrived backdrop between
    /// wanted refreshes.
    pub fn reprioritize(&mut self, mut classify: impl FnMut(CellId) -> Option<UploadPriority>) {
        let mut changed = false;
        for entry in self.entries.values_mut() {
            if let Some(priority) = classify(entry.meshed.cell()) {
                let priority = sanitize_priority(priority);
                changed |= priority.class != entry.priority.class
                    || priority.score.to_bits() != entry.priority.score.to_bits();
                entry.priority = priority;
            }
        }
        if changed {
            self.order.clear();
            self.order.extend(self.entries.values().map(UploadRank::of));
        }
    }

    /// Remove results that no longer have any desired, presented, guard, or
    /// leased use. Their deterministic disk/RAM chunk-cache entry remains
    /// available; only the pending GPU allocation is rejected.
    pub fn drop_stale(&mut self, mut useful: impl FnMut(CellId) -> bool) -> DropStats {
        let stale: Vec<_> = self
            .entries
            .keys()
            .copied()
            .filter(|cell| !useful(*cell))
            .collect();
        self.remove_cells(stale)
    }

    /// Highest-priority next upload. Lane, score, wait age, then CellId form
    /// a deterministic total order independent of HashMap iteration.
    pub fn pop_next(&mut self) -> Option<StagedMesh> {
        let cell = self.order.pop()?.cell;
        let entry = self.entries.remove(&cell).unwrap();
        self.bytes = self.bytes.saturating_sub(entry.cpu_bytes);
        Some(entry)
    }

    fn next_cell(&self) -> Option<CellId> {
        self.order.peek().map(|rank| rank.cell)
    }

    fn remove_cells(&mut self, cells: Vec<CellId>) -> DropStats {
        let mut dropped = DropStats::default();
        for cell in cells {
            if let Some(entry) = self.entries.remove(&cell) {
                self.bytes = self.bytes.saturating_sub(entry.cpu_bytes);
                dropped.cells += 1;
                dropped.bytes += entry.cpu_bytes;
            }
        }
        if dropped.cells > 0 {
            self.order
                .retain(|rank| self.entries.contains_key(&rank.cell));
        }
        dropped
    }
}

/// Upload-budget admission. Exactly one oversized mesh may make progress on
/// an otherwise empty frame; once any mesh was admitted, the next one must
/// fit in the remaining byte budget.
pub fn upload_fits_budget(
    uploaded_cells: usize,
    uploaded_bytes: usize,
    next_bytes: usize,
    byte_budget: usize,
) -> bool {
    uploaded_cells == 0 || uploaded_bytes.saturating_add(next_bytes) <= byte_budget
}

impl Default for UploadStager {
    fn default() -> Self {
        Self::new()
    }
}

fn sanitize_priority(priority: UploadPriority) -> UploadPriority {
    UploadPriority {
        class: priority.class,
        score: if priority.score.is_finite() {
            priority.score
        } else {
            0.0
        },
    }
}

// The heap holds only scheduling metadata. Mesh ownership stays in the map,
// and every priority refresh or removal keeps both indexes in agreement.
#[derive(Clone, Copy)]
struct UploadRank {
    cell: CellId,
    priority: UploadPriority,
    sequence: u64,
}

impl UploadRank {
    fn of(entry: &StagedMesh) -> Self {
        Self {
            cell: entry.meshed.cell(),
            priority: entry.priority,
            sequence: entry.sequence,
        }
    }
}

impl Ord for UploadRank {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .class
            .cmp(&other.priority.class)
            .then_with(|| self.priority.score.total_cmp(&other.priority.score))
            .then_with(|| other.sequence.cmp(&self.sequence))
            .then_with(|| other.cell.cmp(&self.cell))
    }
}

impl PartialOrd for UploadRank {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for UploadRank {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for UploadRank {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepared_render::pack_surface_cell;
    use vearth_core::cubesphere::Face;
    use vearth_gen::SurfaceCell;

    fn cell(i: u32) -> PreparedRenderCell {
        pack_surface_cell(SurfaceCell {
            cell: CellId {
                face: Face(0),
                level: 8,
                i,
                j: 0,
            },
            complete: true,
            batches: Vec::new(),
            night_lights: Vec::new(),
            tree_instances: Vec::new(),
            solid_collision: Default::default(),
            water: Vec::new(),
            flora: Vec::new(),
            height_range: (0.0, 0.0),
            terrain_step_m: 0.0,
            terrain_relief_m: 0.0,
            terrain_detail: None,
            terrain_collision: None,
            road_guides: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn intent_lane_wins_over_numeric_score() {
        let mut queue = UploadStager::new();
        queue.push(
            cell(1),
            UploadPriority::new(IntentClass::Background, 1_000_000.0),
        );
        queue.push(
            cell(2),
            UploadPriority::new(IntentClass::VisibleCoverage, 1.0),
        );
        assert_eq!(queue.pop_next().unwrap().meshed.cell().i, 2);
    }

    #[test]
    fn equal_priority_is_oldest_then_cell_deterministic() {
        let mut queue = UploadStager::new();
        let priority = UploadPriority::new(IntentClass::VisibleQuality, 10.0);
        queue.push(cell(9), priority);
        queue.push(cell(2), priority);
        assert_eq!(queue.pop_next().unwrap().meshed.cell().i, 9);
        assert_eq!(queue.pop_next().unwrap().meshed.cell().i, 2);
    }

    #[test]
    fn stale_drop_and_duplicate_do_not_corrupt_accounting() {
        let mut queue = UploadStager::new();
        let priority = UploadPriority::new(IntentClass::Guard, 1.0);
        assert!(queue.push(cell(1), priority));
        assert!(!queue.push(cell(1), priority));
        assert!(queue.push(cell(2), priority));
        let dropped = queue.drop_stale(|cell| cell.i == 2);
        assert_eq!(dropped.cells, 1);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.pop_next().unwrap().meshed.cell().i, 2);
        assert!(queue.is_empty());
    }

    #[test]
    fn heap_matches_scan_order_through_reprioritization_pruning_and_sequence_wrap() {
        let mut queue = UploadStager::new();
        queue.next_sequence = u64::MAX - 80;
        let lanes = [
            IntentClass::Background,
            IntentClass::Guard,
            IntentClass::VisibleQuality,
            IntentClass::VisibleCoverage,
        ];
        for i in 0..240 {
            let score = match i % 8 {
                0 => f64::NAN,
                1 => f64::INFINITY,
                2 => -0.0,
                3 => 0.0,
                _ => f64::from(i % 13),
            };
            assert!(queue.push(cell(i), UploadPriority::new(lanes[i as usize % 4], score)));
        }
        queue.reprioritize(|cell| {
            (cell.i % 3 == 0)
                .then(|| UploadPriority::new(IntentClass::Predicted, f64::from(cell.i % 9)))
        });
        queue.drop_stale(|cell| cell.i % 5 != 0);
        while !queue.is_empty() {
            // Independent reference to the former full-map maximum search.
            let expected = queue
                .entries
                .iter()
                .max_by(|(cell_a, a), (cell_b, b)| {
                    a.priority
                        .class
                        .cmp(&b.priority.class)
                        .then_with(|| a.priority.score.total_cmp(&b.priority.score))
                        .then_with(|| b.sequence.cmp(&a.sequence))
                        .then_with(|| cell_b.cmp(cell_a))
                })
                .map(|(cell, entry)| (*cell, entry.gpu_bytes))
                .unwrap();
            assert_eq!(queue.next_gpu_bytes(), Some(expected.1));
            assert_eq!(queue.pop_next().unwrap().meshed.cell(), expected.0);
            assert_eq!(queue.order.len(), queue.entries.len());
        }
        assert_eq!(queue.bytes(), 0);
        assert!(queue.next_gpu_bytes().is_none());
        assert!(queue.pop_next().is_none());
        queue.push(cell(1), UploadPriority::new(IntentClass::Guard, 1.0));
        queue.clear();
        assert!(queue.next_gpu_bytes().is_none());
    }

    #[test]
    fn oversized_upload_is_allowed_only_as_the_frames_progress_item() {
        let mut queue = UploadStager::new();
        let priority = UploadPriority::new(IntentClass::VisibleCoverage, 1.0);
        queue.push(cell(1), priority);
        queue.push(cell(2), priority);
        queue.entries.get_mut(&cell(1).cell()).unwrap().gpu_bytes = 1_024;
        queue.entries.get_mut(&cell(2).cell()).unwrap().gpu_bytes = 50 * 1024 * 1024;

        assert_eq!(queue.next_gpu_bytes(), Some(1_024));
        let small = queue.pop_next().unwrap();
        assert_eq!(small.meshed.cell().i, 1);
        let oversized = queue.next_gpu_bytes().unwrap();
        assert!(!upload_fits_budget(1, 1_024, oversized, 4 * 1024 * 1024));
        assert!(upload_fits_budget(0, 0, oversized, 4 * 1024 * 1024));
    }
}
