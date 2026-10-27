//! A thread-caching front end for the browser's global allocator.
//!
//! With shared memory, the standard wasm allocator guards one heap with one
//! lock, so allocation-heavy work on several workers serializes: measured
//! aggregate throughput fell 16-fold from one thread to four. Small blocks
//! here come from per-thread free lists per size class; each thread takes
//! the global lock only to carve a whole slab or to move a batch of blocks.
//!
//! Blocks freed on another thread (meshes built on generation workers and
//! released by the render thread) join that thread's list. Lists longer than
//! two batches return one batch to a shared per-class depot, which refills
//! other threads before new slabs are carved, so memory circulates instead
//! of accumulating where it is freed. Slabs are never returned to the system
//! allocator; small-block memory is bounded by its peak use. Blocks cached
//! by a thread that exits are not reclaimed.

#[cfg(target_arch = "wasm32")]
pub use wasm::ThreadCachingAlloc;

/// Largest request served from size classes; larger ones go to the system
/// allocator.
const MAX_SMALL: usize = 32 * 1024;
/// Alignment every class block satisfies.
const BLOCK_ALIGN: usize = 16;

/// Class sizes: 16-byte steps to 256 bytes, then four steps per power of two.
const CLASSES: usize = 16 + 4 * 7;

const fn class_size(class: usize) -> usize {
    if class < 16 {
        (class + 1) * 16
    } else {
        let step = class - 16;
        let base = 256 << (step / 4);
        base + (base / 4) * (step % 4 + 1)
    }
}

