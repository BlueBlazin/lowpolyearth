//! Offline preparation of application data and cache assets.
//!
//! The default tile downloader skips existing files. Pack subcommands
//! deterministically write their derived outputs.
//!
//! Usage:
//!   vearth-prep [--max-zoom 3] [--cache-dir cache]
//!   vearth-prep pack-terrain [--cache-dir cache]
//!   vearth-prep pack-bridges --osm region.osm.pbf [--osm another.osm]
//!       [--cache-dir cache] [--build-tag YYYYMMDD]
//!   vearth-prep pack-sky-data --hipparcos hip_main.dat
//!       --black-marble BlackMarble_2016_01deg.jpg
//!       --milky-way eso0932a.jpg
//!       [--output-dir crates/vearth-app/assets/sky]
//!   vearth-prep pack-surface --input-dir /path/to/monthly-blue-marble
//!       [--quality 64]
//!       [--output-dir crates/vearth-data/assets]
//!   vearth-prep pack-place-search --cities cities500.txt
//!       --country-info countryInfo.txt --admin1 admin1CodesASCII.txt
//!       [--all-countries allCountries.txt]
//!       [--alternate-names alternateNamesV2.txt]
//!       [--dataset-date YYYY-MM-DD]
//!   vearth-prep split-landmarks --output DIR
//!   vearth-prep shard-place-search --output DIR
//!       [--input crates/vearth-data/assets/place-search-v1.bin]
//!
//! The default mode downloads the Mapterhorn elevation pyramid through
//! `--max-zoom` into `<cache-dir>/mapterhorn`. `pack-terrain` then
//! deterministically builds the native application asset from it:
//!   native: z0..=3 at 512 px, 1 m quantization, ≤50 MiB
//! Global surface colour is already compiled into the app and is never
//! downloaded at runtime.

use rayon::prelude::*;
use std::cmp::Ordering as CmpOrdering;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use vearth_data::terrarium::DEFAULT_URL;
use vearth_data::{HttpFetcher, TileFetcher};

