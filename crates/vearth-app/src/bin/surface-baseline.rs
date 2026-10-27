//! Deterministic continuous-surface generation, packing and cache baseline.
use std::{path::PathBuf, time::Instant};
use vearth_app::{chunk_cache::profile_surface_codec, prepared_render::pack_surface_cell};
use vearth_core::{cell::CellId, ellipsoid::geodetic_to_ecef};
use vearth_data::{
    FnElevation,
    road_profile::{RoadDetail, RoadProfile},
    vectors::{Building, BuildingKind, FeaturePack, Road, RoadClass, VectorSource, Water},
};
use vearth_gen::{Sources, generate_surface_cell};

#[derive(Clone, Copy)]
enum Features {
    City,
    Highrise,
    Crossing,
    Forest,
    Roads { context: bool },
}

fn rectangle(x: f32, z: f32, w: f32, d: f32) -> Vec<Vec<[f32; 2]>> {
    vec![vec![[x, z], [x + w, z], [x + w, z + d], [x, z + d]]]
}

impl VectorSource for Features {
    fn features(&self, lat: f64, lon: f64, _: f64, _: f64) -> FeaturePack {
        let mut pack = FeaturePack::empty();
        match self {
            Self::City | Self::Highrise => {
                let tall = matches!(self, Self::Highrise);
                for z in -9i32..=9 {
                    for x in -9i32..=9 {
                        let seed = ((z + 9) * 19 + x + 9) as u64;
                        let spacing = if tall { 38.0 } else { 21.0 };
                        let (x, z) = (x as f32 * spacing, z as f32 * spacing);
                        let mut rings = rectangle(
                            x,
                            z,
                            if tall { 28.0 } else { 14.0 },
                            if tall { 26.0 } else { 15.0 },
                        );
                        if seed % 11 == 0 {
                            rings.extend(rectangle(x + 4.0, z + 4.0, 5.0, 5.0));
                        }
                        pack.buildings.push(Building {
                            location_rad: None,
                            roof_shape: None,
                            use_kind: if tall && seed % 3 == 0 {
                                vearth_data::vectors::BuildingUse::Commercial
                            } else if tall && seed % 3 == 1 {
                                vearth_data::vectors::BuildingUse::Residential
                            } else {
                                Default::default()
                            },
                            rings,
                            height_m: if tall {
                                48.0 + (seed % 9) as f32 * 32.0
                            } else {
                                9.0 + (seed % 7) as f32 * 3.0
                            },
                            min_height_m: 0.0,
                            seed,
                            kind: BuildingKind::Building,
                        });
                        if seed % 13 == 0 {
                            pack.buildings.push(Building {
                                location_rad: None,
                                roof_shape: None,
                                use_kind: Default::default(),
                                rings: rectangle(x + 2.0, z + 2.0, 8.0, 9.0),
                                height_m: 36.0,
                                min_height_m: 12.0,
                                seed: seed + 10_000,
                                kind: BuildingKind::Building,
                            });
                        }
                    }
                }
                for z in [-45.0, 18.0, 81.0] {
                    pack.roads.push(Road {
                        profile: Default::default(),
                        pts: vec![[-250.0, z], [250.0, z]],
                        class: RoadClass::Minor,
                        is_bridge: false,
                    });
                }
            }
            Self::Crossing => {
                let mut water = rectangle(-250.0, -20.0, 500.0, 40.0);
                water.extend(rectangle(40.0, -5.0, 12.0, 10.0));
                pack.water.push(Water::inland(water));
                for (a, b, is_bridge) in [
                    (-150.0, -30.0, false),
                    (-30.0, 30.0, true),
                    (30.0, 150.0, false),
                ] {
                    pack.roads.push(Road {
                        profile: Default::default(),
                        pts: vec![[0.0, a], [0.0, b]],
                        class: RoadClass::Major,
                        is_bridge,
                    });
                }
            }
            Self::Forest => pack.forest.push(rectangle(-500.0, -500.0, 1000.0, 1000.0)),
            Self::Roads { context } => {
                pack.ground.push((
                    vearth_data::vectors::GroundKind::Green,
                    rectangle(-500.0, -500.0, 1000.0, 1000.0),
                ));
                for i in 0..24 {
                    let z = (i as f32 - 11.5) * 12.0;
                    pack.roads.push(Road {
                        profile: RoadProfile {
                            detail: [
                                RoadDetail::Footway,
                                RoadDetail::Cycleway,
                                RoadDetail::Pedestrian,
                                RoadDetail::Sidewalk,
                            ][i % 4],
                            seed: i as u64 + 1,
                            location_rad: context.then_some([lat, lon]),
                        },
                        pts: vec![[-500.0, z], [-100.0, z + 2.0], [100.0, z], [500.0, z + 4.0]],
                        class: RoadClass::Path,
                        is_bridge: false,
                    });
                }
            }
        }
        pack
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|i| args.get(i + 1))
    };
    let samples = value("--samples")
        .map(|v| v.parse::<usize>())
        .transpose()?
        .unwrap_or(5)
        .clamp(1, 101);
    let mut fixtures = vec![
        ("terrain-l11", 11, 46.0, 7.0, None),
        ("terrain-l15", 15, 46.0, 7.0, None),
        ("terrain-l19", 19, 46.0, 7.0, None),
        ("city-l15", 15, 46.0, 7.0, Some(Features::City)),
        ("city-l18", 18, 46.0, 7.0, Some(Features::City)),
        ("city-l19", 19, 46.0, 7.0, Some(Features::City)),
        ("crossing-l15", 15, 46.0, 7.0, Some(Features::Crossing)),
        ("crossing-l19", 19, 46.0, 7.0, Some(Features::Crossing)),
        ("forest-l15", 15, 46.0, 7.0, Some(Features::Forest)),
        ("forest-l19", 19, 46.0, 7.0, Some(Features::Forest)),
    ];
    for (id, name, level) in [
        ("landmark-opera-l16", "sydney-opera-house", 16),
        ("landmark-merlion-l20", "singapore-merlion", 20),
    ] {
        let lm = vearth_gen::landmarks::LANDMARKS
            .iter()
            .find(|lm| lm.name == name)
            .unwrap();
        fixtures.push((id, level, lm.lat_deg, lm.lon_deg, None));
    }
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../testdata/contracts/fixture-manifest.json"
    ))?;
    let listed: Vec<_> = manifest["fixtures"]
        .as_array()
        .ok_or("missing fixture list")?
        .iter()
        .map(|f| f["id"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(listed, fixtures.iter().map(|f| f.0).collect::<Vec<_>>());
    if args.iter().any(|arg| arg == "--roads") {
        let context = !args.iter().any(|arg| arg == "--roads-without-context");
        for (id, lat, lon) in [
            ("roads-amsterdam", 52.37, 4.90),
            ("roads-london", 51.51, -0.12),
            ("roads-rural", 46.0, 7.0),
            ("roads-york", 53.95825, -1.07855),
            ("roads-dumbo", 40.70335, -73.98957),
            ("roads-melbourne", -37.814, 144.963),
            ("roads-kyoto", 35.003, 135.775),
            ("roads-soweto", -26.235, 27.909),
            ("roads-paraty", -23.219, -44.712),
            ("roads-delhi", 28.6139, 77.209),
            ("roads-arid", 23.4, 25.7),
            ("roads-tropical", -3.5, -62.2),
        ] {
            for level in [15, 19] {
                fixtures.push((id, level, lat, lon, Some(Features::Roads { context })));
            }
        }
    }
    // External probes let paired binaries exercise exactly the same locations,
    // including places absent from the older binary's compiled atlas.
    let architecture_probes: Vec<serde_json::Value> = value("--architecture-probes")
        .map(|path| -> Result<_, Box<dyn std::error::Error>> {
            Ok(serde_json::from_slice(&std::fs::read(path)?)?)
        })
        .transpose()?
        .unwrap_or_default();
    let architecture_levels = value("--architecture-levels")
        .map(String::as_str)
        .unwrap_or("19")
        .split(',')
        .map(str::parse::<u8>)
        .collect::<Result<Vec<_>, _>>()?;
    if architecture_levels.is_empty()
        || architecture_levels
            .iter()
            .any(|level| !(14..=19).contains(level))
    {
        return Err("architecture levels must be comma-separated values from 14 through 19".into());
    }
    for probe in &architecture_probes {
        let name = probe["style_key"]
            .as_str()
            .ok_or("probe needs a style key")?;
        let lat = probe["latitude_deg"]
            .as_f64()
            .ok_or("probe needs latitude")?;
        let lon = probe["longitude_deg"]
            .as_f64()
            .ok_or("probe needs longitude")?;
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
            return Err("invalid probe coordinates".into());
        }
        for &level in &architecture_levels {
            let features = if args.iter().any(|a| a == "--highrise-probes") {
                Features::Highrise
            } else {
                Features::City
            };
            fixtures.push((name, level, lat, lon, Some(features)));
        }
    }
    // Optional geographic probes use the ordinary generator, bundled monthly
    // imagery and synthetic relief: no provider IO enters the timed workload.
    let month = value("--month")
        .map(|v| v.parse::<u8>())
        .transpose()?
        .unwrap_or(7);
    if !(1..=12).contains(&month) {
        return Err("--month must be 1 through 12".into());
    }
    let imagery = vearth_data::surface_pack::BundledSurface::for_month(month);
    if args.iter().any(|arg| arg == "--landscapes") {
        fixtures.extend([
            ("landscape-atlantic", 17, 53.0, -9.0, None),
            ("landscape-alpine-overlap", 17, 46.5, 10.0, None),
            ("landscape-boreal", 17, 63.0, 21.0, None),
            ("landscape-canyon", 17, 37.5, -110.5, None),
            ("landscape-amazon", 17, -3.0, -59.0, None),
            ("landscape-cerrado", 17, -15.0, -48.0, None),
            ("landscape-sahel", 17, 14.0, -6.0, None),
            ("landscape-deccan-overlap", 17, 18.0, 76.0, None),
            ("landscape-interior", 17, -24.0, 130.0, None),
            ("landscape-tussock-overlap", 17, -44.0, 170.0, None),
            ("landscape-dateline", 17, 69.0, 180.0, None),
            ("landscape-fallback", 17, -18.0, -140.0, None),
        ]);
    }
    let diagnostics = args.iter().any(|arg| arg == "--generation-diagnostics");
    vearth_gen::contracts::set_generation_timing_enabled(diagnostics);
    let mut rows = Vec::new();
    for (id, level, lat, lon, features) in fixtures {
        let base_height = if id == "landscape-alpine-overlap" {
            2600.0
        } else {
            100.0
        };
        let elevation = FnElevation(|lat: f64, lon: f64| {
            if features.is_some() {
                48.0
            } else {
                (base_height + 30.0 * (lat * 1000.0).sin() * (lon * 1000.0).cos()) as f32
            }
        });
        let sources = Sources {
            elevation: &elevation,
            surface: None,
            imagery: id.starts_with("landscape-").then_some(&imagery),
            vectors: features.as_ref().map(|f| f as &dyn VectorSource),
        };
        let cell = CellId::from_dir(
            geodetic_to_ecef(lat.to_radians(), lon.to_radians(), 0.0).normalize(),
            level,
        );
        if diagnostics {
            vearth_gen::contracts::reset_generation_timing();
        }
        let mut generation_ns = Vec::new();
        let mut packing_ns = Vec::new();
        let mut identity = None;
        for _ in 0..samples {
            let started = Instant::now();
            let mut surface = generate_surface_cell(cell, &sources);
            generation_ns.push(started.elapsed().as_nanos() as u64);
            let surface_identity = vearth_gen::contracts::surface_cell_fingerprint(&surface);
            // Isolate visual identity when collision simplification is intentional.
            // Move the query data out temporarily; do not clone dense geometry.
            let collision = std::mem::take(&mut surface.solid_collision);
            let terrain = surface.terrain_collision.take();
            let visual_identity = vearth_gen::contracts::surface_cell_fingerprint(&surface);
            let collision_triangles = collision.triangles().len();
            let collision_capsules = collision.capsules().len();
            surface.solid_collision = collision;
            surface.terrain_collision = terrain;
            let vertices: usize = surface.batches.iter().map(|b| b.mesh.vertices.len()).sum();
            let indices: usize = surface.batches.iter().map(|b| b.mesh.indices.len()).sum();
            let codec = profile_surface_codec(&surface, 1).ok_or("surface codec failed")?;
            let started = Instant::now();
            let packed = pack_surface_cell(surface)?;
            packing_ns.push(started.elapsed().as_nanos() as u64);
            packed.validate()?;
            let fingerprint = packed.compute_semantic_fingerprint();
            if let Some(expected) = identity {
                assert_eq!(expected, (surface_identity, fingerprint));
            } else {
                identity = Some((surface_identity, fingerprint));
            }
            if generation_ns.len() == samples {
                rows.push(serde_json::json!({"id":id,"level":level,"generation_ns":&generation_ns,"packing_ns":&packing_ns,"vertices":vertices,"indices":indices,"surface_bytes":codec.encoded_bytes,"collision_triangles":collision_triangles,"collision_capsules":collision_capsules,"visual_fingerprint":visual_identity.to_string(),"surface_fingerprint":surface_identity.to_string(),"semantic_fingerprint":fingerprint.to_string()}));
            }
        }
        if diagnostics {
            let snapshot = vearth_gen::contracts::generation_timing_snapshot();
            let at = usize::from(level);
            eprintln!(
                "{id} L{level}: phases_ns={:?}, retries={} retry_ns={} roofs={:?}",
                snapshot.phase_ns_by_level.map(|phase| phase[at]),
                snapshot.building_retry_count_by_level[at],
                snapshot.building_retry_ns_by_level[at],
                snapshot
                    .roof_by_shape_and_level
                    .iter()
                    .map(|shape| shape[at])
                    .filter(|work| work.roofs > 0)
                    .collect::<Vec<_>>()
            );
        }
    }
    let mut tree_meshes = Vec::new();
    for profile in vearth_gen::vegetation::TREE_PROFILES {
        for lod in 0..=2 {
            let tree = vearth_gen::tree_mesh::build_tree_mesh(profile.id, lod);
            let mut identity = vearth_core::contracts::SemanticHasher::new("vearth.tree-mesh.v1");
            identity.fingerprint(vearth_core::contracts::mesh_semantic_fingerprint(
                &tree.mesh,
            ));
            identity.f32(tree.height_m);
            identity.f32(tree.radius_m);
            tree_meshes.push(serde_json::json!({
                "profile": format!("{:?}", profile.id),
                "lod": lod,
                "vertices": tree.mesh.vertices.len(),
                "indices": tree.mesh.indices.len(),
                "visual_fingerprint": identity.finish().to_string(),
            }));
        }
    }
    let report = serde_json::json!({"pipeline":"continuous-surfaces","landscape_month":month,"dataset_identity":manifest["dataset_identity"],"generation_version":vearth_gen::GEN_VERSION,"surface_codec_version":vearth_app::chunk_cache::SURFACE_CODEC_VERSION,"samples":samples,"cells":rows,"tree_meshes":tree_meshes});
    let text = serde_json::to_string_pretty(&report)?;
    if let Some(path) = value("--output") {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &text)?;
    }
    if let Some(path) = value("--summary") {
        std::fs::write(
            path,
            format!(
                "Continuous surface generation, GPU packing and cache round trips passed for {} samples per cell.\n",
                samples
            ),
        )?;
    }
    println!("{text}");
    Ok(())
}
