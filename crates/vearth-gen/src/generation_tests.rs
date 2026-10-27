use super::*;
use vearth_core::cubesphere::Face;
use vearth_data::FnElevation;
use vearth_data::bridge_pack::{BridgeStructure, CanonicalBridge};
use vearth_data::protomaps_semantics::SourceFeatureIdentity;
use vearth_data::vectors::{
    Building, FeaturePack, Road, RoadClass, Water, Watercourse, WatercourseKind,
    WatercourseOutputMode,
};

fn cell(level: u8, i: u32, j: u32) -> CellId {
    CellId {
        face: Face(0),
        level,
        i,
        j,
    }
}

fn mid_cell(level: u8) -> CellId {
    let n = 1u32 << level;
    cell(level, n / 2, n / 2)
}

#[test]
fn line_only_watercourse_requests_the_flatten_margin_only_when_admitted() {
    let mut pack = FeaturePack::empty();
    pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
    pack.watercourses.push(Watercourse {
        paths: vec![vec![[-20.0, 0.0], [20.0, 0.0]]],
        kind: WatercourseKind::River,
        width_m: WatercourseKind::River.fallback_width_m(),
        identity: SourceFeatureIdentity::Provided(1),
        style_identity: 1,
        source_tile: (15, 0, 0),
        authority_zoom: 15,
        source_feature_index: 0,
        source_min_zoom: None,
        fragment_count: 1,
        intermittent: false,
        is_bridge: false,
    });
    let frame13 = ChunkFrame::of(ChunkAddr {
        cell: mid_cell(13),
        k: 0,
    });
    let frame14 = ChunkFrame::of(ChunkAddr {
        cell: mid_cell(14),
        k: 0,
    });
    let frame19 = ChunkFrame::of(ChunkAddr {
        cell: mid_cell(19),
        k: 0,
    });
    assert_eq!(elevation_margin_for_pack(&pack, &frame13, 13), 0);
    assert_eq!(
        elevation_margin_for_pack(&pack, &frame14, 14),
        tier0::WATER_MARGIN_COLS
    );

    pack.watercourses[0].intermittent = true;
    assert_eq!(elevation_margin_for_pack(&pack, &frame19, 19), 0);
    pack.watercourses[0].intermittent = false;
    pack.watercourses[0].is_bridge = true;
    assert_eq!(elevation_margin_for_pack(&pack, &frame19, 19), 0);
    pack.watercourses[0].is_bridge = false;
    pack.watercourse_output_mode = WatercourseOutputMode::DisabledG96;
    assert_eq!(elevation_margin_for_pack(&pack, &frame19, 19), 0);
}

#[test]
fn dry_cliffs_and_margin_trees_allocate_their_elevation_support() {
    struct BoundaryVectors {
        cliff: Option<vearth_data::vectors::CliffLine>,
        tree: Option<vearth_data::vectors::ExactTree>,
    }
    impl VectorSource for BoundaryVectors {
        fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
            let mut pack = FeaturePack::empty();
            pack.cliffs.extend(self.cliff.iter().cloned());
            pack.exact_trees.extend(self.tree);
            pack
        }
    }

    let target = mid_cell(19);
    let frame = ChunkFrame::of(ChunkAddr { cell: target, k: 0 });
    let cliff_vectors = BoundaryVectors {
        cliff: Some(vearth_data::vectors::CliffLine {
            pts: vec![[-10_000.0, 0.0], [10_000.0, 0.0]],
            identity: SourceFeatureIdentity::Provided(1),
        }),
        tree: None,
    };
    let cliff_pack = cliff_vectors.features(0.0, 0.0, 0.0, 0.0);
    assert_eq!(elevation_margin_for_pack(&cliff_pack, &frame, 19), 1);
    let sloped = FnElevation(|lat, _| (lat * vearth_core::ellipsoid::MEAN_RADIUS) as f32);
    let cliff_column = generate_surface_cell(
        target,
        &Sources {
            elevation: &sloped,
            surface: None,
            imagery: None,
            vectors: Some(&cliff_vectors),
        },
    );
    assert!(cliff_column.complete);

    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    let (tree_lat, tree_lon) = frame.column_latlon(-0.02, 0.5);
    let mut dlon = tree_lon - lon0;
    if dlon > std::f64::consts::PI {
        dlon -= std::f64::consts::TAU;
    } else if dlon < -std::f64::consts::PI {
        dlon += std::f64::consts::TAU;
    }
    let tree_point = [
        (dlon * lat0.cos() * vearth_core::ellipsoid::MEAN_RADIUS) as f32,
        ((tree_lat - lat0) * vearth_core::ellipsoid::MEAN_RADIUS) as f32,
    ];
    let tree_vectors = BoundaryVectors {
        cliff: None,
        tree: Some(vearth_data::vectors::ExactTree {
            point: tree_point,
            identity: SourceFeatureIdentity::Provided(2),
            seed: 2,
        }),
    };
    let tree_pack = tree_vectors.features(0.0, 0.0, 0.0, 0.0);
    assert!(elevation_margin_for_pack(&tree_pack, &frame, 19) >= 1);
    let flat = FnElevation(|_, _| 40.0);
    let tree_column = generate_surface_cell(
        target,
        &Sources {
            elevation: &flat,
            surface: None,
            imagery: None,
            vectors: Some(&tree_vectors),
        },
    );
    assert!(tree_column.complete);
    assert!(
        tree_column.tree_instances.is_empty(),
        "neighbour owns the root"
    );
    assert!(tree_column.solid_collision.capsules().is_empty());
}

#[test]
fn road_guides_are_fine_level_deduplicated_and_drivable() {
    let fine = mid_cell(19);
    let frame = ChunkFrame::of(ChunkAddr { cell: fine, k: 0 });
    let mut pack = FeaturePack::empty();
    pack.roads.extend([
        Road {
            profile: Default::default(),
            pts: vec![[-20.0, 0.0], [20.0, 0.0]],
            class: RoadClass::Minor,
            is_bridge: false,
        },
        Road {
            profile: Default::default(),
            pts: vec![[20.0, 0.0], [-20.0, 0.0]],
            class: RoadClass::Minor,
            is_bridge: false,
        },
        Road {
            profile: Default::default(),
            pts: vec![[-20.0, 8.0], [20.0, 8.0]],
            class: RoadClass::Path,
            is_bridge: false,
        },
        Road {
            profile: Default::default(),
            pts: vec![[-20.0, 12.0], [20.0, 12.0]],
            class: RoadClass::Service,
            is_bridge: false,
        },
        Road {
            profile: Default::default(),
            pts: vec![[0.0, -20.0], [0.0, 20.0]],
            class: RoadClass::Rail,
            is_bridge: false,
        },
        Road {
            profile: Default::default(),
            pts: vec![[-20.0, 4.0], [20.0, 4.0]],
            class: RoadClass::Major,
            is_bridge: true,
        },
    ]);

    let guides = build_road_guides(fine, &frame, &pack);
    assert_eq!(guides.len(), 3, "duplicate, rail, or bridge guide leaked");
    assert!(
        guides
            .iter()
            .any(|guide| { guide.width_m == RoadClass::Minor.width_m() && guide.assist == 0.9 })
    );
    assert!(
        guides
            .iter()
            .any(|guide| { guide.width_m == RoadClass::Path.width_m() && guide.assist == 0.9 })
    );
    assert!(
        guides
            .iter()
            .any(|guide| { guide.width_m == RoadClass::Service.width_m() && guide.assist == 0.9 })
    );
    assert_eq!(
        road_appearance(RoadClass::Service, 0.5, 1.0).material,
        material::ASPHALT
    );
    assert!(build_road_guides(mid_cell(16), &frame, &pack).is_empty());
}

#[test]
fn road_guides_retain_bounded_polyline_continuations() {
    let fine = mid_cell(19);
    let frame = ChunkFrame::of(ChunkAddr { cell: fine, k: 0 });
    let mut pack = FeaturePack::empty();
    pack.roads.push(Road {
        profile: Default::default(),
        pts: vec![[-12.0, 0.0], [0.0, 0.0], [0.0, 12.0]],
        class: RoadClass::Minor,
        is_bridge: false,
    });

    let guides = build_road_guides(fine, &frame, &pack);
    assert_eq!(guides.len(), 2);
    let exits: Vec<_> = guides
        .iter()
        .flat_map(|guide| [guide.a_exit, guide.b_exit])
        .filter(|exit| exit[0] != 0.0 || exit[1] != 0.0)
        .collect();
    assert_eq!(exits.len(), 2, "only the shared corner should continue");
    for exit in exits {
        let length = (exit[0] * exit[0] + exit[1] * exit[1]).sqrt();
        assert!((length - 1.0).abs() < 1.0e-5);
    }
}

fn destination_cell(
    destination: &crate::flower_destinations::FlowerDestination,
    level: u8,
) -> CellId {
    let (lat, lon, _) = destination.influence.bounding_circle();
    CellId::from_dir(
        vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0).normalize(),
        level,
    )
}

fn test_col(rgb: [u8; 3], climate: Climate, slope: f32, relief: f32) -> Col {
    Col {
        elev: 900.0,
        top: 900.0,
        slope,
        relief,
        climate,
        img_detail: Some(rgb),
        ..Col::EMPTY
    }
}

#[test]
fn grand_canyon_like_surface_is_warm_rock_not_generic_grass() {
    let col = test_col(
        [102, 78, 52],
        Climate {
            temp_c: 16.0,
            arid: 0.72,
            precipitation_mm_day: 0.45,
            humidity: 0.34,
        },
        0.43,
        720.0,
    );
    assert_eq!(classify_surface(&col), SurfaceFamily::RedSedimentary);
    let voxel = terrain_appearance(&col, 820.0, 0.5, 1.0);
    assert_eq!(voxel.material, material::ROCK);
    assert!(voxel.tint[0] >= voxel.tint[1] + 20, "{:?}", voxel.tint);
    assert!(voxel.tint[1] >= voxel.tint[2] + 12, "{:?}", voxel.tint);
    let value = |rgb: [u8; 3]| {
        f32::from(rgb[0]) * 0.2126 + f32::from(rgb[1]) * 0.7152 + f32::from(rgb[2]) * 0.0722
    };
    let source_value = value([102, 78, 52]);
    assert!((source_value..=source_value + 45.0).contains(&value(voxel.tint)));
    let bright_source = Col {
        img_detail: Some([191, 132, 93]),
        ..col
    };
    let bright = terrain_appearance(&bright_source, 820.0, 0.5, 1.0);
    assert_eq!(bright.material, material::ROCK);
    assert!(value(bright.tint) > value(voxel.tint) + 12.0);
    assert_eq!(
        terrain_appearance(&col, 820.0, 0.5, 1.0 / 3.0).tint,
        [102, 78, 52],
    );
}

#[test]
fn olive_regional_imagery_does_not_turn_steep_rock_into_sandstone() {
    let climate = Climate {
        temp_c: 5.0,
        arid: 0.43,
        precipitation_mm_day: 1.9,
        humidity: 0.48,
    };
    for color in [[65, 63, 40], [57, 57, 34], [73, 68, 43]] {
        let cliff = test_col(color, climate, 0.9, 500.0);
        assert_eq!(classify_surface(&cliff), SurfaceFamily::Granite);
        let rock = terrain_appearance(&cliff, cliff.elev, 0.5, 1.0);
        assert_eq!(rock.material, material::ROCK);
        assert!(rock.tint[0].abs_diff(rock.tint[1]) < 16);
        assert!(rock.tint[0].abs_diff(rock.tint[2]) < 40);
        let valley = Col {
            slope: 0.10,
            relief: 20.0,
            ..cliff
        };
        assert_eq!(classify_surface(&valley), SurfaceFamily::Boreal);
    }
    let dry = Climate {
        arid: 0.70,
        ..climate
    };
    assert_eq!(
        classify_surface(&test_col([111, 88, 60], dry, 0.9, 500.0)),
        SurfaceFamily::RedSedimentary,
    );
    assert_eq!(
        classify_surface(&test_col([146, 135, 104], dry, 0.9, 500.0)),
        SurfaceFamily::TanSedimentary,
    );
}

#[test]
fn regional_material_identity_survives_high_resolution_pixel_changes() {
    let regional = test_col(
        [65, 63, 40],
        Climate {
            temp_c: 5.0,
            arid: 0.43,
            precipitation_mm_day: 1.9,
            humidity: 0.48,
        },
        0.9,
        220.0,
    );
    assert_eq!(classify_surface(&regional), SurfaceFamily::Granite);
    for high_resolution in [[0, 255, 0], [240, 35, 10], [12, 12, 12], [250, 250, 250]] {
        let column = Col {
            img_hr: Some(high_resolution),
            ..regional
        };
        assert_eq!(classify_surface(&column), SurfaceFamily::Granite);
        assert_eq!(mapped_rock_family(&column), SurfaceFamily::Granite);
    }
    let only_high_resolution = Col {
        img_detail: None,
        img: None,
        img_hr: Some([240, 35, 10]),
        ..regional
    };
    assert_eq!(
        classify_surface(&only_high_resolution),
        SurfaceFamily::RedSedimentary,
    );
}

#[test]
fn classifier_distinguishes_representative_surface_families() {
    let rainforest = test_col(
        [45, 82, 42],
        Climate {
            temp_c: 25.0,
            arid: 0.08,
            precipitation_mm_day: 6.0,
            humidity: 0.82,
        },
        0.08,
        45.0,
    );
    assert_eq!(classify_surface(&rainforest), SurfaceFamily::Tropical);

    let salt = test_col(
        [228, 224, 213],
        Climate {
            temp_c: 23.0,
            arid: 0.91,
            precipitation_mm_day: 0.08,
            humidity: 0.18,
        },
        0.03,
        8.0,
    );
    assert_eq!(classify_surface(&salt), SurfaceFamily::Salt);

    let basalt = test_col(
        [59, 58, 56],
        Climate {
            temp_c: 9.0,
            arid: 0.38,
            precipitation_mm_day: 2.0,
            humidity: 0.58,
        },
        0.31,
        180.0,
    );
    assert_eq!(classify_surface(&basalt), SurfaceFamily::Volcanic);
}

#[test]
fn low_elevation_does_not_override_observed_humid_land_with_sand() {
    let lowland = Col {
        elev: 1.0,
        top: 1.0,
        sea: 0.0,
        ..test_col(
            [70, 107, 57],
            Climate {
                temp_c: 15.0,
                arid: 0.15,
                precipitation_mm_day: 3.5,
                humidity: 0.78,
            },
            0.02,
            1.0,
        )
    };
    assert_eq!(classify_surface(&lowland), SurfaceFamily::Temperate);
    assert_eq!(
        terrain_appearance(&lowland, 1.0, 0.5, 1.0).material,
        material::GRASS
    );
    assert_eq!(
        ground_appearance(GroundKind::Sand, &lowland, 0.5, 1.0).material,
        material::SAND
    );
    let submerged = Col {
        elev: -1.0,
        ..lowland
    };
    assert_eq!(classify_surface(&submerged), SurfaceFamily::Seabed);
}

#[test]
fn dry_high_plateaus_keep_exposed_ground_while_mapped_ice_remains_authoritative() {
    let plateau = Col {
        elev: 4_500.0,
        top: 4_500.0,
        ..test_col(
            [131, 115, 93],
            Climate {
                temp_c: -9.0,
                arid: 0.84,
                precipitation_mm_day: 0.20,
                humidity: 0.30,
            },
            0.08,
            18.0,
        )
    };
    for temperature in [-18.0, -9.0, 3.0, 12.0] {
        let col = Col {
            climate: Climate {
                temp_c: temperature,
                ..plateau.climate
            },
            ..plateau
        };
        for col in [
            col,
            Col {
                img_detail: None,
                ..col
            },
        ] {
            assert!(!matches!(
                classify_surface(&col),
                SurfaceFamily::Snow | SurfaceFamily::Ice
            ));
            for kind in [GroundKind::Glacier, GroundKind::RegionalGlacier] {
                assert_eq!(
                    ground_appearance(kind, &col, 0.5, 1.0).material,
                    material::ICE
                );
            }
        }
    }
    let observed_snow = Col {
        img_detail: Some([235, 237, 235]),
        ..plateau
    };
    assert_eq!(classify_surface(&observed_snow), SurfaceFamily::Snow);
}

#[test]
fn regional_barren_preserves_observed_sand_and_salt_instead_of_forcing_rock() {
    let climate = Climate {
        temp_c: 25.0,
        arid: 0.85,
        precipitation_mm_day: 0.15,
        humidity: 0.25,
    };
    for (imagery, expected) in [
        ([201, 187, 157], material::SAND),
        ([232, 230, 223], material::SALT),
    ] {
        let col = test_col(imagery, climate, 0.05, 15.0);
        assert_eq!(
            ground_appearance(GroundKind::RegionalBarren, &col, 0.5, 1.0).material,
            expected
        );
        assert_eq!(
            ground_appearance(GroundKind::BareRock, &col, 0.5, 1.0).material,
            material::ROCK
        );
    }
}

#[test]
fn climate_conditioned_vegetation_and_wetland_tints_cross_old_thresholds_continuously() {
    let template = test_col(
        [93, 118, 78],
        Climate {
            temp_c: -3.0,
            arid: 0.25,
            precipitation_mm_day: 3.0,
            humidity: 0.70,
        },
        0.10,
        35.0,
    );
    for ground in [
        None,
        Some(GroundKind::Forest),
        Some(GroundKind::NaturalGrass),
        Some(GroundKind::Scrub),
        Some(GroundKind::Wetland),
    ] {
        let sample = |col: &Col| {
            ground.map_or_else(
                || terrain_appearance(col, col.elev, 0.5, 1.0),
                |kind| ground_appearance(kind, col, 0.5, 1.0),
            )
        };
        let mut previous = sample(&template);
        for step in 1..=320 {
            let col = Col {
                climate: Climate {
                    temp_c: -3.0 + step as f32 * 0.1,
                    ..template.climate
                },
                ..template
            };
            let current = sample(&col);
            assert_eq!(current.material, previous.material);
            assert!(
                current
                    .tint
                    .iter()
                    .zip(previous.tint)
                    .all(|(a, b)| a.abs_diff(b) <= 2),
                "{ground:?}: discontinuity at {} C",
                col.climate.temp_c
            );
            previous = current;
        }
    }
}

