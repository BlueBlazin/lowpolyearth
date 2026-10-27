//! Land-semantic regression baseline.
//!
//! This constructed source fixture is intentionally broader than the visited
//! pinned cache. It protects official Version 4 kinds even when the current
//! local sample has no occurrence, while the observation artifact records which values
//! were actually observed in that sample.

use super::*;
use crate::mvt::encode::{FeatureSpec, LayerSpec};
use crate::protomaps_observability::{
    SemanticDiscardReason, SemanticLayerFamily, SemanticTelemetry,
};
use crate::protomaps_semantics::{
    LandAuthorityCandidate, LandEvidenceAuthority, LandcoverKind, LanduseKind, LayerFacts,
    SemanticDiagnosticKind, SemanticTile, preferred_land_authority,
};

const TILE_KEY: TileKey = (15, 16_384, 16_384);
const LANDUSE_KINDS: [&str; 15] = [
    "bare_rock",
    "wetland",
    "scrub",
    "grass",
    "grassland",
    "glacier",
    "farmyard",
    "railway",
    "aerodrome",
    "runway",
    "taxiway",
    "dam",
    "pitch",
    "residential",
    "farmland",
];
const LANDCOVER_KINDS: [&str; 7] = [
    "barren",
    "farmland",
    "forest",
    "glacier",
    "grassland",
    "scrub",
    "urban_area",
];

fn square(x: i32, y: i32) -> Vec<Vec<[i32; 2]>> {
    vec![vec![[x, y], [x + 120, y], [x + 120, y + 120], [x, y + 120]]]
}

fn fixture_tile() -> SemanticTile {
    let landuse_values = LANDUSE_KINDS
        .iter()
        .map(|kind| Value::Str((*kind).to_string()))
        .chain([Value::Str("soccer".into())])
        .collect::<Vec<_>>();
    let landuse_features = LANDUSE_KINDS
        .iter()
        .enumerate()
        .map(|(index, _)| {
            // Collision pairs share source geometry across layers:
            // bare_rock/barren, grass/grassland, residential/urban_area.
            let (x, y) = match index {
                0 => (100, 100),
                3 => (300, 100),
                13 => (500, 100),
                _ => (
                    100 + (index as i32 % 7) * 180,
                    400 + (index as i32 / 7) * 180,
                ),
            };
            let mut tags = vec![(0, index as u32)];
            if LANDUSE_KINDS[index] == "pitch" {
                tags.push((1, LANDUSE_KINDS.len() as u32));
            }
            FeatureSpec {
                id: Some(1_000 + index as u64),
                geom_type: GeomType::Polygon,
                paths: square(x, y),
                tags,
            }
        })
        .collect::<Vec<_>>();
    let landcover_values = LANDCOVER_KINDS
        .iter()
        .map(|kind| Value::Str((*kind).to_string()))
        .collect::<Vec<_>>();
    let landcover_features = LANDCOVER_KINDS
        .iter()
        .enumerate()
        .map(|(index, kind)| {
            let (x, y) = match *kind {
                "barren" => (100, 100),
                "grassland" => (300, 100),
                "urban_area" => (500, 100),
                _ => (1_600 + index as i32 * 180, 400),
            };
            FeatureSpec {
                id: Some(2_000 + index as u64),
                geom_type: GeomType::Polygon,
                paths: square(x, y),
                tags: vec![(0, index as u32)],
            }
        })
        .collect::<Vec<_>>();
    let bytes = mvt::encode::tile_layers(&[
        LayerSpec {
            name: "landuse",
            extent: 4096,
            keys: &["kind", "sport"],
            values: &landuse_values,
            features: &landuse_features,
        },
        LayerSpec {
            name: "landcover",
            extent: 4096,
            keys: &["kind"],
            values: &landcover_values,
            features: &landcover_features,
        },
    ]);
    SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), TILE_KEY)
}

fn kinds(tile: &SemanticTile, layer_name: &str) -> Vec<String> {
    let layer = tile.semantic_layer(layer_name).unwrap();
    layer
        .features
        .iter()
        .map(|feature| {
            let fact = match &feature.facts {
                LayerFacts::Landuse(facts) => facts.kind,
                LayerFacts::Landcover(facts) => facts.kind,
                _ => None,
            }
            .unwrap();
            tile.source_text(layer, fact).unwrap().to_string()
        })
        .collect()
}