mod place_search;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let argv: Vec<String> = std::env::args().collect();
    if argv
        .get(1)
        .is_some_and(|argument| argument == "pack-terrain")
    {
        if let Err(error) = pack_terrain(&argv) {
            log::error!("terrain pack: {error}");
            std::process::exit(1);
        }
        return;
    }
    if argv
        .get(1)
        .is_some_and(|argument| argument == "pack-bridges")
    {
        if let Err(error) = pack_bridges(&argv) {
            log::error!("bridge pack: {error}");
            std::process::exit(1);
        }
        return;
    }
    if argv
        .get(1)
        .is_some_and(|argument| argument == "pack-sky-data")
    {
        if let Err(error) = pack_sky_data(&argv) {
            log::error!("sky data pack: {error}");
            std::process::exit(1);
        }
        return;
    }
    if argv
        .get(1)
        .is_some_and(|argument| argument == "pack-surface")
    {
        if let Err(error) = pack_surface(&argv) {
            log::error!("surface pack: {error}");
            std::process::exit(1);
        }
        return;
    }
    if argv
        .get(1)
        .is_some_and(|argument| argument == "pack-place-search")
    {
        if let Err(error) = place_search::pack(&argv) {
            log::error!("place search pack: {error}");
            std::process::exit(1);
        }
        return;
    }
    if argv
        .get(1)
        .is_some_and(|argument| argument == "split-landmarks")
    {
        if let Err(error) = split_landmarks(&argv) {
            log::error!("landmark split: {error}");
            std::process::exit(1);
        }
        return;
    }
    if argv
        .get(1)
        .is_some_and(|argument| argument == "shard-place-search")
    {
        if let Err(error) = place_search::shard(&argv) {
            log::error!("place search shards: {error}");
            std::process::exit(1);
        }
        return;
    }
    let flag = |name: &str, default: &str| -> String {
        argv.iter()
            .position(|a| a == name)
            .and_then(|i| argv.get(i + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let max_zoom: u8 = flag("--max-zoom", "3").parse().expect("zoom");
    let cache: PathBuf = flag("--cache-dir", "cache").into();
    let fetcher: Arc<dyn TileFetcher> = Arc::new(HttpFetcher::default());

    // Elevation tile pyramid z0..=max_zoom.
    let tile_dir = cache.join("mapterhorn");
    let jobs: Vec<(u8, u32, u32)> = (0..=max_zoom)
        .flat_map(|z| {
            let n = 1u32 << z;
            (0..n).flat_map(move |x| (0..n).map(move |y| (z, x, y)))
        })
        .collect();
    log::info!("elevation: {} tiles to z{max_zoom}", jobs.len());
    let (done, skipped, failed) = (
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(24)
        .build()
        .unwrap();
    pool.install(|| {
        jobs.par_iter().for_each(|&(z, x, y)| {
            let path = tile_dir.join(format!("{z}/{x}/{y}.webp"));
            let sea = path.with_extension("sea");
            if path.exists() || sea.exists() {
                skipped.fetch_add(1, Ordering::Relaxed);
                return;
            }
            match fetcher.fetch(&format!("{DEFAULT_URL}/{z}/{x}/{y}.webp")) {
                Ok(bytes) => {
                    let _ = std::fs::create_dir_all(path.parent().unwrap());
                    let _ = std::fs::write(&path, bytes);
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if n % 500 == 0 {
                        log::info!("  {n} downloaded…");
                    }
                }
                // The planet archive omits tiles with no land.
                Err(vearth_data::DataError::NotAvailable) => {
                    let _ = std::fs::create_dir_all(sea.parent().unwrap());
                    let _ = std::fs::write(&sea, []);
                    done.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    log::warn!("tile {z}/{x}/{y}: {e}");
                }
            }
        });
    });
    log::info!(
        "elevation: {} downloaded, {} already cached, {} failed",
        done.load(Ordering::Relaxed),
        skipped.load(Ordering::Relaxed),
        failed.load(Ordering::Relaxed)
    );
}

fn pack_surface(argv: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use image::codecs::jpeg::JpegEncoder;
    use image::imageops::FilterType;
    use image::{ExtendedColorType, RgbImage};
    use vearth_data::surface_pack::{
        COARSE_HEIGHT, COARSE_WIDTH, DETAIL_HEIGHT, DETAIL_WIDTH, EncodedSurfaceLevel, MONTHS,
        TILE_SIZE, encode_pack,
    };

    let value = |flag: &str| {
        argv.iter()
            .position(|argument| argument == flag)
            .and_then(|index| argv.get(index + 1))
            .cloned()
    };
    let input_dir = PathBuf::from(
        value("--input-dir").ok_or("pack-surface requires --input-dir with all 12 mosaics")?,
    );
    let output_dir =
        PathBuf::from(value("--output-dir").unwrap_or_else(|| "crates/vearth-data/assets".into()));
    let quality: u8 = value("--quality").unwrap_or_else(|| "64".into()).parse()?;
    if !(40..=95).contains(&quality) {
        return Err("--quality must be between 40 and 95".into());
    }

    let encode_level = |rgb: &RgbImage| -> Result<EncodedSurfaceLevel, Box<dyn std::error::Error>> {
        let (width, height) = rgb.dimensions();
        let tiles_x = (width as usize).div_ceil(TILE_SIZE);
        let tiles_y = (height as usize).div_ceil(TILE_SIZE);
        let rows: Result<Vec<Vec<Vec<u8>>>, image::ImageError> = (0..tiles_y)
            .into_par_iter()
            .map(|tile_y| {
                let mut row = Vec::with_capacity(tiles_x);
                for tile_x in 0..tiles_x {
                    let x = tile_x * TILE_SIZE;
                    let y = tile_y * TILE_SIZE;
                    let tile_width = TILE_SIZE.min(width as usize - x) as u32;
                    let tile_height = TILE_SIZE.min(height as usize - y) as u32;
                    let tile =
                        image::imageops::crop_imm(rgb, x as u32, y as u32, tile_width, tile_height)
                            .to_image();
                    let mut bytes = Vec::new();
                    JpegEncoder::new_with_quality(&mut bytes, quality).encode(
                        tile.as_raw(),
                        tile_width,
                        tile_height,
                        ExtendedColorType::Rgb8,
                    )?;
                    row.push(bytes);
                }
                Ok(row)
            })
            .collect();
        Ok(EncodedSurfaceLevel {
            width,
            height,
            tiles: rows?.into_iter().flatten().collect(),
        })
    };

    std::fs::create_dir_all(&output_dir)?;
    for month in 1..=MONTHS {
        let input = input_dir.join(format!("world.2004{month:02}.3x21600x10800.jpg"));
        log::info!("month {month:02}: decoding {}", input.display());
        let mut reader = image::ImageReader::open(&input)?.with_guessed_format()?;
        // Each trusted NASA source expands to ~667 MiB before it is split into
        // lazy tiles. `image` defaults to a 512 MiB decode guard, appropriate
        // for interactive uploads but too small for this offline pack job.
        reader.no_limits();
        let detail = reader.decode()?.into_rgb8();
        if detail.dimensions() != (DETAIL_WIDTH as u32, DETAIL_HEIGHT as u32) {
            return Err(format!(
                "{} is {}x{}, expected {DETAIL_WIDTH}x{DETAIL_HEIGHT}",
                input.display(),
                detail.width(),
                detail.height()
            )
            .into());
        }
        let coarse = image::imageops::resize(
            &detail,
            COARSE_WIDTH as u32,
            COARSE_HEIGHT as u32,
            FilterType::Lanczos3,
        );
        log::info!("month {month:02}: encoding coarse level");
        let coarse = encode_level(&coarse)?;
        log::info!("month {month:02}: encoding detail level");
        let detail = encode_level(&detail)?;
        let pack = encode_pack(TILE_SIZE as u16, vec![coarse, detail])?;
        let output = output_dir.join(format!("surface-{month:02}.vsp"));
        std::fs::write(&output, &pack)?;
        log::info!(
            "month {month:02}: wrote {} ({:.2} MiB, quality {quality})",
            output.display(),
            pack.len() as f64 / 1_048_576.0
        );
    }
    Ok(())
}

const STAR_CATALOG_MAGIC: &[u8; 4] = b"VSTR";
const NIGHT_LIGHT_MAGIC: &[u8; 4] = b"VNLT";
const MILKY_WAY_MAGIC: &[u8; 4] = b"VMWY";
const SKY_ASSET_VERSION: u32 = 1;
const STAR_LIMITING_MAGNITUDE: f32 = 7.8;
const NIGHT_LIGHT_WIDTH: u32 = 2048;
const NIGHT_LIGHT_HEIGHT: u32 = 1024;
const MILKY_WAY_WIDTH: u32 = 1024;
const MILKY_WAY_HEIGHT: u32 = 512;

#[derive(Clone, Copy)]
struct PackedStar {
    direction_icrs: [f32; 3],
    magnitude: f32,
    bv: f32,
}

fn pack_sky_data(argv: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let flag = |name: &str| -> Option<PathBuf> {
        argv.iter()
            .position(|argument| argument == name)
            .and_then(|index| argv.get(index + 1))
            .map(PathBuf::from)
    };
    let hipparcos = flag("--hipparcos").ok_or("pack-sky-data requires --hipparcos hip_main.dat")?;
    let black_marble =
        flag("--black-marble").ok_or("pack-sky-data requires --black-marble image")?;
    let milky_way = flag("--milky-way").ok_or("pack-sky-data requires --milky-way image")?;
    let output_dir =
        flag("--output-dir").unwrap_or_else(|| PathBuf::from("crates/vearth-app/assets/sky"));
    std::fs::create_dir_all(&output_dir)?;

    let stars = read_bright_stars(&hipparcos)?;
    let star_path = output_dir.join("hipparcos_mag78.bin");
    write_star_catalog(&star_path, &stars)?;
    log::info!(
        "stars: wrote {} Hipparcos entries (V <= {:.1}) to {}",
        stars.len(),
        STAR_LIMITING_MAGNITUDE,
        star_path.display()
    );

    let light_path = output_dir.join("black_marble_2048x1024.bin");
    write_night_lights(&black_marble, &light_path)?;
    log::info!(
        "night lights: wrote {}x{} emission proxy to {}",
        NIGHT_LIGHT_WIDTH,
        NIGHT_LIGHT_HEIGHT,
        light_path.display()
    );

    let milky_way_path = output_dir.join("milky_way_1024x512.bin");
    write_milky_way(&milky_way, &milky_way_path)?;
    log::info!(
        "Milky Way: wrote {}x{} diffuse all-sky texture to {}",
        MILKY_WAY_WIDTH,
        MILKY_WAY_HEIGHT,
        milky_way_path.display()
    );
    Ok(())
}

fn read_bright_stars(path: &Path) -> Result<Vec<PackedStar>, Box<dyn std::error::Error>> {
    let file = BufReader::new(std::fs::File::open(path)?);
    let mut stars = Vec::new();
    for line in file.lines() {
        let line = line?;
        let fields: Vec<&str> = line.split('|').collect();
        if fields.len() < 38 {
            continue;
        }
        let Some(magnitude) = parse_optional_f32(fields[5]) else {
            continue;
        };
        if magnitude > STAR_LIMITING_MAGNITUDE {
            continue;
        }
        let Some(ra_deg) = parse_optional_f64(fields[8]) else {
            continue;
        };
        let Some(dec_deg) = parse_optional_f64(fields[9]) else {
            continue;
        };
        let bv = parse_optional_f32(fields[37])
            .unwrap_or(0.65)
            .clamp(-0.4, 2.0);
        let ra = ra_deg.to_radians();
        let dec = dec_deg.to_radians();
        let (sin_ra, cos_ra) = ra.sin_cos();
        let (sin_dec, cos_dec) = dec.sin_cos();
        stars.push(PackedStar {
            direction_icrs: [
                (cos_dec * cos_ra) as f32,
                (cos_dec * sin_ra) as f32,
                sin_dec as f32,
            ],
            magnitude,
            bv,
        });
    }
    stars.sort_by(|a, b| {
        a.magnitude
            .partial_cmp(&b.magnitude)
            .unwrap_or(CmpOrdering::Equal)
    });
    Ok(stars)
}

fn parse_optional_f32(value: &str) -> Option<f32> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.parse().ok()).flatten()
}

fn parse_optional_f64(value: &str) -> Option<f64> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.parse().ok()).flatten()
}