#[test]
fn mapped_natural_ground_reuses_surface_vocabulary_and_local_variation() {
    let temperate = test_col(
        [74, 105, 63],
        Climate {
            temp_c: 14.0,
            arid: 0.22,
            precipitation_mm_day: 3.2,
            humidity: 0.72,
        },
        0.08,
        30.0,
    );
    let arid = test_col(
        [151, 111, 76],
        Climate {
            temp_c: 24.0,
            arid: 0.72,
            precipitation_mm_day: 0.2,
            humidity: 0.18,
        },
        0.08,
        30.0,
    );
    for (kind, expected_material) in [
        (GroundKind::BareRock, material::ROCK),
        (GroundKind::Wetland, material::WETLAND),
        (GroundKind::Scrub, material::GRASS),
        (GroundKind::NaturalGrass, material::GRASS),
        (GroundKind::Glacier, material::ICE),
    ] {
        let first = ground_appearance(kind, &temperate, 0.5, 1.0);
        let second = ground_appearance(kind, &arid, 0.5, 1.0);
        assert_eq!(first.material, expected_material, "{kind:?}");
        assert_ne!(first.tint, second.tint, "{kind:?} became a flat patch");
    }
}

#[test]
fn mapped_vegetation_shares_hillside_and_cliff_appearance_without_changing_identity() {
    let template = test_col(
        [112, 125, 84],
        Climate {
            temp_c: 9.0,
            arid: 0.18,
            precipitation_mm_day: 3.2,
            humidity: 0.65,
        },
        0.0,
        220.0,
    );
    let kinds = [
        GroundKind::Forest,
        GroundKind::Scrub,
        GroundKind::NaturalGrass,
        GroundKind::RegionalForest,
        GroundKind::RegionalScrub,
        GroundKind::RegionalGrassland,
    ];
    let foothill = Col {
        slope: 35_f32.to_radians(),
        ..template
    };
    let gentle = Col {
        slope: 15_f32.to_radians(),
        ..template
    };
    for kind in kinds {
        assert_eq!(
            ground_appearance(kind, &foothill, 0.5, 1.0).tint,
            ground_appearance(kind, &gentle, 0.5, 1.0).tint,
            "mapped vegetation on an ordinary hillside must not become bare rock"
        );
        let mut previous = ground_appearance(kind, &gentle, 0.5, 1.0);
        for degrees in 30..=700 {
            let col = Col {
                slope: (degrees as f32 * 0.1).to_radians(),
                ..template
            };
            let current = ground_appearance(kind, &col, 0.5, 1.0);
            assert_eq!(current.material, material::GRASS, "{kind:?}");
            assert!(
                current
                    .tint
                    .iter()
                    .zip(previous.tint)
                    .all(|(a, b)| a.abs_diff(b) <= 3),
                "{kind:?}: abrupt slope appearance at {degrees}/10 degrees"
            );
            previous = current;
        }
    }
    let cliff = Col {
        slope: 70_f32.to_radians(),
        ..template
    };
    let geology = terrain_appearance(&cliff, cliff.elev, 0.5, 1.0).tint;
    for kind in kinds {
        assert_eq!(
            ground_appearance(kind, &cliff, 0.5, 1.0).tint,
            geology,
            "{kind:?} must not paint a vegetation wall over a resolved cliff"
        );
    }
    assert_eq!(
        ground_appearance(GroundKind::BareRock, &gentle, 0.5, 1.0).material,
        material::ROCK
    );
    assert_eq!(
        ground_appearance(GroundKind::Glacier, &gentle, 0.5, 1.0).material,
        material::ICE
    );
}

#[test]
fn mapped_vegetation_cliff_blend_is_stable_when_relief_support_refines() {
    let coarse = test_col(
        [112, 125, 84],
        Climate {
            temp_c: 9.0,
            arid: 0.18,
            precipitation_mm_day: 3.2,
            humidity: 0.65,
        },
        48_f32.to_radians(),
        220.0,
    );
    let fine = Col {
        relief: 2.0,
        ..coarse
    };
    for kind in [
        GroundKind::Forest,
        GroundKind::Scrub,
        GroundKind::NaturalGrass,
        GroundKind::RegionalForest,
        GroundKind::RegionalScrub,
        GroundKind::RegionalGrassland,
    ] {
        assert_eq!(
            ground_appearance(kind, &coarse, 0.5, 1.0),
            ground_appearance(kind, &fine, 0.5, 1.0)
        );
    }
    let stamp = tier0::FloraStamp {
        location_rad: None,
        kind: vearth_data::vectors::FloraRegionKind::Meadow,
        seed: 52776577613368,
        curated_style: 0,
        surface: flora::select_field_surface(
            vearth_data::vectors::FloraRegionKind::Meadow,
            52776577613368,
        ),
    };
    for level in 14..=19 {
        let cell = mid_cell(level);
        let a = field_surface_appearance(stamp, &coarse, cell, 16, 16);
        let b = field_surface_appearance(stamp, &fine, cell, 16, 16);
        assert_eq!(
            a, b,
            "meadow geology must not re-roll as relief support shrinks"
        );
        assert_eq!(a.material, material::FIELD_MEADOW);
    }
}

#[test]
fn mapped_natural_ground_has_explicit_vegetation_eligibility() {
    let col = test_col(
        [74, 105, 63],
        Climate {
            temp_c: 14.0,
            arid: 0.22,
            precipitation_mm_day: 3.2,
            humidity: 0.72,
        },
        0.08,
        30.0,
    );
    let cols = [[col; N]; N];
    let landmarks = Vec::new();
    let eligible = |kind| {
        let mut overlay = tier0::CellOverlay::empty();
        overlay.cols[N / 2][N / 2].ground = Some(kind);
        natural_wildflower_surface(Some(&overlay), &cols, &landmarks, N / 2, N / 2)
    };
    assert!(eligible(GroundKind::NaturalGrass));
    for kind in [
        GroundKind::BareRock,
        GroundKind::Wetland,
        GroundKind::Scrub,
        GroundKind::Glacier,
    ] {
        assert!(!eligible(kind), "{kind:?} admitted generic wildflowers");
    }
}

#[test]
fn mapped_managed_ground_reuses_local_colour_without_inventing_transport_detail() {
    let temperate = test_col(
        [76, 101, 67],
        Climate {
            temp_c: 13.0,
            arid: 0.20,
            precipitation_mm_day: 3.2,
            humidity: 0.74,
        },
        0.06,
        18.0,
    );
    let arid = test_col(
        [156, 112, 72],
        Climate {
            temp_c: 25.0,
            arid: 0.76,
            precipitation_mm_day: 0.18,
            humidity: 0.16,
        },
        0.06,
        18.0,
    );
    for (kind, expected_material) in [
        (GroundKind::Farmyard, material::SOIL),
        (GroundKind::RailwayArea, material::PAVED),
        (GroundKind::RunwayArea, material::PAVED),
    ] {
        let first = ground_appearance(kind, &temperate, 0.5, 1.0);
        let second = ground_appearance(kind, &arid, 0.5, 1.0);
        assert_eq!(first.material, expected_material, "{kind:?}");
        assert_ne!(first.tint, second.tint, "{kind:?} became a flat patch");
    }
    let airport_temperate = ground_appearance(GroundKind::AirportArea, &temperate, 0.5, 1.0);
    let airport_arid = ground_appearance(GroundKind::AirportArea, &arid, 0.5, 1.0);
    assert_eq!(
        airport_temperate.material,
        terrain_appearance(&temperate, temperate.elev, 0.5, 1.0).material
    );
    assert_eq!(
        airport_arid.material,
        terrain_appearance(&arid, arid.elev, 0.5, 1.0).material
    );
    assert_ne!(airport_temperate.tint, airport_arid.tint);
}

#[test]
fn mapped_managed_ground_excludes_generic_vegetation() {
    let col = test_col(
        [74, 105, 63],
        Climate {
            temp_c: 14.0,
            arid: 0.22,
            precipitation_mm_day: 3.2,
            humidity: 0.72,
        },
        0.08,
        30.0,
    );
    let cols = [[col; N]; N];
    let landmarks = Vec::new();
    for kind in [
        GroundKind::Farmyard,
        GroundKind::RailwayArea,
        GroundKind::AirportArea,
        GroundKind::RunwayArea,
    ] {
        let mut overlay = tier0::CellOverlay::empty();
        overlay.cols[N / 2][N / 2].ground = Some(kind);
        assert!(
            !natural_wildflower_surface(Some(&overlay), &cols, &landmarks, N / 2, N / 2,),
            "{kind:?} admitted generic wildflowers"
        );
    }
}

#[test]
fn regional_ground_is_imagery_dominant_locally_varied_and_ground_only() {
    let temperate = test_col(
        [76, 101, 67],
        Climate {
            temp_c: 13.0,
            arid: 0.20,
            precipitation_mm_day: 3.2,
            humidity: 0.74,
        },
        0.06,
        18.0,
    );
    let arid = test_col(
        [156, 112, 72],
        Climate {
            temp_c: 25.0,
            arid: 0.76,
            precipitation_mm_day: 0.18,
            humidity: 0.16,
        },
        0.06,
        18.0,
    );
    for kind in [
        GroundKind::RegionalBarren,
        GroundKind::RegionalForest,
        GroundKind::RegionalGlacier,
        GroundKind::RegionalGrassland,
        GroundKind::RegionalScrub,
        GroundKind::RegionalUrban,
    ] {
        let first = ground_appearance(kind, &temperate, 0.5, 1.0);
        let second = ground_appearance(kind, &arid, 0.5, 1.0);
        assert_ne!(first.tint, second.tint, "{kind:?} became a flat patch");
    }
    assert_eq!(
        ground_appearance(GroundKind::RegionalGlacier, &temperate, 0.5, 1.0).material,
        material::ICE
    );
    for col in [&temperate, &arid] {
        assert_eq!(
            ground_appearance(GroundKind::RegionalUrban, col, 0.5, 1.0).material,
            terrain_appearance(col, col.elev, 0.5, 1.0).material,
            "regional urban must retain observed substrate"
        );
    }

    let cols = [[temperate; N]; N];
    let landmarks = Vec::new();
    for kind in [
        GroundKind::RegionalBarren,
        GroundKind::RegionalForest,
        GroundKind::RegionalGlacier,
        GroundKind::RegionalGrassland,
        GroundKind::RegionalScrub,
        GroundKind::RegionalUrban,
    ] {
        let mut overlay = tier0::CellOverlay::empty();
        overlay.cols[N / 2][N / 2].ground = Some(kind);
        assert!(
            !natural_wildflower_surface(Some(&overlay), &cols, &landmarks, N / 2, N / 2),
            "{kind:?} admitted generic wildflowers"
        );
    }
}

#[test]
fn rock_exposure_feathers_slope_and_uses_relief_as_support() {
    let climate = Climate {
        temp_c: 12.0,
        arid: 0.12,
        precipitation_mm_day: 3.0,
        humidity: 0.68,
    };
    let gentle = test_col([78, 105, 69], climate, 0.41, 220.0);
    let transition = test_col([78, 105, 69], climate, 0.56, 220.0);
    let cliff = test_col([78, 105, 69], climate, 0.71, 220.0);
    let unsupported_transition = test_col([78, 105, 69], climate, 0.56, 20.0);

    assert_eq!(rock_exposure(&gentle), 0.0);
    assert!((rock_exposure(&transition) - 0.5).abs() < 0.001);
    assert_eq!(rock_exposure(&cliff), 1.0);
    assert!(rock_exposure(&unsupported_transition) < rock_exposure(&transition));
    let refined_cliff = Col {
        relief: 2.0,
        ..cliff
    };
    assert_eq!(rock_exposure(&refined_cliff), 1.0);
    let coarse = terrain_appearance(&cliff, cliff.elev, 0.5, 1.0);
    let refined = terrain_appearance(&refined_cliff, cliff.elev, 0.5, 1.0);
    assert_eq!(coarse.material, material::ROCK);
    assert_eq!(refined.material, material::ROCK);
    assert_eq!(coarse.tint, refined.tint);
}

#[test]
fn terrain_colour_blends_before_the_rock_material_handoff() {
    let climate = Climate {
        temp_c: 12.0,
        arid: 0.12,
        precipitation_mm_day: 3.0,
        humidity: 0.68,
    };
    let gentle = terrain_appearance(
        &test_col([78, 105, 69], climate, 0.41, 220.0),
        900.0,
        0.5,
        1.0,
    );
    let transition = terrain_appearance(
        &test_col([78, 105, 69], climate, 0.56, 220.0),
        900.0,
        0.5,
        1.0,
    );
    let cliff = terrain_appearance(
        &test_col([78, 105, 69], climate, 0.71, 220.0),
        900.0,
        0.5,
        1.0,
    );

    assert_eq!(gentle.material, material::GRASS);
    assert_eq!(transition.material, material::GRASS);
    assert_eq!(cliff.material, material::ROCK);
    assert_ne!(gentle.tint, transition.tint);
    assert_ne!(transition.tint, cliff.tint);
    for channel in 0..3 {
        let lo = gentle.tint[channel].min(cliff.tint[channel]);
        let hi = gentle.tint[channel].max(cliff.tint[channel]);
        assert!(
            (lo..=hi).contains(&transition.tint[channel]),
            "{:?} is not between {:?} and {:?}",
            transition.tint,
            gentle.tint,
            cliff.tint,
        );
    }
}

#[test]
fn humid_harbours_stay_blue_while_arid_rivers_keep_sediment_tint() {
    let harbour = test_col(
        [70, 110, 125],
        climate_at(-33.8651_f64.to_radians(), 151.2184_f64.to_radians(), 0.0),
        0.02,
        12.0,
    );
    let blue = water_appearance(8.0, &harbour, true).tint;
    assert!(
        blue[2] > blue[1] && blue[1] > blue[0],
        "{blue:?}, arid={}, humidity={}, precipitation={}",
        harbour.climate.arid,
        harbour.climate.humidity,
        harbour.climate.precipitation_mm_day,
    );

    let canyon = test_col(
        [102, 78, 52],
        Climate {
            temp_c: 16.0,
            arid: 0.72,
            precipitation_mm_day: 0.45,
            humidity: 0.34,
        },
        0.43,
        720.0,
    );
    let sediment = water_appearance(8.0, &canyon, true).tint;
    assert!(sediment[1] > sediment[2], "{sediment:?}");
}

#[test]
fn water_depth_signal_creates_distinct_shoal_mid_and_deep_colours() {
    let harbour = test_col(
        [70, 110, 125],
        climate_at(-33.8651_f64.to_radians(), 151.2184_f64.to_radians(), 0.0),
        0.02,
        12.0,
    );
    let shoal = water_appearance(0.5, &harbour, true).tint;
    let middle = water_appearance(6.0, &harbour, true).tint;
    let deep = water_appearance(16.0, &harbour, true).tint;
    assert_ne!(shoal, middle);
    assert_ne!(middle, deep);
    assert!(
        shoal.iter().map(|&channel| u16::from(channel)).sum::<u16>()
            > deep.iter().map(|&channel| u16::from(channel)).sum::<u16>(),
        "{shoal:?} should be brighter than {deep:?}"
    );

    assert!(inland_water_depth(0.0, 0.0) < inland_water_depth(0.0, 1.0));
    assert_eq!(ocean_water_depth(4000.0, 0.0), 12.0);
    assert!((ocean_water_depth(4000.0, 1.0) - 4000.0).abs() < 0.01);
    assert!(ocean_water_depth(0.0, 1.0) > 20.0);
}

#[test]
fn water_palette_is_continuous_across_climate_and_relief_classification_boundaries() {
    let mut cold = Col::EMPTY;
    cold.climate.temp_c = 4.0;
    cold.relief = 120.0;
    let mut mountain = cold;
    mountain.climate.temp_c = 0.0;
    mountain.relief = 60.0;
    let mut dry = Col::EMPTY;
    dry.climate.arid = 0.58;
    dry.climate.humidity = 0.2;
    let mut humid = dry;
    humid.climate.arid = 0.9;
    humid.climate.humidity = 0.45;
    let mut pairs = Vec::new();
    for (mut a, mut b, axis) in [
        (cold, cold, 0),
        (mountain, mountain, 1),
        (dry, dry, 2),
        (humid, humid, 3),
    ] {
        let delta = 0.0001;
        match axis {
            0 => {
                a.climate.temp_c -= delta;
                b.climate.temp_c += delta;
            }
            1 => {
                a.relief -= delta;
                b.relief += delta;
            }
            2 => {
                a.climate.arid -= delta;
                b.climate.arid += delta;
            }
            _ => {
                a.climate.humidity -= delta;
                b.climate.humidity += delta;
            }
        }
        pairs.push((a, b));
    }
    for depth in [0.5, 8.0, 70.0] {
        for (a, b) in &pairs {
            let a = water_appearance(depth, a, true).tint;
            let b = water_appearance(depth, b, true).tint;
            for channel in 0..3 {
                assert!(
                    a[channel].abs_diff(b[channel]) <= 1,
                    "palette jump at {depth}m: {a:?} → {b:?}"
                );
            }
        }
    }
    // Inland family priors must not affect ocean water at the same temperature
    // and humidity, nor overpower the deep-water endpoint.
    let mut ordinary = cold;
    ordinary.relief = 0.0;
    ordinary.climate.arid = 0.0;
    let mut rugged = ordinary;
    rugged.relief = 1000.0;
    rugged.climate.arid = 1.0;
    assert_eq!(
        water_appearance(8.0, &ordinary, false),
        water_appearance(8.0, &rugged, false)
    );
    assert_eq!(
        water_appearance(4000.0, &ordinary, true),
        water_appearance(4000.0, &rugged, true)
    );
}