#[test]
fn fixture_preserves_layer_attribution_identity_geometry_and_context() {
    let tile = fixture_tile();
    assert_eq!(kinds(&tile, "landuse"), LANDUSE_KINDS);
    assert_eq!(kinds(&tile, "landcover"), LANDCOVER_KINDS);

    for layer_name in ["landuse", "landcover"] {
        let layer = tile.semantic_layer(layer_name).unwrap();
        for (index, semantic) in layer.features.iter().enumerate() {
            assert_eq!(semantic.geometry_type, GeomType::Polygon);
            assert_eq!(
                semantic.source_id,
                Some(
                    if layer_name == "landuse" {
                        1_000
                    } else {
                        2_000
                    } + index as u64
                )
            );
            assert!(
                !tile
                    .source_feature(layer, semantic)
                    .unwrap()
                    .paths
                    .is_empty()
            );
        }
    }

    let landuse = tile.semantic_layer("landuse").unwrap();
    let pitch = &landuse.features[12];
    let LayerFacts::Landuse(facts) = &pitch.facts else {
        panic!("pitch must retain landuse facts");
    };
    assert_eq!(
        facts.sport.and_then(|fact| tile.source_text(landuse, fact)),
        Some("soccer")
    );

    let landcover = tile.semantic_layer("landcover").unwrap();
    for (detailed_index, coarse_index) in [(0, 0), (3, 4), (13, 6)] {
        let detailed = tile
            .source_feature(landuse, &landuse.features[detailed_index])
            .unwrap();
        let coarse = tile
            .source_feature(landcover, &landcover.features[coarse_index])
            .unwrap();
        assert_eq!(detailed.paths, coarse.paths);
    }
}

#[test]
fn fixture_classifies_exact_land_kinds_without_collapsing_layers() {
    let tile = fixture_tile();
    let landuse = tile.semantic_layer("landuse").unwrap();
    let expected_landuse = [
        LanduseKind::BareRock,
        LanduseKind::Wetland,
        LanduseKind::Scrub,
        LanduseKind::Grass,
        LanduseKind::Grassland,
        LanduseKind::Glacier,
        LanduseKind::Farmyard,
        LanduseKind::Railway,
        LanduseKind::Aerodrome,
        LanduseKind::Runway,
        LanduseKind::Taxiway,
        LanduseKind::Dam,
        LanduseKind::Pitch,
        LanduseKind::Residential,
        LanduseKind::Farmland,
    ];
    for (feature, expected) in landuse.features.iter().zip(expected_landuse) {
        let LayerFacts::Landuse(facts) = &feature.facts else {
            panic!("fixture feature must retain landuse facts");
        };
        assert_eq!(facts.class, Some(expected));
    }

    let landcover = tile.semantic_layer("landcover").unwrap();
    let expected_landcover = [
        LandcoverKind::Barren,
        LandcoverKind::Farmland,
        LandcoverKind::Forest,
        LandcoverKind::Glacier,
        LandcoverKind::Grassland,
        LandcoverKind::Scrub,
        LandcoverKind::UrbanArea,
    ];
    for (feature, expected) in landcover.features.iter().zip(expected_landcover) {
        let LayerFacts::Landcover(facts) = &feature.facts else {
            panic!("fixture feature must retain landcover facts");
        };
        assert_eq!(facts.class, Some(expected));
    }
}

#[test]
fn unknown_land_kind_remains_raw_and_auditable() {
    let bytes = mvt::encode::tile(
        "landuse",
        4096,
        &["kind"],
        &[Value::Str("future_land_kind".into())],
        &[FeatureSpec {
            id: Some(9_001),
            geom_type: GeomType::Polygon,
            paths: square(100, 100),
            tags: vec![(0, 0)],
        }],
    );
    let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), TILE_KEY);
    let layer = tile.semantic_layer("landuse").unwrap();
    let feature = &layer.features[0];
    let LayerFacts::Landuse(facts) = &feature.facts else {
        panic!("fixture feature must retain landuse facts");
    };
    assert_eq!(facts.class, Some(LanduseKind::Unknown));
    assert_eq!(
        facts.kind.and_then(|fact| tile.source_text(layer, fact)),
        Some("future_land_kind")
    );
    assert!(tile.diagnostics().iter().any(|diagnostic| {
        diagnostic.layer == "landuse"
            && diagnostic.field == "kind"
            && diagnostic.kind == SemanticDiagnosticKind::UnknownText
    }));
}