const fn class_of(size: usize) -> usize {
    if size <= 256 {
        if size == 0 { 0 } else { (size - 1) / 16 }
    } else {
        // Largest power of two not above size - 1, relative to 256.
        let above = size - 1;
        let log = usize::BITS as usize - 1 - above.leading_zeros() as usize;
        let base = 1usize << log;
        let quarter = (above - base) / (base / 4);
        16 + (log - 8) * 4 + quarter
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{BLOCK_ALIGN, CLASSES, MAX_SMALL, class_of, class_size};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::ptr::null_mut;
    use std::sync::Mutex;

    const SLAB_BYTES: usize = 64 * 1024;
    /// Blocks moved to or from the depot at once.
    const BATCH: usize = 32;

    /// An intrusive list of free blocks: each block's first word points to the
    /// next block. A depot batch also links to the next batch in its second word.
    #[derive(Clone, Copy)]
    struct List {
        head: *mut u8,
        len: usize,
    }

    impl List {
        const EMPTY: List = List {
            head: null_mut(),
            len: 0,
        };

        /// # Safety
        /// `block` is a free class block of at least 16 bytes owned by the caller.
        unsafe fn push(&mut self, block: *mut u8) {
            unsafe { (block as *mut *mut u8).write(self.head) };
            self.head = block;
            self.len += 1;
        }

        fn pop(&mut self) -> *mut u8 {
            let block = self.head;
            if !block.is_null() {
                // SAFETY: listed blocks store the next pointer in their first word.
                self.head = unsafe { (block as *mut *mut u8).read() };
                self.len -= 1;
            }
            block
        }

        /// Detaches the first `count` blocks as a list of their own.
        fn split_off_front(&mut self, count: usize) -> List {
            let front = List {
                head: self.head,
                len: count,
            };
            let mut last = self.head;
            for _ in 1..count {
                // SAFETY: the list holds more than `count` linked blocks.
                last = unsafe { (last as *mut *mut u8).read() };
            }
            // SAFETY: `last` is the `count`th block; its link starts the rest.
            self.head = unsafe { (last as *mut *mut u8).read() };
            unsafe { (last as *mut *mut u8).write(null_mut()) };
            self.len -= count;
            front
        }
    }

    /// Full batches per class, linked through each batch head's second word.
    struct Depot {
        batches: [*mut u8; CLASSES],
    }

    // SAFETY: the depot only holds free blocks, owned by whoever pops them.
    unsafe impl Send for Depot {}

    static DEPOT: Mutex<Depot> = Mutex::new(Depot {
        batches: [null_mut(); CLASSES],
    });

    #[thread_local]
    static mut CACHE: [List; CLASSES] = [List::EMPTY; CLASSES];

    /// The browser build's `#[global_allocator]`.
    pub struct ThreadCachingAlloc;

    impl ThreadCachingAlloc {
        fn small_class(layout: &Layout) -> Option<usize> {
            (layout.size() <= MAX_SMALL && layout.align() <= BLOCK_ALIGN)
                .then(|| class_of(layout.size()))
        }

        /// Refills `list` from the depot, or else from a new slab.
        ///
        /// # Safety
        /// Called with this thread's list for `class`.
        #[cold]
        unsafe fn refill(class: usize, list: &mut List) {
            let batch = {
                let mut depot = DEPOT.lock().unwrap_or_else(|poison| poison.into_inner());
                let head = depot.batches[class];
                if !head.is_null() {
                    // SAFETY: a batch head's second word links the next batch.
                    depot.batches[class] = unsafe { (head as *mut *mut u8).add(1).read() };
                }
                head
            };
            if !batch.is_null() {
                *list = List {
                    head: batch,
                    len: BATCH,
                };
                return;
            }
            let size = class_size(class);
            let blocks = (SLAB_BYTES / size).max(BATCH);
            // SAFETY: non-zero size and a valid power-of-two alignment.
            let slab = unsafe {
                System.alloc(Layout::from_size_align_unchecked(
                    blocks * size,
                    BLOCK_ALIGN,
                ))
            };
            if slab.is_null() {
                return;
            }
            for index in (0..blocks).rev() {
                // SAFETY: each block lies inside the fresh slab.
                unsafe { list.push(slab.add(index * size)) };
            }
        }

        /// Returns one batch from an overlong list to the depot.
        #[cold]
        fn spill(class: usize, list: &mut List) {
            let batch = list.split_off_front(BATCH);
            let mut depot = DEPOT.lock().unwrap_or_else(|poison| poison.into_inner());
            // SAFETY: class blocks are at least 16 bytes, so the head has a
            // second word for the batch link.
            unsafe {
                (batch.head as *mut *mut u8)
                    .add(1)
                    .write(depot.batches[class])
            };
            depot.batches[class] = batch.head;
        }
    }

    unsafe impl GlobalAlloc for ThreadCachingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let Some(class) = Self::small_class(&layout) else {
                return unsafe { System.alloc(layout) };
            };
            // SAFETY: the thread-local cache is only touched by its own thread,
            // and allocator calls do not re-enter on the same thread.
            let list = unsafe { &mut (*std::ptr::addr_of_mut!(CACHE))[class] };
            if list.head.is_null() {
                unsafe { Self::refill(class, list) };
            }
            list.pop()
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            let Some(class) = Self::small_class(&layout) else {
                return unsafe { System.dealloc(ptr, layout) };
            };
            let list = unsafe { &mut (*std::ptr::addr_of_mut!(CACHE))[class] };
            unsafe { list.push(ptr) };
            if list.len > 2 * BATCH {
                Self::spill(class, list);
            }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: `GlobalAlloc::realloc` guarantees a valid new layout.
            let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
            match (Self::small_class(&layout), Self::small_class(&new_layout)) {
                (Some(old), Some(new)) if old == new => ptr,
                (None, None) => unsafe { System.realloc(ptr, layout, new_size) },
                _ => {
                    let new = unsafe { self.alloc(new_layout) };
                    if !new.is_null() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                            self.dealloc(ptr, layout);
                        }
                    }
                    new
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_small_size_maps_to_a_class_that_fits_it() {
        for size in 1..=MAX_SMALL {
            let class = class_of(size);
            assert!(class < CLASSES, "size {size}");
            assert!(class_size(class) >= size, "size {size}");
            assert_eq!(class_size(class) % BLOCK_ALIGN, 0);
            if class > 0 {
                assert!(class_size(class - 1) < size, "size {size} not tight");
            }
        }
        assert_eq!(class_size(CLASSES - 1), MAX_SMALL);
    }
}