#[test]
fn ocean_bathymetry_confidence_has_no_one_metre_depth_jump() {
    for shore in [0.0, 0.55, 0.8, 1.0] {
        let a = ocean_water_depth(0.9999, shore);
        let b = ocean_water_depth(1.0001, shore);
        assert!(
            (a - b).abs() < 0.001,
            "optical depth jump at shore {shore}: {a} → {b}"
        );
        let mut previous = ocean_water_depth(0.0, shore);
        for step in 1..=200 {
            let current = ocean_water_depth(step as f32 * 0.01, shore);
            assert!(current.is_finite() && current >= 0.0);
            assert!((current - previous).abs() < 0.5);
            previous = current;
        }
        for bathymetry in [1.0, 12.0, 4000.0] {
            let shelf = 12.0 + 18.35 * smoothstep01(shore);
            let expected = shelf + (bathymetry - shelf) * smoothstep01((shore - 0.55) / 0.45);
            assert_eq!(ocean_water_depth(bathymetry, shore), expected);
        }
    }
    assert!((ocean_water_depth(0.0, 1.0) - 30.35).abs() < 1e-5);
}

#[test]
fn water_catalog_reaches_generated_vertices_without_source_identity_or_wet_coverage_changes() {
    struct MappedLake {
        identity: SourceFeatureIdentity,
        wet: bool,
    }
    impl VectorSource for MappedLake {
        fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
            let mut pack = FeaturePack::empty();
            if self.wet {
                let mut water = Water::inland(vec![vec![
                    [-2000.0, -2000.0],
                    [2000.0, -2000.0],
                    [2000.0, 2000.0],
                    [-2000.0, 2000.0],
                ]]);
                water.identity = self.identity;
                pack.water.push(water);
            }
            pack
        }
    }
    let direction = vearth_core::ellipsoid::geodetic_to_ecef(
        -28.17_f64.to_radians(),
        114.25_f64.to_radians(),
        0.0,
    )
    .normalize();
    let (face, u, v) = vearth_core::cubesphere::dir_to_face_uv(direction);
    let count = (1u32 << 17) as f64;
    let cell = CellId {
        face,
        level: 17,
        i: ((u + 1.0) * 0.5 * count) as u32,
        j: ((v + 1.0) * 0.5 * count) as u32,
    };
    let elevation = FnElevation(|_, _| 10.0);
    let generate = |identity, wet| {
        generate_surface_cell(
            cell,
            &Sources {
                elevation: &elevation,
                imagery: None,
                surface: None,
                vectors: Some(&MappedLake { identity, wet }),
            },
        )
    };
    let a = generate(SourceFeatureIdentity::Provided(2377858110), true);
    let b = generate(SourceFeatureIdentity::DeterministicFallback(0), true);
    assert!(a.complete && b.complete);
    assert!(!a.water.is_empty());
    assert_eq!(a.water, b.water);
    let water_tints = |column: &SurfaceCell| {
        vertices(column)
            .filter(|v| ((v.face_ao >> 5) & 255) as u8 == material::WATER)
            .map(|v| v.rgba)
            .collect::<Vec<_>>()
    };
    let tints = water_tints(&a);
    assert_eq!(tints, water_tints(&b));
    assert!(
        tints
            .iter()
            .all(|c| c[0] > c[1] && c[0] > c[2] && c[3] == 255)
    );
    let dry = generate(SourceFeatureIdentity::Provided(2377858110), false);
    assert!(dry.complete && dry.water.is_empty());
    assert!(water_tints(&dry).is_empty());
}

#[test]
fn ocean_tint_uses_the_same_bathymetry_level_after_refinement() {
    struct LodSensitiveOcean(std::sync::Mutex<Vec<f64>>);

    impl ElevationSource for LodSensitiveOcean {
        fn elevation_m(&self, _lat: f64, _lon: f64, sample_pitch_m: f64) -> f32 {
            self.0.lock().unwrap().push(sample_pitch_m);
            if (sample_pitch_m - WATER_BATHYMETRY_SAMPLE_M).abs() < f64::EPSILON {
                -24.0
            } else {
                -4_000.0
            }
        }
    }

    fn center_water_tint(column: &SurfaceCell) -> [u8; 3] {
        let v = vertices(column)
            .find(|v| ((v.face_ao >> 5) & 255) as u8 == material::WATER)
            .expect("water mesh");
        [v.rgba[0], v.rgba[1], v.rgba[2]]
    }

    let source = LodSensitiveOcean(std::sync::Mutex::new(Vec::new()));
    let parent = mid_cell(11);
    let scale = 1u32 << 4;
    let child = cell(
        15,
        parent.i * scale + scale / 2 - 1,
        parent.j * scale + scale / 2 - 1,
    );
    let parent_column = generate_surface_cell(parent, &Sources::terrain_only(&source));
    let child_column = generate_surface_cell(child, &Sources::terrain_only(&source));

    // The two cells cover different geographic substrate fields. Compare
    // each against a uniform-depth source at its own location/detail so a
    // fine-level NoData/abyss sample cannot override the fixed optical source.
    let uniform = FnElevation(|_, _| -24.0);
    for (cell, column) in [(parent, &parent_column), (child, &child_column)] {
        let expected = generate_surface_cell(cell, &Sources::terrain_only(&uniform));
        assert_eq!(
            center_water_tint(column),
            center_water_tint(&expected),
            "terrain refinement changed the bathymetry driving water colour"
        );
    }
    let fixed_samples = source
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|&&hint| (hint - WATER_BATHYMETRY_SAMPLE_M).abs() < f64::EPSILON)
        .count();
    assert_eq!(fixed_samples, 2 * N * N);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordedLayer {
    Elevation,
    Surface,
    Imagery,
    ImageryDetail,
    ImageryHr,
}

struct RecordingElevation {
    layer: RecordedLayer,
    value: f32,
    samples: std::sync::Mutex<Vec<(RecordedLayer, f64, f64)>>,
}

impl RecordingElevation {
    fn new(layer: RecordedLayer, value: f32) -> Self {
        Self {
            layer,
            value,
            samples: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn points(&self) -> Vec<(RecordedLayer, f64, f64)> {
        self.samples.lock().unwrap().clone()
    }
}

impl ElevationSource for RecordingElevation {
    fn elevation_m(&self, lat: f64, lon: f64, _sample_pitch_m: f64) -> f32 {
        self.samples.lock().unwrap().push((self.layer, lat, lon));
        self.value
    }
}

#[derive(Default)]
struct RecordingImagery {
    samples: std::sync::Mutex<Vec<(RecordedLayer, f64, f64)>>,
}

impl ColorSource for RecordingImagery {
    fn color(&self, lat: f64, lon: f64) -> Option<[u8; 3]> {
        self.samples
            .lock()
            .unwrap()
            .push((RecordedLayer::Imagery, lat, lon));
        Some([80, 100, 120])
    }

    fn color_detail(&self, lat: f64, lon: f64, _sample_pitch_m: f64) -> Option<[u8; 3]> {
        self.samples
            .lock()
            .unwrap()
            .push((RecordedLayer::ImageryDetail, lat, lon));
        Some([85, 105, 125])
    }

    fn color_hr(&self, lat: f64, lon: f64, _sample_pitch_m: f64) -> Option<[u8; 3]> {
        self.samples
            .lock()
            .unwrap()
            .push((RecordedLayer::ImageryHr, lat, lon));
        Some([90, 110, 130])
    }
}

struct FootprintVectors {
    half_e: f64,
    half_n: f64,
}

impl VectorSource for FootprintVectors {
    fn features(&self, _lat0: f64, _lon0: f64, _half_e_m: f64, _half_n_m: f64) -> FeaturePack {
        let (e, n) = (self.half_e as f32 * 0.95, self.half_n as f32 * 0.95);
        FeaturePack {
            buildings: Vec::new(),
            observation_hints: Vec::new(),
            exact_trees: Vec::new(),
            cliffs: Vec::new(),
            roads: vec![
                Road {
                    profile: Default::default(),
                    pts: vec![[-e, -n], [e, n]],
                    class: RoadClass::Major,
                    is_bridge: false,
                },
                Road {
                    profile: Default::default(),
                    pts: vec![[-e, n], [e, -n]],
                    class: RoadClass::Minor,
                    is_bridge: false,
                },
            ],
            water: vec![Water::inland(vec![vec![
                [-e, -n],
                [e, -n],
                [e, n],
                [-e, n],
                [-e, -n],
            ]])],
            watercourses: Vec::new(),
            watercourse_output_mode: vearth_data::vectors::WatercourseOutputMode::DisabledG96,
            water_polygon_mode: vearth_data::vectors::WaterPolygonMode::Typed,
            forest: Vec::new(),
            ground: Vec::new(),
            flora: Vec::new(),
            regional_landcover: Vec::new(),
            regional_landcover_mode:
                vearth_data::vectors::RegionalLandcoverMode::SelectiveRegionalTerrain,
            bridges: Vec::new(),
            bridge_authority: false,
            complete: true,
        }
    }
}

fn assert_samples_inside(
    footprint: SourceFootprint,
    datum: &[SourceFootprint],
    samples: &[(RecordedLayer, f64, f64)],
) {
    for &(layer, lat, lon) in samples {
        assert!(
            footprint.contains(lat, lon) || datum.iter().any(|cap| cap.contains(lat, lon)),
            "{layer:?} sample ({}, {}) lies outside cap center ({}, {}) radius {}",
            lat.to_degrees(),
            lon.to_degrees(),
            footprint.center_lat.to_degrees(),
            footprint.center_lon.to_degrees(),
            footprint.radius_rad,
        );
    }
}

#[test]
fn source_footprint_contains_generation_raster_samples() {
    let target = mid_cell(19);
    let footprint = source_query_footprint(target);
    let frame = ChunkFrame::of(ChunkAddr { cell: target, k: 0 });
    let (_, _, half_e, half_n) = vector_query_bounds(target);
    assert_eq!(
        footprint.sample_hint_m,
        frame.grid_scale.y * SAMPLE_ZOOM_FACTOR
    );

    let elevation = RecordingElevation::new(RecordedLayer::Elevation, 30.0);
    let surface = RecordingElevation::new(RecordedLayer::Surface, 42.0);
    let imagery = RecordingImagery::default();
    let vectors = FootprintVectors { half_e, half_n };
    let (lat, lon, _, _) = vector_query_bounds(target);
    let source_pack = vectors.features(lat, lon, half_e, half_n);
    let datum = built_surface::geometry_source_footprints(target, &source_pack);

    let _ = generate_surface_cell(
        target,
        &Sources {
            elevation: &elevation,
            surface: Some(&surface),
            imagery: Some(&imagery),
            vectors: Some(&vectors),
        },
    );

    let elevation_samples = elevation.points();
    let surface_samples = surface.points();
    let imagery_samples = imagery.samples.lock().unwrap().clone();
    assert!(
        elevation_samples.len() > (N + 1) * (N + 1),
        "water margin and road profile should add elevation samples"
    );
    assert_eq!(
        surface_samples.len(),
        0,
        "this map has no buildings requiring a surface-height sample"
    );
    assert_eq!(
        imagery_samples.len(),
        (N * N + (N + 1) * (N + 1)) * 3,
        "global + bundled-detail + high-res imagery"
    );
    assert_samples_inside(footprint, &datum, &elevation_samples);
    assert_samples_inside(footprint, &datum, &surface_samples);
    assert_samples_inside(footprint, &datum, &imagery_samples);
}

#[test]
fn datum_pitch_caps_the_datum_level_and_keeps_mid_latitudes_unchanged() {
    use vearth_data::terrarium::TerrariumSource;
    for tenth in 0..=850 {
        for lat in [f64::from(tenth) * 0.1, f64::from(tenth) * -0.1] {
            let lat = lat.to_radians();
            let pitch = datum_pitch_m(lat);
            let zoom = TerrariumSource::zoom_for(pitch, lat);
            assert_eq!(
                zoom,
                TerrariumSource::zoom_for(8.0, lat).min(13),
                "{}",
                lat.to_degrees()
            );
            // Callers pass f32 hints, and one prepared cap spans a few km.
            assert_eq!(
                TerrariumSource::zoom_for(f64::from(pitch as f32), lat),
                zoom
            );
            for offset_m in [-5_000.0, 5_000.0] {
                let nearby = lat + offset_m / vearth_core::ellipsoid::MEAN_RADIUS;
                if nearby.abs() < 85f64.to_radians() {
                    assert!(TerrariumSource::zoom_for(pitch, nearby) <= 13);
                }
            }
            if lat.abs() >= 34.5f64.to_radians() {
                assert_eq!(pitch, 8.0, "mid and high latitudes keep their datum");
            }
        }
    }
}

/// Fixed-pitch datum reads on a coarse cell must stay inside the per-feature
/// caps, which must not blanket the whole cell for a few local features.
#[test]
fn coarse_datum_reads_stay_inside_feature_caps() {
    struct HintRecorder(std::sync::Mutex<Vec<(f64, f64, f64)>>);
    impl ElevationSource for HintRecorder {
        fn elevation_m(&self, lat: f64, lon: f64, pitch_m: f64) -> f32 {
            self.0.lock().unwrap().push((lat, lon, pitch_m));
            100.0
        }
    }
    struct CornerFeatures {
        half_e: f32,
        half_n: f32,
    }
    impl VectorSource for CornerFeatures {
        fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
            let (e, n) = (self.half_e, self.half_n);
            let square = |x: f32, z: f32, r: f32| {
                vec![vec![
                    [x - r, z - r],
                    [x + r, z - r],
                    [x + r, z + r],
                    [x - r, z + r],
                    [x - r, z - r],
                ]]
            };
            let mut pack = FeaturePack::empty();
            // A small lake near the north-east corner.
            pack.water
                .push(Water::inland(square(e * 0.7, n * 0.7, e * 0.08)));
            // A few houses near the south-west corner.
            for (i, (x, z)) in [(-0.7, -0.7), (-0.65, -0.72), (-0.72, -0.64)]
                .into_iter()
                .enumerate()
            {
                pack.buildings.push(Building {
                    location_rad: None,
                    roof_shape: None,
                    use_kind: Default::default(),
                    rings: square(e * x, n * z, 6.0),
                    height_m: 6.0,
                    min_height_m: 0.0,
                    seed: 7 + i as u64,
                    kind: vearth_data::vectors::BuildingKind::Building,
                });
            }
            // A river along the western edge.
            pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
            pack.watercourses.push(Watercourse {
                paths: vec![vec![[-e * 0.8, -n * 0.2], [-e * 0.8, n * 0.3]]],
                kind: WatercourseKind::River,
                width_m: WatercourseKind::River.fallback_width_m(),
                identity: SourceFeatureIdentity::Provided(1),
                style_identity: 1,
                source_tile: (15, 0, 0),
                authority_zoom: 15,
                source_feature_index: 0,
                source_min_zoom: None,
                fragment_count: 1,
                intermittent: false,
                is_bridge: false,
            });
            pack
        }
    }

    for level in [12, 14] {
        let target = mid_cell(level);
        let (lat0, lon0, half_e, half_n) = vector_query_bounds(target);
        // Keep the features inside the cell itself, not just the query window.
        let frame = ChunkFrame::of(ChunkAddr { cell: target, k: 0 });
        let cell_half = (frame.grid_scale.x * N as f64 * 0.5) as f32;
        let vectors = CornerFeatures {
            half_e: cell_half,
            half_n: cell_half,
        };
        let pack = vectors.features(lat0, lon0, half_e, half_n);
        let caps = built_surface::geometry_source_footprints(target, &pack);
        assert!(!caps.is_empty(), "L{level}");

        let elevation = HintRecorder(Default::default());
        let _ = generate_surface_cell(
            target,
            &Sources {
                elevation: &elevation,
                surface: None,
                imagery: None,
                vectors: Some(&vectors),
            },
        );
        let datum_reads: Vec<_> = elevation
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|&&(lat, _, pitch)| pitch == datum_pitch_m(lat))
            .map(|&(lat, lon, _)| (lat, lon))
            .collect();
        assert!(
            !datum_reads.is_empty(),
            "L{level}: fixture made no datum reads"
        );
        for (lat, lon) in datum_reads {
            assert!(
                caps.iter().any(|cap| cap.contains(lat, lon)),
                "L{level}: datum read ({}, {}) outside every cap",
                lat.to_degrees(),
                lon.to_degrees()
            );
        }
        // The features occupy two corners and an edge; on a coarse cell the
        // centre and the other two corners need no fixed-detail elevation.
        if level <= 12 {
            for (e, n) in [(0.0, 0.0), (0.7, -0.7), (-0.7, 0.8)] {
                let lat = lat0 + n * f64::from(cell_half) / vearth_core::ellipsoid::MEAN_RADIUS;
                let lon = lon0
                    + e * f64::from(cell_half) / (vearth_core::ellipsoid::MEAN_RADIUS * lat0.cos());
                assert!(
                    !caps.iter().any(|cap| cap.contains(lat, lon)),
                    "L{level}: caps cover featureless ({e}, {n})"
                );
            }
        }
    }
}

