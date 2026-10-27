//! Native codec microbenchmark over real vearth cache payloads.
//!
//! Usage: `cargo run --release -p vearth-data --bin native-codec-bench --
//! LABEL CHUNK_DB SOURCE_DB BLUE_MARBLE_JPEG [SAMPLES]`

use rusqlite::{Connection, OpenFlags};
use std::hint::black_box;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

const TERRARIUM_SAMPLE_TILES: usize = 128;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments.len() < 5 {
        eprintln!("usage: native-codec-bench LABEL CHUNK_DB SOURCE_DB BLUE_MARBLE_JPEG [SAMPLES]");
        std::process::exit(2);
    }
    let label = &arguments[1];
    let samples = arguments
        .get(5)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(15)
        .max(3);

    let chunks = load_chunk_payloads(Path::new(&arguments[2]))?;
    let terrarium = load_terrarium_payloads(Path::new(&arguments[3]))?;
    let jpeg = std::fs::read(&arguments[4])?;
    if chunks.is_empty() || terrarium.is_empty() || jpeg.is_empty() {
        return Err("benchmark inputs must be nonempty".into());
    }

    let gzip = bench_gzip(&chunks, samples)?;
    let png = bench_terrarium(&terrarium, samples)?;
    let jpeg = bench_jpeg(&jpeg, samples)?;
    let report = serde_json::json!({
        "label": label,
        "target": {
            "arch": std::env::consts::ARCH,
            "os": std::env::consts::OS,
        },
        "samples": samples,
        "gzip_chunk_decode": gzip,
        "terrarium_png_decode": png,
        "blue_marble_jpeg_decode": jpeg,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn read_only(path: &Path) -> rusqlite::Result<Connection> {
    let uri = format!("file:{}?mode=ro&immutable=1", path.display());
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )
}

fn load_chunk_payloads(path: &Path) -> rusqlite::Result<Vec<Vec<u8>>> {
    let connection = read_only(path)?;
    let mut statement =
        connection.prepare("SELECT payload FROM chunks ORDER BY face, level, i, j")?;
    statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()
}

fn load_terrarium_payloads(path: &Path) -> rusqlite::Result<Vec<Vec<u8>>> {
    let connection = read_only(path)?;
    let mut statement = connection.prepare(
        "SELECT payload
         FROM source_tiles
         WHERE namespace_id = (
             SELECT id FROM source_namespaces WHERE provider = 'terrarium' LIMIT 1
         )
           AND state = 1
         ORDER BY z, x, y
         LIMIT ?1",
    )?;
    statement
        .query_map([TERRARIUM_SAMPLE_TILES as u32], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()
}

fn bench_gzip(
    payloads: &[Vec<u8>],
    samples: usize,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let run = || -> std::io::Result<usize> {
        let mut decoded_bytes = 0usize;
        for payload in payloads {
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(payload.as_slice()).read_to_end(&mut decoded)?;
            decoded_bytes += black_box(decoded).len();
        }
        Ok(decoded_bytes)
    };
    let decoded_bytes = run()?;
    let timing = measure(samples, run)?;
    Ok(codec_report(
        timing,
        payloads.len(),
        payloads.iter().map(Vec::len).sum(),
        decoded_bytes,
    ))
}

fn bench_terrarium(
    payloads: &[Vec<u8>],
    samples: usize,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let run = || -> Result<usize, vearth_data::DataError> {
        let mut decoded_bytes = 0usize;
        for payload in payloads {
            let tile = vearth_data::terrarium::decode_terrarium(payload)?;
            decoded_bytes += std::mem::size_of_val(tile.heights());
            black_box(tile);
        }
        Ok(decoded_bytes)
    };
    let decoded_bytes = run()?;
    let timing = measure(samples, run)?;
    Ok(codec_report(
        timing,
        payloads.len(),
        payloads.iter().map(Vec::len).sum(),
        decoded_bytes,
    ))
}

fn bench_jpeg(
    payload: &[u8],
    samples: usize,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let run = || {
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(payload), options);
        decoder.decode().map(|decoded| black_box(decoded).len())
    };
    let decoded_bytes = run()?;
    let timing = measure(samples, run)?;
    Ok(codec_report(timing, 1, payload.len(), decoded_bytes))
}

#[derive(Clone, Copy)]
struct Timing {
    median_ms: f64,
    p95_ms: f64,
}

fn measure<E>(samples: usize, mut run: impl FnMut() -> Result<usize, E>) -> Result<Timing, E> {
    let mut durations = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        black_box(run()?);
        durations.push(started.elapsed().as_secs_f64() * 1e3);
    }
    durations.sort_by(f64::total_cmp);
    Ok(Timing {
        median_ms: percentile(&durations, 50),
        p95_ms: percentile(&durations, 95),
    })
}

fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    sorted[(sorted.len() - 1) * percentile / 100]
}

fn codec_report(
    timing: Timing,
    items: usize,
    encoded_bytes: usize,
    decoded_bytes: usize,
) -> serde_json::Value {
    serde_json::json!({
        "items_per_sample": items,
        "encoded_bytes_per_sample": encoded_bytes,
        "decoded_bytes_per_sample": decoded_bytes,
        "median_ms": timing.median_ms,
        "p95_ms": timing.p95_ms,
        "decoded_mib_per_second": decoded_bytes as f64
            / (1024.0 * 1024.0)
            / (timing.median_ms / 1e3),
    })
}
