//! Local microbenchmark for the legacy-file and indexed generated-chunk stores.
//!
//! Usage:
//!   VEARTH_CHUNK_STORE=files cargo run --release -p vearth-app \
//!     --bin chunk-cache-bench -- CACHE_VERSION_DIRECTORY 10000
//!   VEARTH_CHUNK_STORE=sqlite cargo run --release -p vearth-app \
//!     --bin chunk-cache-bench -- CACHE_VERSION_DIRECTORY 10000

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;
use vearth_app::chunk_cache::ChunkCache;
use vearth_core::cell::CellId;
use vearth_core::cubesphere::Face;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(source_arg) = args.get(1) else {
        eprintln!("usage: chunk-cache-bench CACHE_VERSION_DIRECTORY [ENTRY_LIMIT]");
        std::process::exit(2);
    };
    let source = PathBuf::from(source_arg);
    let limit = args
        .get(2)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10_000);
    let mode = std::env::var("VEARTH_CHUNK_STORE").unwrap_or_else(|_| "sqlite".into());
    let tag = source
        .file_name()
        .and_then(|name| name.to_str())
        .expect("source directory has a UTF-8 version tag");
    let prefix = format!(
        "g{}-s{}-",
        vearth_gen::GEN_VERSION,
        vearth_app::chunk_cache::SURFACE_CODEC_VERSION
    );
    let dataset = tag
        .strip_prefix(&prefix)
        .expect("source directory must match the current GEN_VERSION");

    let mut source_files = Vec::new();
    collect_vxs(&source, &mut source_files);
    source_files.sort();
    if source_files.len() > limit {
        let total = source_files.len();
        source_files = (0..limit)
            .map(|index| source_files[index * total / limit].clone())
            .collect();
    }
    assert!(!source_files.is_empty(), "no .vxs files found");

    let bench_root = std::env::temp_dir().join(format!(
        "vearth-chunk-bench-{}-{}",
        mode,
        std::process::id()
    ));
    let target_version = bench_root.join("surfaces").join(tag);
    let mut cells = Vec::with_capacity(source_files.len());
    let mut logical_bytes = 0u64;
    let mut legacy_allocated_bytes = 0u64;
    for source_file in &source_files {
        let relative = source_file.strip_prefix(&source).unwrap();
        let target = target_version.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(source_file, &target).unwrap();
        let metadata = target.metadata().unwrap();
        logical_bytes += metadata.len();
        legacy_allocated_bytes += allocated_bytes(&metadata);
        cells.push(parse_cell(relative).expect("canonical cache path"));
    }

    let disk_budget = logical_bytes.saturating_mul(2).max(64 * 1024 * 1024);
    let started = Instant::now();
    let cache = ChunkCache::with_budgets(bench_root.to_str().unwrap(), dataset, 0, disk_budget);
    let startup_ms = started.elapsed().as_secs_f64() * 1e3;

    let migration_started = Instant::now();
    for cell in &cells {
        black_box(cache.load(*cell).expect("valid source payload"));
    }
    cache.flush();
    let migration_ms = migration_started.elapsed().as_secs_f64() * 1e3;
    let migration_stats = cache.stats();
    drop(cache);

    let reopened_at = Instant::now();
    let cache = ChunkCache::with_budgets(bench_root.to_str().unwrap(), dataset, 0, disk_budget);
    let reopen_ms = reopened_at.elapsed().as_secs_f64() * 1e3;
    let mut latencies_ns = Vec::with_capacity(cells.len());
    // An LCG walk avoids measuring only one directory/B-tree neighborhood and
    // is deterministic without another benchmark dependency.
    let mut state = 0x4d59_5df4_d0f3_3173u64;
    for _ in 0..cells.len() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let cell = cells[(state as usize) % cells.len()];
        let began = Instant::now();
        black_box(cache.load(cell).expect("warm cache hit"));
        latencies_ns.push(began.elapsed().as_nanos() as u64);
    }
    cache.flush();
    latencies_ns.sort_unstable();
    let warm_stats = cache.stats();
    let write_started = Instant::now();
    for cell in &cells {
        let column = cache.load(*cell).expect("write benchmark source hit");
        cache.store(&column);
    }
    cache.flush();
    let write_ms = write_started.elapsed().as_secs_f64() * 1e3;
    let stats = cache.stats();
    let (current_rss_bytes, peak_rss_bytes) = memory_bytes();

    let report = serde_json::json!({
        "mode": mode,
        "sample_entries": cells.len(),
        "payload_bytes": logical_bytes,
        "legacy_allocated_bytes": legacy_allocated_bytes,
        "indexed_physical_bytes": stats.physical_disk_bytes,
        "initial_startup_ms": startup_ms,
        "reopen_ms": reopen_ms,
        "migration_or_file_scan_ms": migration_ms,
        "migration_entries_per_second": cells.len() as f64 / (migration_ms / 1e3),
        "load_encode_write_ms": write_ms,
        "write_entries_per_second": cells.len() as f64 / (write_ms / 1e3),
        "hit_us": {
            "p50": percentile(&latencies_ns, 50) as f64 / 1e3,
            "p95": percentile(&latencies_ns, 95) as f64 / 1e3,
            "p99": percentile(&latencies_ns, 99) as f64 / 1e3,
            "max": latencies_ns.last().copied().unwrap_or(0) as f64 / 1e3,
        },
        "first_pass": {
            "indexed_hits": migration_stats.indexed_hits,
            "legacy_hits": migration_stats.legacy_hits,
            "legacy_imports": migration_stats.legacy_imports,
            "writes": migration_stats.writes,
            "dropped_writes": migration_stats.dropped_writes,
        },
        "warm_pass": {
            "indexed_hits": warm_stats.indexed_hits,
            "legacy_hits": warm_stats.legacy_hits,
            "disk_entries": warm_stats.disk_entries,
            "logical_disk_bytes": warm_stats.logical_disk_bytes,
            "queued_writes": warm_stats.queued_writes,
        },
        "write_pass": {
            "writes": stats.writes.saturating_sub(warm_stats.writes),
            "dropped_writes": stats.dropped_writes.saturating_sub(warm_stats.dropped_writes),
        },
        "current_rss_bytes": current_rss_bytes,
        "peak_rss_bytes": peak_rss_bytes,
        "temporary_root": bench_root,
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

fn collect_vxs(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_vxs(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "vxs") {
            output.push(path);
        }
    }
}

fn parse_cell(relative: &Path) -> Option<CellId> {
    let mut components = relative.components();
    let face = components
        .next()?
        .as_os_str()
        .to_str()?
        .strip_prefix('f')?
        .parse()
        .ok()?;
    let level = components
        .next()?
        .as_os_str()
        .to_str()?
        .strip_prefix('l')?
        .parse()
        .ok()?;
    let stem = components
        .next()?
        .as_os_str()
        .to_str()?
        .strip_suffix(".vxs")?;
    let (i, j) = stem.split_once('_')?;
    Some(CellId {
        face: Face(face),
        level,
        i: i.parse().ok()?,
        j: j.parse().ok()?,
    })
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[(sorted.len() - 1) * percentile / 100]
}

#[cfg(unix)]
fn allocated_bytes(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn allocated_bytes(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

#[cfg(target_os = "macos")]
fn memory_bytes() -> (Option<u64>, Option<u64>) {
    #[repr(C)]
    struct TimeValue {
        seconds: i32,
        microseconds: i32,
    }
    #[repr(C)]
    struct MachTaskBasicInfo {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time: TimeValue,
        system_time: TimeValue,
        policy: i32,
        suspend_count: i32,
    }
    unsafe extern "C" {
        static mach_task_self_: u32;
        fn task_info(task: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    }
    let mut info = MachTaskBasicInfo {
        virtual_size: 0,
        resident_size: 0,
        resident_size_max: 0,
        user_time: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        system_time: TimeValue {
            seconds: 0,
            microseconds: 0,
        },
        policy: 0,
        suspend_count: 0,
    };
    let mut count = (std::mem::size_of::<MachTaskBasicInfo>() / std::mem::size_of::<u32>()) as u32;
    // SAFETY: task_info writes exactly `count` natural_t words to the correctly
    // sized C-layout structure for MACH_TASK_BASIC_INFO (flavor 20).
    let result = unsafe {
        task_info(
            mach_task_self_,
            20,
            (&mut info as *mut MachTaskBasicInfo).cast(),
            &mut count,
        )
    };
    if result == 0 {
        (Some(info.resident_size), Some(info.resident_size_max))
    } else {
        (None, None)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn memory_bytes() -> (Option<u64>, Option<u64>) {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok();
    let current = output
        .as_ref()
        .and_then(|output| std::str::from_utf8(&output.stdout).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|kib| kib * 1024);
    (current, None)
}

#[cfg(not(unix))]
fn memory_bytes() -> (Option<u64>, Option<u64>) {
    (None, None)
}