#[test]
fn generated_road_corners_preserve_grade_and_join_neighboring_cells() {
    struct RoadMap {
        latitude: f64,
        longitude: f64,
    }
    impl VectorSource for RoadMap {
        fn features(&self, lat: f64, lon: f64, _: f64, _: f64) -> FeaturePack {
            let radius = vearth_core::ellipsoid::MEAN_RADIUS;
            let east = ((self.longitude - lon) * radius * lat.cos()) as f32;
            let north = ((self.latitude - lat) * radius) as f32;
            let mut pack = FeaturePack::empty();
            pack.roads.push(Road {
                profile: Default::default(),
                pts: vec![[east - 200.0, north], [east + 200.0, north]],
                class: RoadClass::Major,
                is_bridge: false,
            });
            pack
        }
    }
    let a = mid_cell(19);
    let b = CellId { i: a.i + 1, ..a };
    let frame = ChunkFrame::of(ChunkAddr { cell: a, k: 0 });
    let (latitude, longitude) = frame.column_latlon(0.5, 0.5);
    let vectors = RoadMap {
        latitude,
        longitude,
    };
    let elevation = FnElevation(|lat: f64, lon: f64| {
        let east = (lon - longitude) * vearth_core::ellipsoid::MEAN_RADIUS * latitude.cos();
        let north = (lat - latitude) * vearth_core::ellipsoid::MEAN_RADIUS;
        (40.0 + east * 0.2 + 2.0 * (north * 0.4).sin()) as f32
    });
    let sources = Sources {
        elevation: &elevation,
        surface: None,
        imagery: None,
        vectors: Some(&vectors),
    };
    let columns = [a, b].map(|cell| generate_surface_cell(cell, &sources));
    for column in &columns {
        let frame = ChunkFrame::of(ChunkAddr {
            cell: column.cell,
            k: 0,
        });
        let collision = column.terrain_collision.as_ref().unwrap();
        let mut graded = 0;
        for z in 0..=N {
            for x in 0..=N {
                let (lat, lon) = frame.column_latlon(x as f64 / N as f64, z as f64 / N as f64);
                let east = (lon - longitude) * vearth_core::ellipsoid::MEAN_RADIUS * latitude.cos();
                let north = (lat - latitude) * vearth_core::ellipsoid::MEAN_RADIUS;
                if north.abs() < 4.0 {
                    let (_, off) = frame.column_surface(x as f64 / N as f64, z as f64 / N as f64);
                    let actual = collision.sample_local(x as f64, z as f64).unwrap()
                        * frame.grid_scale.y
                        - off;
                    assert!((actual - (40.0 + east * 0.2)).abs() < 0.001);
                    graded += 1;
                }
            }
        }
        assert!(graded > 100);
        assert!(has_material(column, material::ASPHALT));
    }
    for z in 0..=N {
        let height = |column: &SurfaceCell, x: f64| {
            let frame = ChunkFrame::of(ChunkAddr {
                cell: column.cell,
                k: 0,
            });
            let (_, off) = frame.column_surface(x / N as f64, z as f64 / N as f64);
            column
                .terrain_collision
                .as_ref()
                .unwrap()
                .sample_local(x, z as f64)
                .unwrap()
                * frame.grid_scale.y
                - off
        };
        assert!((height(&columns[0], N as f64) - height(&columns[1], 0.0)).abs() < 0.001);
    }
}

#[test]
fn road_grading_samples_only_the_local_profile_inside_source_footprint() {
    struct LongRoad;

    impl VectorSource for LongRoad {
        fn features(&self, _lat0: f64, _lon0: f64, _half_e_m: f64, _half_n_m: f64) -> FeaturePack {
            FeaturePack {
                buildings: Vec::new(),
                observation_hints: Vec::new(),
                exact_trees: Vec::new(),
                cliffs: Vec::new(),
                roads: vec![Road {
                    profile: Default::default(),
                    pts: vec![[-1_000.0, 0.0], [1_000.0, 0.0]],
                    class: RoadClass::Major,
                    is_bridge: false,
                }],
                water: Vec::new(),
                watercourses: Vec::new(),
                watercourse_output_mode: vearth_data::vectors::WatercourseOutputMode::DisabledG96,
                water_polygon_mode: vearth_data::vectors::WaterPolygonMode::Typed,
                forest: Vec::new(),
                ground: Vec::new(),
                flora: Vec::new(),
                regional_landcover: Vec::new(),
                regional_landcover_mode:
                    vearth_data::vectors::RegionalLandcoverMode::SelectiveRegionalTerrain,
                bridges: Vec::new(),
                bridge_authority: false,
                complete: true,
            }
        }
    }

    for (latitude, longitude) in [
        (0.0_f64, 0.0_f64),
        (60.3913, 5.3336),
        (78.0, 16.0),
        (-78.0, -179.999),
        (89.0, 5.3336),
    ] {
        let direction = vearth_core::ellipsoid::geodetic_to_ecef(
            latitude.to_radians(),
            longitude.to_radians(),
            0.0,
        )
        .normalize();
        let target = CellId::from_dir(direction, 19);
        let footprint = source_query_footprint(target);
        let elevation = RecordingElevation::new(RecordedLayer::Elevation, 30.0);
        let _ = generate_surface_cell(
            target,
            &Sources {
                elevation: &elevation,
                surface: None,
                imagery: None,
                vectors: Some(&LongRoad),
            },
        );
        let samples = elevation.points();
        assert_samples_inside(footprint, &[], &samples);
        assert!(
            samples.len() < (N + 3) * (N + 3) + 100,
            "remote portions of a tile-spanning road were still sampled: {} calls",
            samples.len()
        );
    }
}

#[test]
fn source_footprint_contains_terrain_landmark_base_sample() {
    let lm = landmarks::LANDMARKS
        .iter()
        .find(|landmark| landmark.name == "eiffel-tower")
        .unwrap();
    let lat = lm.lat_deg.to_radians();
    let lon = lm.lon_deg.to_radians();
    let dir = vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0).normalize();
    let target = CellId::from_dir(dir, 16);
    let footprint = source_query_footprint(target);
    let elevation = RecordingElevation::new(RecordedLayer::Elevation, 35.0);

    let _ = generate_surface_cell(target, &Sources::terrain_only(&elevation));
    let samples = elevation.points();
    assert!(
        samples.iter().any(|&(_, sample_lat, sample_lon)| {
            (sample_lat - lat).abs() < 1.0e-14 && (sample_lon - lon).abs() < 1.0e-14
        }),
        "terrain-backed Eiffel base was not sampled"
    );
    assert_samples_inside(footprint, &[], &samples);
}

#[test]
fn source_footprint_bounds_high_latitude_windows_without_longitude_inflation() {
    for (latitude, longitude) in [
        (60.3913_f64, 5.3336_f64),
        (78.0, 16.0),
        (-78.0, -179.999),
        (85.0, 179.999),
        (89.0, 5.3336),
        (-89.0, -179.999),
    ] {
        for level in [9, 13, 16, 19] {
            let direction = vearth_core::ellipsoid::geodetic_to_ecef(
                latitude.to_radians(),
                longitude.to_radians(),
                0.0,
            )
            .normalize();
            let target = CellId::from_dir(direction, level);
            let footprint = source_query_footprint(target);
            let (lat0, lon0, half_e, half_n) = vector_query_bounds(target);
            let radius = vearth_core::ellipsoid::MEAN_RADIUS;
            let previous_bound =
                (half_n / radius + half_e / (radius * lat0.cos().abs())).min(std::f64::consts::PI);
            if lat0.abs() + footprint.radius_rad >= 1.484_422 {
                assert!(
                    footprint.radius_rad >= previous_bound,
                    "polar provider conversion needs the established bound for {target:?}"
                );
            } else {
                assert!(
                    footprint.radius_rad < previous_bound * 0.8,
                    "high-latitude cap remains unnecessarily wide for {target:?}"
                );
            }
            for north in 0..=32 {
                for east in 0..=32 {
                    let lat = lat0 + (north as f64 / 16.0 - 1.0) * half_n / radius;
                    let lon = lon0 + (east as f64 / 16.0 - 1.0) * half_e / (radius * lat0.cos());
                    assert!(
                        footprint.contains(lat, lon),
                        "vector-window sample escaped {target:?} at ({east}, {north})"
                    );
                }
            }
        }
    }
}

#[test]
fn source_footprint_covers_bounded_domains_across_the_planet() {
    let levels = [0u8, 3, 9, 13, 16, 19];
    for face in 0..6 {
        for level in levels {
            let n = 1u32 << level;
            let last = n - 1;
            let positions = [(0, 0), (last, 0), (0, last), (last, last), (n / 2, n / 2)];
            for (i, j) in positions {
                let target = CellId {
                    face: Face(face),
                    level,
                    i,
                    j,
                };
                let frame = ChunkFrame::of(ChunkAddr { cell: target, k: 0 });
                let footprint = source_query_footprint(target);
                assert_eq!(
                    footprint.sample_hint_m,
                    frame.grid_scale.y * SAMPLE_ZOOM_FACTOR
                );

                // The complete maximum water-margin lattice contains all
                // terrain corners generation can data-dependently request.
                let margin = tier0::WATER_MARGIN_COLS as i64;
                for z in -margin..=N as i64 + margin {
                    for x in -margin..=N as i64 + margin {
                        let (lat, lon) =
                            frame.column_latlon(x as f64 / N as f64, z as f64 / N as f64);
                        assert!(
                            footprint.contains(lat, lon),
                            "water-margin sample escaped {target:?} at ({x}, {z})"
                        );
                    }
                }

                // DSM uses every interior center plus a four-point ring.
                let ring = frame.grid_scale.x * 0.35;
                for z in 0..N {
                    for x in 0..N {
                        let (lat, lon) = frame.column_latlon(
                            (x as f64 + 0.5) / N as f64,
                            (z as f64 + 0.5) / N as f64,
                        );
                        assert!(footprint.contains(lat, lon));
                        let dlat = ring / vearth_core::ellipsoid::MEAN_RADIUS;
                        let dlon =
                            ring / (vearth_core::ellipsoid::MEAN_RADIUS * lat.cos().max(1.0e-6));
                        for (sa, so) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                            assert!(
                                footprint.contains(lat + sa * dlat, lon + so * dlon),
                                "DSM ring escaped {target:?}"
                            );
                        }
                    }
                }

                // Road profiles may use any point in the advertised
                // vector query window; cover its corners and axes.
                let (lat0, lon0, half_e, half_n) = vector_query_bounds(target);
                for north in [-half_n, 0.0, half_n] {
                    for east in [-half_e, 0.0, half_e] {
                        let lat = lat0 + north / vearth_core::ellipsoid::MEAN_RADIUS;
                        let lon = if lat0.cos().abs() <= 1.0e-12 {
                            lon0
                        } else {
                            lon0 + east / (vearth_core::ellipsoid::MEAN_RADIUS * lat0.cos())
                        };
                        assert!(
                            footprint.contains(lat, lon),
                            "vector-window sample escaped {target:?}"
                        );
                    }
                }
            }
        }
    }
}

/// Terrarium dependency contract copied independently so changes to the
/// richer provider footprint cannot silently alter this exact plan.
fn expected_required_tiles(cell: CellId) -> Vec<(u8, u32, u32)> {
    use vearth_data::terrarium::{TerrariumSource, sampling_tiles};

    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let sample_hint_m = frame.grid_scale.y * SAMPLE_ZOOM_FACTOR;
    let mut tiles = std::collections::BTreeSet::new();
    for z in -1..=N as i64 + 1 {
        for x in -1..=N as i64 + 1 {
            let (lat, lon) = frame.column_latlon(x as f64 / N as f64, z as f64 / N as f64);
            let zoom = TerrariumSource::zoom_for(sample_hint_m, lat);
            tiles.extend(sampling_tiles(lat, lon, zoom));
        }
    }
    if cell.level >= WATER_BATHYMETRY_MIN_LEVEL {
        for z in 0..N {
            for x in 0..N {
                let (lat, lon) =
                    frame.column_latlon((x as f64 + 0.5) / N as f64, (z as f64 + 0.5) / N as f64);
                let zoom = TerrariumSource::zoom_for(WATER_BATHYMETRY_SAMPLE_M, lat);
                tiles.extend(sampling_tiles(lat, lon, zoom));
            }
        }
    }
    tiles.into_iter().collect()
}

#[test]
fn source_footprint_preserves_required_tiles_contract() {
    for face in 0..6 {
        for level in [0u8, 3, 9, 15, 19] {
            let n = 1u32 << level;
            for (i, j) in [(0, 0), (n / 2, n / 2), (n - 1, n - 1)] {
                let target = CellId {
                    face: Face(face),
                    level,
                    i,
                    j,
                };
                let expected = expected_required_tiles(target);
                let _ = source_query_footprint(target);
                assert_eq!(required_tiles(target), expected, "{target:?}");
            }
        }
    }
}

#[test]
fn flat_plain_generates_only_a_grass_surface() {
    let src = FnElevation(|_, _| 120.0);
    let col = generate_surface_cell(mid_cell(12), &Sources::terrain_only(&src));
    assert_eq!(col.terrain_step_m, 0.0);
    assert_eq!(col.batches.len(), 1);
    assert!(has_material(&col, material::GRASS));
    assert!(col.water.is_empty());
    assert!(
        col.terrain_collision
            .as_ref()
            .unwrap()
            .source_columns
            .iter()
            .all(|&row| row == u32::MAX)
    );
    assert!(col.solid_collision.triangles().is_empty());
    assert!(col.solid_collision.validate().is_ok());
}

#[test]
fn regional_lod_keeps_geodetic_height_unexaggerated() {
    let src = FnElevation(|_, _| 2200.0);
    for level in [5, 8, 11, 15, 19] {
        let cell = generate_surface_cell(mid_cell(level), &Sources::terrain_only(&src));
        let batch = &cell.batches[0];
        let frame = ChunkFrame::of(batch.addr);
        // The first surface vertex precedes seam skirts.
        let p = frame.local_to_ecef(DVec3::from_array(batch.mesh.vertices[0].pos.map(f64::from)));
        let (_, _, alt) = vearth_core::ellipsoid::ecef_to_geodetic(p);
        assert!((alt - 2200.0).abs() < 2.0, "L{level}: {alt}");
    }
}

#[test]
fn deep_ocean_generates_surface_and_seabed_only() {
    let shallow = FnElevation(|_, _| -20.0);
    let deep = FnElevation(|_, _| -4000.0);
    let a = generate_surface_cell(mid_cell(15), &Sources::terrain_only(&shallow));
    let b = generate_surface_cell(mid_cell(15), &Sources::terrain_only(&deep));
    assert_eq!(a.batches.len(), 2);
    assert_eq!(b.batches.len(), 2);
    assert_eq!(
        a.solid_collision.triangles().len(),
        b.solid_collision.triangles().len()
    );
    assert_eq!(a.water.len(), b.water.len());
    assert!(has_material(&b, material::WATER));
    assert!(b.height_range.0 <= -4000.0 && b.height_range.1 >= 0.0);
}

#[test]
fn mountain_surface_and_collision_retain_full_relief() {
    // The compact heightfield must preserve the complete steep source relief.
    let target = mid_cell(13);
    let frame = ChunkFrame::of(ChunkAddr { cell: target, k: 0 });
    let (clat, clon) = frame.column_latlon(0.5, 0.5);
    let radius = frame.grid_scale.x * 20.0;
    let src = FnElevation(move |lat: f64, lon: f64| {
        let d = ((lat - clat).powi(2) + ((lon - clon) * clat.cos()).powi(2)).sqrt()
            * vearth_core::ellipsoid::MEAN_RADIUS;
        (2200.0 * (1.0 - d / radius)).max(10.0) as f32
    });
    let col = generate_surface_cell(target, &Sources::terrain_only(&src));
    assert!(
        col.terrain_step_m > 25.0,
        "mountain step metric was too small: {} m",
        col.terrain_step_m
    );
    assert!(col.solid_collision.triangles().is_empty());
    let ground = col.terrain_collision.as_ref().unwrap();
    assert!(ground.sample_local(16.0, 16.0).unwrap() * frame.grid_scale.y > 2100.0);
    assert!(col.height_range.1 >= 2199.0);
}

#[test]
fn high_elevation_without_snow_evidence_does_not_impose_a_global_snowline() {
    let src = FnElevation(|_, _| 4200.0);
    let n = 1u32 << 12;
    let col = generate_surface_cell(
        cell(12, n / 2, (n as f64 * 0.9) as u32),
        &Sources::terrain_only(&src),
    );
    let has_snow = has_material(&col, material::SNOW) || has_material(&col, material::ICE);
    assert!(
        !has_snow,
        "height alone must not author persistent snow without source or climate support"
    );
}

#[test]
fn landmarks_stamp_into_generation() {
    // Generate the L16 cell containing the Opera House anchor over flat
    // sea-level terrain: shell voxels (MARBLE) must appear, well above
    // the ground, purely from the landmark pack (no vectors at all).
    let lm = landmarks::LANDMARKS
        .iter()
        .find(|l| l.name == "sydney-opera-house")
        .unwrap();
    let dir = vearth_core::ellipsoid::geodetic_to_ecef(
        lm.lat_deg.to_radians(),
        lm.lon_deg.to_radians(),
        0.0,
    )
    .normalize();
    let c = CellId::from_dir(dir, 16);
    let src = FnElevation(|_, _| 0.5);
    let col = generate_surface_cell(c, &Sources::terrain_only(&src));
    assert!(count_material(&col, material::MARBLE) > 50);
    let again = generate_surface_cell(c, &Sources::terrain_only(&src));
    assert_eq!(
        contracts::surface_cell_fingerprint(&col),
        contracts::surface_cell_fingerprint(&again)
    );
}

#[test]
fn merlion_fine_level_has_direct_surface_geometry() {
    let lm = landmarks::LANDMARKS
        .iter()
        .find(|landmark| landmark.name == "singapore-merlion")
        .unwrap();
    let dir = vearth_core::ellipsoid::geodetic_to_ecef(
        lm.lat_deg.to_radians(),
        lm.lon_deg.to_radians(),
        0.0,
    )
    .normalize();
    let cell = CellId::from_dir(dir, MAX_LEVEL + 1);
    let elevation = FnElevation(|_, _| 0.5);
    let column = generate_surface_cell(cell, &Sources::terrain_only(&elevation));
    assert!(count_material(&column, material::MARBLE) > 100);
    assert!(column.solid_collision.validate().is_ok());
}

#[test]
fn golden_gate_suppresses_source_bridge_tower_buildings() {
    struct AnchorBuilding {
        lat: f64,
        lon: f64,
    }

    impl VectorSource for AnchorBuilding {
        fn features(&self, query_lat: f64, query_lon: f64, _: f64, _: f64) -> FeaturePack {
            let mut dlon = self.lon - query_lon;
            if dlon > std::f64::consts::PI {
                dlon -= std::f64::consts::TAU;
            } else if dlon < -std::f64::consts::PI {
                dlon += std::f64::consts::TAU;
            }
            let east = dlon * query_lat.cos() * vearth_core::ellipsoid::MEAN_RADIUS;
            let north = (self.lat - query_lat) * vearth_core::ellipsoid::MEAN_RADIUS;
            let half = 18.0f32;
            let (east, north) = (east as f32, north as f32);
            let mut pack = FeaturePack::empty();
            pack.buildings.push(Building {
                location_rad: None,
                roof_shape: None,
                use_kind: Default::default(),
                rings: vec![vec![
                    [east - half, north - half],
                    [east + half, north - half],
                    [east + half, north + half],
                    [east - half, north + half],
                ]],
                height_m: 260.0,
                min_height_m: 0.0,
                seed: 0x4747,
                kind: tier0::BuildingKind::Building,
            });
            pack
        }
    }

    let lm = landmarks::LANDMARKS
        .iter()
        .find(|landmark| landmark.name == "golden-gate-bridge")
        .unwrap();
    let lat = lm.lat_deg.to_radians();
    let lon = lm.lon_deg.to_radians();
    let cell = CellId::from_dir(
        vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0).normalize(),
        16,
    );
    let elevation = FnElevation(|_, _| 0.0);
    let vectors = AnchorBuilding { lat, lon };
    let column = generate_surface_cell(
        cell,
        &Sources {
            elevation: &elevation,
            surface: None,
            imagery: None,
            vectors: Some(&vectors),
        },
    );
    assert!(has_material(&column, material::STEEL));
    assert!(!vertices(&column).any(|v| {
        let m = ((v.face_ao >> 5) & 255) as u8;
        material::is_architecture_wall(m)
            || material::is_architecture_roof(m)
            || matches!(m, material::WINDOW | material::LIT_WINDOW)
    }));
}

