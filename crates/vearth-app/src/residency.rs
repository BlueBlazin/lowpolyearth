//! Byte-budgeted GPU residency and explicit short-lived leases.

use std::collections::{HashMap, HashSet};
use vearth_core::cell::CellId;
use vearth_core::cell_key::CellKey;

/// Eviction begins above the hard budget and trims unleased residents toward
/// the low budget, so turning back toward recent terrain skips a reload.
pub const DEFAULT_GPU_HARD_BYTES: usize = 1024 * 1024 * 1024;
pub const DEFAULT_GPU_LOW_BYTES: usize = 920 * 1024 * 1024;
pub const DEFAULT_EVICT_BYTES_PER_FRAME: usize = 8 * 1024 * 1024;
pub const DEFAULT_EVICT_CELLS_PER_FRAME: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseKind {
    Presented,
    Transition,
    RequiredAncestor,
    Guard,
    Predicted,
    Destination,
}

#[derive(Clone, Copy, Debug)]
struct Lease {
    kind: LeaseKind,
    until_frame: u64,
}

#[derive(Default)]
pub struct LeaseBook {
    leases: HashMap<CellKey, Lease>,
    // A lower bound on the earliest deadline. Renewal may leave it stale,
    // causing one extra scan, but can never postpone expiry.
    earliest_deadline: Option<u64>,
    registered: HashSet<CellKey>,
    evictable: HashSet<CellKey>,
    protected_eviction_attempts: u64,
    #[cfg(test)]
    expiry_scans: usize,
}

impl LeaseBook {
    /// Extend, never shorten, a cell lease.
    pub fn protect(&mut self, cell: CellId, kind: LeaseKind, until_frame: u64) {
        let key = CellKey::from(cell);
        self.evictable.remove(&key);
        let lease = self
            .leases
            .entry(key)
            .and_modify(|lease| {
                if until_frame >= lease.until_frame {
                    lease.until_frame = until_frame;
                    lease.kind = kind;
                }
            })
            .or_insert(Lease { kind, until_frame });
        self.earliest_deadline = Some(
            self.earliest_deadline
                .map_or(lease.until_frame, |earliest| {
                    earliest.min(lease.until_frame)
                }),
        );
    }

    /// Mirror a completed resident insertion. Metadata replacement is idempotent;
    /// an existing prefetch or presentation lease survives removal and reinsertion.
    pub fn register_resident(&mut self, cell: CellId, frame: u64) {
        // Coarse global coverage is permanently pinned, independently of leases.
        if cell.level <= 6 {
            return;
        }
        let key = CellKey::from(cell);
        self.registered.insert(key);
        if self.is_protected(cell, frame) {
            self.evictable.remove(&key);
        } else {
            self.evictable.insert(key);
        }
    }

    /// Mirror every resident removal, including removals under legacy policy.
    /// Lease expiry must never resurrect an ID whose GPU allocation was released.
    pub fn unregister_resident(&mut self, cell: CellId) {
        let key = CellKey::from(cell);
        self.registered.remove(&key);
        self.evictable.remove(&key);
    }

    pub fn is_protected(&self, cell: CellId, frame: u64) -> bool {
        self.leases
            .get(&CellKey::from(cell))
            .is_some_and(|lease| lease.until_frame >= frame)
    }

    pub fn kind(&self, cell: CellId, frame: u64) -> Option<LeaseKind> {
        self.leases
            .get(&CellKey::from(cell))
            .filter(|lease| lease.until_frame >= frame)
            .map(|lease| lease.kind)
    }

    pub fn expire(&mut self, frame: u64) {
        if self
            .earliest_deadline
            .is_none_or(|earliest| earliest >= frame)
        {
            return;
        }
        #[cfg(test)]
        {
            self.expiry_scans += 1;
        }
        let mut earliest = None::<u64>;
        self.leases.retain(|key, lease| {
            if lease.until_frame >= frame {
                earliest =
                    Some(earliest.map_or(lease.until_frame, |old| old.min(lease.until_frame)));
                true
            } else {
                if self.registered.contains(key) {
                    self.evictable.insert(*key);
                }
                false
            }
        });
        self.earliest_deadline = earliest;
    }

