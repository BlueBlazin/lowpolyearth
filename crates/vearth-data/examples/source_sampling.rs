use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::time::Instant;
use vearth_data::terrarium::TerrariumSource;
use vearth_data::{DataError, ElevationSource, TileFetcher};

struct FixtureFetcher(Vec<u8>);

impl TileFetcher for FixtureFetcher {
    fn fetch(&self, _: &str) -> Result<Vec<u8>, DataError> {
        Ok(self.0.clone())
    }
}

#[cfg(feature = "async-http")]
impl vearth_data::async_http::AsyncTileFetcher for FixtureFetcher {
    fn fetch<'a>(&'a self, _: &'a str) -> vearth_data::async_http::FetchFuture<'a> {
        Box::pin(std::future::ready(Ok(self.0.clone())))
    }
}

#[cfg(feature = "async-http")]
fn prepare(source: &impl vearth_data::async_prep::AsyncSourcePreparer) {
    use vearth_data::async_prep::{PrepareRequest, SourceFootprint};
    let request = PrepareRequest {
        terrain_tiles: Vec::new(),
        footprint: SourceFootprint {
            center_lat: 0.70001,
            center_lon: 0.20001,
            radius_rad: 1e-5,
            sample_hint_m: 10.0,
        },
        vector_bounds: None,
        night_building_bounds: None,
        priority: 1.0,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    assert!(runtime.block_on(source.prepare(&request)).complete());
}

fn fixture_png() -> Vec<u8> {
    let rgb: Vec<u8> = (0..256)
        .flat_map(|y| (0..256).flat_map(move |x| [128 + y as u8 / 4, x as u8, y as u8]))
        .collect();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 256, 256);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgb)
            .unwrap();
    }
    bytes
}

fn measure(
    iterations: usize,
    repeats: usize,
    query: &(impl Fn(f64, f64) -> u64 + Sync),
) -> Vec<serde_json::Value> {
    let points: Vec<_> = (0..4096)
        .map(|i| {
            (
                0.70001 + (i / 64) as f64 * 1e-7,
                0.20001 + (i % 64) as f64 * 1e-7,
            )
        })
        .collect();
    for &(lat, lon) in &points {
        black_box(query(lat, lon));
    }
    [1, 4, 8]
        .into_iter()
        .map(|threads| {
            let mut durations = Vec::new();
            let mut expected = None;
            for _ in 0..repeats {
                let (elapsed, checksum) = std::thread::scope(|scope| {
                    let start = Arc::new(Barrier::new(threads + 1));
                    let done = Arc::new(Barrier::new(threads + 1));
                    let workers: Vec<_> = (0..threads)
                        .map(|worker| {
                            let start = Arc::clone(&start);
                            let done = Arc::clone(&done);
                            let points = &points;
                            scope.spawn(move || {
                                start.wait();
                                let mut checksum = 0u64;
                                for i in 0..iterations {
                                    let &(lat, lon) = &points[(i + worker * 509) % points.len()];
                                    checksum = checksum.wrapping_add(black_box(query(lat, lon)));
                                }
                                done.wait();
                                checksum
                            })
                        })
                        .collect();
                    let began = Instant::now();
                    start.wait();
                    done.wait();
                    let elapsed = began.elapsed().as_secs_f64() * 1e3;
                    let checksum = workers
                        .into_iter()
                        .fold(0u64, |sum, worker| sum.wrapping_add(worker.join().unwrap()));
                    (elapsed, checksum)
                });
                assert_eq!(*expected.get_or_insert(checksum), checksum);
                durations.push(elapsed);
            }
            durations.sort_by(f64::total_cmp);
            let median_ms = durations[durations.len() / 2];
            serde_json::json!({
                "threads": threads,
                "samples": iterations * threads,
                "median_ms": median_ms,
                "million_samples_per_second": (iterations * threads) as f64 / median_ms / 1000.0,
                "checksum": format!("{:016x}", expected.unwrap()),
            })
        })
        .collect()
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let iterations = arguments
        .next()
        .map(|s| s.parse().unwrap())
        .unwrap_or(200_000);
    let repeats: usize = arguments.next().map(|s| s.parse().unwrap()).unwrap_or(5);
    assert!(iterations > 0 && repeats > 0);
    let fetcher = Arc::new(FixtureFetcher(fixture_png()));
    #[cfg(not(feature = "async-http"))]
    let terrain = TerrariumSource::new(fetcher, "fixture://terrain", None);
    #[cfg(feature = "async-http")]
    let terrain = TerrariumSource::new_async(fetcher, "fixture://terrain", None);
    #[cfg(feature = "async-http")]
    prepare(&terrain);
    let terrain = measure(iterations, repeats, &|lat, lon| {
        let sample = terrain.sample_elevation(lat, lon, 10.0);
        assert!(sample.complete);
        u64::from(sample.value.unwrap().to_bits())
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
        "iterations_per_thread": iterations,
        "repeats": repeats,
        "async_prepared": cfg!(feature = "async-http"),
            "terrarium": terrain,
        }))
        .unwrap()
    );
}