#[test]
fn source_corner_peak_extends_height_range_beyond_column_averages() {
    let cell = mid_cell(17);
    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let (peak_lat, peak_lon) = frame.column_latlon(17.0 / N as f64, 19.0 / N as f64);
    let elevation = FnElevation(move |lat: f64, lon: f64| {
        if (lat - peak_lat).abs() < 1.0e-12 && (lon - peak_lon).abs() < 1.0e-12 {
            160.125
        } else {
            40.0
        }
    });
    let column = generate_surface_cell(cell, &Sources::terrain_only(&elevation));
    let surface = column.terrain_collision.as_ref().unwrap();
    for z in 0..=N {
        for x in 0..=N {
            let local_y = surface.sample_local(x as f64, z as f64).unwrap();
            let (_, offset_m) = frame.column_surface(x as f64 / N as f64, z as f64 / N as f64);
            let rendered_m = local_y * frame.grid_scale.y - offset_m;
            assert!(rendered_m >= f64::from(column.height_range.0));
            assert!(rendered_m <= f64::from(column.height_range.1));
        }
    }
    let quantum_m = frame.grid_scale.y / 64.0;
    assert!(column.height_range.1 >= 160.125);
    assert!(f64::from(column.height_range.1) <= 160.125 + quantum_m);
    assert!(column.height_range.0 <= 40.0);
}

#[test]
fn source_height_range_preserves_conditioned_bounds_and_excludes_protected_corners() {
    let frame = ChunkFrame::of(ChunkAddr {
        cell: mid_cell(17),
        k: 0,
    });
    let mut source = TerrainSurfaceData {
        shading: None,
        heights_m: Box::new([9_000.0; (N + 1) * (N + 1)]),
        source_columns: [0; N],
    };
    let conditioned = (-35.0, 450.0);
    assert_eq!(
        include_source_height_range(conditioned, &frame, &source),
        conditioned
    );
    source.source_columns[10] = 1 << 12;
    for (x, z) in [(12, 10), (13, 10), (12, 11), (13, 11)] {
        source.heights_m[z * (N + 1) + x] = 20.0;
    }
    assert_eq!(
        include_source_height_range(conditioned, &frame, &source),
        conditioned
    );
    let bounded = include_source_height_range((-5.0, 10.0), &frame, &source);
    assert_eq!(bounded.0, -5.0);
    assert!((20.0..21.0).contains(&bounded.1));
}

#[test]
fn generation_is_deterministic() {
    let src = FnElevation(|lat: f64, lon: f64| {
        (120.0 + (lat * 30.0).sin() * 20.0 + (lon * 10.0).cos() * 15.0) as f32
    });
    let a = generate_surface_cell(mid_cell(10), &Sources::terrain_only(&src));
    let b = generate_surface_cell(mid_cell(10), &Sources::terrain_only(&src));
    assert_eq!(
        contracts::surface_cell_fingerprint(&a),
        contracts::surface_cell_fingerprint(&b)
    );
}

/// End-to-end bridge synthesis: a Major way flagged
/// `is_bridge` across a 300 m river must generate an elevated deck with
/// superstructure — not the legacy flat deck painted on the water.
#[test]
fn bridge_over_water_synthesizes_elevated_typed_deck() {
    struct BridgeVectors;
    impl VectorSource for BridgeVectors {
        fn features(&self, _lat0: f64, _lon0: f64, _half_e_m: f64, _half_n_m: f64) -> FeaturePack {
            let mut pack = FeaturePack::empty();
            pack.water.push(Water::inland(vec![vec![
                [-150.0, -2000.0],
                [150.0, -2000.0],
                [150.0, 2000.0],
                [-150.0, 2000.0],
            ]]));
            pack.roads.push(Road {
                profile: Default::default(),
                pts: vec![[-400.0, 0.0], [400.0, 0.0]],
                class: RoadClass::Major,
                is_bridge: true,
            });
            pack.bridges.push(CanonicalBridge {
                material: Default::default(),
                id: 1,
                class: RoadClass::Major,
                alignment: vec![[-400.0, 0.0], [400.0, 0.0]],
                deck_outline: Vec::new(),
                bridge_start_m: 235.0,
                bridge_end_m: 565.0,
                water_start_m: 250.0,
                water_end_m: 550.0,
                deck_width_m: 18.0,
                road_width_m: 14.0,
                water_is_ocean: false,
                structure: BridgeStructure::Suspension,
            });
            pack.bridge_authority = true;
            pack
        }
    }

    // Flat 5 m terrain: river surface and banks share the elevation, so
    // any deck lift comes from the synthesized clearance alone. L17: the
    // cell sits mid-river; the legacy deck voxel (water + 2 voxels) and
    // the synthesized one land in clearly distinct voxel layers.
    let src = FnElevation(|_, _| 5.0);
    let cell = mid_cell(17);
    let column = generate_surface_cell(
        cell,
        &Sources {
            elevation: &src,
            surface: None,
            imagery: None,
            vectors: Some(&BridgeVectors),
        },
    );
    let water = material_height_at(&column, 16.0, 16.0, material::WATER).unwrap();
    let deck = material_height_at(&column, 16.0, 16.0, material::ASPHALT).unwrap();
    assert!(deck >= water + 10.0, "deck {deck} water {water}");
    assert!(has_material(&column, material::STEEL));
    assert!(column.solid_collision.validate().is_ok());
}

#[test]
fn uncatalogued_crossings_render_continuously_at_each_terrain_lod() {
    struct MappedCrossing;
    impl VectorSource for MappedCrossing {
        fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
            let mut pack = FeaturePack::empty();
            pack.water.push(Water::inland(vec![vec![
                [-150.0, -2_000.0],
                [150.0, -2_000.0],
                [150.0, 2_000.0],
                [-150.0, 2_000.0],
            ]]));
            pack.roads.push(Road {
                profile: Default::default(),
                pts: vec![[-400.0, -40.0], [400.0, 40.0]],
                class: RoadClass::Major,
                is_bridge: true,
            });
            pack
        }
    }
    let elevation = FnElevation(|_, _| 5.0);
    let mut deck_heights = Vec::new();
    for level in [15, 17, 19] {
        let cell = mid_cell(level);
        let column = generate_surface_cell(
            cell,
            &Sources {
                elevation: &elevation,
                surface: None,
                imagery: None,
                vectors: Some(&MappedCrossing),
            },
        );
        let deck = material_height_at(&column, 16.0, 16.0, material::ASPHALT).unwrap();
        let water = material_height_at(&column, 16.0, 16.0, material::WATER).unwrap();
        assert!(deck > water + 1.0);
        assert!(column.solid_collision.validate().is_ok());
        deck_heights.push(deck);
    }
    let lo = deck_heights.iter().copied().reduce(f64::min).unwrap();
    let hi = deck_heights.iter().copied().reduce(f64::max).unwrap();
    assert!(
        hi - lo < 0.10,
        "deck height depends on voxel size: {deck_heights:?}"
    );
}

#[test]
fn line_only_watercourse_crossings_keep_water_and_elevated_bridge_authority() {
    struct CrossingVectors {
        bridge: bool,
    }

    impl VectorSource for CrossingVectors {
        fn features(&self, _lat0: f64, _lon0: f64, _half_e_m: f64, _half_n_m: f64) -> FeaturePack {
            let mut pack = FeaturePack::empty();
            pack.watercourse_output_mode = WatercourseOutputMode::RiversAndCanals;
            pack.watercourses.push(Watercourse {
                paths: vec![vec![[0.0, -2_000.0], [0.0, 2_000.0]]],
                kind: WatercourseKind::River,
                width_m: WatercourseKind::River.fallback_width_m(),
                identity: SourceFeatureIdentity::Provided(7),
                style_identity: 7,
                source_tile: (15, 0, 0),
                authority_zoom: 15,
                source_feature_index: 0,
                source_min_zoom: None,
                fragment_count: 1,
                intermittent: false,
                is_bridge: false,
            });
            pack.roads.push(Road {
                profile: Default::default(),
                pts: vec![[-400.0, 0.0], [400.0, 0.0]],
                class: RoadClass::Major,
                is_bridge: self.bridge,
            });
            if self.bridge {
                pack.bridges.push(CanonicalBridge {
                    material: Default::default(),
                    id: 9,
                    class: RoadClass::Major,
                    alignment: vec![[-400.0, 0.0], [400.0, 0.0]],
                    deck_outline: Vec::new(),
                    bridge_start_m: 235.0,
                    bridge_end_m: 565.0,
                    water_start_m: 250.0,
                    water_end_m: 550.0,
                    deck_width_m: 18.0,
                    road_width_m: 14.0,
                    water_is_ocean: false,
                    structure: BridgeStructure::Suspension,
                });
                pack.bridge_authority = true;
            }
            pack
        }
    }

    fn material_high(column: &SurfaceCell, x: usize, z: usize, wanted: u8) -> Option<f64> {
        material_height_at(column, x as f64, z as f64, wanted)
    }
    let elevation = FnElevation(|_, _| 5.0);
    let cell = mid_cell(17);
    let generate = |bridge| {
        generate_surface_cell(
            cell,
            &Sources {
                elevation: &elevation,
                surface: None,
                imagery: None,
                vectors: Some(&CrossingVectors { bridge }),
            },
        )
    };
    let ordinary = generate(false);
    let bridged = generate(true);
    for column in [&ordinary, &bridged] {
        for z in [N / 4, N / 2, N * 3 / 4] {
            assert!(
                material_high(column, N / 2, z, material::WATER).is_some(),
                "line-only river broke at or beside the road crossing"
            );
        }
    }

    let _ordinary_water = material_high(&ordinary, N / 2, N / 2, material::WATER).unwrap();
    assert!(
        material_high(&ordinary, N / 2, N / 2, material::ASPHALT).is_none(),
        "at-grade road incorrectly covered line water at the crossing"
    );

    let bridge_water = material_high(&bridged, N / 2, N / 2, material::WATER).unwrap();
    let bridge_deck = material_high(&bridged, N / 2, N / 2, material::ASPHALT).unwrap();
    assert!(
        bridge_deck >= bridge_water + 10.0,
        "line-water bridge deck {bridge_deck} does not clear water {bridge_water}"
    );
    assert!(has_material(&bridged, material::STEEL));
}

/// Stable bytewise digest for fixed-input migration parity. This hashes
/// the canonical column payload rather than its in-memory enum layout.

#[test]
fn required_tiles_covers_generation_and_memory_store_matches() {
    use std::sync::Arc;
    use std::sync::Mutex;
    use vearth_data::TileFetcher;
    use vearth_data::terrarium::{DEFAULT_URL, MemoryTiles, TILE_SIZE, TerrariumSource};

    // Fetcher serving a synthetic gradient tile for ANY url, recording
    // which tiles were requested.
    struct Recording {
        urls: Mutex<Vec<String>>,
        png: Vec<u8>,
    }
    impl TileFetcher for Recording {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, vearth_data::DataError> {
            self.urls.lock().unwrap().push(url.to_string());
            Ok(self.png.clone())
        }
    }
    // Encode one synthetic terrarium tile (h = 500 m everywhere).
    let mut rgb = vec![0u8; TILE_SIZE * TILE_SIZE * 3];
    for px in rgb.chunks_mut(3) {
        let v = (500.0f64 + 32768.0) as u32;
        px[0] = (v / 256) as u8;
        px[1] = (v % 256) as u8;
    }
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, TILE_SIZE as u32, TILE_SIZE as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    }

    let cell = mid_cell(9);
    let rec = Arc::new(Recording {
        urls: Mutex::new(Vec::new()),
        png: png.clone(),
    });
    let src = TerrariumSource::new(rec.clone(), DEFAULT_URL, None);
    let via_fetch = generate_surface_cell(cell, &Sources::terrain_only(&src));

    // Every fetched tile must be predicted by required_tiles.
    let predicted = required_tiles(cell);
    let fetched: Vec<String> = rec.urls.lock().unwrap().clone();
    assert!(!fetched.is_empty());
    for url in &fetched {
        let suffix = url.strip_prefix(&format!("{DEFAULT_URL}/")).unwrap();
        let parts: Vec<u32> = suffix
            .trim_end_matches(".webp")
            .split('/')
            .map(|s| s.parse().unwrap())
            .collect();
        let id = (parts[0] as u8, parts[1], parts[2]);
        assert!(predicted.contains(&id), "unpredicted fetch: {id:?}");
    }

    // A MemoryTiles store filled with exactly the predicted tiles must
    // generate identical chunks.
    let mem = MemoryTiles::default();
    for (z, x, y) in &predicted {
        mem.insert_png(*z, *x, *y, &png).unwrap();
    }
    let via_mem = generate_surface_cell(cell, &Sources::terrain_only(&mem));
    assert_eq!(
        contracts::surface_cell_fingerprint(&via_fetch),
        contracts::surface_cell_fingerprint(&via_mem)
    );
}

#[test]
fn surface_cell_has_geometry() {
    let src = FnElevation(|_, _| 120.0);
    let cell = generate_surface_cell(mid_cell(11), &Sources::terrain_only(&src));
    assert!(cell.batches.iter().any(|b| !b.mesh.indices.is_empty()));
    assert!(cell.solid_collision.validate().is_ok());
}

#[test]
fn all_thirty_destinations_stamp_their_intended_location() {
    use crate::flower_destinations::{DestinationMode, SurfaceMask};
    use vearth_data::vectors::FloraRegionKind;

    for destination in &crate::flower_destinations::FLOWER_DESTINATIONS {
        let center = destination_cell(destination, 19);
        let center_frame = ChunkFrame::of(ChunkAddr { cell: center, k: 0 });
        assert!(cell_may_intersect_flower_destination(center, &center_frame));
        let mut cols = [[Col::EMPTY; N]; N];
        for col in cols.iter_mut().flatten() {
            col.elev = 1_800.0;
            col.top = 1_800.0;
            col.climate = Climate {
                temp_c: 15.0,
                arid: 0.18,
                precipitation_mm_day: 3.0,
                humidity: 0.74,
            };
        }
        let radius = if destination.mode == DestinationMode::EcologicalBoost {
            4
        } else {
            0
        };
        let mut stamped = false;
        'cells: for dj in -radius..=radius {
            for di in -radius..=radius {
                let cell = CellId {
                    i: center.i.saturating_add_signed(di),
                    j: center.j.saturating_add_signed(dj),
                    ..center
                };
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                let mut overlay = tier0::CellOverlay::empty();
                if destination.mode == DestinationMode::MappedFieldOverride {
                    let kind = if destination.allowed_surfaces.contains(SurfaceMask::FARMLAND) {
                        FloraRegionKind::Farmland
                    } else {
                        FloraRegionKind::Meadow
                    };
                    for col in overlay.cols.iter_mut().flatten() {
                        col.ground = Some(GroundKind::Green);
                        col.flora = Some(tier0::FloraStamp {
                            location_rad: None,
                            kind,
                            seed: 77,
                            curated_style: 0,
                            surface: flora::select_field_surface(kind, 77),
                        });
                    }
                }
                apply_curated_flower_destinations(cell, &frame, &mut overlay, &cols);
                stamped = overlay.cols.iter().flatten().any(|col| {
                    col.flora
                        .is_some_and(|stamp| stamp.curated_style == destination.id as u8)
                });
                if stamped {
                    break 'cells;
                }
            }
        }
        assert!(
            stamped,
            "{} did not stamp a bounded eligible patch near its center",
            destination.name
        );
    }
}

#[test]
fn curated_stamps_respect_physical_exclusions() {
    let destination = crate::flower_destinations::flower_destination(
        crate::flower_destinations::FlowerDestinationId::FarmTomita,
    );
    let cell = destination_cell(destination, 19);
    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let mut overlay = tier0::CellOverlay::empty();
    for col in overlay.cols.iter_mut().flatten() {
        col.ground = Some(GroundKind::Paved);
        col.water = true;
    }
    let cols = [[Col::EMPTY; N]; N];
    apply_curated_flower_destinations(cell, &frame, &mut overlay, &cols);
    assert!(
        overlay.cols.iter().flatten().all(|col| col.flora.is_none()),
        "curated exact sites must not override water or paving"
    );
}

#[test]
fn farm_tomita_descriptor_carries_authored_style_without_solid_geometry() {
    let destination = crate::flower_destinations::flower_destination(
        crate::flower_destinations::FlowerDestinationId::FarmTomita,
    );
    let cell = destination_cell(destination, 19);
    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let mut overlay = tier0::CellOverlay::empty();
    let mut cols = [[Col::EMPTY; N]; N];
    for col in cols.iter_mut().flatten() {
        col.elev = 180.0;
        col.top = 180.0;
        col.climate = Climate {
            temp_c: 20.0,
            arid: 0.2,
            precipitation_mm_day: 1.5,
            humidity: 0.65,
        };
    }
    apply_curated_flower_destinations(cell, &frame, &mut overlay, &cols);
    let descriptors = build_flora_descriptors(cell, &frame, Some(&overlay), &cols, &[], None);
    let descriptor = descriptors
        .iter()
        .find(|descriptor| descriptor.curated_style == destination.id as u8)
        .expect("Farm Tomita exact-site descriptor");
    assert_eq!(descriptor.archetype, flora::FloraArchetype::FlowerCrop);
    assert_eq!(
        descriptor.calendar,
        flora::CultivationCalendar::CuratedBloom
    );
    assert_eq!(descriptor.base_density, destination.density);
    assert_eq!(
        descriptor.phenology_offset,
        flora::curated_phenology_offset(destination)
    );
    assert!(descriptor.covered_samples() > 0);
    assert!(overlay.tree_instances.is_empty() && overlay.tree_instances.is_empty());
}