    pub fn len(&self) -> usize {
        self.leases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    pub fn protected_eviction_attempts(&self) -> u64 {
        self.protected_eviction_attempts
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ResidentCell {
    pub cell: CellId,
    pub gpu_bytes: usize,
    pub last_used_frame: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct ResidencyBudget {
    pub hard_bytes: usize,
    pub low_bytes: usize,
    pub evict_bytes_per_frame: usize,
}

impl ResidencyBudget {
    /// The default budget, halved where the host reports little memory.
    pub fn for_host() -> Self {
        let budget = Self::default();
        if crate::host::low_memory() {
            ResidencyBudget {
                hard_bytes: budget.hard_bytes / 2,
                low_bytes: budget.low_bytes / 2,
                ..budget
            }
        } else {
            budget
        }
    }

    pub fn validated(self) -> Self {
        let hard = self.hard_bytes.max(1);
        ResidencyBudget {
            hard_bytes: hard,
            low_bytes: self.low_bytes.min(hard),
            evict_bytes_per_frame: self.evict_bytes_per_frame.max(1),
        }
    }
}

impl Default for ResidencyBudget {
    fn default() -> Self {
        ResidencyBudget {
            hard_bytes: DEFAULT_GPU_HARD_BYTES,
            low_bytes: DEFAULT_GPU_LOW_BYTES,
            evict_bytes_per_frame: DEFAULT_EVICT_BYTES_PER_FRAME,
        }
    }
}

/// Select a deterministic, bounded LRU batch from registered, unleased residents.
/// The caller must lease every presentation/transition endpoint before planning
/// and mirror resident membership through [`LeaseBook::register_resident`] and
/// [`LeaseBook::unregister_resident`]. Coarse global coverage is pinned.
/// If protected data alone exceeds the hard budget, the planner reports no
/// victim instead of creating a visible hole.
pub fn eviction_plan(
    resident_count: usize,
    resident_bytes: usize,
    frame: u64,
    leases: &mut LeaseBook,
    budget: ResidencyBudget,
    mut resident: impl FnMut(CellId) -> ResidentCell,
) -> Vec<CellId> {
    let budget = budget.validated();
    leases.expire(frame);
    if resident_bytes <= budget.hard_bytes {
        return Vec::new();
    }
    // This is the same number of protected residents the full scan rejected;
    // no candidate metadata lookup is needed while every resident is protected.
    leases.protected_eviction_attempts += (resident_count - leases.evictable.len()) as u64;
    let candidates = leases
        .evictable
        .iter()
        .map(|key| resident(CellId::from(*key)))
        .collect();
    select_victims(candidates, resident_bytes, budget)
}

fn select_victims(
    mut candidates: Vec<ResidentCell>,
    resident_bytes: usize,
    budget: ResidencyBudget,
) -> Vec<CellId> {
    let need_total = resident_bytes.saturating_sub(budget.low_bytes);
    let frame_target = need_total.min(budget.evict_bytes_per_frame);
    let victim_order = |a: &ResidentCell, b: &ResidentCell| {
        a.last_used_frame
            .cmp(&b.last_used_frame)
            .then_with(|| b.gpu_bytes.cmp(&a.gpu_bytes))
            .then_with(|| a.cell.cmp(&b.cell))
    };
    if candidates.len() > DEFAULT_EVICT_CELLS_PER_FRAME {
        candidates.select_nth_unstable_by(DEFAULT_EVICT_CELLS_PER_FRAME, victim_order);
        candidates.truncate(DEFAULT_EVICT_CELLS_PER_FRAME);
    }
    candidates.sort_unstable_by(victim_order);
    let mut freed = 0usize;
    let mut victims = Vec::new();
    for candidate in candidates {
        if freed >= frame_target || victims.len() >= DEFAULT_EVICT_CELLS_PER_FRAME {
            break;
        }
        freed = freed.saturating_add(candidate.gpu_bytes);
        victims.push(candidate.cell);
    }
    victims
}

#[cfg(test)]
fn reference_eviction_plan(
    residents: impl IntoIterator<Item = ResidentCell>,
    resident_bytes: usize,
    frame: u64,
    leases: &mut LeaseBook,
    presented: &HashSet<CellId>,
    budget: ResidencyBudget,
) -> Vec<CellId> {
    let budget = budget.validated();
    leases.leases.retain(|_, lease| lease.until_frame >= frame);
    if resident_bytes <= budget.hard_bytes {
        return Vec::new();
    }

    let need_total = resident_bytes.saturating_sub(budget.low_bytes);
    let frame_target = need_total.min(budget.evict_bytes_per_frame);
    let mut candidates = Vec::new();
    for resident in residents {
        let protected = resident.cell.level <= 6
            || presented.contains(&resident.cell)
            || leases.is_protected(resident.cell, frame);
        if protected {
            leases.protected_eviction_attempts += 1;
        } else {
            candidates.push(resident);
        }
    }
    let victim_order = |a: &ResidentCell, b: &ResidentCell| {
        a.last_used_frame
            .cmp(&b.last_used_frame)
            .then_with(|| b.gpu_bytes.cmp(&a.gpu_bytes))
            .then_with(|| a.cell.cmp(&b.cell))
    };
    // Only a small batch can be evicted this frame. Partition in linear time
    // and sort that bounded prefix rather than sorting every resident cell
    // while pressure remains above the high watermark.
    if candidates.len() > DEFAULT_EVICT_CELLS_PER_FRAME {
        candidates.select_nth_unstable_by(DEFAULT_EVICT_CELLS_PER_FRAME, victim_order);
        candidates.truncate(DEFAULT_EVICT_CELLS_PER_FRAME);
    }
    candidates.sort_unstable_by(victim_order);

    let mut freed = 0usize;
    let mut victims = Vec::new();
    for candidate in candidates {
        if freed >= frame_target || victims.len() >= DEFAULT_EVICT_CELLS_PER_FRAME {
            break;
        }
        freed = freed.saturating_add(candidate.gpu_bytes);
        victims.push(candidate.cell);
    }
    victims
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::cubesphere::Face;

    fn id(level: u8, i: u32) -> CellId {
        CellId {
            face: Face(0),
            level,
            i,
            j: 0,
        }
    }

    fn checked_plan(
        residents: impl IntoIterator<Item = ResidentCell>,
        resident_bytes: usize,
        frame: u64,
        leases: &mut LeaseBook,
        presented: &HashSet<CellId>,
        budget: ResidencyBudget,
    ) -> Vec<CellId> {
        let residents: HashMap<_, _> = residents.into_iter().map(|r| (r.cell, r)).collect();
        let mut reference = LeaseBook {
            leases: leases.leases.clone(),
            earliest_deadline: leases.earliest_deadline,
            protected_eviction_attempts: leases.protected_eviction_attempts,
            ..LeaseBook::default()
        };
        for &cell in residents.keys() {
            leases.register_resident(cell, frame);
        }
        for &cell in presented {
            leases.protect(cell, LeaseKind::Presented, frame);
        }
        let expected = reference_eviction_plan(
            residents.values().copied(),
            resident_bytes,
            frame,
            &mut reference,
            presented,
            budget,
        );
        let actual = eviction_plan(
            residents.len(),
            resident_bytes,
            frame,
            leases,
            budget,
            |cell| residents[&cell],
        );
        assert_eq!(actual, expected);
        assert_eq!(
            leases.protected_eviction_attempts(),
            reference.protected_eviction_attempts()
        );
        actual
    }

    #[test]
    fn under_hard_watermark_does_not_evict() {
        let mut leases = LeaseBook::default();
        let plan = checked_plan(
            [ResidentCell {
                cell: id(8, 0),
                gpu_bytes: 10,
                last_used_frame: 0,
            }],
            99,
            10,
            &mut leases,
            &HashSet::new(),
            ResidencyBudget {
                hard_bytes: 100,
                low_bytes: 80,
                evict_bytes_per_frame: 10,
            },
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn eviction_is_incremental_lru_and_byte_bounded() {
        let residents = [
            ResidentCell {
                cell: id(8, 1),
                gpu_bytes: 6,
                last_used_frame: 1,
            },
            ResidentCell {
                cell: id(8, 2),
                gpu_bytes: 6,
                last_used_frame: 2,
            },
            ResidentCell {
                cell: id(8, 3),
                gpu_bytes: 6,
                last_used_frame: 3,
            },
        ];
        let mut leases = LeaseBook::default();
        let plan = checked_plan(
            residents,
            118,
            10,
            &mut leases,
            &HashSet::new(),
            ResidencyBudget {
                hard_bytes: 100,
                low_bytes: 80,
                evict_bytes_per_frame: 10,
            },
        );
        assert_eq!(plan, vec![id(8, 1), id(8, 2)]);
    }

    #[test]
    fn presented_leased_and_coarse_cells_are_never_victims() {
        let presented_cell = id(8, 1);
        let leased_cell = id(8, 2);
        let coarse_cell = id(6, 3);
        let ordinary_cell = id(8, 4);
        let residents =
            [presented_cell, leased_cell, coarse_cell, ordinary_cell].map(|cell| ResidentCell {
                cell,
                gpu_bytes: 10,
                last_used_frame: 0,
            });
        let mut leases = LeaseBook::default();
        leases.protect(leased_cell, LeaseKind::Destination, 20);
        let presented = HashSet::from([presented_cell]);
        let plan = checked_plan(
            residents,
            140,
            10,
            &mut leases,
            &presented,
            ResidencyBudget {
                hard_bytes: 100,
                low_bytes: 80,
                evict_bytes_per_frame: 100,
            },
        );
        assert_eq!(plan, vec![ordinary_cell]);
        assert_eq!(leases.kind(leased_cell, 10), Some(LeaseKind::Destination));
        assert_eq!(leases.protected_eviction_attempts(), 3);
    }

    #[test]
    fn expired_lease_becomes_evictable() {
        let cell = id(8, 1);
        let mut leases = LeaseBook::default();
        leases.protect(cell, LeaseKind::Guard, 5);
        let plan = checked_plan(
            [ResidentCell {
                cell,
                gpu_bytes: 20,
                last_used_frame: 0,
            }],
            120,
            6,
            &mut leases,
            &HashSet::new(),
            ResidencyBudget {
                hard_bytes: 100,
                low_bytes: 80,
                evict_bytes_per_frame: 20,
            },
        );
        assert_eq!(plan, vec![cell]);
        assert_eq!(leases.len(), 0);
    }

    #[test]
    fn expiry_deadline_preserves_inclusive_frames_and_renewals() {
        let [a, b, c] = [id(8, 1), id(8, 2), id(8, 3)];
        let mut leases = LeaseBook::default();
        leases.expire(u64::MAX);
        assert_eq!(leases.expiry_scans, 0);
        for cell in [a, b, c] {
            leases.register_resident(cell, 0);
        }
        leases.protect(a, LeaseKind::Presented, 10);
        leases.protect(b, LeaseKind::Guard, 20);
        leases.expire(10);
        assert_eq!(leases.expiry_scans, 0);
        assert!(leases.is_protected(a, 10));

        leases.protect(a, LeaseKind::Transition, 30);
        leases.protect(a, LeaseKind::Predicted, 29);
        assert_eq!(leases.kind(a, 30), Some(LeaseKind::Transition));
        // The old deadline is conservative after renewal. Its scan discovers
        // the next real deadline without expiring either surviving lease.
        leases.expire(11);
        assert_eq!(leases.expiry_scans, 1);
        assert_eq!(leases.earliest_deadline, Some(20));
        leases.protect(c, LeaseKind::Destination, 15);
        leases.expire(15);
        assert_eq!(leases.expiry_scans, 1);
        assert!(leases.evictable.is_empty());
        leases.expire(16);
        assert_eq!(leases.expiry_scans, 2);
        assert_eq!(leases.evictable, HashSet::from([CellKey::from(c)]));
        assert_eq!(leases.earliest_deadline, Some(20));
        leases.expire(20);
        assert_eq!(leases.expiry_scans, 2);
        leases.expire(21);
        assert_eq!(leases.expiry_scans, 3);
        assert_eq!(leases.earliest_deadline, Some(30));
        leases.expire(30);
        assert_eq!(leases.expiry_scans, 3);
        leases.expire(31);
        assert_eq!(leases.expiry_scans, 4);
        assert!(leases.is_empty());
        assert_eq!(leases.earliest_deadline, None);
        assert_eq!(leases.evictable.len(), 3);

        leases.protect(a, LeaseKind::Presented, u64::MAX);
        leases.expire(u64::MAX);
        assert_eq!(leases.expiry_scans, 4);
        assert!(leases.is_protected(a, u64::MAX));
        assert!(!leases.evictable.contains(&CellKey::from(a)));
    }

    #[test]
    fn eligibility_tracks_registration_renewal_expiry_and_removal() {
        let cell = id(8, 3);
        let key = CellKey::from(cell);
        let mut leases = LeaseBook::default();
        leases.protect(cell, LeaseKind::Destination, 8);
        leases.register_resident(cell, 4);
        leases.register_resident(cell, 4);
        assert_eq!(leases.registered.len(), 1);
        assert!(leases.evictable.is_empty());
        leases.protect(cell, LeaseKind::Presented, 6);
        assert_eq!(leases.kind(cell, 8), Some(LeaseKind::Destination));
        leases.protect(cell, LeaseKind::Guard, 8);
        assert_eq!(leases.kind(cell, 8), Some(LeaseKind::Guard));
        leases.expire(8);
        assert!(leases.evictable.is_empty());
        leases.expire(9);
        assert!(leases.evictable.contains(&key));
        leases.protect(cell, LeaseKind::Transition, 13);
        assert!(leases.evictable.is_empty());
        leases.unregister_resident(cell);
        leases.expire(14);
        assert!(leases.evictable.is_empty());
        assert!(leases.registered.is_empty());

        // Registration may precede this frame's expiry scan.
        leases.protect(cell, LeaseKind::Predicted, 15);
        leases.register_resident(cell, 16);
        assert!(leases.evictable.contains(&key));
        leases.unregister_resident(cell);
        leases.protect(cell, LeaseKind::Destination, 30);
        leases.register_resident(cell, 17);
        assert!(leases.evictable.is_empty());
        leases.unregister_resident(cell);
        leases.register_resident(cell, 31);
        leases.expire(31);
        assert!(leases.evictable.contains(&key));
        leases.register_resident(id(6, 3), 31);
        leases.protect(id(6, 3), LeaseKind::Presented, 31);
        leases.expire(32);
        assert_eq!(leases.registered, HashSet::from([key]));
        assert_eq!(leases.evictable, HashSet::from([key]));
    }

    #[test]
    fn protected_residency_needs_no_candidate_metadata_lookups() {
        let mut leases = LeaseBook::default();
        for i in 0..128 {
            let cell = id(8, i);
            leases.register_resident(cell, 7);
            leases.protect(cell, LeaseKind::Presented, 11);
        }
        leases.register_resident(id(6, 0), 7);
        let victims = eviction_plan(
            129,
            usize::MAX,
            7,
            &mut leases,
            ResidencyBudget::default(),
            |_| panic!("protected residents must not be scanned"),
        );
        assert!(victims.is_empty());
        assert_eq!(leases.protected_eviction_attempts(), 129);
        leases.expire(12);
        // Even eligible records need no lookup below the high watermark.
        assert!(
            eviction_plan(
                129,
                0,
                12,
                &mut leases,
                ResidencyBudget::default(),
                |_| panic!("below-watermark residents must not be scanned"),
            )
            .is_empty()
        );
        assert_eq!(leases.protected_eviction_attempts(), 129);
    }

    #[test]
    fn eligibility_preserves_zero_byte_order_and_oversize_progress() {
        let zeroes = (0..130).map(|i| ResidentCell {
            cell: id(8, i),
            gpu_bytes: 0,
            last_used_frame: 0,
        });
        let plan = checked_plan(
            zeroes,
            1000,
            1,
            &mut LeaseBook::default(),
            &HashSet::new(),
            ResidencyBudget {
                hard_bytes: 100,
                low_bytes: 80,
                evict_bytes_per_frame: 10,
            },
        );
        assert_eq!(plan, (0..64).map(|i| id(8, i)).collect::<Vec<_>>());
        let residents = [8, 9, 7].map(|i| ResidentCell {
            cell: id(8, i),
            gpu_bytes: if i == 7 { 1000 } else { 2000 },
            last_used_frame: 0,
        });
        assert_eq!(
            checked_plan(
                residents,
                5000,
                1,
                &mut LeaseBook::default(),
                &HashSet::new(),
                ResidencyBudget {
                    hard_bytes: 100,
                    low_bytes: 80,
                    evict_bytes_per_frame: 10,
                },
            ),
            [id(8, 8)]
        );
    }

    #[test]
    fn eligibility_matches_full_scan_through_deterministic_resident_events() {
        let mut indexed = LeaseBook::default();
        let mut reference = LeaseBook::default();
        let mut residents = HashMap::<CellId, ResidentCell>::new();
        let mut rng = 0x4a95_b20d_6327_80c1_u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let kinds = [
            LeaseKind::Presented,
            LeaseKind::Transition,
            LeaseKind::RequiredAncestor,
            LeaseKind::Guard,
            LeaseKind::Predicted,
            LeaseKind::Destination,
        ];
        for frame in 0..640_u64 {
            if frame == 193 || frame == 401 {
                residents.clear();
                indexed = LeaseBook::default();
                reference = LeaseBook::default();
            }
            for _ in 0..12 {
                let random = next();
                let level = 6 + ((random >> 9) % 5) as u8;
                let cell = id(level, ((random >> 20) % 64) as u32);
                match random % 7 {
                    0 | 1 => {
                        let record = ResidentCell {
                            cell,
                            gpu_bytes: [0, 5, 20, 100, 1000][(random >> 32) as usize % 5],
                            last_used_frame: frame.saturating_sub((random >> 40) % 8),
                        };
                        residents.insert(cell, record);
                        indexed.register_resident(cell, frame);
                    }
                    2 | 3 => {
                        let kind = kinds[(random >> 35) as usize % kinds.len()];
                        let until = frame.saturating_sub(3).saturating_add((random >> 42) % 15);
                        indexed.protect(cell, kind, until);
                        reference.protect(cell, kind, until);
                    }
                    4 => {
                        residents.remove(&cell);
                        indexed.unregister_resident(cell);
                    }
                    5 => {
                        if let Some(record) = residents.get_mut(&cell) {
                            record.last_used_frame = frame;
                        }
                    }
                    _ => {
                        indexed.expire(frame);
                        reference.expire(frame);
                    }
                }
            }
            let presented: HashSet<_> = residents
                .keys()
                .copied()
                .filter(|cell| (u64::from(cell.i) + frame) % 11 < 3)
                .collect();
            for &cell in &presented {
                let kind = if (cell.i + frame as u32).is_multiple_of(2) {
                    LeaseKind::Presented
                } else {
                    LeaseKind::Transition
                };
                indexed.protect(cell, kind, frame + 4);
                reference.protect(cell, kind, frame + 4);
                residents.get_mut(&cell).unwrap().last_used_frame = frame;
            }
            let total = residents.values().map(|r| r.gpu_bytes).sum::<usize>();
            let budget = if frame.is_multiple_of(9) {
                ResidencyBudget {
                    hard_bytes: total,
                    low_bytes: total.saturating_sub(10),
                    evict_bytes_per_frame: 13,
                }
            } else {
                ResidencyBudget {
                    hard_bytes: 600,
                    low_bytes: [0, 540, 900][frame as usize % 3],
                    evict_bytes_per_frame: [0, 15, 500, usize::MAX][frame as usize % 4],
                }
            };
            let expected = reference_eviction_plan(
                residents.values().copied(),
                total,
                frame,
                &mut reference,
                &presented,
                budget,
            );
            let mut lookups = 0;
            let actual = eviction_plan(
                residents.len(),
                total,
                frame,
                &mut indexed,
                budget,
                |cell| {
                    lookups += 1;
                    residents[&cell]
                },
            );
            let expected_eligible: HashSet<_> = residents
                .keys()
                .copied()
                .filter(|cell| cell.level > 6 && !reference.is_protected(*cell, frame))
                .map(CellKey::from)
                .collect();
            assert_eq!(indexed.evictable, expected_eligible, "frame {frame}");
            assert_eq!(actual, expected, "victims at frame {frame}");
            assert_eq!(
                indexed.protected_eviction_attempts(),
                reference.protected_eviction_attempts(),
                "protected count at frame {frame}"
            );
            assert_eq!(
                lookups,
                if total > budget.validated().hard_bytes {
                    expected_eligible.len()
                } else {
                    0
                }
            );
            let expected_freed: usize = expected.iter().map(|cell| residents[cell].gpu_bytes).sum();
            let mut freed = 0;
            for cell in actual {
                freed += residents.remove(&cell).unwrap().gpu_bytes;
                indexed.unregister_resident(cell);
            }
            assert_eq!(freed, expected_freed, "freed bytes at frame {frame}");
            assert_eq!(
                indexed.registered,
                residents
                    .keys()
                    .copied()
                    .filter(|cell| cell.level > 6)
                    .map(CellKey::from)
                    .collect()
            );
        }
    }
}
