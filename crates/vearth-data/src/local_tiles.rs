use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};

pub(crate) type TileKey = (u8, u32, u32);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceId(u64);

impl SourceId {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("raster source identities exhausted");
        Self(id)
    }
}

const LOCAL_TILES: usize = 8;
/// Two decoded 512×512 elevation tiles: a bilinear sample near a tile
/// corner can alternate between neighbours without leaving this cache.
const LOCAL_BYTES: usize = 2 * 1024 * 1024;
const REFRESH_HITS: u8 = 128;

struct Entry<T> {
    source: SourceId,
    key: TileKey,
    slot: Weak<OnceLock<Option<Arc<T>>>>,
    tile: Arc<T>,
    bytes: usize,
    remaining: u8,
}

// Successful tiles are immutable. Local readers borrow their raster without
// contending on a provider mutex or Arc counter. Weak keyed-slot liveness
// remains authoritative even if another key shares the same raster Arc.
// Strong raster leases have a per-type, per-thread byte cap; oversized native
// DEM tiles bypass this cache. Periodic misses refresh the shared LRU age.
pub(crate) struct LocalTiles<T> {
    entries: [Option<Entry<T>>; LOCAL_TILES],
    next: usize,
    bytes: usize,
}

impl<T> LocalTiles<T> {
    pub(crate) const fn can_retain(bytes: usize) -> bool {
        bytes <= LOCAL_BYTES
    }

    pub(crate) const fn new() -> Self {
        Self {
            entries: [const { None }; LOCAL_TILES],
            next: 0,
            bytes: 0,
        }
    }

    pub(crate) fn with<R>(
        &mut self,
        source: SourceId,
        key: TileKey,
        read: impl FnOnce(&Arc<T>) -> R,
    ) -> Option<R> {
        let position = self.entries.iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.source == source && entry.key == key)
        })?;
        let entry = self.entries[position].as_mut().unwrap();
        // Eviction can race this load just as it can race an ordinary map
        // lookup. The local lease owns the immutable memory through the read.
        if entry.slot.strong_count() == 0 {
            self.remove(position);
            return None;
        }
        if entry.remaining == 0 {
            return None;
        }
        entry.remaining -= 1;
        Some(read(&entry.tile))
    }

    pub(crate) fn get(&mut self, source: SourceId, key: TileKey) -> Option<Arc<T>> {
        self.with(source, key, Arc::clone)
    }

    fn remove(&mut self, position: usize) {
        if let Some(entry) = self.entries[position].take() {
            self.bytes -= entry.bytes;
        }
    }

    pub(crate) fn insert(
        &mut self,
        source: SourceId,
        key: TileKey,
        slot: &Arc<OnceLock<Option<Arc<T>>>>,
        bytes: usize,
    ) {
        if !Self::can_retain(bytes) {
            return;
        }
        let Some(Some(tile)) = slot.get() else {
            return;
        };
        let position = self.entries.iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.source == source && entry.key == key)
        });
        if let Some(position) = position {
            self.remove(position);
        }
        while self.bytes + bytes > LOCAL_BYTES {
            self.remove(self.next);
            self.next = (self.next + 1) % LOCAL_TILES;
        }
        let position = position.unwrap_or_else(|| {
            let position = self.next;
            self.next = (self.next + 1) % LOCAL_TILES;
            position
        });
        self.remove(position);
        self.entries[position] = Some(Entry {
            source,
            key,
            slot: Arc::downgrade(slot),
            tile: Arc::clone(tile),
            bytes,
            remaining: REFRESH_HITS,
        });
        self.bytes += bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot<T>(value: T) -> Arc<OnceLock<Option<Arc<T>>>> {
        Arc::new(OnceLock::from(Some(Arc::new(value))))
    }

    #[test]
    fn providers_with_identical_keys_stay_independent() {
        let mut cache = LocalTiles::new();
        let first = SourceId::new();
        let second = SourceId::new();
        let a = slot(10);
        let b = slot(20);
        cache.insert(first, (1, 0, 0), &a, 4);
        cache.insert(second, (1, 0, 0), &b, 4);
        assert_eq!(*cache.get(first, (1, 0, 0)).unwrap(), 10);
        assert_eq!(*cache.get(second, (1, 0, 0)).unwrap(), 20);
        assert!(cache.get(SourceId::new(), (1, 0, 0)).is_none());
    }

    #[test]
    fn local_entries_do_not_pin_shared_slots_and_refresh_shared_recency() {
        let mut cache = LocalTiles::new();
        let source = SourceId::new();
        let tile = slot(vec![1; 65536]);
        cache.insert(source, (1, 0, 0), &tile, 65536);
        assert_eq!(Arc::strong_count(&tile), 1);
        for _ in 0..REFRESH_HITS {
            assert!(cache.get(source, (1, 0, 0)).is_some());
        }
        assert!(cache.get(source, (1, 0, 0)).is_none());
        cache.insert(source, (1, 0, 0), &tile, 65536);
        assert!(cache.get(source, (1, 0, 0)).is_some());
        let other_key = Arc::new(OnceLock::from(tile.get().unwrap().clone()));
        drop(tile);
        assert!(cache.get(source, (1, 0, 0)).is_none());
        assert_eq!(cache.bytes, 0);
        assert!(other_key.get().unwrap().is_some());
    }

    #[test]
    fn local_metadata_and_raster_bytes_are_bounded() {
        let mut cache = LocalTiles::new();
        let half = LOCAL_BYTES / 2;
        let tile = slot(vec![1u8; half]);
        for x in 0..1024 {
            cache.insert(SourceId::new(), (12, x, 0), &tile, half);
            assert!(cache.bytes <= LOCAL_BYTES);
        }
        assert_eq!(cache.entries.iter().flatten().count(), 2);
        assert_eq!(Arc::weak_count(&tile), 2);
        assert_eq!(Arc::strong_count(&tile), 1);
        let large = slot(vec![2u8; LOCAL_BYTES + 1]);
        let source = SourceId::new();
        cache.insert(source, (0, 0, 0), &large, LOCAL_BYTES + 1);
        assert!(cache.get(source, (0, 0, 0)).is_none());
        assert_eq!(Arc::strong_count(large.get().unwrap().as_ref().unwrap()), 1);
        let tiny = slot(vec![1u8]);
        for x in 0..1024 {
            cache.insert(SourceId::new(), (12, x, 0), &tiny, 1);
        }
        assert_eq!(cache.entries.iter().flatten().count(), LOCAL_TILES);
    }

    #[test]
    fn local_reads_borrow_without_cloning_the_raster() {
        let mut cache = LocalTiles::new();
        let source = SourceId::new();
        let tile = slot(42);
        cache.insert(source, (0, 0, 0), &tile, 4);
        let shared = tile.get().unwrap().as_ref().unwrap();
        let owners = Arc::strong_count(shared);
        assert_eq!(
            cache.with(source, (0, 0, 0), |raster| {
                assert_eq!(Arc::strong_count(raster), owners);
                **raster
            }),
            Some(42)
        );
    }
}