#[test]
fn curated_flower_ground_keeps_neutral_substrate_at_every_terrain_lod() {
    use vearth_data::vectors::FloraRegionKind;

    let destination = crate::flower_destinations::flower_destination(
        crate::flower_destinations::FlowerDestinationId::FarmTomita,
    );
    let seed = 0x464c_4f57_4552_4636;
    let stamp = tier0::FloraStamp {
        location_rad: None,
        kind: FloraRegionKind::Garden,
        seed,
        curated_style: destination.id as u8,
        surface: flora::select_field_surface(FloraRegionKind::Garden, seed),
    };
    let col = test_col(
        [86, 112, 64],
        Climate {
            temp_c: 20.0,
            arid: 0.2,
            precipitation_mm_day: 1.5,
            humidity: 0.65,
        },
        0.03,
        4.0,
    );
    let palette = flora::curated_palette_linear(stamp.curated_style).unwrap();

    let mut weights = Vec::new();
    for level in [19, 18, 17, 16] {
        let cell = destination_cell(destination, level);
        let sample = flora::sample_field_column(stamp.surface, seed, cell, N / 2, N / 2);
        let authored = palette[usize::from(sample.flower_variant)].map(|value| value * 255.0);
        let terrain = terrain_appearance(&col, col.elev, 0.5, 0.0)
            .tint
            .map(f32::from);
        let actual = field_surface_appearance(stamp, &col, cell, N / 2, N / 2)
            .tint
            .map(f32::from);
        let authored_delta = [
            authored[0] - terrain[0],
            authored[1] - terrain[1],
            authored[2] - terrain[2],
        ];
        let actual_delta = [
            actual[0] - terrain[0],
            actual[1] - terrain[1],
            actual[2] - terrain[2],
        ];
        let denom = authored_delta
            .iter()
            .map(|value| value * value)
            .sum::<f32>();
        let projected = actual_delta
            .iter()
            .zip(authored_delta)
            .map(|(actual, authored)| actual * authored)
            .sum::<f32>()
            / denom;
        weights.push(projected);
    }

    for actual in weights {
        assert!(
            (actual - FIELD_FLOWER_SEMANTIC_WEIGHT).abs() < 0.025,
            "synthetic palette weight {actual:.3} changed with terrain LOD"
        );
    }
}

#[test]
fn every_field_surface_weight_is_muted_and_lod_independent() {
    use crate::flora::FieldSurfaceKind as Kind;

    let expected = [
        (Kind::Crop, FIELD_CULTIVATED_SEMANTIC_WEIGHT),
        (Kind::FlowerField, FIELD_FLOWER_SEMANTIC_WEIGHT),
        (Kind::Orchard, FIELD_CULTIVATED_SEMANTIC_WEIGHT),
        (Kind::Vineyard, FIELD_CULTIVATED_SEMANTIC_WEIGHT),
        (Kind::Meadow, FIELD_MEADOW_SEMANTIC_WEIGHT),
        (Kind::Dormant, FIELD_CULTIVATED_SEMANTIC_WEIGHT),
        (Kind::Harvested, FIELD_CULTIVATED_SEMANTIC_WEIGHT),
    ];
    for (kind, weight) in expected {
        assert_eq!(field_semantic_weight(kind), weight);
        assert!(
            weight <= 0.18,
            "{kind:?} surface is too opaque at {weight:.2}"
        );
    }
}

#[test]
fn flower_destinations_never_paint_authored_colour_into_land() {
    use vearth_data::vectors::FloraRegionKind;

    let col = test_col(
        [86, 112, 64],
        Climate {
            temp_c: 20.0,
            arid: 0.2,
            precipitation_mm_day: 1.5,
            humidity: 0.65,
        },
        0.03,
        4.0,
    );
    for destination in &crate::flower_destinations::FLOWER_DESTINATIONS {
        let seed = flora::curated_field_seed(destination.id);
        let stamp = tier0::FloraStamp {
            location_rad: None,
            kind: FloraRegionKind::Garden,
            seed,
            curated_style: destination.id as u8,
            surface: flora::curated_field_surface(destination),
        };
        let palette = flora::curated_palette_linear(stamp.curated_style).unwrap();
        for level in [19, 18, 17, 16] {
            let cell = destination_cell(destination, level);
            let sample = flora::sample_field_column(stamp.surface, seed, cell, N / 2, N / 2);
            let authored = palette[usize::from(sample.flower_variant)].map(|value| value * 255.0);
            let terrain = terrain_appearance(&col, col.elev, 0.5, 0.0);
            let substrate = terrain.tint.map(f32::from);
            let actual_appearance = field_surface_appearance(stamp, &col, cell, N / 2, N / 2);
            let actual = actual_appearance.tint.map(f32::from);
            if destination.mode == crate::flower_destinations::DestinationMode::EcologicalBoost {
                assert_eq!(
                    actual_appearance, terrain,
                    "{} L{level} changed ecological-boost land cover",
                    destination.name
                );
                continue;
            }
            assert_eq!(
                actual_appearance.material,
                material::FIELD_FLOWER,
                "{} L{level} lost flower-field material semantics",
                destination.name
            );
            let authored_delta: [f32; 3] =
                std::array::from_fn(|channel| authored[channel] - substrate[channel]);
            let actual_delta: [f32; 3] =
                std::array::from_fn(|channel| actual[channel] - substrate[channel]);
            let denom = authored_delta
                .iter()
                .map(|value| value * value)
                .sum::<f32>();
            let weight = actual_delta
                .iter()
                .zip(authored_delta)
                .map(|(actual, authored)| actual * authored)
                .sum::<f32>()
                / denom;
            assert!(
                (weight - FIELD_FLOWER_SEMANTIC_WEIGHT).abs() < 0.03,
                "{} L{level} painted authored colour at weight {weight:.3}",
                destination.name
            );
        }
    }
}

#[test]
fn ecological_boost_surface_matches_sparse_descriptor_coverage() {
    let destination = crate::flower_destinations::flower_destination(
        crate::flower_destinations::FlowerDestinationId::Namaqualand,
    );
    let center = destination_cell(destination, 19);
    let mut cols = [[Col::EMPTY; N]; N];
    for col in cols.iter_mut().flatten() {
        col.elev = 320.0;
        col.top = 320.0;
        col.climate = Climate {
            temp_c: 17.0,
            arid: 0.28,
            precipitation_mm_day: 2.4,
            humidity: 0.68,
        };
    }

    let mut checked = false;
    'cells: for dj in -4..=4 {
        for di in -4..=4 {
            let cell = CellId {
                i: center.i.saturating_add_signed(di),
                j: center.j.saturating_add_signed(dj),
                ..center
            };
            let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
            let mut overlay = tier0::CellOverlay::empty();
            apply_curated_flower_destinations(cell, &frame, &mut overlay, &cols);
            let surface_cols = overlay
                .cols
                .iter()
                .flatten()
                .filter(|col| {
                    col.flora
                        .is_some_and(|stamp| stamp.curated_style == destination.id as u8)
                })
                .count();
            if surface_cols == 0 {
                continue;
            }
            let descriptor =
                build_flora_descriptors(cell, &frame, Some(&overlay), &cols, &[], None)
                    .into_iter()
                    .find(|descriptor| descriptor.curated_style == destination.id as u8)
                    .expect("curated ecological surface must emit its matching descriptor");
            assert_eq!(
                surface_cols,
                descriptor.covered_samples() as usize * (N / flora::FLORA_COVERAGE_EDGE).pow(2),
                "terrain paint and procedural coverage diverged"
            );
            assert!(
                descriptor.covered_samples() < 192,
                "ecological boost became a broad regional carpet"
            );
            checked = true;
            break 'cells;
        }
    }
    assert!(checked, "fixture did not find an eligible ecological patch");
}

// Vector synthesis.

/// Fixed feature pack, regardless of query location.
struct FixedVectors(fn() -> FeaturePack);

impl VectorSource for FixedVectors {
    fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
        (self.0)()
    }
}

fn sq(half: f32) -> vearth_data::vectors::Rings {
    vec![vec![
        [-half, -half],
        [half, -half],
        [half, half],
        [-half, half],
    ]]
}

#[test]
fn generated_forest_roots_touch_the_fractional_source_surface() {
    fn forest_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.forest.push(sq(400.0));
        pack
    }
    let cell = mid_cell(17);
    let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
    let elevation = FnElevation(move |lat: f64, lon: f64| {
        (100.0 + (lat - lat0) * 900_000.0 + (lon - lon0) * 1_100_000.0) as f32
    });
    let vectors = FixedVectors(forest_pack);
    let column = generate_surface_cell(
        cell,
        &Sources {
            elevation: &elevation,
            surface: None,
            imagery: None,
            vectors: Some(&vectors),
        },
    );
    let ground = column.terrain_collision.as_ref().unwrap();
    assert!(column.tree_instances.len() > 8);
    assert!(column.solid_collision.capsules().is_empty());
    assert!(column.solid_collision.triangles().is_empty());
    let mut corrected = 0;
    for tree in &column.tree_instances {
        let root = frame.ecef_to_local(vearth_core::ellipsoid::geodetic_to_ecef(
            tree.lat_rad,
            tree.lon_rad,
            f64::from(tree.elevation_m),
        ));
        let Some(rendered) = ground.sample_local(root.x, root.z) else {
            continue;
        };
        assert!((root.y - rendered).abs() * frame.grid_scale.y < 0.0001);
        let (x, z) = (tree.cell_x.floor() as usize, tree.cell_z.floor() as usize);
        let h = &column.terrain_collision.as_ref().unwrap().heights_local;
        let centre = (h[z * (N + 1) + x]
            + h[z * (N + 1) + x + 1]
            + h[(z + 1) * (N + 1) + x]
            + h[(z + 1) * (N + 1) + x + 1])
            * 0.25;
        corrected += usize::from((root.y as f32 - centre).abs() > 0.02);
    }
    assert!(
        corrected > 4,
        "fixture must distinguish fractional roots from column averages"
    );
}

#[test]
fn flora_ground_samples_share_source_lattice_edges_without_column_center_phase() {
    fn farmland_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(400.0),
            kind: vearth_data::vectors::FloraRegionKind::Farmland,
            seed: 41,
        });
        pack
    }
    let target = mid_cell(19);
    let frame = ChunkFrame::of(ChunkAddr { cell: target, k: 0 });
    let (lat0, lon0) = frame.column_latlon(0.0, 0.0);
    let elevation = FnElevation(move |lat, lon| {
        (40.0
            + (lon - lon0) * vearth_core::ellipsoid::MEAN_RADIUS * 0.4
            + (lat - lat0) * vearth_core::ellipsoid::MEAN_RADIUS * 0.2) as f32
    });
    let vectors = FixedVectors(farmland_pack);
    let column = generate_surface_cell(
        target,
        &Sources {
            elevation: &elevation,
            surface: None,
            imagery: None,
            vectors: Some(&vectors),
        },
    );
    let terrain = column.terrain_collision.as_ref().unwrap();
    let descriptor = column
        .flora
        .first()
        .expect("mapped farm must retain its descriptor");
    for z in 0..flora::FLORA_GROUND_EDGE {
        for x in 0..flora::FLORA_GROUND_EDGE {
            let fx = x as f32 / (flora::FLORA_GROUND_EDGE - 1) as f32;
            let fz = z as f32 / (flora::FLORA_GROUND_EDGE - 1) as f32;
            let height = terrain
                .sample_local(f64::from(fx) * N as f64, f64::from(fz) * N as f64)
                .unwrap();
            assert!(
                (f64::from(descriptor.ground_height_m(fx, fz)) - height * frame.grid_scale.y).abs()
                    <= 0.006,
                "flora ground samples must use actual source corners, including the final cell edge"
            );
        }
    }
}

#[test]
fn flora_semantic_surfaces_preserve_collision() {
    fn flora_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(400.0),
            kind: vearth_data::vectors::FloraRegionKind::Farmland,
            seed: 0x0046_4945_4c44,
        });
        pack
    }

    let elevation = FnElevation(|_, _| 40.0);
    let vectors = FixedVectors(flora_pack);
    crate::flora_test_support::assert_visual_flora_preserves_collision(|mode| {
        generate_surface_cell(
            mid_cell(17),
            &Sources {
                elevation: &elevation,
                surface: None,
                imagery: None,
                vectors: match mode {
                    crate::flora_test_support::VisualFloraControl::Disabled => None,
                    crate::flora_test_support::VisualFloraControl::Enabled => Some(&vectors),
                },
            },
        )
    });
}

#[test]
fn orchards_and_vineyards_remain_visual_descriptors() {
    fn orchard_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(400.0),
            kind: vearth_data::vectors::FloraRegionKind::Orchard,
            seed: 0x4f52_4348_4152_4437,
        });
        pack
    }

    fn vineyard_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(400.0),
            kind: vearth_data::vectors::FloraRegionKind::Vineyard,
            seed: 0x5649_4e45_5944_4637,
        });
        pack
    }

    let elevation = FnElevation(|_, _| 84.0);
    for (make_pack, expected) in [
        (
            orchard_pack as fn() -> FeaturePack,
            flora::FloraArchetype::Orchard,
        ),
        (
            vineyard_pack as fn() -> FeaturePack,
            flora::FloraArchetype::Vineyard,
        ),
    ] {
        let vectors = FixedVectors(make_pack);
        let cell = mid_cell(19);
        let control = generate_surface_cell(cell, &Sources::terrain_only(&elevation));
        let managed = generate_surface_cell(
            cell,
            &Sources {
                elevation: &elevation,
                surface: None,
                imagery: None,
                vectors: Some(&vectors),
            },
        );
        assert_eq!(
            crate::flora_test_support::solid_geometry(&control),
            crate::flora_test_support::solid_geometry(&managed)
        );
        assert_eq!(managed.height_range, control.height_range);
        assert!(
            managed.tree_instances.is_empty(),
            "{expected:?} entered the natural-tree ownership path"
        );
        assert_eq!(managed.flora.len(), 1);
        assert_eq!(managed.flora[0].archetype, expected);
    }
}

#[test]
fn semantic_field_materials_render_from_l11_through_l19() {
    fn flora_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(20_000.0),
            kind: vearth_data::vectors::FloraRegionKind::Vineyard,
            seed: 0x4652_4f4d_5f4c_3131,
        });
        pack
    }

    let elevation = FnElevation(|_, _| 40.0);
    let vectors = FixedVectors(flora_pack);
    for level in 11..=19 {
        let column = generate_surface_cell(
            mid_cell(level),
            &Sources {
                elevation: &elevation,
                surface: None,
                imagery: None,
                vectors: Some(&vectors),
            },
        );
        assert!(
            has_material(&column, material::FIELD_VINEYARD),
            "vineyard surface missing at L{level}"
        );
        assert!(
            column.tree_instances.is_empty(),
            "semantic fields added plant/tree geometry at L{level}"
        );
    }
}

#[test]
fn changes_only_flora_surface_appearance_in_an_otherwise_identical_cell() {
    fn flora_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(18.0),
            kind: vearth_data::vectors::FloraRegionKind::Garden,
            seed: 0x4e4f_4e46_4c4f_5241,
        });
        pack
    }

    let elevation = FnElevation(|_, _| 40.0);
    let vectors = FixedVectors(flora_pack);
    let cell = mid_cell(17);
    let control = generate_surface_cell(cell, &Sources::terrain_only(&elevation));
    let flora = generate_surface_cell(
        cell,
        &Sources {
            elevation: &elevation,
            surface: None,
            imagery: None,
            vectors: Some(&vectors),
        },
    );
    assert_eq!(
        crate::flora_test_support::solid_geometry(&control),
        crate::flora_test_support::solid_geometry(&flora)
    );
    assert_eq!(control.batches.len(), flora.batches.len());
    let mut changed = 0;
    for (a, b) in control.batches.iter().zip(&flora.batches) {
        assert_eq!(a.addr, b.addr);
        assert_eq!(a.mesh.indices, b.mesh.indices);
        for (av, bv) in a.mesh.vertices.iter().zip(&b.mesh.vertices) {
            assert_eq!(av.pos, bv.pos);
            changed += usize::from(av.rgba != bv.rgba || av.face_ao != bv.face_ao);
        }
    }
    assert!(changed > 0);
}

#[test]
fn descriptors_are_deterministic_and_visual_only() {
    fn flora_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(400.0),
            kind: vearth_data::vectors::FloraRegionKind::Vineyard,
            seed: 0x4633_5f44_4553_4352,
        });
        pack
    }

    let elevation = FnElevation(|_, _| 1_842.125);
    let vectors = FixedVectors(flora_pack);
    let sources = Sources {
        elevation: &elevation,
        surface: None,
        imagery: None,
        vectors: Some(&vectors),
    };
    let cell = mid_cell(19);
    let first = generate_surface_cell(cell, &sources);
    let second = generate_surface_cell(cell, &sources);
    assert_eq!(first.flora, second.flora);
    assert_eq!(first.flora.len(), 1);
    let descriptor = &first.flora[0];
    assert_eq!(descriptor.archetype, flora::FloraArchetype::Vineyard);
    assert_eq!(descriptor.covered_samples(), 256);
    assert_eq!(
        descriptor.row_heading_rad,
        flora::select_field_surface(
            vearth_data::vectors::FloraRegionKind::Vineyard,
            descriptor.field_seed,
        )
        .heading_rad() as f32
    );
    assert_eq!(
        descriptor.palette,
        flora::select_field_surface(
            vearth_data::vectors::FloraRegionKind::Vineyard,
            descriptor.field_seed,
        )
        .palette
    );
    assert!((descriptor.ground_height_m(0.5, 0.5) - 1_842.125).abs() < 0.02);
    assert_eq!(
        crate::flora_test_support::solid_geometry(&first),
        crate::flora_test_support::solid_geometry(&second)
    );
    assert_eq!(first.height_range, second.height_range);

    let neighbor = generate_surface_cell(
        CellId {
            i: cell.i + 1,
            ..cell
        },
        &sources,
    );
    let adjacent = &neighbor.flora[0];
    assert_eq!(adjacent.field_seed, descriptor.field_seed);
    assert_eq!(adjacent.archetype, descriptor.archetype);
    assert_eq!(adjacent.row_heading_rad, descriptor.row_heading_rad);
    assert_eq!(adjacent.palette, descriptor.palette);
    assert_eq!(adjacent.phenology_offset, descriptor.phenology_offset);
}