#[test]
fn land_authority_is_order_independent() {
    let candidates = [
        LandAuthorityCandidate {
            authority: LandEvidenceAuthority::RegionalLandcover,
            stable_identity: 50,
        },
        LandAuthorityCandidate {
            authority: LandEvidenceAuthority::InferredTerrain,
            stable_identity: 40,
        },
        LandAuthorityCandidate {
            authority: LandEvidenceAuthority::DetailedLanduse,
            stable_identity: 30,
        },
        LandAuthorityCandidate {
            authority: LandEvidenceAuthority::ExistingMappedFloraOrForest,
            stable_identity: 20,
        },
        LandAuthorityCandidate {
            authority: LandEvidenceAuthority::PhysicalOverlay,
            stable_identity: 10,
        },
    ];
    let choose = |values: &[LandAuthorityCandidate]| {
        values
            .iter()
            .copied()
            .reduce(preferred_land_authority)
            .unwrap()
    };
    let mut reversed = candidates;
    reversed.reverse();
    assert_eq!(choose(&candidates), choose(&reversed));
    assert_eq!(choose(&candidates), candidates[4]);

    let high_id = LandAuthorityCandidate {
        authority: LandEvidenceAuthority::DetailedLanduse,
        stable_identity: 9,
    };
    let low_id = LandAuthorityCandidate {
        authority: LandEvidenceAuthority::DetailedLanduse,
        stable_identity: 3,
    };
    assert_eq!(preferred_land_authority(high_id, low_id), low_id);
    assert_eq!(preferred_land_authority(low_id, high_id), low_id);
}

#[test]
fn detailed_land_extraction_changes_only_the_reviewed_target_kinds() {
    let tile = fixture_tile();
    let telemetry = SemanticTelemetry::new_enabled("land-l0-fixture", 0x20);
    telemetry.record_normalized_tile(&tile);
    let mut pack = FeaturePack::empty();
    let mut detail = telemetry.consumer_report();
    append_current_tile_features(&mut pack, &tile, TILE_KEY, 15, (0.0, 0.0), &mut detail);
    telemetry.record_consumer(&detail);
    let mut coarse = telemetry.consumer_report();
    append_semantic_land_regions(
        &mut pack,
        &tile,
        "landcover",
        TILE_KEY.0,
        TILE_KEY.1,
        TILE_KEY.2,
        0.0,
        0.0,
        &mut coarse,
    );
    telemetry.record_consumer(&coarse);

    assert_eq!(
        pack.ground
            .iter()
            .map(|(kind, _)| *kind)
            .collect::<Vec<_>>(),
        vec![
            GroundKind::BareRock,
            GroundKind::Wetland,
            GroundKind::Scrub,
            GroundKind::NaturalGrass,
            GroundKind::Glacier,
            GroundKind::Farmyard,
            GroundKind::RailwayArea,
            GroundKind::AirportArea,
            GroundKind::RunwayArea,
            GroundKind::RunwayArea,
            GroundKind::Green,
            GroundKind::Urban,
        ]
    );
    assert!(pack.forest.is_empty());
    assert_eq!(pack.flora.len(), 1, "detailed flora remains separate");
    assert_eq!(pack.flora[0].kind, FloraRegionKind::Farmland);
    assert_eq!(pack.regional_landcover.len(), 7);
    assert_eq!(
        pack.regional_landcover
            .iter()
            .map(|region| region.kind)
            .collect::<Vec<_>>(),
        vec![
            LandcoverKind::Barren,
            LandcoverKind::Farmland,
            LandcoverKind::Forest,
            LandcoverKind::Glacier,
            LandcoverKind::Grassland,
            LandcoverKind::Scrub,
            LandcoverKind::UrbanArea,
        ]
    );
    for (index, region) in pack.regional_landcover.iter().enumerate() {
        assert_eq!(region.source_id, Some(2_000 + index as u64));
        assert_ne!(region.stable_identity, 0);
        assert!(!region.rings.is_empty());
    }

    let snapshot = telemetry.snapshot();
    assert_eq!(snapshot.layer_source(SemanticLayerFamily::Landuse), 15);
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Landuse), 13);
    assert_eq!(snapshot.layer_rejected(SemanticLayerFamily::Landuse), 2);
    assert_eq!(snapshot.layer_source(SemanticLayerFamily::Landcover), 7);
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Landcover), 7);
    assert_eq!(snapshot.layer_rejected(SemanticLayerFamily::Landcover), 0);
    assert_eq!(
        snapshot.discard_count(
            SemanticLayerFamily::Landuse,
            SemanticDiscardReason::UnsupportedSemantic,
        ),
        2
    );
    assert_eq!(
        snapshot.discard_count(
            SemanticLayerFamily::Landcover,
            SemanticDiscardReason::UnsupportedSemantic,
        ),
        0
    );
}

