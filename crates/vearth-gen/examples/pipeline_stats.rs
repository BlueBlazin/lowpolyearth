//! End-to-end pipeline smoke test with real data: pick a few famous places,
//! generate their cells at several LODs — including street level with the
//! full Tier-0 vector pipeline (buildings/roads/water/trees) — and print
//! chunk/mesh stats and timings.
//!
//! Run: cargo run -p vearth-gen --example pipeline_stats --release

use std::sync::Arc;
use std::time::{Duration, Instant};
use vearth_core::cell::CellId;
use vearth_data::pmtiles::{HttpRange, PmTiles};
use vearth_data::source_cache::{DEFAULT_SOURCE_CACHE_BUDGET, LegacyLayout, SourceTileStore};
use vearth_data::terrarium::{DEFAULT_URL, TerrariumSource};
use vearth_data::vectors::{ProtomapsSource, VectorSource, WatercourseKind, WatercourseOutputMode};
use vearth_gen::{Sources, generate_surface_cell, material, vector_query_bounds};

fn median_and_max(mut samples: Vec<Duration>) -> (Duration, Duration) {
    samples.sort_unstable();
    (samples[samples.len() / 2], *samples.last().unwrap())
}

fn main() {
    let pinned_build = std::env::var("VEARTH_PROTOMAPS_BUILD").ok();
    let arguments = std::env::args().collect::<Vec<_>>();
    let foundation_scope = arguments.iter().any(|argument| argument == "--foundation");
    let hydrography_scope = arguments.iter().any(|argument| argument == "--hydrography");
    let watercourse_mode = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix("--watercourses="))
        .map_or(
            WatercourseOutputMode::RiversAndCanals,
            |value| match value {
                "disabled" => WatercourseOutputMode::DisabledG96,
                "rivers-canals" => WatercourseOutputMode::RiversAndCanals,
                _ => panic!("--watercourses must be disabled or rivers-canals"),
            },
        );
    if hydrography_scope {
        println!("hydrography watercourse mode: {watercourse_mode:?}");
    }
    let source_store =
        SourceTileStore::open("cache/source_tiles.sqlite3", DEFAULT_SOURCE_CACHE_BUDGET).ok();
    let terrain_cache = source_store.as_ref().and_then(|store| {
        store
            .namespace(
                vearth_data::terrarium::CACHE_PROVIDER,
                vearth_data::terrarium::CACHE_REVISION,
                vearth_data::terrarium::CACHE_CONTENT_TYPE,
                None,
            )
            .ok()
    });
    let elevation = TerrariumSource::new(
        Arc::new(vearth_data::HttpFetcher::default()),
        DEFAULT_URL,
        terrain_cache,
    );
    let vectors: Option<ProtomapsSource> = {
        let tag = pinned_build.unwrap_or_else(|| vearth_data::vectors::PROTOMAPS_BUILD.into());
        let url = vearth_data::vectors::protomaps_build_url(&tag);
        println!("vectors: protomaps build {tag}");
        match PmTiles::open(Box::new(HttpRange::new(&url))) {
            Ok(pm) => {
                let vector_cache = source_store.as_ref().and_then(|store| {
                    store
                        .namespace(
                            "vectors",
                            &tag,
                            "application/vnd.mapbox-vector-tile",
                            Some(LegacyLayout::Vectors {
                                root: "cache/vectors".into(),
                                build_tag: tag.clone(),
                            }),
                        )
                        .ok()
                });
                match ProtomapsSource::new(pm, tag, vector_cache) {
                    Ok(mut source) => {
                        source = source.with_watercourse_output_mode(watercourse_mode);
                        source.set_semantic_observability(true);
                        Some(source)
                    }
                    Err(e) => {
                        println!("vectors incompatible ({e}); terrain only");
                        None
                    }
                }
            }
            Err(e) => {
                println!("vectors unavailable ({e}); terrain only");
                None
            }
        }
    };
    let sources = Sources {
        surface: None,
        elevation: &elevation,
        imagery: None,
        vectors: vectors.as_ref().map(|v| v as &dyn VectorSource),
    };
    assert!(
        !hydrography_scope || vectors.is_some(),
        "--hydrography requires the pinned compatible vector source"
    );

    let all_places = [
        ("Matterhorn", 45.9766, 7.6585),
        ("Everest", 27.9881, 86.925),
        ("Amsterdam", 52.3702, 4.8952),
        ("Manhattan", 40.7580, -73.9855),
        ("Mid-Pacific", 0.0, -150.0),
        ("Grand Canyon", 36.1, -112.1),
    ];
    let places: &[(&str, f64, f64)] = if hydrography_scope {
        &all_places[2..3]
    } else if foundation_scope {
        &all_places[2..]
    } else {
        &all_places
    };
    let levels: &[u8] = if hydrography_scope {
        &[14, 15, 17, 19]
    } else if foundation_scope {
        &[12, 15, 17]
    } else {
        &[6, 9, 12, 15, 17, 19]
    };
    let mut cold_vector_fills = 0u64;
    let mut warm_vector_fills = 0u64;
    let mut hydro_watercourse_records = 0usize;
    for &(name, lat_deg, lon_deg) in places {
        let dir = vearth_core::ellipsoid::geodetic_to_ecef(
            lat_deg.to_radians(),
            lon_deg.to_radians(),
            0.0,
        )
        .normalize();
        for &level in levels {
            let center_cell = CellId::from_dir(dir, level);
            let cell = if hydrography_scope {
                let vectors = vectors.as_ref().unwrap();
                let edge = (1u32 << level) - 1;
                let mut candidates = Vec::new();
                for dj in -1i64..=1 {
                    for di in -1i64..=1 {
                        let i = i64::from(center_cell.i) + di;
                        let j = i64::from(center_cell.j) + dj;
                        if i < 0 || j < 0 || i > i64::from(edge) || j > i64::from(edge) {
                            continue;
                        }
                        let candidate = CellId {
                            face: center_cell.face,
                            level,
                            i: i as u32,
                            j: j as u32,
                        };
                        let (lat0, lon0, half_e_m, half_n_m) = vector_query_bounds(candidate);
                        let pack = vectors.features(lat0, lon0, half_e_m, half_n_m);
                        let score = pack
                            .watercourses
                            .iter()
                            .filter(|watercourse| {
                                matches!(
                                    watercourse.kind,
                                    WatercourseKind::River | WatercourseKind::Canal
                                ) && !watercourse.intermittent
                                    && !watercourse.is_bridge
                                    && watercourse.source_tile.0 == watercourse.authority_zoom
                                    && watercourse.visible_at_terrain_level(level)
                            })
                            .flat_map(|watercourse| &watercourse.paths)
                            .map(Vec::len)
                            .sum::<usize>();
                        candidates.push((score, std::cmp::Reverse(candidate)));
                    }
                }
                candidates.sort_unstable();
                let (score, std::cmp::Reverse(selected)) = candidates.pop().unwrap();
                println!(
                    "{name:<12} L{level:<2} selected dense neighbor {selected:?} admitted_points={score}"
                );
                selected
            } else {
                center_cell
            };
            if hydrography_scope && let Some(vectors) = &vectors {
                let (lat0, lon0, half_e_m, half_n_m) = vector_query_bounds(cell);
                let extract_start = Instant::now();
                let mut pack = vectors.features(lat0, lon0, half_e_m, half_n_m);
                hydro_watercourse_records += pack.watercourses.len();
                let extract = extract_start.elapsed();
                let paths = pack
                    .watercourses
                    .iter()
                    .map(|watercourse| watercourse.paths.len())
                    .sum::<usize>();
                let points = pack
                    .watercourses
                    .iter()
                    .flat_map(|watercourse| &watercourse.paths)
                    .map(Vec::len)
                    .sum::<usize>();
                let frame =
                    vearth_core::frame::ChunkFrame::of(vearth_core::cell::ChunkAddr { cell, k: 0 });
                let mut measure_raster = |mode| {
                    pack.watercourse_output_mode = mode;
                    let build = || {
                        vearth_gen::tier0::build_overlay(
                            cell,
                            &frame,
                            &pack,
                            &|_, _| 0.0,
                            vearth_gen::tier0::WATER_MARGIN_COLS,
                            None,
                            None,
                            None,
                        )
                    };
                    let warm = build();
                    let wet_columns = warm
                        .cols
                        .iter()
                        .flatten()
                        .filter(|column| column.water)
                        .count();
                    let timings = (0..10)
                        .map(|_| {
                            let start = Instant::now();
                            let _ = build();
                            start.elapsed()
                        })
                        .collect();
                    let (median, max) = median_and_max(timings);
                    (wet_columns, median, max)
                };
                let (disabled_wet_columns, disabled_median, disabled_max) =
                    measure_raster(WatercourseOutputMode::DisabledG96);
                let (wet_columns, raster_median, raster_max) =
                    measure_raster(WatercourseOutputMode::RiversAndCanals);
                println!(
                    "{name:<12} L{level:<2} hydro records {:>4} paths {paths:>5} points {points:>7} projected {:>8} B wet_cols {disabled_wet_columns:>4}->{wet_columns:>4} complete={} extract {extract:>8.1?} raster off med/max {disabled_median:>8.1?}/{disabled_max:>8.1?} on {raster_median:>8.1?}/{raster_max:>8.1?}",
                    pack.watercourses.len(),
                    pack.watercourse_projected_bytes(),
                    pack.complete,
                );
            }
            let before_cold = vectors
                .as_ref()
                .map_or(0, ProtomapsSource::normalization_count);
            let t0 = Instant::now();
            let meshed = generate_surface_cell(cell, &sources);
            let cold = t0.elapsed();
            let after_cold = vectors
                .as_ref()
                .map_or(0, ProtomapsSource::normalization_count);
            let t1 = Instant::now();
            let _ = generate_surface_cell(cell, &sources);
            let warm = t1.elapsed();
            let settled = hydrography_scope.then(|| {
                let timings = (0..10)
                    .map(|_| {
                        let start = Instant::now();
                        let _ = generate_surface_cell(cell, &sources);
                        start.elapsed()
                    })
                    .collect();
                median_and_max(timings)
            });
            let after_warm = vectors
                .as_ref()
                .map_or(0, ProtomapsSource::normalization_count);
            cold_vector_fills += after_cold.saturating_sub(before_cold);
            warm_vector_fills += after_warm.saturating_sub(after_cold);
            let verts: usize = meshed.batches.iter().map(|c| c.mesh.vertices.len()).sum();
            let tris: usize = meshed
                .batches
                .iter()
                .map(|c| c.mesh.indices.len() / 3)
                .sum();
            // Material census over the meshed cell (vertex materials).
            let (mut walls, mut roads, mut water, mut leaves) = (0usize, 0usize, 0usize, 0usize);
            for vertex in meshed.batches.iter().flat_map(|b| &b.mesh.vertices) {
                match ((vertex.face_ao >> 5) & 255) as u8 {
                    m if material::is_architecture_wall(m) || material::is_architecture_roof(m) => {
                        walls += 1
                    }
                    material::ASPHALT | material::PATH | material::RAIL => roads += 1,
                    material::WATER => water += 1,
                    material::TRUNK => leaves += 1,
                    m if material::is_tree_foliage(m) => leaves += 1,
                    _ => {}
                }
            }
            if let Some((median, max)) = settled {
                println!(
                    "{name:<12} L{level:<2} chunks {:>2} verts {verts:>7} tris {tris:>7} bld {walls:>6} road {roads:>5} wat {water:>6} tree {leaves:>5} h [{:>6.0},{:>6.0}] m first {cold:>8.1?} repeat {warm:>7.1?} settled med/max {median:>8.1?}/{max:>8.1?}",
                    meshed.batches.len(),
                    meshed.height_range.0,
                    meshed.height_range.1,
                );
            } else {
                println!(
                    "{name:<12} L{level:<2} chunks {:>2}  verts {verts:>7}  tris {tris:>7}  bld {walls:>6} road {roads:>5} wat {water:>6} tree {leaves:>5}  h [{:>6.0},{:>6.0}] m  first {cold:>8.1?} repeat {warm:>7.1?}",
                    meshed.batches.len(),
                    meshed.height_range.0,
                    meshed.height_range.1,
                );
            }
        }
    }
    if let Some(vectors) = &vectors {
        println!(
            "vector tile fills: cold={} repeated={} total={}",
            cold_vector_fills,
            warm_vector_fills,
            vectors.normalization_count()
        );
        println!("{}", vectors.semantic_report());
    }
    assert!(
        !hydrography_scope || hydro_watercourse_records > 0,
        "hydrography measurement extracted no Amsterdam watercourses"
    );
}