#[test]
fn farmland_inference_keeps_one_catalogue_identity_across_cells() {
    fn farmland_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(400.0),
            kind: vearth_data::vectors::FloraRegionKind::Farmland,
            // This stable identity selects the ordinary crop surface;
            // flower-field routing is tested independently.
            seed: 1,
        });
        pack
    }
    assert_eq!(
        flora::select_field_surface(vearth_data::vectors::FloraRegionKind::Farmland, 1).kind,
        flora::FieldSurfaceKind::Crop
    );
    let vectors = FixedVectors(farmland_pack);
    let elevation = FnElevation(|_, _| 120.0);
    let sources = Sources {
        elevation: &elevation,
        surface: None,
        imagery: None,
        vectors: Some(&vectors),
    };
    let cell = mid_cell(19);
    let neighbor = CellId {
        i: cell.i + 1,
        ..cell
    };
    let a = generate_surface_cell(cell, &sources);
    let b = generate_surface_cell(neighbor, &sources);
    let a = &a.flora[0];
    let b = &b.flora[0];
    assert!(matches!(
        a.archetype,
        flora::FloraArchetype::Cereal
            | flora::FloraArchetype::BroadleafRows
            | flora::FloraArchetype::TallRows
            | flora::FloraArchetype::LowDenseCrop
    ));
    assert_eq!(a.archetype, b.archetype);
    assert_eq!(a.field_seed, b.field_seed);
    assert_eq!(a.row_heading_rad, b.row_heading_rad);
    assert_eq!(a.palette, b.palette);
    assert_eq!(a.base_density, b.base_density);
}

#[test]
fn garden_descriptor_covers_only_stable_authored_beds() {
    fn garden_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(400.0),
            kind: vearth_data::vectors::FloraRegionKind::Garden,
            seed: 0x4636_5f47_4152_444e,
        });
        pack
    }
    let vectors = FixedVectors(garden_pack);
    let elevation = FnElevation(|_, _| 80.0);
    let sources = Sources {
        elevation: &elevation,
        surface: None,
        imagery: None,
        vectors: Some(&vectors),
    };
    let column = generate_surface_cell(mid_cell(19), &sources);
    let descriptor = column
        .flora
        .iter()
        .find(|flora| flora.archetype == flora::FloraArchetype::OrnamentalBed)
        .expect("fixture must retain at least one ornamental bed");
    assert!(descriptor.covered_samples() > 0);
    assert!(
        descriptor.covered_samples() < 192,
        "a garden polygon must not become a nearly uniform flower carpet"
    );
    assert_eq!(
        descriptor.coverage,
        generate_surface_cell(mid_cell(19), &sources).flora[0].coverage
    );
}

#[test]
fn natural_grass_wildflowers_are_sparse_visual_descriptors_only() {
    let lush = test_col(
        [58, 112, 52],
        Climate {
            temp_c: 13.0,
            arid: 0.24,
            precipitation_mm_day: 2.6,
            humidity: 0.70,
        },
        0.06,
        34.0,
    );
    let lush_cols = [[lush; N]; N];
    let empty_overlay = tier0::CellOverlay::empty();
    let base = mid_cell(19);
    assert!(
        build_flora_descriptors(
            base,
            &ChunkFrame::of(ChunkAddr { cell: base, k: 0 }),
            None,
            &lush_cols,
            &[],
            None,
        )
        .is_empty(),
        "natural inference must wait for semantic exclusion data"
    );
    let mut found = None;
    for dj in 0..16 {
        for di in 0..16 {
            let cell = CellId {
                i: base.i + di,
                j: base.j + dj,
                ..base
            };
            let descriptors = build_flora_descriptors(
                cell,
                &ChunkFrame::of(ChunkAddr { cell, k: 0 }),
                Some(&empty_overlay),
                &lush_cols,
                &[],
                None,
            );
            if let Some(descriptor) = descriptors.into_iter().next() {
                found = Some(descriptor);
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let descriptor = found.expect("suitable natural grass must admit some clustered patches");
    assert_eq!(
        descriptor.archetype,
        flora::FloraArchetype::MeadowWildflowers
    );
    assert!(descriptor.covered_samples() > 0);
    assert!(descriptor.covered_samples() < 192);

    let drought = test_col(
        [132, 104, 64],
        Climate {
            temp_c: 23.0,
            arid: 0.93,
            precipitation_mm_day: 0.08,
            humidity: 0.16,
        },
        0.04,
        20.0,
    );
    let drought_cols = [[drought; N]; N];
    assert!(
        build_flora_descriptors(
            base,
            &ChunkFrame::of(ChunkAddr { cell: base, k: 0 }),
            Some(&empty_overlay),
            &drought_cols,
            &[],
            None,
        )
        .is_empty(),
        "drought terrain must not receive a natural wildflower descriptor"
    );
}

#[test]
fn coverage_is_built_after_holes_and_physical_exclusions() {
    fn base_pack() -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.flora.push(vearth_data::vectors::FloraRegion {
            location_rad: None,
            rings: sq(400.0),
            kind: vearth_data::vectors::FloraRegionKind::Farmland,
            seed: 0x4633_5f4d_4153_4b53,
        });
        pack
    }
    fn hole_pack() -> FeaturePack {
        let mut pack = base_pack();
        pack.flora[0].rings.push(sq(5.0).remove(0));
        pack
    }
    fn road_pack() -> FeaturePack {
        let mut pack = base_pack();
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-30.0, 0.0], [30.0, 0.0]],
            class: RoadClass::Minor,
            is_bridge: false,
        });
        pack
    }
    fn water_pack() -> FeaturePack {
        let mut pack = base_pack();
        pack.water.push(Water::inland(sq(5.0)));
        pack
    }
    fn building_pack() -> FeaturePack {
        let mut pack = base_pack();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: sq(5.0),
            height_m: 8.0,
            min_height_m: 0.0,
            seed: 7,
            kind: tier0::BuildingKind::Building,
        });
        pack
    }
    fn paving_pack() -> FeaturePack {
        let mut pack = base_pack();
        pack.ground.push((GroundKind::Paved, sq(5.0)));
        pack
    }

    let elevation = FnElevation(|_, _| 40.0);
    let generate = |pack: fn() -> FeaturePack| {
        let vectors = FixedVectors(pack);
        generate_surface_cell(
            mid_cell(19),
            &Sources {
                elevation: &elevation,
                surface: None,
                imagery: None,
                vectors: Some(&vectors),
            },
        )
    };
    let baseline = generate(base_pack);
    assert_eq!(baseline.flora[0].covered_samples(), 256);
    for (name, pack) in [
        ("polygon hole", hole_pack as fn() -> FeaturePack),
        ("road", road_pack),
        ("water", water_pack),
        ("building", building_pack),
        ("paving", paving_pack),
    ] {
        let column = generate(pack);
        assert_eq!(column.flora.len(), 1, "{name} removed the whole field");
        let covered = column.flora[0].covered_samples();
        assert!(
            covered < 256,
            "{name} was not removed from descriptor coverage"
        );
        assert!(covered > 0, "{name} removed unrelated field coverage");
    }
}

#[test]
fn building_extrudes_with_walls_and_roof() {
    let elev = FnElevation(|_, _| 40.0);
    let vecs = FixedVectors(|| {
        let mut p = FeaturePack::empty();
        p.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: sq(6.0),
            height_m: 30.0,
            min_height_m: 0.0,
            seed: 7,
            kind: tier0::BuildingKind::Building,
        });
        p
    });
    let src = Sources {
        elevation: &elev,
        surface: None,
        imagery: None,
        vectors: Some(&vecs),
    };
    let col = generate_surface_cell(mid_cell(19), &src);
    assert!(vertices(&col).any(|v| material::is_architecture_wall(((v.face_ao >> 5) & 255) as u8)));
    assert!(vertices(&col).any(|v| material::is_architecture_roof(((v.face_ao >> 5) & 255) as u8)));
    assert!(has_material(&col, material::WINDOW));
    assert!(col.height_range.1 >= 70.0);
    assert!(solid_at_height(&col, 16.0, 16.0, 50.0));
    assert!(!solid_at_height(&col, 16.0, 16.0, 90.0));
}

#[test]
fn elevated_source_parts_retain_only_the_mapped_parent_body_below_them() {
    struct Parts {
        parent: Option<f32>,
        courtyard: bool,
        reverse: bool,
    }
    impl VectorSource for Parts {
        fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
            let mut pack = FeaturePack::empty();
            if let Some(height_m) = self.parent {
                let mut rings = sq(6.3);
                if self.courtyard {
                    rings.extend(sq(3.0));
                }
                pack.buildings.push(Building {
                    location_rad: None,
                    roof_shape: None,
                    use_kind: Default::default(),
                    rings,
                    height_m,
                    min_height_m: 0.0,
                    seed: 77,
                    kind: tier0::BuildingKind::Building,
                });
            }
            pack.buildings.push(Building {
                location_rad: None,
                roof_shape: None,
                use_kind: Default::default(),
                rings: sq(4.0),
                height_m: 98.6,
                min_height_m: 70.0,
                seed: 78,
                kind: tier0::BuildingKind::Part,
            });
            pack.observation_hints
                .push(vearth_data::vectors::ObservationHint {
                    point: [0.0, 0.0],
                    seed: 77,
                    kind: tier0::ObservationHintKind::Viewpoint,
                });
            if self.reverse {
                pack.buildings.reverse();
            }
            pack
        }
    }
    let elevation = FnElevation(|_, _| 40.0);
    for level in [17, 19] {
        let cell = mid_cell(level);
        let generate = |parent, courtyard, reverse| {
            generate_surface_cell(
                cell,
                &Sources {
                    elevation: &elevation,
                    surface: None,
                    imagery: None,
                    vectors: Some(&Parts {
                        parent,
                        courtyard,
                        reverse,
                    }),
                },
            )
        };
        let sample =
            |column: &SurfaceCell, above: f64| solid_at_height(column, 16.0, 16.0, 40.0 + above);
        let parent = generate(Some(98.6), false, false);
        let reversed = generate(Some(98.6), false, true);
        for height in [5.0, 20.0, 50.0, 68.0, 73.0, 96.0] {
            assert!(
                sample(&parent, height),
                "mapped tower body disappeared at{height}m/L{level}"
            );
            assert_eq!(sample(&parent, height), sample(&reversed, height));
        }
        for (parent_height, courtyard) in [(None, false), (Some(50.0), false), (Some(98.6), true)] {
            let column = generate(parent_height, courtyard, false);
            assert!(!sample(&column, 62.0), "unsupported gap was filled");
            assert!(sample(&column, 75.0), "source cap was removed");
            if parent_height == Some(50.0) && !courtyard {
                assert!(sample(&column, 20.0));
            } else {
                assert!(
                    !sample(&column, 20.0),
                    "standalone part/courtyard gained a shaft"
                );
            }
        }
    }
}

#[test]
fn distant_buildings_omit_subpixel_glazing_without_detached_lights() {
    let elevation = FnElevation(|_, _| 40.0);
    let vectors = FixedVectors(|| {
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: sq(240.0),
            height_m: 180.0,
            min_height_m: 0.0,
            seed: 0x5eed,
            kind: tier0::BuildingKind::Building,
        });
        pack
    });
    let column = generate_surface_cell(
        mid_cell(NIGHT_LIGHT_MIN_LEVEL),
        &Sources {
            elevation: &elevation,
            surface: None,
            imagery: None,
            vectors: Some(&vectors),
        },
    );
    let facade_light = vertices(&column).any(|v| {
        matches!(
            ((v.face_ao >> 5) & 255) as u8,
            material::WINDOW
                | material::LIT_WINDOW
                | material::LIT_FACADE_LOW
                | material::LIT_FACADE_MEDIUM
                | material::LIT_FACADE_HIGH
        )
    });
    assert!(
        !facade_light,
        "individual panes below one pixel must not produce distant sparkling"
    );
    assert!(
        column.night_lights.is_empty(),
        "omitted distant panes must not become detached lights"
    );
}

#[test]
fn terminal_windows_do_not_depend_on_a_cross_zoom_id_join() {
    struct DifferentPyramid;
    impl VectorSource for DifferentPyramid {
        fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
            let mut pack = FeaturePack::empty();
            pack.buildings.push(Building {
                location_rad: None,
                roof_shape: None,
                use_kind: Default::default(),
                rings: sq(6.0),
                height_m: 36.0,
                min_height_m: 0.0,
                seed: 77,
                kind: tier0::BuildingKind::Building,
            });
            pack
        }

        fn night_buildings(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
            FeaturePack::empty()
        }
    }

    let elevation = FnElevation(|_, _| 40.0);
    let vectors = DifferentPyramid;
    let column = generate_surface_cell(
        mid_cell(MAX_LEVEL),
        &Sources {
            elevation: &elevation,
            surface: None,
            imagery: None,
            vectors: Some(&vectors),
        },
    );
    let materials: HashSet<u8> = vertices(&column)
        .map(|v| ((v.face_ao >> 5) & 255) as u8)
        .collect();
    assert!(
        materials.iter().any(|id| (111..=118).contains(id)),
        "production panels must retain night eligibility from their own source feature"
    );
}

#[test]
fn facade_openings_are_bay_coherent_and_preserve_night_eligibility() {
    let mut stamp = BuildingStamp {
        source_index: None,
        parent_index: None,
        facade_openings: true,
        design: Default::default(),
        min_h: 0.0,
        height: 30.0,
        lidar: false,
        top_geo: f32::NAN,
        seed: 0,
        building_kind: tier0::BuildingKind::Building,
        source_total_height: 30.0,
        night_eligible: true,
        facade_style: FacadeStyle::Masonry,
        tall_style: None,
        highrise: None,
        roof_shape: None,
        roof_height_m: None,
        facade: tier0::FacadeStamp {
            boundary: true,
            corner: false,
            edge: 2,
            along_m: 1.4,
            length_m: 12.0,
            edge_distance_m: 0.3,
        },
    };
    let mut roof_only = stamp;
    roof_only.building_kind = tier0::BuildingKind::RoofOnly;
    roof_only.night_eligible = true;
    for alt in [1.0, 12.0, 29.0] {
        let voxel = building_appearance(alt, 0.0, 30.0, &roof_only, None, 0.6);
        assert_eq!(voxel.material, material::ROOF);
    }
    let mut courtyard_roof = stamp;
    courtyard_roof.design.roof_material = material::TILE_ROOF;
    courtyard_roof.source_total_height = 13.0;
    courtyard_roof.facade_style = FacadeStyle::Industrial;
    courtyard_roof.roof_shape = Some(tier0::RoofShape::Hipped);
    {
        let roof = building_appearance(12.8, 0.0, 13.0, &courtyard_roof, None, 0.6);
        assert_eq!(
            roof.material,
            material::TILE_ROOF,
            "facade dimensions must not override the selected covering"
        );
        let mut traditional = courtyard_roof;
        traditional.facade_style = FacadeStyle::Mediterranean;
        assert_eq!(
            roof.tint,
            building_appearance(12.8, 0.0, 13.0, &traditional, None, 0.6).tint
        );
    }
    courtyard_roof.roof_shape = Some(tier0::RoofShape::Flat);
    courtyard_roof.design.roof_material = material::ROOF;
    assert_eq!(
        building_appearance(12.8, 0.0, 13.0, &courtyard_roof, None, 0.6).material,
        material::ROOF,
        "flat covering stays generic"
    );
    // Middle of the second floor and first fitted bay.
    let alt = 4.9;
    let opening = (0..4096u64).find_map(|seed| {
        stamp.seed = seed;
        let voxel = building_appearance(alt, 0.0, 30.0, &stamp, None, 0.6);
        (voxel.material == material::WINDOW).then_some((seed, voxel))
    });
    let (seed, a) = opening.expect("at least one deterministic opening state");
    stamp.seed = seed;
    stamp.facade.along_m = 1.6;
    let b = building_appearance(alt, 0.0, 30.0, &stamp, None, 0.6);
    assert_eq!(a, b, "every point in one opening shares a material state");

    stamp.facade.corner = true;
    let corner = building_appearance(alt, 0.0, 30.0, &stamp, None, 0.6);
    assert_eq!(
        corner.material, stamp.design.wall_material,
        "corners remain solid piers"
    );
    stamp.facade.corner = false;
    stamp.facade_style = FacadeStyle::Masonry;
    stamp.facade.along_m = 1.5;
    stamp.seed = (0..4096u64)
        .find(|seed| {
            let mut candidate = stamp;
            candidate.seed = *seed;
            building_appearance(alt, 0.0, 30.0, &candidate, None, 0.001).material
                == material::LIT_WINDOW
        })
        .expect("deterministic occupied opening");
    stamp.night_eligible = false;
    assert_eq!(
        building_appearance(alt, 0.0, 30.0, &stamp, None, 0.001).material,
        material::WINDOW
    );
}

