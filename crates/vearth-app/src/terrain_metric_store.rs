//! Session-owned LOD evidence, independent of GPU and collision residency.

use crate::streaming::TerrainMetricRef;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use vearth_core::cell::CellId;
use vearth_core::cell_key::CellKey;

const HISTORY_BYTES: usize = 16 * 1024 * 1024;
const HISTORY_ENTRIES: usize = 2048;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainMetric {
    pub height_range: (f32, f32),
    pub terrain_step_m: f32,
    pub terrain_relief_m: f32,
    pub terrain_detail: Option<Arc<vearth_gen::TerrainDetailSummary>>,
}

impl TerrainMetric {
    pub(crate) fn as_ref(&self) -> TerrainMetricRef<'_> {
        TerrainMetricRef {
            height_range: self.height_range,
            terrain_step_m: self.terrain_step_m,
            terrain_relief_m: self.terrain_relief_m,
            terrain_detail: self.terrain_detail.as_deref(),
        }
    }
}

#[derive(Debug)]
struct Entry {
    cell: CellId,
    metric: TerrainMetric,
    resident: bool,
    complete: bool,
    used: Cell<u64>,
}

impl Entry {
    fn bytes(&self) -> usize {
        std::mem::size_of::<(CellKey, Self)>()
            + self
                .metric
                .terrain_detail
                .as_ref()
                .map_or(0, |detail| detail.heap_bytes())
    }
}

/// The fixed budget covers inactive history. Resident evidence and records
/// used by the last two selector queries are protected separately: evicting
/// an active ancestor's metric would change the cut and request its mesh again.
/// Their count is bounded by residency and the selector's bounded traversal.
/// Summaries share their existing allocation; raw source grids are not copied.
#[derive(Debug)]
pub(crate) struct TerrainMetricStore {
    entries: HashMap<CellKey, Entry>,
    epoch: u64,
    querying: bool,
    observed: Cell<bool>,
    history_bytes: usize,
    history_entries: usize,
    has_provisional_history: bool,
    byte_limit: usize,
    entry_limit: usize,
}

impl Default for TerrainMetricStore {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            epoch: 2,
            querying: false,
            observed: Cell::new(false),
            history_bytes: 0,
            history_entries: 0,
            has_provisional_history: false,
            byte_limit: HISTORY_BYTES,
            entry_limit: HISTORY_ENTRIES,
        }
    }
}

impl TerrainMetricStore {
    pub(crate) fn upsert(&mut self, cell: CellId, metric: TerrainMetric, complete: bool) {
        let used = self
            .entries
            .get(&CellKey::from(cell))
            .map_or(0, |entry| entry.used.get());
        let entry = Entry {
            cell,
            metric,
            resident: true,
            complete,
            used: Cell::new(used),
        };
        if let Some(old) = self.entries.insert(CellKey::from(cell), entry)
            && !old.resident
        {
            self.history_entries -= 1;
            self.history_bytes -= old.bytes();
        }
    }

    pub(crate) fn set_complete(&mut self, cell: CellId, complete: bool) {
        if let Some(entry) = self.entries.get_mut(&CellKey::from(cell)) {
            entry.complete = complete;
            self.has_provisional_history |= !entry.resident && !complete;
        }
    }

