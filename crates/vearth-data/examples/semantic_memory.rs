//! Measure decoded and normalized resident heap estimates for a cached vector
//! source revision. This intentionally holds only one tile at a time.

use rusqlite::{Connection, params};
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vearth_data::mvt;
use vearth_data::protomaps_semantics::SemanticTile;

const SCAN_PASSES: usize = 32;

#[derive(Clone, Copy)]
struct Sample {
    raw: usize,
    semantic: usize,
    total: usize,
}

fn percentile(values: &mut [usize], numerator: usize, denominator: usize) -> usize {
    values.sort_unstable();
    let index = (values.len().saturating_sub(1) * numerator) / denominator;
    values[index]
}

fn print_measure(label: &str, samples: &[Sample], select: fn(&Sample) -> usize) {
    let values: Vec<_> = samples.iter().map(select).collect();
    let sum: usize = values.iter().sum();
    println!(
        "{label}: avg={} p50={} p95={} max={}",
        sum / values.len(),
        percentile(&mut values.clone(), 50, 100),
        percentile(&mut values.clone(), 95, 100),
        values.iter().copied().max().unwrap()
    );
}

fn raw_property_checksum(tile: &mvt::VectorTile) -> usize {
    tile.layers
        .iter()
        .map(|layer| {
            layer
                .features
                .iter()
                .map(|feature| {
                    ["kind", "kind_detail", "min_zoom", "name:en", "name"]
                        .iter()
                        .filter_map(|key| feature.get(layer, key))
                        .count()
                })
                .sum::<usize>()
        })
        .sum()
}

fn semantic_fact_checksum(tile: &SemanticTile) -> usize {
    tile.semantic_layers()
        .iter()
        .map(|layer| {
            layer
                .features
                .iter()
                .map(|feature| {
                    let (kind, detail) = feature.kind_and_detail();
                    usize::from(kind.is_some())
                        + usize::from(detail.is_some())
                        + usize::from(feature.common.min_zoom.is_some())
                        + feature
                            .common
                            .names
                            .iter()
                            .filter(|name| {
                                matches!(
                                    tile.source_name_key(layer, **name),
                                    Some("name:en" | "name")
                                )
                            })
                            .count()
                })
                .sum::<usize>()
        })
        .sum()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let database = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cache/source_tiles.sqlite3"));
    let revision = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .unwrap_or_else(|| "20260807".to_string());
    let database_uri = format!("file:{}?immutable=1", database.display());
    let connection = Connection::open_with_flags(
        database_uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let mut statement = connection.prepare(
        "SELECT t.z, t.x, t.y, t.payload
         FROM source_tiles t
         JOIN source_namespaces n ON n.id = t.namespace_id
         WHERE n.provider = 'vectors' AND n.revision = ?1 AND t.state = 1
         ORDER BY t.z, t.x, t.y",
    )?;
    let mut rows = statement.query(params![revision])?;
    let mut samples = Vec::new();
    let mut payload_bytes = 0usize;
    let mut decode_time = Duration::ZERO;
    let mut normalize_time = Duration::ZERO;
    let mut raw_scan_time = Duration::ZERO;
    let mut semantic_scan_time = Duration::ZERO;
    let mut checksums = (0usize, 0usize);
    while let Some(row) = rows.next()? {
        let z = row.get::<_, u8>(0)?;
        let x = row.get::<_, u32>(1)?;
        let y = row.get::<_, u32>(2)?;
        let payload = row.get::<_, Vec<u8>>(3)?;
        payload_bytes += payload.len();
        let start = Instant::now();
        let decoded = mvt::decode_tile(&payload)?;
        decode_time += start.elapsed();
        let start = Instant::now();
        for _ in 0..SCAN_PASSES {
            checksums.0 = checksums
                .0
                .wrapping_add(black_box(raw_property_checksum(&decoded)));
        }
        raw_scan_time += start.elapsed();
        let start = Instant::now();
        let tile = SemanticTile::normalize_at(decoded, (z, x, y));
        normalize_time += start.elapsed();
        let start = Instant::now();
        for _ in 0..SCAN_PASSES {
            checksums.1 = checksums
                .1
                .wrapping_add(black_box(semantic_fact_checksum(&tile)));
        }
        semantic_scan_time += start.elapsed();
        samples.push(Sample {
            raw: tile.raw_heap_bytes(),
            semantic: tile.semantic_heap_bytes(),
            total: tile.estimated_heap_bytes(),
        });
    }
    if samples.is_empty() {
        return Err("no present vector tiles matched the requested revision".into());
    }

    println!(
        "revision={revision} tiles={} payload_bytes={payload_bytes}",
        samples.len()
    );
    print_measure("raw_heap", &samples, |sample| sample.raw);
    print_measure("semantic_heap", &samples, |sample| sample.semantic);
    print_measure("total_heap", &samples, |sample| sample.total);
    let mut ratios: Vec<_> = samples
        .iter()
        .map(|sample| sample.semantic as f64 / sample.raw.max(1) as f64)
        .collect();
    ratios.sort_by(f64::total_cmp);
    let ratio_at = |numerator| ratios[(ratios.len() - 1) * numerator / 100];
    println!(
        "semantic_raw_ratio: p50={:.3} p95={:.3} max={:.3}",
        ratio_at(50),
        ratio_at(95),
        ratios.last().copied().unwrap()
    );
    let max_total = samples.iter().map(|sample| sample.total).max().unwrap();
    let sum_total: usize = samples.iter().map(|sample| sample.total).sum();
    println!(
        "projected_256_entries: average={} unconstrained_worst_case={} byte_limit={}",
        sum_total / samples.len() * 256,
        max_total * 256,
        384 * 1024 * 1024,
    );
    let micros_per_tile =
        |duration: Duration| duration.as_secs_f64() * 1_000_000.0 / samples.len() as f64;
    println!(
        "cpu_per_tile_us: decode={:.2} normalize={:.2}",
        micros_per_tile(decode_time),
        micros_per_tile(normalize_time),
    );
    println!(
        "property_scan_{SCAN_PASSES}x_ms: raw={:.3} semantic={:.3} speedup={:.2} checksums={}:{}",
        raw_scan_time.as_secs_f64() * 1_000.0,
        semantic_scan_time.as_secs_f64() * 1_000.0,
        raw_scan_time.as_secs_f64() / semantic_scan_time.as_secs_f64().max(f64::EPSILON),
        checksums.0,
        checksums.1,
    );
    Ok(())
}