#[test]
fn tall_building_profiles_preserve_the_selected_regional_material_and_albedo() {
    use crate::tall_buildings::TALL_BUILDING_PROFILES;
    for profile in TALL_BUILDING_PROFILES {
        let height = profile.floor_m * 40.0;
        let mut stamp = BuildingStamp {
            source_index: None,
            parent_index: None,
            facade_openings: true,
            design: regional_architecture::BuildingDesign {
                wall_material: material::BRICK_WALL,
                wall: [170, 100, 70],
                roof: [160, 150, 130],
                ..Default::default()
            },
            min_h: 0.0,
            height: height as f32,
            lidar: false,
            top_geo: f32::NAN,
            seed: 0,
            building_kind: tier0::BuildingKind::Building,
            source_total_height: height as f32,
            night_eligible: false,
            facade_style: FacadeStyle::Tower,
            tall_style: Some(profile.id),
            highrise: None,
            roof_shape: Some(tier0::RoofShape::Flat),
            roof_height_m: None,
            facade: tier0::FacadeStamp {
                boundary: false,
                corner: false,
                edge: 0,
                along_m: 0.0,
                length_m: 12.0,
                edge_distance_m: 0.0,
            },
        };
        let brick = building_appearance(height * 0.5, 0.0, height, &stamp, None, 0.6);
        assert_eq!(brick.material, material::BRICK_WALL);
        stamp.design.wall_material = material::PLASTER_WALL;
        stamp.design.wall = [220, 215, 190];
        let plaster = building_appearance(height * 0.5, 0.0, height, &stamp, None, 0.6);
        assert_eq!(plaster.material, material::PLASTER_WALL);
        assert_ne!(
            brick.tint, plaster.tint,
            "{} erased regional albedo",
            profile.slug
        );
        let roof = building_appearance(height, 0.0, height, &stamp, None, 0.6);
        assert_eq!(roof.material, material::ROOF);
        assert_eq!(
            roof.tint,
            tint(
                art_direction::architecture_surface(material::ROOF, stamp.design.roof)
                    .map(f32::from),
                0.95
            )
        );
    }
}

#[test]
fn river_carves_and_fills_water() {
    let elev = FnElevation(|_, _| 80.0);
    let vecs = FixedVectors(|| {
        let mut p = FeaturePack::empty();
        p.water.push(vearth_data::vectors::Water::inland(sq(400.0)));
        p
    });
    let src = Sources {
        elevation: &elev,
        surface: None,
        imagery: None,
        vectors: Some(&vecs),
    };
    let col = generate_surface_cell(mid_cell(15), &src);
    assert!(!col.water.is_empty());
    assert!(has_material(&col, material::WATER));
    let water = material_height_at(&col, 16.0, 16.0, material::WATER).unwrap();
    assert!(
        col.terrain_collision
            .as_ref()
            .unwrap()
            .sample_local(16.0, 16.0)
            .is_none(),
        "exact wet-footprint subdivision must bypass the unsplit source heightfield"
    );
    let ground = (0..=u8::MAX)
        .filter(|&mat| mat != material::WATER)
        .filter_map(|mat| material_height_at(&col, 16.0, 16.0, mat))
        .max_by(f64::total_cmp)
        .expect("the water has a rendered terrain bed");
    assert!(water - ground > 1.19);
}

#[test]
fn osm_coastline_is_authoritative() {
    // DEM says shallow sea everywhere; OSM (a complete pack with no
    // ocean polygon) says dry land: the polder stays dry, and its floor
    // is land palette, not seabed.
    let elev = FnElevation(|_, _| -4.0);
    let land = FixedVectors(|| {
        let mut p = FeaturePack::empty();
        p.ground
            .push((vearth_data::vectors::GroundKind::Green, sq(400.0)));
        p
    });
    let src = Sources {
        elevation: &elev,
        surface: None,
        imagery: None,
        vectors: Some(&land),
    };
    let col = generate_surface_cell(mid_cell(15), &src);
    let mats: Vec<u8> = vertices(&col)
        .map(|v| ((v.face_ao >> 5) & 255) as u8)
        .collect();
    assert!(
        !mats.contains(&material::WATER),
        "OSM land must dry DEM sea"
    );
    assert!(
        !mats.contains(&material::SEABED),
        "dried floor is land, not seabed"
    );

    // The reverse: DEM says land (+3 m), OSM ocean polygon covers the
    // cell → carved coast fills with sea surface water.
    let elev_land = FnElevation(|_, _| 3.0);
    let ocean = FixedVectors(|| {
        let mut p = FeaturePack::empty();
        p.water.push(vearth_data::vectors::Water::ocean(sq(400.0)));
        p
    });
    let src = Sources {
        elevation: &elev_land,
        surface: None,
        imagery: None,
        vectors: Some(&ocean),
    };
    let col = generate_surface_cell(mid_cell(15), &src);
    assert!(has_material(&col, material::WATER));
    assert!(!col.water.is_empty());
}

#[test]
fn roads_and_trees_stamp_at_street_level() {
    let elev = FnElevation(|_, _| 10.0);
    let vecs = FixedVectors(|| {
        let mut p = FeaturePack::empty();
        p.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-200.0, 0.0], [200.0, 0.0]],
            class: vearth_data::vectors::RoadClass::Major,
            is_bridge: false,
        });
        // The L20 lattice supplies 64 candidates in this L17 forest,
        // with the mapped road excluding roots from its full width.
        p.forest.push(sq(300.0));
        p
    });
    let src = Sources {
        elevation: &elev,
        surface: None,
        imagery: None,
        vectors: Some(&vecs),
    };
    let col = generate_surface_cell(mid_cell(17), &src);
    assert!(has_material(&col, material::ASPHALT));
    assert!(!col.tree_instances.is_empty());
    assert!(col.solid_collision.capsules().is_empty());
    assert!(vertices(&col).any(|v| material::is_foliage(((v.face_ao >> 5) & 255) as u8)));
}

#[test]
fn railway_generates_only_flush_steel_rails() {
    let elev = FnElevation(|_, _| 10.0);
    let vectors = FixedVectors(|| {
        let mut pack = FeaturePack::empty();
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-200.0, 0.0], [200.0, 0.0]],
            class: RoadClass::Rail,
            is_bridge: false,
        });
        pack
    });
    let sources = Sources {
        elevation: &elev,
        surface: None,
        imagery: None,
        vectors: Some(&vectors),
    };
    let cell = mid_cell(19);
    let column = generate_surface_cell(cell, &sources);
    assert!(!has_material(&column, material::RAIL));
    assert!(has_material(&column, material::STEEL));
}

#[test]
fn tier0_generation_is_deterministic() {
    let elev = FnElevation(|_, _| 25.0);
    let vecs = FixedVectors(|| {
        let mut p = FeaturePack::empty();
        p.buildings.push(Building {
            location_rad: None,
            roof_shape: None,
            use_kind: Default::default(),
            rings: sq(5.0),
            height_m: 12.0,
            min_height_m: 0.0,
            seed: 99,
            kind: tier0::BuildingKind::Building,
        });
        p.forest.push(sq(150.0));
        p
    });
    let src = Sources {
        elevation: &elev,
        surface: None,
        imagery: None,
        vectors: Some(&vecs),
    };
    let a = generate_surface_cell(mid_cell(18), &src);
    let b = generate_surface_cell(mid_cell(18), &src);
    assert_eq!(
        contracts::surface_cell_fingerprint(&a),
        contracts::surface_cell_fingerprint(&b)
    );
}

#[test]
fn golden_gate_generated_deck_is_continuous_across_cell_boundaries() {
    let lm = landmarks::LANDMARKS
        .iter()
        .find(|landmark| landmark.name == "golden-gate-bridge")
        .unwrap();
    let lat = lm.lat_deg.to_radians();
    let lon = lm.lon_deg.to_radians();
    let heading = lm.heading_deg.to_radians();
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    let (sin_h, cos_h) = heading.sin_cos();
    let anchor = vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
    let east = DVec3::new(-sin_lon, cos_lon, 0.0);
    let north = DVec3::new(-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat);
    let model_x = east * cos_h - north * sin_h;
    let model_z = east * sin_h + north * cos_h;
    let assert_coverage = |mut intervals: Vec<(f64, f64)>, start: f64, end: f64, label: &str| {
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut covered = start;
        let mut gaps = Vec::new();
        for (lo, hi) in intervals {
            if hi < start || lo > end {
                continue;
            }
            if lo > covered + 0.05 {
                gaps.push((covered, lo));
            }
            covered = covered.max(hi);
        }
        if covered < end {
            gaps.push((covered, end));
        }
        assert!(gaps.is_empty(), "{label} gaps: {gaps:?}");
    };
    let source = FnElevation(|_, _| -30.0);
    let sources = Sources::terrain_only(&source);

    for level in [13, 16, 19] {
        let mut center_cells = std::collections::HashSet::new();
        for z in -1360..=1360 {
            let point = anchor + model_z * z as f64;
            center_cells.insert(CellId::from_dir(point.normalize(), level));
        }
        let mut cells = std::collections::HashSet::new();
        let n = 1i64 << level;
        for center in center_cells {
            for dj in -1i64..=1 {
                for di in -1i64..=1 {
                    let i = center.i as i64 + di;
                    let j = center.j as i64 + dj;
                    if i >= 0 && j >= 0 && i < n && j < n {
                        cells.insert(CellId {
                            i: i as u32,
                            j: j as u32,
                            ..center
                        });
                    }
                }
            }
        }

        let mut bridge_intervals = Vec::new();
        for cell in cells {
            let column = generate_surface_cell(cell, &sources);
            for batch in &column.batches {
                let frame = ChunkFrame::of(batch.addr);
                for tri in batch.mesh.indices.chunks_exact(3) {
                    let vertices = tri
                        .iter()
                        .map(|i| &batch.mesh.vertices[*i as usize])
                        .collect::<Vec<_>>();
                    if !matches!(
                        ((vertices[0].face_ao >> 5) & 255) as u8,
                        material::STEEL | material::ASPHALT | material::PAVED
                    ) {
                        continue;
                    }
                    let positions = vertices
                        .iter()
                        .map(|v| {
                            frame.local_to_ecef(DVec3::from_array(v.pos.map(f64::from))) - anchor
                        })
                        .collect::<Vec<_>>();
                    let xlo = positions
                        .iter()
                        .map(|p| p.dot(model_x))
                        .fold(f64::INFINITY, f64::min);
                    let xhi = positions
                        .iter()
                        .map(|p| p.dot(model_x))
                        .fold(f64::NEG_INFINITY, f64::max);
                    if xlo > 25.0 || xhi < -25.0 {
                        continue;
                    }
                    let lo = positions
                        .iter()
                        .map(|p| p.dot(model_z))
                        .fold(f64::INFINITY, f64::min);
                    let hi = positions
                        .iter()
                        .map(|p| p.dot(model_z))
                        .fold(f64::NEG_INFINITY, f64::max);
                    bridge_intervals.push((lo, hi));
                }
            }
        }
        assert_coverage(
            bridge_intervals,
            -1368.5,
            1368.5,
            &format!("L{level} generated authored bridge"),
        );
    }
}

fn vertices(cell: &SurfaceCell) -> impl Iterator<Item = &vearth_core::mesh::SurfaceVertex> {
    cell.batches.iter().flat_map(|b| &b.mesh.vertices)
}
fn count_material(cell: &SurfaceCell, material: u8) -> usize {
    vertices(cell)
        .filter(|v| ((v.face_ao >> 5) & 255) as u8 == material)
        .count()
}
fn has_material(cell: &SurfaceCell, material: u8) -> bool {
    count_material(cell, material) > 0
}

fn solid_at_height(cell: &SurfaceCell, x: f64, z: f64, height_m: f64) -> bool {
    let frame = ChunkFrame::of(ChunkAddr {
        cell: cell.cell,
        k: 0,
    });
    let metric = collision::MetricFrame::of(&frame)
        .point_from_ecef(frame.local_to_ecef(DVec3::new(x, height_m / frame.grid_scale.y, z)));
    cell.solid_collision
        .contains(metric[0], metric[1], metric[2])
}
fn material_height_at(cell: &SurfaceCell, x: f64, z: f64, material: u8) -> Option<f64> {
    let mut top: Option<f64> = None;
    for batch in &cell.batches {
        let frame = ChunkFrame::of(batch.addr);
        for tri in batch.mesh.indices.chunks_exact(3) {
            let v = [
                &batch.mesh.vertices[tri[0] as usize],
                &batch.mesh.vertices[tri[1] as usize],
                &batch.mesh.vertices[tri[2] as usize],
            ];
            if ((v[0].face_ao >> 5) & 255) as u8 != material {
                continue;
            }
            let [a, b, c] = v.map(|v| v.pos.map(f64::from));
            let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
            if d.abs() < 1e-12 {
                continue;
            }
            let u = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
            let w = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
            if u >= -1e-6 && w >= -1e-6 && u + w <= 1.0 + 1e-6 {
                let h = (u * a[1]
                    + w * b[1]
                    + (1.0 - u - w) * c[1]
                    + f64::from(batch.addr.k) * N as f64)
                    * frame.grid_scale.y;
                top = Some(top.map_or(h, |old| old.max(h)));
            }
        }
    }
    top
}

#[test]
fn complete_empty_map_keeps_below_sea_land_dry() {
    let elevation = FnElevation(|_, _| -4.0);
    let vectors = FixedVectors(FeaturePack::empty);
    let cell = generate_surface_cell(
        mid_cell(15),
        &Sources {
            elevation: &elevation,
            surface: None,
            imagery: None,
            vectors: Some(&vectors),
        },
    );
    assert!(cell.complete);
    assert!(cell.water.is_empty());
    assert!(!has_material(&cell, material::WATER));
}

#[test]
fn regional_priors_preserve_semantic_ownership_and_orbital_imagery() {
    let lat = 18.0_f64.to_radians();
    let lon = 76.0_f64.to_radians();
    let climate = climate_at(lat, lon, 500.0);
    let baseline = test_col([113, 121, 74], climate, 0.1, 10.0);
    let regional = Col {
        regional: regional_landscape::resolve(lat, lon, 500.0, climate, Some(4)),
        ..baseline
    };
    for kind in [
        GroundKind::Urban,
        GroundKind::Green,
        GroundKind::Paved,
        GroundKind::Sand,
        GroundKind::Farmyard,
        GroundKind::AirportArea,
        GroundKind::RunwayArea,
        GroundKind::RailwayArea,
        GroundKind::Glacier,
        GroundKind::Wetland,
        GroundKind::RegionalUrban,
        GroundKind::RegionalGlacier,
    ] {
        assert_eq!(
            ground_appearance(kind, &baseline, 0.5, 1.0),
            ground_appearance(kind, &regional, 0.5, 1.0),
            "{kind:?}"
        );
    }
    for kind in [
        GroundKind::Forest,
        GroundKind::NaturalGrass,
        GroundKind::Scrub,
        GroundKind::RegionalForest,
    ] {
        assert_eq!(
            ground_appearance(kind, &baseline, 0.5, 0.0),
            ground_appearance(kind, &regional, 0.5, 0.0)
        );
    }
    for kind in [
        vearth_data::vectors::FloraRegionKind::Farmland,
        vearth_data::vectors::FloraRegionKind::Orchard,
        vearth_data::vectors::FloraRegionKind::Vineyard,
    ] {
        let stamp = tier0::FloraStamp {
            location_rad: None,
            kind,
            seed: 42,
            curated_style: 0,
            surface: flora::select_field_surface(kind, 42),
        };
        for image in [Some([113, 121, 74]), None] {
            let baseline = Col {
                img_detail: image,
                ..baseline
            };
            let regional = Col {
                img_detail: image,
                ..regional
            };
            assert_eq!(
                field_surface_appearance(stamp, &baseline, mid_cell(17), 16, 16),
                field_surface_appearance(stamp, &regional, mid_cell(17), 16, 16)
            );
        }
    }
    for exposure in [0.0, 0.2, 0.6, 1.0] {
        assert_eq!(
            terrain_appearance_with_exposure(&baseline, 500.0, 0.0, exposure),
            terrain_appearance_with_exposure(&regional, 500.0, 0.0, exposure)
        );
    }
    assert_ne!(
        ground_appearance(GroundKind::NaturalGrass, &baseline, 0.5, 1.0).tint,
        ground_appearance(GroundKind::NaturalGrass, &regional, 0.5, 1.0).tint
    );
    for kind in [
        GroundKind::NaturalGrass,
        GroundKind::Forest,
        GroundKind::Scrub,
        GroundKind::BareRock,
    ] {
        assert_eq!(
            ground_appearance(kind, &baseline, 0.5, 1.0).material,
            ground_appearance(kind, &regional, 0.5, 1.0).material
        );
    }
}

#[test]
fn generated_seasonal_colors_follow_the_source_month_through_sampling_wrappers() {
    use std::sync::atomic::{AtomicU8, Ordering};
    struct Calendar(AtomicU8);
    impl ColorSource for Calendar {
        fn color(&self, _: f64, _: f64) -> Option<[u8; 3]> {
            Some([112, 136, 83])
        }
        fn month(&self) -> Option<u8> {
            Some(self.0.load(Ordering::Relaxed))
        }
    }
    let imagery = Calendar(AtomicU8::new(4));
    let elevation = FnElevation(|_, _| 500.0);
    let sources = Sources {
        imagery: Some(&imagery),
        ..Sources::terrain_only(&elevation)
    };
    let direction =
        vearth_core::ellipsoid::geodetic_to_ecef(18.0_f64.to_radians(), 76.0_f64.to_radians(), 0.0)
            .normalize();
    let generate_month = |month, level| {
        imagery.0.store(month, Ordering::Relaxed);
        generate_surface_cell(CellId::from_dir(direction, level), &sources)
    };
    let dry = generate_month(4, 17);
    let wet = generate_month(7, 17);
    let revisited = generate_month(4, 17);
    assert_eq!(
        contracts::surface_cell_fingerprint(&dry),
        contracts::surface_cell_fingerprint(&revisited)
    );
    assert_ne!(
        contracts::surface_cell_fingerprint(&dry),
        contracts::surface_cell_fingerprint(&wet)
    );
    assert_eq!(dry.batches.len(), wet.batches.len());
    for (a, b) in dry.batches.iter().zip(&wet.batches) {
        assert_eq!(a.mesh.indices, b.mesh.indices);
        assert_eq!(a.mesh.vertices.len(), b.mesh.vertices.len());
        for (a, b) in a.mesh.vertices.iter().zip(&b.mesh.vertices) {
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.face_ao, b.face_ao);
            assert_eq!(a.rgba[3], b.rgba[3]);
        }
    }
    for month in [1, 4, 7, 10, 12, 1] {
        assert_eq!(
            contracts::surface_cell_fingerprint(&generate_month(month, 5)),
            contracts::surface_cell_fingerprint(&generate_month(4, 5))
        );
    }
}