    pub(crate) fn get(&self, cell: CellId) -> Option<TerrainMetricRef<'_>> {
        let entry = self.entries.get(&CellKey::from(cell))?;
        if self.querying {
            entry.used.set(self.epoch);
            self.observed.set(true);
        }
        Some(entry.metric.as_ref())
    }

    pub(crate) fn is_resident(&self, cell: CellId) -> bool {
        self.entries
            .get(&CellKey::from(cell))
            .is_some_and(|entry| entry.resident)
    }

    pub(crate) fn remove_resident(&mut self, cell: CellId) {
        let key = CellKey::from(cell);
        let Some(entry) = self.entries.get_mut(&key) else {
            return;
        };
        if !entry.resident {
            return;
        }
        if !entry.complete
            && (entry.metric.terrain_detail.is_none()
                || entry.used.get() < self.epoch.saturating_sub(1))
        {
            // Missing elevation or inactive partial output cannot become history.
            self.entries.remove(&key);
            return;
        }
        // The summary certifies successful elevation sampling independently of
        // building/road generation. Keep an active provisional observation's
        // terrain and emitted-content range stable across mesh eviction; a newer
        // result still replaces it, and inactivity retires it below any budget.
        self.has_provisional_history |= !entry.complete;
        entry.resident = false;
        self.history_entries += 1;
        self.history_bytes += entry.bytes();
    }

    pub(crate) fn begin_query(&mut self) {
        assert!(!self.querying);
        // Preserve relative ages on the practically unreachable counter wrap.
        if self.epoch == u64::MAX {
            for entry in self.entries.values() {
                entry.used.set(if entry.used.get() == self.epoch {
                    2
                } else {
                    u64::from(entry.used.get() == self.epoch - 1)
                });
            }
            self.epoch = 2;
        }
        self.epoch += 1;
        self.querying = true;
        self.observed.set(false);
    }

    pub(crate) fn finish_query(&mut self) -> Vec<CellId> {
        if !self.end_query() {
            self.prune();
            return Vec::new();
        }
        let mut dependencies: Vec<_> = self
            .entries
            .values()
            .filter(|entry| entry.used.get() == self.epoch)
            .map(|entry| entry.cell)
            .collect();
        dependencies.sort_unstable();
        self.prune();
        dependencies
    }

    pub(crate) fn protect_dependencies(&mut self, dependencies: &[CellId]) {
        if dependencies.is_empty() {
            self.prune();
            return;
        }
        self.begin_query();
        for &cell in dependencies {
            let _ = self.get(cell);
        }
        // The caller already owns these dependencies. Retire the query without
        // scanning and sorting the full store to construct the same list again.
        self.end_query();
        self.prune();
    }

    fn end_query(&mut self) -> bool {
        assert!(self.querying);
        self.querying = false;
        if !self.observed.get() {
            // A no-query fast path is not evidence that the old dependencies
            // are irrelevant. Keep their protection until a real query runs.
            self.epoch -= 1;
            return false;
        }
        true
    }

    /// Call once after a batch of residency removals, not for each mesh.
    pub(crate) fn prune(&mut self) {
        if self.has_provisional_history {
            let active_epoch = self.epoch.saturating_sub(1);
            let mut removed_entries = 0;
            let mut removed_bytes = 0;
            let mut retained_provisional = false;
            self.entries.retain(|_, entry| {
                if entry.resident || entry.complete {
                    return true;
                }
                if entry.metric.terrain_detail.is_some() && entry.used.get() >= active_epoch {
                    retained_provisional = true;
                    return true;
                }
                removed_entries += 1;
                removed_bytes += entry.bytes();
                false
            });
            self.history_entries -= removed_entries;
            self.history_bytes -= removed_bytes;
            self.has_provisional_history = retained_provisional;
        }
        if self.history_entries <= self.entry_limit && self.history_bytes <= self.byte_limit {
            return;
        }
        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.resident && entry.used.get() < self.epoch.saturating_sub(1))
            .map(|(&key, entry)| (entry.used.get(), key, entry.bytes()))
            .collect();
        let mut bytes: usize = candidates.iter().map(|entry| entry.2).sum();
        let mut count = candidates.len();
        if count <= self.entry_limit && bytes <= self.byte_limit {
            return;
        }
        candidates.sort_unstable_by_key(|entry| (entry.0, entry.1));
        for (_, key, cost) in candidates {
            if count <= self.entry_limit && bytes <= self.byte_limit {
                break;
            }
            self.entries.remove(&key);
            self.history_entries -= 1;
            self.history_bytes -= cost;
            count -= 1;
            bytes -= cost;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.epoch = 2;
        self.querying = false;
        self.observed.set(false);
        self.history_entries = 0;
        self.has_provisional_history = false;
        self.history_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::cubesphere::Face;

    fn cell(i: u32) -> CellId {
        CellId {
            face: Face(4),
            level: 12,
            i,
            j: 2198,
        }
    }

    fn metric(step: f32) -> TerrainMetric {
        TerrainMetric {
            height_range: (-156.13591, 2.3989596),
            terrain_step_m: step,
            terrain_relief_m: 64.97048,
            terrain_detail: None,
        }
    }

    fn certified_metric(step: f32) -> TerrainMetric {
        let mut value = metric(step);
        value.terrain_detail = Some(Arc::new(
            vearth_gen::TerrainDetailSummary::from_grid(&vearth_gen::TerrainDetailGrid {
                heights_m: Box::new(std::array::from_fn(|i| (i % 33) as f32 * step)),
                source_columns: [u32::MAX; 32],
            })
            .unwrap(),
        ));
        value
    }

    #[test]
    fn active_certified_partial_evidence_expires_independently_of_history_budget() {
        let mut store = TerrainMetricStore::default();
        let source = cell(1);
        let other = cell(2);
        let original = certified_metric(2.0);
        store.upsert(source, original.clone(), false);
        store.upsert(other, metric(0.0), true);
        store.begin_query();
        assert_eq!(store.get(source), Some(original.as_ref()));
        assert_eq!(store.finish_query(), vec![source]);
        store.remove_resident(source);
        assert!(!store.is_resident(source));
        assert_eq!(store.get(source), Some(original.as_ref()));
        assert!(!store.entries[&CellKey::from(source)].complete);
        assert_eq!(store.history_entries, 1);
        assert!(store.history_bytes < store.byte_limit);

        // A current or immediately previous selector dependency remains live.
        store.begin_query();
        assert!(store.get(other).is_some());
        store.finish_query();
        assert_eq!(store.get(source), Some(original.as_ref()));
        store.begin_query();
        assert!(store.get(other).is_some());
        store.finish_query();
        assert!(store.get(source).is_none());
        assert_eq!((store.history_entries, store.history_bytes), (0, 0));
        assert!(!store.has_provisional_history);
    }

    #[test]
    fn partial_evidence_requires_certified_active_terrain_and_never_restores_old_values() {
        let mut store = TerrainMetricStore::default();
        let source = cell(1);
        let old = certified_metric(2.0);
        store.upsert(source, old.clone(), false);
        store.remove_resident(source);
        assert!(
            store.get(source).is_none(),
            "unqueried partial output is not history"
        );

        store.upsert(source, old.clone(), false);
        store.begin_query();
        assert!(store.get(source).is_some());
        store.finish_query();
        store.remove_resident(source);
        let replacement = certified_metric(4.0);
        store.upsert(source, replacement.clone(), false);
        assert_eq!(store.history_entries, 0);
        assert_eq!(store.get(source), Some(replacement.as_ref()));
        store.remove_resident(source);
        assert_eq!(store.get(source), Some(replacement.as_ref()));

        store.upsert(source, metric(0.0), false);
        store.begin_query();
        assert!(store.get(source).is_some());
        store.finish_query();
        store.remove_resident(source);
        assert!(
            store.get(source).is_none(),
            "failed elevation cannot resurrect older evidence"
        );
        store.prune();
        assert_eq!((store.history_entries, store.history_bytes), (0, 0));
        assert!(!store.has_provisional_history);

        store.upsert(source, old, false);
        store.begin_query();
        assert!(store.get(source).is_some());
        store.finish_query();
        store.remove_resident(source);
        store.clear();
        assert!(store.get(source).is_none());
        assert!(!store.has_provisional_history);
    }

    #[test]
    fn metric_history_preserves_evidence_without_residency_or_old_partial_values() {
        let mut store = TerrainMetricStore::default();
        let source = cell(3791);
        let original = metric(61.563374);
        store.upsert(source, original.clone(), true);
        store.begin_query();
        assert_eq!(store.get(source), Some(original.as_ref()));
        assert_eq!(store.finish_query(), vec![source]);
        store.remove_resident(source);
        store.remove_resident(source);
        assert!(!store.is_resident(source));
        assert_eq!(store.get(source), Some(original.as_ref()));
        assert_eq!(store.history_entries, 1);
        store.upsert(source, metric(0.0), false);
        assert!(store.is_resident(source));
        assert_eq!(store.history_entries, 0);
        assert_eq!(store.get(source).unwrap().terrain_step_m, 0.0);
        store.remove_resident(source);
        assert!(
            store.get(source).is_none(),
            "partial replacement must not resurrect old data"
        );
        store.upsert(source, original, true);
        store.remove_resident(source);
        store.clear();
        assert!(store.get(source).is_none());
        assert_eq!((store.history_entries, store.history_bytes), (0, 0));
    }

    #[test]
    fn metric_history_pressure_preserves_current_and_previous_query_dependencies() {
        let mut store = TerrainMetricStore {
            byte_limit: 0,
            entry_limit: 0,
            ..Default::default()
        };
        for i in 0..80 {
            store.upsert(cell(i), metric(i as f32), true);
        }
        for i in 0..80 {
            store.remove_resident(cell(i));
        }
        for active in [vec![cell(1), cell(2)], vec![cell(2)], vec![cell(2)]] {
            store.begin_query();
            for cell in active {
                assert!(store.get(cell).is_some());
            }
            store.finish_query();
        }
        assert!(store.get(cell(1)).is_none());
        assert!(store.get(cell(2)).is_some());
        assert_eq!(store.history_entries, 1);
        assert_eq!(store.entries.len(), 1);
        for _ in 0..10 {
            store.begin_query();
            assert!(store.get(cell(79)).is_none());
            assert!(store.finish_query().is_empty());
            assert!(
                store.get(cell(2)).is_some(),
                "an empty lookup must not retire the active query"
            );
        }
        // Simulate the accepted dependency transfer from the async worker.
        store.protect_dependencies(&[cell(2)]);
        assert!(store.get(cell(2)).is_some());
        assert!(!store.is_resident(cell(2)));
    }

    #[test]
    fn metric_history_unqueried_completions_remain_within_inactive_limits() {
        let mut store = TerrainMetricStore {
            byte_limit: usize::MAX,
            entry_limit: 3,
            ..Default::default()
        };
        for i in 0..80 {
            store.upsert(cell(i), metric(i as f32), true);
            store.remove_resident(cell(i));
        }
        store.prune();
        assert_eq!(store.history_entries, 3);
        for i in 100..180 {
            store.upsert(cell(i), metric(i as f32), true);
            store.remove_resident(cell(i));
        }
        store.protect_dependencies(&[]);
        assert_eq!(store.history_entries, 3);
        assert!(store.entries.values().all(|entry| !entry.resident));
    }

    #[test]
    fn metric_history_budget_counts_summary_bytes_and_releases_inactive_arcs() {
        let detail = Arc::new(
            vearth_gen::TerrainDetailSummary::from_grid(&vearth_gen::TerrainDetailGrid {
                heights_m: Box::new([0.0; 33 * 33]),
                source_columns: [u32::MAX; 32],
            })
            .unwrap(),
        );
        let mut store = TerrainMetricStore {
            byte_limit: detail.heap_bytes(),
            entry_limit: 8,
            ..Default::default()
        };
        for i in 0..4 {
            let mut m = metric(0.0);
            m.terrain_detail = Some(detail.clone());
            store.upsert(cell(i), m, true);
            store.remove_resident(cell(i));
        }
        assert_eq!(Arc::strong_count(&detail), 5);
        for _ in 0..3 {
            store.begin_query();
            for _ in 0..100 {
                assert!(std::ptr::eq(
                    store.get(cell(3)).unwrap().terrain_detail.unwrap(),
                    detail.as_ref()
                ));
            }
            store.finish_query();
        }
        assert_eq!(store.history_entries, 1);
        assert_eq!(Arc::strong_count(&detail), 2);
        assert!(
            store.history_bytes > store.byte_limit,
            "active evidence is accounted separately from the inactive budget"
        );
        store.clear();
        assert_eq!(Arc::strong_count(&detail), 1);
    }
}