#[test]
fn managed_substrates_leave_deferred_and_existing_land_contracts_intact() {
    for (class, expected) in [
        (LanduseKind::Farmyard, GroundKind::Farmyard),
        (LanduseKind::Railway, GroundKind::RailwayArea),
        (LanduseKind::Aerodrome, GroundKind::AirportArea),
        (LanduseKind::Airfield, GroundKind::AirportArea),
        (LanduseKind::Runway, GroundKind::RunwayArea),
        (LanduseKind::Taxiway, GroundKind::RunwayArea),
    ] {
        assert_eq!(managed_ground_kind(Some(class)), Some(expected));
    }
    for class in [
        LanduseKind::Dam,
        LanduseKind::NationalPark,
        LanduseKind::NatureReserve,
    ] {
        assert_eq!(managed_ground_kind(Some(class)), None);
        assert_eq!(natural_ground_kind(Some(class)), None);
    }

    for (source, expected) in [
        ("park", GroundKind::Green),
        ("garden", GroundKind::Green),
        ("beach", GroundKind::Sand),
        ("residential", GroundKind::Urban),
    ] {
        if source == "garden" {
            assert_eq!(flora_region_kind(source), Some(FloraRegionKind::Garden));
        } else {
            assert_eq!(ground_kind(source), Some(expected));
        }
    }
    assert_eq!(
        flora_region_kind("farmland"),
        Some(FloraRegionKind::Farmland)
    );
    assert_eq!(flora_region_kind("garden"), Some(FloraRegionKind::Garden));
    assert_eq!(ground_kind("aerodrome"), None);

    let source_kinds = [
        "farmyard",
        "railway",
        "aerodrome",
        "airfield",
        "runway",
        "taxiway",
    ];
    let values = source_kinds
        .into_iter()
        .map(|kind| Value::Str(kind.into()))
        .collect::<Vec<_>>();
    let features = (0..values.len())
        .map(|index| FeatureSpec {
            id: Some(2_500 + index as u64),
            geom_type: GeomType::Polygon,
            paths: square(100 + index as i32 * 180, 100),
            tags: vec![(0, index as u32)],
        })
        .collect::<Vec<_>>();
    let bytes = mvt::encode::tile("landuse", 4096, &["kind"], &values, &features);
    let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), TILE_KEY);
    let telemetry = SemanticTelemetry::new_enabled("land-l2-managed", 0x21);
    telemetry.record_normalized_tile(&tile);
    let mut pack = FeaturePack::empty();
    let mut report = telemetry.consumer_report();
    append_current_tile_features(&mut pack, &tile, TILE_KEY, 15, (0.0, 0.0), &mut report);
    telemetry.record_consumer(&report);
    assert_eq!(
        pack.ground
            .iter()
            .map(|(kind, _)| *kind)
            .collect::<Vec<_>>(),
        vec![
            GroundKind::Farmyard,
            GroundKind::RailwayArea,
            GroundKind::AirportArea,
            GroundKind::AirportArea,
            GroundKind::RunwayArea,
            GroundKind::RunwayArea,
        ]
    );
    let snapshot = telemetry.snapshot();
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Landuse), 6);
    assert_eq!(snapshot.layer_rejected(SemanticLayerFamily::Landuse), 0);
}

#[test]
fn dam_and_protected_areas_remain_typed_but_physically_deferred() {
    let values = ["dam", "national_park", "nature_reserve"]
        .into_iter()
        .map(|kind| Value::Str(kind.into()))
        .collect::<Vec<_>>();
    let features = (0..values.len())
        .map(|index| FeatureSpec {
            id: Some(3_000 + index as u64),
            geom_type: GeomType::Polygon,
            paths: square(100 + index as i32 * 180, 100),
            tags: vec![(0, index as u32)],
        })
        .collect::<Vec<_>>();
    let bytes = mvt::encode::tile("landuse", 4096, &["kind"], &values, &features);
    let tile = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), TILE_KEY);
    let layer = tile.semantic_layer("landuse").unwrap();
    let classes = layer
        .features
        .iter()
        .map(|feature| match &feature.facts {
            LayerFacts::Landuse(facts) => facts.class.unwrap(),
            _ => panic!("fixture feature must retain landuse facts"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        classes,
        vec![
            LanduseKind::Dam,
            LanduseKind::NationalPark,
            LanduseKind::NatureReserve,
        ]
    );

    let telemetry = SemanticTelemetry::new_enabled("land-l2-deferred", 0x22);
    telemetry.record_normalized_tile(&tile);
    let mut pack = FeaturePack::empty();
    let mut report = telemetry.consumer_report();
    append_current_tile_features(&mut pack, &tile, TILE_KEY, 15, (0.0, 0.0), &mut report);
    telemetry.record_consumer(&report);
    assert!(pack.ground.is_empty());
    assert!(pack.forest.is_empty());
    assert!(pack.flora.is_empty());
    let snapshot = telemetry.snapshot();
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Landuse), 0);
    assert_eq!(snapshot.layer_rejected(SemanticLayerFamily::Landuse), 3);
    assert_eq!(
        snapshot.discard_count(
            SemanticLayerFamily::Landuse,
            SemanticDiscardReason::UnsupportedSemantic,
        ),
        3
    );
}
