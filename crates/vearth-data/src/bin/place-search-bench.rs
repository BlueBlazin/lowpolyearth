//! Repeatable local measurements for the bundled place-search asset.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use vearth_data::geocode::Geocoder;

struct TrackingAllocator;
static CURRENT_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let current = CURRENT_BYTES.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK_BYTES.fetch_max(current, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        CURRENT_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, old: Layout, new_size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, old, new_size) };
        if !result.is_null() {
            if new_size >= old.size() {
                let added = new_size - old.size();
                let current = CURRENT_BYTES.fetch_add(added, Ordering::Relaxed) + added;
                PEAK_BYTES.fetch_max(current, Ordering::Relaxed);
            } else {
                CURRENT_BYTES.fetch_sub(old.size() - new_size, Ordering::Relaxed);
            }
        }
        result
    }
}

fn begin_peak_window() -> usize {
    let baseline = CURRENT_BYTES.load(Ordering::Relaxed);
    PEAK_BYTES.store(baseline, Ordering::Relaxed);
    baseline
}

fn peak_delta(baseline: usize) -> usize {
    PEAK_BYTES.load(Ordering::Relaxed).saturating_sub(baseline)
}

fn percentile(sorted: &[u128], percentile: f64) -> u128 {
    let index = ((sorted.len() - 1) as f64 * percentile).round() as usize;
    sorted[index]
}

fn main() {
    let open_baseline = begin_peak_window();
    let open_start = Instant::now();
    let geocoder = Geocoder::new();
    let open_us = open_start.elapsed().as_micros();
    let open_peak_heap_bytes = peak_delta(open_baseline);
    let metadata = geocoder.metadata();
    let prefix_queries = [
        "taj", "farm tom", "syd", "tok", "muni", "flor", "paris", "hallst",
    ];
    let mut timings = Vec::with_capacity(prefix_queries.len() * 200);
    let query_baseline = begin_peak_window();
    for _ in 0..200 {
        for query in prefix_queries {
            let started = Instant::now();
            let results = geocoder.search(query).expect("benchmark search");
            timings.push(started.elapsed().as_nanos());
            std::hint::black_box(results);
        }
    }
    let query_peak_heap_bytes = peak_delta(query_baseline);
    timings.sort_unstable();
    let samples = timings.len();
    let mut fuzzy_timings = Vec::with_capacity(100);
    for _ in 0..100 {
        let started = Instant::now();
        std::hint::black_box(geocoder.search("sydny").expect("fuzzy benchmark"));
        fuzzy_timings.push(started.elapsed().as_nanos());
    }
    fuzzy_timings.sort_unstable();
    let report = serde_json::json!({
        "artifact": {
            "dataset_date": metadata.dataset_date,
            "source": metadata.source,
            "representative": metadata.representative,
            "records": metadata.record_count,
            "aliases": metadata.alias_count,
            "installed_bytes": metadata.installed_bytes
        },
        "open": {
            "microseconds": open_us,
            "peak_heap_bytes": open_peak_heap_bytes,
            "index_bytes_borrowed_in_place": metadata.installed_bytes
        },
        "prefix_query": {
            "samples": samples,
            "p50_microseconds": percentile(&timings, 0.50) as f64 / 1000.0,
            "p95_microseconds": percentile(&timings, 0.95) as f64 / 1000.0,
            "p99_microseconds": percentile(&timings, 0.99) as f64 / 1000.0,
            "max_microseconds": timings.last().copied().unwrap_or_default() as f64 / 1000.0,
            "peak_heap_bytes": query_peak_heap_bytes
        },
        "fuzzy_query": {
            "samples": fuzzy_timings.len(),
            "p50_microseconds": percentile(&fuzzy_timings, 0.50) as f64 / 1000.0,
            "p95_microseconds": percentile(&fuzzy_timings, 0.95) as f64 / 1000.0,
            "max_microseconds": fuzzy_timings.last().copied().unwrap_or_default() as f64 / 1000.0
        }
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