fn write_star_catalog(path: &Path, stars: &[PackedStar]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(STAR_CATALOG_MAGIC)?;
    file.write_all(&SKY_ASSET_VERSION.to_le_bytes())?;
    file.write_all(&(stars.len() as u32).to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    for star in stars {
        for value in star
            .direction_icrs
            .into_iter()
            .chain([star.magnitude, star.bv])
        {
            file.write_all(&value.to_le_bytes())?;
        }
    }
    Ok(())
}

fn write_night_lights(input: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let rgb = image::open(input)?.into_rgb8();
    let mut emission = image::GrayImage::new(rgb.width(), rgb.height());
    for (x, y, pixel) in rgb.enumerate_pixels() {
        let r = f32::from(pixel[0]) / 255.0;
        let g = f32::from(pixel[1]) / 255.0;
        let b = f32::from(pixel[2]) / 255.0;
        // NASA's presentation map includes a dim blue land/ice layer in
        // addition to city lights. Isolate neutral/warm emission before
        // downsampling so Greenland and Antarctica do not become cities.
        let upward_emission = ((r + g) * 0.5 - 0.75 * b - 0.02).max(0.0);
        let compressed = 1.0 - (-8.0 * upward_emission).exp();
        emission.put_pixel(x, y, image::Luma([(compressed * 255.0).round() as u8]));
    }
    let downsampled = image::imageops::resize(
        &emission,
        NIGHT_LIGHT_WIDTH,
        NIGHT_LIGHT_HEIGHT,
        image::imageops::FilterType::Gaussian,
    );
    let mut file = std::fs::File::create(output)?;
    file.write_all(NIGHT_LIGHT_MAGIC)?;
    file.write_all(&SKY_ASSET_VERSION.to_le_bytes())?;
    file.write_all(&NIGHT_LIGHT_WIDTH.to_le_bytes())?;
    file.write_all(&NIGHT_LIGHT_HEIGHT.to_le_bytes())?;
    file.write_all(downsampled.as_raw())?;
    Ok(())
}

fn write_milky_way(input: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let rgba = image::open(input)?.into_rgba8();
    let downsampled = image::imageops::resize(
        &rgba,
        MILKY_WAY_WIDTH,
        MILKY_WAY_HEIGHT,
        image::imageops::FilterType::Gaussian,
    );
    // The source panorama includes resolved photographic stars. A small blur
    // retains dust lanes and the Galactic bulge while turning those points
    // into a genuinely diffuse layer beneath the Hipparcos star pass.
    let mut diffuse = image::imageops::blur(&downsampled, 1.15);
    for pixel in diffuse.pixels_mut() {
        pixel[3] = 255;
    }

    let mut file = std::fs::File::create(output)?;
    file.write_all(MILKY_WAY_MAGIC)?;
    file.write_all(&SKY_ASSET_VERSION.to_le_bytes())?;
    file.write_all(&MILKY_WAY_WIDTH.to_le_bytes())?;
    file.write_all(&MILKY_WAY_HEIGHT.to_le_bytes())?;
    file.write_all(diffuse.as_raw())?;
    Ok(())
}

fn pack_bridges(argv: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let flag = |name: &str| -> Option<String> {
        argv.iter()
            .position(|argument| argument == name)
            .and_then(|index| argv.get(index + 1))
            .cloned()
    };
    let cache = PathBuf::from(flag("--cache-dir").unwrap_or_else(|| "cache".to_string()));
    let vectors = cache.join("vectors");
    let build_tag = flag("--build-tag").unwrap_or_else(|| {
        std::fs::read_to_string(vectors.join("pinned_build.txt"))
            .unwrap_or_default()
            .trim()
            .to_string()
    });
    if build_tag.is_empty() {
        return Err("no --build-tag and cache/vectors/pinned_build.txt is empty".into());
    }
    let tile_root = vectors.join(&build_tag);
    let osm_inputs: Vec<PathBuf> = argv
        .iter()
        .enumerate()
        .filter(|(_, argument)| argument.as_str() == "--osm")
        .filter_map(|(index, _)| argv.get(index + 1))
        .map(PathBuf::from)
        .collect();
    if osm_inputs.is_empty() {
        return Err("pack-bridges requires at least one raw OSM .pbf or XML --osm file".into());
    }
    let output = flag("--output")
        .map(PathBuf::from)
        .unwrap_or_else(|| tile_root.join("bridges.vbp"));
    log::info!(
        "bridges: resolving stable OSM objects from {} input(s); cached z14 is water-mask only",
        osm_inputs.len()
    );
    let summary = vearth_data::bridge_pack::build_osm_catalog(&tile_root, &osm_inputs, &output)?;
    log::info!(
        "bridges: wrote {} physical bridges from {} raw OSM input(s) to {} ({:.2} MiB)",
        summary.bridges,
        summary.inputs,
        output.display(),
        summary.bytes as f64 / 1_048_576.0
    );
    Ok(())
}

fn pack_terrain(argv: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use vearth_data::packed_terrain::{
        NATIVE_BUDGET_BYTES, NATIVE_MAX_ZOOM, NATIVE_QUANTIZATION_M,
    };

    let flag = |name: &str, default: &str| -> String {
        argv.iter()
            .position(|argument| argument == name)
            .and_then(|index| argv.get(index + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let tile_dir = PathBuf::from(flag("--cache-dir", "cache")).join("mapterhorn");
    let native_output = PathBuf::from(flag(
        "--native-output",
        "crates/vearth-data/assets/terrain-native.vtp",
    ));
    build_pack(
        "native",
        &tile_dir,
        &native_output,
        NATIVE_MAX_ZOOM,
        NATIVE_QUANTIZATION_M,
        NATIVE_BUDGET_BYTES,
        None,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_pack(
    label: &str,
    tile_dir: &Path,
    output: &Path,
    max_zoom: u8,
    quantization_m: u16,
    budget_bytes: usize,
    target_bytes: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    use vearth_data::packed_terrain;
    use vearth_data::terrarium::decode_terrarium;

    let tile_count: usize = (0..=max_zoom).map(|z| 1usize << (2 * z)).sum();
    log::info!("{label}: encoding {tile_count} tiles through z{max_zoom} at {quantization_m} m");
    let mut tiles = Vec::with_capacity(tile_count);
    for z in 0..=max_zoom {
        let n = 1u32 << z;
        for x in 0..n {
            for y in 0..n {
                let path = tile_dir.join(format!("{z}/{x}/{y}.webp"));
                let decoded = if !path.exists() && path.with_extension("sea").exists() {
                    vearth_data::terrarium::sea_tile()
                } else {
                    let webp = std::fs::read(&path).map_err(|error| {
                        format!(
                            "missing source tile {} ({error}); run vearth-prep --max-zoom {max_zoom}",
                            path.display()
                        )
                    })?;
                    decode_terrarium(&webp)?
                };
                tiles.push(packed_terrain::encode_tile(
                    z,
                    x,
                    y,
                    decoded.heights(),
                    quantization_m,
                )?);
            }
        }
    }

    let bytes = packed_terrain::encode_pack(quantization_m, tiles)?;
    if bytes.len() > budget_bytes {
        return Err(format!(
            "{label} terrain pack is {:.2} MiB, over the {:.0} MiB budget",
            bytes.len() as f64 / 1_048_576.0,
            budget_bytes as f64 / 1_048_576.0
        )
        .into());
    }
    if let Some(target) = target_bytes
        && bytes.len() > target
    {
        log::warn!(
            "{label} terrain pack is {:.2} MiB (within budget, above {:.0} MiB target)",
            bytes.len() as f64 / 1_048_576.0,
            target as f64 / 1_048_576.0
        );
    }

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = output.with_extension("vtp.tmp");
    std::fs::write(&temporary, &bytes)?;
    std::fs::rename(&temporary, output)?;
    log::info!(
        "{label}: wrote {} ({:.2} MiB, {:.1}% of budget)",
        output.display(),
        bytes.len() as f64 / 1_048_576.0,
        bytes.len() as f64 / budget_bytes as f64 * 100.0
    );
    Ok(())
}

/// Writes every landmark's levels of detail as separate files,
/// `<output>/<name>/<lod>.bin` (`output` is the generated tree's `landmarks`), for the browser build to download on demand.
/// Directories are read in the app's lookup order, so a name bundled twice
/// resolves to the same asset.
fn split_landmarks(argv: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    const ROOT: &str = "crates/vearth-gen/assets/landmarks";
    const DIRECTORIES: [&str; 6] = [
        "island-surfaces",
        "monument-surfaces",
        "skyscraper-surfaces",
        "heritage-surfaces",
        "city-surfaces",
        "surfaces",
    ];
    let output = argv
        .iter()
        .position(|argument| argument == "--output")
        .and_then(|index| argv.get(index + 1))
        .map(PathBuf::from)
        .ok_or("split-landmarks requires --output DIR")?;
    if output.exists() {
        std::fs::remove_dir_all(&output)?;
    }
    let mut written = std::collections::BTreeSet::new();
    let mut bytes = 0;
    for directory in DIRECTORIES {
        let mut files: Vec<_> = std::fs::read_dir(Path::new(ROOT).join(directory))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "lms"))
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or("landmark file name")?
                .to_owned();
            if !written.insert(name.clone()) {
                continue;
            }
            let levels = vearth_gen::landmarks::split_levels(&std::fs::read(&path)?)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            for (lod, level) in levels.iter().enumerate() {
                // Published as `generated/landmarks/...`; `output` is that
                // `landmarks` directory of the generated tree.
                let published = vearth_gen::landmarks::level_path(&name, lod);
                let relative = published
                    .strip_prefix("generated/landmarks/")
                    .ok_or("landmark level path")?;
                let target = output.join(relative);
                std::fs::create_dir_all(target.parent().unwrap())?;
                std::fs::write(target, level)?;
                bytes += level.len();
            }
        }
    }
    log::info!(
        "wrote {} landmarks' levels to {}: {:.1} MiB",
        written.len(),
        output.display(),
        bytes as f64 / 1048576.0
    );
    Ok(())
}
